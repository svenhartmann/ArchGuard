//! Windows starter for confined project compiler tooling (ADR 0007).
//!
//! `archguard-confine [--run DIR | --read DIR | --write DIR]... -- PROGRAM [ARGS]...`
//! starts PROGRAM in the AppContainer `ArchGuard.Confined`: no network
//! capability, and of the user's files only the listed directories
//! (`--run`: read and execute, `--read`: read, `--write`: everything but
//! execute). The rights are granted to the container once per directory and
//! stay. A job object ends PROGRAM with the starter; the exit code is passed
//! on. Rust's `Command` cannot start an AppContainer process yet
//! (`raw_attribute` is unstable), hence this separate program.

#[cfg(not(windows))]
fn main() {
    eprintln!("archguard-confine runs on Windows only");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match windows::run(&args) {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("archguard-confine: {message}");
            std::process::exit(125);
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::{OsStr, OsString, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE,
        SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
        TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_CAPABILITIES, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_EXECUTE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList,
        EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread,
        STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
    };

    const CONTAINER: &str = "ArchGuard.Confined";
    /// HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS).
    const ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain([0]).collect()
    }

    struct Plan {
        grants: Vec<(OsString, u32)>,
        program: Vec<OsString>,
    }

    fn parse(args: &[OsString]) -> Result<Plan, String> {
        let mut grants = Vec::new();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let rights = match flag.to_str() {
                Some("--run") => FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                Some("--read") => FILE_GENERIC_READ,
                Some("--write") => FILE_ALL_ACCESS & !FILE_EXECUTE,
                Some("--") => break,
                _ => return Err(format!("unknown argument {}", flag.to_string_lossy())),
            };
            let dir = rest.next().ok_or("a directory must follow")?;
            grants.push((dir.clone(), rights));
        }
        let program: Vec<OsString> = rest.cloned().collect();
        if program.is_empty() {
            return Err("no program after --".into());
        }
        Ok(Plan { grants, program })
    }

    pub fn run(args: &[OsString]) -> Result<i32, String> {
        let plan = parse(args)?;
        let sid = container_sid()?;
        for (dir, rights) in &plan.grants {
            grant(dir, sid, *rights)?;
        }
        launch(sid, &plan.program)
    }

    fn container_sid() -> Result<PSID, String> {
        let name = wide(OsStr::new(CONTAINER));
        let mut sid: PSID = null_mut();
        // SAFETY: valid NUL-terminated strings; `sid` receives an owned SID.
        let created = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if created == 0 {
            return Ok(sid);
        }
        if created != ALREADY_EXISTS {
            return Err(format!("CreateAppContainerProfile failed: {created:#x}"));
        }
        // SAFETY: as above.
        let derived = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if derived != 0 {
            return Err(format!(
                "DeriveAppContainerSidFromAppContainerName failed: {derived:#x}"
            ));
        }
        Ok(sid)
    }

    /// Whether `dacl` already allows `sid` all of `rights`.
    fn granted(dacl: *mut ACL, sid: PSID, rights: u32) -> bool {
        if dacl.is_null() {
            return false;
        }
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `dacl` comes from GetNamedSecurityInfoW; the buffer matches the class.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        if ok == 0 {
            return false;
        }
        (0..info.AceCount).any(|i| {
            let mut ace: *mut c_void = null_mut();
            // SAFETY: index below AceCount; ACEs of type 0 are ACCESS_ALLOWED_ACE.
            unsafe {
                if GetAce(dacl, i, &mut ace) == 0 || *(ace as *const u8) != 0 {
                    return false;
                }
                let allowed = ace as *const ACCESS_ALLOWED_ACE;
                let ace_sid = std::ptr::addr_of!((*allowed).SidStart) as PSID;
                (*allowed).Mask & rights == rights && EqualSid(ace_sid, sid) != 0
            }
        })
    }

    /// Allow the container `rights` on `dir` and everything below it, once.
    fn grant(dir: &OsStr, sid: PSID, rights: u32) -> Result<(), String> {
        let mut path = wide(dir);
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: NUL-terminated path; out pointers are valid.
        let read = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if read != 0 {
            return Err(format!(
                "reading the rights of {} failed: {read}",
                dir.to_string_lossy()
            ));
        }
        let result = if granted(dacl, sid, rights) {
            Ok(())
        } else {
            add(&mut path, dacl, sid, rights, dir)
        };
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(descriptor) };
        result
    }

    fn add(
        path: &mut [u16],
        dacl: *mut ACL,
        sid: PSID,
        rights: u32,
        dir: &OsStr,
    ) -> Result<(), String> {
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: rights,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid as *mut u16,
            },
        };
        let mut merged: *mut ACL = null_mut();
        // SAFETY: one valid entry; `dacl` may be null (empty DACL).
        let made = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut merged) };
        if made != 0 {
            return Err(format!(
                "building the rights of {} failed: {made}",
                dir.to_string_lossy()
            ));
        }
        // SAFETY: NUL-terminated path; `merged` is a valid ACL.
        let set = unsafe {
            SetNamedSecurityInfoW(
                path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                merged,
                null_mut(),
            )
        };
        // SAFETY: allocated by SetEntriesInAclW.
        unsafe { LocalFree(merged.cast()) };
        if set != 0 {
            return Err(format!("granting {} failed: {set}", dir.to_string_lossy()));
        }
        Ok(())
    }

    /// A command line as the C runtime splits it (quotes and backslashes).
    fn command_line(program: &[OsString]) -> Vec<u16> {
        let mut line = String::new();
        for (i, arg) in program.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            let arg = arg.to_string_lossy();
            if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
                line.push_str(&arg);
                continue;
            }
            line.push('"');
            let mut slashes = 0;
            for c in arg.chars() {
                match c {
                    '\\' => slashes += 1,
                    '"' => {
                        line.push_str(&"\\".repeat(slashes * 2 + 1));
                        line.push('"');
                        slashes = 0;
                    }
                    _ => {
                        line.push_str(&"\\".repeat(slashes));
                        line.push(c);
                        slashes = 0;
                    }
                }
            }
            line.push_str(&"\\".repeat(slashes * 2));
            line.push('"');
        }
        OsStr::new(&line).encode_wide().chain([0]).collect()
    }

    fn launch(sid: PSID, program: &[OsString]) -> Result<i32, String> {
        let mut size = 0usize;
        // SAFETY: the first call only reports the size.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
        let mut buffer = vec![0u8; size];
        let list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        let mut capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: sid,
            Capabilities: null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        // SAFETY: `buffer` has the reported size; `capabilities` outlives the process start.
        unsafe {
            if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
                return Err("InitializeProcThreadAttributeList failed".into());
            }
            if UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                (&mut capabilities as *mut SECURITY_CAPABILITIES).cast(),
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                null_mut(),
                null(),
            ) == 0
            {
                DeleteProcThreadAttributeList(list);
                return Err("UpdateProcThreadAttribute failed".into());
            }
        }
        let started = start(list, program);
        // SAFETY: initialized above.
        unsafe { DeleteProcThreadAttributeList(list) };
        let (process, thread) = started?;
        wait(process, thread)
    }

    fn start(
        list: LPPROC_THREAD_ATTRIBUTE_LIST,
        program: &[OsString],
    ) -> Result<(HANDLE, HANDLE), String> {
        // SAFETY: plain C structs, zero is their documented empty state.
        let mut info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        info.lpAttributeList = list;
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        // SAFETY: querying the starter's own standard handles.
        unsafe {
            info.StartupInfo.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
            info.StartupInfo.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
            info.StartupInfo.hStdError = GetStdHandle(STD_ERROR_HANDLE);
        }
        let mut line = command_line(program);
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: mutable NUL-terminated command line; handles are inherited.
        let ok = unsafe {
            CreateProcessW(
                null(),
                line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
                null(),
                null(),
                &info.StartupInfo,
                &mut process,
            )
        };
        if ok == 0 {
            return Err(format!(
                "starting {} failed: {}",
                program[0].to_string_lossy(),
                std::io::Error::last_os_error()
            ));
        }
        Ok((process.hProcess, process.hThread))
    }

    fn wait(process: HANDLE, thread: HANDLE) -> Result<i32, String> {
        // SAFETY: handles from CreateProcessW; the job ends the process with
        // the starter (kill on close), so a killed starter leaves nothing.
        unsafe {
            let job = CreateJobObjectW(null(), null());
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let limited = !job.is_null()
                && SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0
                && AssignProcessToJobObject(job, process) != 0;
            if !limited {
                windows_sys::Win32::System::Threading::TerminateProcess(process, 125);
                return Err("the job object could not be set up".into());
            }
            ResumeThread(thread);
            WaitForSingleObject(process, INFINITE);
            let mut code = 0u32;
            GetExitCodeProcess(process, &mut code);
            CloseHandle(thread);
            CloseHandle(process);
            Ok(code as i32)
        }
    }
}
