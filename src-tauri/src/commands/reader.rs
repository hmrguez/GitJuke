use super::safe;
use crate::app::Lifecycle;
use gitjuke_core::{model::*, runtime::Runtime};
use std::sync::atomic::Ordering;
use tauri::Manager;
use tauri::State;

#[tauri::command]
pub(crate) fn runtime_status(app: tauri::AppHandle) -> Result<Status, String> {
    if app.state::<Lifecycle>().invalidated.load(Ordering::Acquire) {
        return Err("The installed app moved or was removed. Monitoring stopped. Reinstall, quit this instance and relaunch before repairing login.".into());
    }
    if let Some(error) = app
        .state::<Lifecycle>()
        .startup_error
        .lock()
        .ok()
        .and_then(|e| e.clone())
    {
        return Err(error);
    }
    let runtime = app.try_state::<Runtime>().ok_or("Install GitJuke in /Applications/GitJuke.app, quit this copy, and launch the installed app. Monitoring is stopped; existing data is preserved.")?;
    safe(runtime.status())
}
#[tauri::command]
pub(crate) async fn authored_page(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<Page, String> {
    safe(runtime.prepared_page("authored", offset).await)
}
#[tauri::command]
pub(crate) async fn requested_page(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<Page, String> {
    safe(runtime.prepared_page("requested", offset).await)
}
#[tauri::command]
pub(crate) fn watch_page(runtime: State<'_, Runtime>, offset: usize) -> Result<WatchPage, String> {
    safe(runtime.watches(offset))
}
#[tauri::command]
pub(crate) fn read_overview(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
) -> Result<Snapshot<Option<Overview>>, String> {
    safe(runtime.overview(&repo, number))
}
#[tauri::command]
pub(crate) fn refresh(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.refresh())
}
#[tauri::command]
pub(crate) fn select_pr(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
) -> Result<(), String> {
    safe(runtime.select(repo, number))
}
#[tauri::command]
pub(crate) fn set_pause(runtime: State<'_, Runtime>, paused: bool) -> Result<(), String> {
    safe(runtime.set_pause(paused))
}
#[tauri::command]
pub(crate) fn set_theme(runtime: State<'_, Runtime>, theme: Theme) -> Result<(), String> {
    safe(runtime.set_theme(theme))
}
#[tauri::command]
pub(crate) fn clear_downloads(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.clear())
}
#[tauri::command]
pub(crate) fn configure_gh(
    runtime: State<'_, Runtime>,
    path: String,
    config: String,
) -> Result<(), String> {
    safe(runtime.configure(path, config))
}
#[tauri::command]
pub(crate) fn open_pr(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
) -> Result<(), String> {
    // Resolve a permitted cached record; never expose an arbitrary URL or command API.
    let snapshot = safe(runtime.overview(&repo, number))?;
    let pr = snapshot
        .data
        .ok_or("Pull request is not available in the authorized cache.")?;
    let expected = format!("https://github.com/{repo}/pull/{number}");
    if pr.url != expected {
        return Err("Invalid GitHub destination.".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg(&expected)
        .spawn()
        .map_err(|_| "Could not open the GitHub destination.".to_owned())?;
    Ok(())
}
#[tauri::command]
pub(crate) fn open_markdown_link(url: String) -> Result<(), String> {
    let parsed = tauri::Url::parse(&url).map_err(|_| "Invalid link.".to_owned())?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only HTTP and HTTPS links are supported.".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg(parsed.as_str())
        .spawn()
        .map_err(|_| "Could not open the link.".to_owned())?;
    Ok(())
}
