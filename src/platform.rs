use std::path::{Path, PathBuf};

const APP: &str = "t3-codemode";

#[cfg(target_os = "linux")]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

#[cfg(target_os = "linux")]
pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

#[cfg(target_os = "linux")]
pub fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".config")).join(APP)
}

#[cfg(target_os = "linux")]
pub fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local").join("state")).join(APP)
}

#[cfg(not(target_os = "linux"))]
fn base_dirs() -> Option<directories::BaseDirs> {
    directories::BaseDirs::new()
}

#[cfg(not(target_os = "linux"))]
pub fn home_dir() -> Option<PathBuf> {
    base_dirs().map(|b| b.home_dir().to_path_buf()).filter(|h| !h.as_os_str().is_empty())
}

#[cfg(not(target_os = "linux"))]
pub fn home() -> PathBuf {
    home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(not(target_os = "linux"))]
pub fn config_dir() -> PathBuf {
    base_dirs().map(|b| b.config_local_dir().to_path_buf()).unwrap_or_else(|| home().join(".config")).join(APP)
}

#[cfg(not(target_os = "linux"))]
pub fn state_dir() -> PathBuf {
    base_dirs()
        .map(|b| b.state_dir().unwrap_or(b.data_local_dir()).to_path_buf())
        .unwrap_or_else(|| home().join(".local").join("state"))
        .join(APP)
}

#[cfg(windows)]
pub fn installed_exe() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"))
        .join("Programs")
        .join(APP)
        .join("t3cm.exe")
}

#[cfg(not(windows))]
pub fn installed_exe() -> PathBuf {
    home().join(".local").join("bin").join("t3cm")
}

pub struct Service {
    pub status: &'static str,
    pub start: &'static str,
    pub restart: &'static str,
    pub logs: &'static str,
}

#[cfg(target_os = "macos")]
pub const SERVICE: Service = Service {
    status: "launchctl print gui/$(id -u)/com.ottoops.t3-codemode",
    start: "launchctl kickstart gui/$(id -u)/com.ottoops.t3-codemode",
    restart: "launchctl kickstart -k gui/$(id -u)/com.ottoops.t3-codemode",
    logs: "~/Library/Logs/t3-codemode/t3cm.log",
};

#[cfg(windows)]
pub const SERVICE: Service = Service {
    status: "Get-ScheduledTask t3-codemode | Get-ScheduledTaskInfo",
    start: "Enable-ScheduledTask t3-codemode; Start-ScheduledTask t3-codemode",
    restart: "Stop-ScheduledTask t3-codemode; Start-ScheduledTask t3-codemode",
    logs: "Get-Content \"$env:LOCALAPPDATA\\t3-codemode\\logs\\t3cm.log\"",
};

#[cfg(not(any(target_os = "macos", windows)))]
pub const SERVICE: Service = Service {
    status: "systemctl --user status t3-codemode",
    start: "systemctl --user start t3-codemode",
    restart: "systemctl --user restart t3-codemode",
    logs: "journalctl --user -u t3-codemode",
};

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(unix)]
pub fn private_dir(path: &Path) -> std::io::Result<()> {
    set_mode(path, 0o700)
}

#[cfg(unix)]
pub fn private_file(path: &Path) -> std::io::Result<()> {
    set_mode(path, 0o600)
}

#[cfg(not(unix))]
pub fn private_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub fn private_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
pub fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(windows)]
pub fn executable_variants(path: PathBuf) -> Vec<PathBuf> {
    const LAUNCHABLE: &[&str] = &[".COM", ".EXE", ".BAT", ".CMD"];
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| LAUNCHABLE.join(";"));
    let mut out = Vec::new();
    if path.extension().is_some() {
        out.push(path.clone());
    }
    for ext in exts.split(';').filter(|e| LAUNCHABLE.iter().any(|l| l.eq_ignore_ascii_case(e))) {
        let mut p = path.clone().into_os_string();
        p.push(ext.to_ascii_lowercase());
        out.push(p.into());
    }
    out
}

#[cfg(not(windows))]
pub fn executable_variants(path: PathBuf) -> Vec<PathBuf> {
    vec![path]
}

pub fn has_separator(s: &str) -> bool {
    s.contains('/') || (cfg!(windows) && s.contains('\\'))
}

pub fn exe_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(target_os = "linux")]
pub fn process_exe(pid: u32) -> Option<PathBuf> {
    std::fs::canonicalize(format!("/proc/{pid}/exe")).ok()
}

#[cfg(target_os = "macos")]
pub fn process_exe(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let pid = i32::try_from(pid).ok()?;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let n = usize::try_from(n).ok().filter(|n| *n > 0)?;
    std::fs::canonicalize(std::ffi::OsStr::from_bytes(&buf[..n])).ok()
}

#[cfg(windows)]
pub fn process_exe(pid: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        ok
    };
    (ok != 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize])))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn process_exe(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(unix)]
pub struct ProcessTree {
    pgid: i32,
}

#[cfg(unix)]
impl ProcessTree {
    pub fn prepare(cmd: &mut tokio::process::Command) {
        cmd.process_group(0);
    }

    pub fn start(child: &tokio::process::Child) -> std::io::Result<ProcessTree> {
        child
            .id()
            .and_then(|p| i32::try_from(p).ok())
            .filter(|p| *p > 1)
            .map(|pgid| ProcessTree { pgid })
            .ok_or_else(|| std::io::Error::other("the child has no process id"))
    }

    pub fn kill(&self) {
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
pub struct ProcessTree {
    job: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ProcessTree {
    pub fn prepare(cmd: &mut tokio::process::Command) {
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }

    pub fn start(child: &tokio::process::Child) -> std::io::Result<ProcessTree> {
        use std::io::Error;
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        let process = child.raw_handle().ok_or_else(|| Error::other("the child has no process handle"))?;
        let pid = child.id().ok_or_else(|| Error::other("the child has no process id"))?;
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(Error::last_os_error());
            }
            let job = OwnedHandle::from_raw_handle(raw);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(raw, process) == 0 {
                return Err(Error::last_os_error());
            }
            resume_threads(pid)?;
            Ok(ProcessTree { job })
        }
    }

    pub fn kill(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job.as_raw_handle(), 1);
        }
    }
}

#[cfg(windows)]
fn resume_threads(pid: u32) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut more = Thread32First(snap, &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    resumed |= ResumeThread(thread) != u32::MAX;
                    CloseHandle(thread);
                }
            }
            more = Thread32Next(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        if resumed { Ok(()) } else { Err(std::io::Error::other("could not resume the suspended child")) }
    }
}
