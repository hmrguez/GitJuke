//! macOS application menus. Custom commands reuse the reader's existing actions.
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem as Native, Submenu},
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderMenuState {
    modal: bool,
    selected: bool,
    filter: bool,
    collapsed: bool,
    text_size: u16,
    paused: bool,
    can_monitor: bool,
}

impl ReaderMenuState {
    fn availability(&self) -> [(&'static str, bool); 15] {
        let workspace = !self.modal;
        [
            ("reader-about", workspace),
            ("reader-preferences", workspace),
            ("reader-add", workspace),
            ("reader-query", workspace),
            ("reader-open-pr", workspace && self.selected),
            ("reader-filter", workspace && self.filter),
            ("reader-sidebar", workspace),
            ("reader-mine", workspace),
            ("reader-review", workspace),
            ("reader-activity", workspace),
            ("reader-refresh", workspace && self.can_monitor),
            ("reader-pause", workspace && self.can_monitor),
            ("reader-larger", self.text_size < 200),
            ("reader-smaller", self.text_size > 100),
            ("reader-actual", self.text_size != 100),
        ]
    }
}

fn find_item(menu: &Menu<tauri::Wry>, id: &str) -> tauri::Result<Option<MenuItem<tauri::Wry>>> {
    Ok(menu
        .items()?
        .into_iter()
        .find_map(|item| item.as_submenu()?.get(id)?.as_menuitem().cloned()))
}

#[tauri::command]
pub fn sync_reader_menu(app: tauri::AppHandle, state: ReaderMenuState) -> Result<(), String> {
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    for (id, enabled) in state.availability() {
        if let Some(item) = find_item(&menu, id).map_err(|e| e.to_string())? {
            item.set_enabled(enabled).map_err(|e| e.to_string())?;
        }
    }
    for (id, text) in [
        (
            "reader-sidebar",
            if state.collapsed {
                "Show Sidebar"
            } else {
                "Hide Sidebar"
            },
        ),
        (
            "reader-pause",
            if state.paused {
                "Resume Updates"
            } else {
                "Pause Updates"
            },
        ),
    ] {
        if let Some(item) = find_item(&menu, id).map_err(|e| e.to_string())? {
            item.set_text(text).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let item = |id: &str, label: &str, shortcut: Option<&str>| {
        MenuItem::with_id(app, id, label, false, shortcut)
    };
    let application = Submenu::with_items(
        app,
        "GitJuke",
        true,
        &[
            &item("reader-about", "About GitJuke", None)?,
            &Native::separator(app)?,
            &item("reader-preferences", "Preferences…", Some("Cmd+,"))?,
            &Native::separator(app)?,
            &Native::services(app, None)?,
            &Native::separator(app)?,
            &Native::hide(app, None)?,
            &Native::hide_others(app, None)?,
            &Native::show_all(app, None)?,
            &Native::separator(app)?,
            &MenuItem::with_id(app, "app-quit", "Quit GitJuke", true, Some("Cmd+Q"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item("reader-add", "Add Pull Request…", Some("Cmd+N"))?,
            &item("reader-query", "New Saved Query…", None)?,
            &Native::separator(app)?,
            &item(
                "reader-open-pr",
                "Open Selected PR in GitHub",
                Some("Cmd+Shift+O"),
            )?,
            &Native::separator(app)?,
            &Native::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &Native::undo(app, None)?,
            &Native::redo(app, None)?,
            &Native::separator(app)?,
            &Native::cut(app, None)?,
            &Native::copy(app, None)?,
            &Native::paste(app, None)?,
            &Native::select_all(app, None)?,
            &Native::separator(app)?,
            &item("reader-filter", "Filter Pull Requests", Some("Cmd+F"))?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &item("reader-sidebar", "Hide Sidebar", Some("Cmd+Ctrl+S"))?,
            &Native::separator(app)?,
            &item("reader-mine", "My PRs", Some("Cmd+1"))?,
            &item("reader-review", "Review Requests", Some("Cmd+2"))?,
            &item("reader-activity", "Activity", Some("Cmd+3"))?,
            &Native::separator(app)?,
            &item("reader-larger", "Larger Text", Some("Cmd+Shift+="))?,
            &item("reader-smaller", "Smaller Text", Some("Cmd+-"))?,
            &item("reader-actual", "Actual Size", Some("Cmd+0"))?,
            &Native::separator(app)?,
            &Native::fullscreen(app, None)?,
        ],
    )?;
    let monitor = Submenu::with_items(
        app,
        "Monitor",
        true,
        &[
            &item("reader-refresh", "Refresh", Some("Cmd+R"))?,
            &item("reader-pause", "Pause Updates", None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &Native::minimize(app, None)?,
            &Native::maximize(app, Some("Zoom"))?,
            &Native::separator(app)?,
            &Native::bring_all_to_front(app, None)?,
        ],
    )?;
    app.set_menu(Menu::with_items(
        app,
        &[&application, &file, &edit, &view, &monitor, &window],
    )?)?;
    window.set_as_windows_menu_for_nsapp()?;
    Ok(())
}

pub fn handle(app: &tauri::AppHandle, id: &str) {
    if id.starts_with("reader-") {
        super::show(app);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.emit("reader-menu", id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_selection_and_text_limits_control_menu_availability() {
        let mut state = ReaderMenuState {
            modal: false,
            selected: false,
            filter: true,
            collapsed: false,
            text_size: 100,
            paused: false,
            can_monitor: false,
        };
        let enabled = |state: &ReaderMenuState, id: &str| {
            state
                .availability()
                .into_iter()
                .find(|(key, _)| *key == id)
                .unwrap()
                .1
        };
        assert!(!enabled(&state, "reader-open-pr"));
        assert!(!enabled(&state, "reader-pause"));
        assert!(!enabled(&state, "reader-smaller"));
        assert!(!enabled(&state, "reader-actual"));
        assert!(enabled(&state, "reader-larger"));
        state.selected = true;
        state.can_monitor = true;
        assert!(enabled(&state, "reader-open-pr"));
        assert!(enabled(&state, "reader-pause"));
        state.modal = true;
        state.text_size = 200;
        for id in [
            "reader-open-pr",
            "reader-filter",
            "reader-add",
            "reader-query",
            "reader-sidebar",
            "reader-refresh",
            "reader-pause",
        ] {
            assert!(!enabled(&state, id), "{id} must respect the modal");
        }
        assert!(!enabled(&state, "reader-larger"));
        assert!(enabled(&state, "reader-smaller"));
        assert!(enabled(&state, "reader-actual"));
    }
}
