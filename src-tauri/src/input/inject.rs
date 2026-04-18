use std::sync::mpsc::{self, Sender};
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use once_cell::sync::OnceCell;
use crate::network::protocol::InputEvent;

/// Local-only inject commands (not serialized on the wire). Protocol-level events
/// come in as InputEvent::Remote; trackpad/UI-level commands use the other variants.
pub enum InjectCmd {
    Remote(InputEvent),
    MoveRel { dx: i32, dy: i32 },
    Scroll { dx: i32, dy: i32 },
    Button { button: u8, pressed: bool },
    Text(String),
}

static INJECT_TX: OnceCell<Sender<InjectCmd>> = OnceCell::new();

pub fn start_injector() {
    let (tx, rx) = mpsc::channel::<InjectCmd>();
    INJECT_TX.set(tx).ok();

    std::thread::spawn(move || {
        let mut enigo = match Enigo::new(&Settings::default()) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to create Enigo: {:?}", e);
                return;
            }
        };
        while let Ok(cmd) = rx.recv() {
            inject_cmd(&mut enigo, cmd);
        }
    });
}

pub fn process_event(event: InputEvent) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(InjectCmd::Remote(event));
    }
}

pub fn inject_move_rel(dx: i32, dy: i32) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(InjectCmd::MoveRel { dx, dy });
    }
}

pub fn inject_scroll(dx: i32, dy: i32) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(InjectCmd::Scroll { dx, dy });
    }
}

pub fn inject_button(button: u8, pressed: bool) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(InjectCmd::Button { button, pressed });
    }
}

pub fn inject_text(text: String) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(InjectCmd::Text(text));
    }
}

fn inject_cmd(enigo: &mut Enigo, cmd: InjectCmd) {
    match cmd {
        InjectCmd::Remote(event) => inject(enigo, event),
        InjectCmd::MoveRel { dx, dy } => {
            let _ = enigo.move_mouse(dx, dy, Coordinate::Rel);
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
}

fn inject(enigo: &mut Enigo, event: InputEvent) {
    match event {
        InputEvent::MouseMove { x, y } => {
            let _ = enigo.move_mouse(x as i32, y as i32, Coordinate::Abs);
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
