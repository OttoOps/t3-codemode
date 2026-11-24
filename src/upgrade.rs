use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::release::{self, Checked, REPO, Version};
use crate::{drift, platform, skills, steps};

const HEALTH_WAIT: Duration = Duration::from_secs(45);
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(300);

pub struct Opts {
    pub check: bool,
    pub tag: Option<String>,
    pub port: u16,
    pub restart: bool,
    pub state_dir: PathBuf,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
pub const TARGET: Option<&str> = Some("x86_64-unknown-linux-musl");
#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
pub const TARGET: Option<&str> = Some("aarch64-unknown-linux-musl");
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub const TARGET: Option<&str> = Some("x86_64-unknown-linux-gnu");
#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
pub const TARGET: Option<&str> = Some("aarch64-unknown-linux-gnu");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const TARGET: Option<&str> = Some("aarch64-apple-darwin");
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const TARGET: Option<&str> = Some("x86_64-apple-darwin");
#[cfg(all(windows, target_arch = "x86_64"))]
pub const TARGET: Option<&str> = Some("x86_64-pc-windows-msvc");
#[cfg(not(any(
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64"),
        any(target_env = "musl", target_env = "gnu")
    ),
    all(target_os = "macos", any(target_arch = "x86_64", target_arch = "aarch64")),
    all(windows, target_arch = "x86_64"),
)))]
pub const TARGET: Option<&str> = None;

pub fn archive_stem(v: &Version, target: &str) -> String {
    format!("t3cm-{v}-{target}")
}

pub fn archive_name(v: &Version, target: &str) -> String {
    let ext = if target.contains("-windows-") { "zip" } else { "tar.gz" };
    format!("{}.{ext}", archive_stem(v, target))
}

pub fn listed_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim_end().split_once(char::is_whitespace)?;
        let file = file.trim_start();
        let file = file.strip_prefix('*').unwrap_or(file);
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

pub fn verify_sum(bytes: &[u8], sums: &str, name: &str) -> Result<()> {
    let want = listed_sum(sums, name).ok_or_else(|| anyhow!("{name} is not listed in SHA256SUMS"))?;
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        bail!("checksum mismatch for {name}: SHA256SUMS says {want}, the download is {got}");
    }
    Ok(())
}

pub fn cargo_build(exe: &Path) -> bool {
    let mut up = exe.ancestors().skip(1);
    let profile = up.next().and_then(|p| p.file_name()).and_then(|n| n.to_str());
    let in_target = exe.ancestors().any(|a| a.file_name().is_some_and(|n| n == "target"));
    matches!(profile, Some("release" | "debug" | "dist")) && in_target
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch> {
        let mut b = [0u8; 8];
        let _ = getrandom::fill(&mut b);
        let tag: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("t3cm-update-{tag}"));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        platform::private_dir(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let run = async {
        let resp = client.get(url).send().await.with_context(|| format!("download failed: {url}"))?;
        if !resp.status().is_success() {
            bail!("download failed: {url} answered {}", resp.status());
        }
        Ok(resp.bytes().await.with_context(|| format!("download failed: {url}"))?.to_vec())
    };
    tokio::time::timeout(DOWNLOAD_DEADLINE, run)
        .await
        .map_err(|_| anyhow!("download of {url} took longer than {}s", DOWNLOAD_DEADLINE.as_secs()))?
}

async fn quiet(cmd: &str, args: &[&str]) -> Option<std::process::Output> {
    tokio::process::Command::new(cmd).args(args).stdin(Stdio::null()).env("GH_PROMPT_DISABLED", "1").output().await.ok()
}

async fn ok(cmd: &str, args: &[&str]) -> bool {
    quiet(cmd, args).await.is_some_and(|o| o.status.success())
}

async fn verify_provenance(file: &Path) -> Result<()> {
    if !ok("gh", &["auth", "token", "--hostname", "github.com"]).await {
        println!("gh is missing or not signed in; skipped the provenance check (gh attestation verify).");
        return Ok(());
    }
    let path = file.to_string_lossy();
    let out = quiet("gh", &["attestation", "verify", &path, "-R", REPO])
        .await
        .ok_or_else(|| anyhow!("could not run gh attestation verify"))?;
    if !out.status.success() {
        bail!(
            "gh attestation verify failed for {}: {}",
            file.file_name().unwrap_or_default().to_string_lossy(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    println!("Provenance OK: the archive has a build attestation from {REPO}");
    Ok(())
}

fn tar_program() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        root.join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    }
}

async fn extract(archive: &Path, into: &Path) -> Result<()> {
    let flags = if cfg!(windows) { "-xf" } else { "-xzf" };
    let out = tokio::process::Command::new(tar_program())
        .arg(flags)
        .arg(archive)
        .arg("-C")
        .arg(into)
        .stdin(Stdio::null())
        .output()
        .await
        .context("could not run tar to unpack the release")?;
    if !out.status.success() {
        bail!("tar could not unpack {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

async fn reported_version(bin: &Path) -> Result<String> {
    let out = tokio::process::Command::new(bin)
        .arg("--version")
        .env(release::NO_CHECK_ENV, "1")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("could not run {}", bin.display()))?;
    if !out.status.success() {
        bail!("{} --version failed; is it the right build for this machine?", bin.display());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn served_version(port: u16) -> Option<String> {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().ok()?;
    let v: serde_json::Value =
        client.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok()?.json().await.ok()?;
    v["version"].as_str().map(str::to_string)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Manager {
    Systemd,
    Launchd,
    Task,
}

const UNIT: &str = "t3-codemode.service";
const LABEL: &str = "com.ottoops.t3-codemode";
const TASK: &str = "t3-codemode";
const TASK_END_WAIT: Duration = Duration::from_secs(10);

#[cfg(unix)]
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

type Step = (&'static str, Vec<String>);

fn step(cmd: &'static str, args: &[&str]) -> Step {
    (cmd, args.iter().map(|a| a.to_string()).collect())
}

fn shown(s: &Step) -> String {
    std::iter::once(s.0.to_string()).chain(s.1.iter().cloned()).collect::<Vec<_>>().join(" ")
}

fn powershell(script: &str) -> Step {
    let script = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {script}");
    step("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])
}

impl Manager {
    fn job(uid: u32) -> String {
        format!("gui/{uid}/{LABEL}")
    }

    fn probe(self, uid: u32) -> Step {
        match self {
            Manager::Systemd => step("systemctl", &["--user", "is-active", "--quiet", UNIT]),
            Manager::Launchd => step("launchctl", &["print", &Manager::job(uid)]),
            Manager::Task => powershell(&format!("(Get-ScheduledTask -TaskName '{TASK}').State")),
        }
    }

    fn restart_steps(self, uid: u32) -> Vec<Step> {
        match self {
            Manager::Systemd => vec![step("systemctl", &["--user", "restart", UNIT])],
            Manager::Launchd => vec![step("launchctl", &["kickstart", "-k", &Manager::job(uid)])],
            Manager::Task => vec![step("schtasks", &["/End", "/TN", TASK]), step("schtasks", &["/Run", "/TN", TASK])],
        }
    }

    fn restart_hint(self, uid: u32) -> String {
        self.restart_steps(uid).iter().map(shown).collect::<Vec<_>>().join("; ")
    }

    fn stop_hint(self, uid: u32) -> String {
        match self {
            Manager::Systemd => format!("systemctl --user stop {UNIT}"),
            Manager::Launchd => format!("launchctl kill TERM {}", Manager::job(uid)),
            Manager::Task => format!("schtasks /End /TN {TASK}"),
        }
    }

    fn drains(self) -> bool {
        self == Manager::Systemd
    }
}

fn launchd_field<'a>(print: &'a str, key: &str) -> Option<&'a str> {
    print.lines().find_map(|l| {
        let top = l.strip_prefix('\t').filter(|r| !r.starts_with(char::is_whitespace))?;
        top.strip_prefix(key)?.strip_prefix(" = ").map(str::trim)
    })
}

fn launchd_program(print: &str) -> Option<PathBuf> {
    let first_arg = || {
        let mut lines = print.lines().skip_while(|l| l.trim_end() != "\targuments = {");
        lines.nth(1).map(str::trim).filter(|a| !a.is_empty() && *a != "}")
    };
    launchd_field(print, "program").or_else(first_arg).map(PathBuf::from)
}

fn systemd_exec_path(show: &str) -> Option<PathBuf> {
    let path = show.split_once("path=")?.1.split(" ;").next()?.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn launchd_running(print: &str) -> bool {
    launchd_field(print, "state") == Some("running")
}

fn launchd_pid(print: &str) -> Option<u32> {
    launchd_field(print, "pid")?.parse().ok().filter(|p| *p > 0)
}

async fn run_step(s: &Step) -> Option<std::process::Output> {
    let args: Vec<&str> = s.1.iter().map(String::as_str).collect();
    quiet(s.0, &args).await
}

async fn manager() -> Option<Manager> {
    if cfg!(target_os = "linux") {
        ok("systemctl", &["--user", "cat", UNIT]).await.then_some(Manager::Systemd)
    } else if cfg!(target_os = "macos") {
        ok("launchctl", &["print", &Manager::job(uid())]).await.then_some(Manager::Launchd)
    } else if cfg!(windows) {
        ok("schtasks", &["/Query", "/TN", TASK]).await.then_some(Manager::Task)
    } else {
        None
    }
}

async fn active(m: Manager) -> bool {
    let Some(out) = run_step(&m.probe(uid())).await.filter(|o| o.status.success()) else { return false };
    let text = String::from_utf8_lossy(&out.stdout);
    match m {
        Manager::Systemd => true,
        Manager::Launchd => launchd_running(&text),
        Manager::Task => text.trim() == "Running",
    }
}

async fn service_pid(m: Manager) -> Option<u32> {
    match m {
        Manager::Systemd => {
            let out = quiet("systemctl", &["--user", "show", "-p", "MainPID", "--value", UNIT]).await?;
            if !out.status.success() {
                return None;
            }
            String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|p| *p > 0)
        }
        Manager::Launchd => {
            let out = run_step(&m.probe(uid())).await.filter(|o| o.status.success())?;
            launchd_pid(&String::from_utf8_lossy(&out.stdout))
        }
        Manager::Task => None,
    }
}

async fn service_exe(m: Manager) -> Option<PathBuf> {
    match m {
        Manager::Systemd | Manager::Launchd => platform::process_exe(service_pid(m).await?),
        Manager::Task => task_exe().await,
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_proc_addr(hex: &str) -> Option<std::net::IpAddr> {
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        8 => u32::from_str_radix(hex, 16).ok().map(|v| std::net::Ipv4Addr::from(v.to_ne_bytes()).into()),
        32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let w = u32::from_str_radix(&hex[i * 8..(i + 1) * 8], 16).ok()?;
                *chunk = w.to_ne_bytes();
            }
            Some(std::net::Ipv6Addr::from(bytes).into())
        }
        _ => None,
    }
}

#[cfg(any(target_os = "linux", test))]
fn proc_listen_inode(table: &str, port: u16) -> Option<u64> {
    let mut exact = None;
    let mut wildcard = None;
    for line in table.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let Some(local) = cols.nth(1) else { continue };
        let Some(state) = cols.nth(1) else { continue };
        if !state.eq_ignore_ascii_case("0A") {
            continue;
        }
        let Some((addr, hex_port)) = local.split_once(':') else { continue };
        if u16::from_str_radix(hex_port, 16).ok() != Some(port) {
            continue;
        }
        let Some(ip) = parse_proc_addr(addr) else { continue };
        let Some(inode) = cols.nth(5).and_then(|s| s.parse::<u64>().ok()).filter(|i| *i > 0) else {
            continue;
        };
        let is_exact = match ip {
            std::net::IpAddr::V4(v4) => v4 == std::net::Ipv4Addr::LOCALHOST,
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped() == Some(std::net::Ipv4Addr::LOCALHOST),
        };
        if is_exact {
            if exact.is_some_and(|prev| prev != inode) {
                return None;
            }
            exact = Some(inode);
        } else if ip.is_unspecified() {
            wildcard = Some(if wildcard.is_some_and(|prev| prev != inode) { 0 } else { inode });
        }
    }
    exact.or_else(|| wildcard.filter(|i| *i > 0))
}

#[cfg(target_os = "linux")]
fn proc_socket_pid(inode: u64) -> Option<u32> {
    let want = format!("socket:[{inode}]");
    let want = std::ffi::OsStr::new(&want);
    let mut found = None;
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()).filter(|p| *p > 0) else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path()).is_ok_and(|p| p.as_os_str() == want) {
                if found.is_some_and(|prev| prev != pid) {
                    return None;
                }
                found = Some(pid);
                break;
            }
        }
    }
    found
}

#[cfg(any(target_os = "macos", windows, test))]
fn unique_pid(output: &str) -> Option<u32> {
    let mut found = None;
    for line in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let pid: u32 = line.parse().ok().filter(|p| *p > 0)?;
        if found.is_some_and(|prev| prev != pid) {
            return None;
        }
        found = Some(pid);
    }
    found
}

#[cfg(target_os = "linux")]
async fn listener_pid(port: u16) -> Option<u32> {
    let inode = std::fs::read_to_string("/proc/net/tcp")
        .ok()
        .and_then(|t| proc_listen_inode(&t, port))
        .or_else(|| std::fs::read_to_string("/proc/net/tcp6").ok().and_then(|t| proc_listen_inode(&t, port)))?;
    proc_socket_pid(inode)
}

#[cfg(target_os = "macos")]
async fn listener_pid(port: u16) -> Option<u32> {
    let out = quiet("lsof", &["-nP", &format!("-iTCP@127.0.0.1:{port}"), "-sTCP:LISTEN", "-t"]).await?;
    out.status.success().then(|| unique_pid(&String::from_utf8_lossy(&out.stdout)))?
}

#[cfg(windows)]
async fn listener_pid(port: u16) -> Option<u32> {
    let out = run_step(&powershell(&format!(
        "(Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort {port} -State Listen -ErrorAction SilentlyContinue).OwningProcess"
    )))
    .await?;
    out.status.success().then(|| unique_pid(&String::from_utf8_lossy(&out.stdout)))?
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn listener_pid(_port: u16) -> Option<u32> {
    None
}

fn scheduler_parent_probe(pid: u32) -> Step {
    powershell(&format!(
        "$p = (Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').ParentProcessId; \
         $s = (Get-CimInstance Win32_Service -Filter 'Name=''Schedule''').ProcessId; \
         if ($p -and $s) {{ $p -eq $s }}"
    ))
}

async fn started_by_scheduler(pid: u32) -> Option<bool> {
    let out = run_step(&scheduler_parent_probe(pid)).await.filter(|o| o.status.success())?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

fn matches_task_exe(proc_exe: &Path, launch: &Path) -> bool {
    let proc_exe = canonical(proc_exe.to_path_buf());
    proc_exe == canonical(launch.to_path_buf()) || proc_exe == canonical(sibling(launch, ".old"))
}

enum ListenerOwner {
    Service,
    Other,
    Unknown,
}

async fn listener_owner(m: Manager, port: u16, launch: &Path) -> ListenerOwner {
    let Some(listener) = listener_pid(port).await else { return ListenerOwner::Unknown };
    match m {
        Manager::Systemd | Manager::Launchd => match service_pid(m).await {
            Some(svc) if svc == listener => ListenerOwner::Service,
            Some(_) => ListenerOwner::Other,
            None => ListenerOwner::Unknown,
        },
        Manager::Task => match platform::process_exe(listener) {
            Some(proc_exe) if matches_task_exe(&proc_exe, launch) => match started_by_scheduler(listener).await {
                Some(true) => ListenerOwner::Service,
                Some(false) => ListenerOwner::Other,
                None => ListenerOwner::Unknown,
            },
            Some(_) => ListenerOwner::Other,
            None => ListenerOwner::Unknown,
        },
    }
}

async fn task_exe() -> Option<PathBuf> {
    let out = run_step(&powershell(&format!("(Get-ScheduledTask -TaskName '{TASK}').Actions[0].Execute"))).await?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

async fn launch_exe(m: Manager) -> Option<PathBuf> {
    match m {
        Manager::Systemd => {
            let out = quiet("systemctl", &["--user", "show", "-p", "ExecStart", "--value", UNIT]).await?;
            out.status.success().then(|| systemd_exec_path(&String::from_utf8_lossy(&out.stdout)))?
        }
        Manager::Launchd => {
            let out = run_step(&m.probe(uid())).await.filter(|o| o.status.success())?;
            launchd_program(&String::from_utf8_lossy(&out.stdout))
        }
        Manager::Task => task_exe().await,
    }
}

async fn restart(m: Manager) -> Result<()> {
    let steps = m.restart_steps(uid());
    let last = steps.len() - 1;
    for (i, s) in steps.iter().enumerate() {
        let out = run_step(s).await.ok_or_else(|| anyhow!("could not run {}", s.0))?;
        if i < last {
            let start = Instant::now();
            while active(m).await && start.elapsed() < TASK_END_WAIT {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            continue;
        }
        if !out.status.success() {
            bail!("{} failed: {}", shown(s), String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    Ok(())
}

async fn await_version(m: Manager, port: u16, want: &str) -> Result<Duration> {
    let start = Instant::now();
    let mut down = 0;
    loop {
        if served_version(port).await.as_deref() == Some(want) {
            return Ok(start.elapsed());
        }
        down = if active(m).await { 0 } else { down + 1 };
        if down >= 3 {
            bail!("the service stopped before it reported {want} on 127.0.0.1:{port}");
        }
        if start.elapsed() > HEALTH_WAIT {
            bail!("the service did not report {want} on 127.0.0.1:{port} within {}s", HEALTH_WAIT.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(crate) fn plain(p: &Path) -> String {
    let s = p.display().to_string();
    match (s.strip_prefix(r"\\?\UNC\"), s.strip_prefix(r"\\?\")) {
        (Some(unc), _) => format!(r"\\{unc}"),
        (None, Some(local)) => local.to_string(),
        (None, None) => s,
    }
}

fn manual_steps(m: Manager, exe: &Path, old: &Path) -> Option<(String, String)> {
    let (exe, old) = (plain(exe), plain(old));
    let kick = m.restart_hint(uid());
    match m {
        Manager::Systemd => None,
        Manager::Launchd => Some((kick.clone(), format!("mv '{old}' '{exe}' && {kick}"))),
        Manager::Task => Some((
            kick,
            format!(
                "schtasks /End /TN {TASK}; Move-Item -Force -LiteralPath '{old}' -Destination '{exe}'; schtasks /Run /TN {TASK}"
            ),
        )),
    }
}

fn unit_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| platform::home().join(".config"))
        .join("systemd")
        .join("user")
        .join(UNIT)
}

const SHIPPED_UNIT: &[u8] = include_bytes!("../deploy/t3-codemode.service");

fn unit_changed(local: &[u8], packaged: &[u8], shipped: &[u8]) -> bool {
    packaged != shipped && local != packaged
}

fn save_unit(packaged: &Path, state_dir: &Path) -> Result<Option<PathBuf>> {
    let Ok(new) = std::fs::read(packaged) else { return Ok(None) };
    let Ok(local) = std::fs::read(unit_path()) else { return Ok(None) };
    if !unit_changed(&local, &new, SHIPPED_UNIT) {
        return Ok(None);
    }
    std::fs::create_dir_all(state_dir).with_context(|| format!("creating {}", state_dir.display()))?;
    let copy = state_dir.join(UNIT);
    std::fs::write(&copy, &new).with_context(|| format!("writing {}", copy.display()))?;
    Ok(Some(copy))
}

#[derive(Debug)]
struct UpdateLock(std::fs::File);

impl Drop for UpdateLock {
    fn drop(&mut self) {
        // Unlock explicitly before closing so a sibling thread's forked child
        // that transiently inherited this fd cannot keep the flock held.
        let _ = self.0.unlock();
    }
}

fn lock_updates(state_dir: &Path) -> Result<UpdateLock> {
    std::fs::create_dir_all(state_dir).with_context(|| format!("creating {}", state_dir.display()))?;
    let path = state_dir.join("update.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match f.try_lock() {
        Ok(()) => Ok(UpdateLock(f)),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("another t3cm update is running (it holds {}); try again when it finishes", path.display())
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e).with_context(|| format!("locking {}", path.display())),
    }
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

struct Swap {
    exe: PathBuf,
    old: PathBuf,
}

impl Swap {
    fn install(exe: &Path, new: &Path) -> Result<Swap> {
        let old = sibling(exe, ".old");
        let staged = sibling(exe, ".new");
        let _ = std::fs::remove_file(&old);
        std::fs::copy(new, &staged).with_context(|| format!("writing {}", staged.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        #[cfg(unix)]
        {
            if std::fs::hard_link(exe, &old).is_err() {
                std::fs::copy(exe, &old).with_context(|| format!("backing up {}", exe.display()))?;
            }
            if let Err(e) = std::fs::rename(&staged, exe) {
                let _ = std::fs::remove_file(&staged);
                let _ = std::fs::remove_file(&old);
                return Err(e).with_context(|| format!("putting the new binary at {}", exe.display()));
            }
        }
        #[cfg(not(unix))]
        {
            std::fs::rename(exe, &old).with_context(|| format!("moving {} aside", exe.display()))?;
            if let Err(e) = std::fs::rename(&staged, exe) {
                let _ = std::fs::rename(&old, exe);
                let _ = std::fs::remove_file(&staged);
                return Err(e).with_context(|| format!("putting the new binary at {}", exe.display()));
            }
        }
        Ok(Swap { exe: exe.to_path_buf(), old })
    }

    #[cfg(unix)]
    fn roll_back(&self) -> Result<()> {
        std::fs::rename(&self.old, &self.exe).with_context(|| format!("restoring {}", self.exe.display()))
    }

    #[cfg(not(unix))]
    fn roll_back(&self) -> Result<()> {
        let failed = sibling(&self.exe, ".failed");
        let _ = std::fs::remove_file(&failed);
        std::fs::rename(&self.exe, &failed).with_context(|| format!("moving {} aside", self.exe.display()))?;
        std::fs::rename(&self.old, &self.exe).with_context(|| format!("restoring {}", self.exe.display()))?;
        let _ = std::fs::remove_file(&failed);
        Ok(())
    }

    fn finish(&self) {
        let _ = std::fs::remove_file(&self.old);
    }
}

fn skills_managed() -> Result<Option<skills::Paths>> {
    let paths = skills::Paths::from_env()?;
    Ok(skills::Manifest::load(&paths.manifest)?.is_some().then_some(paths))
}

fn skills_behind() -> Result<bool> {
    let Some(paths) = skills_managed()? else { return Ok(false) };
    let bundle = skills::Bundle::embedded();
    let st = skills::status(&paths, &bundle)?;
    Ok(!drift::current(&st, &steps::check(&steps::Env::from_env()?, &bundle)?))
}

fn offer_take_over() -> Result<()> {
    let paths = skills::Paths::from_env()?;
    if let Some(f) = skills::status(&paths, &skills::Bundle::embedded())?.foreign {
        println!(
            "The workflow skills in {} came from {}, so t3cm left them alone. To have t3cm keep them current: t3cm skills install --take-over",
            paths.dir.display(),
            f.tool
        );
    }
    Ok(())
}

async fn sync_skills(exe: &Path, release: &Version) -> Result<()> {
    if skills_managed()?.is_none() {
        return offer_take_over();
    }
    let done = tokio::process::Command::new(exe)
        .args(["skills", "update"])
        .env(release::NO_CHECK_ENV, "1")
        .stdin(Stdio::null())
        .status()
        .await
        .with_context(|| format!("could not run {} skills update", exe.display()))?;
    if !done.success() {
        bail!(
            "t3cm {release} is installed, but its skills update failed; fix what it printed, then run: t3cm skills update"
        );
    }
    Ok(())
}

fn up_to_date(current: &Version, published: &Version) -> String {
    if published < current {
        format!("t3cm {current} is up to date (newer than the latest published release, {published}).")
    } else {
        format!("t3cm {current} is up to date (latest release: {published}).")
    }
}

fn restart_hint(found: Option<Manager>) -> String {
    found.map(|m| m.restart_hint(uid())).unwrap_or_else(|| platform::SERVICE.restart.to_string())
}

fn canonical(p: PathBuf) -> PathBuf {
    p.canonicalize().unwrap_or(p)
}

async fn restart_stale(o: &Opts, current: &Version) -> Result<()> {
    let Some(served) = served_version(o.port).await else { return Ok(()) };
    if served == current.to_string() {
        return Ok(());
    }
    let port = o.port;
    let found = manager().await;
    let hint = restart_hint(found);
    if Version::parse(&served).is_none_or(|v| v > *current) {
        bail!(
            "a t3cm {served} answers on 127.0.0.1:{port}, but this is t3cm {current}; run the t3cm the service runs, or restart the service on this one: {hint}"
        );
    }
    let Some(m) = found.filter(|_| o.restart) else {
        bail!(
            "the service on 127.0.0.1:{port} still runs t3cm {served}, not {current}; restart it to run {current}: {hint}"
        );
    };
    if !active(m).await {
        bail!(
            "a t3cm {served} answers on 127.0.0.1:{port}, but the service is not running; stop that t3cm, then start the service: {hint}"
        );
    }
    let exe = canonical(std::env::current_exe()?);
    let runs = launch_exe(m)
        .await
        .ok_or_else(|| anyhow!("could not tell which binary the service starts; restart it ({hint}), then update"))?;
    if canonical(runs.clone()) != exe {
        bail!(
            "the service runs {}, but this is {}; run `{} update` instead, or restart the service: {hint}",
            runs.display(),
            exe.display(),
            runs.display()
        );
    }
    match listener_owner(m, port, &runs).await {
        ListenerOwner::Service => {}
        ListenerOwner::Other => bail!(
            "a t3cm {served} answers on 127.0.0.1:{port}, but that listener is not the service; stop it, or pass the service's port with --port (or restart the service: {hint})"
        ),
        ListenerOwner::Unknown => bail!(
            "a t3cm {served} answers on 127.0.0.1:{port}, but could not tell whether that listener is the service; pass the service's port with --port, or restart the service: {hint}"
        ),
    }
    println!("The service still runs t3cm {served}; restarting it on {current}.");
    let restarted = match restart(m).await {
        Ok(()) => await_version(m, port, &current.to_string()).await,
        Err(e) => Err(e),
    };
    let took = restarted.with_context(|| {
        format!(
            "restart the service with {hint}; check it with {} and its logs at {}",
            platform::SERVICE.status,
            platform::SERVICE.logs
        )
    })?;
    println!("The service answered on t3cm {current} in {:.1}s.", took.as_secs_f64());
    Ok(())
}

pub async fn run(o: Opts) -> Result<()> {
    let releases = release::releases_url()?;
    let current = Version::current();
    let asked = match &o.tag {
        Some(t) => Some(Version::parse(t).ok_or_else(|| anyhow!("{t:?} is not a version like v0.5.0"))?),
        None => None,
    };
    let want = match &asked {
        Some(v) => v.clone(),
        None => {
            let v = release::latest(&releases).await?;
            let _ = release::write_cache(&o.state_dir, &Checked::now(&v));
            v
        }
    };
    if o.check {
        match release::notice(&current, &want, &releases) {
            Some(n) => println!("{n}"),
            None => println!("{}", up_to_date(&current, &want)),
        }
        return Ok(());
    }
    if want == current || (asked.is_none() && want < current) {
        let _lock = lock_updates(&o.state_dir)?;
        restart_stale(&o, &current).await?;
        println!("{}", up_to_date(&current, &want));
        if skills_managed()?.is_none() {
            if let Err(e) = offer_take_over() {
                eprintln!("warn: workflow skills not checked ({e:#})");
            }
        } else if skills_behind()? {
            return sync_skills(&std::env::current_exe()?, &current).await;
        }
        return Ok(());
    }
    if want < current {
        println!("Going back from {current} to {want}, as asked.");
    }

    let exe = std::env::current_exe()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if cargo_build(&exe) {
        bail!(
            "this t3cm runs from a cargo build ({}); pull the checkout and run deploy/install.sh --from-source instead",
            exe.display()
        );
    }
    let target = TARGET.ok_or_else(|| anyhow!("no release build for this platform; build from source"))?;
    let _lock = lock_updates(&o.state_dir)?;
    let on_disk = reported_version(&exe).await?;
    if on_disk != format!("t3cm {current}") {
        bail!(
            "{} is now {on_disk:?}, not the t3cm {current} running this update; run `t3cm update` again",
            exe.display()
        );
    }
    let name = archive_name(&want, target);
    let client = release::client(&releases, true)?;
    let tag = want.tag();
    println!("Downloading t3cm {want} ({target})");
    let archive = fetch(&client, &format!("{releases}/download/{tag}/{name}")).await?;
    let sums = String::from_utf8(fetch(&client, &format!("{releases}/download/{tag}/SHA256SUMS")).await?)
        .context("SHA256SUMS is not text")?;
    verify_sum(&archive, &sums, &name)?;
    println!("Checksum OK: {name}");
    let work = Scratch::new()?;
    let file = work.0.join(&name);
    std::fs::write(&file, &archive)?;
    verify_provenance(&file).await?;
    let unpacked = work.0.join("x");
    std::fs::create_dir_all(&unpacked)?;
    extract(&file, &unpacked).await?;
    let dir = unpacked.join(archive_stem(&want, target));
    let new = dir.join(platform::exe_name("t3cm"));
    if !new.is_file() {
        bail!("{name} has no {}", Path::new(&archive_stem(&want, target)).join(platform::exe_name("t3cm")).display());
    }
    let says = reported_version(&new).await?;
    if says != format!("t3cm {want}") {
        bail!("the binary in {name} reports {says:?}, not t3cm {want}");
    }

    let found = manager().await;
    let served = served_version(o.port).await;
    if let Some(v) = served.as_deref().filter(|v| *v != current.to_string()) {
        bail!(
            "a t3cm {v} answers on 127.0.0.1:{}, but {} is t3cm {current}; restart the service so it runs {current} ({}), then update",
            o.port,
            exe.display(),
            restart_hint(found)
        );
    }
    let running = match found {
        Some(m) => active(m).await,
        None => false,
    };
    if let Some(m) = found.filter(|_| running && served.is_none()) {
        bail!(
            "the service is running but nothing answers on 127.0.0.1:{}; pass its port with --port, or stop it ({}) and run `t3cm update` again",
            o.port,
            m.stop_hint(uid())
        );
    }
    if let Some(m) = found {
        let runs = if running {
            service_exe(m).await.ok_or_else(|| {
                anyhow!(
                    "could not tell which binary the running service runs; restart it ({}), then update",
                    m.restart_hint(uid())
                )
            })?
        } else {
            platform::installed_exe()
        };
        if runs.canonicalize().unwrap_or_else(|_| runs.clone()) != exe {
            bail!(
                "the service runs {}, but this is {}; run `{} update` instead",
                runs.display(),
                exe.display(),
                runs.display()
            );
        }
    }

    let unit = match found {
        Some(Manager::Systemd) => save_unit(&dir.join(UNIT), &o.state_dir).unwrap_or_else(|e| {
            println!("Could not save this release's {UNIT}: {e:#}");
            None
        }),
        _ => None,
    };
    if let Some(copy) = &unit {
        println!(
            "This release changes {UNIT}. {} was left as it is, and the new unit is saved at {}. \
             To use it, merge any edits of yours into the copy, then run: cp '{}' '{}' && systemctl --user daemon-reload",
            unit_path().display(),
            copy.display(),
            copy.display(),
            unit_path().display()
        );
    }

    let swap = Swap::install(&exe, &new)?;
    println!("Installed t3cm {want} at {}", exe.display());
    let Some(m) = found.filter(|_| o.restart && running) else {
        println!("t3cm {current} is kept at {} to go back to.", swap.old.display());
        match found.and_then(|m| manual_steps(m, &exe, &swap.old)) {
            Some((restart, back)) => {
                println!("Restart the service to run {want} (this ends runs in flight): {restart}");
                println!("To go back to {current}: {back}");
            }
            None if running => {
                println!("The service still runs {current}; restart it to run {want}: systemctl --user restart {UNIT}")
            }
            None if found.is_some() => println!("The service is stopped; it runs {want} when it next starts."),
            None if served.is_some() => {
                println!("A t3cm {current} still answers on 127.0.0.1:{}; restart it to run {want}.", o.port)
            }
            None => {}
        }
        return sync_skills(&exe, &want).await;
    };

    if m.drains() {
        println!("Restarting the service; runs in flight finish first.");
    } else {
        println!("Restarting the service; this ends runs in flight.");
    }
    let cutover = match restart(m).await {
        Ok(()) => await_version(m, o.port, &want.to_string()).await,
        Err(e) => Err(e),
    };
    match cutover {
        Ok(took) => {
            swap.finish();
            println!(
                "t3cm updated: {current} -> {want}. The service answered on the new version in {:.1}s.",
                took.as_secs_f64()
            );
            sync_skills(&exe, &want).await
        }
        Err(e) => {
            let back = async {
                swap.roll_back()?;
                restart(m).await?;
                await_version(m, o.port, &current.to_string()).await.map(|_| ())
            };
            let hint = match &unit {
                Some(copy) => format!(
                    " If {want} needs its new unit, install {} as shown above and update again.",
                    copy.display()
                ),
                None => String::new(),
            };
            let svc = &platform::SERVICE;
            match back.await {
                Ok(()) => bail!("{e:#}. Rolled back to {current}, which is serving again.{hint} Logs: {}", svc.logs),
                Err(r) => bail!(
                    "{e:#}. Rolling back to {current} failed too: {r:#}. Check: {}; logs: {}",
                    svc.status,
                    svc.logs
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_are_named_like_the_release_workflow_names_them() {
        let v = Version::parse("0.5.0").unwrap();
        assert_eq!(archive_name(&v, "x86_64-unknown-linux-musl"), "t3cm-0.5.0-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(archive_name(&v, "x86_64-pc-windows-msvc"), "t3cm-0.5.0-x86_64-pc-windows-msvc.zip");
    }

    #[test]
    fn sums_lines_match_by_exact_name() {
        let h = "a".repeat(64);
        let sums = format!("{h}  t3cm-1.tar.gz\n{}  *t3cm-1.zip\n", "B".repeat(64));
        assert_eq!(listed_sum(&sums, "t3cm-1.tar.gz").as_deref(), Some(h.as_str()));
        assert_eq!(listed_sum(&sums, "t3cm-1.zip").unwrap(), "b".repeat(64));
        assert!(listed_sum(&sums, "t3cm-1.tar").is_none());
        let good: String = Sha256::digest(b"hi").iter().map(|b| format!("{b:02x}")).collect();
        assert!(verify_sum(b"hi", &format!("{good}  f\n"), "f").is_ok());
        assert!(verify_sum(b"ho", &format!("{good}  f\n"), "f").unwrap_err().to_string().contains("mismatch"));
    }

    #[test]
    fn printed_windows_steps_drop_the_verbatim_prefix() {
        assert_eq!(plain(Path::new(r"\\?\C:\Users\a\t3cm.exe")), r"C:\Users\a\t3cm.exe");
        assert_eq!(plain(Path::new(r"\\?\UNC\srv\share\t3cm.exe")), r"\\srv\share\t3cm.exe");
        assert_eq!(plain(Path::new("/home/a/t3cm")), "/home/a/t3cm");
        let (_, back) =
            manual_steps(Manager::Task, Path::new(r"\\?\C:\p\t3cm.exe"), Path::new(r"\\?\C:\p\t3cm.old")).unwrap();
        assert!(back.contains(r"Move-Item -Force -LiteralPath 'C:\p\t3cm.old' -Destination 'C:\p\t3cm.exe'"), "{back}");
    }

    fn argv(steps: &[Step]) -> Vec<String> {
        steps.iter().map(shown).collect()
    }

    #[test]
    fn each_manager_restarts_its_own_service() {
        assert_eq!(argv(&Manager::Systemd.restart_steps(501)), ["systemctl --user restart t3-codemode.service"]);
        assert_eq!(
            argv(&Manager::Launchd.restart_steps(501)),
            ["launchctl kickstart -k gui/501/com.ottoops.t3-codemode"]
        );
        assert_eq!(
            argv(&Manager::Task.restart_steps(0)),
            ["schtasks /End /TN t3-codemode", "schtasks /Run /TN t3-codemode"]
        );
        assert_eq!(Manager::Task.restart_hint(0), "schtasks /End /TN t3-codemode; schtasks /Run /TN t3-codemode");
        assert!(Manager::Systemd.drains() && !Manager::Launchd.drains() && !Manager::Task.drains());
    }

    #[test]
    fn each_manager_probes_its_own_service() {
        assert_eq!(shown(&Manager::Systemd.probe(501)), "systemctl --user is-active --quiet t3-codemode.service");
        assert_eq!(shown(&Manager::Launchd.probe(501)), "launchctl print gui/501/com.ottoops.t3-codemode");
        let (cmd, args) = Manager::Task.probe(0);
        assert_eq!(cmd, "powershell");
        assert_eq!(
            args.last().unwrap(),
            "[Console]::OutputEncoding = [Text.Encoding]::UTF8; (Get-ScheduledTask -TaskName 't3-codemode').State"
        );
        assert_eq!(Manager::Launchd.stop_hint(501), "launchctl kill TERM gui/501/com.ottoops.t3-codemode");
        assert_eq!(Manager::Task.stop_hint(0), "schtasks /End /TN t3-codemode");
    }

    const LAUNCHD_RUNNING: &str = "gui/501/com.ottoops.t3-codemode = {
\tactive count = 1
\tpath = /Users/a/Library/LaunchAgents/com.ottoops.t3-codemode.plist
\ttype = LaunchAgent
\tstate = running

\tprogram = /Users/a/.local/bin/t3cm
\targuments = {
\t\t/Users/a/.local/bin/t3cm
\t\tserve
\t}

\tendpoints = {
\t\tstate = waiting
\t\tpid = 7
\t}
\tpid = 4242
\tlast exit code = (never exited)
}
";

    #[test]
    fn the_service_managers_name_the_binary_they_start() {
        assert_eq!(launchd_program(LAUNCHD_RUNNING), Some(PathBuf::from("/Users/a/.local/bin/t3cm")));
        let no_program = LAUNCHD_RUNNING.replace("\tprogram = /Users/a/.local/bin/t3cm\n", "");
        assert_eq!(launchd_program(&no_program), Some(PathBuf::from("/Users/a/.local/bin/t3cm")));
        let show = "{ path=/home/a/.local/bin/t3cm ; argv[]=/home/a/.local/bin/t3cm serve ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }\n";
        assert_eq!(systemd_exec_path(show), Some(PathBuf::from("/home/a/.local/bin/t3cm")));
        assert_eq!(systemd_exec_path(""), None);
    }

    #[test]
    fn launchctl_print_tells_whether_the_agent_runs_and_its_pid() {
        assert!(launchd_running(LAUNCHD_RUNNING));
        assert_eq!(launchd_pid(LAUNCHD_RUNNING), Some(4242));
        let stopped = "gui/501/com.ottoops.t3-codemode = {
\tstate = not running
\tlast exit code = 1
}
";
        assert!(!launchd_running(stopped));
        assert_eq!(launchd_pid(stopped), None);
        assert_eq!(launchd_pid("\tpid = 0\n"), None);
    }

    #[test]
    fn manual_steps_print_the_same_restart_the_update_runs() {
        let (kick, back) = manual_steps(Manager::Launchd, Path::new("/u/t3cm"), Path::new("/u/t3cm.old")).unwrap();
        assert_eq!(kick, Manager::Launchd.restart_hint(uid()));
        assert_eq!(back, format!("mv '/u/t3cm.old' '/u/t3cm' && {kick}"));
        assert!(manual_steps(Manager::Systemd, Path::new("/u/t3cm"), Path::new("/u/t3cm.old")).is_none());
    }

    #[test]
    fn a_unit_change_is_reported_only_when_the_release_makes_one() {
        assert!(!unit_changed(b"old", b"old", b"old"));
        assert!(!unit_changed(b"mine", b"old", b"old"));
        assert!(unit_changed(b"old", b"new", b"old"));
        assert!(unit_changed(b"mine", b"new", b"old"));
        assert!(!unit_changed(b"new", b"new", b"old"));
        assert!(SHIPPED_UNIT.starts_with(b"[Unit]"));
    }

    #[test]
    fn a_second_update_cannot_take_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock_updates(dir.path()).unwrap();
        let err = lock_updates(dir.path()).unwrap_err().to_string();
        assert!(err.contains("another t3cm update is running"), "{err}");
        drop(held);
        assert!(lock_updates(dir.path()).is_ok());
    }

    #[test]
    fn a_swap_keeps_the_old_binary_and_can_put_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("t3cm");
        let new = dir.path().join("download");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        let swap = Swap::install(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(sibling(&exe, ".old")).unwrap(), b"old");
        swap.roll_back().unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert!(!sibling(&exe, ".new").exists());
    }

    #[test]
    fn proc_net_tcp_lines_find_the_listening_loopback_inode() {
        let lo4 = format!("{:08X}", u32::from_ne_bytes([127, 0, 0, 1]));
        let other4 = format!("{:08X}", u32::from_ne_bytes([10, 0, 0, 1]));
        let v4 = format!(
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
               0: {other4}:BA73 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1\n\
               1: {lo4}:BA73 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 222 1\n\
               2: 00000000:BA73 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 333 1\n\
               3: {lo4}:BA73 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 444 1\n\
               4: 00000000:BA74 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 555 1\n\
               5: {lo4}:BA75 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 0 1\n"
        );
        assert_eq!(proc_listen_inode(&v4, 47731), Some(444));
        assert_eq!(proc_listen_inode(&v4, 47732), Some(555));
        assert_eq!(proc_listen_inode(&v4, 47733), None);
        assert_eq!(proc_listen_inode(&v4, 47734), None);

        let mapped_lo = format!(
            "0000000000000000{:08X}{:08X}",
            u32::from_ne_bytes([0, 0, 0xFF, 0xFF]),
            u32::from_ne_bytes([127, 0, 0, 1])
        );
        let lo6 = format!("000000000000000000000000{:08X}", u32::from_ne_bytes([0, 0, 0, 1]));
        let v6 = format!(
            "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
               0: {lo6}:BA73 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 666 1\n\
               1: {mapped_lo}:BA74 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 777 1\n\
               2: 00000000000000000000000000000000:BA75 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 888 1\n"
        );
        assert_eq!(proc_listen_inode(&v6, 47731), None);
        assert_eq!(proc_listen_inode(&v6, 47732), Some(777));
        assert_eq!(proc_listen_inode(&v6, 47733), Some(888));

        let dup = format!(
            "  sl local rem st q t r u to inode\n\
               0: {lo4}:BA73 00000000:0000 0A 0:0 0:0 0 1000 0 111 1\n\
               1: {lo4}:BA73 00000000:0000 0A 0:0 0:0 0 1000 0 222 1\n"
        );
        assert_eq!(proc_listen_inode(&dup, 47731), None);
    }

    #[test]
    fn unique_pid_accepts_one_positive_pid_and_refuses_ambiguity() {
        assert_eq!(unique_pid("4242\n"), Some(4242));
        assert_eq!(unique_pid(" 4242 \r\n4242\r\n"), Some(4242));
        assert_eq!(unique_pid("4242\n4243\n"), None);
        assert_eq!(unique_pid("0\n"), None);
        assert_eq!(unique_pid(""), None);
        assert_eq!(unique_pid("err\n"), None);
    }

    #[test]
    fn the_task_listener_must_be_a_child_of_the_task_scheduler_service() {
        let (cmd, args) = scheduler_parent_probe(4242);
        assert_eq!(cmd, "powershell");
        let script = args.last().unwrap();
        assert!(script.contains("Win32_Process -Filter 'ProcessId=4242'"), "{script}");
        assert!(script.contains("Win32_Service -Filter 'Name=''Schedule'''"), "{script}");
        assert!(script.contains("$p -eq $s"), "{script}");
    }

    #[test]
    fn task_exe_match_accepts_the_installed_binary_and_its_old_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("t3cm.exe");
        let old = sibling(&exe, ".old");
        let other = dir.path().join("other.exe");
        std::fs::write(&exe, b"new").unwrap();
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&other, b"other").unwrap();
        assert!(matches_task_exe(&exe, &exe));
        assert!(matches_task_exe(&old, &exe));
        assert!(!matches_task_exe(&other, &exe));
    }

    #[test]
    fn cargo_builds_are_recognised() {
        assert!(cargo_build(Path::new("/src/t3cm/target/release/t3cm")));
        assert!(cargo_build(Path::new("/src/t3cm/target/dist/t3cm")));
        assert!(cargo_build(Path::new("/src/t3cm/target/x86_64-unknown-linux-musl/debug/t3cm")));
        assert!(cargo_build(Path::new("/src/t3cm/target/x86_64-unknown-linux-musl/dist/t3cm")));
        assert!(!cargo_build(Path::new("/home/u/.local/bin/t3cm")));
        assert!(!cargo_build(Path::new("/opt/release/t3cm")));
        assert!(!cargo_build(Path::new("/opt/dist/t3cm")));
    }
}
