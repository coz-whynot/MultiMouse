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
    AppHandle, Emitter, Manager,
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = Arc::new(AppState::new());
            app.manage(state.clone());

            // Populate monitor info at startup. All geometry is stored in
            // LOGICAL pixels to match rdev's mouse-coordinate space (see
            // Bug #14 comment in commands::get_monitors).
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
                                let sf = m.scale_factor().max(1e-6);
                                let pos = m.position();
                                let sz = m.size();
                                state::MonitorInfo {
                                    name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                                    x: ((pos.x as f64) / sf).round() as i32,
                                    y: ((pos.y as f64) / sf).round() as i32,
                                    width: ((sz.width as f64) / sf).round() as u32,
                                    height: ((sz.height as f64) / sf).round() as u32,
                                    scale_factor: sf,
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
            let gaming = MenuItemBuilder::with_id("gaming", "Toggle Gaming Mode (Pause)").build(app)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = Menu::with_items(app, &[&show, &release, &disconnect, &gaming, &sep, &quit])?;

            // `default_window_icon()` can return None on Linux AppImage where
            // the icon resource isn't always registered by tauri-build. Use the
            // bundled PNG as a last resort rather than panicking at startup.
            let mut tray_builder = TrayIconBuilder::new();
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let _tray = tray_builder
                .menu(&menu)
                .tooltip("MultiMouse")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    // "Show" always unconditionally brings the window forward —
                    // toggling caused double-click flakiness on macOS because
                    // is_visible() can lag behind actual window state for
                    // accessory-policy apps.
                    "show" => show_window(app),
                    "release" => emergency_release(app),
                    "disconnect" => emergency_disconnect(app),
                    "gaming" => toggle_gaming_mode(app),
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
                        // Left-click tray = always show (don't toggle). Users close
                        // via the window's close button; tray click is always reveal.
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;

            let app_handle = app.handle().clone();
            let state_bg = state.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(network::start_all_services(app_handle, state_bg));
            });

            input::inject::start_injector(app.handle().clone());
            input::capture::start(app.handle().clone(), state.clone());
            start_clipboard_writer(state.clone());

            // Periodically refresh monitor info so edge detection stays accurate when
            // displays are plugged/unplugged. 5s is cheap and updates feel instant.
            let app_mon = app.handle().clone();
            let state_mon = state.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                refresh_monitors(&app_mon, &state_mon);
            });

            // Deep-link handler: emit incoming multimouse:// URLs to the frontend
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_handle_dl = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        tracing::info!("Deep link: {}", url);
                        let _ = app_handle_dl.emit("deep-link", url.to_string());
                    }
                });
            }

            // Idle auto-lock checker: every 30s, if idle_lock_minutes > 0 and the
            // session has been idle for longer than that, drop the connection.
            let state_idle = state.clone();
            let app_idle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                let minutes = state_idle.settings.read().idle_lock_minutes;
                if minutes == 0 {
                    continue;
                }
                let last = *state_idle.last_activity.lock();
                if last.elapsed().as_secs() >= (minutes as u64) * 60
                    && state_idle.connected_peer.lock().is_some()
                {
                    tracing::info!("Idle auto-lock triggered ({} min)", minutes);
                    state_idle.mark_intentional_disconnect();
                    state_idle.abort_reconnect();
                    // Also wake any running server-side read loop so cleanup
                    // runs promptly (otherwise the read loop only breaks when
                    // the next frame arrives, delaying the UI event).
                    state_idle.signal_disconnect();
                    let state_for_task = state_idle.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::state::disconnect_gracefully(&state_for_task).await;
                    });
                    let _ = app_idle.emit("idle-lock-triggered", ());
                    // `disconnected` is emitted once by whichever cleanup path
                    // actually tears down the session — do not duplicate it
                    // here.
                }
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
            commands::get_bandwidth,
            commands::get_audit_log,
            commands::clear_audit_log,
            hide_window,
        ])
        .on_window_event(|window, event| {
            // Intercept the native close (Cmd+W, Cmd+Q window, OS-level close)
            // and hide to tray instead of quitting. The only path to a real
            // exit is the tray "Quit" menu item.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                hide_to_tray(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running MultiMouse");
}

/// Spawn the persistent clipboard-writer thread. One pending message at a
/// time — newer messages replace the pending one. Replaces the old pattern
/// of `std::thread::spawn` per incoming clipboard event, which caused
/// dozens of contending threads under clipboard flood.
fn start_clipboard_writer(state: Arc<AppState>) {
    let (tx, rx) = std::sync::mpsc::sync_channel::<state::ClipboardSet>(4);
    *state.clipboard_tx.lock() = Some(tx);
    std::thread::spawn(move || {
        while let Ok(mut msg) = rx.recv() {
            // Drain any queued messages — only the newest clipboard state matters.
            while let Ok(next) = rx.try_recv() {
                msg = next;
            }
            let Ok(mut ctx) = arboard::Clipboard::new() else { continue };
            match msg {
                state::ClipboardSet::Text(text) => {
                    let _ = ctx.set_text(&text);
                }
                state::ClipboardSet::Image { width, height, bytes } => {
                    let img = arboard::ImageData {
                        width: width as usize,
                        height: height as usize,
                        bytes: std::borrow::Cow::Owned(bytes),
                    };
                    let _ = ctx.set_image(img);
                }
            }
        }
    });
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
            // LOGICAL pixels (see Bug #14 note in commands::get_monitors).
            let sf = m.scale_factor().max(1e-6);
            let pos = m.position();
            let sz = m.size();
            state::MonitorInfo {
                name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                x: ((pos.x as f64) / sf).round() as i32,
                y: ((pos.y as f64) / sf).round() as i32,
                width: ((sz.width as f64) / sf).round() as u32,
                height: ((sz.height as f64) / sf).round() as u32,
                scale_factor: sf,
                is_primary,
            }
        })
        .collect();
    *state.monitors.write() = monitors;
}

/// Graceful shutdown: flush any pending Bye to the peer, signal the trackpad
/// server to stop, deregister mDNS, and only THEN let the caller exit(0).
/// The old implementation called exit(0) immediately after a try_send,
/// which killed the writer task before it could flush Bye and killed the
/// trackpad server before it could close its sockets.
fn shutdown_services(app: &AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        let state_clone = state.inner().clone();
        state_clone.mark_intentional_disconnect();

        // Flush Bye to the peer gracefully, under a blocking runtime so we
        // can wait for the writer task without needing an async caller.
        tauri::async_runtime::block_on(async move {
            crate::state::disconnect_gracefully(&state_clone).await;
        });

        // Tell the trackpad server to close its listening sockets.
        if let Some(tx) = state.trackpad_shutdown.lock().take() {
            let _ = tx.send(());
        }

        // Deregister mDNS so peers on the LAN see us disappear immediately
        // instead of having to wait for the service TTL to expire.
        if let Some(mdns) = state.mdns.lock().take() {
            let _ = mdns.shutdown();
        }

        // Give sockets a moment to actually close on the wire.
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}


/// Unconditionally show + focus the main window. Handles the macOS accessory-policy
/// quirk where a plain show() doesn't always bring the app to the foreground.
fn show_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            // Briefly flip to Regular so the app becomes activatable, then show+focus.
            // Keep it Regular while the window is visible so macOS treats clicks normally.
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Hide the window back to the tray. On macOS this also flips the activation
/// policy back to Accessory so the app disappears from the dock.
fn hide_to_tray(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide_to_tray(&app);
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

/// Tray-menu "Toggle Gaming Mode" — flips the same flag Pause/Break toggles.
/// Keeps edge-cross from firing mid-match. Persists and notifies the frontend.
fn toggle_gaming_mode(app: &AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        let enabled = {
            let mut s = state.settings.write();
            s.gaming_mode = !s.gaming_mode;
            s.gaming_mode
        };
        let snapshot = state.settings.read().clone();
        std::thread::spawn(move || storage::save_settings(&snapshot));
        let _ = app.emit("gaming-mode-changed", enabled);
        tracing::info!("Gaming mode {} (tray)", if enabled { "ON" } else { "OFF" });
    }
}

/// Tray-menu "Disconnect" — fully drops the session, flushing Bye first.
fn emergency_disconnect(app: &AppHandle) {
    use tauri::Emitter;
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.mark_intentional_disconnect();
        state.set_relaying(false);
        *state.relay_entry.lock() = None;

        // Graceful disconnect: spawn onto the async runtime and DO NOT block
        // the tray thread (menu callbacks must return quickly).
        let state_clone = state.inner().clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::state::disconnect_gracefully(&state_clone).await;
            let monitors = state_clone.monitors.read().clone();
            let (min_x, min_y, max_x, max_y) = screen::layout::virtual_bounds(&monitors);
            input::inject::warp_abs(((min_x + max_x) / 2.0) as i32, ((min_y + max_y) / 2.0) as i32);
            let _ = app_clone.emit("disconnected", ());
        });
        tracing::info!("Emergency disconnect from tray");
    }
}
