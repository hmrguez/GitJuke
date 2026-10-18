//! Explicit local-only fixture preparation. Never linked into the application.
//! Refuses existing databases and every directory except the qualification app's namespace.
use gitjuke_core::{activity::ActivityRecord, model::Identity, store::Store};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: reader_qualification NEW_FIXTURE_DIRECTORY QUALIFICATION_APP_DATA_DIRECTORY"
                .into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let directory = PathBuf::from(&args[1]);
    let accessibility = directory.file_name().and_then(|v| v.to_str())
        == Some("dev.hmrguez.gitjuke.qualification.accessibility");
    if !root.is_absolute()
        || !directory.is_absolute()
        || directory.file_name().and_then(|v| v.to_str())
            != Some("dev.hmrguez.gitjuke.qualification")
            && !accessibility
        || root.exists()
        || directory.exists()
    {
        return Err("Only new absolute fixture directories and the separate qualification app namespace are accepted. Existing data is never overwritten.".into());
    }
    std::fs::create_dir_all(root.parent().ok_or("fixture parent missing")?)?;
    std::fs::create_dir(&root)?;
    std::fs::create_dir(&directory)?;
    let gh = root.join("gh");
    std::fs::write(&gh, include_str!("../tests/support/gh.py"))?;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700))?;
    std::fs::create_dir(root.join("config"))?;
    let segmented = format!(
        "LOCAL SEGMENT 1\n{}LOCAL SEGMENT 2\n{}LOCAL SEGMENT 3\n{}END OF LOCAL BODY",
        "x".repeat(32753),
        "y".repeat(32753),
        "z".repeat(5000)
    );
    let head = "a".repeat(40);
    let mut fixtures = vec![
        (
            "local-short",
            "comment",
            "Short complete body",
            "IssueComment",
            "LOCAL SHORT BODY — no remote data.".to_owned(),
        ),
        (
            "local-segments",
            "comment",
            "Three explicit body segments",
            "IssueComment",
            segmented,
        ),
        (
            "local-anchor",
            "comment",
            "Current RIGHT range 199–201",
            "PullRequestReviewComment",
            "LOCAL CURRENT ANCHOR — exact file-0.txt / RIGHT / 199–201.".to_owned(),
        ),
        (
            "local-oversized",
            "comment",
            "Oversized placeholder stays unseen",
            "IssueComment",
            "x".repeat(131073),
        ),
        (
            "local-empty",
            "review",
            "Complete textless review",
            "PullRequestReview",
            String::new(),
        ),
        (
            "local-neighbor",
            "comment",
            "Unopened neighbor stays unseen",
            "IssueComment",
            "LOCAL NEIGHBOR — opening other sources must leave this unseen.".to_owned(),
        ),
    ];
    if accessibility {
        fixtures.extend([
            (
                "local-unicode-limit",
                "comment",
                "Inclusive 128 KiB UTF-8 with crossing emoji",
                "IssueComment",
                format!("{}😀{}x", "a".repeat(32767), "é".repeat(49150)),
            ),
            (
                "local-left",
                "comment",
                "Current LEFT range 199–201",
                "PullRequestReviewComment",
                "LOCAL LEFT RANGE".into(),
            ),
            (
                "local-wrong-commit",
                "comment",
                "Wrong commit must not attach",
                "PullRequestReviewComment",
                "LOCAL WRONG COMMIT".into(),
            ),
            (
                "local-mixed-side",
                "comment",
                "Mixed side range must not attach",
                "PullRequestReviewComment",
                "LOCAL MIXED SIDE".into(),
            ),
            (
                "local-negative-line",
                "comment",
                "Negative line must not attach",
                "PullRequestReviewComment",
                "LOCAL NEGATIVE COORDINATE".into(),
            ),
            (
                "local-outdated",
                "comment",
                "Outdated source must not attach",
                "PullRequestReviewComment",
                "LOCAL OUTDATED".into(),
            ),
            (
                "local-wrong-file",
                "comment",
                "Different path must not attach",
                "PullRequestReviewComment",
                "LOCAL DIFFERENT FILE".into(),
            ),
        ]);
    }
    let mut destinations = serde_json::Map::new();
    for (i, (id, _, _, kind, body)) in fixtures.iter().enumerate() {
        destinations.insert((*id).to_owned(), json!({"node":{"id":id,"__typename":kind,"body":body,"fullDatabaseId":(101+i).to_string()},"coordinates":{"path":"file-0.txt","side":"RIGHT","line":201,"start_line":199,"start_side":"RIGHT","position":201,"commit_id":head}}));
    }
    if accessibility {
        for (id, field, value) in [
            ("local-left", "side", json!("LEFT")),
            ("local-left", "start_side", json!("LEFT")),
            ("local-left", "commit_id", json!("b".repeat(40))),
            ("local-wrong-commit", "commit_id", json!("c".repeat(40))),
            ("local-mixed-side", "start_side", json!("LEFT")),
            ("local-negative-line", "line", json!(-1)),
            ("local-outdated", "position", json!(null)),
            ("local-outdated", "original_line", json!(201)),
            ("local-wrong-file", "path", json!("other.txt")),
        ] {
            destinations.get_mut(id).unwrap()["coordinates"][field] = value;
        }
    }
    let patch = if accessibility {
        format!(
            "@@ -1,201 +1,201 @@\n{}",
            (1..=201)
                .map(|i| format!("-OLD LOCAL LINE {i}\n+NEW LOCAL LINE {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        format!(
            "@@ -0,0 +1,201 @@\n{}",
            (1..=201)
                .map(|i| format!("+LOCAL FIXTURE LINE {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let scenario = json!({
        "head":head,"no_teams":true,"direct_ids":[],"authored":1,
        "files_count":if accessibility {22} else {1},"patch_status":if accessibility {"modified"} else {"added"},"patch_additions":201,"patch_deletions":if accessibility {201} else {0},
        "patch":patch,
        "conversation_count":if accessibility {55} else {1},"destinations":destinations
    });
    std::fs::write(root.join("scenario.json"), serde_json::to_vec(&scenario)?)?;
    let database = directory.join("gitjuke.sqlite3");
    let mut store = Store::open(&database)?;
    let mut settings = store.settings()?;
    settings.paused = true;
    settings.paged_changes = true;
    settings.gh_path = Some(gh.display().to_string());
    settings.config_dir = Some(root.join("config").display().to_string());
    store.save_settings(&settings)?;
    store.bind(&Identity {
        id: 42,
        login: "reader".into(),
    })?;
    let generation = store.generation()?;
    let now = gitjuke_core::now();
    for (i, (node, kind, label, _, _)) in fixtures.iter().enumerate() {
        let record = ActivityRecord {
            id: format!("github.com:a/b:1:{node}"),
            repo: "a/b".into(),
            number: 1,
            kind: (*kind).into(),
            occurred: now - i as u64,
            actor: Some(format!("LOCAL: {label}")),
            url: "https://github.com/a/b/pull/1".into(),
            text: None,
            eligible: false,
            reason: "controlled local fixture".into(),
            seen: false,
            head: Some(head.clone()),
        };
        store.put(
            &format!("activity-payload:{}", record.id),
            &record,
            generation,
        )?;
    }
    drop(store);
    let mut sql = rusqlite::Connection::open(&database)?;
    let tx = sql.transaction()?;
    for (i, (node, kind, _, _, _)) in fixtures.iter().enumerate() {
        let id = format!("github.com:a/b:1:{node}");
        tx.execute(
            "INSERT INTO activity VALUES(?1,'a/b',1,?2,?3,0,'controlled local fixture',?4)",
            rusqlite::params![id, kind, (now - i as u64) as i64, now as i64],
        )?;
        tx.execute("INSERT INTO seen VALUES(?1,0)", [id])?;
    }
    tx.commit()?;
    println!(
        "{}",
        json!({"fixtureDirectory":root,"database":database,"records":fixtures.len(),"allUnseen":true,"remoteAccess":false})
    );
    Ok(())
}
