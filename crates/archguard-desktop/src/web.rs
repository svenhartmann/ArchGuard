//! "Open in web viewer": start `archguard serve` once per project root,
//! read the URL it prints and open the requested tab in the browser.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;

/// A project's server: being started (by the call holding `token`), or
/// running with its base URL.
enum Server {
    Starting(u64),
    Running(Child, String),
}

/// Servers by project root.
static SERVERS: Mutex<BTreeMap<PathBuf, Server>> = Mutex::new(BTreeMap::new());
static TOKENS: AtomicU64 = AtomicU64::new(0);

fn servers() -> Result<MutexGuard<'static, BTreeMap<PathBuf, Server>>, String> {
    SERVERS
        .lock()
        .map_err(|_| "server lock poisoned".to_string())
}

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
    server_url_with(root, start)
}

/// As [`server_url`] with the start step given. The lock is not held while
/// `start` waits for the server: other projects open meanwhile, and
/// [`stop`] during the start makes it stop the new server.
fn server_url_with(
    root: &Path,
    start: impl FnOnce(&Path) -> Result<(Child, String), String>,
) -> Result<String, String> {
    let token = TOKENS.fetch_add(1, Ordering::Relaxed);
    {
        let mut servers = servers()?;
        match servers.get_mut(root) {
            Some(Server::Running(child, url)) => {
                if child.try_wait().ok().flatten().is_none() {
                    return Ok(url.clone());
                }
            }
            Some(Server::Starting(_)) => return Err("archguard serve is starting".into()),
            None => {}
        }
        servers.insert(root.to_path_buf(), Server::Starting(token));
    }
    let started = start(root);
    let mut servers = servers()?;
    let reserved = matches!(servers.get(root), Some(Server::Starting(t)) if *t == token);
    match started {
        Ok((child, url)) if reserved => {
            servers.insert(root.to_path_buf(), Server::Running(child, url.clone()));
            Ok(url)
        }
        Ok((mut child, _)) => {
            drop(servers);
            kill(&mut child);
            Err("the project was closed".into())
        }
        Err(e) => {
            if reserved {
                servers.remove(root);
            }
            Err(e)
        }
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn start(root: &Path) -> Result<(Child, String), String> {
    start_with(&binary(), root)
}

/// Start `bin serve root` and wait for its URL.
fn start_with(bin: &Path, root: &Path) -> Result<(Child, String), String> {
    let mut child = Command::new(bin)
        .arg("serve")
        .arg(root)
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    let stderr = child.stderr.take().map(tail);
    match first_url(&mut child) {
        // The reader keeps draining stderr while the server runs.
        Ok(url) => Ok((child, url)),
        Err(e) => {
            kill(&mut child);
            let said = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
            Err(if said.is_empty() {
                e
            } else {
                format!("{e}\n{said}")
            })
        }
    }
}

/// Bytes kept from the end of the server's stderr for the error.
const STDERR_TAIL: usize = 4096;

/// Read `stream` to its end on a thread; returns its last
/// [`STDERR_TAIL`] bytes.
fn tail(mut stream: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = stream.read(&mut buf) {
            kept.extend_from_slice(&buf[..n]);
            let excess = kept.len().saturating_sub(STDERR_TAIL);
            kept.drain(..excess);
        }
        String::from_utf8_lossy(&kept).trim().to_string()
    })
}

/// The URL the server prints first; keeps draining its output afterwards
/// so later output cannot block or kill it.
fn first_url(child: &mut Child) -> Result<String, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or("no stdout from archguard serve")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    std::thread::spawn(move || for _ in reader.lines() {});
    let url = line.trim().to_string();
    if !url.starts_with("http://") {
        return Err(format!("archguard serve printed no URL: {url:?}"));
    }
    Ok(url)
}

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

/// Blocking: call from a worker thread. Returns the opened URL.
pub fn open(root: &Path, tab: &str) -> Result<String, String> {
    let url = format!("{}{}", server_url(root)?, route(tab));
    open_in_browser(&url)?;
    Ok(url)
}

/// Stop the server of `root` started by [`open`], if any; a server still
/// starting is stopped when its start finishes.
pub fn stop(root: &Path) {
    let server = servers().ok().and_then(|mut s| s.remove(root));
    if let Some(Server::Running(mut child, _)) = server {
        kill(&mut child);
    }
}

/// Stop every server started by [`open`].
pub fn shutdown() {
    let all = servers().map(|mut s| std::mem::take(&mut *s));
    for server in all.unwrap_or_default().into_values() {
        if let Server::Running(mut child, _) = server {
            kill(&mut child);
        }
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

    /// A failed `archguard serve` says why: its stderr is in the error,
    /// not only "printed no URL".
    #[cfg(unix)]
    #[test]
    fn a_failed_serve_reports_its_stderr() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("archguard-web-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("archguard");
        std::fs::write(
            &bin,
            "#!/bin/sh\nhead -c 9000 /dev/zero | tr '\\0' x >&2\necho 'error: no archguard.toml in /p' >&2\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = start_with(&bin, Path::new("/p")).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("printed no URL"), "{err}");
        assert!(err.contains("error: no archguard.toml in /p"), "{err}");
        assert!(err.len() < 4096 + 200, "bounded: {} bytes", err.len());
    }

    fn sleeper() -> Child {
        Command::new("sleep").arg("30").spawn().expect("sleep")
    }

    /// Opening the web viewer for one project must not block another
    /// project's start: the lock is free while the server starts.
    #[test]
    fn the_lock_is_free_while_a_server_starts() {
        let root = Path::new("/archguard-test/lock-free");
        let url = server_url_with(root, |_| {
            assert!(SERVERS.try_lock().is_ok(), "lock held during start");
            Ok((sleeper(), "http://127.0.0.1:1/".into()))
        });
        assert_eq!(url.as_deref(), Ok("http://127.0.0.1:1/"));
        assert_eq!(server_url(root).as_deref(), Ok("http://127.0.0.1:1/"));
        stop(root);
        assert!(!servers().unwrap().contains_key(root));
    }

    /// Closing the document while its server starts leaves no server.
    #[test]
    fn closing_during_the_start_stops_the_new_server() {
        let root = Path::new("/archguard-test/closed");
        let mut pid = 0;
        let url = server_url_with(root, |r| {
            let child = sleeper();
            pid = child.id();
            stop(r);
            Ok((child, "http://127.0.0.1:2/".into()))
        });
        assert!(url.is_err());
        assert!(!servers().unwrap().contains_key(root));
        let alive = Command::new("kill").args(["-0", &pid.to_string()]).status();
        assert!(
            !alive.expect("kill -0").success(),
            "server {pid} still runs"
        );
    }

    /// A failed start leaves the project free for the next attempt.
    #[test]
    fn a_failed_start_can_be_retried() {
        let root = Path::new("/archguard-test/retry");
        assert!(server_url_with(root, |_| Err("no URL".into())).is_err());
        let url = server_url_with(root, |_| Ok((sleeper(), "http://127.0.0.1:3/".into())));
        assert!(url.is_ok());
        stop(root);
    }
}
