mod commands;
mod crypto;
mod input;
mod network;
mod screen;
mod state;
mod storage;

use std::path::PathBuf;
use std::sync::Arc;
use once_cell::sync::OnceCell;
use tauri::{
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use state::AppState;

// Held for the lifetime of the process so the background log-writer thread
// keeps running. Dropping this guard would flush and close the writer.
static LOG_GUARD: OnceCell<WorkerGuard> = OnceCell::new();
// Absolute path of the current run's log file, if file logging was set up
// successfully. Read by the tray "Show Log File" menu item.
static LOG_PATH: OnceCell<Option<PathBuf>> = OnceCell::new();

fn log_dir() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("MultiMouse").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Public path accessor used by the tray "Show Log File" menu item.
pub fn log_file_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned().flatten()
}

/// Initialise tracing with two layers: stderr (for `cargo run` / dev) and a
/// non-blocking file writer at `<AppData>/MultiMouse/logs/multimouse.log`.
///
/// Behaviour choices, and why:
/// - **Rename-before-create**: on startup we rename the previous run's
///   `multimouse.log` to `multimouse.log.prev`. Single file per run keeps
///   "send me your log" unambiguous and bounds disk use (max 2 files).
/// - **Non-blocking writer with `lossy = true`**: any tracing call from the
///   hot rdev capture / inject threads must never block on disk. Under a
///   tracing flood we drop events rather than stall input latency.
/// - **Fallback**: if anything about the file path fails (denied disk,
///   readonly volume), we silently fall back to stderr-only — the app still
///   runs, the tray menu item just won't have a path to open.
fn init_logging() {
    // EnvFilter matches on `::` boundaries — not raw prefixes — so
    // `multimouse=debug` does NOT match the `multimouse_lib::…` targets
    // that all application code actually emits under (the Cargo.toml
    // binary is `multimouse` but the library crate where every module
    // lives is `multimouse_lib`). Without the `multimouse_lib=debug`
    // directive, every `tracing::debug!` from the app is silently
    // filtered out — this was the reason v0.3.7 logs had zero DEBUG
    // events even with the filter nominally at `debug`. Final `warn`
    // is the global default for everything else (tungstenite, mdns_sd,
    // …). Env var still overrides for ad-hoc debugging.
    let filter_str = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "multimouse_lib=debug,multimouse=debug,warn".to_string());
    let env_filter = EnvFilter::try_new(&filter_str)
        .unwrap_or_else(|_| EnvFilter::new("warn"));

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let file_path: Option<PathBuf> = log_dir().map(|dir| {
        let current = dir.join("multimouse.log");
        let prev = dir.join("multimouse.log.prev");
        if current.exists() {
            let _ = std::fs::rename(&current, &prev);
        }
        current
    });

    if let Some(path) = file_path {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let (nb, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
                .lossy(true)
                .finish(file);
            // Only publish the path after we've confirmed the file actually
            // opened — the tray "Show Log File" handler reveals LOG_PATH in
            // Finder/Explorer, and showing a ghost path is worse than
            // showing nothing (the menu item logs "no log file set" and
            // does nothing, which is a clearer signal that file logging
            // is not available this run).
            let _ = LOG_PATH.set(Some(path));
            let _ = LOG_GUARD.set(guard);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(nb)
                .with_ansi(false);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(stderr_layer)
                .with(file_layer)
                .init();
            return;
        }
    }

    let _ = LOG_PATH.set(None);
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    // If the previous process crashed mid-session via SIGKILL / force-quit
    // (panic hook and Drop chains can't recover from those), the user's
    // cursor may still be hidden and/or disassociated. A preemptive
    // restore at startup undoes the damage before any session begins.
    // Both calls are no-ops if state is already normal.
    #[cfg(target_os = "macos")]
    input::raw_mouse_mac::recover_from_previous_crash();
    #[cfg(target_os = "windows")]
    input::raw_mouse_win::recover_from_previous_crash();

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

            // v5: monitor geometry stored in PHYSICAL virtual-desktop pixels.
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
                                    x: pos.x as i32,
                                    y: pos.y as i32,
                                    width: sz.width,
                                    height: sz.height,
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
            let logs = MenuItemBuilder::with_id("logs", "Show Log File").build(app)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = Menu::with_items(
                app,
                &[&show, &release, &disconnect, &gaming, &logs, &sep, &quit],
            )?;

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
                    "logs" => reveal_log_file(),
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

            input::inject::start_injector(app.handle().clone(), state.clone());
            input::capture::start(app.handle().clone(), state.clone());
            // Auto-gaming-mode polling — detects foreground fullscreen
            // apps and toggles `gaming_mode` when the user has opted in
            // via `settings.auto_gaming_mode`. Skipped on Linux until we
            // add an X11/Wayland detector.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            input::fullscreen_detect::start(app.handle().clone(), state.clone());
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
            commands::open_log_file,
            commands::copy_log_to_clipboard,
            commands::export_diagnostics_bundle,
            commands::request_peer_logs,
            commands::accept_log_request,
            commands::reject_log_request,
            commands::get_peer_app_version,
            commands::get_debug_state,
            commands::clear_all_cooldowns,
            commands::force_dial_peer,
            commands::get_log_tail,
            commands::run_diagnostics,
            commands::pull_peer_dev_state,
            commands::get_peer_dev_state,
            commands::get_input_grab_status,
            commands::open_input_permissions,
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
            // v5: PHYSICAL virtual-desktop pixels (see commands::get_monitors).
            let sf = m.scale_factor().max(1e-6);
            let pos = m.position();
            let sz = m.size();
            state::MonitorInfo {
                name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                x: pos.x as i32,
                y: pos.y as i32,
                width: sz.width,
                height: sz.height,
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

/// Tray-menu "Show Log File" — reveals the current run's log file in Finder /
/// Explorer so the user can attach it to a GitHub issue without knowing the
/// filesystem path. Uses `reveal_item_in_dir` instead of `open_path` so the
/// user can also see `multimouse.log.prev` (the previous run's rotated log)
/// sitting alongside the current one — the previous run is usually the one
/// that reproduced the bug.
fn reveal_log_file() {
    let Some(path) = log_file_path() else {
        tracing::warn!("Show Log File clicked but no log file path is set");
        return;
    };
    if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(&path) {
        tracing::error!("reveal_item_in_dir({}) failed: {}", path.display(), e);
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
