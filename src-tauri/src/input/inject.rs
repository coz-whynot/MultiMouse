use std::sync::mpsc::{self, Sender};
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use once_cell::sync::OnceCell;
use crate::network::protocol::InputEvent;

static INJECT_TX: OnceCell<Sender<InputEvent>> = OnceCell::new();

pub fn start_injector() {
    let (tx, rx) = mpsc::channel::<InputEvent>();
    INJECT_TX.set(tx).ok();

    std::thread::spawn(move || {
        let mut enigo = match Enigo::new(&Settings::default()) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to create Enigo: {:?}", e);
                return;
            }
        };
        while let Ok(event) = rx.recv() {
            inject(&mut enigo, event);
        }
    });
}

pub fn process_event(event: InputEvent) {
    if let Some(tx) = INJECT_TX.get() {
        let _ = tx.send(event);
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
        "Return" => Key::Return,
        "Escape" => Key::Escape,
        "BackSpace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Space" => Key::Space,
        "ShiftLeft" | "ShiftRight" => Key::Shift,
        "ControlLeft" | "ControlRight" => Key::Control,
        "MetaLeft" | "MetaRight" => Key::Meta,
        "Alt" | "AltGr" => Key::Alt,
        "UpArrow" => Key::UpArrow,
        "DownArrow" => Key::DownArrow,
        "LeftArrow" => Key::LeftArrow,
        "RightArrow" => Key::RightArrow,
        "Delete" => Key::Delete,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
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
        _ => return None,
    })
}
