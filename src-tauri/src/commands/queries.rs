use super::safe;
use gitjuke_core::runtime::Runtime;
use tauri::State;

#[tauri::command]
pub(crate) fn saved_queries(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<gitjuke_core::saved_queries::QueryList, String> {
    safe(runtime.saved_queries(offset))
}
#[tauri::command]
pub(crate) fn save_query(
    runtime: State<'_, Runtime>,
    id: Option<i64>,
    version: Option<u64>,
    name: String,
    search: String,
) -> Result<gitjuke_core::saved_queries::SavedQuery, String> {
    safe(runtime.save_query(id, version, name, search))
}
#[tauri::command]
pub(crate) fn delete_query(
    runtime: State<'_, Runtime>,
    id: i64,
    version: u64,
) -> Result<(), String> {
    safe(runtime.delete_query(id, version))
}
#[tauri::command]
pub(crate) fn activate_query(
    runtime: State<'_, Runtime>,
    id: Option<i64>,
    version: Option<u64>,
) -> Result<(), String> {
    safe(runtime.activate_query(id, version))
}
#[tauri::command]
pub(crate) fn request_query(
    runtime: State<'_, Runtime>,
    id: i64,
    version: u64,
    page: u32,
) -> Result<(), String> {
    safe(runtime.request_query(id, version, page))
}
#[tauri::command]
pub(crate) async fn query_page(
    runtime: State<'_, Runtime>,
    id: i64,
    version: u64,
    page: u32,
) -> Result<gitjuke_core::saved_queries::QueryPage, String> {
    safe(runtime.prepared_query_page(id, version, page).await)
}
