//! A failed database open keeps a usable shell; retry never resets or renames data.
use gitjuke_core::runtime::Runtime;
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    ready: bool,
    database_path: String,
    message: String,
    database_state: gitjuke_core::recovery::DatabaseState,
}
pub(crate) struct Startup {
    path: PathBuf,
    report: Mutex<Report>,
    attempt: Mutex<()>,
}
impl Startup {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            attempt: Mutex::new(()),
            report: Mutex::new(Report {
                database_state: gitjuke_core::recovery::DatabaseState::Missing,
                ready: false,
                database_path: path.to_string_lossy().into_owned(),
                message: "Local storage is being checked.".into(),
            }),
            path,
        }
    }
}

#[cfg(test)]
fn open(
    path: &std::path::Path,
    revision: impl Fn(u64) + Send + Sync + 'static,
) -> gitjuke_core::Result<Runtime> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    Runtime::start(path.to_owned(), revision)
}

pub(crate) fn initialize(app: &tauri::AppHandle) {
    let state = app.state::<Startup>();
    // Serializes repeated Retry clicks, and prevents a second worker/session.
    let _attempt = state.attempt.lock().unwrap();
    if app.try_state::<Runtime>().is_some() {
        state.report.lock().unwrap().ready = true;
        return;
    }
    let lifecycle = app.state::<super::Lifecycle>();
    if lifecycle
        .quitting
        .load(std::sync::atomic::Ordering::Acquire)
        || lifecycle
            .invalidated
            .load(std::sync::atomic::Ordering::Acquire)
        || !super::monitor_allowed(app)
        || lifecycle.startup_error.lock().unwrap().is_some()
    {
        let mut report = state.report.lock().unwrap();
        report.ready = false;
        report.database_state = gitjuke_core::recovery::inspect(&state.path);
        report.message = lifecycle.startup_error.lock().unwrap().clone().unwrap_or_else(||
            "Monitoring is stopped. Install GitJuke in its stable application location, then quit and reopen it.".into());
        return;
    }
    let handle = app.clone();
    let result = tauri::async_runtime::block_on(async {
        if let Some(directory) = state.path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let notify = move |revision| {
            let _ = handle.emit("cache-revision", revision);
        };
        if super::background_launch() {
            Runtime::start_background(state.path.clone(), notify)
        } else {
            Runtime::start(state.path.clone(), notify)
        }
    });
    let mut report = state.report.lock().unwrap();
    match result {
        Ok(runtime) => {
            runtime.install_alert_sink(std::sync::Arc::new(super::NativeAlerts));
            app.manage(runtime);
            report.ready = true;
            report.message.clear();
            // Honor reader visibility even if recovery was retried after close.
            if let Some(runtime) = app.try_state::<Runtime>() {
                if app
                    .get_webview_window("main")
                    .is_some_and(|w| w.is_visible().unwrap_or(false))
                {
                    let _ = runtime.reader_opened();
                } else {
                    let _ = runtime.reader_closed();
                }
            }
        }
        Err(_) => {
            report.ready = false;
            report.database_state = gitjuke_core::recovery::inspect(&state.path);
            // Never expose SQL, source text or private cache values in diagnostics.
            report.message = "GitJuke could not safely open local storage. Monitoring and alerts are stopped. Original database files are preserved; no reset or new historical baseline was performed.".into();
        }
    }
}

#[tauri::command]
pub(crate) fn startup_status(state: tauri::State<'_, Startup>) -> Report {
    state.report.lock().unwrap().clone()
}
#[tauri::command]
pub(crate) async fn retry_startup(app: tauri::AppHandle) -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(move || {
        initialize(&app);
        app.state::<Startup>().report.lock().unwrap().clone()
    }).await.map_err(|_| "The storage check could not complete. Quit and reopen GitJuke; original files are preserved.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn corrupt_and_unreadable_files_survive_failed_startup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let original = b"original corrupt database -- do not replace";
        std::fs::write(&path, original).unwrap();
        assert!(open(&path, |_| {}).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("original"), original).unwrap();
        assert!(open(&path, |_| {}).is_err());
        assert_eq!(std::fs::read(path.join("original")).unwrap(), original);
    }
    #[tokio::test]
    async fn missing_directory_can_be_repaired_and_retried_without_reset() {
        let d = tempfile::tempdir().unwrap();
        let directory = d.path().join("blocked");
        std::fs::write(&directory, b"original").unwrap();
        let path = directory.join("db");
        assert!(open(&path, |_| {}).is_err());
        assert_eq!(std::fs::read(&directory).unwrap(), b"original");
        std::fs::remove_file(&directory).unwrap();
        std::fs::create_dir(&directory).unwrap();
        let mut store = gitjuke_core::store::Store::open(&path).unwrap();
        let settings = gitjuke_core::model::Settings {
            paused: true,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        drop(store);
        let runtime = open(&path, |_| {}).unwrap();
        assert!(runtime.status().unwrap().locked);
        runtime.shutdown().await;
        assert!(path.is_file());
    }
}
