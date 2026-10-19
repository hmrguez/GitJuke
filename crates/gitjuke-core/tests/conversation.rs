use gitjuke_core::{
    conversation::{ConversationJob, ConversationPage},
    runtime::Runtime,
    store::Store,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tempfile::TempDir;
static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn scenario(d: &TempDir, v: Value) {
    let p = d.path().join("scenario.tmp");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    std::fs::rename(p, d.path().join("scenario.json")).unwrap();
}
fn config() -> Value {
    json!({"head":"a".repeat(40),"no_teams":true,"direct_ids":[],"files_count":45})
}
async fn until(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Conversation fixture reached expected state");
}
async fn start() -> (TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    let gh = d.path().join("gh");
    std::fs::write(&gh, include_str!("support/gh.py")).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    scenario(&d, config());
    let mut s = Store::open(&d.path().join("db")).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    settings.gh_path = Some(gh.display().to_string());
    settings.config_dir = Some(d.path().join("config").display().to_string());
    s.save_settings(&settings).unwrap();
    drop(s);
    let r = Runtime::start(d.path().join("db"), |_| {}).unwrap();
    r.refresh().unwrap();
    until(|| r.status().is_ok_and(|s| !s.locked)).await;
    r.select("a/b".into(), 1).unwrap();
    until(|| r.overview("a/b", 1).is_ok_and(|s| s.data.is_some())).await;
    (d, r)
}
fn job(source: &str) -> ConversationJob {
    ConversationJob {
        repo: "a/b".into(),
        number: 1,
        head: "a".repeat(40),
        source: source.into(),
        thread: None,
        after: None,
        seq: 0,
    }
}
async fn read(r: &Runtime, j: &ConversationJob) -> ConversationPage {
    r.request_conversation(j.clone()).unwrap();
    until(|| {
        r.conversation(j)
            .is_ok_and(|p| p.data.is_some() && p.error.is_none())
    })
    .await;
    r.conversation(j).unwrap().data.unwrap()
}
async fn failure(r: &Runtime, j: &ConversationJob) {
    until(|| {
        r.conversation(j)
            .is_ok_and(|p| p.error.is_some_and(|e| !e.starts_with("Queued —")))
    })
    .await;
}
#[tokio::test]
async fn independent_pages_thread_replies_and_ipc_subdivision() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let comments = read(&r, &job("comments")).await;
    assert_eq!(comments.messages.len(), 50);
    assert_eq!(comments.next.as_deref(), Some("50"));
    assert!(!comments.complete);
    let reviews = read(&r, &job("reviews")).await;
    assert_eq!(reviews.messages.len(), 50);
    assert_eq!(reviews.messages[0].kind, "APPROVED");
    let mut next = job("comments");
    next.after = comments.next;
    let p = read(&r, &next).await;
    assert_eq!(p.messages[0].id, "comments_50");
    assert_eq!(p.messages.len(), 50);
    next.after = p.next;
    let p = read(&r, &next).await;
    assert_eq!(p.messages.len(), 5);
    assert!(p.next.is_none());
    assert!(!p.complete);
    let threads = read(&r, &job("threads")).await;
    assert_eq!(threads.threads.len(), 20);
    assert!(threads.messages.is_empty());
    let mut replies = job("replies");
    replies.thread = Some(threads.threads[1].id.clone());
    let p = read(&r, &replies).await;
    assert_eq!(p.messages.len(), 50);
    assert_eq!(p.next.as_deref(), Some("50"));
    let mut v = config();
    v["conversation_body"] = json!("\u{0001}".repeat(131072));
    scenario(&d, v);
    let p = read(&r, &job("comments")).await;
    assert_eq!(p.messages.len(), 1);
    assert_eq!(p.next.as_deref(), Some("1"));
    assert_eq!(p.messages[0].body.as_ref().unwrap().len(), 131072);
    assert!(
        serde_json::to_vec(&r.conversation(&job("comments")).unwrap())
            .unwrap()
            .len()
            < 1024 * 1024
    );
    let mut v = config();
    v["conversation_body"] = json!("é".repeat(65536) + "x");
    scenario(&d, v);
    let p = read(&r, &job("comments")).await;
    // Python JSON escapes non-ASCII: 50 bodies exceed the 16-MiB transport cap.
    assert_eq!(p.messages.len(), 25);
    assert_eq!(p.messages[0].state, "oversized");
    assert!(p.messages[0].body.is_none());
    let mut v = config();
    v["conversation_body"] = json!("");
    v["conversation_count"] = json!(1);
    scenario(&d, v);
    let p = read(&r, &job("reviews")).await;
    assert_eq!(p.messages[0].body.as_deref(), Some(""));
    assert!(p.complete);
    r.shutdown().await;
}
#[tokio::test]
async fn cancellation_clear_and_stale_revision_never_publish() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut v = config();
    v["slow_conversation"] = json!(3);
    scenario(&d, v);
    let j = job("comments");
    r.request_conversation(j.clone()).unwrap();
    until(|| d.path().join("conversation-started").exists()).await;
    r.cancel_conversation().unwrap();
    scenario(&d, config());
    until(|| r.status().is_ok_and(|s| !s.busy)).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    let _ = std::fs::remove_file(d.path().join("conversation-started"));
    let mut v = config();
    v["slow_conversation"] = json!(3);
    scenario(&d, v);
    r.request_conversation(j.clone()).unwrap();
    until(|| d.path().join("conversation-started").exists()).await;
    r.clear().unwrap();
    scenario(&d, config());
    until(|| r.status().is_ok_and(|s| !s.busy)).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    let mut v = config();
    v["conversation_head"] = json!("c".repeat(40));
    scenario(&d, v);
    r.request_conversation(j.clone()).unwrap();
    failure(&r, &j).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    let mut v = config();
    v["thread_repo"] = json!("wrong/repo");
    scenario(&d, v);
    let mut reply = job("replies");
    reply.thread = Some("T_1".into());
    r.request_conversation(reply.clone()).unwrap();
    failure(&r, &reply).await;
    assert!(r.conversation(&reply).unwrap().data.is_none());
    r.shutdown().await;
}
#[tokio::test]
async fn denial_identity_and_selected_pr_barriers() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let j = job("comments");
    read(&r, &j).await;
    let mut v = config();
    v["deny_conversation"] = json!(true);
    scenario(&d, v);
    r.request_conversation(j.clone()).unwrap();
    failure(&r, &j).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    scenario(&d, config());
    assert!(!read(&r, &j).await.messages.is_empty());
    std::fs::write(d.path().join("mode"), "mismatch").unwrap();
    r.request_conversation(j.clone()).unwrap();
    until(|| r.status().is_ok_and(|s| s.locked)).await;
    assert!(r.conversation(&j).is_err());
    r.shutdown().await;
}
#[tokio::test]
async fn explicit_seen_receipts_reject_changed_body_clear_and_placeholders() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    let ident = "github.com:a/b:1:comments_0";
    sql.execute(
        "INSERT INTO activity VALUES(?1,'a/b',1,'comment',?2,1,'eligible',1)",
        rusqlite::params![ident, gitjuke_core::now() as i64],
    )
    .unwrap();
    sql.execute("INSERT INTO seen VALUES(?1,0)", [ident])
        .unwrap();
    let j = job("comments");
    let p = read(&r, &j).await;
    let m = &p.messages[0];
    let seen = || {
        sql.query_row("SELECT seen FROM seen WHERE source_id=?", [ident], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    assert_eq!(seen(), 0);
    assert!(
        r.conversation_seen(&j, &m.id, "forged", p.generation)
            .is_err()
    );
    assert_eq!(seen(), 0);
    r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
        .unwrap();
    assert_eq!(seen(), 1);
    sql.execute("UPDATE seen SET seen=0 WHERE source_id=?", [ident])
        .unwrap();
    let mut v = config();
    v["conversation_body"] = json!("Changed body");
    scenario(&d, v);
    let updated = read(&r, &j).await;
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
            .is_err()
    );
    assert_eq!(seen(), 0);
    r.clear().unwrap();
    assert!(
        r.conversation_seen(
            &j,
            &m.id,
            updated.messages[0].digest.as_deref().unwrap(),
            updated.generation
        )
        .is_err()
    );
    assert_eq!(seen(), 0);
    let mut v = config();
    v["conversation_body"] = json!("x".repeat(131073));
    scenario(&d, v);
    let large = read(&r, &j).await;
    assert!(
        r.conversation_seen(&j, &large.messages[0].id, "", large.generation)
            .is_err()
    );
    assert_eq!(seen(), 0);
    let approval = "github.com:a/b:1:reviews_0";
    sql.execute(
        "INSERT INTO activity VALUES(?1,'a/b',1,'review',?2,1,'eligible',1)",
        rusqlite::params![approval, gitjuke_core::now() as i64],
    )
    .unwrap();
    sql.execute("INSERT INTO seen VALUES(?1,0)", [approval])
        .unwrap();
    let mut v = config();
    v["conversation_body"] = json!("");
    scenario(&d, v);
    let j = job("reviews");
    let p = read(&r, &j).await;
    let m = &p.messages[0];
    r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
        .unwrap();
    assert_eq!(
        sql.query_row("SELECT seen FROM seen WHERE source_id=?", [approval], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    r.shutdown().await;
}
#[tokio::test]
async fn replacement_ticket_and_moving_comparison_reject_late_results() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    let mut v = config();
    v["slow_conversation"] = json!(3);
    scenario(&d, v);
    let old = job("comments");
    r.request_conversation(old.clone()).unwrap();
    until(|| d.path().join("conversation-started").exists()).await;
    scenario(&d, config());
    let replacement = read(&r, &job("reviews")).await;
    assert_eq!(replacement.messages.len(), 50);
    assert!(r.conversation(&old).unwrap().data.is_none());
    let _ = std::fs::remove_file(d.path().join("conversation-started"));
    let mut v = config();
    v["slow_conversation"] = json!(2);
    scenario(&d, v);
    r.request_conversation(old.clone()).unwrap();
    until(|| d.path().join("conversation-started").exists()).await;
    let mut moved = config();
    moved["base_sha"] = json!("c".repeat(40));
    scenario(&d, moved);
    failure(&r, &old).await;
    assert!(r.conversation(&old).unwrap().data.is_none());
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM pending WHERE key GLOB 'conversation:selected:*'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    r.shutdown().await;
}

fn retain(d: &TempDir, node: &str) {
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    let id = format!("github.com:a/b:1:{node}");
    sql.execute(
        "INSERT INTO activity VALUES(?1,'a/b',1,'comment',?2,1,'eligible',1)",
        rusqlite::params![id, gitjuke_core::now() as i64],
    )
    .unwrap();
    sql.execute("INSERT INTO seen VALUES(?1,0)", [id]).unwrap();
}
fn destination(node: &str) -> ConversationJob {
    ConversationJob {
        source: "destination".into(),
        thread: Some(node.into()),
        ..job("comments")
    }
}
#[tokio::test]
async fn exact_destinations_without_excerpts_or_history_scans() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    retain(&d, "target");
    retain(&d, "neighbor");
    let j = destination("target");
    assert!(r.request_conversation(destination("unknown")).is_err());
    for kind in [
        "IssueComment",
        "PullRequestReview",
        "PullRequestReviewComment",
    ] {
        let mut v = config();
        v["destination_type"] = json!(kind);
        v["conversation_body"] = json!("é".repeat(65536));
        scenario(&d, v);
        let page = read(&r, &j).await;
        let m = &page.messages[0];
        assert_eq!(page.messages.len(), 1);
        assert_eq!(m.id, "target");
        assert_eq!(m.bytes, Some(131072));
        assert!(!page.complete);
        assert!(page.next.is_none());
        assert_eq!(
            m.kind,
            if kind == "PullRequestReview" {
                "APPROVED"
            } else if kind == "IssueComment" {
                "Comment"
            } else {
                "Review comment"
            }
        );
        assert_eq!(
            page.destination_anchor.is_some(),
            kind == "PullRequestReviewComment"
        );
        assert!(
            serde_json::to_vec(&r.conversation(&j).unwrap())
                .unwrap()
                .len()
                < 1024 * 1024
        );
    }
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    assert_eq!(
        sql.query_row("SELECT SUM(seen) FROM seen", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let page = r.conversation(&j).unwrap().data.unwrap();
    let m = &page.messages[0];
    assert!(
        r.conversation_seen(
            &j,
            "neighbor",
            m.digest.as_deref().unwrap(),
            page.generation
        )
        .is_err()
    );
    sql.execute(
        "UPDATE content SET json=json_set(json,'$.head',?1) WHERE key='pr:a/b:1'",
        ["c".repeat(40)],
    )
    .unwrap();
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), page.generation)
            .is_err()
    );
    sql.execute(
        "UPDATE content SET json=json_set(json,'$.head',?1) WHERE key='pr:a/b:1'",
        [j.head.clone()],
    )
    .unwrap();
    r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), page.generation)
        .unwrap();
    assert_eq!(
        sql.query_row("SELECT SUM(seen) FROM seen", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    sql.execute("UPDATE seen SET seen=0", []).unwrap();
    let mut v = config();
    v["conversation_body"] = json!("é".repeat(65536) + "x");
    scenario(&d, v);
    let large = read(&r, &j).await;
    assert_eq!(large.messages[0].state, "oversized");
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), page.generation)
            .is_err()
    );
    // Every direct request is a singleton node read: no connection pagination/history search.
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    for line in reads.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["query"] == "query ConversationDestination" {
            assert_eq!(v["variables"]["thread"], "target");
            assert!(v["variables"].get("first").is_none());
            assert!(v["variables"].get("after").is_none());
        }
    }
    r.shutdown().await;
}
#[tokio::test]
async fn exact_destination_ownership_missing_cancellation_and_receipt_barriers() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    retain(&d, "target");
    let j = destination("target");
    for (key, value) in [
        ("destination_id", json!("neighbor")),
        ("thread_repo", json!("a/other")),
        ("destination_number", json!(2)),
        ("missing_destination", json!(true)),
        ("destination_type", json!("Issue")),
        ("conversation_head", json!("c".repeat(40))),
    ] {
        let mut v = config();
        v[key] = value;
        scenario(&d, v);
        r.request_conversation(j.clone()).unwrap();
        failure(&r, &j).await;
        assert!(r.conversation(&j).unwrap().data.is_none());
    }
    let mut v = config();
    v["slow_conversation"] = json!(3);
    scenario(&d, v);
    r.request_conversation(j.clone()).unwrap();
    until(|| d.path().join("conversation-started").exists()).await;
    r.cancel_conversation().unwrap();
    scenario(&d, config());
    until(|| r.status().is_ok_and(|s| !s.busy)).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    let p = read(&r, &j).await;
    let m = &p.messages[0];
    r.clear().unwrap();
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
            .is_err()
    );
    let mut v = config();
    v["destination_type"] = json!("PullRequestReview");
    v["conversation_body"] = json!("");
    scenario(&d, v);
    let p = read(&r, &j).await;
    let m = &p.messages[0];
    r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
        .unwrap();
    r.cancel_conversation().unwrap();
    // Eviction rejects a complete old receipt even though the durable Activity survives.
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    sql.execute("DELETE FROM content WHERE key=?", [j.key()])
        .unwrap();
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
            .is_err()
    );
    r.shutdown().await;
}

/// Read-only compatibility probe against pre-existing owned fixtures; explicitly opt in.
#[tokio::test]
#[ignore = "requires authorized stored gh identity and live network"]
async fn live_existing_owned_destinations_and_cursor_pages() {
    use gitjuke_core::{
        conversation::{destination_anchor, parse},
        transport::{ReadOp, Transport},
    };
    let t = Transport::discover(Some("/opt/homebrew/bin/gh"), None).unwrap();
    let c = tokio_util::sync::CancellationToken::new();
    let mut j = ConversationJob {
        repo: "hmrguez/FantasyEditor".into(),
        number: 4,
        ..job("comments")
    };
    let overview = t
        .read(
            ReadOp::Overview {
                repo: j.repo.clone(),
                number: j.number,
            },
            &c,
        )
        .await
        .unwrap();
    let overview = gitjuke_core::github::Response::parse(overview).unwrap();
    overview.require_ok().unwrap();
    j.head = overview.body["head"]["sha"].as_str().unwrap().into();
    for source in ["comments", "threads"] {
        j.source = source.into();
        j.thread = None;
        j.after = None;
        let first = t.read(ReadOp::Conversation(j.query(1)), &c).await.unwrap();
        let first = gitjuke_core::github::Response::parse(first).unwrap();
        first.require_ok().unwrap();
        let page = parse(&first.body, &j, 1).unwrap();
        assert!(page.next.is_some());
        let first_id = if source == "threads" {
            page.threads[0].id.clone()
        } else {
            page.messages[0].id.clone()
        };
        j.after = page.next;
        let second = t.read(ReadOp::Conversation(j.query(1)), &c).await.unwrap();
        let second = gitjuke_core::github::Response::parse(second).unwrap();
        second.require_ok().unwrap();
        let page = parse(&second.body, &j, 1).unwrap();
        let second_id = if source == "threads" {
            &page.threads[0].id
        } else {
            &page.messages[0].id
        };
        assert_ne!(&first_id, second_id);
        if source == "threads" {
            let reply = ConversationJob {
                source: "replies".into(),
                thread: Some(first_id),
                after: None,
                ..j.clone()
            };
            let response = t
                .read(ReadOp::Conversation(reply.query(50)), &c)
                .await
                .unwrap();
            let response = gitjuke_core::github::Response::parse(response).unwrap();
            response.require_ok().unwrap();
            assert!(
                !parse(&response.body, &reply, 50)
                    .unwrap()
                    .messages
                    .is_empty()
            );
        }
        println!(
            "live {source}: distinct first/second cursor pages, total {}",
            page.total
        );
    }
    for node in [
        "IC_kwDOQ6BuSs8AAAABN8uCzA",
        "PRR_kwDOQ6BuSs8AAAABI4jy-w",
        "PRRC_kwDOQ6BuSs7fIRUs",
    ] {
        j.source = "destination".into();
        j.thread = Some(node.into());
        j.after = None;
        let response = t.read(ReadOp::Conversation(j.query(1)), &c).await.unwrap();
        let response = gitjuke_core::github::Response::parse(response).unwrap();
        response.require_ok().unwrap();
        let page = parse(&response.body, &j, 1).unwrap();
        assert_eq!(page.messages[0].id, node);
        if response.body["data"]["node"]["__typename"] == "PullRequestReviewComment" {
            let id = response.body["data"]["node"]["fullDatabaseId"]
                .as_str()
                .unwrap();
            let coordinates = t
                .read(
                    ReadOp::Activity(gitjuke_core::activity::ActivityRead {
                        url: format!("repos/{}/pulls/comments/{id}", j.repo),
                        etag: None,
                        modified: None,
                    }),
                    &c,
                )
                .await
                .unwrap();
            let coordinates = gitjuke_core::github::Response::parse(coordinates).unwrap();
            coordinates.require_ok().unwrap();
            let anchor = destination_anchor(&coordinates.body, &j, &page.messages[0]).unwrap();
            println!(
                "live exact reply: body {} bytes; supplied side {}; outdated {}",
                page.messages[0].bytes.unwrap(),
                anchor.side,
                anchor.outdated
            );
        } else {
            println!(
                "live exact {}: body {} bytes",
                page.messages[0].kind,
                page.messages[0].bytes.unwrap()
            );
        }
    }
}

#[tokio::test]
async fn exact_coordinate_denial_and_edit_cannot_publish_mixed_destination() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    retain(&d, "target");
    let j = destination("target");
    let mut v = config();
    v["destination_type"] = json!("PullRequestReviewComment");
    scenario(&d, v.clone());
    let p = read(&r, &j).await;
    assert!(p.destination_anchor.is_some());
    v["conversation_body"] = json!("edited");
    v["coordinate_body"] = json!("different edit");
    scenario(&d, v.clone());
    r.request_conversation(j.clone()).unwrap();
    failure(&r, &j).await;
    assert_eq!(
        r.conversation(&j).unwrap().data.unwrap().messages[0].digest,
        p.messages[0].digest
    );
    v["deny_coordinates"] = json!(true);
    scenario(&d, v);
    r.request_conversation(j.clone()).unwrap();
    failure(&r, &j).await;
    assert!(r.conversation(&j).unwrap().data.is_none());
    let m = &p.messages[0];
    assert!(
        r.conversation_seen(&j, &m.id, m.digest.as_deref().unwrap(), p.generation)
            .is_err()
    );
    assert!(r.overview("a/b", 1).unwrap().data.is_some());
    scenario(&d, config());
    assert_eq!(read(&r, &j).await.messages[0].id, "target");
    r.shutdown().await;
}

#[tokio::test]
async fn local_mixed_identity_fixture_preserves_seen_and_current_range() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    for node in [
        "local-segments",
        "local-anchor",
        "local-oversized",
        "local-neighbor",
    ] {
        retain(&d, node);
    }
    let mut v = config();
    let body = "s".repeat(70571);
    v["files_count"] = json!(1);
    v["destinations"] = json!({
        "local-segments":{"node":{"body":body,"__typename":"IssueComment"}},
        "local-anchor":{"node":{"body":"local anchor","__typename":"PullRequestReviewComment","fullDatabaseId":"103"},"coordinates":{"path":"file-0.txt","side":"RIGHT","line":201,"start_line":199,"start_side":"RIGHT","position":201}},
        "local-oversized":{"node":{"body":"x".repeat(131073),"__typename":"IssueComment"}}
    });
    scenario(&d, v);
    let sql = rusqlite::Connection::open(d.path().join("db")).unwrap();
    let seen = || {
        sql.prepare("SELECT source_id,seen FROM seen ORDER BY source_id")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let segmented_job = destination("local-segments");
    let segmented = read(&r, &segmented_job).await;
    assert_eq!(segmented.messages[0].body.as_deref(), Some(body.as_str()));
    assert!(seen().iter().all(|(_, value)| *value == 0));
    let anchor = read(&r, &destination("local-anchor")).await;
    let coordinates = anchor.destination_anchor.as_ref().unwrap();
    assert_eq!(coordinates.id, "local-anchor");
    assert_eq!(coordinates.path, "file-0.txt");
    assert_eq!(coordinates.commit.as_deref(), Some("a".repeat(40).as_str()));
    assert_eq!(coordinates.line, Some(201));
    assert_eq!(coordinates.start_line, Some(199));
    assert_eq!(coordinates.start_side.as_deref(), Some("RIGHT"));
    assert!(!coordinates.outdated);
    assert!(seen().iter().all(|(_, value)| *value == 0));
    let limited_job = destination("local-oversized");
    let limited = read(&r, &limited_job).await;
    assert_eq!(limited.messages[0].state, "oversized");
    assert!(limited.messages[0].body.is_none());
    assert!(
        r.conversation_seen(&limited_job, "local-oversized", "", limited.generation)
            .is_err()
    );
    r.conversation_seen(
        &segmented_job,
        "local-segments",
        segmented.messages[0].digest.as_deref().unwrap(),
        segmented.generation,
    )
    .unwrap();
    assert_eq!(
        seen()
            .into_iter()
            .filter(|(_, value)| *value == 1)
            .collect::<Vec<_>>(),
        vec![("github.com:a/b:1:local-segments".into(), 1)]
    );
    // One point lookup matched the supplied fullDatabaseId; no source or thread scan.
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    assert!(reads.contains("repos/a/b/pulls/comments/103"));
    assert!(reads.lines().all(|line| {
        serde_json::from_str::<Value>(line).unwrap()["query"] != "query Conversation"
    }));
    r.shutdown().await;
}

#[tokio::test]
async fn explicit_comments_and_thread_replies_validate_context_and_never_retry_writes() {
    let _gate = FIXTURES.lock().await;
    let (d, r) = start().await;
    assert!(
        r.post_conversation(job("comments"), "  ".into(), None)
            .await
            .is_err()
    );
    assert!(
        r.post_conversation(job("comments"), "x".repeat(65537), None)
            .await
            .is_err()
    );
    assert!(
        r.post_conversation(
            job("comments"),
            "hello".into(),
            Some("unknown-thread".into())
        )
        .await
        .is_err()
    );
    let body = "Markdown **comment** with `code`, $HOME and @literal\nSecond line";
    let comment = r
        .post_conversation(job("comments"), body.into(), None)
        .await
        .unwrap();
    assert_eq!(comment.body.as_deref(), Some(body));
    assert_eq!(comment.kind, "comment");
    let threads = read(&r, &job("threads")).await;
    let reply = r
        .post_conversation(
            job("threads"),
            "Thread reply".into(),
            Some(threads.threads[0].id.clone()),
        )
        .await
        .unwrap();
    assert_eq!(reply.kind, "reply");
    let mut settings = config();
    settings["deny_comment"] = json!(true);
    scenario(&d, settings);
    assert!(
        r.post_conversation(job("comments"), "Denied draft".into(), None)
            .await
            .is_err()
    );
    let reads = std::fs::read_to_string(d.path().join("reads")).unwrap();
    let writes: Vec<Value> = reads
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|v: &Value| v["query"] == "mutation")
        .collect();
    assert_eq!(
        writes.len(),
        3,
        "each explicit submission is sent exactly once"
    );
    assert_eq!(writes[0]["variables"]["subject"], "PR_1");
    assert_eq!(writes[0]["variables"]["body"], body);
    assert_eq!(writes[1]["variables"]["subject"], "T_0");
    r.shutdown().await;
}
