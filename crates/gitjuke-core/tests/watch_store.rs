use gitjuke_core::{github::Response, model::*, runtime::Job, store::Store, sync::*};
use serde_json::json;
fn pr(n: u64) -> PullRequest {
    PullRequest {
        id: format!("P{n}"),
        repo_id: Some(1),
        repo: "a/b".into(),
        number: n,
        title: "T".into(),
        url: format!("https://github.com/a/b/pull/{n}"),
        state: "open".into(),
        draft: false,
        author: None,
        updated_at: "2026-10-17T00:00:00Z".into(),
    }
}
#[test]
fn schema_one_upgrade_preserves_identity_pause_and_watch_reasons() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = Store::open(&p).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.begin_source("authored").unwrap();
    s.stage_source("authored", &[pr(1)]).unwrap();
    s.publish_source("authored").unwrap();
    drop(s);
    let c = rusqlite::Connection::open(&p).unwrap();
    let names = c
        .prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND name GLOB 'shared_*'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for name in names {
        c.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    }
    c.execute_batch("DROP TABLE saved_query;DROP TABLE shared_bytes;DROP TABLE shared_check_lru")
        .unwrap();
    for t in [
        "watch_locator",
        "watch_source",
        "source_stage",
        "teams",
        "teams_stage",
        "pending",
        "check_context",
        "check_state",
        "activity",
        "activity_stage",
        "activity_target",
        "activity_cycle",
        "activity_intent",
        "intent_member",
        "api_validator",
        "invalidation",
        "content_clear",
        "check_purge",
        "pr_purge",
        "source_restart",
        "publication_row",
        "publication",
    ] {
        c.execute(&format!("DROP TABLE {t}"), []).unwrap();
    }
    c.execute_batch("DROP TABLE retention_cursor; DROP INDEX IF EXISTS activity_expiry; DROP INDEX IF EXISTS check_expiry; DROP INDEX IF EXISTS check_published_keys; DROP INDEX IF EXISTS publication_unapplied; DROP TABLE control_sweep; DROP INDEX content_lru")
        .unwrap();
    c.pragma_update(None, "user_version", 1).unwrap();
    drop(c);
    let s = Store::open(&p).unwrap();
    assert!(s.settings().unwrap().paused);
    assert_eq!(s.identity().unwrap().unwrap().id, 42);
    let w = s.watch_page(0).unwrap();
    assert_eq!(w.items.len(), 1);
    assert_eq!(w.items[0].reasons, vec!["authored"]);
}
#[test]
fn durable_due_work_is_bounded_in_memory_fair_and_does_not_lose_the_129th() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    for n in 0..140 {
        s.enqueue(
            &format!("fg{n}"),
            &Job::Overview("a/b".into(), n),
            true,
            true,
            0,
            false,
        )
        .unwrap();
    }
    s.enqueue(
        "background",
        &Job::Discover(Discovery::new("authored", "")),
        false,
        false,
        0,
        false,
    )
    .unwrap();
    let page = s.pending(1).unwrap();
    assert_eq!(page.len(), 128);
    assert!(page.iter().any(|row| !row.4));
    for p in &page {
        s.ack(p.0).unwrap();
    }
    assert_eq!(s.pending(1).unwrap().len(), 13);
}

#[test]
fn head_reservations_exclude_exact_ids_without_null_or_unrelated_job_poisoning() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.begin_source("authored").unwrap();
    s.stage_source("authored", &[pr(1), pr(2), pr(3)]).unwrap();
    s.publish_source("authored").unwrap();
    let targets = s.due_watches(gitjuke_core::now(), 3, None, false).unwrap();
    assert_eq!(targets.len(), 3);
    s.enqueue(
        "reserved",
        &Job::Heads(vec![targets[0].clone()]),
        false,
        false,
        0,
        false,
    )
    .unwrap();
    s.enqueue(
        "nulls",
        &json!({"Heads":[{"id":null},{"number":2},{"id":99}]}),
        false,
        false,
        0,
        false,
    )
    .unwrap();
    s.enqueue(
        "overview",
        &Job::Overview("a/b".into(), 2),
        false,
        false,
        0,
        false,
    )
    .unwrap();
    let ids = |s: &Store| {
        s.due_watches(gitjuke_core::now(), 3, Some(&("a/b".into(), 3)), false)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&s), ["P3", "P2"]);
    let seq = s
        .pending(gitjuke_core::now())
        .unwrap()
        .into_iter()
        .find(|p| p.1 == "reserved")
        .unwrap()
        .0;
    s.ack(seq).unwrap();
    assert_eq!(ids(&s), ["P3", "P1", "P2"]);
}
#[test]
fn head_generation_blocks_old_pages_and_clear_does_not_remove_watch_reasons() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.begin_source("authored").unwrap();
    s.stage_source("authored", &[pr(1)]).unwrap();
    s.publish_source("authored").unwrap();
    let target = s.due_watches(1, 10, None, false).unwrap().remove(0);
    let read = HeadRead {
        snapshot: HeadSnapshot {
            pr: pr(1),
            head: "old".into(),
            merge: None,
            review: None,
            generation: 0,
            fetched_at: 1,
        },
        pages: vec![],
    };
    let current = s.accept_head(&read, &target).unwrap().unwrap();
    let old = CheckTarget {
        page_size: 25,
        watch: current.clone(),
        head: "old".into(),
        commit: "old".into(),
        cursor: "next".into(),
        total: 0,
    };
    assert!(s.valid_check(&old).unwrap());
    let mut read = read;
    read.snapshot.head = "new".into();
    s.accept_head(&read, &current).unwrap();
    assert!(!s.valid_check(&old).unwrap());
    assert!(
        !s.stage_contexts(
            &old,
            &ContextPage {
                contexts: vec![],
                next: None,
                total: 0,
                commit: "old".into()
            },
            false
        )
        .unwrap()
    );
    s.clear().unwrap();
    assert_eq!(s.watch_page(0).unwrap().items.len(), 1);
}
#[test]
fn budget_deadlines_and_exponential_backoff_survive_restart_and_do_not_cross_primary_buckets() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let s = Store::open(&p).unwrap();
    let mut r = Response {
        status: 200,
        headers: Default::default(),
        body: json!({}),
    };
    r.headers.insert("x-ratelimit-remaining".into(), "0".into());
    r.headers.insert("x-ratelimit-reset".into(), "500".into());
    s.record_response("search", &r, 123, 100).unwrap();
    assert!(!s.budget_eligible("search", 101).unwrap());
    assert!(s.budget_eligible("graphql", 101).unwrap());
    s.record_failure("graphql", 100).unwrap();
    assert_eq!(s.due("backoff:graphql").unwrap(), 130);
    s.record_failure("graphql", 100).unwrap();
    assert_eq!(s.due("backoff:graphql").unwrap(), 160);
    drop(s);
    let s = Store::open(&p).unwrap();
    assert!(!s.budget_eligible("search", 101).unwrap());
    assert!(!s.budget_eligible("graphql", 101).unwrap());
    assert!(s.budget_eligible("graphql", 161).unwrap());
}
#[test]
fn complete_team_membership_removal_keeps_other_membership_reasons() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    for source in ["authored", "team:T1"] {
        s.begin_source(source).unwrap();
        s.stage_source(source, &[pr(1)]).unwrap();
        s.publish_source(source).unwrap();
    }
    s.team_stage(&json!([]), true).unwrap();
    s.publish_teams().unwrap();
    assert_eq!(s.watch_page(0).unwrap().items[0].reasons, vec!["authored"]);
}
#[test]
fn restarting_paused_does_not_replay_prior_session_manual_jobs() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = Store::open(&p).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    s.enqueue(
        "heads",
        &Job::Heads(vec![WatchTarget {
            id: "P1".into(),
            repo: "a/b".into(),
            number: 1,
            generation: 0,
        }]),
        true,
        true,
        0,
        false,
    )
    .unwrap();
    drop(s);
    let s = Store::open(&p).unwrap();
    assert!(s.pending(1).unwrap().is_empty());
    assert!(s.settings().unwrap().paused);
}

#[test]
fn list_bundles_stack_metadata_atomically_with_prs_and_includes_every_stack_on_the_page() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let items: Vec<_> = (1..=12).map(pr).collect();
    s.begin_source("authored").unwrap();
    s.stage_source("authored", &items).unwrap();
    s.publish_source("authored").unwrap();
    let generation = s.generation().unwrap();
    for item in &items {
        let overview = Overview {
            repo_id: item.repo_id,
            id: item.id.clone(),
            repo: item.repo.clone(),
            number: item.number,
            title: item.title.clone(),
            url: item.url.clone(),
            body: None,
            body_limited: false,
            head: "a".repeat(40),
            base: "main".into(),
            state: "open".into(),
            draft: false,
            author: None,
            stack: StackState::Published {
                number: item.number,
                trunk: "main".into(),
                members: vec![StackMember {
                    number: item.number,
                    title: item.title.clone(),
                    url: item.url.clone(),
                }],
            },
        };
        s.put(&format!("pr:a/b:{}", item.number), &overview, generation)
            .unwrap();
    }
    let watches_before = s.watch_page(0).unwrap().items.len();
    let page = s.list_page("authored", 0).unwrap();
    assert_eq!(page.items.len(), 12);
    assert_eq!(page.stacks.len(), 12);
    assert_eq!(page.stacks[0].repo, "a/b");
    assert!(
        matches!(&page.stacks[0].stack, StackState::Published { members, .. }
        if members.iter().any(|member| member.number == page.items[0].number))
    );
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json["stacks"].as_array().unwrap().len(), 12);
    assert_eq!(s.watch_page(0).unwrap().items.len(), watches_before);
    assert!(s.pending(u64::MAX / 2).unwrap().is_empty());
    // A second list read returns the same bundled topology, without hydration work.
    assert_eq!(s.list_page("authored", 0).unwrap().stacks.len(), 12);
    s.clear().unwrap();
    let cleared = s.list_page("authored", 0).unwrap();
    assert!(cleared.items.is_empty());
    assert!(cleared.stacks.is_empty());
}

#[test]
fn main_lists_filter_closed_prs_and_sort_by_updated_before_pagination() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })
    .unwrap();
    let items: Vec<_> = (1..=107)
        .map(|n| {
            let mut item = pr(n);
            item.updated_at = date(1_790_000_000 + n);
            item
        })
        .collect();
    for source in ["authored", "direct", "team:1"] {
        s.begin_source(source).unwrap();
        s.stage_source(source, &items).unwrap();
        s.publish_source(source).unwrap();
    }
    // A newer cached snapshot can close a PR before source discovery catches up.
    let generation = s.generation().unwrap();
    for (number, state) in [(1, "closed"), (2, "merged")] {
        let mut item = pr(number);
        item.state = state.into();
        item.updated_at = date(1_790_001_000);
        s.put(&format!("list:{}", item.id), &item, generation)
            .unwrap();
    }
    for view in ["authored", "requested"] {
        let first = s.list_page(view, 0).unwrap();
        assert_eq!(first.items.len(), 100);
        assert_eq!(first.next_offset, Some(100));
        assert_eq!(
            first.items.iter().map(|p| p.number).collect::<Vec<_>>(),
            (8..=107).rev().collect::<Vec<_>>()
        );
        let last = s.list_page(view, 100).unwrap();
        assert_eq!(
            last.items.iter().map(|p| p.number).collect::<Vec<_>>(),
            vec![7, 6, 5, 4, 3]
        );
        assert_eq!(last.next_offset, None);
        assert!(
            first
                .items
                .iter()
                .chain(&last.items)
                .all(|p| p.state == "open")
        );
        // Re-reading preserves pagination order, even though cache access times change.
        assert_eq!(s.list_page(view, 0).unwrap().items[0].number, 107);
    }
}
