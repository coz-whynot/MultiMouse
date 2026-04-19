use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;
use rdev::{Event, EventType};
use tauri::{AppHandle, Emitter};
use crate::state::AppState;
use crate::network::protocol::{InputEvent, NetCommand};
use crate::screen::layout;
use crate::input::inject;

const DOUBLE_CTRL_MS: u128 = 400;
/// Throttle mouse-move to ~120 Hz max (8 ms between sends)
const MOUSE_MOVE_INTERVAL_MS: u128 = 8;
/// After a release, block edge activation for this long so the cursor has time
/// to pull away from the edge without immediately re-triggering relay.
const RELEASE_DEBOUNCE_MS: u128 = 800;

thread_local! {
    static EDGE_FIRST_TOUCH: Cell<Option<Instant>> = Cell::new(None);
    static LAST_RELEASE: Cell<Option<Instant>> = Cell::new(None);
}

/// Returns true if `key` is the trigger key for the configured hotkey.
fn is_release_key(key: &rdev::Key, hotkey: &str) -> bool {
    match hotkey {
        "ctrl+ctrl"   => matches!(key, rdev::Key::ControlLeft),
        "shift+shift" => matches!(key, rdev::Key::ShiftLeft),
        "alt+alt"     => matches!(key, rdev::Key::Alt),
        "caps_lock"   => matches!(key, rdev::Key::CapsLock),
        _             => matches!(key, rdev::Key::ControlLeft), // fallback
    }
}

/// Returns true for hotkeys that fire on a single press (no double-press required).
fn is_single_key_release(hotkey: &str) -> bool {
    matches!(hotkey, "caps_lock")
}

/// Pause/Break toggles gaming mode (edge-cross disabled). Returns true if the
/// event was the toggle and should be consumed. Persists the new setting on a
/// background thread so we never block the input pipeline on disk I/O.
fn try_toggle_gaming_mode(event: &Event, state: &AppState, app: &AppHandle) -> bool {
    if !matches!(event.event_type, EventType::KeyPress(rdev::Key::Pause)) {
        return false;
    }
    let enabled = {
        let mut s = state.settings.write();
        s.gaming_mode = !s.gaming_mode;
        s.gaming_mode
    };
    // Clear any lingering edge-dwell so flipping off mid-game doesn't instantly fire.
    EDGE_FIRST_TOUCH.with(|t| t.set(None));
    let snapshot = state.settings.read().clone();
    std::thread::spawn(move || crate::storage::save_settings(&snapshot));
    let _ = app.emit("gaming-mode-changed", enabled);
    tracing::info!("Gaming mode {}", if enabled { "ON" } else { "OFF" });
    true
}

pub fn start(app: AppHandle, state: Arc<AppState>) {
    let state_grab = state.clone();
    let app_grab = app.clone();

    std::thread::spawn(move || {
        let result = rdev::grab(move |event: Event| -> Option<Event> {
            handle_grab(event, &state_grab, &app_grab)
        });

        if let Err(e) = result {
            tracing::warn!("rdev::grab unavailable ({:?}), using listen fallback", e);
            let platform = if cfg!(target_os = "macos") { "macos" }
                           else if cfg!(target_os = "windows") { "windows" }
                           else { "linux" };
            let _ = app.emit("accessibility-needed", serde_json::json!({ "platform": platform }));
            let app_listen = app.clone();
            let _ = rdev::listen(move |event: Event| {
                handle_listen(&event, &state, &app_listen);
            });
        }
    });
}

fn handle_grab(event: Event, state: &AppState, app: &AppHandle) -> Option<Event> {
    // Global toggle: Pause/Break key flips gaming mode regardless of relay state.
    // Consumed so games don't receive it.
    if try_toggle_gaming_mode(&event, state, app) {
        return None;
    }

    if state.is_relaying() {
        // Escape = immediate release while relaying. Single-press so users
        // can always bail out instantly, even when the cursor is stuck.
        if let EventType::KeyPress(rdev::Key::Escape) = &event.event_type {
            state.set_relaying(false);
            *state.relay_entry.lock() = None;
            state.send_net(NetCommand::FocusReleased);
            let monitors = state.monitors.read().clone();
            let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(&monitors);
            inject::warp_abs(((min_x + max_x) / 2.0) as i32, ((min_y + max_y) / 2.0) as i32);
            sync_clipboard_async(state);
            LAST_RELEASE.with(|t| t.set(Some(Instant::now())));
            let _ = app.emit("focus-released", ());
            tracing::info!("Relay released via Escape");
            return None;
        }
        if let EventType::KeyPress(key) = &event.event_type {
            let hotkey = state.settings.read().hotkey_release.clone();
            if is_release_key(key, &hotkey) {
                if is_single_key_release(&hotkey) {
                    // Single press triggers release immediately (e.g. caps_lock)
                    state.set_relaying(false);
                    *state.relay_entry.lock() = None;
                    *state.last_ctrl_press.lock() = None;
                    state.send_net(NetCommand::FocusReleased);
                    sync_clipboard_async(state);
                    let _ = app.emit("focus-released", ());
                    tracing::info!("Relay released via hotkey ({})", hotkey);
                    return None;
                }
                // Double-press logic
                let now = Instant::now();
                let mut last = state.last_ctrl_press.lock();
                let double = last
                    .map(|t| now.duration_since(t).as_millis() < DOUBLE_CTRL_MS)
                    .unwrap_or(false);
                *last = Some(now);

                if double {
                    state.set_relaying(false);
                    *state.relay_entry.lock() = None;
                    *last = None;
                    state.send_net(NetCommand::FocusReleased);
                    sync_clipboard_async(state);
                    let _ = app.emit("focus-released", ());
                    tracing::info!("Relay released via hotkey ({})", hotkey);
                    return None;
                }
            }
        }

        if let Some(ev) = convert_and_remap(&event.event_type, state) {
            state.send_net(NetCommand::Input(ev));
        }
        None
    } else {
        if let EventType::KeyPress(key) = &event.event_type {
            let hotkey = state.settings.read().hotkey_release.clone();
            if is_release_key(key, &hotkey) && !is_single_key_release(&hotkey) {
                *state.last_ctrl_press.lock() = Some(Instant::now());
            }
        }

        // If this device is currently being CONTROLLED by a peer (state.is_controlled),
        // Escape key triggers a forced disconnect so the user can grab their mouse back.
        // We can't send a message back (server-side has no net_tx), so we break the
        // connection entirely — client will see disconnect and stop sending events.
        if state.is_controlled() {
            if let EventType::KeyPress(rdev::Key::Escape) = &event.event_type {
                tracing::info!("Receiver pressed Escape — signaling disconnect");
                state.signal_disconnect();
                return None;
            }
        }

        if let EventType::MouseMove { x, y } = event.event_type {
            let (edge, dwell_ms, gaming) = {
                let s = state.settings.read();
                (s.transition_edge.clone(), s.edge_dwell_ms as u128, s.gaming_mode)
            };
            if gaming {
                EDGE_FIRST_TOUCH.with(|t| t.set(None));
                return Some(event);
            }
            let monitors = state.monitors.read().clone();
            // Only the CLIENT side (with a net_tx to send messages) should ever
            // activate relay. Without this guard, the server side would also
            // enter is_relaying=true, rdev::grab would start consuming events,
            // and the user's cursor would freeze with nothing being sent anywhere.
            let can_send = state.net_tx.lock().is_some();
            // Block re-activation for a short window after a release.
            let recently_released = LAST_RELEASE.with(|t| {
                t.get().map(|r| r.elapsed().as_millis() < RELEASE_DEBOUNCE_MS).unwrap_or(false)
            });
            let at_edge = can_send
                && !recently_released
                && layout::is_at_edge(x, y, &edge, &monitors)
                && state.connected_peer.lock().is_some();

            if at_edge {
                let now = Instant::now();
                let should_activate = EDGE_FIRST_TOUCH.with(|t| {
                    match t.get() {
                        None => { t.set(Some(now)); false }
                        Some(first) => now.duration_since(first).as_millis() >= dwell_ms,
                    }
                });
                if should_activate {
                    EDGE_FIRST_TOUCH.with(|t| t.set(None));
                    let (entry_x, entry_y) = compute_entry_point(x, y, &edge, &monitors, state);

                    // Warp the local cursor off the edge to the screen center so the OS
                    // stops clamping subsequent mouse motion (otherwise we'd only ever
                    // see x=edge on the next event and the delta would be zero).
                    let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(&monitors);
                    let center_x = (min_x + max_x) / 2.0;
                    let center_y = (min_y + max_y) / 2.0;
                    inject::warp_abs(center_x as i32, center_y as i32);

                    // Delta anchor is the CENTER (where cursor now lives after the warp),
                    // not the edge. This keeps the delta math numerically sane.
                    *state.relay_entry.lock() = Some((center_x, center_y, entry_x, entry_y));
                    state.set_relaying(true);
                    // Warp remote cursor to the entry point before activating control
                    state.send_net(NetCommand::Input(InputEvent::MouseMove { x: entry_x, y: entry_y }));
                    state.send_net(NetCommand::FocusAcquired);
                    sync_clipboard_async(state);
                    let _ = app.emit("focus-acquired", ());
                    tracing::debug!("Relay ON — cursor at {} edge ({x:.0}, {y:.0}) → warped local to center ({center_x:.0}, {center_y:.0}), remote entry ({entry_x:.0}, {entry_y:.0})", edge);
                }
            } else {
                EDGE_FIRST_TOUCH.with(|t| t.set(None));
            }
        }

        Some(event)
    }
}

fn handle_listen(event: &Event, state: &AppState, app: &AppHandle) {
    // Observe-only path: we can't consume the Pause key, so the game will also
    // see it. Toggle still fires so the UI/tray reflects the correct state.
    if try_toggle_gaming_mode(event, state, app) {
        return;
    }

    if state.is_relaying() {
        if let EventType::KeyPress(key) = &event.event_type {
            let hotkey = state.settings.read().hotkey_release.clone();
            if is_release_key(key, &hotkey) {
                if is_single_key_release(&hotkey) {
                    state.set_relaying(false);
                    *state.relay_entry.lock() = None;
                    *state.last_ctrl_press.lock() = None;
                    state.send_net(NetCommand::FocusReleased);
                    sync_clipboard_async(state);
                    tracing::info!("Relay released via hotkey ({})", hotkey);
                    return;
                }
                let now = Instant::now();
                let mut last = state.last_ctrl_press.lock();
                let double = last
                    .map(|t| now.duration_since(t).as_millis() < DOUBLE_CTRL_MS)
                    .unwrap_or(false);
                *last = Some(now);

                if double {
                    state.set_relaying(false);
                    *state.relay_entry.lock() = None;
                    *last = None;
                    state.send_net(NetCommand::FocusReleased);
                    sync_clipboard_async(state);
                    tracing::info!("Relay released via hotkey ({})", hotkey);
                    return;
                }
            }
        }

        if let Some(ev) = convert_and_remap(&event.event_type, state) {
            state.send_net(NetCommand::Input(ev));
        }
    } else {
        if let EventType::KeyPress(key) = &event.event_type {
            let hotkey = state.settings.read().hotkey_release.clone();
            if is_release_key(key, &hotkey) && !is_single_key_release(&hotkey) {
                *state.last_ctrl_press.lock() = Some(Instant::now());
            }
        }

        if let EventType::MouseMove { x, y } = event.event_type {
            let (edge, dwell_ms, gaming) = {
                let s = state.settings.read();
                (s.transition_edge.clone(), s.edge_dwell_ms as u128, s.gaming_mode)
            };
            if gaming {
                EDGE_FIRST_TOUCH.with(|t| t.set(None));
                return;
            }
            let monitors = state.monitors.read().clone();
            let at_edge = layout::is_at_edge(x, y, &edge, &monitors)
                && state.connected_peer.lock().is_some();

            if at_edge {
                let now = Instant::now();
                let should_activate = EDGE_FIRST_TOUCH.with(|t| {
                    match t.get() {
                        None => { t.set(Some(now)); false }
                        Some(first) => now.duration_since(first).as_millis() >= dwell_ms,
                    }
                });
                if should_activate {
                    EDGE_FIRST_TOUCH.with(|t| t.set(None));
                    let (entry_x, entry_y) = compute_entry_point(x, y, &edge, &monitors, state);
                    let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(&monitors);
                    let center_x = (min_x + max_x) / 2.0;
                    let center_y = (min_y + max_y) / 2.0;
                    inject::warp_abs(center_x as i32, center_y as i32);
                    *state.relay_entry.lock() = Some((center_x, center_y, entry_x, entry_y));
                    state.set_relaying(true);
                    state.send_net(NetCommand::Input(InputEvent::MouseMove { x: entry_x, y: entry_y }));
                    state.send_net(NetCommand::FocusAcquired);
                    sync_clipboard_async(state);
                }
            } else {
                EDGE_FIRST_TOUCH.with(|t| t.set(None));
            }
        }
    }
}

/// Compute where the cursor should appear on the remote screen when crossing an edge.
/// Uses proportional Y (or X) mapping for the perpendicular axis, and places
/// the cursor just inside the opposite edge on the remote.
fn compute_entry_point(x: f64, y: f64, edge: &str, monitors: &[crate::state::MonitorInfo], state: &AppState) -> (f64, f64) {
    let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(monitors);
    let local_w = (max_x - min_x).max(1.0);
    let local_h = (max_y - min_y).max(1.0);

    let remote = *state.remote_screen.lock();
    let (rw, rh) = remote.unwrap_or((local_w, local_h));

    match edge {
        "right"  => (1.0, ((y - min_y) / local_h * rh).clamp(0.0, rh - 1.0)),
        "left"   => (rw - 2.0, ((y - min_y) / local_h * rh).clamp(0.0, rh - 1.0)),
        "top"    => (((x - min_x) / local_w * rw).clamp(0.0, rw - 1.0), rh - 2.0),
        "bottom" => (((x - min_x) / local_w * rw).clamp(0.0, rw - 1.0), 1.0),
        _        => (1.0, ((y - min_y) / local_h * rh).clamp(0.0, rh - 1.0)),
    }
}

/// Convert an rdev event to an InputEvent for the network, applying throttling and
/// delta-based coordinate remapping for mouse moves when in relay mode.
fn convert_and_remap(event_type: &EventType, state: &AppState) -> Option<InputEvent> {
    match event_type {
        EventType::MouseMove { x, y } => {
            thread_local! {
                static LAST_MOVE: Cell<Option<Instant>> = Cell::new(None);
            }
            let now = Instant::now();
            let should_send = LAST_MOVE.with(|last| {
                let ok = last
                    .get()
                    .map(|t| now.duration_since(t).as_millis() >= MOUSE_MOVE_INTERVAL_MS)
                    .unwrap_or(true);
                if ok { last.set(Some(now)); }
                ok
            });
            if !should_send { return None; }

            let entry = *state.relay_entry.lock();
            let remote = *state.remote_screen.lock();

            let (rx, ry) = if let Some((lx, ly, ex, ey)) = entry {
                // Delta-based: track movement relative to where we entered relay mode
                let dx = x - lx;
                let dy = y - ly;
                let nx = ex + dx;
                let ny = ey + dy;
                if let Some((rw, rh)) = remote {
                    (nx.clamp(0.0, rw - 1.0), ny.clamp(0.0, rh - 1.0))
                } else {
                    (nx.max(0.0), ny.max(0.0))
                }
            } else {
                (*x, *y)
            };

            Some(InputEvent::MouseMove { x: rx, y: ry })
        }
        other => convert_event(other),
    }
}

fn convert_event(event_type: &EventType) -> Option<InputEvent> {
    match event_type {
        EventType::MouseMove { x, y } => Some(InputEvent::MouseMove { x: *x, y: *y }),
        EventType::ButtonPress(btn) => Some(InputEvent::MouseButton {
            button: button_to_u8(btn),
            pressed: true,
        }),
        EventType::ButtonRelease(btn) => Some(InputEvent::MouseButton {
            button: button_to_u8(btn),
            pressed: false,
        }),
        EventType::KeyPress(key) => Some(InputEvent::Key {
            key: format!("{:?}", key),
            pressed: true,
        }),
        EventType::KeyRelease(key) => Some(InputEvent::Key {
            key: format!("{:?}", key),
            pressed: false,
        }),
        EventType::Wheel { delta_x, delta_y } => Some(InputEvent::MouseScroll {
            dx: *delta_x,
            dy: *delta_y,
        }),
    }
}

/// Sync clipboard to remote in a background OS thread (never blocks the input capture thread).
fn sync_clipboard_async(state: &AppState) {
    let tx = state.net_tx.lock().clone();
    if let Some(tx) = tx {
        std::thread::spawn(move || {
            let Ok(mut ctx) = arboard::Clipboard::new() else { return };
            if let Ok(text) = ctx.get_text() {
                let _ = tx.try_send(NetCommand::ClipboardText(text));
                return;
            }
            if let Ok(img) = ctx.get_image() {
                // Downsample/cap: skip images larger than 4 MB raw RGBA
                let size = img.width * img.height * 4;
                if size < 4_000_000 {
                    let _ = tx.try_send(NetCommand::ClipboardImage {
                        width: img.width as u32,
                        height: img.height as u32,
                        bytes: img.bytes.into_owned(),
                    });
                }
            }
        });
    }
}

fn button_to_u8(button: &rdev::Button) -> u8 {
    match button {
        rdev::Button::Left => 0,
        rdev::Button::Right => 1,
        rdev::Button::Middle => 2,
        rdev::Button::Unknown(n) => *n as u8,
    }
}
