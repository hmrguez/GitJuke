//! Creates a fresh isolated native query qualification environment, never production data.
use gitjuke_core::store::Store;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: query_qualification NEW_FIXTURE_ROOT QUERY_QUALIFICATION_APP_DATA".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let data = PathBuf::from(&args[1]);
    if !root.is_absolute()
        || !data.is_absolute()
        || root.exists()
        || data.exists()
        || data.file_name().and_then(|s| s.to_str())
            != Some("dev.hmrguez.gitjuke.queryqualification")
    {
        return Err("Refusing existing data or non-qualification namespace".into());
    }
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(&data)?;
    std::fs::create_dir(root.join("config"))?;
    let gh = root.join("gh");
    std::fs::write(&gh, include_str!("../tests/support/gh.py"))?;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(
        root.join("scenario.json"),
        serde_json::to_vec(
            &serde_json::json!({"authored":0,"no_teams":true,"direct_ids":[],"query_ids":(1..=150).collect::<Vec<_>>(),"head":"a".repeat(40)}),
        )?,
    )?;
    let mut store = Store::open(&data.join("gitjuke.sqlite3"))?;
    let mut s = store.settings()?;
    s.paused = true;
    s.gh_path = Some(gh.display().to_string());
    s.config_dir = Some(root.join("config").display().to_string());
    store.save_settings(&s)?;
    println!("Prepared isolated synthetic source; no query is pre-created.");
    Ok(())
}
