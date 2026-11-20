use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::slots::{self, Caps, Enabled, Folders, Inputs, Resolved};
use crate::store::InstanceFolder;
use crate::wf::Wf;

pub const CONFIG_DIR_HEADER: &str = "x-t3-config-dir";
pub const CLAUDE_DIR_HEADER: &str = "x-t3-claude-config-dir";
pub const CODEX_HOME_HEADER: &str = "x-t3-codex-home";
const CODEX: &str = "codex";
const CLAUDE: &str = "claudeAgent";
const REREGISTER: &str = "run `t3cm skills update`, which re-registers Claude Code's t3cm server with the \
X-T3-Config-Dir header, then start a new session";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reported {
    pub own: Option<String>,
    pub claude: Option<String>,
    pub codex: Option<String>,
}

fn header_value(raw: Option<&str>) -> Option<String> {
    let v = raw?.trim();
    if v.contains("${") { None } else { Some(v.to_string()) }
}

impl Reported {
    pub fn from_header_map(h: &axum::http::HeaderMap) -> Reported {
        Reported::from_headers(|name| h.get(name).and_then(|v| std::str::from_utf8(v.as_bytes()).ok()))
    }

    pub fn from_headers<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Reported {
        Reported {
            own: header_value(get(CONFIG_DIR_HEADER)),
            claude: header_value(get(CLAUDE_DIR_HEADER)),
            codex: header_value(get(CODEX_HOME_HEADER)),
        }
    }

    pub fn from_env() -> Reported {
        let var = |k| Some(std::env::var_os(k).map(|v| v.to_string_lossy().into_owned()).unwrap_or_default());
        Reported { own: None, claude: var("CLAUDE_CONFIG_DIR"), codex: var("CODEX_HOME") }
    }

    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        for (name, v) in
            [(CONFIG_DIR_HEADER, &self.own), (CLAUDE_DIR_HEADER, &self.claude), (CODEX_HOME_HEADER, &self.codex)]
        {
            if let Some(v) = v {
                out.push((name, v.clone()));
            }
        }
        out
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Where {
    repo: Option<String>,
    instance: Option<String>,
    skills: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotArgs {
    name: String,
    repo: Option<String>,
    instance: Option<String>,
    skills: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenewalArgs {
    output: String,
    repo: Option<String>,
    instance: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(method: &str, args: Value) -> Result<T> {
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args).map_err(|e| anyhow!("wf.{method}: {e}"))
}

pub struct Ctx {
    pub folders: Folders,
    pub root: PathBuf,
    pub key: String,
    pub instance: Option<String>,
    pub driver: Option<String>,
    pub folder_source: &'static str,
    pub warnings: Vec<String>,
}

impl Ctx {
    fn view(&self) -> Value {
        json!({
            "instance": self.instance,
            "driver": self.driver,
            "claudeDir": self.folders.claude.to_string_lossy(),
            "codexHome": self.folders.codex.to_string_lossy(),
            "folderFrom": self.folder_source,
            "config": self.folders.config.to_string_lossy(),
        })
    }
}

fn expand(home: &Path, v: &str) -> PathBuf {
    match v.strip_prefix("~/").or_else(|| v.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None if v == "~" => home.to_path_buf(),
        None => PathBuf::from(v),
    }
}

fn reported_path(home: &Path, v: &str, default: PathBuf) -> PathBuf {
    if v.trim().is_empty() { default } else { expand(home, v.trim()) }
}

impl Wf {
    pub fn set_slot_folders(&self, f: Folders) {
        *self.slot_folders.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    fn base_folders(&self) -> Result<Folders> {
        if let Some(f) = self.slot_folders.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(f);
        }
        Folders::from_env()
    }

    async fn driver_of(&self, instance: &str) -> Option<String> {
        if let Some(d) = self.drivers.lock().unwrap_or_else(|e| e.into_inner()).get(instance) {
            return Some(d.clone());
        }
        let caps = self.t3.call("orchestrator_capabilities", json!({})).await.ok()?;
        let mut known = self.drivers.lock().unwrap_or_else(|e| e.into_inner());
        for p in caps["providers"].as_array().into_iter().flatten() {
            if let (Some(id), Some(d)) = (p["providerInstanceId"].as_str(), p["driverKind"].as_str()) {
                known.insert(id.to_string(), d.to_string());
            }
        }
        known.get(instance).cloned()
    }

    async fn project_root(&self, project: &str) -> Option<PathBuf> {
        let projects = self.project_list().await.ok()?;
        let p = projects.into_iter().find(|p| p["id"] == project)?;
        p["workspaceRoot"].as_str().filter(|r| !r.is_empty()).map(PathBuf::from)
    }

    pub async fn slot_ctx(
        &self,
        caller: Option<&str>,
        rep: &Reported,
        repo: Option<&str>,
        instance: Option<&str>,
    ) -> Result<Ctx> {
        let mut folders = self.base_folders()?;
        let home = folders.home.clone();
        let mut warnings = Vec::new();
        let mut thread = Value::Null;
        if let Some(me) = caller {
            match self.read_thread(me).await {
                Ok(th) => thread = th,
                Err(e) => warnings.push(format!("could not read thread {me} ({e}); resolving without its instance")),
            }
        }
        let own_instance = thread["providerInstanceId"].as_str().map(String::from);
        let reports_apply = instance.is_none() || instance == own_instance.as_deref();
        let instance = instance.map(String::from).or(own_instance);
        let driver = match &instance {
            Some(i) => self.driver_of(i).await,
            None => None,
        };
        let codex = driver.as_deref() == Some(CODEX);
        let (own_default, other_default) = if codex {
            (folders.default_codex(), folders.default_claude())
        } else {
            (folders.default_claude(), folders.default_codex())
        };
        let (own_rep, other_rep) = if codex { (&rep.codex, &rep.claude) } else { (&rep.claude, &rep.codex) };
        let mut folder_source = "reported";
        let reported = if reports_apply { rep.own.as_ref().or(own_rep.as_ref()) } else { None };
        let own = match reported {
            Some(v) => {
                let p = reported_path(&home, v, own_default);
                if let Some(i) = &instance {
                    let at = self.now();
                    let driver = driver.clone().unwrap_or_else(|| CLAUDE.to_string());
                    let rec =
                        InstanceFolder { instance: i.clone(), driver, folder: p.to_string_lossy().into_owned(), at };
                    if let Err(e) = self.store.instance_folder_set(&rec) {
                        tracing::warn!("recording the config folder for {i}: {e:#}");
                    }
                }
                p
            }
            None => {
                let recorded = match &instance {
                    Some(i) => self.store.instance_folder(i)?.map(|r| PathBuf::from(r.folder)),
                    None => None,
                };
                let (p, from) = match recorded {
                    Some(p) => {
                        folder_source = "recorded";
                        (p, "the folder recorded for this instance")
                    }
                    None => {
                        folder_source = "default";
                        (own_default, "the default folder")
                    }
                };
                if reports_apply && (caller.is_some() || instance.is_some()) {
                    warnings.push(format!(
                        "this call reported no config folder, so slots resolved with {from}, {}; to fix it, {REREGISTER}",
                        p.display()
                    ));
                }
                p
            }
        };
        let other = match other_rep.as_ref().filter(|_| reports_apply) {
            Some(v) => reported_path(&home, v, other_default),
            None => other_default,
        };
        if codex {
            folders.codex = own;
            folders.claude = other;
        } else {
            folders.claude = own;
            folders.codex = other;
        }
        let (root, key) = match repo {
            Some(r) => {
                let start = expand(&home, r);
                let root = tokio::task::spawn_blocking(move || slots::git_root(&start)).await?;
                let key = format!("repo:{}", root.display());
                (root, key)
            }
            None => {
                let project = thread["projectId"].as_str().map(String::from);
                let Some(project) = project else {
                    bail!("pass {{ repo }}: there is no caller thread whose project names the repository");
                };
                let Some(root) = self.project_root(&project).await else {
                    bail!("T3 has no workspace root for project {project}; pass {{ repo }}");
                };
                (root, format!("project:{project}"))
            }
        };
        Ok(Ctx { folders, root, key, instance, driver, folder_source, warnings })
    }

    pub async fn resolve_slots(&self, ctx: &Ctx, listed: &[String]) -> Result<Resolved> {
        let folders = ctx.folders.clone();
        let root = ctx.root.clone();
        let listed = listed.to_vec();
        let key = format!("{}|{}", ctx.key, ctx.instance.as_deref().unwrap_or("-"));
        let store = self.store.clone();
        let at = self.now();
        tokio::task::spawn_blocking(move || -> Result<Resolved> {
            let enabled = Enabled::load(&folders.claude, &root);
            let inputs = Inputs { folders: &folders, root: &root, listed: &listed, enabled: &enabled };
            if !listed.is_empty() {
                return Ok(Resolved::build(&inputs));
            }
            let fp = slots::fingerprint(&inputs);
            if let Some((stored, data)) = store.slot_resolution(&key)?
                && stored == fp
                && let Ok(r) = serde_json::from_value::<Resolved>(data)
            {
                return Ok(r);
            }
            let r = Resolved::build(&inputs);
            store.slot_resolution_set(&key, &fp, &serde_json::to_value(&r)?, &at)?;
            Ok(r)
        })
        .await?
    }

    async fn caps(&self) -> Caps {
        match self.t3.call("orchestrator_capabilities", json!({})).await {
            Ok(v) => Caps::Got(v),
            Err(e) => Caps::Failed(e.to_string()),
        }
    }

    pub async fn slot_call(&self, method: &str, caller: Option<&str>, rep: &Reported, args: Value) -> Result<Value> {
        match method {
            "slots" => {
                let a: Where = parse(method, args)?;
                let ctx = self.slot_ctx(caller, rep, a.repo.as_deref(), a.instance.as_deref()).await?;
                let r = self.resolve_slots(&ctx, &a.skills.unwrap_or_default()).await?;
                let mut v = r.all(&self.caps().await);
                v["problems"] = json!(r.problems());
                v["context"] = ctx.view();
                v["warnings"] = json!(ctx.warnings);
                Ok(v)
            }
            "slot" => {
                let a: SlotArgs = parse(method, args)?;
                if !slots::SLOTS.contains(&a.name.as_str()) {
                    bail!("wf.slot: no slot {:?}; slots are {}", a.name, slots::SLOTS.join(", "));
                }
                let ctx = self.slot_ctx(caller, rep, a.repo.as_deref(), a.instance.as_deref()).await?;
                let r = self.resolve_slots(&ctx, &a.skills.unwrap_or_default()).await?;
                let caps = if a.name == "models" { self.caps().await } else { Caps::Unchecked };
                let eff = r.all(&caps);
                let mut out = slot_view(&a.name, &eff, ctx.instance.as_deref(), &caps);
                out["line"] = json!(slots::slot_line(&eff, &a.name).join("\n"));
                let mut warnings = ctx.warnings.clone();
                warnings.extend(r.problems().into_iter().map(|p| format!("config problem (ignored): {p}")));
                out["warnings"] = json!(warnings);
                Ok(out)
            }
            "renewal" => {
                let a: RenewalArgs = parse(method, args)?;
                let ctx = self.slot_ctx(caller, rep, a.repo.as_deref(), a.instance.as_deref()).await?;
                let r = self.resolve_slots(&ctx, &[]).await?;
                let (hit, mut warnings) = slots::renewal(&r, &a.output);
                warnings.extend(ctx.warnings);
                if hit.is_null() {
                    for w in &warnings {
                        tracing::info!("wf.renewal: {w}");
                    }
                    return Ok(Value::Null);
                }
                let mut hit = hit;
                hit["warnings"] = json!(warnings);
                Ok(hit)
            }
            other => bail!("wf has no method {other}"),
        }
    }
}

fn with_how(e: &Value) -> Value {
    let mut e = e.clone();
    if e.is_object() {
        e["how"] = json!(slots::skill_how(&e));
    }
    e
}

const EFFORT_OPTIONS: &[&str] = &["effort", "thinking_level", "reasoningEffort"];

fn model_selection(m: &Value, instance: Option<&str>, caps: &Caps) -> Value {
    let spec = &m["spec"];
    if let Some(pool) = spec["pool"].as_str() {
        return json!({"pool": pool, "pick": format!("wf.pick({{ pool: {pool:?} }}) gives the modelSelection")});
    }
    let (Some(model), Caps::Got(caps)) = (spec["model"].as_str(), caps) else { return Value::Null };
    let effort = spec["effort"].as_str();
    let option_of = |entry: &Value| -> Option<Value> {
        let effort = effort?;
        entry["options"].as_array().into_iter().flatten().find_map(|o| {
            let fits = o["id"].as_str().is_some_and(|i| EFFORT_OPTIONS.contains(&i))
                && o["values"].as_array().is_some_and(|v| v.iter().any(|x| x == effort));
            fits.then(|| json!({"id": o["id"], "value": effort}))
        })
    };
    let offers = slots::model_offers(caps, model, spec["provider"].as_str());
    let fitting: Vec<&(String, Value)> = offers.iter().filter(|(_, e)| option_of(e).is_some()).collect();
    let pool: Vec<&(String, Value)> = if fitting.is_empty() { offers.iter().collect() } else { fitting };
    let pick = pool.iter().find(|(id, _)| Some(id.as_str()) == instance);
    let Some((id, entry)) = pick.or(pool.first()).copied() else { return Value::Null };
    let mut sel = json!({"instanceId": id, "model": model});
    if let Some(o) = option_of(entry) {
        sel["options"] = json!([o]);
    }
    sel
}

fn slot_view(name: &str, eff: &Value, instance: Option<&str>, caps: &Caps) -> Value {
    match name {
        "review" => {
            let mut v = eff["review"].clone();
            v["skill"] = with_how(&v["skill"]);
            v
        }
        "passes" => {
            let mut v = eff["passes"].clone();
            v["skills"] = json!(v["skills"].as_array().into_iter().flatten().map(with_how).collect::<Vec<_>>());
            v
        }
        "credentials" => {
            let mut v = eff["credentials"].clone();
            let renewals: Vec<Value> = v["renewals"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| json!({"match": x["match"], "skill": with_how(&x["skill"])}))
                .collect();
            v["renewals"] = json!(renewals);
            v
        }
        "delivery" => {
            let mut v = eff["delivery"].clone();
            if v["skill"].is_object() {
                v["skill"] = with_how(&v["skill"]);
            }
            v
        }
        _ => {
            let mut out = json!({"slot": "models"});
            for (role, m) in eff["models"].as_object().into_iter().flatten() {
                let mut m = m.clone();
                m["modelSelection"] = model_selection(&m, instance, caps);
                out[role] = m;
            }
            out
        }
    }
}
