use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::platform;
use crate::release::{self, Checked, REPO, Version};

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
    matches!(profile, Some("release" | "debug")) && in_target
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

#[cfg(unix)]
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

async fn manager() -> Option<Manager> {
    if cfg!(target_os = "linux") {
        ok("systemctl", &["--user", "cat", UNIT]).await.then_some(Manager::Systemd)
    } else if cfg!(target_os = "macos") {
        ok("launchctl", &["print", &format!("gui/{}/{LABEL}", uid())]).await.then_some(Manager::Launchd)
    } else if cfg!(windows) {
        ok("schtasks", &["/Query", "/TN", TASK]).await.then_some(Manager::Task)
    } else {
        None
    }
}

async fn active() -> bool {
    ok("systemctl", &["--user", "is-active", "--quiet", UNIT]).await
}

async fn service_exe() -> Option<PathBuf> {
    let out = quiet("systemctl", &["--user", "show", "-p", "MainPID", "--value", UNIT]).await?;
    let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok().filter(|p| *p > 0)?;
    platform::process_exe(pid)
}

async fn restart() -> Result<()> {
    let out =
        quiet("systemctl", &["--user", "restart", UNIT]).await.ok_or_else(|| anyhow!("could not run systemctl"))?;
    if !out.status.success() {
        bail!("systemctl --user restart {UNIT} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

async fn await_version(port: u16, want: &str) -> Result<Duration> {
    let start = Instant::now();
    let mut down = 0;
    loop {
        if served_version(port).await.as_deref() == Some(want) {
            return Ok(start.elapsed());
        }
        down = if active().await { 0 } else { down + 1 };
        if down >= 3 {
            bail!("the service stopped before it reported {want} on 127.0.0.1:{port}");
        }
        if start.elapsed() > HEALTH_WAIT {
            bail!("the service did not report {want} on 127.0.0.1:{port} within {}s", HEALTH_WAIT.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn plain(p: &Path) -> String {
    let s = p.display().to_string();
    match (s.strip_prefix(r"\\?\UNC\"), s.strip_prefix(r"\\?\")) {
        (Some(unc), _) => format!(r"\\{unc}"),
        (None, Some(local)) => local.to_string(),
        (None, None) => s,
    }
}

fn manual_steps(m: Manager, exe: &Path, old: &Path) -> Option<(String, String)> {
    let (exe, old) = (plain(exe), plain(old));
    match m {
        Manager::Systemd => None,
        Manager::Launchd => {
            let kick = format!("launchctl kickstart -k gui/{}/{LABEL}", uid());
            Some((kick.clone(), format!("mv '{old}' '{exe}' && {kick}")))
        }
        Manager::Task => Some((
            format!("schtasks /End /TN {TASK}; schtasks /Run /TN {TASK}"),
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

fn lock_updates(state_dir: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(state_dir).with_context(|| format!("creating {}", state_dir.display()))?;
    let path = state_dir.join("update.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match f.try_lock() {
        Ok(()) => Ok(f),
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
            None => println!("t3cm {current} is up to date (latest release: {want})."),
        }
        return Ok(());
    }
    if want == current || (asked.is_none() && want < current) {
        println!("t3cm {current} is up to date (latest release: {want}).");
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
            "a t3cm {v} answers on 127.0.0.1:{}, but {} is t3cm {current}; restart the service so it runs {current}, then update",
            o.port,
            exe.display()
        );
    }
    let running = found == Some(Manager::Systemd) && active().await;
    if running && served.is_none() {
        bail!(
            "the service is running but nothing answers on 127.0.0.1:{}; pass its port with --port, or stop it (systemctl --user stop {UNIT}) and run `t3cm update` again",
            o.port
        );
    }
    if found.is_some() {
        let runs = if running {
            service_exe().await.ok_or_else(|| {
                anyhow!("could not tell which binary the running service runs; restart it (systemctl --user restart {UNIT}), then update")
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
    if !(o.restart && running) {
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
        return Ok(());
    }

    println!("Restarting the service; runs in flight finish first.");
    let cutover = match restart().await {
        Ok(()) => await_version(o.port, &want.to_string()).await,
        Err(e) => Err(e),
    };
    match cutover {
        Ok(took) => {
            swap.finish();
            println!(
                "t3cm updated: {current} -> {want}. The service answered on the new version in {:.1}s.",
                took.as_secs_f64()
            );
            Ok(())
        }
        Err(e) => {
            let back = async {
                swap.roll_back()?;
                restart().await?;
                await_version(o.port, &current.to_string()).await.map(|_| ())
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
    fn cargo_builds_are_recognised() {
        assert!(cargo_build(Path::new("/src/t3cm/target/release/t3cm")));
        assert!(cargo_build(Path::new("/src/t3cm/target/x86_64-unknown-linux-musl/debug/t3cm")));
        assert!(!cargo_build(Path::new("/home/u/.local/bin/t3cm")));
        assert!(!cargo_build(Path::new("/opt/release/t3cm")));
    }
}
