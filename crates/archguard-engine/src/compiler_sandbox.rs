//! Confinement of a project's compiler tooling (ADR 0007): rust-analyzer
//! and the project's TypeScript compiler read the repository's own
//! configuration and so may run code from it. Confined, they read only the
//! system, their toolchain and the project, write only a scratch
//! directory, start only toolchain programs and have no network.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this platform confines the tooling.
pub(crate) fn available() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).exists()
}

/// `program` as a command that runs confined to `project` (read) and
/// `scratch` (read and write), where [`available`]; elsewhere as is.
pub(crate) fn command(program: &OsStr, project: &Path, scratch: &Path) -> Command {
    if !available() {
        return Command::new(program);
    }
    let program = resolve(program);
    let mut command = Command::new(SANDBOX_EXEC);
    command
        .arg("-p")
        .arg(profile(&program, project, scratch))
        .arg(&program);
    command
}

/// `program` as an absolute path: the sandbox starts no program by name.
fn resolve(program: &OsStr) -> PathBuf {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return real(path);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .map_or_else(|| path.to_path_buf(), |p| real(&p))
}

/// The real path (`/var` is `/private/var` on macOS; profiles need it).
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Directories the toolchain of `program` lives in: rustup, cargo and the
/// installation `program` comes from (`…/bin/node` → `…`).
fn toolchain(program: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
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

fn subpaths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!(" (subpath \"{}\")", quote(p)))
        .collect()
}

fn quote(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

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

#[cfg(all(test, target_os = "macos"))]
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
        let reach = |confined: bool| {
            let mut c = if confined {
                command(
                    OsStr::new("/usr/bin/nc"),
                    d.project.path(),
                    d.scratch.path(),
                )
            } else {
                Command::new("/usr/bin/nc")
            };
            c.args(["-z", "-w", "2", "127.0.0.1", &port])
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
}
