use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/skills.rs"));
}

pub const RELEASE: &str = env!("CARGO_PKG_VERSION");
pub const STAMP_KEY: &str = "t3-codemode-release";
pub const MANIFEST_SCHEMA: u32 = 1;
const RETIRED_FILES: &[&str] = &[
    "setup-t3-workflow/scripts/setup_workflow.py",
    "t3-overseer/t3-role.json",
    "t3-ticket/t3-role.json",
    "t3-worker/t3-role.json",
    "t3-workflow/defaults.json",
    "t3-workflow/scripts/slots.py",
    "t3-workflow/scripts/t3api.py",
    "t3-workflow/scripts/t3wf.py",
];

pub struct Bundle {
    pub release: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    pub fn embedded() -> Bundle {
        Bundle::new(RELEASE, bundled::FILES.iter().map(|(p, b)| (p.to_string(), b.to_vec())))
    }

    pub fn new(release: &str, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Bundle {
        Bundle { release: release.to_string(), files: files.into_iter().collect() }
    }

    pub fn skills(&self) -> BTreeSet<String> {
        self.files.keys().filter_map(|p| p.split('/').next()).map(str::to_string).collect()
    }

    pub fn raw(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn expected(&self) -> BTreeMap<String, Vec<u8>> {
        self.files
            .iter()
            .map(|(p, b)| {
                let body = if is_skill_md(p) { stamp(b, &self.release) } else { b.clone() };
                (p.clone(), body)
            })
            .collect()
    }
}

fn is_skill_md(path: &str) -> bool {
    matches!(path.split('/').collect::<Vec<_>>().as_slice(), [_, "SKILL.md"])
}

pub fn stamp(text: &[u8], release: &str) -> Vec<u8> {
    let Ok(s) = std::str::from_utf8(text) else { return text.to_vec() };
    let nl = if s.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> =
        s.split_inclusive('\n').filter(|l| !l.trim_start().starts_with(&format!("{STAMP_KEY}:"))).collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return text.to_vec();
    }
    let Some(end) = lines.iter().skip(1).position(|l| l.trim_end() == "---").map(|i| i + 1) else {
        return text.to_vec();
    };
    let entry = format!("  {STAMP_KEY}: \"{release}\"{nl}");
    let mut out = String::with_capacity(s.len() + entry.len() + 12);
    match lines[1..end].iter().position(|l| l.trim_end() == "metadata:").map(|i| i + 1) {
        Some(m) => {
            out.extend(lines[..=m].iter().copied());
            out.push_str(&entry);
            out.extend(lines[m + 1..].iter().copied());
        }
        None => {
            out.extend(lines[..end].iter().copied());
            out.push_str("metadata:");
            out.push_str(nl);
            out.push_str(&entry);
            out.extend(lines[end..].iter().copied());
        }
    }
    out.into_bytes()
}

pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub struct Paths {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub lock: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Paths> {
        let home = user_home().context("no home folder: set HOME (USERPROFILE on Windows)")?;
        let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
        Ok(Paths::under(&home, config.as_deref()))
    }

    pub fn under(home: &Path, config_home: Option<&Path>) -> Paths {
        let config = config_home.map(Path::to_path_buf).unwrap_or_else(|| home.join(".config"));
        let agents = home.join(".agents");
        Paths {
            dir: agents.join("skills"),
            manifest: config.join("t3-workflow").join("installed.json"),
            lock: agents.join(".skill-lock.json"),
        }
    }
}

#[cfg(windows)]
pub(crate) fn user_home() -> Option<PathBuf> {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty());
    var("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| {
            let mut p = var("HOMEDRIVE")?;
            p.push(var("HOMEPATH")?);
            Some(PathBuf::from(p))
        })
        .or_else(crate::platform::home_dir)
}

#[cfg(not(windows))]
pub(crate) fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(crate::platform::home_dir)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub release: String,
    pub dir: PathBuf,
    pub installed_at: String,
    pub skills: Vec<String>,
    pub files: BTreeMap<String, String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Option<Manifest>> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let m: Manifest =
                    serde_json::from_str(&s).with_context(|| format!("{} is not a skills manifest", path.display()))?;
                if m.schema > MANIFEST_SCHEMA {
                    bail!(
                        "{} has schema {}, newer than this t3cm reads ({MANIFEST_SCHEMA}); update t3cm",
                        path.display(),
                        m.schema
                    );
                }
                Ok(Some(m))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileState {
    Current,
    Stale,
    Edited,
    Missing,
    Obsolete,
    ObsoleteEdited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillState {
    Current,
    Stale,
    Edited,
    Missing,
    Unmanaged,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileStatus {
    pub path: String,
    pub state: FileState,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillStatus {
    pub name: String,
    pub state: SkillState,
    pub files: Vec<FileStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Foreign {
    pub tool: String,
    pub lock: Option<PathBuf>,
    pub commit: Option<String>,
    pub skills: Vec<String>,
    pub matches_release: bool,
    pub differing_files: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub release: String,
    pub installed: Option<String>,
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub current: bool,
    pub skills: Vec<SkillStatus>,
    pub foreign: Option<Foreign>,
    pub fix: Option<String>,
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn on_disk(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(dir.to_path_buf(), |p, part| p.join(part))
}

fn classify(disk: Option<&[u8]>, want: Option<&[u8]>, recorded: Option<&str>) -> FileState {
    let disk_hash = disk.map(hash);
    match (want, disk_hash) {
        (Some(_), None) => FileState::Missing,
        (Some(w), Some(d)) if d == hash(w) => FileState::Current,
        (Some(_), Some(d)) if recorded == Some(d.as_str()) => FileState::Stale,
        (Some(_), Some(_)) => FileState::Edited,
        (None, None) => FileState::Current,
        (None, Some(d)) if recorded == Some(d.as_str()) => FileState::Obsolete,
        (None, Some(_)) => FileState::ObsoleteEdited,
    }
}

type LockEntries = Vec<(String, Option<String>)>;

fn lock_entries(lock: &Path, skills: &BTreeSet<String>) -> Result<(Option<Value>, LockEntries)> {
    let Some(bytes) = read_file(lock)? else { return Ok((None, Vec::new())) };
    let v: Value = serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid JSON", lock.display()))?;
    let found = v
        .get("skills")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(name, _)| skills.contains(*name))
                .map(|(name, e)| (name.clone(), e.get("commit").and_then(Value::as_str).map(str::to_string)))
                .collect()
        })
        .unwrap_or_default();
    Ok((Some(v), found))
}

fn check_links(dir: &Path, skills: &BTreeSet<String>) -> Result<()> {
    for name in skills {
        let path = dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            let target = std::fs::read_link(&path).map(|t| t.display().to_string()).unwrap_or_default();
            bail!(
                "{} is a link to {target}; t3cm does not write through it. Remove the link, then run the command again",
                path.display()
            );
        }
    }
    Ok(())
}

pub fn status(paths: &Paths, bundle: &Bundle) -> Result<Status> {
    let skills = bundle.skills();
    let manifest = Manifest::load(&paths.manifest)?;
    let expected = bundle.expected();
    let (_, locked) = lock_entries(&paths.lock, &skills)?;
    let mut out = Vec::new();
    let mut differing: BTreeMap<&str, usize> = BTreeMap::new();
    let mut ours = true;
    let mut present = Vec::new();
    for name in &skills {
        let mut files = Vec::new();
        let mut exists = paths.dir.join(name).is_dir();
        for (rel, want) in expected.iter().filter(|(p, _)| p.split('/').next() == Some(name)) {
            let disk = read_file(&on_disk(&paths.dir, rel))?;
            exists |= disk.is_some();
            match &manifest {
                Some(m) => {
                    let state = classify(disk.as_deref(), Some(want), m.files.get(rel).map(String::as_str));
                    if state != FileState::Current {
                        files.push(FileStatus { path: rel.clone(), state });
                    }
                }
                None if exists => {
                    let raw = bundle.raw(rel).unwrap_or_default();
                    if disk.as_deref() != Some(raw) && disk.as_deref() != Some(want.as_slice()) {
                        *differing.entry(name.as_str()).or_default() += 1;
                    }
                    ours &= disk.is_none() || disk.as_deref() == Some(want.as_slice());
                }
                None => {}
            }
        }
        if let Some(m) = &manifest {
            for (rel, recorded) in m.files.iter().filter(|(p, _)| p.split('/').next() == Some(name)) {
                if expected.contains_key(rel) {
                    continue;
                }
                let disk = read_file(&on_disk(&paths.dir, rel))?;
                let state = classify(disk.as_deref(), None, Some(recorded));
                if state != FileState::Current {
                    files.push(FileStatus { path: rel.clone(), state });
                }
            }
        }
        if exists {
            present.push(name.clone());
        }
        let state = if manifest.is_none() {
            if exists { SkillState::Unmanaged } else { SkillState::Missing }
        } else if !exists {
            SkillState::Missing
        } else if files.iter().any(|f| matches!(f.state, FileState::Edited | FileState::ObsoleteEdited)) {
            SkillState::Edited
        } else if files.is_empty() {
            SkillState::Current
        } else {
            SkillState::Stale
        };
        out.push(SkillStatus { name: name.clone(), state, files });
    }
    if let Some(m) = &manifest {
        for name in m.skills.iter().filter(|n| !skills.contains(*n)) {
            let mut files = Vec::new();
            for (rel, recorded) in m.files.iter().filter(|(p, _)| p.split('/').next() == Some(name)) {
                let disk = read_file(&on_disk(&paths.dir, rel))?;
                let state = classify(disk.as_deref(), None, Some(recorded));
                if state != FileState::Current {
                    files.push(FileStatus { path: rel.clone(), state });
                }
            }
            if !files.is_empty() {
                let state = if files.iter().any(|f| f.state == FileState::ObsoleteEdited) {
                    SkillState::Edited
                } else {
                    SkillState::Stale
                };
                out.push(SkillStatus { name: name.clone(), state, files });
            }
        }
    }
    let foreign = if !locked.is_empty() {
        let commit = locked.iter().find_map(|(_, c)| c.clone());
        let count: usize = locked.iter().filter_map(|(n, _)| differing.get(n.as_str())).sum();
        Some(Foreign {
            tool: "npx skills".into(),
            lock: Some(paths.lock.clone()),
            commit,
            skills: locked.iter().map(|(n, _)| n.clone()).collect(),
            matches_release: manifest.is_none() && count == 0,
            differing_files: count,
        })
    } else if manifest.is_none() && !present.is_empty() && !ours {
        Some(Foreign {
            tool: "unknown".into(),
            lock: None,
            commit: None,
            skills: present.clone(),
            matches_release: differing.is_empty(),
            differing_files: differing.values().sum(),
        })
    } else {
        None
    };
    let all_current = out.iter().all(|s| s.state == SkillState::Current);
    let current = manifest.is_some() && all_current && foreign.is_none();
    let fix = if current {
        None
    } else if foreign.is_some() {
        Some("t3cm skills install --take-over".to_string())
    } else if manifest.is_none() {
        Some("t3cm skills install".to_string())
    } else if out.iter().any(|s| matches!(s.state, SkillState::Stale | SkillState::Missing)) {
        Some("t3cm skills update".to_string())
    } else {
        Some("t3cm skills update --force".to_string())
    };
    Ok(Status {
        release: bundle.release.clone(),
        installed: manifest.map(|m| m.release),
        dir: paths.dir.clone(),
        manifest: paths.manifest.clone(),
        current,
        skills: out,
        foreign,
        fix,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Install,
    Update,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub mode: Mode,
    pub force: bool,
    pub take_over: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub release: String,
    pub previous: Option<String>,
    pub dir: PathBuf,
    pub skills: Vec<String>,
    pub written: Vec<String>,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub released_from_lock: Vec<String>,
    pub also_locked: Vec<String>,
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a file path has a parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".t3cm-tmp");
    let tmp = PathBuf::from(tmp);
    let perms = std::fs::metadata(path).ok().map(|m| m.permissions());
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(p) = &perms {
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, std::os::unix::fs::PermissionsExt::mode(p));
    }
    let written =
        opts.open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, bytes)).and_then(|()| match perms {
            Some(p) => std::fs::set_permissions(&tmp, p),
            None => Ok(()),
        });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", tmp.display()));
    }
    std::fs::rename(&tmp, path)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("replacing {}", path.display()))
}

fn has_symlink_parent(root: &Path, rel: &str) -> bool {
    let mut cur = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        cur.push(part);
        if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

fn remove_with_empty_parents(dir: &Path, rel: &str) -> Result<()> {
    let path = on_disk(dir, rel);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
    }
    let mut parent = path.parent();
    while let Some(p) = parent.filter(|p| *p != dir) {
        if std::fs::remove_dir(p).is_err() {
            break;
        }
        parent = p.parent();
    }
    Ok(())
}

fn foreign_message(f: &Foreign, dir: &Path) -> String {
    let who = match (f.tool.as_str(), &f.commit) {
        ("npx skills", Some(c)) => format!("npx skills (commit {})", &c[..c.len().min(7)]),
        ("npx skills", None) => "npx skills".to_string(),
        _ => "hand or another tool".to_string(),
    };
    let skew = if f.matches_release {
        "its files match this release".to_string()
    } else {
        format!("{} of its files differ from this release", f.differing_files)
    };
    format!("{} in {} were installed by {who}, and {skew}", f.skills.join(", "), dir.display())
}

pub fn sync(paths: &Paths, bundle: &Bundle, opts: Options) -> Result<Report> {
    let skills = bundle.skills();
    check_links(&paths.dir, &skills)?;
    let manifest = Manifest::load(&paths.manifest)?;
    let (lock, locked) = lock_entries(&paths.lock, &skills)?;
    let expected = bundle.expected();
    let mut report = Report {
        release: bundle.release.clone(),
        previous: manifest.as_ref().map(|m| m.release.clone()),
        dir: paths.dir.clone(),
        skills: skills.iter().cloned().collect(),
        ..Report::default()
    };
    let mut files = BTreeMap::new();
    match &manifest {
        None => {
            let st = status(paths, bundle)?;
            if let Some(f) = &st.foreign {
                if opts.mode == Mode::Update || !opts.take_over {
                    bail!(
                        "{}. To let t3cm manage them, run `t3cm skills install --take-over`; it replaces those files with this release's",
                        foreign_message(f, &paths.dir)
                    );
                }
            } else if opts.mode == Mode::Update {
                bail!("t3cm has not installed the skills in {}; run `t3cm skills install`", paths.dir.display());
            }
            for (rel, want) in &expected {
                let path = on_disk(&paths.dir, rel);
                if read_file(&path)?.as_deref() != Some(want.as_slice()) {
                    write_atomic(&path, want)?;
                    report.written.push(rel.clone());
                }
                files.insert(rel.clone(), hash(want));
            }
            if opts.take_over {
                for rel in RETIRED_FILES {
                    if expected.contains_key(*rel) {
                        continue;
                    }
                    let path = on_disk(&paths.dir, rel);
                    if has_symlink_parent(&paths.dir, rel) {
                        if path.exists() || std::fs::symlink_metadata(&path).is_ok() {
                            report.kept.push((*rel).to_string());
                        }
                        continue;
                    }
                    if path.exists() || std::fs::symlink_metadata(&path).is_ok() {
                        remove_with_empty_parents(&paths.dir, rel)?;
                        report.removed.push((*rel).to_string());
                    }
                }
            }
        }
        Some(m) => {
            for (rel, want) in &expected {
                let path = on_disk(&paths.dir, rel);
                let disk = read_file(&path)?;
                let state = classify(disk.as_deref(), Some(want), m.files.get(rel).map(String::as_str));
                match state {
                    FileState::Current => {}
                    FileState::Edited if !opts.force => {
                        report.kept.push(rel.clone());
                        files.insert(rel.clone(), m.files.get(rel).cloned().unwrap_or_else(|| hash(want)));
                        continue;
                    }
                    _ => {
                        write_atomic(&path, want)?;
                        report.written.push(rel.clone());
                    }
                }
                files.insert(rel.clone(), hash(want));
            }
            for (rel, recorded) in m.files.iter().filter(|(p, _)| !expected.contains_key(*p)) {
                let disk = read_file(&on_disk(&paths.dir, rel))?;
                match classify(disk.as_deref(), None, Some(recorded)) {
                    FileState::Current => {}
                    FileState::ObsoleteEdited if !opts.force => {
                        report.kept.push(rel.clone());
                        files.insert(rel.clone(), recorded.clone());
                    }
                    _ => {
                        remove_with_empty_parents(&paths.dir, rel)?;
                        report.removed.push(rel.clone());
                    }
                }
            }
        }
    }
    let names: Vec<String> = locked.iter().map(|(n, _)| n.clone()).collect();
    if opts.take_over && !names.is_empty() {
        let mut lock = lock.expect("entries come from a lock file");
        if let Some(map) = lock.get_mut("skills").and_then(Value::as_object_mut) {
            for n in &names {
                map.shift_remove(n);
            }
        }
        let mut text = serde_json::to_string_pretty(&lock)?;
        text.push('\n');
        write_atomic(&paths.lock, text.as_bytes())?;
        report.released_from_lock = names;
    } else {
        report.also_locked = names;
    }
    let mut kept_skills: BTreeSet<String> = skills.clone();
    kept_skills.extend(files.keys().filter_map(|p| p.split('/').next()).map(str::to_string));
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA,
        release: bundle.release.clone(),
        dir: paths.dir.clone(),
        installed_at: jiff::Timestamp::now().to_string(),
        skills: kept_skills.into_iter().collect(),
        files,
    };
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    write_atomic(&paths.manifest, text.as_bytes())?;
    Ok(report)
}

pub fn describe_status(s: &Status) -> String {
    let mut out = format!("t3cm {}: skills in {}", s.release, s.dir.display());
    match &s.installed {
        Some(r) => out.push_str(&format!(", installed by t3cm at release {r}\n")),
        None => out.push_str(", not installed by t3cm\n"),
    }
    let width = s.skills.iter().map(|k| k.name.len()).max().unwrap_or(0);
    for k in &s.skills {
        let state = match k.state {
            SkillState::Current => format!("current ({})", s.release),
            SkillState::Stale => "out of date".to_string(),
            SkillState::Edited => "edited locally".to_string(),
            SkillState::Missing => "missing".to_string(),
            SkillState::Unmanaged => "not managed by t3cm".to_string(),
        };
        out.push_str(&format!("  {:width$}  {state}\n", k.name));
        for f in &k.files {
            let what = match f.state {
                FileState::Current => "current",
                FileState::Stale => "out of date",
                FileState::Edited => "edited locally",
                FileState::Missing => "missing",
                FileState::Obsolete => "no longer in this release",
                FileState::ObsoleteEdited => "edited locally, no longer in this release",
            };
            out.push_str(&format!("    {}: {what}\n", f.path));
        }
    }
    if let Some(f) = &s.foreign {
        out.push_str(&foreign_message(f, &s.dir));
        out.push_str(".\n");
        if s.installed.is_some() {
            out.push_str("npx skills update would overwrite what t3cm installed.\n");
        }
    }
    if let Some(fix) = &s.fix {
        let note = if fix.ends_with("--take-over") {
            " (lets t3cm manage them)"
        } else if fix.ends_with("--force") {
            " (discards the local edits)"
        } else {
            ""
        };
        out.push_str(&format!("fix: {fix}{note}\n"));
    }
    out
}

pub fn describe_report(r: &Report) -> String {
    let mut out = match &r.previous {
        Some(p) if *p != r.release => format!("updated the skills from {p} to {}", r.release),
        Some(_) => format!("synced the skills at {}", r.release),
        None => format!("installed the skills at {}", r.release),
    };
    out.push_str(&format!(
        " in {}: {}; {} files written, {} removed\n",
        r.dir.display(),
        r.skills.join(", "),
        r.written.len(),
        r.removed.len()
    ));
    if !r.kept.is_empty() {
        out.push_str("warning: kept these locally edited files; pass --force to overwrite them:\n");
        for k in &r.kept {
            out.push_str(&format!("  {k}\n"));
        }
    }
    if !r.released_from_lock.is_empty() {
        out.push_str(&format!("npx skills no longer manages: {}\n", r.released_from_lock.join(", ")));
    }
    if !r.also_locked.is_empty() {
        out.push_str(&format!(
            "warning: npx skills also tracks {}; `npx skills update` would overwrite them. Run `t3cm skills install --take-over` to stop that\n",
            r.also_locked.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_adds_a_metadata_block_before_the_closing_fence() {
        let src = b"---\nname: x\ndescription: y\n---\n\nbody\n";
        let out = String::from_utf8(stamp(src, "1.2.3")).unwrap();
        assert_eq!(out, "---\nname: x\ndescription: y\nmetadata:\n  t3-codemode-release: \"1.2.3\"\n---\n\nbody\n");
    }

    #[test]
    fn stamp_joins_an_existing_metadata_block_and_replaces_an_old_stamp() {
        let src = b"---\r\nname: x\r\nmetadata:\r\n  t3-codemode-release: \"0.1.0\"\r\n  other: v\r\n---\r\nbody\r\n";
        let out = String::from_utf8(stamp(src, "0.2.0")).unwrap();
        assert_eq!(
            out,
            "---\r\nname: x\r\nmetadata:\r\n  t3-codemode-release: \"0.2.0\"\r\n  other: v\r\n---\r\nbody\r\n"
        );
    }

    #[test]
    fn stamp_leaves_a_file_without_frontmatter_alone() {
        assert_eq!(stamp(b"# no frontmatter\n", "1.0.0"), b"# no frontmatter\n");
    }

    #[test]
    fn the_embedded_bundle_carries_every_skill_folder() {
        let b = Bundle::embedded();
        let skills = b.skills();
        for name in ["setup-t3-workflow", "t3-overseer", "t3-ticket", "t3-worker", "t3-workflow"] {
            assert!(skills.contains(name), "{name} missing from {skills:?}");
            assert!(b.raw(&format!("{name}/SKILL.md")).is_some());
        }
        assert!(!b.expected().keys().any(|p| p.ends_with(".py")), "embedded bundle must carry no Python files");
    }
}
