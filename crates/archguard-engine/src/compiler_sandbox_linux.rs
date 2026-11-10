//! Linux confinement (ADR 0007) with Landlock: the child restricts itself
//! between fork and exec. Reading: system locations, the toolchain and the
//! project; executing: system locations and the toolchain only; writing:
//! the scratch directory and `/dev/null`; TCP bind and connect: none.
//! Landlock covers only TCP, so the child also moves into a new user and
//! network namespace: no interface but a loopback that is down, so no UDP
//! and no abstract Unix sockets of the host either. Where unprivileged user
//! namespaces are off, Landlock alone confines it and a warning says so.

use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
    RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetError,
};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, Once, OnceLock};

const ABI_NEEDED: ABI = ABI::V4;

/// System locations tools read and start programs from.
const SYSTEM: [&str; 9] = [
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/opt", "/nix",
];

/// A ruleset handling every file and TCP access of [`ABI_NEEDED`]; fails
/// where the kernel cannot enforce all of them.
fn ruleset() -> Result<RulesetCreated, RulesetError> {
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI_NEEDED))?
        .handle_access(AccessNet::from_all(ABI_NEEDED))?
        .create()
}

pub(super) fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| ruleset().is_ok())
}

/// Whether a child can get its own user and network namespace (not where
/// unprivileged user namespaces are off); probed once.
pub(super) fn namespaces() -> bool {
    static NAMESPACES: OnceLock<bool> = OnceLock::new();
    *NAMESPACES.get_or_init(|| {
        let mut probe = Command::new("/bin/sh");
        probe.args(["-c", ":"]);
        // SAFETY: as in `command`; `isolate` only calls unshare(2).
        unsafe { probe.pre_exec(isolate) };
        probe.status().is_ok_and(|s| s.success())
    })
}

const CLONE_NEWUSER: i32 = 0x1000_0000;
const CLONE_NEWNET: i32 = 0x4000_0000;

unsafe extern "C" {
    fn unshare(flags: i32) -> i32;
}

/// Moves the calling (single-threaded, forked) process into a new user and
/// network namespace. Fails where unprivileged user namespaces are off.
fn isolate() -> std::io::Result<()> {
    // SAFETY: unshare(2) takes plain flags and touches no memory.
    if unsafe { unshare(CLONE_NEWUSER | CLONE_NEWNET) } == 0 {
        return Ok(());
    }
    let cause = std::io::Error::last_os_error();
    Err(std::io::Error::new(
        cause.kind(),
        format!(
            "sandbox: no private network namespace ({cause}); are unprivileged user namespaces off?"
        ),
    ))
}

fn existing(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().filter(|p| p.exists()).collect()
}

fn rules(
    tools: Vec<PathBuf>,
    files: Vec<PathBuf>,
    project: &Path,
    scratch: &Path,
) -> Result<RulesetCreated, RulesetError> {
    let read = AccessFs::ReadFile | AccessFs::ReadDir;
    let run = AccessFs::from_read(ABI_NEEDED);
    let system = existing(SYSTEM.iter().map(PathBuf::from));
    let mut set = ruleset()?;
    for (path, access) in existing([system, tools].concat())
        .into_iter()
        .map(|p| (p, run))
        .chain([(project.to_path_buf(), read)])
        .chain(files.into_iter().map(|p| (p, AccessFs::ReadFile.into())))
        // rustup's proxies find themselves through /proc/self/exe. Landlock
        // also forbids ptrace on processes outside the domain, so another
        // process' /proc/<pid>/environ stays closed.
        .chain(
            existing(["/proc", "/sys"].map(PathBuf::from))
                .into_iter()
                .map(|p| (p, read)),
        )
        .chain([(
            scratch.to_path_buf(),
            AccessFs::from_all(ABI_NEEDED) & !AccessFs::Execute,
        )])
        .chain([(
            PathBuf::from("/dev/null"),
            AccessFs::ReadFile | AccessFs::WriteFile,
        )])
    {
        if let Ok(fd) = PathFd::new(&path) {
            set = set.add_rule(PathBeneath::new(fd, access))?;
        }
    }
    Ok(set)
}

/// `program` restricting itself before it starts. If the rules cannot be
/// built, the command fails to start rather than run unconfined.
pub(super) fn command(
    program: &Path,
    tools: Vec<PathBuf>,
    files: Vec<PathBuf>,
    project: &Path,
    scratch: &Path,
) -> Command {
    let namespaces = namespaces();
    if !namespaces {
        static WARNED: Once = Once::new();
        WARNED.call_once(|| eprintln!("warning: {NO_NAMESPACES}"));
    }
    command_with(program, tools, files, project, scratch, namespaces)
}

const NO_NAMESPACES: &str = "unprivileged user namespaces are off: project compiler \
    tooling runs confined by Landlock only, so UDP and abstract Unix sockets stay open";

/// [`command`], in its own namespaces where `namespaces` holds.
pub(super) fn command_with(
    program: &Path,
    tools: Vec<PathBuf>,
    files: Vec<PathBuf>,
    project: &Path,
    scratch: &Path,
    namespaces: bool,
) -> Command {
    let built = rules(tools, files, project, scratch).map_err(|e| e.to_string());
    let rules = Mutex::new(Some(built));
    let mut command = Command::new(program);
    // SAFETY: the closure runs in the child between fork and exec. It calls
    // unshare(2), takes the ruleset prepared in the parent (an uncontended
    // mutex owned by this command) and calls prctl(PR_SET_NO_NEW_PRIVS) and
    // landlock_restrict_self through the crate; the error path allocates,
    // after which the child exits without exec.
    unsafe {
        command.pre_exec(move || {
            if namespaces {
                isolate()?;
            }
            let taken = rules.lock().ok().and_then(|mut r| r.take());
            match taken {
                Some(Ok(set)) => set.restrict_self().map(drop).map_err(std::io::Error::other),
                Some(Err(e)) => Err(std::io::Error::other(e)),
                None => Err(std::io::Error::other("sandbox rules already used")),
            }
        });
    }
    command
}
