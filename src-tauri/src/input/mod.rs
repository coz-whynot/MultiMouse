pub mod active_window;
pub mod capture;
pub mod inject;

#[cfg(target_os = "macos")]
pub mod raw_mouse_mac;
#[cfg(target_os = "windows")]
pub mod raw_mouse_win;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod fullscreen_detect;

// Re-export the platform's RelayGuard under a single name so state.rs and
// capture.rs can use `crate::input::RelayGuard` regardless of OS. Both
// implementations expose the same public surface: `activate(Arc<AppState>)`
// returning `Self`, and a `Drop` impl that tears everything down.
#[cfg(target_os = "macos")]
pub use raw_mouse_mac::RelayGuard;
#[cfg(target_os = "windows")]
pub use raw_mouse_win::RelayGuard;

/// Returns true if the given rdev-formatted key name is a *modifier* key
/// whose held state can visibly affect the peer's behaviour when left
/// un-released (typing in ALL CAPS, Cmd+X shortcuts firing, Alt-menu
/// activation, etc.). Used by Phase 6 (release-all-on-disconnect).
///
/// Matches the exact string produced by `format!("{:?}", rdev::Key)` for
/// the modifier variants. Non-modifier keys (letters, F-keys, etc.) are
/// excluded — they can technically stick but the OS typically handles
/// single-key stuck-state cheaply on its own.
pub fn is_modifier_key_name(name: &str) -> bool {
    matches!(
        name,
        "ShiftLeft" | "ShiftRight"
            | "ControlLeft" | "ControlRight"
            | "MetaLeft" | "MetaRight"
            | "Alt" | "AltGr"
    )
}
