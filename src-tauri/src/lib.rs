mod commands;
mod crypto;
mod input;
mod network;
mod screen;
mod state;
mod storage;

use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("multimouse=debug,warn")
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = Arc::new(AppState::new());
            app.manage(state.clone());

            // Populate monitor info at startup
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(primary) = win.primary_monitor() {
                    if let Ok(available) = win.available_monitors() {
                        let monitors: Vec<state::MonitorInfo> = available
                            .into_iter()
                            .map(|m| {
                                let is_primary = primary
                                    .as_ref()
                                    .and_then(|p| p.name())
                                    .zip(m.name())
                                    .map(|(a, b)| a == b)
                                    .unwrap_or(false);
                                state::MonitorInfo {
                                    name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                                    x: m.position().x,
                                    y: m.position().y,
                                    width: m.size().width,
                                    height: m.size().height,
                                    scale_factor: m.scale_factor(),
                                    is_primary,
                                }
                            })
                            .collect();
                        *state.monitors.write() = monitors;
                    }
                }
            }

            let show = MenuItemBuilder::with_id("show", "Show MultiMouse").build(app)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = Menu::with_items(app, &[&show, &sep, &quit])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("MultiMouse")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_window(app),
                    "quit" => std::process::exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            let app_handle = app.handle().clone();
            let state_bg = state.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(network::start_all_services(app_handle, state_bg));
            });

            input::inject::start_injector();
            input::capture::start(app.handle().clone(), state.clone());

            // Show window on first launch so the user sees something immediately.
            // They can close it to the tray after that.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_devices,
            commands::get_status,
            commands::connect_to_device,
            commands::disconnect,
            commands::release_cursor,
            commands::take_control,
            commands::get_settings,
            commands::update_settings,
            commands::get_monitors,
            commands::send_files,
            commands::accept_transfer,
            commands::reject_transfer,
            commands::get_transfers,
            commands::clear_transfers,
            commands::get_known_devices,
            commands::forget_device,
            commands::create_internet_session,
            commands::join_internet_session,
            commands::accept_pairing,
            commands::reject_pairing,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MultiMouse");
}

fn toggle_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}
