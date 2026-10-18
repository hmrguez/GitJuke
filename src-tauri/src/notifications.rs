use gitjuke_core::alerts::{Alert, AlertSink, Permission};
use std::ffi::CString;
unsafe extern "C" {
    fn gj_notifications_initialize();
    fn gj_notifications_refresh();
    fn gj_notifications_permission() -> i32;
    fn gj_notifications_request();
    fn gj_notifications_send(
        id: i64,
        title: *const std::ffi::c_char,
        body: *const std::ffi::c_char,
    );
    fn gj_notifications_click() -> i64;
    fn gj_notifications_error() -> i32;
}
pub struct NativeAlerts;
impl NativeAlerts {
    pub fn initialize() {
        unsafe { gj_notifications_initialize() }
    }
    pub fn request() {
        unsafe { gj_notifications_request() }
    }
    pub fn refresh() {
        unsafe { gj_notifications_refresh() }
    }
    pub fn click() -> i64 {
        unsafe { gj_notifications_click() }
    }
    pub fn failed() -> bool {
        unsafe { gj_notifications_error() != 0 }
    }
}
impl AlertSink for NativeAlerts {
    fn permission(&self) -> Permission {
        match unsafe { gj_notifications_permission() } {
            1 => Permission::NotDetermined,
            2 => Permission::Denied,
            3 => Permission::Granted,
            _ => Permission::Unknown,
        }
    }
    fn dispatch(&self, a: &Alert) {
        if let (Ok(title), Ok(body)) = (
            CString::new(a.title.as_str()),
            CString::new(a.body.as_str()),
        ) {
            unsafe { gj_notifications_send(a.intent, title.as_ptr(), body.as_ptr()) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbundled_notification_calls_are_safe() {
        // Like `tauri dev`, Cargo's test executable has no .app bundle.
        // Every notification entry point must remain safe in that environment.
        NativeAlerts::initialize();
        NativeAlerts::refresh();
        NativeAlerts::request();
        assert!(matches!(NativeAlerts.permission(), Permission::Unknown));
        let text = CString::new("test").unwrap();
        unsafe { gj_notifications_send(1, text.as_ptr(), text.as_ptr()) };
        assert!(NativeAlerts::failed());
        assert_eq!(NativeAlerts::click(), 0);
    }
}
