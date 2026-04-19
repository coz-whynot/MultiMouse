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
    // NSWorkspace.frontmostApplication is a public AppKit API and does NOT
    // require Automation / Apple Events permission, unlike osascript.
    use objc2_app_kit::NSWorkspace;
    pub fn current() -> Option<String> {
        unsafe {
            let ws = NSWorkspace::sharedWorkspace();
            let app = ws.frontmostApplication()?;
            let name = app.localizedName()?;
            Some(name.to_string())
        }
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
