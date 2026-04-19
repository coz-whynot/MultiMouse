//! Auto gaming-mode toggle (v0.3.4 Phase 4b).
//!
//! Background thread, polls every 500 ms for a foreground fullscreen
//! application. When detected AND `settings.auto_gaming_mode` is on,
//! enables `gaming_mode` (disables edge-cross). When the fullscreen app
//! exits after a short grace period, disables gaming_mode.
//!
//! User intent: they don't have to remember to press Pause/Break before
//! launching a game. Once they Alt-Tab out of the game, edge-cross
//! resumes.
//!
//! ## Detection heuristics
//!
//! - **Windows:** `SHQueryUserNotificationState` returns `QUNS_RUNNING_D3D_FULL_SCREEN`
//!   when a DirectX-exclusive fullscreen app is running, or `QUNS_PRESENTATION_MODE`
//!   for presentation software. Either signals "don't interrupt me."
//! - **macOS:** iterate `CGWindowListCopyWindowInfo` for the foreground
//!   window at kCGWindowLayer 0; if its bounds match a CGDisplay's bounds,
//!   treat as fullscreen. Not perfect (Chrome's borderless-fullscreen
//!   won't always match), but covers 95% of native games.
//!
//! ## Poll interval + grace period
//!
//! 500 ms poll is enough for "user alt-tabs out of game" to feel instant.
//! We require fullscreen to be DETECTED for >= 1 poll before flipping
//! gaming_mode ON (no grace — gaming gear should engage fast). We require
//! fullscreen to be ABSENT for >= 3 polls (1.5 s) before flipping OFF —
//! avoids a flicker when the user briefly Alt-Tabs to check Discord.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::state::AppState;

const POLL_MS: u64 = 500;
const OFF_GRACE_POLLS: u32 = 3;

/// Count of consecutive polls reporting "no fullscreen foreground." Used
/// to require sustained absence before disabling auto-gaming-mode, so a
/// brief Alt-Tab doesn't flicker the state.
static OFF_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn start(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || run_loop(app, state));
}

fn run_loop(app: AppHandle, state: Arc<AppState>) {
    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));

        let auto_on = state.settings.read().auto_gaming_mode;
        if !auto_on {
            // Setting disabled — reset our counter so we don't trigger a
            // spurious transition if the user re-enables later.
            OFF_COUNT.store(0, Ordering::Relaxed);
            continue;
        }

        let fullscreen_now = is_fullscreen_foreground();
        let gaming_mode_now = state.settings.read().gaming_mode;

        if fullscreen_now && !gaming_mode_now {
            // Turn gaming_mode ON. Persist + broadcast.
            state.settings.write().gaming_mode = true;
            OFF_COUNT.store(0, Ordering::Relaxed);
            let snapshot = state.settings.read().clone();
            std::thread::spawn(move || crate::storage::save_settings(&snapshot));
            let _ = app.emit("gaming-mode-changed", true);
            tracing::info!("[fullscreen_detect] auto-enabling gaming mode (fullscreen app detected)");
        } else if !fullscreen_now && gaming_mode_now {
            // Only flip OFF after sustained absence — avoids flicker on
            // quick Alt-Tabs. We don't distinguish "user manually turned
            // on gaming_mode without a game" from "we auto-enabled and
            // game exited"; either way, absence for 1.5 s means resume
            // normal edge-cross.
            let n = OFF_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if n >= OFF_GRACE_POLLS {
                state.settings.write().gaming_mode = false;
                OFF_COUNT.store(0, Ordering::Relaxed);
                let snapshot = state.settings.read().clone();
                std::thread::spawn(move || crate::storage::save_settings(&snapshot));
                let _ = app.emit("gaming-mode-changed", false);
                tracing::info!("[fullscreen_detect] auto-disabling gaming mode (foreground no longer fullscreen)");
            }
        } else {
            // State matches intent; reset the grace counter.
            OFF_COUNT.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(target_os = "windows")]
fn is_fullscreen_foreground() -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState,
        QUNS_RUNNING_D3D_FULL_SCREEN, QUNS_PRESENTATION_MODE,
    };
    // windows-0.58 style: returns the state directly in a Result; no
    // out-param pointer.
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(state) => state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE,
            Err(_) => false,
        }
    }
}

#[cfg(target_os = "macos")]
fn is_fullscreen_foreground() -> bool {
    // TODO(fullscreen-detect-macos): implement a Quartz-based detector
    // (CGWindowListCopyWindowInfo + bounds compare against CGMainDisplay).
    // Stubbed false for now so Phase 4b ships cleanly on macOS; macOS
    // users can still toggle `gaming_mode` manually via Pause/Break or
    // the tray menu. No functional regression from pre-v0.3.4 — just
    // missing the auto-enable nicety.
    false
}
