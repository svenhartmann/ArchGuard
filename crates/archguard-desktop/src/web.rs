//! "Open in web viewer": start `archguard serve` once per project root,
//! read the URL it prints and open the requested tab in the browser.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// Running servers by project root, with their base URL.
static SERVERS: Mutex<BTreeMap<PathBuf, (Child, String)>> = Mutex::new(BTreeMap::new());

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

/// Base URL of the server for `root`, starting it on first use.
fn server_url(root: &Path) -> Result<String, String> {
    let mut servers = SERVERS
        .lock()
        .map_err(|_| "server lock poisoned".to_string())?;
    if let Some((child, url)) = servers.get_mut(root)
        && child.try_wait().ok().flatten().is_none()
    {
        return Ok(url.clone());
    }
    let (child, url) = start(root)?;
    servers.insert(root.to_path_buf(), (child, url.clone()));
    Ok(url)
}

fn start(root: &Path) -> Result<(Child, String), String> {
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
    Ok((child, url))
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

/// Stop the server of `root` started by [`open`], if any.
pub fn stop(root: &Path) {
    let server = SERVERS.lock().ok().and_then(|mut s| s.remove(root));
    if let Some((mut child, _)) = server {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Stop every server started by [`open`].
pub fn shutdown() {
    let servers = SERVERS.lock().map(|mut s| std::mem::take(&mut *s));
    for (_, (mut child, _)) in servers.unwrap_or_default() {
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
