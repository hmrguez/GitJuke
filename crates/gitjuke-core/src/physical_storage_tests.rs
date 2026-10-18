//! Opt-in tests on an explicitly mounted disposable volume. No app database paths.
use crate::{Error, store::Store};
use rusqlite::Connection;
use std::{io::Write, path::PathBuf};
fn volume() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("GITJUKE_TEST_FAULT_VOLUME").expect("explicit disposable mount required"),
    );
    assert_eq!(
        path,
        PathBuf::from("/private/tmp/gitjuke-storage-fault-volume")
    );
    assert!(path.join("gitjuke-disposable-volume").is_file());
    use std::os::unix::fs::MetadataExt;
    assert_ne!(
        std::fs::metadata(&path).unwrap().dev(),
        std::fs::metadata(path.parent().unwrap()).unwrap().dev(),
        "refuse a directory on the host filesystem"
    );
    let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    assert_eq!(
        unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) },
        0
    );
    let stat = unsafe { stat.assume_init() };
    let capacity = u64::from(stat.f_blocks).saturating_mul(stat.f_frsize);
    assert!(
        (32 * 1024 * 1024..=128 * 1024 * 1024).contains(&capacity),
        "refuse a volume outside the disposable size bound"
    );
    path
}
#[tokio::test]
#[ignore = "requires a disposable mounted 64-MiB fault volume"]
async fn actual_volume_exhaustion_rolls_back_and_preserves_history() {
    let directory = volume();
    let path = directory.join("physical.sqlite3");
    assert!(!path.exists(), "refuse to overwrite any input");
    let mut conn = Connection::open(&path).unwrap();
    // This deliberately tiny volume cannot pass the production 1-GiB floor.
    // Build the exact current schema using the migration engine in this test only.
    crate::migrations::upgrade_for_physical_test(&mut conn, &directory).unwrap();
    let mut s = Store {
        conn,
        cache_writes: true,
    };
    s.conn.execute_batch("INSERT INTO identity VALUES(1,'github.com',42,'reader'); INSERT INTO settings VALUES(1,'{\"paused\":true}'); INSERT INTO seen VALUES('A',0); INSERT INTO dedup VALUES('A'); INSERT INTO alert_attempt VALUES('attempt',1); INSERT INTO activity_intent VALUES(1,'pr','attempted',1); INSERT INTO intent_member VALUES(1,'A'); INSERT INTO due VALUES('activity:notifications',4070908800);").unwrap();
    let settings = crate::model::Settings {
        paused: true,
        ..Default::default()
    };
    s.save_settings(&settings).unwrap();
    let generation = s.generation().unwrap();
    s.conn
        .execute(
            "INSERT INTO content VALUES('old','\"original\"',?1,?1)",
            [crate::now() as i64],
        )
        .unwrap();
    let revision = s.revision().unwrap();
    let filler = directory.join("filler");
    let mut f = std::fs::File::create(&filler).unwrap();
    let block = [0xa5; 65536];
    let mut exhausted = false;
    // A second independent bound prevents accidental host-volume exhaustion.
    for _ in 0..2048 {
        if let Err(e) = f.write_all(&block) {
            assert_eq!(e.raw_os_error(), Some(libc::ENOSPC));
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "fault volume did not exhaust within the 128-MiB fixture cap"
    );
    let _ = f.sync_all();
    drop(f);
    assert!(matches!(
        s.put("new", &"x".repeat(900 * 1024), generation),
        Err(Error::StorageUnavailable(_))
    ));
    // Deliberately bypass the free-space gate to qualify SQLite on actual ENOSPC.
    let error = s
        .conn
        .execute(
            "INSERT INTO content VALUES('new',?1,1,1)",
            ["x".repeat(900 * 1024)],
        )
        .unwrap_err();
    assert!(
        matches!(error, rusqlite::Error::SqliteFailure(e,_) if e.code == rusqlite::ErrorCode::DiskFull),
        "{error:?}"
    );
    assert_eq!(s.revision().unwrap(), revision);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM content WHERE key='new'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let runtime = crate::runtime::Runtime::start(path.clone(), |_| {}).unwrap();
    assert!(runtime.status().unwrap().storage_stopped);
    assert!(runtime.status().unwrap().locked);
    assert!(runtime.status().unwrap().settings.paused);
    runtime.shutdown().await;
    std::fs::remove_file(filler).unwrap();
    crate::admission_query::tests::assert_exact(&s.conn);
    s.conn
        .execute("INSERT INTO content VALUES('new','\"repaired\"',1,1)", [])
        .unwrap();
    assert_eq!(
        s.conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        s.conn
            .query_row("SELECT user_id FROM identity", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
    assert!(s.settings().unwrap().paused);
    assert_eq!(
        s.conn
            .query_row("SELECT seen FROM seen WHERE source_id='A'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM dedup", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        s.conn
            .query_row("SELECT state FROM activity_intent", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "attempted"
    );
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM intent_member", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        s.conn
            .query_row("SELECT attempted FROM alert_attempt", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        s.conn
            .query_row("SELECT at FROM due", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4070908800
    );
}
#[tokio::test]
#[ignore = "requires the disposable fault volume remounted read-only"]
async fn actual_readonly_volume_rejects_writes_and_keeps_cache_readable() {
    let path = volume().join("physical.sqlite3");
    assert!(path.is_file());
    let c = Connection::open(&path).unwrap();
    let error = c
        .execute("INSERT INTO dedup VALUES('must-not-commit')", [])
        .unwrap_err();
    assert!(
        matches!(error, rusqlite::Error::SqliteFailure(e,_) if e.code == rusqlite::ErrorCode::ReadOnly),
        "{error:?}"
    );
    let mut s = Store::open_cached(&path).unwrap();
    assert_eq!(
        s.get::<String>("old").unwrap().data.as_deref(),
        Some("original")
    );
    assert_eq!(s.identity().unwrap().unwrap().id, 42);
    assert!(s.settings().unwrap().paused);
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM dedup", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
