//! Atomic schema upgrades with storage admission at every step and commit.
use crate::{
    Result,
    store::{SCHEMA_VERSION, storage_healthy},
};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn upgrade(conn: &mut Connection, version: i64, directory: &Path) -> Result<()> {
    upgrade_checked(conn, version, directory, storage_healthy)
}

fn upgrade_checked(
    conn: &mut Connection,
    version: i64,
    directory: &Path,
    mut check: impl FnMut(&Path) -> Result<()>,
) -> Result<()> {
    // Apply durability before any migration write, including databases that
    // still use rollback journals. Journal-mode conversion remains after commit.
    crate::shared_storage::register(conn)?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction()?;
    #[cfg(test)]
    tests::point(&tx, "before_steps");
    for (target, sql) in STEPS {
        if version < target {
            check(directory)?;
            tx.execute_batch(sql)?;
            #[cfg(test)]
            tests::point(&tx, &format!("after_schema_{target}"));
        }
    }
    // A late low-space signal must roll back every version, not publish an
    // intermediate schema. This remains one transaction, never batched commits.
    check(directory)?;
    #[cfg(test)]
    tests::point(&tx, "before_commit");
    tx.commit()?;
    #[cfg(test)]
    tests::point(conn, "after_commit");
    Ok(())
}

const STEPS: [(i64, &str); 16] = [
    (1, "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                INSERT INTO meta VALUES('revision','0'),('generation','0');
                CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
                CREATE TABLE identity(id INTEGER PRIMARY KEY CHECK(id=1),host TEXT NOT NULL,user_id INTEGER NOT NULL,login TEXT NOT NULL);
                CREATE TABLE content(key TEXT PRIMARY KEY,json TEXT NOT NULL,fetched INTEGER NOT NULL,accessed INTEGER NOT NULL);
                CREATE TABLE resource_state(key TEXT PRIMARY KEY,fetched INTEGER,complete INTEGER NOT NULL DEFAULT 0,error TEXT);
                CREATE TABLE stage(id TEXT PRIMARY KEY,json TEXT NOT NULL);
                CREATE TABLE watch(id TEXT PRIMARY KEY,authored INTEGER NOT NULL DEFAULT 0,requested INTEGER NOT NULL DEFAULT 0,selected INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE seen(source_id TEXT PRIMARY KEY,seen INTEGER NOT NULL);
                CREATE TABLE dedup(source_id TEXT PRIMARY KEY);
                CREATE TABLE progress(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                CREATE TABLE alert_attempt(id TEXT PRIMARY KEY,attempted INTEGER NOT NULL);
                CREATE TABLE due(key TEXT PRIMARY KEY,at INTEGER NOT NULL);
                PRAGMA user_version=1;"),
    (2, "CREATE TABLE watch_locator(id TEXT PRIMARY KEY,repo TEXT NOT NULL,number INTEGER NOT NULL,repo_id INTEGER,state TEXT NOT NULL DEFAULT 'open',review TEXT,head TEXT,merge_sha TEXT,epoch INTEGER NOT NULL DEFAULT 0,due INTEGER NOT NULL DEFAULT 0,last_success INTEGER,error TEXT);
                CREATE TABLE watch_source(id TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(id,source));
                INSERT INTO watch_source SELECT id,'authored' FROM watch WHERE authored=1;
                INSERT INTO watch_source SELECT id,'direct' FROM watch WHERE requested=1;
                INSERT INTO watch_locator(id,repo,number,repo_id) SELECT substr(key,6),json_extract(json,'$.repo'),json_extract(json,'$.number'),json_extract(json,'$.repoId') FROM content WHERE key LIKE 'list:%';
                CREATE TABLE source_stage(source TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(source,id));
                CREATE TABLE teams(id TEXT PRIMARY KEY,slug TEXT NOT NULL);
                CREATE TABLE teams_stage(id TEXT PRIMARY KEY,slug TEXT NOT NULL);
                CREATE TABLE pending(seq INTEGER PRIMARY KEY AUTOINCREMENT,key TEXT NOT NULL UNIQUE,job TEXT NOT NULL,manual INTEGER NOT NULL,foreground INTEGER NOT NULL,at INTEGER NOT NULL,resource TEXT NOT NULL DEFAULT 'core');
                CREATE TABLE check_context(pr_id TEXT NOT NULL,commit_sha TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,staged INTEGER NOT NULL,PRIMARY KEY(pr_id,commit_sha,id,staged));
                CREATE TABLE check_state(pr_id TEXT NOT NULL,commit_sha TEXT NOT NULL,fetched INTEGER,complete INTEGER NOT NULL DEFAULT 0,error TEXT,PRIMARY KEY(pr_id,commit_sha));
                PRAGMA user_version=2;"),
    (3, "CREATE TABLE activity(id TEXT PRIMARY KEY,repo TEXT NOT NULL,number INTEGER NOT NULL,kind TEXT NOT NULL,occurred INTEGER NOT NULL,eligible INTEGER NOT NULL,reason TEXT NOT NULL,created INTEGER NOT NULL);
                CREATE INDEX activity_order ON activity(occurred DESC,id);
                CREATE TABLE activity_stage(repo TEXT NOT NULL,number INTEGER NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(repo,number,id));
                CREATE TABLE activity_target(repo TEXT NOT NULL,number INTEGER NOT NULL,due INTEGER NOT NULL DEFAULT 0,done INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(repo,number));
                CREATE TABLE activity_cycle(id INTEGER PRIMARY KEY CHECK(id=1),mode TEXT NOT NULL,boundary INTEGER NOT NULL,notifications_done INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE activity_intent(id INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT NOT NULL,state TEXT NOT NULL,created INTEGER NOT NULL);
                CREATE TABLE intent_member(intent INTEGER NOT NULL,activity TEXT NOT NULL,PRIMARY KEY(intent,activity));
                CREATE TABLE api_validator(url TEXT PRIMARY KEY,etag TEXT,modified TEXT,next_at INTEGER NOT NULL DEFAULT 0);
                PRAGMA user_version=3;"),
    (4, "ALTER TABLE check_state ADD COLUMN accessed INTEGER NOT NULL DEFAULT 0;
                UPDATE check_state SET accessed=COALESCE(fetched,0);
                PRAGMA user_version=4;"),
    (5, "CREATE TABLE invalidation(id INTEGER PRIMARY KEY CHECK(id=1)); PRAGMA user_version=5;"),
    (6, "CREATE TABLE content_clear(id INTEGER PRIMARY KEY CHECK(id=1)); PRAGMA user_version=6;"),
    (7, "CREATE TABLE check_purge(pr_id TEXT PRIMARY KEY); PRAGMA user_version=7;"),
    (8, "CREATE TABLE pr_purge(repo TEXT COLLATE NOCASE NOT NULL,number INTEGER NOT NULL,generation INTEGER NOT NULL,retry_at INTEGER NOT NULL,PRIMARY KEY(repo,number)); CREATE TABLE source_restart(source TEXT PRIMARY KEY); ALTER TABLE watch_locator ADD COLUMN purged_generation INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=8;"),
    (9, "CREATE TABLE publication(id INTEGER PRIMARY KEY CHECK(id=1),kind TEXT NOT NULL,args TEXT NOT NULL,phase TEXT NOT NULL,step INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE publication_row(seq INTEGER PRIMARY KEY AUTOINCREMENT,tbl TEXT NOT NULL,k TEXT NOT NULL,old_json TEXT,new_json TEXT,reference TEXT,old_bytes INTEGER NOT NULL DEFAULT 0,applied INTEGER NOT NULL DEFAULT 0,UNIQUE(tbl,k));
         PRAGMA user_version=9;"),
    // New journal kinds require Activity-aware protected readers. Older binaries must refuse them.
    (10, "CREATE INDEX content_lru ON content(accessed,key); PRAGMA user_version=10;"),
    (11, "CREATE TABLE control_sweep(id INTEGER PRIMARY KEY CHECK(id=1),kind TEXT NOT NULL,args TEXT NOT NULL); PRAGMA user_version=11;"),
    (12, "CREATE TABLE retention_cursor(class TEXT PRIMARY KEY,position TEXT NOT NULL,ceiling TEXT NOT NULL);
          CREATE INDEX activity_expiry ON activity(occurred,id);
          CREATE INDEX check_expiry ON check_state(accessed,pr_id,commit_sha);
          CREATE INDEX check_published_keys ON check_context(pr_id,commit_sha,id) WHERE staged=0;
          CREATE INDEX publication_unapplied ON publication_row(seq) WHERE applied=0;
          PRAGMA user_version=12;"),
    (13, include_str!("shared_storage.sql")),
    (14, include_str!("publication_windows.sql")),
    (15, "CREATE INDEX activity_intent_dispatch ON activity_intent(state,id); PRAGMA user_version=15;"),
    (16, "CREATE INDEX IF NOT EXISTS activity_intent_dispatch ON activity_intent(state,id); CREATE TABLE IF NOT EXISTS saved_query(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,search TEXT NOT NULL,version INTEGER NOT NULL,limited INTEGER NOT NULL DEFAULT 0); PRAGMA user_version=16;"),
];

#[cfg(test)]
pub(crate) fn upgrade_for_physical_test(conn: &mut Connection, directory: &Path) -> Result<()> {
    upgrade_checked(conn, 0, directory, |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Error,
        model::{Identity, Settings},
        now,
        runtime::Runtime,
        store::Store,
    };
    use rusqlite::{params, types::Value};
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    pub(super) fn point(conn: &Connection, phase: &str) {
        if std::env::var_os("GITJUKE_TEST_MIGRATION_DB").is_some() {
            assert_eq!(
                conn.pragma_query_value::<i64, _>(None, "synchronous", |r| r.get(0))
                    .unwrap(),
                2
            );
            // Force small fixtures to spill pages rather than testing only the
            // process's uncommitted memory. This is a child-test setting only.
            if phase == "before_steps" {
                conn.pragma_update(None, "cache_size", 2).unwrap();
            }
            if phase == "before_commit" {
                // Small DDL-only upgrades can fit in the minimum page cache.
                // Flush dirty pages without committing the migration.
                conn.cache_flush().unwrap();
            }
        }
        if std::env::var("GITJUKE_TEST_MIGRATION_PHASE").as_deref() == Ok(phase) {
            std::fs::write(
                std::env::var_os("GITJUKE_TEST_MIGRATION_MARKER").unwrap(),
                phase,
            )
            .unwrap();
            loop {
                std::thread::park();
            }
        }
    }
    fn integer(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    #[derive(Debug, PartialEq)]
    struct Snapshot {
        version: i64,
        schema: Vec<(String, String, Option<String>)>,
        tables: Vec<(String, Vec<Vec<Value>>)>,
    }
    fn assert_snapshot(actual: &Snapshot, expected: &Snapshot, context: &str) {
        assert_eq!(actual.version, expected.version, "{context}: version");
        assert_eq!(actual.schema, expected.schema, "{context}: schema");
        assert_eq!(
            actual.tables.len(),
            expected.tables.len(),
            "{context}: tables"
        );
        for ((name, rows), (expected_name, expected_rows)) in
            actual.tables.iter().zip(&expected.tables)
        {
            assert_eq!(name, expected_name, "{context}: table identity");
            assert_eq!(
                rows.len(),
                expected_rows.len(),
                "{context}: {name} row count"
            );
            for (n, (row, expected_row)) in rows.iter().zip(expected_rows).enumerate() {
                assert_eq!(row, expected_row, "{context}: {name} row {n}");
            }
        }
    }
    fn snapshot(c: &Connection) -> Snapshot {
        let schema = c
            .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let names = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let tables = names
            .into_iter()
            .map(|name| {
                let mut q = c
                    .prepare(&format!(
                        "SELECT * FROM \"{}\" ORDER BY rowid",
                        name.replace('"', "\"\"")
                    ))
                    .unwrap();
                let columns = q.column_count();
                let rows = q
                    .query_map([], |r| {
                        (0..columns)
                            .map(|n| r.get::<_, Value>(n))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (name, rows)
            })
            .collect();
        Snapshot {
            version: integer(c, "PRAGMA user_version"),
            schema,
            tables,
        }
    }
    fn seed(path: &Path, version: i64, journal: &str) -> Connection {
        let c = Connection::open(path).unwrap();
        crate::shared_storage::register(&c).unwrap();
        c.pragma_update(None, "journal_mode", journal).unwrap();
        for (target, sql) in STEPS {
            if target <= version {
                c.execute_batch(sql).unwrap();
            }
        }
        if version == 0 {
            return c;
        }
        let mut settings = Settings {
            paused: true,
            ..Settings::default()
        };
        settings.theme.accent = "#abcdef".into();
        settings.gh_path = Some("/configured/gh".into());
        settings.config_dir = Some("/configured/config".into());
        c.execute(
            "INSERT INTO settings VALUES(1,?)",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
        c.execute_batch("UPDATE meta SET value='17' WHERE key='revision'; UPDATE meta SET value='9' WHERE key='generation';
            INSERT INTO identity VALUES(1,'github.com',42,'reader');
            INSERT INTO watch VALUES('P',1,1,1);
            INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A');
            INSERT INTO progress VALUES('selection','[\"a/b\",1]'),('durable','17');
            INSERT INTO alert_attempt VALUES('attempt',1);
            INSERT INTO due VALUES('activity:notifications',4070908800),('discovery',99);
            INSERT INTO resource_state VALUES('authored',1,1,NULL);
            INSERT INTO stage VALUES('abandoned','{}');").unwrap();
        let pr = r#"{"id":"P","repo":"a/b","repoId":1,"number":1,"title":"cached","url":"https://github.com/a/b/pull/1","state":"open","draft":false,"author":"reader","updatedAt":"2026-10-17T00:00:00Z"}"#;
        c.execute(
            "INSERT INTO content VALUES('list:P',?,?,?)",
            params![pr, now() as i64, now() as i64],
        )
        .unwrap();
        if version >= 2 {
            c.execute_batch(
                "INSERT INTO watch_locator(id,repo,number,head) VALUES('P','a/b',1,'sha');
                INSERT INTO watch_source VALUES('P','authored'),('P','direct'),('P','selected');
                INSERT INTO source_stage VALUES('authored','abandoned','{}');
                INSERT INTO teams VALUES('1','team');
                INSERT INTO check_context VALUES('P','sha','published','{}',0);",
            )
            .unwrap();
            for n in 0..257 {
                c.execute(
                    "INSERT INTO check_state(pr_id,commit_sha,fetched,complete,error) VALUES('P',?,?,1,NULL)",
                    params![
                        if n == 0 {
                            "sha".into()
                        } else {
                            format!("sha-{n}-{}", "x".repeat(128))
                        },
                        if n % 2 == 0 { Some(123) } else { None }
                    ],
                )
                .unwrap();
            }
        }
        if version >= 4 {
            c.execute("UPDATE check_state SET accessed=COALESCE(fetched,0)", [])
                .unwrap();
        }
        if version >= 3 {
            c.execute_batch("INSERT INTO activity VALUES('A','a/b',1,'comment',1,1,'eligible',1);
                INSERT INTO activity_intent VALUES(1,'pr','attempted',1),(2,'summary','unavailable',1);
                INSERT INTO intent_member VALUES(1,'A'),(2,'A');
                INSERT INTO activity_cycle VALUES(1,'recovery',123,0);
                INSERT INTO progress VALUES('activity:baseline','17'),('catchup:A','A');
                INSERT INTO api_validator VALUES('notifications','etag',NULL,4070908800);").unwrap();
        }
        c
    }
    fn preserved(s: &mut Store, original: i64, revision: u64) {
        assert_eq!(integer(&s.conn, "PRAGMA user_version"), SCHEMA_VERSION);
        if original == 0 {
            assert!(s.identity().unwrap().is_none());
            assert_eq!(s.activity_progress("activity:baseline").unwrap(), 0);
            return;
        }
        assert_eq!(
            s.identity().unwrap(),
            Some(Identity {
                id: 42,
                login: "reader".into()
            })
        );
        assert!(s.settings().unwrap().paused);
        assert_eq!(s.settings().unwrap().theme.accent, "#abcdef");
        assert_eq!(s.revision().unwrap(), revision);
        assert_eq!(s.generation().unwrap(), 9);
        assert_eq!(
            integer(&s.conn, "SELECT seen FROM seen WHERE source_id='A'"),
            0
        );
        assert_eq!(integer(&s.conn, "SELECT COUNT(*) FROM dedup"), 1);
        assert_eq!(integer(&s.conn, "SELECT COUNT(*) FROM alert_attempt"), 1);
        assert_eq!(s.due("activity:notifications").unwrap(), 4070908800);
        assert_eq!(s.selection().unwrap(), Some(("a/b".into(), 1)));
        assert_eq!(
            s.watch_page(0).unwrap().items[0].reasons,
            ["authored", "direct"]
        );
        assert_eq!(s.page(0).unwrap().items[0].title, "cached");
        assert_eq!(integer(&s.conn, "SELECT COUNT(*) FROM stage"), 0);
        if original >= 2 {
            assert_eq!(integer(&s.conn, "SELECT COUNT(*) FROM check_state"), 257);
            assert_eq!(
                integer(
                    &s.conn,
                    "SELECT COUNT(*) FROM check_state WHERE accessed=COALESCE(fetched,0)"
                ),
                257
            );
            assert_eq!(
                integer(&s.conn, "SELECT COUNT(*) FROM check_context WHERE staged=0"),
                1
            );
        }
        if original >= 3 {
            assert_eq!(s.activity_progress("activity:baseline").unwrap(), 17);
            assert_eq!(integer(&s.conn, "SELECT boundary FROM activity_cycle"), 123);
            assert_eq!(
                integer(
                    &s.conn,
                    "SELECT COUNT(*) FROM activity_intent WHERE (id=1 AND state='attempted') OR (id=2 AND state='unavailable')"
                ),
                2
            );
            assert_eq!(integer(&s.conn, "SELECT COUNT(*) FROM intent_member"), 2);
            assert_eq!(
                integer(&s.conn, "SELECT next_at FROM api_validator"),
                4070908800
            );
        }
    }
    fn seed_twelve_accounting(path: &Path, journal: &str) -> Connection {
        let c = seed(path, 12, journal);
        let pr: serde_json::Value = serde_json::from_str(
            &c.query_row("SELECT json FROM content WHERE key='list:P'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        )
        .unwrap();
        let time = now();
        c.execute(
            "UPDATE check_state SET fetched=?1,accessed=?1",
            [time as i64],
        )
        .unwrap();
        let args = serde_json::json!({"time":time,"pr":pr,"changed":false,"locator":["P","a/b",1,1,"open",null,"sha",null,0,time+10,time,null,0]});
        c.execute(
            "INSERT INTO publication(id,kind,args,phase) VALUES(1,'head',?,'planning')",
            [args.to_string()],
        )
        .unwrap();
        c.execute_batch("INSERT INTO publication_row(tbl,k,old_json,new_json,old_bytes) SELECT 'content',json_array(key),json_array(key,json,fetched,accessed),json_array(key,json,fetched,accessed),octet_length(json) FROM content;").unwrap();
        c
    }

    #[test]
    fn schema_twelve_accounting_backfill_is_atomic_on_late_failure_and_resumes_pending_work() {
        for journal in ["DELETE", "WAL"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut c = seed_twelve_accounting(&path, journal);
            let before = snapshot(&c);
            let mut checks = 0;
            assert!(
                upgrade_checked(&mut c, 12, d.path(), |_| {
                    checks += 1;
                    if checks == 2 {
                        Err(Error::StorageUnavailable(
                            "late backfill space fault".into(),
                        ))
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            assert_eq!(snapshot(&c), before);
            upgrade(&mut c, 12, d.path()).unwrap();
            crate::admission_query::tests::assert_exact(&c);
            assert_eq!(integer(&c, "SELECT COUNT(*) FROM shared_check_lru"), 1);
            drop(c);
            let mut s = Store::open(&path).unwrap();
            assert!(!crate::publication::pending(&s.conn).unwrap());
            crate::admission_query::tests::assert_exact(&s.conn);
            preserved(&mut s, 12, 18);
        }
    }

    #[test]
    fn schema_twelve_accounting_sigkill_is_previous_or_complete_and_restart_exact() {
        use std::os::unix::process::ExitStatusExt;
        for journal in ["DELETE", "WAL"] {
            for phase in ["before_commit", "after_commit"] {
                let d = tempfile::tempdir().unwrap();
                let path = d.path().join("db");
                let marker = d.path().join("marker");
                let c = seed_twelve_accounting(&path, journal);
                let before = snapshot(&c);
                drop(c);
                let mut child = OwnedChild(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "migrations::tests::crash_child_entry",
                            "--nocapture",
                        ])
                        .env("GITJUKE_TEST_MIGRATION_DB", &path)
                        .env("GITJUKE_TEST_MIGRATION_PHASE", phase)
                        .env("GITJUKE_TEST_MIGRATION_MARKER", &marker)
                        .stdout(Stdio::null())
                        .spawn()
                        .unwrap(),
                );
                let start = Instant::now();
                while !marker.exists() {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    assert!(child.0.try_wait().unwrap().is_none());
                    std::thread::sleep(Duration::from_millis(5));
                }
                child.0.kill().unwrap();
                assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
                let c = Connection::open(&path).unwrap();
                assert_eq!(
                    c.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                        .unwrap(),
                    "ok"
                );
                if phase == "before_commit" {
                    assert_snapshot(&snapshot(&c), &before, "schema12 backfill kill");
                } else {
                    assert_eq!(integer(&c, "PRAGMA user_version"), SCHEMA_VERSION);
                    crate::admission_query::tests::assert_exact(&c);
                }
                drop(c);
                let mut s = Store::open(&path).unwrap();
                crate::admission_query::tests::assert_exact(&s.conn);
                assert!(!crate::publication::pending(&s.conn).unwrap());
                preserved(&mut s, 12, 18);
            }
        }
    }

    fn seed_thirteen_planning(path: &Path, journal: &str) -> Connection {
        let c = seed_twelve_accounting(path, journal);
        c.execute_batch(include_str!("shared_storage.sql")).unwrap();
        c.execute("UPDATE publication SET step=1", []).unwrap();
        c
    }
    #[test]
    fn schema_thirteen_planning_upgrade_rolls_back_late_failure_and_resets_preparation_safely() {
        for journal in ["DELETE", "WAL"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut c = seed_thirteen_planning(&path, journal);
            let old = snapshot(&c);
            let mut checks = 0;
            assert!(
                upgrade_checked(&mut c, 13, d.path(), |_| {
                    checks += 1;
                    if checks == 2 {
                        Err(Error::StorageUnavailable(
                            "late planning migration space failure".into(),
                        ))
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            assert_eq!(snapshot(&c), old);
            upgrade(&mut c, 13, d.path()).unwrap();
            assert_eq!(integer(&c, "SELECT step FROM publication"), 0);
            assert_eq!(integer(&c, "SELECT COUNT(*) FROM publication_row"), 1);
            assert_eq!(integer(&c, "SELECT contexts FROM shared_check_lru"), 1);
            crate::admission_query::tests::assert_exact(&c);
            drop(c);
            let mut s = Store::open(&path).unwrap();
            assert!(!crate::publication::pending(&s.conn).unwrap());
            crate::admission_query::tests::assert_exact(&s.conn);
            preserved(&mut s, 13, 18);
        }
    }
    #[test]
    fn schema_thirteen_planning_sigkill_keeps_previous_or_complete_upgrade_and_resumes() {
        use std::os::unix::process::ExitStatusExt;
        for journal in ["DELETE", "WAL"] {
            for phase in ["before_commit", "after_commit"] {
                let d = tempfile::tempdir().unwrap();
                let path = d.path().join("db");
                let marker = d.path().join("marker");
                let c = seed_thirteen_planning(&path, journal);
                let old = snapshot(&c);
                drop(c);
                let mut child = OwnedChild(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "migrations::tests::crash_child_entry",
                            "--nocapture",
                        ])
                        .env("GITJUKE_TEST_MIGRATION_DB", &path)
                        .env("GITJUKE_TEST_MIGRATION_PHASE", phase)
                        .env("GITJUKE_TEST_MIGRATION_MARKER", &marker)
                        .stdout(Stdio::null())
                        .spawn()
                        .unwrap(),
                );
                let start = Instant::now();
                while !marker.exists() {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    assert!(child.0.try_wait().unwrap().is_none());
                    std::thread::sleep(Duration::from_millis(5));
                }
                child.0.kill().unwrap();
                assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
                let c = Connection::open(&path).unwrap();
                assert_eq!(
                    c.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                        .unwrap(),
                    "ok"
                );
                if phase == "before_commit" {
                    assert_snapshot(&snapshot(&c), &old, "planning migration");
                } else {
                    assert_eq!(integer(&c, "PRAGMA user_version"), SCHEMA_VERSION);
                    assert_eq!(integer(&c, "SELECT step FROM publication"), 0);
                    crate::admission_query::tests::assert_exact(&c);
                }
                drop(c);
                let mut s = Store::open(&path).unwrap();
                assert!(!crate::publication::pending(&s.conn).unwrap());
                crate::admission_query::tests::assert_exact(&s.conn);
                preserved(&mut s, 13, 18);
            }
        }
    }

    #[test]
    fn alert_dispatch_index_upgrade_preserves_history_and_rolls_back_late_failure() {
        for journal in ["DELETE", "WAL"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut c = seed(&path, 14, journal);
            let before = snapshot(&c);
            let mut calls = 0;
            let failure = upgrade_checked(&mut c, 14, d.path(), |_| {
                calls += 1;
                if calls == 2 {
                    Err(Error::StorageUnavailable("fixture late admission".into()))
                } else {
                    Ok(())
                }
            });
            assert!(failure.is_err());
            assert_eq!(snapshot(&c), before);
            drop(c);
            let mut s = Store::open(&path).unwrap();
            preserved(&mut s, 14, 17);
            assert!(!s.settings().unwrap().alerts.enabled);
            assert_eq!(
                integer(
                    &s.conn,
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='activity_intent_dispatch'"
                ),
                1
            );
        }
    }
    #[test]
    fn late_space_admission_rolls_back_every_schema_step_in_both_journals() {
        for journal in ["DELETE", "WAL"] {
            for version in 0..SCHEMA_VERSION {
                for fail_at in 1..=(SCHEMA_VERSION - version + 1) {
                    let d = tempfile::tempdir().unwrap();
                    let path = d.path().join("db");
                    let mut c = seed(&path, version, journal);
                    let before = snapshot(&c);
                    c.pragma_update(None, "synchronous", "OFF").unwrap();
                    let mut calls = 0;
                    let result = upgrade_checked(&mut c, version, d.path(), |directory| {
                        assert_eq!(directory, d.path());
                        calls += 1;
                        if calls == fail_at {
                            Err(Error::StorageUnavailable(
                                "fixture: space lost during migration".into(),
                            ))
                        } else {
                            Ok(())
                        }
                    });
                    assert!(matches!(result, Err(Error::StorageUnavailable(_))));
                    assert_eq!(calls, fail_at);
                    assert_eq!(integer(&c, "PRAGMA synchronous"), 2);
                    assert_eq!(snapshot(&c), before, "{journal}/{version}/{fail_at}");
                    drop(c);
                    let mut s = Store::open(&path).unwrap();
                    preserved(&mut s, version, 17);
                }
            }
        }
    }
    #[test]
    fn crash_child_entry() {
        let Some(path) = std::env::var_os("GITJUKE_TEST_MIGRATION_DB") else {
            return;
        };
        Store::open(Path::new(&path)).unwrap();
        panic!("child did not reach migration barrier");
    }
    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn sigkill_migrations_preserve_previous_or_complete_schema_and_all_rows() {
        for journal in ["DELETE", "WAL"] {
            for version in 0..SCHEMA_VERSION {
                let phases = std::iter::once("before_steps".to_string())
                    .chain(((version + 1)..=SCHEMA_VERSION).map(|v| format!("after_schema_{v}")))
                    .chain(["before_commit".into(), "after_commit".into()]);
                for phase in phases {
                    let d = tempfile::tempdir().unwrap();
                    let path = d.path().join("db");
                    let marker = d.path().join("barrier");
                    let c = seed(&path, version, journal);
                    let before = snapshot(&c);
                    drop(c);
                    let expected_path = d.path().join("expected");
                    // Copy the exact closed input; independently generated fixtures
                    // can have different fetch timestamps across a second boundary.
                    std::fs::copy(&path, &expected_path).unwrap();
                    let mut expected = Connection::open(&expected_path).unwrap();
                    upgrade(&mut expected, version, d.path()).unwrap();
                    let after = snapshot(&expected);
                    drop(expected);
                    let mut child = OwnedChild(
                        Command::new(std::env::current_exe().unwrap())
                            .args([
                                "--exact",
                                "migrations::tests::crash_child_entry",
                                "--nocapture",
                            ])
                            .env("GITJUKE_TEST_MIGRATION_DB", &path)
                            .env("GITJUKE_TEST_MIGRATION_PHASE", &phase)
                            .env("GITJUKE_TEST_MIGRATION_MARKER", &marker)
                            .stdout(Stdio::null())
                            .spawn()
                            .unwrap(),
                    );
                    let start = Instant::now();
                    while !marker.exists() {
                        assert!(
                            start.elapsed() < Duration::from_secs(10),
                            "{journal}/{version}/{phase}"
                        );
                        assert!(
                            child.0.try_wait().unwrap().is_none(),
                            "child exited before {phase}"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    if phase == "before_commit" {
                        let mut sidecar = path.as_os_str().to_os_string();
                        sidecar.push(if journal == "WAL" { "-wal" } else { "-journal" });
                        assert!(
                            std::fs::metadata(Path::new(&sidecar)).unwrap().len()
                                > if journal == "WAL" { 32 } else { 512 },
                            "{journal}/{version}: migration must reach disk before kill"
                        );
                    }
                    child.0.kill().unwrap();
                    use std::os::unix::process::ExitStatusExt;
                    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
                    let c = Connection::open(&path).unwrap();
                    assert_eq!(
                        c.query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
                            .unwrap(),
                        "ok"
                    );
                    assert_snapshot(
                        &snapshot(&c),
                        if phase == "after_commit" {
                            &after
                        } else {
                            &before
                        },
                        &format!("{journal}/{version}/{phase}"),
                    );
                    drop(c);
                    let mut s = Store::open(&path).unwrap();
                    preserved(&mut s, version, 17);
                }
            }
        }
    }
    #[tokio::test]
    async fn failed_legacy_migration_stops_runtime_then_reopens_paused_after_repair() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let marker = d.path().join("unexpected-read");
        let executable = d.path().join("gh");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\n/usr/bin/touch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let c = seed(&path, 3, "WAL");
        let mut settings: Settings = serde_json::from_str(
            &c.query_row("SELECT json FROM settings", [], |r| r.get::<_, String>(0))
                .unwrap(),
        )
        .unwrap();
        settings.gh_path = Some(executable.display().to_string());
        settings.config_dir = Some(d.path().join("config").display().to_string());
        c.execute(
            "UPDATE settings SET json=?",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
        c.execute_batch("CREATE TRIGGER fail_migration BEFORE UPDATE ON check_state BEGIN SELECT RAISE(ABORT,'fixture migration fault'); END;").unwrap();
        let before = snapshot(&c);
        assert!(Runtime::start(path.clone(), |_| {}).is_err());
        assert_eq!(snapshot(&c), before);
        assert!(!marker.exists());
        c.execute_batch("DROP TRIGGER fail_migration").unwrap();
        drop(c);
        let runtime = Runtime::start(path.clone(), |_| {}).unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(runtime.status().unwrap().settings.paused);
        assert!(runtime.status().unwrap().locked);
        assert!(!marker.exists());
        assert!(
            runtime.page(0).is_err(),
            "migration cannot bypass identity validation"
        );
        runtime.shutdown().await;
        let mut s = Store::open(&path).unwrap();
        preserved(&mut s, 3, 18);
    }
    #[test]
    fn saved_query_schema_16_late_space_failure_preserves_15_and_retries() {
        for journal in ["DELETE", "WAL"] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut c = seed(&path, 15, journal);
            let before = snapshot(&c);
            let mut checks = 0;
            let result = upgrade_checked(&mut c, 15, d.path(), |_| {
                checks += 1;
                if checks == 2 {
                    Err(crate::Error::StorageUnavailable(
                        "injected late space failure".into(),
                    ))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert_snapshot(&snapshot(&c), &before, journal);
            assert_eq!(integer(&c, "PRAGMA user_version"), 15);
            upgrade_checked(&mut c, 15, d.path(), |_| Ok(())).unwrap();
            assert_eq!(integer(&c, "PRAGMA user_version"), 16);
            c.execute(
                "INSERT INTO saved_query(name,search,version) VALUES('Local','repo:a/b',1)",
                [],
            )
            .unwrap();
            drop(c);
            let mut s = Store::open(&path).unwrap();
            assert_eq!(s.saved_queries(0).unwrap().items[0].name, "Local");
            preserved(&mut s, 15, 17);
        }
    }
    #[test]
    fn both_independent_schema_15_variants_upgrade_without_losing_configuration() {
        for query_branch in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("db");
            let mut c = seed(&path, 14, "WAL");
            if query_branch {
                c.execute_batch("CREATE TABLE saved_query(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,search TEXT NOT NULL,version INTEGER NOT NULL,limited INTEGER NOT NULL DEFAULT 0); INSERT INTO saved_query(name,search,version) VALUES('Keep me','repo:a/b',3); PRAGMA user_version=15;").unwrap();
            } else {
                c.execute_batch("CREATE INDEX activity_intent_dispatch ON activity_intent(state,id); PRAGMA user_version=15;").unwrap();
            }
            upgrade_checked(&mut c, 15, d.path(), |_| Ok(())).unwrap();
            assert_eq!(integer(&c, "PRAGMA user_version"), 16);
            assert_eq!(
                integer(
                    &c,
                    "SELECT count(*) FROM sqlite_schema WHERE name='activity_intent_dispatch'"
                ),
                1
            );
            assert_eq!(
                integer(&c, "SELECT count(*) FROM saved_query"),
                i64::from(query_branch)
            );
            if query_branch {
                let row: (String, String, i64) = c
                    .query_row("SELECT name,search,version FROM saved_query", [], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .unwrap();
                assert_eq!(row, ("Keep me".into(), "repo:a/b".into(), 3));
            }
            drop(c);
            let mut store = Store::open(&path).unwrap();
            preserved(&mut store, 14, 17);
        }
    }
}
