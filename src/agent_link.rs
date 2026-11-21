use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    Linked,
    Missing,
    Taken,
    NoAgent,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub skill: String,
    pub path: PathBuf,
    pub target: PathBuf,
    pub state: LinkState,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Linked {
    pub links: Vec<Link>,
    pub made: Vec<String>,
    pub manual: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Agent {
    pub name: &'static str,
    pub key: &'static str,
    pub folder: PathBuf,
    pub present: bool,
}

impl Agent {
    pub fn claude(claude: &Path) -> Agent {
        Agent { name: "Claude Code", key: "claudeLinks", folder: claude.join("skills"), present: claude.is_dir() }
    }

    pub fn antigravity(gemini: &Path) -> Agent {
        let config = gemini.join("config");
        let present =
            [config.clone(), gemini.join("antigravity"), gemini.join("antigravity-cli")].iter().any(|d| d.is_dir());
        Agent { name: "Antigravity", key: "antigravityLinks", folder: config.join("skills"), present }
    }

    pub fn all(env: &crate::steps::Env) -> Vec<Agent> {
        vec![Agent::claude(&env.claude), Agent::antigravity(&env.gemini)]
    }
}

fn same(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

pub fn check(agent: &Agent, skills_dir: &Path, names: &BTreeSet<String>) -> Vec<Link> {
    names
        .iter()
        .map(|name| {
            let path = agent.folder.join(name);
            let target = skills_dir.join(name);
            let state = if !agent.present {
                LinkState::NoAgent
            } else if std::fs::symlink_metadata(&path).is_err() {
                LinkState::Missing
            } else if same(&path, &target) {
                LinkState::Linked
            } else {
                LinkState::Taken
            };
            Link { skill: name.clone(), path, target, state }
        })
        .collect()
}

#[cfg(unix)]
fn make(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn make(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, path)
}

fn manual(l: &Link) -> String {
    let quote = if cfg!(windows) { "''" } else { r"'\''" };
    let [to, from] = [&l.path, &l.target].map(|p| crate::upgrade::plain(p).replace('\'', quote));
    if cfg!(windows) {
        format!("Copy-Item -Recurse -LiteralPath '{from}' -Destination '{to}'")
    } else {
        format!("ln -s '{from}' '{to}'")
    }
}

pub fn link(agent: &Agent, skills_dir: &Path, names: &BTreeSet<String>) -> Linked {
    let mut out = Linked::default();
    for l in check(agent, skills_dir, names) {
        if l.state != LinkState::Missing || !l.target.is_dir() {
            continue;
        }
        if out.error.is_none() {
            let made = std::fs::create_dir_all(&agent.folder).and_then(|()| make(&l.target, &l.path));
            match made {
                Ok(()) => {
                    out.made.push(l.skill.clone());
                    continue;
                }
                Err(e) => out.error = Some(format!("could not link {}: {e}", l.path.display())),
            }
        }
        out.manual.push(manual(&l));
    }
    out.links = check(agent, skills_dir, names);
    out
}

pub fn describe_check(agent: &Agent, links: &[Link]) -> String {
    let mut out = String::new();
    for l in links {
        match l.state {
            LinkState::Missing => out.push_str(&format!(
                "info: {} does not see {}; t3cm skills update links {} to it\n",
                agent.name,
                l.skill,
                l.path.display()
            )),
            LinkState::Taken => out.push_str(&format!(
                "info: {} is not a link to {}, so {} loads that copy instead; remove it and run t3cm skills update to link it\n",
                l.path.display(),
                l.target.display(),
                agent.name
            )),
            LinkState::Linked | LinkState::NoAgent => {}
        }
    }
    out
}

pub fn describe(agent: &Agent, l: &Linked) -> String {
    let mut out = String::new();
    if !l.made.is_empty() {
        out.push_str(&format!("linked {} into {} for {}\n", l.made.join(", "), agent.folder.display(), agent.name));
    }
    if let Some(e) = &l.error {
        out.push_str(&format!("warning: {e}. To give {} the skills, run:\n", agent.name));
        for m in &l.manual {
            out.push_str(&format!("  {m}\n"));
        }
    }
    let taken: Vec<Link> = l.links.iter().filter(|k| k.state == LinkState::Taken).cloned().collect();
    out.push_str(&describe_check(agent, &taken));
    out
}
