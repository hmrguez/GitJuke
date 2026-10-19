use super::safe;
use crate::app::{NativeAlerts, NotificationRoute};
use gitjuke_core::runtime::Runtime;
use tauri::State;

#[tauri::command]
pub(crate) fn set_alert_choices(
    runtime: State<'_, Runtime>,
    choices: gitjuke_core::alerts::AlertChoices,
) -> Result<(), String> {
    safe(runtime.set_alert_choices(choices))
}
#[tauri::command]
pub(crate) fn enable_notifications(runtime: State<'_, Runtime>) -> Result<(), String> {
    let a = safe(runtime.status())?.settings.alerts;
    safe(
        runtime.set_alert_choices(gitjuke_core::alerts::AlertChoices {
            enabled: true,
            previews: a.previews,
            categories: a.categories,
        }),
    )?;
    NativeAlerts::request();
    Ok(())
}
#[tauri::command]
pub(crate) fn notification_settings() -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        .spawn()
        .map_err(|_| "Could not open System Settings.".to_owned())?;
    Ok(())
}
#[tauri::command]
pub(crate) fn take_notification_route(
    runtime: State<'_, Runtime>,
    route: State<'_, NotificationRoute>,
) -> Result<Option<gitjuke_core::alerts::NotificationDestination>, String> {
    let mut pending = route
        .0
        .lock()
        .map_err(|_| "Notification routing is unavailable.")?;
    let Some(id) = *pending else { return Ok(None) };
    let destination = safe(runtime.notification_destination(id))?;
    *pending = None;
    Ok(Some(destination))
}
