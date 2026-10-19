use super::safe;
use gitjuke_core::{model::Snapshot, runtime::Runtime};
use tauri::State;

#[tauri::command]
pub(crate) fn request_changes(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<(), String> {
    safe(runtime.request_changes(job))
}
#[tauri::command]
pub(crate) fn set_changes_mode(runtime: State<'_, Runtime>, paged: bool) -> Result<(), String> {
    safe(runtime.set_changes_mode(paged))
}
#[tauri::command]
pub(crate) fn cancel_changes(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.cancel_changes())
}
#[tauri::command]
pub(crate) fn read_files(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<Snapshot<Option<gitjuke_core::changes::Files>>, String> {
    safe(runtime.changes_files(&job))
}
#[tauri::command]
pub(crate) fn read_patch(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<Snapshot<Option<gitjuke_core::changes::Patch>>, String> {
    safe(runtime.changes_patch(&job))
}
#[tauri::command]
pub(crate) fn read_patch_chunk(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
    digest: String,
    chunk: usize,
) -> Result<Snapshot<Option<String>>, String> {
    safe(runtime.changes_chunk(&job, &digest, chunk))
}
#[tauri::command]
pub(crate) fn open_changes(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<(), String> {
    let url = safe(runtime.changes_url(&job))?;
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map_err(|_| "Could not open GitHub.".to_owned())?;
    Ok(())
}
