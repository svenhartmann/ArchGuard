//! Open a URL in the default browser (security advisories, release pages).

use std::process::Command;

/// Open `url` in the default browser. On Windows `rundll32` instead of
/// `cmd /C start`, which would treat `&` in a URL as a command separator.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(windows) {
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("browser opener exited with {s}")),
        Err(e) => Err(e.to_string()),
    }
}
