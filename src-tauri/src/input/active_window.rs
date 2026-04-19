/// Returns the name of the currently foreground application, or None.
pub fn current_app() -> Option<String> {
    #[cfg(target_os = "macos")]
    return macos::current();
    #[cfg(target_os = "windows")]
    return windows::current();
    #[cfg(target_os = "linux")]
    return linux::current();
    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::Command;
    pub fn current() -> Option<String> {
        // osascript is cheap (~5ms) and doesn't need extra crates
        let out = Command::new("osascript")
            .args(["-e", "tell application \"System Events\" to get name of first application process whose frontmost is true"])
            .output()
            .ok()?;
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    pub fn current() -> Option<String> {
        // TODO: GetForegroundWindow + GetWindowText via winapi — stub for now
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    pub fn current() -> Option<String> { None }
}
