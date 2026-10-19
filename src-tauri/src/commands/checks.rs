use super::safe;
use gitjuke_core::{model::*, runtime::Runtime};
use tauri::State;

#[tauri::command]
pub(crate) fn request_logs(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::logs::LogJob,
) -> Result<u64, String> {
    safe(runtime.request_logs(job))
}
#[tauri::command]
pub(crate) fn read_logs(
    runtime: State<'_, Runtime>,
    seq: u64,
) -> Result<gitjuke_core::logs::LogRead, String> {
    safe(runtime.read_logs(seq))
}
#[tauri::command]
pub(crate) fn cancel_logs(runtime: State<'_, Runtime>, seq: u64) -> Result<(), String> {
    safe(runtime.cancel_logs(seq))
}
#[tauri::command]
pub(crate) fn open_check_details(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::logs::LogJob,
) -> Result<(), String> {
    let url = safe(runtime.check_details(&job))?;
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map_err(|_| "Could not open check details.".to_owned())?;
    Ok(())
}
#[tauri::command]
pub(crate) fn read_checks(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
    offset: usize,
) -> Result<CheckPage, String> {
    safe(runtime.checks(&repo, number, offset))
}
