//! "Open in web viewer": start `archguard serve` for the project once,
//! read the URL it prints and open the requested tab in the browser.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static SERVER: Mutex<Option<(Child, String)>> = Mutex::new(None);

/// Route of the web viewer for a desktop tab id (`viewer.js`,
/// `parseViewerRoute`).
pub fn route(tab: &str) -> String {
    let tab = match tab {
        "start" | "architecture" | "graph" | "dsm" | "calls" | "services" | "quality"
        | "security" => tab,
        _ => "graph",
    };
    format!("#tab={tab}")
}

/// The `archguard` binary: `ARCHGUARD_BIN`, else next to this
/// executable, else from `PATH`.
fn binary() -> PathBuf {
    if let Some(bin) = std::env::var_os("ARCHGUARD_BIN") {
        return bin.into();
    }
    let name = if cfg!(windows) {
        "archguard.exe"
    } else {
        "archguard"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| name.into())
}

/// Base URL of the running server, starting it on first use.
fn server_url(root: &Path) -> Result<String, String> {
    let mut server = SERVER
        .lock()
        .map_err(|_| "server lock poisoned".to_string())?;
    if let Some((child, url)) = server.as_mut()
        && child.try_wait().ok().flatten().is_none()
    {
        return Ok(url.clone());
    }
    let bin = binary();
    let mut child = Command::new(&bin)
        .arg("serve")
        .arg(root)
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("no stdout from archguard serve")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    // Keep draining stdout so later output cannot block or kill the server.
    std::thread::spawn(move || for _ in reader.lines() {});
    let url = line.trim().to_string();
    if !url.starts_with("http://") {
        let _ = child.kill();
        return Err(format!("archguard serve printed no URL: {url:?}"));
    }
    *server = Some((child, url.clone()));
    Ok(url)
}

fn open_in_browser(url: &str) -> Result<(), String> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("browser opener exited with {s}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Blocking: call from a worker thread. Returns the opened URL.
pub fn open(root: &Path, tab: &str) -> Result<String, String> {
    let url = format!("{}{}", server_url(root)?, route(tab));
    open_in_browser(&url)?;
    Ok(url)
}

/// Stop the server started by [`open`], if any.
pub fn shutdown() {
    if let Ok(mut server) = SERVER.lock()
        && let Some((mut child, _)) = server.take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_follow_the_web_viewer() {
        assert_eq!(route("dsm"), "#tab=dsm");
        assert_eq!(route("security"), "#tab=security");
        assert_eq!(route("unknown"), "#tab=graph");
    }
}
