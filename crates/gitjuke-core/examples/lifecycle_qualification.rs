//! Local-only native qualification preparation, never used by the production runtime.
use gitjuke_core::store::Store;
fn main() {
    let directory = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("isolated app-data directory"),
    );
    assert_eq!(
        directory.file_name().unwrap(),
        "dev.hmrguez.gitjuke.lifecycle-qualification"
    );
    assert!(
        !directory.exists(),
        "Qualification data already exists; stop and archive it explicitly before a fresh preparation."
    );
    std::fs::create_dir_all(&directory).unwrap();
    let mut store = Store::open(&directory.join("gitjuke.sqlite3")).unwrap();
    let mut settings = store.settings().unwrap();
    settings.paused = true;
    settings.gh_path = Some("/private/tmp/gitjuke-lifecycle-no-gh".into());
    settings.config_dir = Some(
        directory
            .join("no-credentials")
            .to_string_lossy()
            .into_owned(),
    );
    store.save_settings(&settings).unwrap();
    // Preference-only synthetic setup: no LaunchAgent is registered by this tool.
    std::fs::write(directory.join("login.json"), br#"{"enabled":false}"#).unwrap();
    println!("Prepared isolated paused lifecycle state; no gh or remote reads.");
}
