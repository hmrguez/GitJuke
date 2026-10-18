use gitjuke_core::{model::*, runtime::Runtime};
use tauri::{
    Emitter, Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

fn safe<T: serde::Serialize>(result: gitjuke_core::Result<T>) -> Result<T, String> {
    let value = result.map_err(|e| e.to_string())?;
    let bytes =
        serde_json::to_vec(&value).map_err(|_| "The response could not be encoded.".to_owned())?;
    if bytes.len() > 1024 * 1024 {
        return Err("This response exceeds the 1 MiB IPC limit. Use a smaller page or the GitHub source; no truncated data was returned.".into());
    }
    Ok(value)
}
#[tauri::command]
fn runtime_status(runtime: State<'_, Runtime>) -> Result<Status, String> {
    safe(runtime.status())
}
#[tauri::command]
fn authored_page(runtime: State<'_, Runtime>, offset: usize) -> Result<Page, String> {
    safe(runtime.page(offset))
}
#[tauri::command]
fn requested_page(runtime: State<'_, Runtime>, offset: usize) -> Result<Page, String> {
    safe(runtime.requested_page(offset))
}
#[tauri::command]
fn watch_page(runtime: State<'_, Runtime>, offset: usize) -> Result<WatchPage, String> {
    safe(runtime.watches(offset))
}
#[tauri::command]
fn activity_page(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<gitjuke_core::activity::ActivityPage, String> {
    safe(runtime.activity(offset))
}
#[tauri::command]
fn activity_seen(runtime: State<'_, Runtime>, ids: Vec<String>) -> Result<(), String> {
    safe(runtime.mark_activity_seen(&ids))
}
#[tauri::command]
fn read_checks(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
    offset: usize,
) -> Result<CheckPage, String> {
    safe(runtime.checks(&repo, number, offset))
}
#[tauri::command]
fn read_overview(
    runtime: State<'_, Runtime>,
    repo: String,
    number: u64,
) -> Result<Snapshot<Option<Overview>>, String> {
    safe(runtime.overview(&repo, number))
}
#[tauri::command]
fn refresh(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.refresh())
}
#[tauri::command]
fn select_pr(runtime: State<'_, Runtime>, repo: String, number: u64) -> Result<(), String> {
    safe(runtime.select(repo, number))
}
#[tauri::command]
fn set_pause(runtime: State<'_, Runtime>, paused: bool) -> Result<(), String> {
    safe(runtime.set_pause(paused))
}
#[tauri::command]
fn set_theme(runtime: State<'_, Runtime>, theme: Theme) -> Result<(), String> {
    safe(runtime.set_theme(theme))
}
#[tauri::command]
fn clear_downloads(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.clear())
}
#[tauri::command]
fn configure_gh(runtime: State<'_, Runtime>, path: String, config: String) -> Result<(), String> {
    safe(runtime.configure(path, config))
}
#[tauri::command]
fn open_pr(runtime: State<'_, Runtime>, repo: String, number: u64) -> Result<(), String> {
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
fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(r) = app.try_state::<Runtime>() {
        let _ = r.reader_opened();
    }
}
fn quit(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Some(r) = app.try_state::<Runtime>() {
            r.shutdown().await;
        }
        app.exit(0);
    });
}

pub fn run() {
    // The runtime uses Tokio independently of WebView/window lifetime.
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == "--background") {
                show(app)
            }
        }))
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let handle = app.handle().clone();
            let runtime = tauri::async_runtime::block_on(async {
                Runtime::start(directory.join("gitjuke.sqlite3"), move |revision| {
                    let _ = handle.emit("cache-revision", revision);
                })
            })?;
            app.manage(runtime);
            let open = MenuItem::with_id(app, "open", "Open GitJuke", true, None::<&str>)?;
            let pause = MenuItem::with_id(
                app,
                "pause",
                "Pause / Resume monitoring",
                true,
                None::<&str>,
            )?;
            let status = MenuItem::with_id(
                app,
                "status",
                "Implementation preview — alerts unavailable",
                false,
                None::<&str>,
            )?;
            let quit_item =
                MenuItem::with_id(app, "quit", "Quit GitJuke", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&status, &open, &pause, &quit_item])?;
            TrayIconBuilder::with_id("monitor")
                .title("GJ")
                .tooltip("GitJuke — PR watch monitor")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "pause" => {
                        let runtime = app.state::<Runtime>();
                        if let Ok(s) = runtime.status() {
                            let _ = runtime.set_pause(!s.settings.paused);
                        }
                    }
                    "quit" => quit(app.clone()),
                    _ => {}
                })
                .build(app)?;
            // App menu's Quit and Cmd-Q use the same cancel/reap path as the tray.
            let app_quit =
                MenuItem::with_id(app, "app-quit", "Quit GitJuke", true, Some("CmdOrCtrl+Q"))?;
            let app_menu = Menu::with_items(
                app,
                &[&tauri::menu::Submenu::with_items(
                    app,
                    "GitJuke",
                    true,
                    &[&app_quit],
                )?],
            )?;
            app.set_menu(app_menu)?;
            if std::env::args().any(|a| a == "--background") {
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
                app.state::<Runtime>().reader_closed()?;
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "app-quit" {
                quit(app.clone());
            }
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::Focused(_) | tauri::WindowEvent::Resized(_)
            ) {
                let visible =
                    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
                let _ = window.state::<Runtime>().reader_visibility(visible);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let _ = window.state::<Runtime>().reader_closed();
            }
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            authored_page,
            requested_page,
            watch_page,
            read_checks,
            activity_page,
            activity_seen,
            read_overview,
            refresh,
            select_pr,
            set_pause,
            set_theme,
            clear_downloads,
            configure_gh,
            open_pr
        ])
        .build(tauri::generate_context!())
        .expect("GitJuke could not start; existing application data has been preserved")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } if code.is_none() => {
                api.prevent_exit();
                quit(app.clone());
            }
            tauri::RunEvent::Reopen { .. } => show(app),
            _ => {}
        });
}

#[cfg(test)]
mod ipc_tests {
    #[test]
    fn serialized_response_ceiling_is_inclusive_and_never_clips() {
        let exact = "x".repeat(1024 * 1024 - 2);
        assert!(super::safe(Ok(exact)).is_ok());
        let over = "x".repeat(1024 * 1024 - 1);
        assert!(super::safe(Ok(over)).is_err());
    }
}
