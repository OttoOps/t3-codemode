use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::t3::BoxFut;

const HOSTED_THREADS: &str = "https://app.t3.codes/threads/";
const APP_SCHEME: &str = "t3code";
const APP_IMAGE: &str = "T3 Code";
const STEP_TIMEOUT: Duration = Duration::from_secs(2);

pub trait Desktop: Send + Sync {
    fn open<'a>(&'a self, app_link: &'a str) -> BoxFut<'a, bool>;
}

pub fn app_link(link: &str) -> Option<String> {
    let rest = link.strip_prefix(HOSTED_THREADS)?;
    let (env, thread) = rest.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b));
    (ok(env) && ok(thread)).then(|| format!("{APP_SCHEME}://threads/{env}/{thread}"))
}

pub struct WindowsDesktop;

impl Desktop for WindowsDesktop {
    fn open<'a>(&'a self, app_link: &'a str) -> BoxFut<'a, bool> {
        Box::pin(async move {
            let (Some(tasklist), Some(rundll32)) = (windows_exe("tasklist.exe"), windows_exe("rundll32.exe")) else {
                return false;
            };
            let filter = format!("IMAGENAME eq {APP_IMAGE}*");
            let Some(listed) = run(&tasklist, &["/FI", &filter, "/NH", "/FO", "CSV"]).await else {
                return false;
            };
            if !listed.contains(&format!("\"{APP_IMAGE}")) {
                return false;
            }
            let launch = tokio::process::Command::new(&rundll32)
                .args(["url.dll,FileProtocolHandler", app_link])
                .current_dir("/")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = launch else {
                return false;
            };
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            true
        })
    }
}

async fn run(exe: &Path, args: &[&str]) -> Option<String> {
    let child = tokio::process::Command::new(exe)
        .args(args)
        .current_dir("/")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(STEP_TIMEOUT, child).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn windows_exe(name: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect::<Vec<_>>())
        .unwrap_or_default();
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let drives = mounts.lines().filter_map(|l| {
        let mut f = l.split_whitespace();
        let (_, dir, _, opts) = (f.next()?, f.next()?, f.next()?, f.next()?);
        opts.contains("drvfs").then(|| Path::new(dir).join("Windows").join("System32").join(name))
    });
    on_path.into_iter().chain(drives).find(|p| p.is_file())
}
