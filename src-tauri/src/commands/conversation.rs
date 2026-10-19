use super::safe;
use gitjuke_core::{model::Snapshot, runtime::Runtime};
use tauri::State;

#[tauri::command]
pub(crate) fn activity_rendered(
    runtime: State<'_, Runtime>,
    id: String,
    text: String,
) -> Result<(), String> {
    safe(runtime.activity_rendered(&id, &text))
}
#[tauri::command]
pub(crate) fn conversation_seen(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
    id: String,
    digest: String,
    generation: u64,
) -> Result<(), String> {
    safe(runtime.conversation_seen(&job, &id, &digest, generation))
}
#[tauri::command]
pub(crate) async fn post_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
    body: String,
    thread: Option<String>,
) -> Result<gitjuke_core::conversation::Message, String> {
    safe(runtime.post_conversation(job, body, thread).await)
}
#[tauri::command]
pub(crate) fn request_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
) -> Result<(), String> {
    safe(runtime.request_conversation(job))
}
#[tauri::command]
pub(crate) fn cancel_conversation(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.cancel_conversation())
}
#[tauri::command]
pub(crate) fn read_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
) -> Result<Snapshot<Option<gitjuke_core::conversation::ConversationPage>>, String> {
    safe(runtime.conversation(&job))
}
#[tauri::command]
pub(crate) fn open_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
    id: String,
) -> Result<(), String> {
    let url = safe(runtime.conversation_url(&job, &id))?;
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map_err(|_| "Could not open GitHub.".to_owned())?;
    Ok(())
}
#[tauri::command]
pub(crate) fn activity_page(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<gitjuke_core::activity::ActivityPage, String> {
    safe(runtime.activity(offset))
}
#[tauri::command]
pub(crate) fn activity_seen(runtime: State<'_, Runtime>, ids: Vec<String>) -> Result<(), String> {
    safe(runtime.mark_activity_seen(&ids))
}
