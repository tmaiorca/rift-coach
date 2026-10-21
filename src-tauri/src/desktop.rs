//! Tray icon, close-to-tray, start with Windows, and bringing the window
//! back when Rift Coach is launched a second time.

use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;
use crate::commands::{CmdResult, err};

/// Argument Windows passes when starting Rift Coach at sign-in: start in the tray.
pub const MINIMIZED_ARG: &str = "--minimized";

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Rift Coach", true, None::<&str>)?;
    let live = MenuItem::with_id(app, "live", "Live window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &live, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Rift Coach")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "live" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::commands::open_live_window(app).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Closing the main window hides it to the tray when that's on.
pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let state = window.state::<AppState>();
        if state.close_to_tray.load(Ordering::Relaxed) {
            api.prevent_close();
            let _ = window.hide();
            if !state.tray_hint_shown.swap(true, Ordering::Relaxed) {
                use tauri_plugin_notification::NotificationExt;
                let _ = window
                    .app_handle()
                    .notification()
                    .builder()
                    .title("Rift Coach is still running")
                    .body("It's in the tray, ready for your next game. Right-click the icon to quit.")
                    .show();
            }
        } else {
            // The live window would otherwise keep the app alive with no main window.
            window.app_handle().exit(0);
        }
    }
}

#[tauri::command]
pub fn autostart_enabled(app: AppHandle) -> CmdResult<bool> {
    app.autolaunch().is_enabled().map_err(err)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<bool> {
    let m = app.autolaunch();
    if enabled { m.enable() } else { m.disable() }.map_err(err)?;
    m.is_enabled().map_err(err)
}

/// Keeps the close-to-tray flag in step with saved settings.
pub fn apply_settings(state: &State<'_, AppState>, s: &rc_store::settings::AppSettings) {
    state.close_to_tray.store(s.desktop.close_to_tray, Ordering::Relaxed);
}
