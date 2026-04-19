use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use once_cell::sync::OnceCell;
use tauri::{AppHandle, Emitter};
use crate::network::protocol::InputEvent;

/// Local-only inject commands (not serialized on the wire). Protocol-level events
/// come in as InputEvent::Remote; trackpad/UI-level commands use the other variants.
pub enum InjectCmd {
    Remote(InputEvent),
    MoveRel { dx: i32, dy: i32 },
    MoveAbs { x: i32, y: i32 },
    Scroll { dx: i32, dy: i32 },
    Button { button: u8, pressed: bool },
    Text(String),
}

/// Bounded injector queue. An unbounded mpsc::channel allowed the queue to
/// grow arbitrarily under clipboard/input floods; 1024 is ample for normal
/// use and drops cleanly when a peer misbehaves.
const INJECT_QUEUE_CAP: usize = 1024;

static INJECT_TX: OnceCell<SyncSender<InjectCmd>> = OnceCell::new();
static INJECT_READY: AtomicBool = AtomicBool::new(false);
// Rate-limit the "queue full" warning so a flood doesn't spam the log.
static LAST_FULL_WARN_MS: AtomicU64 = AtomicU64::new(0);
/// Unix-ms of the last time we actually injected an OS event. Used by
/// `capture.rs` on the receiver to distinguish our own injection (which rdev
/// sees back through CGEventTap) from the local user's real hardware input.
static LAST_INJECT_MS: AtomicU64 = AtomicU64::new(0);

/// Was an inject done within the last `window_ms` milliseconds? If so, a
/// contemporaneous rdev event is probably our echo rather than user input.
pub fn recently_injected(window_ms: u64) -> bool {
    let last = LAST_INJECT_MS.load(Ordering::Relaxed);
    if last == 0 { return false; }
    let now = start_ms();
    now.saturating_sub(last) <= window_ms
}

/// Primary display's scale factor, stored as f64 bits in a u64 atomic so the
/// inject thread can read it without a mutex. Updated from
/// `refresh_monitors` on the main thread.
static PRIMARY_SCALE_BITS: AtomicU64 = AtomicU64::new(f64::to_bits(1.0));

/// Called by the monitor refresh path whenever primary display changes, so
/// the inject thread knows how to convert logical wire coords to the
/// physical pixels SetCursorPos expects on Windows.
pub fn set_primary_scale(sf: f64) {
    PRIMARY_SCALE_BITS.store(sf.to_bits(), Ordering::Relaxed);
}

/// Convert logical wire coords to the coordinate unit `enigo.move_mouse` wants
/// on this platform. macOS's CGWarp takes logical points as-is; Windows's
/// SetCursorPos takes physical pixels so we scale up.
#[inline]
fn logical_to_inject_xy(x: f64, y: f64) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        let sf = f64::from_bits(PRIMARY_SCALE_BITS.load(Ordering::Relaxed));
        let sf = if sf.is_finite() && sf > 0.0 { sf } else { 1.0 };
        (x * sf, y * sf)
    }
    #[cfg(not(target_os = "windows"))]
    { (x, y) }
}

fn mark_injected() {
    LAST_INJECT_MS.store(start_ms(), Ordering::Relaxed);
}

fn start_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn start_injector(app: AppHandle) {
    let (tx, rx) = mpsc::sync_channel::<InjectCmd>(INJECT_QUEUE_CAP);
    INJECT_TX.set(tx).ok();
    // Enigo is not Send on macOS (holds raw CGEventSource pointers), so we
    // construct it inside the worker thread. A oneshot-like `init_tx` sends
    // the result back so we can synchronously mark the injector ready or
    // emit the `inject-unavailable` event for the UI.
    let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    std::thread::spawn(move || {
        let mut enigo = match Enigo::new(&Settings::default()) {
            Ok(e) => {
                let _ = init_tx.send(Ok(()));
                e
            }
            Err(e) => {
                let msg = format!("{:?}", e);
                let _ = init_tx.send(Err(msg));
                return;
            }
        };
        while let Ok(cmd) = rx.recv() {
            inject_cmd(&mut enigo, cmd);
        }
        INJECT_READY.store(false, Ordering::SeqCst);
    });

    match init_rx.recv() {
        Ok(Ok(())) => {
            INJECT_READY.store(true, Ordering::SeqCst);
        }
        Ok(Err(msg)) => {
            tracing::error!("Failed to create Enigo: {}", msg);
            INJECT_READY.store(false, Ordering::SeqCst);
            let _ = app.emit(
                "inject-unavailable",
                serde_json::json!({ "error": msg }),
            );
        }
        Err(_) => {
            // Worker thread died before sending a status.
            INJECT_READY.store(false, Ordering::SeqCst);
            let _ = app.emit(
                "inject-unavailable",
                serde_json::json!({ "error": "injector thread did not start" }),
            );
        }
    }
}

/// Returns whether the injector thread is up and accepting commands.
pub fn is_ready() -> bool {
    INJECT_READY.load(Ordering::SeqCst) && INJECT_TX.get().is_some()
}

fn try_send(cmd: InjectCmd) {
    let Some(tx) = INJECT_TX.get() else { return };
    match tx.try_send(cmd) {
        Ok(_) => {}
        Err(TrySendError::Full(_)) => {
            let now = start_ms();
            let last = LAST_FULL_WARN_MS.load(Ordering::Relaxed);
            // Throttle the full-queue warning to once per second.
            if now.saturating_sub(last) > 1000 {
                LAST_FULL_WARN_MS.store(now, Ordering::Relaxed);
                tracing::warn!("Inject queue full; dropping command");
            }
        }
        Err(TrySendError::Disconnected(_)) => {
            INJECT_READY.store(false, Ordering::SeqCst);
        }
    }
}

pub fn process_event(event: InputEvent) {
    if !is_ready() { return; }
    try_send(InjectCmd::Remote(event));
}

pub fn inject_move_rel(dx: i32, dy: i32) {
    if !is_ready() { return; }
    try_send(InjectCmd::MoveRel { dx, dy });
}

/// Warp the LOCAL cursor to an absolute position. Used on relay activation to
/// free the cursor from the screen edge so subsequent mouse motion generates
/// real delta events rather than being clamped.
pub fn warp_abs(x: i32, y: i32) {
    if !is_ready() { return; }
    try_send(InjectCmd::MoveAbs { x, y });
}

pub fn inject_scroll(dx: i32, dy: i32) {
    if !is_ready() { return; }
    try_send(InjectCmd::Scroll { dx, dy });
}

pub fn inject_button(button: u8, pressed: bool) {
    if !is_ready() { return; }
    try_send(InjectCmd::Button { button, pressed });
}

pub fn inject_text(text: String) {
    if !is_ready() { return; }
    try_send(InjectCmd::Text(text));
}


fn inject_cmd(enigo: &mut Enigo, cmd: InjectCmd) {
    // Stamp before the enigo call (same reason as `inject`): the synthetic
    // event can be observed by rdev during the call, so the timestamp has
    // to be in place before the syscall lands.
    if !matches!(cmd, InjectCmd::Remote(_)) {
        mark_injected();
    }
    match cmd {
        InjectCmd::Remote(event) => inject(enigo, event),
        InjectCmd::MoveRel { dx, dy } => {
            let _ = enigo.move_mouse(dx, dy, Coordinate::Rel);
        }
        InjectCmd::MoveAbs { x, y } => {
            let (fx, fy) = logical_to_inject_xy(x as f64, y as f64);
            let _ = enigo.move_mouse(fx as i32, fy as i32, Coordinate::Abs);
        }
        InjectCmd::Scroll { dx, dy } => {
            if dy != 0 {
                let _ = enigo.scroll(dy, enigo::Axis::Vertical);
            }
            if dx != 0 {
                let _ = enigo.scroll(dx, enigo::Axis::Horizontal);
            }
        }
        InjectCmd::Button { button, pressed } => {
            let btn = match button {
                0 => Button::Left,
                1 => Button::Right,
                2 => Button::Middle,
                _ => return,
            };
            let dir = if pressed { Direction::Press } else { Direction::Release };
            let _ = enigo.button(btn, dir);
        }
        InjectCmd::Text(text) => {
            let _ = enigo.text(&text);
        }
    }
    // NOTE: mark_injected is now called *before* each enigo call at the
    // top of the match arms (inside `inject`), not at the end. The receiver's
    // rdev callback can fire synchronously during the enigo call — if we
    // marked AFTER, the receiver's `recently_injected()` check would see a
    // stale timestamp and misclassify the injected echo as local user input.
}

fn inject(enigo: &mut Enigo, event: InputEvent) {
    // Stamp BEFORE the enigo call so the receiver's capture thread —
    // which may observe the synthetic event synchronously from the OS
    // event tap during the call — always reads a fresh LAST_INJECT_MS.
    mark_injected();
    match event {
        InputEvent::MouseMove { x, y } => {
            // Wire coords are LOGICAL pixels. enigo on macOS uses
            // CGWarpMouseCursorPosition which takes logical points directly,
            // but enigo on Windows uses SetCursorPos which takes PHYSICAL
            // pixels. Multiply by scale_factor on Windows so cursor covers
            // the full desktop rather than only 1/sf of it.
            let (fx, fy) = logical_to_inject_xy(x, y);
            let _ = enigo.move_mouse(fx as i32, fy as i32, Coordinate::Abs);
        }
        InputEvent::MouseButton { button, pressed } => {
            let btn = match button {
                0 => Button::Left,
                1 => Button::Right,
                2 => Button::Middle,
                #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
                3 => Button::Back,
                #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
                4 => Button::Forward,
                _ => return,
            };
            let dir = if pressed { Direction::Press } else { Direction::Release };
            let _ = enigo.button(btn, dir);
        }
        InputEvent::MouseScroll { dx, dy } => {
            if dy != 0 {
                let _ = enigo.scroll(dy as i32, enigo::Axis::Vertical);
            }
            if dx != 0 {
                let _ = enigo.scroll(dx as i32, enigo::Axis::Horizontal);
            }
        }
        InputEvent::Key { key, pressed } => {
            let dir = if pressed { Direction::Press } else { Direction::Release };
            if let Some(k) = rdev_key_to_enigo(&key) {
                let _ = enigo.key(k, dir);
            }
        }
    }
}

fn rdev_key_to_enigo(key_str: &str) -> Option<Key> {
    Some(match key_str {
        // Letters
        "KeyA" => Key::Unicode('a'),
        "KeyB" => Key::Unicode('b'),
        "KeyC" => Key::Unicode('c'),
        "KeyD" => Key::Unicode('d'),
        "KeyE" => Key::Unicode('e'),
        "KeyF" => Key::Unicode('f'),
        "KeyG" => Key::Unicode('g'),
        "KeyH" => Key::Unicode('h'),
        "KeyI" => Key::Unicode('i'),
        "KeyJ" => Key::Unicode('j'),
        "KeyK" => Key::Unicode('k'),
        "KeyL" => Key::Unicode('l'),
        "KeyM" => Key::Unicode('m'),
        "KeyN" => Key::Unicode('n'),
        "KeyO" => Key::Unicode('o'),
        "KeyP" => Key::Unicode('p'),
        "KeyQ" => Key::Unicode('q'),
        "KeyR" => Key::Unicode('r'),
        "KeyS" => Key::Unicode('s'),
        "KeyT" => Key::Unicode('t'),
        "KeyU" => Key::Unicode('u'),
        "KeyV" => Key::Unicode('v'),
        "KeyW" => Key::Unicode('w'),
        "KeyX" => Key::Unicode('x'),
        "KeyY" => Key::Unicode('y'),
        "KeyZ" => Key::Unicode('z'),
        // Numbers row
        "Num0" => Key::Unicode('0'),
        "Num1" => Key::Unicode('1'),
        "Num2" => Key::Unicode('2'),
        "Num3" => Key::Unicode('3'),
        "Num4" => Key::Unicode('4'),
        "Num5" => Key::Unicode('5'),
        "Num6" => Key::Unicode('6'),
        "Num7" => Key::Unicode('7'),
        "Num8" => Key::Unicode('8'),
        "Num9" => Key::Unicode('9'),
        // Numpad
        "Kp0" => Key::Unicode('0'),
        "Kp1" => Key::Unicode('1'),
        "Kp2" => Key::Unicode('2'),
        "Kp3" => Key::Unicode('3'),
        "Kp4" => Key::Unicode('4'),
        "Kp5" => Key::Unicode('5'),
        "Kp6" => Key::Unicode('6'),
        "Kp7" => Key::Unicode('7'),
        "Kp8" => Key::Unicode('8'),
        "Kp9" => Key::Unicode('9'),
        "KpMinus" => Key::Unicode('-'),
        "KpPlus" => Key::Unicode('+'),
        "KpMultiply" => Key::Unicode('*'),
        "KpDivide" => Key::Unicode('/'),
        "KpDecimal" => Key::Unicode('.'),
        "KpReturn" => Key::Return,
        // Punctuation & symbols
        "Minus" => Key::Unicode('-'),
        "Equal" => Key::Unicode('='),
        "LeftBracket" => Key::Unicode('['),
        "RightBracket" => Key::Unicode(']'),
        "BackSlash" => Key::Unicode('\\'),
        "SemiColon" => Key::Unicode(';'),
        "Quote" => Key::Unicode('\''),
        "BackQuote" => Key::Unicode('`'),
        "Comma" => Key::Unicode(','),
        "Dot" => Key::Unicode('.'),
        "Slash" => Key::Unicode('/'),
        // Control keys
        "Return" => Key::Return,
        "Escape" => Key::Escape,
        "BackSpace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Space" => Key::Space,
        "CapsLock" => Key::CapsLock,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "Insert" => Key::Insert,
        "Delete" => Key::Delete,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "PrintScreen" => Key::Print,
        #[cfg(all(unix, not(target_os = "macos")))]
        "ScrollLock" => Key::ScrollLock,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "Pause" => Key::Pause,
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
        "NumLock" => Key::Numlock,
        // Modifiers
        "ShiftLeft" | "ShiftRight" => Key::Shift,
        "ControlLeft" | "ControlRight" => Key::Control,
        "MetaLeft" | "MetaRight" => Key::Meta,
        "Alt" | "AltGr" => Key::Alt,
        // Function keys
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "F13" => Key::F13,
        "F14" => Key::F14,
        "F15" => Key::F15,
        "F16" => Key::F16,
        "F17" => Key::F17,
        "F18" => Key::F18,
        "F19" => Key::F19,
        "F20" => Key::F20,
        // Arrow keys
        "UpArrow" => Key::UpArrow,
        "DownArrow" => Key::DownArrow,
        "LeftArrow" => Key::LeftArrow,
        "RightArrow" => Key::RightArrow,
        _ => return None,
    })
}
