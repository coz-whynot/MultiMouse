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
            let release = MenuItemBuilder::with_id("release", "Release Control (⎋)").build(app)?;
            let disconnect = MenuItemBuilder::with_id("disconnect", "Disconnect").build(app)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = Menu::with_items(app, &[&show, &release, &disconnect, &sep, &quit])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("MultiMouse")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_window(app),
                    "release" => emergency_release(app),
                    "disconnect" => emergency_disconnect(app),
                    "quit" => {
                        shutdown_services(app);
                        std::process::exit(0);
                    }
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

            // Periodically refresh monitor info so edge detection stays accurate when
            // displays are plugged/unplugged. 5s is cheap and updates feel instant.
            let app_mon = app.handle().clone();
            let state_mon = state.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                refresh_monitors(&app_mon, &state_mon);
            });

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
            commands::start_trackpad,
            commands::stop_trackpad,
            commands::get_trackpad_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MultiMouse");
}

fn refresh_monitors(app: &AppHandle, state: &Arc<AppState>) {
    let Some(win) = app.get_webview_window("main") else { return };
    let primary = win.primary_monitor().ok().flatten();
    let Ok(available) = win.available_monitors() else { return };
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

/// Graceful shutdown: deregister mDNS and drop its daemon so other devices on the LAN
/// see us disappear immediately instead of waiting for the service TTL to expire.
fn shutdown_services(app: &AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        if let Some(mdns) = state.mdns.lock().take() {
            let _ = mdns.shutdown();
        }
        // Best-effort disconnect from any active peer
        state.send_net(network::protocol::NetCommand::Disconnect);
    }
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

/// Tray-menu "Release Control" — stops input forwarding and returns cursor to
/// this machine. Works even when the mouse is stuck relaying to the remote.
fn emergency_release(app: &AppHandle) {
    use tauri::Emitter;
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.set_relaying(false);
        *state.relay_entry.lock() = None;
        state.send_net(network::protocol::NetCommand::FocusReleased);
        // Warp local cursor to center so user can find it
        let monitors = state.monitors.read().clone();
        let (min_x, min_y, max_x, max_y) = screen::layout::virtual_bounds(&monitors);
        let cx = ((min_x + max_x) / 2.0) as i32;
        let cy = ((min_y + max_y) / 2.0) as i32;
        input::inject::warp_abs(cx, cy);
        let _ = app.emit("focus-released", ());
        tracing::info!("Emergency release from tray");
    }
}

/// Tray-menu "Disconnect" — fully drops the session.
fn emergency_disconnect(app: &AppHandle) {
    use tauri::Emitter;
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.mark_intentional_disconnect();
        state.set_relaying(false);
        *state.relay_entry.lock() = None;
        state.send_net(network::protocol::NetCommand::Disconnect);
        *state.connected_peer.lock() = None;
        *state.net_tx.lock() = None;
        let monitors = state.monitors.read().clone();
        let (min_x, min_y, max_x, max_y) = screen::layout::virtual_bounds(&monitors);
        input::inject::warp_abs(((min_x + max_x) / 2.0) as i32, ((min_y + max_y) / 2.0) as i32);
        let _ = app.emit("disconnected", ());
        tracing::info!("Emergency disconnect from tray");
    }
}
