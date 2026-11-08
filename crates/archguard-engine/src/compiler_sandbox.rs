//! Confinement of a project's compiler tooling (ADR 0007): rust-analyzer
//! and the project's TypeScript compiler read the repository's own
//! configuration and so may run code from it. Confined, they read only the
//! system, their toolchain and the project, write only a scratch
//! directory, start only toolchain programs and have no network.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(not(any(target_os = "linux", windows)))]
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this platform confines the tooling: macOS with `sandbox-exec`,
/// Linux with Landlock including its network rules (kernel 6.7+), Windows
/// with an AppContainer through the `archguard-confine` starter.
pub(crate) fn available() -> bool {
    #[cfg(target_os = "linux")]
    return linux::available();
    #[cfg(windows)]
    return launcher().is_some();
    #[cfg(not(any(target_os = "linux", windows)))]
    return cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).exists();
}

/// The `archguard-confine` starter next to the running program (or one
/// directory up, where test binaries run from `deps`).
#[cfg(windows)]
fn launcher() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("archguard-confine.exe"))
        .find(|p| p.is_file())
}

/// `program` as a command that runs confined to `project` (read) and
/// `scratch` (read and write), where [`available`]; elsewhere as is.
pub(crate) fn command(program: &OsStr, project: &Path, scratch: &Path) -> Command {
    if !available() {
        return Command::new(program);
    }
    let program = resolve(program);
    #[cfg(target_os = "linux")]
    return linux::command(&program, toolchain(&program), project, scratch);
    #[cfg(windows)]
    {
        let launcher = launcher().unwrap_or_default();
        let mut command = Command::new(launcher);
        for dir in toolchain(&program).into_iter().filter(|d| d.is_dir()) {
            command.arg("--run").arg(dir);
        }
        command
            .arg("--read")
            .arg(project)
            .arg("--write")
            .arg(scratch)
            .arg("--")
            .arg(&program);
        command
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let mut command = Command::new(SANDBOX_EXEC);
        command
            .arg("-p")
            .arg(profile(&program, project, scratch))
            .arg(&program);
        command
    }
}

/// `program` as an absolute path: the sandbox starts no program by name.
/// Symlinks stay as found: rustup's proxies (`rust-analyzer` → `rustup`)
/// pick the tool by the name they are started under.
fn resolve(program: &OsStr) -> PathBuf {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.to_path_buf();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .flat_map(|dir| {
            let plain = dir.join(program);
            let exe = plain.with_extension("exe");
            [plain, exe]
        })
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| path.to_path_buf())
}

/// The real path (`/var` is `/private/var` on macOS; profiles need it).
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Directories the toolchain of `program` lives in: rustup, cargo and the
/// installation `program` comes from (`…/bin/node` → `…`).
fn toolchain(program: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = home
        .iter()
        .flat_map(|h| [h.join(".rustup"), h.join(".cargo")])
        .chain(std::env::var_os("RUSTUP_HOME").map(PathBuf::from))
        .chain(std::env::var_os("CARGO_HOME").map(PathBuf::from))
        .collect();
    // `/bin/cat` would give `/`: an installation sits at least two levels
    // deep (`/usr` is a system location anyway).
    if let Some(install) = program.parent().and_then(Path::parent)
        && install.components().count() > 2
    {
        dirs.push(install.to_path_buf());
    }
    dirs.into_iter().map(|d| real(&d)).collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
/// System locations tools read and start programs from.
const SYSTEM: [&str; 9] = [
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library",
    "/opt/homebrew",
    "/private/etc",
    "/private/var/db",
    "/dev",
];

#[cfg(not(any(target_os = "linux", windows)))]
fn subpaths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!(" (subpath \"{}\")", quote(p)))
        .collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn quote(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(not(any(target_os = "linux", windows)))]
/// Deny everything, then allow: reading the system, the toolchain and the
/// project; writing the scratch directory; starting system and toolchain
/// programs. No network rule is allowed, so there is none.
fn profile(program: &Path, project: &Path, scratch: &Path) -> String {
    let system: Vec<PathBuf> = SYSTEM.iter().map(PathBuf::from).collect();
    let tools = [system.clone(), toolchain(program)].concat();
    let (project, scratch) = (real(project), real(scratch));
    let readable = [tools.clone(), vec![project, scratch.clone()]].concat();
    format!(
        "(version 1)\n(deny default)\n(allow process-fork)\n(allow sysctl-read)\n(allow mach-lookup)\n\
         (allow signal (target same-sandbox))\n(allow ipc-posix-shm)\n(allow file-read-metadata)\n\
         (allow file-read* (literal \"/\"){})\n(allow file-write* (literal \"/dev/null\"){})\n\
         (allow process-exec{})\n",
        subpaths(&readable),
        subpaths(&[scratch]),
        subpaths(&tools),
    )
}

#[cfg(target_os = "linux")]
#[path = "compiler_sandbox_linux.rs"]
mod linux;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use std::net::TcpListener;

    struct Dirs {
        project: tempfile::TempDir,
        scratch: tempfile::TempDir,
        secret: tempfile::TempDir,
    }

    fn dirs() -> Dirs {
        let d = Dirs {
            project: tempfile::tempdir().unwrap(),
            scratch: tempfile::tempdir().unwrap(),
            secret: tempfile::tempdir().unwrap(),
        };
        std::fs::write(d.project.path().join("p.txt"), "project").unwrap();
        std::fs::write(d.secret.path().join("s.txt"), "secret").unwrap();
        d
    }

    fn run(d: &Dirs, program: &str, args: &[&str]) -> bool {
        command(OsStr::new(program), d.project.path(), d.scratch.path())
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn confined_tooling_reads_the_project_but_nothing_private() {
        let d = dirs();
        assert!(available());
        let p = d.project.path().join("p.txt");
        assert!(
            run(&d, "/bin/cat", &[p.to_str().unwrap()]),
            "the project is readable"
        );
        let s = d.secret.path().join("s.txt");
        assert!(
            !run(&d, "/bin/cat", &[s.to_str().unwrap()]),
            "other directories are not"
        );
    }

    #[test]
    fn confined_tooling_writes_only_its_scratch_directory() {
        let d = dirs();
        let ok = format!("echo x > '{}'", d.scratch.path().join("o").display());
        assert!(run(&d, "/bin/sh", &["-c", &ok]));
        for target in [d.secret.path(), d.project.path()] {
            let bad = format!("echo x > '{}'", target.join("o").display());
            assert!(!run(&d, "/bin/sh", &["-c", &bad]), "{}", target.display());
        }
    }

    #[test]
    fn confined_tooling_has_no_network_and_runs_no_repository_program() {
        let d = dirs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        // bash's /dev/tcp opens a connection on macOS and Linux alike.
        let probe = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");
        let reach = |confined: bool| {
            let mut c = if confined {
                command(OsStr::new("/bin/bash"), d.project.path(), d.scratch.path())
            } else {
                Command::new("/bin/bash")
            };
            c.args(["-c", &probe])
                .output()
                .is_ok_and(|o| o.status.success())
        };
        assert!(reach(false), "the listener answers outside the sandbox");
        assert!(!reach(true), "no network inside");
        let evil = d.project.path().join("evil");
        std::fs::copy("/bin/echo", &evil).unwrap();
        assert!(
            !run(&d, evil.to_str().unwrap(), &["hi"]),
            "programs from the project do not start"
        );
    }

    #[test]
    fn node_runs_confined_from_its_own_installation() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let d = dirs();
        let script = d.scratch.path().join("read.cjs");
        std::fs::write(
            &script,
            "process.stdout.write(require('fs').readFileSync(process.argv[2], 'utf8'))",
        )
        .unwrap();
        let read = |file: &Path| {
            command(OsStr::new("node"), d.project.path(), d.scratch.path())
                .arg(&script)
                .arg(file)
                .output()
                .unwrap()
        };
        let ok = read(&d.project.path().join("p.txt"));
        assert_eq!(
            String::from_utf8_lossy(&ok.stdout),
            "project",
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        assert!(!read(&d.secret.path().join("s.txt")).status.success());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn other_processes_stay_closed_on_linux() {
        let d = dirs();
        // The parent's environment through /proc: readable outside, not inside.
        let environ = format!("/proc/{}/environ", std::process::id());
        assert!(
            Command::new("/bin/cat")
                .arg(&environ)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!run(&d, "/bin/cat", &[&environ]));
        assert!(
            run(&d, "/bin/cat", &["/proc/self/status"]),
            "its own /proc stays readable"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const CMD: &str = r"C:\Windows\System32\cmd.exe";

    struct Dirs {
        project: tempfile::TempDir,
        scratch: tempfile::TempDir,
        secret: tempfile::TempDir,
    }

    fn dirs() -> Dirs {
        let d = Dirs {
            project: tempfile::tempdir().unwrap(),
            scratch: tempfile::tempdir().unwrap(),
            secret: tempfile::tempdir().unwrap(),
        };
        std::fs::write(d.project.path().join("p.txt"), "project").unwrap();
        std::fs::write(d.secret.path().join("s.txt"), "secret").unwrap();
        d
    }

    fn cmd(d: &Dirs, line: &str) -> std::process::Output {
        command(OsStr::new(CMD), d.project.path(), d.scratch.path())
            .args(["/c", line])
            .output()
            .unwrap()
    }

    #[test]
    fn confined_tooling_reads_the_project_but_nothing_private() {
        let d = dirs();
        assert!(
            available(),
            "archguard-confine.exe is built next to the tests"
        );
        let p = cmd(
            &d,
            &format!("type \"{}\"", d.project.path().join("p.txt").display()),
        );
        assert_eq!(
            String::from_utf8_lossy(&p.stdout).trim(),
            "project",
            "{}",
            String::from_utf8_lossy(&p.stderr)
        );
        let s = cmd(
            &d,
            &format!("type \"{}\"", d.secret.path().join("s.txt").display()),
        );
        assert!(!String::from_utf8_lossy(&s.stdout).contains("secret"));
    }

    #[test]
    fn confined_tooling_writes_only_its_scratch_directory() {
        let d = dirs();
        cmd(
            &d,
            &format!("echo x> \"{}\"", d.scratch.path().join("o").display()),
        );
        assert!(d.scratch.path().join("o").exists());
        for target in [d.secret.path(), d.project.path()] {
            cmd(&d, &format!("echo x> \"{}\"", target.join("o").display()));
            assert!(!target.join("o").exists(), "{}", target.display());
        }
    }

    #[test]
    fn confined_tooling_has_no_network_and_runs_no_repository_program() {
        let d = dirs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        let curl = r"C:\Windows\System32\curl.exe";
        let url = format!("http://127.0.0.1:{port}/");
        let open = Command::new(curl)
            .args(["-s", "-m", "5", &url])
            .output()
            .unwrap();
        assert!(
            open.status.success(),
            "the listener answers outside the sandbox"
        );
        let closed = command(OsStr::new(curl), d.project.path(), d.scratch.path())
            .args(["-s", "-m", "5", &url])
            .output()
            .unwrap();
        assert!(!closed.status.success(), "no network inside");
        let evil = d.project.path().join("evil.exe");
        std::fs::copy(r"C:\Windows\System32\whoami.exe", &evil).unwrap();
        let ran = command(evil.as_os_str(), d.project.path(), d.scratch.path())
            .output()
            .unwrap();
        assert!(
            !ran.status.success(),
            "programs from the project do not start"
        );
    }
}
