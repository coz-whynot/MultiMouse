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
/// Throttle mouse-move to ~500 Hz max (2 ms between sends). Tight enough for
/// gaming input (most gaming mice poll at 1 kHz); going lower costs bandwidth
/// without meaningful accuracy gain given the wire is encrypted TCP.
const MOUSE_MOVE_INTERVAL_MS: u128 = 2;
/// After a release, block edge activation for this long so the cursor has time
/// to pull away from the edge without immediately re-triggering relay.
const RELEASE_DEBOUNCE_MS: u128 = 800;
/// Cursor-tracker emit throttle for the Developer panel. Real mouse motion
/// fires at up to 1 kHz; shipping each event to the webview is wasteful. 10
/// Hz is visible and cheap.
const DEV_CURSOR_INTERVAL_MS: u128 = 100;

/// Emit a timestamped diagnostic event to the Developer panel — but ONLY
/// when `settings.developer_mode` is true. When off, this is a single
/// read-lock check and an early return, so instrumenting hot paths with
/// it is safe.
///
/// Event shape: `{ "ts": <unix_ms>, "kind": <str>, "detail": <any> }`.
/// The UI listens for `"mm-dev-event"` and maintains a rolling list.
pub(crate) fn dev_event(
    app: &AppHandle,
    state: &AppState,
    kind: &'static str,
    detail: serde_json::Value,
) {
    if !state.settings.read().developer_mode { return; }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ev = serde_json::json!({
        "ts": ts,
        "kind": kind,
        "detail": detail,
    });
    // Keep a bounded ring so the cross-PC DevStateShare reply can
    // include the recent timeline. 50 entries is roughly a minute of
    // interesting activity without blowing the 2 MiB frame cap.
    {
        let mut ring = state.dev_events.lock();
        if ring.len() >= 50 { ring.pop_front(); }
        ring.push_back(ev.clone());
    }
    let _ = app.emit("mm-dev-event", ev);
}

/// Build the JSON payload used by `Message::DevStateShare` — a
/// best-effort snapshot of live debug flags plus the recent event ring.
/// Returns `(state_json, events_json)` as strings so the wire format
/// stays stable even if the internal shape of debug state evolves.
pub(crate) fn snapshot_dev_state_and_events(state: &AppState) -> (String, String) {
    use std::sync::atomic::Ordering;
    let now = std::time::Instant::now();
    let connected_peer = state.connected_peer.lock().clone();
    let has_net_tx = state.net_tx.lock().is_some();
    let is_relaying = state.is_relaying();
    let is_controlled = state.is_controlled();
    let last_activity_s_ago = state.last_activity.lock().elapsed().as_secs();
    let session_duration_s = state.session_start.lock().map(|t| t.elapsed().as_secs());
    let peer_app_version = state.peer_app_version.lock().clone();
    let cooldowns: Vec<serde_json::Value> = state
        .peer_cooldowns
        .lock()
        .iter()
        .filter_map(|(pid, until)| {
            let r = until.saturating_duration_since(now).as_secs();
            if r == 0 { None } else { Some(serde_json::json!({ "peer_id": pid, "remaining_s": r })) }
        })
        .collect();
    let (edge, dwell, gaming) = {
        let s = state.settings.read();
        (s.transition_edge.clone(), s.edge_dwell_ms, s.gaming_mode)
    };
    let state_value = serde_json::json!({
        "connected_peer": connected_peer,
        "has_net_tx": has_net_tx,
        "can_edge_cross": has_net_tx && !is_controlled,
        "is_relaying": is_relaying,
        "is_controlled": is_controlled,
        "last_activity_s_ago": last_activity_s_ago,
        "session_duration_s": session_duration_s,
        "peer_app_version": peer_app_version,
        "peer_cooldowns": cooldowns,
        "transition_edge": edge,
        "edge_dwell_ms": dwell,
        "gaming_mode": gaming,
        "bytes_in": state.bytes_received.load(Ordering::Relaxed),
        "bytes_out": state.bytes_sent.load(Ordering::Relaxed),
    });
    let events_vec: Vec<serde_json::Value> = state.dev_events.lock().iter().cloned().collect();
    (
        serde_json::to_string(&state_value).unwrap_or_default(),
        serde_json::to_string(&events_vec).unwrap_or_default(),
    )
}

/// 10 Hz cursor-position tracker for the Developer panel. Only fires when
/// `settings.developer_mode` is on — the hot-path cost when off is a
/// single RwLock read plus an Instant compare. Uses a thread_local Cell
/// so the 100 ms throttle doesn't need a mutex.
pub(crate) fn dev_cursor_track(app: &AppHandle, state: &AppState, x: f64, y: f64) {
    if !state.settings.read().developer_mode { return; }
    thread_local! {
        static LAST_EMIT: Cell<Option<Instant>> = Cell::new(None);
    }
    let now = Instant::now();
    let should_emit = LAST_EMIT.with(|cell| {
        let ok = cell.get().map_or(true, |t| now.duration_since(t).as_millis() >= DEV_CURSOR_INTERVAL_MS);
        if ok { cell.set(Some(now)); }
        ok
    });
    if !should_emit { return; }
    let _ = app.emit("mm-dev-cursor", serde_json::json!({
        "x": x,
        "y": y,
        "is_relaying": state.is_relaying(),
        "is_controlled": state.is_controlled(),
    }));
}

/// Convert rdev's raw mouse coords into our canonical wire unit: **physical
/// virtual-desktop pixels of the sender** (v5).
///
/// - **Windows:** rdev hooks report physical virtual-screen coords natively
///   under PER_MONITOR_V2 (which Tauri 2 embeds via tao's manifest).
///   Pass-through.
/// - **macOS:** rdev reports logical points (global display coordinate space).
///   Multiply by the primary display's backing scale factor. Multi-monitor
///   Mac with mixed sf across displays is a known limitation — we'd need to
///   pick the screen under `(x, y)` and use *its* sf. See
///   `TODO(multimon-mac-capture)`.
/// - **Linux:** unit is fuzzy on X11/Wayland; we pass-through with sf=1.0 to
///   preserve existing behavior.
#[inline]
fn rdev_to_physical_xy(x: f64, y: f64, state: &AppState) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        let _ = state;
        (x, y)
    }
    #[cfg(target_os = "macos")]
    {
        // TODO(multimon-mac-capture): look up per-screen sf if multi-display.
        let sf = state
            .monitors
            .read()
            .iter()
            .find(|m| m.is_primary)
            .map(|m| m.scale_factor)
            .unwrap_or(1.0)
            .max(1e-6);
        (x * sf, y * sf)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = state;
        (x, y)
    }
}

// NOTE: edge-dwell + release-debounce timestamps are stored on AppState
// (state.edge_first_touch, state.last_release) instead of thread_local!.
// That way every disconnect path can reset them via state.reset_edge_state(),
// which prevents stale values from triggering instant relay or a skipped
// dwell window immediately after a reconnect.

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

/// Parse a key-name string (e.g. `"F13"`, `"ScrollLock"`) into the matching
/// `rdev::Key`. Returns None for empty or unknown names. Kept intentionally
/// narrow — only keys a user would bind as a "switch control" hotkey. The
/// full rdev Key enum is large and most variants aren't useful here (letters
/// would conflict with normal typing, for example).
fn parse_switch_hotkey_name(name: &str) -> Option<rdev::Key> {
    use rdev::Key as K;
    Some(match name.trim() {
        "F1"  => K::F1,  "F2"  => K::F2,  "F3"  => K::F3,  "F4"  => K::F4,
        "F5"  => K::F5,  "F6"  => K::F6,  "F7"  => K::F7,  "F8"  => K::F8,
        "F9"  => K::F9,  "F10" => K::F10, "F11" => K::F11, "F12" => K::F12,
        // Rare extended function keys, often present on full keyboards.
        "ScrollLock" | "Scroll_Lock" => {
            // rdev exposes Key::ScrollLock only on non-mac Unix; fall back
            // to a platform-agnostic detectable alternative by returning
            // None on macOS. macOS users should bind an F-key instead.
            #[cfg(all(unix, not(target_os = "macos")))]
            { K::ScrollLock }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            { return None }
        }
        _ => return None,
    })
}

/// If `event` is a `KeyPress` matching one of the user's configured
/// switch hotkeys AND gaming_mode is currently on, toggle which machine
/// has the mouse and return true (caller consumes the event). Returns
/// false otherwise — event flows through normal handling.
///
/// Takes `state: &Arc<AppState>` rather than `&AppState` so we can clone
/// an Arc for the platform `RelayGuard` when activating.
fn try_handle_switch_hotkey(event: &Event, state: &Arc<AppState>, app: &AppHandle) -> bool {
    let EventType::KeyPress(pressed) = &event.event_type else { return false };
    let settings = state.settings.read();
    // Hotkey is a no-op outside gaming mode. Normal edge-cross handles the
    // "mouse follows cursor" flow when gaming mode is off.
    if !settings.gaming_mode {
        return false;
    }
    // Honour at most 9 configured bindings; silently ignore the rest to
    // keep the hot path bounded.
    let bindings = settings.switch_hotkeys.iter().take(9).cloned().collect::<Vec<_>>();
    drop(settings);
    let matched = bindings
        .iter()
        .filter_map(|name| parse_switch_hotkey_name(name))
        .any(|k| &k == pressed);
    if !matched {
        return false;
    }

    // Toggle: if we're currently relaying, release; otherwise try to
    // activate. Activation requires an established peer connection.
    if state.is_relaying() {
        state.set_relaying(false);
        *state.relay_entry.lock() = None;
        state.send_net(NetCommand::FocusReleased);
        sync_clipboard_async(state);
        *state.last_release.lock() = Some(Instant::now());
        let _ = app.emit("focus-released", ());
        tracing::info!("Relay released via switch hotkey ({:?})", pressed);
        dev_event(app, state, "relay_off", serde_json::json!({ "via": "switch_hotkey" }));
    } else {
        if state.net_tx.lock().is_none() || state.connected_peer.lock().is_none() {
            // No peer to switch to — ignore rather than surprising the user.
            tracing::debug!("Switch hotkey pressed but no peer connected; ignoring");
            return true;
        }
        // Synthetic activation: pretend the cursor is at the transition
        // edge so compute_entry_point produces the right remote coords.
        let edge = state.settings.read().transition_edge.clone();
        let monitors = state.monitors.read().clone();
        let (bx0, by0, bx1, by1) = layout::virtual_bounds(&monitors);
        let (fake_x, fake_y) = match edge.as_str() {
            "left"   => (bx0, (by0 + by1) / 2.0),
            "top"    => ((bx0 + bx1) / 2.0, by0),
            "bottom" => ((bx0 + bx1) / 2.0, by1 - 1.0),
            _        => (bx1 - 1.0, (by0 + by1) / 2.0), // right default
        };
        let (entry_x, entry_y) = compute_entry_point(fake_x, fake_y, &edge, &monitors, state);
        let _ = (fake_x, fake_y); // anchor no longer stored on state
        *state.relay_entry.lock() = Some((entry_x, entry_y));
        state.set_relaying(true);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            *state.relay_guard.lock() = Some(
                crate::input::RelayGuard::activate(state.clone())
            );
        }
        state.send_net(NetCommand::Input(InputEvent::MouseMove { x: entry_x, y: entry_y }));
        state.send_net(NetCommand::FocusAcquired);
        sync_clipboard_async(state);
        let _ = app.emit("focus-acquired", ());
        tracing::info!("Relay activated via switch hotkey ({:?})", pressed);
        dev_event(app, state, "relay_on", serde_json::json!({ "via": "switch_hotkey" }));
    }
    true
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
    *state.edge_first_touch.lock() = None;
    let snapshot = state.settings.read().clone();
    std::thread::spawn(move || crate::storage::save_settings(&snapshot));
    let _ = app.emit("gaming-mode-changed", enabled);
    tracing::info!("Gaming mode {}", if enabled { "ON" } else { "OFF" });
    true
}

pub fn start(app: AppHandle, state: Arc<AppState>) {
    // On macOS and Windows we use rdev::grab (CGEventTap / low-level hook)
    // so we can consume events — required to avoid "cursor moves on both
    // machines simultaneously" when relaying. On Linux, this fork's grab
    // isn't exposed (no `unstable_grab` feature in fufesou/rdev), so we
    // fall back to listen-only which observes events without consuming.
    // Linux KVM-behaviour will be less tight than mac/win until we wire a
    // native X11/Wayland grab path.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let state_grab = state.clone();
        let app_grab = app.clone();
        std::thread::spawn(move || {
            // macOS only: the grab loop runs on this std::thread — NOT the
            // process main thread. Inside rdev's convert() it builds the
            // event's `unicode` field via TSMGetInputSourceProperty, which
            // Apple asserts must be called from the main dispatch queue.
            // The fufesou fork supports bouncing that call via the main
            // queue *only* when `is_main_thread == false`; it defaults to
            // true, so unless we flip it we get `_dispatch_assert_queue_fail`
            // and a SIGTRAP on every keyboard event. This was the
            // "disconnecting again and again" crash in v0.2.2–v0.2.6.
            #[cfg(target_os = "macos")]
            rdev::set_is_main_thread(false);
            let result = rdev::grab(move |event: Event| -> Option<Event> {
                handle_grab(event, &state_grab, &app_grab)
            });
            if let Err(e) = result {
                tracing::warn!("rdev::grab unavailable ({:?}), using listen fallback", e);
                // v0.3.12: record the failure on state so the UI can poll
                // it at any time and show the permissions banner — the
                // `accessibility-needed` event below only reaches a webview
                // that already mounted its listener, which is racy at app
                // startup.
                state.input_grab_ok.store(false, std::sync::atomic::Ordering::SeqCst);
                // v0.3.15: ad-hoc-signed macOS builds get a fresh code
                // signature on every auto-update, which leaves orphaned
                // TCC entries that silently match the OLD binary instead
                // of the current one. Auto-running `tccutil reset` here
                // clears the stale entries so the user only has to re-add
                // MultiMouse to Accessibility + Input Monitoring once —
                // no more manual-terminal-command step after each update.
                // Safe to run repeatedly: it's idempotent when there's
                // nothing to clear, and it only touches THIS app's bundle
                // id (no admin needed).
                #[cfg(target_os = "macos")]
                macos_clear_stale_tcc_entries();
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

    // Belt-and-suspenders: if the process panics during a relay session
    // we must undo the OS-level cursor changes, otherwise the user's
    // cursor stays hidden (Windows) or decoupled (macOS) until reboot.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let existing = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            #[cfg(target_os = "macos")]
            crate::input::raw_mouse_mac::deactivate();
            #[cfg(target_os = "windows")]
            crate::input::raw_mouse_win::deactivate();
            existing(info);
        }));
    }

    #[cfg(target_os = "linux")]
    {
        let state_listen = state.clone();
        let app_listen = app.clone();
        std::thread::spawn(move || {
            let _ = rdev::listen(move |event: Event| {
                handle_listen(&event, &state_listen, &app_listen);
            });
        });
    }
}

fn handle_grab(event: Event, state: &Arc<AppState>, app: &AppHandle) -> Option<Event> {
    // Global toggle: Pause/Break key flips gaming mode regardless of relay state.
    // Consumed so games don't receive it.
    if try_toggle_gaming_mode(&event, state, app) {
        return None;
    }

    // Switch-hotkey handling (v0.3.4 Phase 4c). ONLY active when gaming_mode
    // is on — outside gaming mode, edge-cross works freely and a hotkey
    // would be redundant. When active: pressing any of the configured
    // `switch_hotkeys` toggles which machine has the mouse, bypassing
    // edge-dwell. Consumed so the game doesn't receive it.
    if try_handle_switch_hotkey(&event, state, app) {
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
            *state.last_release.lock() = Some(Instant::now());
            let _ = app.emit("focus-released", ());
            tracing::info!("Relay released via Escape");
            dev_event(app, state, "relay_off", serde_json::json!({ "via": "escape" }));
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
                    dev_event(app, state, "relay_off", serde_json::json!({ "via": "release_hotkey", "hotkey": &hotkey }));
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
                    dev_event(app, state, "relay_off", serde_json::json!({ "via": "release_hotkey", "hotkey": &hotkey }));
                    return None;
                }
            }
        }

        // Route motion through the platform's raw-delta path on macOS and
        // Windows — skip forwarding MouseMove from rdev's grab. rdev's grab
        // still fires and still returns None below (blocking legacy delivery
        // from reaching games), but we don't ship the absolute position on
        // the wire. The HID tap (mac) / Raw Input window (win) emits
        // MouseMoveRel independently. Linux still uses the absolute path
        // until we add native delta capture there.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let skip_motion = matches!(event.event_type, EventType::MouseMove { .. });
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let skip_motion = false;

        if !skip_motion {
            // Extract sender-side layout-translated character name from
            // rdev's UnicodeInfo so the receiver has a fallback when its
            // keycode table doesn't recognise `format!("{:?}", key)`.
            let unicode_name = event.unicode.as_ref().and_then(|u| u.name.clone());
            if let Some(ev) = convert_and_remap(&event.event_type, state, unicode_name) {
                // Diagnostic trace — ship-side of the sender's keyboard
                // path. Parallels the `[mouse]` traces.
                if let InputEvent::Key { key, pressed, unicode } = &ev {
                    tracing::debug!("[key] ship {} pressed={} unicode={:?}", key, pressed, unicode);
                }
                // Phase 6 held-modifier bookkeeping: remember every
                // modifier press we ship, so we can send a release burst
                // if the session ends while the user is still holding
                // them. `is_modifier_key_name` limits the set to
                // Shift/Ctrl/Alt/Meta (the ones that cause visible stuck
                // behaviour on the peer — ALL CAPS, Cmd shortcuts, etc.).
                if let InputEvent::Key { key, pressed, .. } = &ev {
                    if crate::input::is_modifier_key_name(key) {
                        let mut held = state.held_modifiers.lock();
                        if *pressed {
                            held.insert(key.clone());
                        } else {
                            held.remove(key);
                        }
                    }
                }
                state.send_net(NetCommand::Input(ev));
            }
        }
        None
    } else {
        if let EventType::KeyPress(key) = &event.event_type {
            let hotkey = state.settings.read().hotkey_release.clone();
            if is_release_key(key, &hotkey) && !is_single_key_release(&hotkey) {
                *state.last_ctrl_press.lock() = Some(Instant::now());
            }
        }

        // While this device is being CONTROLLED by a peer, ONLY Escape
        // kicks the controller. Pre-v0.3.12 we also kicked on any keypress
        // / button-press, but that made the receiver unable to type
        // anything locally without breaking the session — a stray key on
        // Mac killed the remote from Windows instantly. Classic KVM
        // behaviour: Esc is the explicit escape hatch, other input flows
        // through to the receiver's own OS so they can type / click into
        // local apps while the peer drives the cursor.
        //
        // The recently_injected filter still gates Esc: if the controller
        // pressed Esc to release on their end, that Esc is forwarded and
        // injected here; rdev's tap sees it too, and we must NOT treat
        // that echo as a local kick — otherwise the session bounces into
        // a 5 s cooldown on every graceful release.
        if state.is_controlled() {
            if matches!(&event.event_type, EventType::KeyPress(rdev::Key::Escape))
                && !inject::recently_injected(300)
            {
                tracing::info!("Receiver pressed Escape — signaling disconnect");
                if let Some(pid) = state.connected_peer.lock().clone() {
                    state.mark_peer_kicked(&pid);
                }
                dev_event(app, state, "kick", serde_json::json!({
                    "event": format!("{:?}", event.event_type),
                }));
                state.signal_disconnect();
                return None;
            }
            // Non-Esc keypresses / button-presses while being controlled:
            // let them flow through to this machine's OS (`Some(event)`)
            // so the receiver can type/click locally — do NOT kick the
            // session. rdev's `grab` can't return the event from inside
            // the is_controlled block elegantly; the simpler fix is to
            // fall through to the rest of handle_grab which ends with
            // `Some(event)` for MouseMove and non-relay state. But for
            // non-MouseMove events we need to return `Some(event)`
            // explicitly: return early with Some to bypass the edge-dwell
            // logic below, which doesn't apply while is_controlled
            // anyway (`is_echo` short-circuits it).
            if matches!(
                &event.event_type,
                EventType::KeyPress(_) | EventType::KeyRelease(_)
                | EventType::ButtonPress(_) | EventType::ButtonRelease(_)
            ) {
                return Some(event);
            }
        }

        if let EventType::MouseMove { x, y } = event.event_type {
            // Developer cursor-tracker feed. Runs BEFORE the gaming-mode
            // short-circuit so the Developer panel can still observe cursor
            // motion while edge-cross is disabled.
            dev_cursor_track(app, state, x, y);
            let (edge, dwell_ms, gaming) = {
                let s = state.settings.read();
                (s.transition_edge.clone(), s.edge_dwell_ms as u128, s.gaming_mode)
            };
            if gaming {
                *state.edge_first_touch.lock() = None;
                return Some(event);
            }
            // Normalize rdev's raw coord into sender-physical *once*. Every
            // downstream comparison (is_at_edge, virtual_bounds, relay_entry)
            // operates in physical space post-v5.
            let (px, py) = rdev_to_physical_xy(x, y, state);
            let monitors = state.monitors.read().clone();
            // Only the CLIENT side (with a net_tx to send messages) should ever
            // activate relay. Without this guard, the server side would also
            // enter is_relaying=true, rdev::grab would start consuming events,
            // and the user's cursor would freeze with nothing being sent anywhere.
            let can_send = state.net_tx.lock().is_some();
            // While we're the RECEIVER in a session, mouse moves we see here
            // are the controller's injection echoed back by rdev's event tap.
            // Don't arm edge-activation on those — it caused a feedback
            // "rubber band" where injected moves at the edge would trigger
            // our own relay-start and pull control back mid-move.
            let is_echo = state.is_controlled() || inject::recently_injected(300);
            // Block re-activation for a short window after a release.
            let recently_released = state
                .last_release
                .lock()
                .map(|r| r.elapsed().as_millis() < RELEASE_DEBOUNCE_MS)
                .unwrap_or(false);
            let at_edge = can_send
                && !recently_released
                && !is_echo
                && layout::is_at_edge(px, py, &edge, &monitors)
                && state.connected_peer.lock().is_some();

            if at_edge {
                let now = Instant::now();
                let should_activate = {
                    let mut guard = state.edge_first_touch.lock();
                    match *guard {
                        None => {
                            *guard = Some(now);
                            dev_event(app, state, "edge_touch", serde_json::json!({
                                "edge": edge, "x": px, "y": py, "dwell_ms": dwell_ms,
                            }));
                            false
                        }
                        Some(first) => now.duration_since(first).as_millis() >= dwell_ms,
                    }
                };
                if should_activate {
                    *state.edge_first_touch.lock() = None;
                    let (entry_x, entry_y) = compute_entry_point(px, py, &edge, &monitors, state);

                    // Linux only still needs the warp-to-center: its
                    // absolute-delta fallback in `convert_and_remap` reads
                    // cursor positions from rdev, which would otherwise
                    // clamp at the screen edge after the activation motion.
                    // macOS and Windows capture HID-level deltas directly,
                    // so there's nothing to "un-stick" — skip the warp.
                    #[cfg(target_os = "linux")]
                    {
                        let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(&monitors);
                        let center_x = (min_x + max_x) / 2.0;
                        let center_y = (min_y + max_y) / 2.0;
                        inject::warp_abs(center_x as i32, center_y as i32);
                        let _ = (center_x, center_y);
                    }
                    // `relay_entry` now holds only the remote entry point.
                    // The local anchor (used on Linux's fallback path) is a
                    // thread_local inside `convert_and_remap` that
                    // re-seeds from the first post-activation rdev event.
                    let _ = px; let _ = py;
                    *state.relay_entry.lock() = Some((entry_x, entry_y));

                    state.set_relaying(true);
                    // macOS: HID tap + CGAssociate(false). Windows: Raw
                    // Input hidden window. Both expose `crate::input::RelayGuard`
                    // so we install through a single name. Must happen
                    // AFTER set_relaying(true) because the capture layer
                    // short-circuits on !is_relaying as a defensive check.
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    {
                        *state.relay_guard.lock() = Some(
                            crate::input::RelayGuard::activate(state.clone())
                        );
                    }
                    // Warp remote cursor to the entry point before activating control
                    state.send_net(NetCommand::Input(InputEvent::MouseMove { x: entry_x, y: entry_y }));
                    state.send_net(NetCommand::FocusAcquired);
                    sync_clipboard_async(state);
                    let _ = app.emit("focus-acquired", ());
                    tracing::debug!("Relay ON — {} edge, entry=({entry_x:.0}, {entry_y:.0})", edge);
                    dev_event(app, state, "relay_on", serde_json::json!({
                        "edge": edge,
                        "entry_x": entry_x, "entry_y": entry_y,
                        "via": "edge_dwell",
                    }));
                }
            } else {
                // Cursor left the edge before dwell completed — clear the timer.
                // Emit a dev event only if a timer was actually armed.
                if state.edge_first_touch.lock().is_some() {
                    dev_event(app, state, "edge_leave", serde_json::json!({ "edge": edge }));
                }
                *state.edge_first_touch.lock() = None;
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
                    dev_event(app, state, "relay_off", serde_json::json!({ "via": "release_hotkey", "hotkey": &hotkey }));
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
                    dev_event(app, state, "relay_off", serde_json::json!({ "via": "release_hotkey", "hotkey": &hotkey }));
                    return;
                }
            }
        }

        let unicode_name = event.unicode.as_ref().and_then(|u| u.name.clone());
        if let Some(ev) = convert_and_remap(&event.event_type, state, unicode_name) {
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
            dev_cursor_track(app, state, x, y);
            let (edge, dwell_ms, gaming) = {
                let s = state.settings.read();
                (s.transition_edge.clone(), s.edge_dwell_ms as u128, s.gaming_mode)
            };
            if gaming {
                *state.edge_first_touch.lock() = None;
                return;
            }
            // v5: normalize to sender-physical before any edge/bounds math.
            let (px, py) = rdev_to_physical_xy(x, y, state);
            let monitors = state.monitors.read().clone();
            let at_edge = layout::is_at_edge(px, py, &edge, &monitors)
                && state.connected_peer.lock().is_some();

            if at_edge {
                let now = Instant::now();
                let should_activate = {
                    let mut guard = state.edge_first_touch.lock();
                    match *guard {
                        None => {
                            *guard = Some(now);
                            dev_event(app, state, "edge_touch", serde_json::json!({
                                "edge": edge, "x": px, "y": py, "dwell_ms": dwell_ms,
                            }));
                            false
                        }
                        Some(first) => now.duration_since(first).as_millis() >= dwell_ms,
                    }
                };
                if should_activate {
                    *state.edge_first_touch.lock() = None;
                    let (entry_x, entry_y) = compute_entry_point(px, py, &edge, &monitors, state);
                    // Linux listen-fallback still warps local cursor to
                    // center so `convert_and_remap`'s absolute-delta path
                    // has room to track motion without hitting screen edge.
                    let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(&monitors);
                    let center_x = (min_x + max_x) / 2.0;
                    let center_y = (min_y + max_y) / 2.0;
                    inject::warp_abs(center_x as i32, center_y as i32);
                    let _ = (center_x, center_y);
                    *state.relay_entry.lock() = Some((entry_x, entry_y));
                    state.set_relaying(true);
                    state.send_net(NetCommand::Input(InputEvent::MouseMove { x: entry_x, y: entry_y }));
                    state.send_net(NetCommand::FocusAcquired);
                    sync_clipboard_async(state);
                    dev_event(app, state, "relay_on", serde_json::json!({
                        "edge": edge,
                        "entry_x": entry_x, "entry_y": entry_y,
                        "via": "edge_dwell_listen",
                    }));
                }
            } else {
                if state.edge_first_touch.lock().is_some() {
                    dev_event(app, state, "edge_leave", serde_json::json!({ "edge": edge }));
                }
                *state.edge_first_touch.lock() = None;
            }
        }
    }
}

/// Compute where the cursor should appear on the remote screen when crossing
/// an edge. The perpendicular axis is normalised within the **source monitor**
/// (the one the cursor is on), not the whole virtual desktop — the previous
/// whole-desktop normalisation placed the remote cursor in the wrong row/
/// column when the user crossed from a non-primary monitor whose height (or
/// width) differed from the primary.
///
/// Delegates to `layout::entry_point_on_monitor` (pure, unit-tested in
/// `screen::layout`) once we've resolved which monitor the cursor is on.
/// Falls back to the whole-virtual-desktop bounds if we can't find a
/// containing monitor (rare — can happen transiently during hotplug before
/// `refresh_monitors` runs, or for a synthetic edge-cross coordinate from
/// the switch-hotkey handler).
fn compute_entry_point(x: f64, y: f64, edge: &str, monitors: &[crate::state::MonitorInfo], state: &AppState) -> (f64, f64) {
    let (min_x, min_y, max_x, max_y) = layout::virtual_bounds(monitors);
    let local_w = (max_x - min_x).max(1.0);
    let local_h = (max_y - min_y).max(1.0);

    let remote = *state.remote_screen.lock();
    let (rw, rh) = remote.unwrap_or((local_w, local_h));

    let source_rect = layout::monitor_containing(x, y, monitors)
        .map(|m| (m.x as f64, m.y as f64, m.width as f64, m.height as f64))
        .unwrap_or((min_x, min_y, local_w, local_h));

    layout::entry_point_on_monitor(x, y, source_rect, edge, (rw, rh))
}

/// Convert an rdev event to an InputEvent for the network, applying
/// throttling and delta-based coordinate remapping for mouse moves when
/// in relay mode.
///
/// Post-v0.3.4 this function's `EventType::MouseMove` arm is **only
/// reached on Linux**. macOS routes motion through `raw_mouse_mac` (HID
/// tap) and Windows through `raw_mouse_win` (Raw Input); both platforms
/// filter MouseMove out via `skip_motion` in `handle_grab` so this arm
/// is dead code there. Non-motion events (buttons, keys, scroll) still
/// pass through on all platforms via the fallthrough `convert_event`
/// call.
///
/// All coords are in **sender physical virtual-desktop pixels** (v5+).
fn convert_and_remap(
    event_type: &EventType,
    state: &AppState,
    unicode: Option<String>,
) -> Option<InputEvent> {
    match event_type {
        EventType::MouseMove { x, y } => {
            // Local-anchor cache. `relay_entry` now only holds the REMOTE
            // entry point (2-tuple) — the local anchor we diff against is
            // kept in this thread_local and re-seeded every time the
            // remote entry changes (i.e., every new session). Wrapping the
            // cache as `(entry_signature, local_anchor)` self-invalidates
            // when session boundaries roll over without needing a separate
            // reset hook.
            thread_local! {
                static LAST_MOVE: Cell<Option<Instant>> = Cell::new(None);
                static LOCAL_ANCHOR: Cell<Option<(f64, f64, f64, f64)>> = Cell::new(None);
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

            let (px, py) = rdev_to_physical_xy(*x, *y, state);
            let entry = *state.relay_entry.lock();
            let remote = *state.remote_screen.lock();

            let (rx, ry) = if let Some((ex, ey)) = entry {
                // Re-seed the local anchor if the remote entry changed
                // (new session). On first event post-activation this also
                // runs and seeds to the current cursor position — meaning
                // the first `MouseMoveRel`-equivalent delta is 0 and the
                // remote stays at the entry point until the user moves.
                let (lx, ly) = LOCAL_ANCHOR.with(|c| {
                    match c.get() {
                        Some((cached_ex, cached_ey, lx, ly))
                            if cached_ex == ex && cached_ey == ey => (lx, ly),
                        _ => {
                            c.set(Some((ex, ey, px, py)));
                            (px, py)
                        }
                    }
                });
                let dx = px - lx;
                let dy = py - ly;
                let monitors = state.monitors.read().clone();
                let (bx0, by0, bx1, by1) = layout::virtual_bounds(&monitors);
                let (lw, lh) = ((bx1 - bx0).max(1.0), (by1 - by0).max(1.0));
                // Sensitivity multiplier (user setting, clamped). Folded
                // into the scale so deltas from rdev-observed motion are
                // sped up / slowed down before reaching the remote.
                let sensitivity = state.settings.read().mouse_sensitivity.clamp(0.1, 5.0);
                let (sx, sy) = if let Some((rw, rh)) = remote {
                    ((rw / lw) * sensitivity, (rh / lh) * sensitivity)
                } else {
                    (sensitivity, sensitivity)
                };
                let nx = ex + dx * sx;
                let ny = ey + dy * sy;
                if let Some((rw, rh)) = remote {
                    (nx.clamp(0.0, rw - 1.0), ny.clamp(0.0, rh - 1.0))
                } else {
                    (nx.max(0.0), ny.max(0.0))
                }
            } else {
                // No session active — drop the cached anchor so a future
                // session starts clean.
                LOCAL_ANCHOR.with(|c| c.set(None));
                (px, py)
            };

            Some(InputEvent::MouseMove { x: rx, y: ry })
        }
        other => convert_event(other, unicode),
    }
}

/// Translate rdev's raw event shape into our wire protocol. `unicode` is
/// the sender-side layout-translated character (from rdev's `Event.name`)
/// — populated for KeyPress/KeyRelease so the receiver has a fallback path
/// for keys its name-table doesn't recognise, and for any keys where the
/// sender's and receiver's keyboard layouts differ.
fn convert_event(event_type: &EventType, unicode: Option<String>) -> Option<InputEvent> {
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
            unicode,
        }),
        EventType::KeyRelease(key) => Some(InputEvent::Key {
            key: format!("{:?}", key),
            pressed: false,
            unicode,
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
            if let Ok(mut text) = ctx.get_text() {
                // Truncate oversize clipboard text on the SEND side so we never
                // force the receiver to drop the session on a large paste.
                if text.len() > crate::network::protocol::CLIPBOARD_TEXT_MAX {
                    // Find the nearest valid UTF-8 boundary at/below the cap.
                    let mut cutoff = crate::network::protocol::CLIPBOARD_TEXT_MAX;
                    while cutoff > 0 && !text.is_char_boundary(cutoff) {
                        cutoff -= 1;
                    }
                    text.truncate(cutoff);
                    tracing::warn!(
                        "Clipboard text exceeded {} bytes; truncated before send",
                        crate::network::protocol::CLIPBOARD_TEXT_MAX
                    );
                }
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

/// v0.3.15: shell out to `tccutil` to clear stale TCC entries for this
/// app's bundle id. Called at startup whenever `rdev::grab` fails, so
/// ad-hoc-signed auto-updates don't leave a pile of orphaned
/// "MultiMouse" rows in System Settings that silently match an older
/// binary. Running once per failed boot is fine — it's a no-op when
/// the list is already empty.
///
/// Only touches our own bundle id, so it doesn't require admin
/// privileges (verified via manual test). The bundle id is hardcoded
/// to match `tauri.conf.json`'s `identifier` — if that changes, this
/// string has to change too. A mismatch silently fails to clear
/// anything; the banner still fires and the user can do the manual
/// reset like before v0.3.15.
#[cfg(target_os = "macos")]
fn macos_clear_stale_tcc_entries() {
    use std::process::Command;
    const BUNDLE_ID: &str = "com.multimouse.app";
    for perm in &["Accessibility", "ListenEvent"] {
        let out = Command::new("tccutil")
            .args(["reset", perm, BUNDLE_ID])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                tracing::info!(
                    "Cleared stale TCC entries for {} (bundle={})",
                    perm, BUNDLE_ID
                );
            }
            Ok(o) => {
                tracing::warn!(
                    "tccutil reset {} failed: code={:?}, stderr={}",
                    perm,
                    o.status.code(),
                    String::from_utf8_lossy(&o.stderr)
                );
            }
            Err(e) => {
                tracing::warn!("tccutil spawn failed: {} (PATH issue?)", e);
            }
        }
    }
}
