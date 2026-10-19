//! Explicit opt-in only. Status/startup never writes or repairs a LaunchAgent.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

pub const NORMAL_ID: &str = "dev.hmrguez.gitjuke";
pub fn installed_bundle(identifier: &str) -> PathBuf {
    #[cfg(all(debug_assertions, feature = "lifecycle-qualification"))]
    if identifier == "dev.hmrguez.gitjuke.lifecycle-qualification" {
        return "/private/tmp/GitJuke Lifecycle.app".into();
    }
    let _ = identifier;
    "/Applications/GitJuke.app".into()
}
pub fn valid_install(executable: &Path, bundle: &Path, identifier: &str) -> bool {
    let expected = bundle.join("Contents/MacOS/gitjuke");
    executable == expected
        && executable.canonicalize().ok().as_ref() == Some(&expected)
        && bundle.canonicalize().ok().as_deref() == Some(bundle)
        && plist::Value::from_file(bundle.join("Contents/Info.plist"))
            .ok()
            .and_then(|v| {
                v.as_dictionary()?
                    .get("CFBundleIdentifier")?
                    .as_string()
                    .map(str::to_owned)
            })
            .as_deref()
            == Some(identifier)
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    enabled: bool,
}
pub struct Login {
    preference: PathBuf,
    operation: Mutex<()>,
    stopping: AtomicBool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    pub desired: bool,
    pub installed: bool,
    pub registered: bool,
    pub registration_valid: bool,
    pub os_disabled: Option<bool>,
    pub state: String,
    pub message: String,
}
impl Login {
    pub fn new(directory: &Path) -> Self {
        Self {
            preference: directory.join("login.json"),
            operation: Mutex::new(()),
            stopping: AtomicBool::new(false),
        }
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }
    pub fn drain(&self) {
        drop(self.operation.lock());
    }
    fn running(&self) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            Err("GitJuke is quitting; retry after a manual launch.".into())
        } else {
            Ok(())
        }
    }
    pub fn desired(&self) -> Result<bool, String> {
        Ok(self.preference()?.enabled)
    }
    fn preference(&self) -> Result<Preference, String> {
        use std::io::Read;
        let bytes = std::fs::File::open(&self.preference).and_then(|f| {
            let mut bytes = Vec::new();
            f.take(4097).read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        match bytes {
            Ok(bytes) if bytes.len() <= 4096 => serde_json::from_slice(&bytes)
                .map_err(|_| "Login preference is unreadable. Existing file was preserved.".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preference::default()),
            _ => Err("Login preference could not be read. Existing file was preserved.".into()),
        }
    }
    fn save(&self, enabled: bool) -> Result<(), String> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let temp = self.preference.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(|_| "Could not save the login preference.")?;
        file.write_all(&serde_json::to_vec(&Preference { enabled }).unwrap())
            .and_then(|_| file.sync_all())
            .and_then(|_| std::fs::rename(&temp, &self.preference))
            .and_then(|_| std::fs::File::open(self.preference.parent().unwrap())?.sync_all())
            .map_err(|_| "Could not commit the login preference.".into())
    }
    fn paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf, String), String> {
        let id = app.config().identifier.clone();
        let home = app
            .path()
            .home_dir()
            .map_err(|_| "OS account home is unavailable.")?;
        Ok((
            home.join("Library/LaunchAgents")
                .join(format!("{id}.plist")),
            installed_bundle(&id),
            id,
        ))
    }
    pub fn status(&self, app: &tauri::AppHandle) -> Result<LoginStatus, String> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| "Login controls are unavailable.")?;
        self.running()?;
        self.inspect(app)
    }
    fn inspect(&self, app: &tauri::AppHandle) -> Result<LoginStatus, String> {
        let desired = self.preference()?.enabled;
        let (path, bundle, id) = Self::paths(app)?;
        let installed = std::env::current_exe()
            .ok()
            .is_some_and(|p| valid_install(&p, &bundle, &id));
        let registered = path
            .try_exists()
            .map_err(|_| "Login registration could not be inspected.")?;
        let registration_valid = registered
            && valid_registration(&path, &bundle, &id)
            && valid_install(&bundle.join("Contents/MacOS/gitjuke"), &bundle, &id);
        let os_disabled = launchctl(&["print-disabled", &domain()])
            .ok()
            .and_then(|s| disabled(&s, &id));
        let (state, message) = classify(
            desired,
            installed,
            registered,
            registration_valid,
            os_disabled,
        );
        Ok(LoginStatus {
            desired,
            installed,
            registered,
            registration_valid,
            os_disabled,
            state: state.into(),
            message: message.into(),
        })
    }
    pub fn change(
        &self,
        app: &tauri::AppHandle,
        enabled: bool,
        repair: bool,
    ) -> Result<LoginStatus, String> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| "Login controls are unavailable.")?;
        self.running()?;
        let (path, bundle, id) = Self::paths(app)?;
        let preference = self.preference()?;
        if repair && !preference.enabled {
            return Err("Enable launch at login before repairing registration.".into());
        }
        if enabled
            && !std::env::current_exe()
                .ok()
                .is_some_and(|p| valid_install(&p, &bundle, &id))
        {
            return Err("Install GitJuke in /Applications/GitJuke.app, quit other copies, then explicitly enable or repair launch at login.".into());
        }
        // Persist intent before OS mutation. A partial failure remains visible and requires repair.
        self.save(enabled)?;
        if enabled {
            if path
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(
                    "Login registration is a symbolic link. Remove it explicitly before repair."
                        .into(),
                );
            }
            app.autolaunch().enable().map_err(|_|"LaunchAgent registration failed. Your choice is saved; retry Repair registration.".to_owned())?;
            launchctl(&["enable", &format!("{}/{id}", domain())])?;
        } else {
            // Disable the loaded label without bootout/termination of this monitor.
            launchctl(&["disable", &format!("{}/{id}", domain())])?;
            app.autolaunch().disable().map_err(|_|"Could not remove the LaunchAgent. Launch at login is disabled locally; retry Disable.".to_owned())?;
        }
        let status = self.inspect(app)?;
        if enabled && (!status.registration_valid || status.os_disabled != Some(false)) {
            return Err(format!("Registration needs attention: {}", status.message));
        }
        Ok(status)
    }
}
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}
fn classify(
    desired: bool,
    installed: bool,
    registered: bool,
    valid: bool,
    disabled: Option<bool>,
) -> (&'static str, &'static str) {
    if !installed {
        (
            "installRequired",
            "Install GitJuke in /Applications/GitJuke.app and quit other copies. This copy cannot monitor or register at login.",
        )
    } else if !desired {
        if registered {
            (
                "unexpectedRegistration",
                "Launch at login is off, but a registration remains. Use Disable to remove it.",
            )
        } else {
            ("off", "Launch at login is off.")
        }
    } else if !registered || !valid {
        (
            "repairRequired",
            "Login registration is missing or targets an invalid path. Repair explicitly to restore next-login launch.",
        )
    } else if disabled == Some(true) {
        (
            "osDisabled",
            "macOS has disabled this registration. Monitoring continues; repair only when you want to enable it again.",
        )
    } else if disabled.is_none() {
        (
            "unknown",
            "Registration exists, but macOS status could not be verified. Check Login Items in System Settings.",
        )
    } else {
        (
            "registered",
            "Registered for next login. macOS Login Items controls may still prevent launch. Quit stops this monitor until a manual launch or next login.",
        )
    }
}
fn valid_registration(path: &Path, bundle: &Path, id: &str) -> bool {
    if path
        .symlink_metadata()
        .map_or(true, |m| !m.is_file() || m.len() > 16384)
    {
        return false;
    }
    let Ok(value) = plist::Value::from_file(path) else {
        return false;
    };
    let Some(d) = value.as_dictionary() else {
        return false;
    };
    let expected = vec![
        plist::Value::String(
            bundle
                .join("Contents/MacOS/gitjuke")
                .to_string_lossy()
                .into_owned(),
        ),
        plist::Value::String("--background".into()),
    ];
    d.keys().all(|k| {
        matches!(
            k.as_str(),
            "Label" | "AssociatedBundleIdentifiers" | "ProgramArguments" | "RunAtLoad"
        )
    }) && d.get("Label").and_then(plist::Value::as_string) == Some(id)
        && d.get("ProgramArguments").and_then(plist::Value::as_array) == Some(&expected)
        && d.get("RunAtLoad").and_then(plist::Value::as_boolean) == Some(true)
        && !d.contains_key("KeepAlive")
        && !d.contains_key("StartInterval")
        && !d.contains_key("StartCalendarInterval")
        && !d.contains_key("WatchPaths")
        && !d.contains_key("QueueDirectories")
}
fn disabled(text: &str, id: &str) -> Option<bool> {
    let text = text.trim();
    if !text.starts_with("disabled services = {") || !text.ends_with('}') {
        return None;
    }
    let key = format!("\"{id}\" =>");
    match text.lines().find_map(|line| line.trim().strip_prefix(&key)) {
        Some(v) if matches!(v.trim(), "true" | "disabled") => Some(true),
        Some(v) if matches!(v.trim(), "false" | "enabled") => Some(false),
        Some(_) => None,
        None => Some(false),
    }
}
// Local system helper: fixed arguments, no shell, 2-second deadline and 64-KiB cap.
fn launchctl(args: &[&str]) -> Result<String, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "macOS login controls could not start.")?;
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(65537).read_to_end(&mut bytes).map(|_| bytes)
    });
    let until = Instant::now() + Duration::from_secs(2);
    let success = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader
        .join()
        .ok()
        .and_then(Result::ok)
        .ok_or("macOS login status could not be read.")?;
    if !success || bytes.len() > 65536 {
        return Err("macOS login control failed or timed out. Your choice is saved; check Login Items and explicitly repair.".into());
    }
    String::from_utf8(bytes).map_err(|_| "macOS login status was not recognized.".into())
}
#[tauri::command]
pub async fn login_status(app: tauri::AppHandle) -> Result<LoginStatus, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Login>().status(&app))
        .await
        .map_err(|_| "Login status unavailable.".to_owned())?
}
#[tauri::command]
pub async fn set_launch_at_login(
    app: tauri::AppHandle,
    enabled: bool,
    repair: bool,
) -> Result<LoginStatus, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Login>().change(&app, enabled, repair))
        .await
        .map_err(|_| "Login operation unavailable.".to_owned())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_disabled_requires_exact_label_and_valid_output() {
        assert_eq!(
            disabled("disabled services = {\n \"a.b\" => true\n}", "a.b"),
            Some(true)
        );
        assert_eq!(
            disabled("disabled services = {\n \"a.b.extra\" => true\n}", "a.b"),
            Some(false)
        );
        assert_eq!(
            disabled("disabled services = {\n \"a.b\" => disabled\n}", "a.b"),
            Some(true)
        );
        assert_eq!(
            disabled("disabled services = {\n \"a.b\" => enabled\n}", "a.b"),
            Some(false)
        );
        assert_eq!(disabled("permission denied", "a.b"), None);
        assert_eq!(
            disabled("disabled services = {\n \"a.b\" => maybe\n}", "a.b"),
            None
        );
    }
    #[test]
    fn status_never_confuses_desire_with_registration() {
        assert_eq!(classify(false, true, false, false, Some(false)).0, "off");
        assert_eq!(
            classify(true, true, false, false, Some(false)).0,
            "repairRequired"
        );
        assert_eq!(
            classify(true, true, true, false, Some(false)).0,
            "repairRequired"
        );
        assert_eq!(classify(true, true, true, true, Some(true)).0, "osDisabled");
        assert_eq!(classify(true, true, true, true, None).0, "unknown");
        assert_eq!(
            classify(false, true, true, true, Some(false)).0,
            "unexpectedRegistration"
        );
    }
    #[test]
    fn preference_is_default_off_atomic_and_preserves_corruption() {
        let dir = std::env::temp_dir().join(format!("gitjuke-login-pref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let login = Login::new(&dir);
        assert!(!login.preference().unwrap().enabled);
        login.save(true).unwrap();
        assert!(Login::new(&dir).preference().unwrap().enabled);
        std::fs::write(&login.preference, b"broken").unwrap();
        assert!(login.preference().is_err());
        assert_eq!(std::fs::read(&login.preference).unwrap(), b"broken");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn stable_path_identity_and_no_watchdog_are_required() {
        let dir = std::env::temp_dir().join(format!("gitjuke-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let bundle = dir.join("GitJuke.app");
        let exe = bundle.join("Contents/MacOS/gitjuke");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"executable").unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), NORMAL_ID.into());
        plist::Value::Dictionary(info)
            .to_file_xml(bundle.join("Contents/Info.plist"))
            .unwrap();
        assert!(valid_install(&exe, &bundle, NORMAL_ID));
        assert!(!valid_install(&exe, &bundle, "other"));
        assert!(!valid_install(
            &dir.join("moved.app/Contents/MacOS/gitjuke"),
            &bundle,
            NORMAL_ID
        ));
        let link = dir.join("Linked.app");
        std::os::unix::fs::symlink(&bundle, &link).unwrap();
        assert!(!valid_install(
            &link.join("Contents/MacOS/gitjuke"),
            &link,
            NORMAL_ID
        ));
        let mut agent = plist::Dictionary::new();
        agent.insert("Label".into(), NORMAL_ID.into());
        agent.insert(
            "ProgramArguments".into(),
            vec![
                plist::Value::String(exe.to_string_lossy().into_owned()),
                "--background".into(),
            ]
            .into(),
        );
        agent.insert("RunAtLoad".into(), true.into());
        let path = dir.join("agent.plist");
        plist::Value::Dictionary(agent.clone())
            .to_file_xml(&path)
            .unwrap();
        assert!(valid_registration(&path, &bundle, NORMAL_ID));
        agent.insert("KeepAlive".into(), true.into());
        plist::Value::Dictionary(agent).to_file_xml(&path).unwrap();
        assert!(!valid_registration(&path, &bundle, NORMAL_ID));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
