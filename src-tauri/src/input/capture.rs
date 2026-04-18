use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;
use rdev::{Event, EventType};
use tauri::{AppHandle, Emitter};
use crate::state::AppState;
use crate::network::protocol::{InputEvent, NetCommand};
use crate::screen::layout;

const DOUBLE_CTRL_MS: u128 = 400;
/// Throttle mouse-move to ~120 Hz max (8 ms between sends)
const MOUSE_MOVE_INTERVAL_MS: u128 = 8;

pub fn start(app: AppHandle, state: Arc<AppState>) {
    let state_grab = state.clone();
    let app_grab = app.clone();

    std::thread::spawn(move || {
        let result = rdev::grab(move |event: Event| -> Option<Event> {
            handle_grab(event, &state_grab, &app_grab)
        });

        if let Err(e) = result {
            tracing::warn!("rdev::grab unavailable ({:?}), using listen fallback", e);
            let _ = app.emit("accessibility-needed", ());
            let _ = rdev::listen(move |event: Event| {
                handle_listen(&event, &state);
            });
        }
    });
}

fn handle_grab(event: Event, state: &AppState, app: &AppHandle) -> Option<Event> {
    if state.is_relaying() {
        if let EventType::KeyPress(rdev::Key::ControlLeft) = event.event_type {
            let now = Instant::now();
            let mut last = state.last_ctrl_press.lock();
            let double = last
                .map(|t| now.duration_since(t).as_millis() < DOUBLE_CTRL_MS)
                .unwrap_or(false);
            *last = Some(now);

            if double {
                state.set_relaying(false);
                *last = None;
                state.send_net(NetCommand::FocusReleased);
                sync_clipboard_async(state);
                let _ = app.emit("focus-released", ());
                tracing::info!("Relay released via double-Ctrl");
                return None;
            }
        }

        if let Some(ev) = convert_event_throttled(&event.event_type) {
            state.send_net(NetCommand::Input(ev));
        }
        None
    } else {
        if let EventType::KeyPress(rdev::Key::ControlLeft) = event.event_type {
            *state.last_ctrl_press.lock() = Some(Instant::now());
        }

        if let EventType::MouseMove { x, y } = event.event_type {
            let edge = state.settings.read().transition_edge.clone();
            let monitors = state.monitors.read().clone();
            if layout::is_at_edge(x, y, &edge, &monitors)
                && state.connected_peer.lock().is_some()
            {
                state.set_relaying(true);
                state.send_net(NetCommand::FocusAcquired);
                sync_clipboard_async(state);
                let _ = app.emit("focus-acquired", ());
                tracing::debug!("Relay ON — cursor at {} edge ({x:.0}, {y:.0})", edge);
            }
        }

        Some(event)
    }
}

fn handle_listen(event: &Event, state: &AppState) {
    if state.is_relaying() {
        if let Some(ev) = convert_event_throttled(&event.event_type) {
            state.send_net(NetCommand::Input(ev));
        }
    } else if let EventType::MouseMove { x, y } = event.event_type {
        let edge = state.settings.read().transition_edge.clone();
        let monitors = state.monitors.read().clone();
        if layout::is_at_edge(x, y, &edge, &monitors) && state.connected_peer.lock().is_some() {
            state.set_relaying(true);
            state.send_net(NetCommand::FocusAcquired);
        }
    }
}

/// Convert and throttle mouse moves; all other events pass through immediately.
fn convert_event_throttled(event_type: &EventType) -> Option<InputEvent> {
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
                if ok {
                    last.set(Some(now));
                }
                ok
            });
            if should_send {
                Some(InputEvent::MouseMove { x: *x, y: *y })
            } else {
                None
            }
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
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                if let Ok(text) = ctx.get_text() {
                    let _ = tx.try_send(NetCommand::ClipboardText(text));
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
