//! Explicit local-only native log qualification; never linked into the app.
use gitjuke_core::store::Store;
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: log_qualification NEW_FIXTURE_DIR NEW_LOG_QUALIFICATION_APP_DATA_DIR".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let data = PathBuf::from(&args[1]);
    if !root.is_absolute()
        || !data.is_absolute()
        || root.exists()
        || data.exists()
        || data.file_name().and_then(|s| s.to_str()) != Some("dev.hmrguez.gitjuke.logqualification")
    {
        return Err("Only new absolute directories and the isolated log qualification namespace are permitted.".into());
    }
    std::fs::create_dir(&root)?;
    std::fs::create_dir(&data)?;
    std::fs::create_dir(root.join("config"))?;
    let gh = root.join("gh");
    std::fs::write(&gh, include_str!("../tests/support/gh.py"))?;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(
        root.join("scenario.json"),
        serde_json::to_vec(
            &json!({"head":"a".repeat(40),"authored":1,"no_teams":true,"direct_ids":[],"actions_logs":true,"log_text":"LOCAL SYNTHETIC ACTIONS LOG\nNo remote data or credentials.\n<untrusted> stays plain text.\nEnd of supplied job output."}),
        )?,
    )?;
    let mut s = Store::open(&data.join("gitjuke.sqlite3"))?;
    let mut settings = s.settings()?;
    settings.paused = true;
    settings.gh_path = Some(gh.display().to_string());
    settings.config_dir = Some(root.join("config").display().to_string());
    s.save_settings(&settings)?;
    println!("Prepared isolated paused log qualification app; use Refresh and open a/b #1.");
    Ok(())
}
