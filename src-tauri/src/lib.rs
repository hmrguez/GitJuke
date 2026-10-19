#[cfg(target_os = "macos")]
mod menus;
mod build_info;
mod notifications;
use notifications::NativeAlerts;
use std::sync::Mutex;
struct NotificationRoute(Mutex<Option<i64>>);
mod login;
mod singleton;
use std::sync::atomic::{AtomicBool, Ordering};
struct Lifecycle {
    quitting: AtomicBool,
    invalidated: AtomicBool,
    startup_error: std::sync::Mutex<Option<String>>,
}
mod startup;
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
fn set_alert_choices(
    runtime: State<'_, Runtime>,
    choices: gitjuke_core::alerts::AlertChoices,
) -> Result<(), String> {
    safe(runtime.set_alert_choices(choices))
}
#[tauri::command]
fn enable_notifications(runtime: State<'_, Runtime>) -> Result<(), String> {
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
fn notification_settings() -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        .spawn()
        .map_err(|_| "Could not open System Settings.".to_owned())?;
    Ok(())
}
#[tauri::command]
fn take_notification_route(
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
#[tauri::command]
fn request_logs(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::logs::LogJob,
) -> Result<u64, String> {
    safe(runtime.request_logs(job))
}
#[tauri::command]
fn read_logs(runtime: State<'_, Runtime>, seq: u64) -> Result<gitjuke_core::logs::LogRead, String> {
    safe(runtime.read_logs(seq))
}
#[tauri::command]
fn cancel_logs(runtime: State<'_, Runtime>, seq: u64) -> Result<(), String> {
    safe(runtime.cancel_logs(seq))
}
#[tauri::command]
fn open_check_details(
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
fn saved_queries(
    runtime: State<'_, Runtime>,
    offset: usize,
) -> Result<gitjuke_core::saved_queries::QueryList, String> {
    safe(runtime.saved_queries(offset))
}
#[tauri::command]
fn save_query(
    runtime: State<'_, Runtime>,
    id: Option<i64>,
    version: Option<u64>,
    name: String,
    search: String,
) -> Result<gitjuke_core::saved_queries::SavedQuery, String> {
    safe(runtime.save_query(id, version, name, search))
}
#[tauri::command]
fn delete_query(runtime: State<'_, Runtime>, id: i64, version: u64) -> Result<(), String> {
    safe(runtime.delete_query(id, version))
}
#[tauri::command]
fn activate_query(
    runtime: State<'_, Runtime>,
    id: Option<i64>,
    version: Option<u64>,
) -> Result<(), String> {
    safe(runtime.activate_query(id, version))
}
#[tauri::command]
fn request_query(
    runtime: State<'_, Runtime>,
    id: i64,
    version: u64,
    page: u32,
) -> Result<(), String> {
    safe(runtime.request_query(id, version, page))
}
#[tauri::command]
fn query_page(
    runtime: State<'_, Runtime>,
    id: i64,
    version: u64,
    page: u32,
) -> Result<gitjuke_core::saved_queries::QueryPage, String> {
    safe(runtime.query_page(id, version, page))
}
#[tauri::command]
fn activity_rendered(runtime: State<'_, Runtime>, id: String, text: String) -> Result<(), String> {
    safe(runtime.activity_rendered(&id, &text))
}
#[tauri::command]
fn conversation_seen(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
    id: String,
    digest: String,
    generation: u64,
) -> Result<(), String> {
    safe(runtime.conversation_seen(&job, &id, &digest, generation))
}
#[tauri::command]
async fn post_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
    body: String,
    thread: Option<String>,
) -> Result<gitjuke_core::conversation::Message, String> {
    safe(runtime.post_conversation(job, body, thread).await)
}
#[tauri::command]
fn request_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
) -> Result<(), String> {
    safe(runtime.request_conversation(job))
}
#[tauri::command]
fn cancel_conversation(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.cancel_conversation())
}
#[tauri::command]
fn read_conversation(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::conversation::ConversationJob,
) -> Result<Snapshot<Option<gitjuke_core::conversation::ConversationPage>>, String> {
    safe(runtime.conversation(&job))
}
#[tauri::command]
fn open_conversation(
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
fn runtime_status(app: tauri::AppHandle) -> Result<Status, String> {
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
fn request_changes(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<(), String> {
    safe(runtime.request_changes(job))
}
#[tauri::command]
fn set_changes_mode(runtime: State<'_, Runtime>, paged: bool) -> Result<(), String> {
    safe(runtime.set_changes_mode(paged))
}
#[tauri::command]
fn cancel_changes(runtime: State<'_, Runtime>) -> Result<(), String> {
    safe(runtime.cancel_changes())
}
#[tauri::command]
fn read_files(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<Snapshot<Option<gitjuke_core::changes::Files>>, String> {
    safe(runtime.changes_files(&job))
}
#[tauri::command]
fn read_patch(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
) -> Result<Snapshot<Option<gitjuke_core::changes::Patch>>, String> {
    safe(runtime.changes_patch(&job))
}
#[tauri::command]
fn read_patch_chunk(
    runtime: State<'_, Runtime>,
    job: gitjuke_core::changes::ChangesJob,
    digest: String,
    chunk: usize,
) -> Result<Snapshot<Option<String>>, String> {
    safe(runtime.changes_chunk(&job, &digest, chunk))
}
#[tauri::command]
fn open_changes(
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
fn open_markdown_link(url: String) -> Result<(), String> {
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
    if app
        .try_state::<Lifecycle>()
        .is_some_and(|l| l.quitting.load(Ordering::Acquire))
    {
        return;
    }
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    if !app.state::<Lifecycle>().invalidated.load(Ordering::Acquire)
        && let Some(r) = app.try_state::<Runtime>()
    {
        let _ = r.reader_opened();
    }
}
fn quit(app: tauri::AppHandle) {
    if app
        .state::<Lifecycle>()
        .quitting
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    if let Some(login) = app.try_state::<login::Login>() {
        login.stop();
    }
    if let Some(runtime) = app.try_state::<Runtime>() {
        runtime.request_shutdown();
    }
    tauri::async_runtime::spawn(async move {
        if let Some(r) = app.try_state::<Runtime>() {
            r.shutdown().await;
        }
        let handle = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if let Some(login) = handle.try_state::<login::Login>() {
                login.drain();
            }
        })
        .await;
        app.exit(0);
    });
}

fn background_launch() -> bool {
    std::env::args().skip(1).any(|a| a == "--background")
}
fn monitor_allowed(app: &tauri::AppHandle) -> bool {
    let id = &app.config().identifier;
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if login::valid_install(&exe, &login::installed_bundle(id), id) {
        return true;
    }
    // Preserve explicitly separate native reader/log/accessibility harness identities.
    // These debug builds can monitor their own namespaces, but cannot register login.
    #[cfg(debug_assertions)]
    if id != login::NORMAL_ID && id != "dev.hmrguez.gitjuke.lifecycle-qualification" {
        return true;
    }
    // `tauri dev` remains useful. Normal bundled development copies obey installation checks.
    cfg!(debug_assertions) && !exe.to_string_lossy().contains(".app/") && id == login::NORMAL_ID
}
#[cfg(all(debug_assertions, feature = "lifecycle-qualification"))]
fn qualification_visibility(app: &tauri::AppHandle, event: &str) {
    if app.config().identifier == "dev.hmrguez.gitjuke.lifecycle-qualification" {
        println!(
            "GITJUKE_LIFECYCLE pid={} event={} background={} visible={} paused={:?} permission={:?}",
            std::process::id(),
            event,
            background_launch(),
            app.get_webview_window("main")
                .is_some_and(|w| w.is_visible().unwrap_or(false)),
            app.try_state::<Runtime>()
                .and_then(|r| r.status().ok())
                .map(|s| s.settings.paused),
            gitjuke_core::alerts::AlertSink::permission(&NativeAlerts)
        );
    }
}
pub fn run() {
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let identifier = context.config().identifier.clone();
    // The runtime uses Tokio independently of WebView/window lifetime.
    tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("lifecycle-owner")
                .setup(|app, _| {
                    app.manage(Lifecycle {
                        quitting: AtomicBool::new(false),
                        invalidated: AtomicBool::new(false),
                        startup_error: std::sync::Mutex::new(None),
                    });
                    let directory=app.path().app_data_dir()?;
                    if background_launch() {
                        match login::Login::new(&directory).desired() {
                            Ok(false)=>std::process::exit(0),
                            Ok(true)=>{},
                            Err(error)=>{ *app.state::<Lifecycle>().startup_error.lock().unwrap()=Some(error); return Ok(()); }
                        }
                    }
                    if monitor_allowed(app) {
                        match singleton::directory_for(&app.config().identifier).and_then(|owner|singleton::acquire(&owner, background_launch())) {
                            Ok(singleton::Instance::Forwarded) => std::process::exit(0),
                            Ok(singleton::Instance::Owner(owner)) => {
                                let listener = owner.listener.try_clone()?;
                                app.manage(owner);
                                let handle = app.clone();
                                std::thread::spawn(move || {
                                    for connection in listener.incoming() {
                                        let Ok(stream) = connection else { break };
                                        if let Ok(false) = singleton::receive(stream) {
                                            let app = handle.clone();
                                            let _ = handle.run_on_main_thread(move || show(&app));
                                        }
                                    }
                                });
                            }
                            Err(error)=> { *app.state::<Lifecycle>().startup_error.lock().unwrap()=Some(format!("Could not acquire the single-instance monitor: {error}")); }
                        }
                    }
                    Ok(())
                })
                .build(),
        )
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(identifier)
                .macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent)
                .arg("--background")
                .build(),
        )
        .setup(|app| {
            NativeAlerts::initialize();
            app.manage(NotificationRoute(Mutex::new(None)));
            let directory = app.path().app_data_dir()?;
            app.manage(login::Login::new(&directory));
            app.manage(startup::Startup::new(directory.join("gitjuke.sqlite3")));
            startup::initialize(app.handle());
            let notification_app=app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if notification_app.state::<Lifecycle>().quitting.load(Ordering::Acquire) || notification_app.state::<Lifecycle>().invalidated.load(Ordering::Acquire) { break; }
                    NativeAlerts::refresh();
                    let id=NativeAlerts::click();
                    if id>0 {
                        if let Ok(mut route)=notification_app.state::<NotificationRoute>().0.lock(){*route=Some(id);}
                        show(&notification_app);
                        let _=notification_app.emit("notification-route",());
                    }
                    if NativeAlerts::failed(){let _=notification_app.emit("notification-delivery-error","macOS did not confirm the alert request. It will not be replayed; Activity remains available.");}
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
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
                if monitor_allowed(app.handle()) {
                    "Starting monitor — Open GitJuke for status"
                } else {
                    "Install GitJuke in /Applications — monitoring stopped"
                },
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
                        if let Some(runtime) = app.try_state::<Runtime>()
                            && let Ok(s) = runtime.status()
                        {
                            let _ = runtime.set_pause(!s.settings.paused);
                        }
                    }
                    "quit" => quit(app.clone()),
                    _ => {}
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            menus::install(app)?;
            #[cfg(not(target_os = "macos"))]
            {
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
            }
            // The window starts hidden in configuration; login never flashes a reader.
            if background_launch() {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                show(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                #[cfg(all(debug_assertions, feature="lifecycle-qualification"))]
                let mut observations=5;
                loop {
                    #[cfg(all(debug_assertions, feature="lifecycle-qualification"))]
                    if observations>0 { qualification_visibility(&handle,"monitor"); observations-=1; }
                    if handle.state::<Lifecycle>().quitting.load(Ordering::Acquire) {
                        break;
                    }
                    if !monitor_allowed(&handle)
                        && !handle
                            .state::<Lifecycle>()
                            .invalidated
                            .swap(true, Ordering::AcqRel)
                        && let Some(runtime) = handle.try_state::<Runtime>() {
                            runtime.shutdown().await;
                    }
                    let text = if handle
                        .state::<Lifecycle>()
                        .invalidated
                        .load(Ordering::Acquire)
                    {
                        "Install GitJuke in /Applications — monitoring stopped".to_owned()
                    } else {
                        handle
                            .try_state::<Runtime>()
                            .and_then(|r| r.status().ok())
                            .map(|s| {
                                if s.settings.paused {
                                    format!("Paused — {}",s.health)
                                } else {
                                    s.health
                                }
                            })
                            .unwrap_or_else(|| {
                                "Startup stopped — Open GitJuke for recovery guidance".into()
                            })
                    };
                    let _ = status.set_text(text.chars().take(160).collect::<String>());
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            #[cfg(target_os = "macos")]
            menus::handle(app, event.id.as_ref());
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
                if let Some(runtime) = window.try_state::<Runtime>() {
                    let _ = runtime.reader_visibility(visible);
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                if let Some(runtime) = window.try_state::<Runtime>() {
                    let _ = runtime.reader_closed();
                }
                let _ = window.emit("gitjuke-reader-closed", ());
                let _ = window
                    .app_handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(target_os = "macos")]
            menus::sync_reader_menu,
            build_info::build_info,
            login::login_status,
            login::set_launch_at_login,
            set_alert_choices,
            enable_notifications,
            notification_settings,
            take_notification_route,
            saved_queries,
            save_query,
            delete_query,
            activate_query,
            request_query,
            query_page,
            activity_rendered,
            conversation_seen,
            request_conversation,
            post_conversation,
            cancel_conversation,
            read_conversation,
            open_conversation,
            request_logs,
            read_logs,
            cancel_logs,
            open_check_details,
            request_changes,
            set_changes_mode,
            cancel_changes,
            read_files,
            read_patch,
            read_patch_chunk,
            open_changes,
            startup::startup_status,
            startup::retry_startup,
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
            open_pr,
            open_markdown_link
        ])
        .build(context)
        .expect("GitJuke could not start; existing application data has been preserved")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } if code.is_none() => {
                api.prevent_exit();
                quit(app.clone());
            }
            tauri::RunEvent::Reopen { .. } => {
                #[cfg(all(debug_assertions, feature="lifecycle-qualification"))]
                qualification_visibility(app,"reopen");
                show(app)
            },
            #[cfg(all(debug_assertions, feature="lifecycle-qualification"))]
            tauri::RunEvent::Ready => qualification_visibility(app,"ready"),
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
