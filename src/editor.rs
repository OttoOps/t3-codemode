use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn command() -> Result<String> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
        .context("set EDITOR (or VISUAL) to the editor to open, such as EDITOR=vim")
}

pub fn scratch(beside: &Path, text: &str) -> Result<PathBuf> {
    let target = crate::steps::real(beside);
    let dir = target.parent().unwrap_or(Path::new(""));
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let stamp = jiff::Timestamp::now().as_nanosecond();
    let path = dir.join(format!(".config-edit.{}.{stamp}.json", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn open(editor: &str, path: &Path) -> Result<()> {
    let status = shell(editor, path).status().with_context(|| format!("starting {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

#[cfg(unix)]
fn shell(editor: &str, path: &Path) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!("{editor} \"$@\"")).arg("t3cm-editor").arg(path);
    cmd
}

#[cfg(windows)]
fn shell(editor: &str, path: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    cmd.raw_arg(format!("/S /C \"{editor} \"{}\"\"", path.display()));
    cmd
}
