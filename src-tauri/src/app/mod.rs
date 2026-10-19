//! Native lifecycle, window/tray setup, and platform integration.
#[cfg(target_os = "macos")]
mod menus;
mod notifications;
pub(crate) use notifications::NativeAlerts;
use std::sync::Mutex;
pub(crate) struct NotificationRoute(pub(crate) Mutex<Option<i64>>);
mod login;
mod singleton;
use std::sync::atomic::{AtomicBool, Ordering};
pub(crate) struct Lifecycle {
    pub(crate) quitting: AtomicBool,
    pub(crate) invalidated: AtomicBool,
    pub(crate) startup_error: std::sync::Mutex<Option<String>>,
}
mod startup;
use crate::commands::{self, build_info};
use gitjuke_core::runtime::Runtime;
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

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
            commands::notifications::set_alert_choices,
            commands::notifications::enable_notifications,
            commands::notifications::notification_settings,
            commands::notifications::take_notification_route,
            commands::queries::saved_queries,
            commands::queries::save_query,
            commands::queries::delete_query,
            commands::queries::activate_query,
            commands::queries::request_query,
            commands::queries::query_page,
            commands::conversation::activity_rendered,
            commands::conversation::conversation_seen,
            commands::conversation::request_conversation,
            commands::conversation::post_conversation,
            commands::conversation::cancel_conversation,
            commands::conversation::read_conversation,
            commands::conversation::open_conversation,
            commands::checks::request_logs,
            commands::checks::read_logs,
            commands::checks::cancel_logs,
            commands::checks::open_check_details,
            commands::changes::request_changes,
            commands::changes::set_changes_mode,
            commands::changes::cancel_changes,
            commands::changes::read_files,
            commands::changes::read_patch,
            commands::changes::read_patch_chunk,
            commands::changes::open_changes,
            startup::startup_status,
            startup::retry_startup,
            commands::reader::runtime_status,
            commands::reader::authored_page,
            commands::reader::requested_page,
            commands::reader::watch_page,
            commands::checks::read_checks,
            commands::conversation::activity_page,
            commands::conversation::activity_seen,
            commands::reader::read_overview,
            commands::reader::refresh,
            commands::reader::select_pr,
            commands::reader::set_pause,
            commands::reader::set_theme,
            commands::reader::clear_downloads,
            commands::reader::configure_gh,
            commands::reader::open_pr,
            commands::reader::open_markdown_link
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
