use gitjuke_core::{model::*, store::Store};
use tempfile::tempdir;
fn identity() -> Identity {
    Identity {
        id: 42,
        login: "reader".into(),
    }
}
fn pr(n: u64) -> PullRequest {
    PullRequest {
        id: format!("PR_{n}"),
        repo_id: Some(1),
        repo: "a/b".into(),
        number: n,
        title: format!("Change {n}"),
        url: format!("https://github.com/a/b/pull/{n}"),
        state: "open".into(),
        draft: false,
        author: Some("reader".into()),
        updated_at: "2026-10-17T00:00:00Z".into(),
    }
}
#[test]
fn stable_identity_survives_restart_and_refuses_different_user() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    {
        let mut s = Store::open(&path).unwrap();
        s.bind(&identity()).unwrap();
        let mut settings = s.settings().unwrap();
        settings.paused = true;
        s.save_settings(&settings).unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert!(s.settings().unwrap().paused);
    assert_eq!(s.identity().unwrap(), Some(identity()));
    assert!(
        s.bind(&Identity {
            id: 99,
            login: "other".into()
        })
        .is_err()
    );
    assert_eq!(s.identity().unwrap(), Some(identity()));
    // A renamed login with the same stable ID is permitted.
    s.bind(&Identity {
        id: 42,
        login: "renamed".into(),
    })
    .unwrap();
    assert_eq!(s.identity().unwrap().unwrap().login, "renamed");
}
#[test]
fn clear_and_denial_cannot_resurrect_pending_content() {
    let d = tempdir().unwrap();
    let mut s = Store::open(&d.path().join("db")).unwrap();
    s.bind(&identity()).unwrap();
    let generation = s.generation().unwrap();
    assert!(s.put("pr:a/b:1", &"old", generation).unwrap());
    s.clear().unwrap();
    assert!(!s.put("pr:a/b:1", &"late", generation).unwrap());
    assert!(s.get::<String>("pr:a/b:1").unwrap().data.is_none());
    let generation = s.generation().unwrap();
    s.put("pr:a/b:1", &"current", generation).unwrap();
    s.purge("pr:a/b:1").unwrap();
    assert!(!s.put("pr:a/b:1", &"late", generation).unwrap());
    let v = s.get::<String>("pr:a/b:1").unwrap();
    assert!(v.data.is_none());
    assert_eq!(v.error.as_deref(), Some("Resource unavailable"));
}
#[test]
fn only_complete_traversal_replaces_membership_and_100_item_pages() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let g = s.generation().unwrap();
    s.start_authored().unwrap();
    s.stage_authored(&(1..=150).map(pr).collect::<Vec<_>>(), g)
        .unwrap();
    assert!(s.page(0).unwrap().items.is_empty());
    s.finish_authored(g).unwrap();
    let a = s.page(0).unwrap();
    assert_eq!(a.items.len(), 100);
    assert_eq!(a.next_offset, Some(100));
    assert!(a.complete);
    assert_eq!(s.page(100).unwrap().items.len(), 50);
    s.start_authored().unwrap();
    s.stage_authored(&[pr(999)], g).unwrap();
    s.error("authored", "offline").unwrap();
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.page(0).unwrap().items.len(), 100);
    assert_eq!(s.page(0).unwrap().error.as_deref(), Some("offline"));
    let c = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM watch WHERE authored=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        150
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM stage", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn future_and_corrupt_databases_are_preserved() {
    let d = tempdir().unwrap();
    let path = d.path().join("future");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 999).unwrap();
    drop(c);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
    let path = d.path().join("corrupt");
    std::fs::write(&path, b"preserve this invalid database").unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
}
#[test]
fn eviction_preserves_durable_ledgers_and_empty_failure_is_not_success() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.bind(&identity()).unwrap();
    s.error("authored", "offline").unwrap();
    let p = s.page(0).unwrap();
    assert!(!p.complete);
    assert_eq!(p.fetched_at, None);
    assert!(p.error.is_some());
    let c = rusqlite::Connection::open(&path).unwrap();
    for table in ["seen", "dedup", "progress", "alert_attempt"] {
        match table {
            "seen" => c.execute("INSERT INTO seen VALUES('source',1)", []),
            "dedup" => c.execute("INSERT INTO dedup VALUES('source')", []),
            "progress" => c.execute("INSERT INTO progress VALUES('source','done')", []),
            _ => c.execute("INSERT INTO alert_attempt VALUES('source',1)", []),
        }
        .unwrap();
    }
    s.clear().unwrap();
    for table in ["seen", "dedup", "progress", "alert_attempt"] {
        assert_eq!(
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn checkpoint_reports_reader_blocked_wal_then_recovers_without_losing_state() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.bind(&identity()).unwrap();
    s.checkpoint().unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN; SELECT * FROM meta;").unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    assert!(
        s.checkpoint().is_err(),
        "PASSIVE can succeed as SQL while leaving WAL frames uncheckpointed"
    );
    assert!(s.settings().unwrap().paused);
    reader.execute_batch("ROLLBACK").unwrap();
    s.checkpoint().unwrap();
    drop(s);
    assert!(Store::open(&path).unwrap().settings().unwrap().paused);
}

#[test]
fn multi_version_migration_failure_rolls_back_to_original_committed_schema() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.bind(&identity()).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    drop(s);
    let c = rusqlite::Connection::open(&path).unwrap();
    remove_shared_schema(&c);
    // Reconstruct schema 1, with a collision reached only after the schema-2
    // steps have executed. A failed later step must roll back both versions.
    c.execute_batch(
        "DROP TABLE watch_locator; DROP TABLE watch_source;
        DROP TABLE source_stage; DROP TABLE teams; DROP TABLE teams_stage;
        DROP TABLE pending; DROP TABLE check_context; DROP TABLE check_state;
        DROP TABLE activity_stage; DROP TABLE activity_target;
        DROP TABLE activity_cycle; DROP TABLE activity_intent;
        DROP TABLE intent_member; DROP TABLE api_validator; DROP TABLE invalidation; DROP TABLE content_clear; DROP TABLE check_purge; DROP TABLE pr_purge; DROP TABLE source_restart; DROP TABLE publication_row; DROP TABLE publication; DROP TABLE retention_cursor; DROP INDEX IF EXISTS activity_expiry; DROP INDEX IF EXISTS check_expiry; DROP INDEX IF EXISTS check_published_keys; DROP INDEX IF EXISTS publication_unapplied; DROP TABLE control_sweep; DROP INDEX content_lru; DROP INDEX IF EXISTS check_staged_keys; DROP INDEX IF EXISTS watch_source_source; DROP INDEX IF EXISTS watch_source_source_nocase; DROP INDEX IF EXISTS watch_source_reason; DROP INDEX IF EXISTS watch_locator_location; DROP INDEX IF EXISTS pending_key_nocase;
        PRAGMA user_version=1;",
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='watch_locator'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT user_id FROM identity", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
    assert!(
        c.query_row(
            "SELECT json_extract(json,'$.paused') FROM settings",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    c.execute_batch("DROP TABLE activity").unwrap();
    let migrated = Store::open(&path).unwrap();
    assert!(migrated.settings().unwrap().paused);
    assert_eq!(migrated.identity().unwrap(), Some(identity()));
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        gitjuke_core::store::SCHEMA_VERSION
    );
}

#[test]
fn schema_three_check_access_upgrade_preserves_history_and_rolls_back_failure() {
    let d = tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.bind(&identity()).unwrap();
    let mut settings = s.settings().unwrap();
    settings.paused = true;
    s.save_settings(&settings).unwrap();
    drop(s);
    let c = rusqlite::Connection::open(&path).unwrap();
    remove_shared_schema(&c);
    c.execute_batch("DROP TABLE invalidation; DROP TABLE content_clear; DROP TABLE check_purge; DROP TABLE pr_purge; DROP TABLE source_restart; DROP TABLE publication_row; DROP TABLE publication; DROP TABLE retention_cursor; DROP INDEX IF EXISTS activity_expiry; DROP INDEX IF EXISTS check_expiry; DROP INDEX IF EXISTS check_published_keys; DROP INDEX IF EXISTS publication_unapplied; DROP TABLE control_sweep; DROP INDEX content_lru; DROP INDEX IF EXISTS check_staged_keys; DROP INDEX IF EXISTS watch_source_source; DROP INDEX IF EXISTS watch_source_source_nocase; DROP INDEX IF EXISTS watch_source_reason; DROP INDEX IF EXISTS watch_locator_location; DROP INDEX IF EXISTS pending_key_nocase; ALTER TABLE check_state DROP COLUMN accessed; ALTER TABLE watch_locator DROP COLUMN purged_generation;
        INSERT INTO check_state VALUES('P','sha',123,1,NULL);
        PRAGMA user_version=3;
        CREATE TRIGGER migration_fault BEFORE UPDATE ON check_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        c.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        3
    );
    assert!(c.prepare("SELECT accessed FROM check_state").is_err());
    assert_eq!(
        c.query_row("SELECT fetched FROM check_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        123
    );
    c.execute_batch("DROP TRIGGER migration_fault").unwrap();
    let s = Store::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT accessed FROM check_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        123
    );
    assert!(s.settings().unwrap().paused);
    assert_eq!(s.identity().unwrap(), Some(identity()));
}

fn remove_shared_schema(c: &rusqlite::Connection) {
    c.execute_batch("DROP INDEX IF EXISTS activity_intent_dispatch")
        .unwrap();
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
}
