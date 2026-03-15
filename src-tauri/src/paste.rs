use std::process::Command;

/// Check if accessibility permission is granted (macOS)
pub fn is_accessibility_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .arg("-e")
            .arg("tell application \"System Events\" to return true")
            .output();
        matches!(output, Ok(o) if o.status.success())
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Prompt the user to grant accessibility permission (macOS)
pub fn request_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        // Open System Preferences to Accessibility pane
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .output();
        is_accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Simulate Cmd+V to paste clipboard content into the active app
pub fn simulate_paste() {
    #[cfg(target_os = "macos")]
    {
        std::thread::sleep(std::time::Duration::from_millis(80));
        let _ = Command::new("osascript")
            .arg("-e")
            .arg("tell application \"System Events\" to keystroke \"v\" using command down")
            .output();
    }
}
