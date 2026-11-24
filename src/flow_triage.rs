use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{parse_github_issue_only, random_id, triage_dag_brief};
use crate::clock::local_clock;
use crate::flow::{BLOCKED_BY, PARENT, SOURCE_GITHUB, TriageSnapshot, label_of, node_key};
use crate::github;
use crate::issues::{
    WireCaches, WiringPlan, node_cache_key, normalize_priority, normalize_track, parse_github_ref, ref_slug,
};
use crate::t3::truncate_chars;
use crate::wf::Wf;

pub const TRIAGE_KIND: &str = "triageDag";
const MAX_TRIAGE_ISSUES: usize = 80;
const MAX_BODY_CHARS: usize = 600;
const MAX_TREE_FILES_PER_REPO: usize = 120;
const MAX_PROPOSED_CHANGES: usize = 80;
const MAX_REASON_CHARS: usize = 240;
const MAX_SUMMARY_CHARS: usize = 400;

#[derive(Debug)]
pub struct TriageRunning(pub String);

impl std::fmt::Display for TriageRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TriageRunning {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageSnapshotNode {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub title: String,
    pub parent: Option<String>,
    pub blocked_by: Vec<String>,
    pub track: String,
    pub track_by: String,
    pub priority: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageChange {
    pub id: String,
    pub kind: String,
    pub issue: String,
    pub issue_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub reason: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_parent: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TriageApplyArgs {
    #[serde(default)]
    pub change_ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub replace_parent: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TriageDismissArgs {
    #[serde(default)]
    pub change_ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageApplyResult {
    pub id: String,
    pub kind: String,
    pub issue: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageApplyResponse {
    pub ok: bool,
    pub applied: usize,
    pub already_set: usize,
    pub failed: usize,
    pub results: Vec<TriageApplyResult>,
    pub triage: Value,
    pub flow: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_error: Option<String>,
}

fn triage_seq(id: &str) -> Result<i64> {
    id.strip_prefix('t')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| anyhow!("no triage {id:?}; triage ids look like t3"))
}

pub(crate) fn format_triage_context(snap: &TriageSnapshot) -> (String, Vec<TriageSnapshotNode>) {
    let mut lines = Vec::new();
    let repos_str = if snap.covered_repos.is_empty() { "(none)".to_string() } else { snap.covered_repos.join(", ") };
    lines.push(format!("Repositories: {repos_str}"));

    if !snap.repo_trees.is_empty() {
        lines.push(String::new());
        lines.push("## Repository files".to_string());
        for (repo, files) in &snap.repo_trees {
            lines.push(format!("### {repo} ({} tracked files)", files.len()));
            for f in files.iter().take(MAX_TREE_FILES_PER_REPO) {
                lines.push(format!("- {f}"));
            }
            if files.len() > MAX_TREE_FILES_PER_REPO {
                lines.push(format!("- ... (+{} more files)", files.len() - MAX_TREE_FILES_PER_REPO));
            }
        }
    }

    let covered_set: HashSet<String> = snap.covered_repos.iter().map(|r| r.to_ascii_lowercase()).collect();
    let in_covered_id = |id: &str| -> bool {
        id.rsplit_once('#').is_some_and(|(repo, _)| covered_set.contains(&repo.to_ascii_lowercase()))
    };
    let open_nodes: Vec<_> = snap
        .graph
        .nodes
        .iter()
        .filter(|n| n.tracked && n.state != Some("closed") && covered_set.contains(&n.repo.to_ascii_lowercase()))
        .collect();
    let shown_count = open_nodes.len().min(MAX_TRIAGE_ISSUES);
    let mut snapshot_nodes = Vec::with_capacity(shown_count);

    lines.push(String::new());
    lines.push(format!("## Open issues ({shown_count} of {})", open_nodes.len()));
    for n in open_nodes.into_iter().take(MAX_TRIAGE_ISSUES) {
        let title = n.title.clone().unwrap_or_default();
        let key = node_key(&n.repo, n.number);
        snapshot_nodes.push(TriageSnapshotNode {
            id: n.id.clone(),
            repo: n.repo.clone(),
            number: n.number,
            label: n.label.clone(),
            title: title.clone(),
            parent: n.parent.clone(),
            blocked_by: n.blocked_by.clone(),
            track: n.track.clone(),
            track_by: n.track_by.to_string(),
            priority: n.priority.map(String::from),
        });
        let mut meta = Vec::new();
        meta.push(format!("track={} ({})", n.track, n.track_by));
        if let Some(p) = n.priority {
            meta.push(format!("priority={p}"));
        }
        if let Some(p) = &n.parent {
            meta.push(format!("parent={p}"));
        }
        if !n.blocked_by.is_empty() {
            meta.push(format!("blockedBy=[{}]", n.blocked_by.join(", ")));
        }
        if !n.labels.is_empty() {
            meta.push(format!("labels=[{}]", n.labels.join(", ")));
        }
        lines.push(format!("### {} - {}", n.id, title));
        lines.push(meta.join(" | "));
        if let Some(body) = snap.bodies.get(&key) {
            let clean = body.trim();
            if !clean.is_empty() {
                lines.push(truncate_chars(clean, MAX_BODY_CHARS));
            }
        }
    }

    let non_native: Vec<_> = snap
        .graph
        .edges
        .iter()
        .filter(|e| e.source != SOURCE_GITHUB && in_covered_id(&e.from) && in_covered_id(&e.to))
        .collect();
    if !non_native.is_empty() {
        lines.push(String::new());
        lines.push("## Existing inferred or advisory edges".to_string());
        for e in non_native.iter().take(60) {
            let mut extra = String::new();
            if !e.files.is_empty() {
                extra = format!(" files=[{}]", e.files.join(", "));
            } else if !e.modules.is_empty() {
                extra = format!(" modules=[{}]", e.modules.join(", "));
            }
            lines.push(format!("- {} --{} ({})--> {}{}", e.from, e.kind, e.source, e.to, extra));
        }
    }

    (lines.join("\n"), snapshot_nodes)
}

fn extract_json_object(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && v.is_object()
    {
        return Some(v);
    }

    // Check fenced code blocks first (```json ... ``` or ``` ... ```).
    let mut fenced_fallback: Option<Value> = None;
    let mut rest = trimmed;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let body_start = after.find('\n').map_or(after.len(), |i| i + 1);
        let body = &after[body_start..];
        let Some(end) = body.find("```") else { break };
        let block = body[..end].trim();
        if let Ok(v) = serde_json::from_str::<Value>(block)
            && v.is_object()
        {
            if v.get("changes").is_some() {
                return Some(v);
            }
            if fenced_fallback.is_none() {
                fenced_fallback = Some(v);
            }
        }
        rest = &body[end + 3..];
    }
    if fenced_fallback.is_some() {
        return fenced_fallback;
    }

    // Fallback: scan balanced top-level `{ ... }` segments in prose.
    let bytes = trimmed.as_bytes();
    let mut best_with_changes: Option<Value> = None;
    let mut first_obj: Option<Value> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut in_str = false;
        let mut escape = false;
        let mut j = i;
        while j < bytes.len() {
            let b = bytes[j];
            if in_str {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_str = false;
                }
            } else {
                match b {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            let slice = &trimmed[i..=j];
                            if let Ok(v) = serde_json::from_str::<Value>(slice)
                                && v.is_object()
                            {
                                if v.get("changes").is_some() {
                                    best_with_changes = Some(v);
                                } else if first_obj.is_none() {
                                    first_obj = Some(v);
                                }
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        i = if depth == 0 { j + 1 } else { i + 1 };
    }
    best_with_changes.or(first_obj)
}

pub fn parse_triage_proposal(
    raw: &str,
    snapshot: &[TriageSnapshotNode],
    default_repo: Option<&str>,
) -> Result<(Option<String>, Vec<TriageChange>)> {
    let obj =
        extract_json_object(raw).ok_or_else(|| anyhow!("triage reply did not contain a valid JSON proposal object"))?;
    let summary =
        obj["summary"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| truncate_chars(s, MAX_SUMMARY_CHARS));
    let raw_changes = match obj.get("changes") {
        Some(Value::Array(arr)) => arr.as_slice(),
        None if summary.is_some() => &[],
        _ => bail!("triage JSON proposal is missing a \"changes\" array"),
    };

    let by_key: HashMap<String, &TriageSnapshotNode> =
        snapshot.iter().map(|n| (node_key(&n.repo, n.number), n)).collect();
    let single_repo: Option<&str> = default_repo.or_else(|| {
        let mut repos: HashSet<String> = HashSet::new();
        let mut first = None;
        for n in snapshot {
            if repos.insert(n.repo.to_ascii_lowercase()) {
                first = Some(n.repo.as_str());
            }
        }
        (repos.len() == 1).then_some(first).flatten()
    });

    let mut out = Vec::new();
    let mut seen_blocked: HashSet<(String, String)> = HashSet::new();
    let mut seen_parent: HashSet<String> = HashSet::new();
    let mut seen_track: HashSet<String> = HashSet::new();
    let mut seen_priority: HashSet<String> = HashSet::new();

    for entry in raw_changes {
        if out.len() >= MAX_PROPOSED_CHANGES {
            break;
        }
        let Some(raw_kind) = entry["kind"].as_str().map(str::trim) else { continue };
        let kind = match raw_kind {
            "blockedBy" | "blocked_by" => BLOCKED_BY,
            "parent" => PARENT,
            "track" => "track",
            "priority" => "priority",
            _ => continue,
        };
        let Some(raw_issue) = entry["issue"].as_str().map(str::trim).filter(|s| !s.is_empty()) else { continue };
        let parsed_issue = match single_repo {
            Some(def) => parse_github_ref(raw_issue, def, "triage", "issue").ok(),
            None => parse_github_issue_only(raw_issue).and_then(|i| {
                let (o, r, n) = i.github_parts()?;
                Some((format!("{o}/{r}"), n))
            }),
        };
        let Some((irepo, inum)) = parsed_issue else { continue };
        let ikey = node_key(&irepo, inum);
        let Some(node) = by_key.get(&ikey).copied() else { continue };

        let reason = entry["reason"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| truncate_chars(s, MAX_REASON_CHARS))
            .unwrap_or_default();

        match kind {
            BLOCKED_BY => {
                let Some(raw_target) = entry["target"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let Ok((trepo, tnum)) = parse_github_ref(raw_target, &node.repo, "triage", "target") else {
                    continue;
                };
                let tkey = node_key(&trepo, tnum);
                if tkey == ikey {
                    continue;
                }
                if node.blocked_by.iter().any(|b| b.eq_ignore_ascii_case(&format!("{trepo}#{tnum}"))) {
                    continue;
                }
                if !seen_blocked.insert((ikey.clone(), tkey.clone())) {
                    continue;
                }
                let tnode = by_key.get(&tkey).copied();
                let target_id = tnode.map_or_else(|| ref_slug(&trepo, tnum), |n| n.id.clone());
                let target_label = tnode.map_or_else(|| label_of(&trepo, tnum), |n| n.label.clone());
                let target_title = tnode.map(|n| n.title.clone()).filter(|s| !s.is_empty());
                let cid = format!("c{}", out.len() + 1);
                out.push(TriageChange {
                    id: cid,
                    kind: BLOCKED_BY.into(),
                    issue: node.id.clone(),
                    issue_label: node.label.clone(),
                    issue_title: (!node.title.is_empty()).then(|| node.title.clone()),
                    target: Some(target_id),
                    target_label: Some(target_label),
                    target_title,
                    value: None,
                    current: None,
                    reason,
                    status: "pending".into(),
                    error: None,
                    permission: false,
                    conflict: false,
                    existing_parent: None,
                });
            }
            PARENT => {
                let Some(raw_target) = entry["target"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let Ok((trepo, tnum)) = parse_github_ref(raw_target, &node.repo, "triage", "target") else {
                    continue;
                };
                let tkey = node_key(&trepo, tnum);
                if tkey == ikey {
                    continue;
                }
                let target_slug = ref_slug(&trepo, tnum);
                if node.parent.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&target_slug)) {
                    continue;
                }
                if !seen_parent.insert(ikey.clone()) {
                    continue;
                }
                let tnode = by_key.get(&tkey).copied();
                let target_id = tnode.map_or(target_slug, |n| n.id.clone());
                let target_label = tnode.map_or_else(|| label_of(&trepo, tnum), |n| n.label.clone());
                let target_title = tnode.map(|n| n.title.clone()).filter(|s| !s.is_empty());
                let cid = format!("c{}", out.len() + 1);
                out.push(TriageChange {
                    id: cid,
                    kind: PARENT.into(),
                    issue: node.id.clone(),
                    issue_label: node.label.clone(),
                    issue_title: (!node.title.is_empty()).then(|| node.title.clone()),
                    target: Some(target_id),
                    target_label: Some(target_label),
                    target_title,
                    value: None,
                    current: node.parent.clone(),
                    reason,
                    status: "pending".into(),
                    error: None,
                    permission: false,
                    conflict: false,
                    existing_parent: None,
                });
            }
            "track" => {
                let Some(raw_val) = entry["value"].as_str().map(String::from) else { continue };
                let Ok(Some(slug)) = normalize_track(Some(raw_val), "triage") else { continue };
                if node.track_by == "label" && node.track.eq_ignore_ascii_case(&slug) {
                    continue;
                }
                if !seen_track.insert(ikey.clone()) {
                    continue;
                }
                let current = (node.track_by == "label" && !node.track.is_empty()).then(|| node.track.clone());
                let cid = format!("c{}", out.len() + 1);
                out.push(TriageChange {
                    id: cid,
                    kind: "track".into(),
                    issue: node.id.clone(),
                    issue_label: node.label.clone(),
                    issue_title: (!node.title.is_empty()).then(|| node.title.clone()),
                    target: None,
                    target_label: None,
                    target_title: None,
                    value: Some(slug),
                    current,
                    reason,
                    status: "pending".into(),
                    error: None,
                    permission: false,
                    conflict: false,
                    existing_parent: None,
                });
            }
            "priority" => {
                let Some(raw_val) = entry["value"].as_str().map(String::from) else { continue };
                let Ok(Some(prio)) = normalize_priority(Some(raw_val), "triage") else { continue };
                if node.priority.as_deref() == Some(prio) {
                    continue;
                }
                if !seen_priority.insert(ikey.clone()) {
                    continue;
                }
                let cid = format!("c{}", out.len() + 1);
                out.push(TriageChange {
                    id: cid,
                    kind: "priority".into(),
                    issue: node.id.clone(),
                    issue_label: node.label.clone(),
                    issue_title: (!node.title.is_empty()).then(|| node.title.clone()),
                    target: None,
                    target_label: None,
                    target_title: None,
                    value: Some(prio.to_string()),
                    current: node.priority.clone(),
                    reason,
                    status: "pending".into(),
                    error: None,
                    permission: false,
                    conflict: false,
                    existing_parent: None,
                });
            }
            _ => {}
        }
    }

    Ok((summary, out))
}

pub(crate) fn finalize_triage_answer(data: &mut Value, raw_text: &str, now: &str) {
    let snapshot: Vec<TriageSnapshotNode> = serde_json::from_value(data["snapshot"].clone()).unwrap_or_default();
    let default_repo = data["repos"].as_array().filter(|a| a.len() == 1).and_then(|a| a[0].as_str());
    match parse_triage_proposal(raw_text, &snapshot, default_repo) {
        Ok((summary, changes)) => {
            data["status"] = json!("ready");
            data["summary"] = json!(summary);
            data["changes"] = serde_json::to_value(&changes).unwrap_or_else(|_| json!([]));
            data["error"] = Value::Null;
            data["finishedAt"] = json!(now);
        }
        Err(e) => {
            data["status"] = json!("failed");
            data["error"] = json!(e.to_string());
            data["changes"] = json!([]);
            data["finishedAt"] = json!(now);
        }
    }
}

pub fn triage_view(seq: i64, data: &Value) -> Value {
    let mut v = data.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("snapshot");
    }
    let changes: Vec<TriageChange> = serde_json::from_value(data["changes"].clone()).unwrap_or_default();
    let pending = changes.iter().filter(|c| c.status == "pending").count();
    let applied = changes.iter().filter(|c| matches!(c.status.as_str(), "applied" | "alreadySet")).count();
    let dismissed = changes.iter().filter(|c| c.status == "dismissed").count();
    let failed = changes.iter().filter(|c| c.status == "error").count();
    v["id"] = json!(format!("t{seq}"));
    v["seq"] = json!(seq);
    v["atLocal"] = json!(data["at"].as_str().map(local_clock));
    v["finishedAtLocal"] = json!(data["finishedAt"].as_str().map(local_clock));
    v["counts"] = json!({
        "total": changes.len(),
        "pending": pending,
        "applied": applied,
        "dismissed": dismissed,
        "failed": failed,
    });
    v
}

fn can_reach(adj: &HashMap<String, HashSet<String>>, start: &str, goal: &str) -> bool {
    let mut stack = vec![start];
    let mut visited = HashSet::new();
    while let Some(cur) = stack.pop() {
        if cur == goal {
            return true;
        }
        if !visited.insert(cur) {
            continue;
        }
        if let Some(nexts) = adj.get(cur) {
            for nxt in nexts {
                if !visited.contains(nxt.as_str()) {
                    stack.push(nxt.as_str());
                }
            }
        }
    }
    false
}

impl Wf {
    fn canon_issue_key(&self, raw: &str) -> Option<String> {
        let parsed = parse_github_issue_only(raw.trim())?;
        let (owner, repo, number) = parsed.github_parts()?;
        let canon = self.canonical_github_repo(&format!("{owner}/{repo}"));
        Some(node_key(&canon, number))
    }

    pub fn flow_triage_view(&self) -> Result<Value> {
        match self.store.triage_latest_active()? {
            Some((seq, _, data)) => Ok(triage_view(seq, &data)),
            None => Ok(Value::Null),
        }
    }

    pub async fn start_flow_triage(&self) -> Result<Value> {
        let _guard = self.triage_lock.lock().await;
        if let Some((seq, _)) = self.store.triage_open()? {
            return Err(TriageRunning(format!("triage t{seq} is already running")).into());
        }
        let snap = self.flow_triage_snapshot().await?;
        let (context, snapshot_nodes) = format_triage_context(&snap);
        if snapshot_nodes.is_empty() {
            bail!("no open issues found to triage");
        }
        let resolved = match self.resolve_ask_model().await {
            Ok(r) => r,
            Err(e) => {
                let text = format!("Cannot start DAG triage: {e:#}");
                let card = self.launch_failed("triage", &text)?;
                bail!("{text} (recorded as board card {})", card["card"]);
            }
        };
        let projects = self.ask_projects().await;
        let brief = triage_dag_brief(&context, projects.as_deref());
        // Launch the scratch thread idle first so its threadId is durably registered as a read-only
        // triage thread in store.db BEFORE the prompt turn starts.
        let input = json!({
            "scratch": true,
            "title": format!("Triage DAG ({} issues)", snapshot_nodes.len()),
            "modelSelection": resolved.selection,
            "runtimeMode": "approval-required",
            "interactionMode": "plan",
            "clientRequestId": format!("t3cm-triage-{}", random_id()),
        });
        let r = self.t3.call("t3_thread_launch", input).await?;
        let thread = r["threadId"].as_str().ok_or_else(|| anyhow!("t3_thread_launch returned no threadId"))?;

        let now = self.now();
        let mut data = json!({
            "kind": TRIAGE_KIND,
            "thread": thread,
            "link": r["link"],
            "status": "running",
            "at": now,
            "finishedAt": Value::Null,
            "settledAt": Value::Null,
            "archived": false,
            "threadStopped": false,
            "summary": Value::Null,
            "error": Value::Null,
            "issueCount": snapshot_nodes.len(),
            "repos": snap.covered_repos,
            "snapshot": snapshot_nodes,
            "changes": [],
            "instanceId": resolved.instance_id,
            "model": resolved.model,
            "effort": resolved.effort,
            "modelSource": resolved.source,
        });
        if let Some(p) = &resolved.picked {
            data["pool"] = json!(p.pool);
        }
        let seq = self.store.ask_insert(thread, &data)?;
        if let Err(e) = self
            .t3
            .call(
                "t3_thread_send",
                json!({
                    "threadId": thread,
                    "message": brief,
                    "mode": "auto",
                    "clientRequestId": format!("t3cm-triage-send-{}", random_id()),
                }),
            )
            .await
        {
            let fail_at = self.now();
            data["status"] = json!("failed");
            data["error"] = json!(format!("sending triage brief failed: {e:#}"));
            data["finishedAt"] = json!(&fail_at);
            data["settledAt"] = json!(&fail_at);
            let (stopped, archived) = self.stop_triage_thread(thread, "triage send failed").await;
            data["threadStopped"] = json!(stopped);
            data["archived"] = json!(archived);
            let _ = self.store.ask_update(seq, false, &data);
            self.changed();
            return Err(e.into());
        }
        // Only settle previous closed triage reviews once the replacement prompt has been accepted.
        for (old_seq, mut old_data) in self.store.triage_unsettled_closed()? {
            if old_seq == seq {
                continue;
            }
            old_data["settledAt"] = json!(&now);
            let _ = self.store.ask_update(old_seq, false, &old_data);
        }
        let mut out = triage_view(seq, &data);
        if let Some(p) = &resolved.picked {
            self.note_pool_launch(&mut out, p, "Triage DAG", thread);
        }
        self.changed();
        Ok(out)
    }

    pub async fn apply_flow_triage(&self, id: &str, args: TriageApplyArgs) -> Result<TriageApplyResponse> {
        if !args.all && args.change_ids.is_empty() {
            bail!("apply_flow_triage: pass changeIds or set all: true");
        }
        let seq = triage_seq(id)?;
        let _guard = self.triage_lock.lock().await;
        let (open, mut data) = self.store.triage_get(seq)?.ok_or_else(|| anyhow!("no triage {id:?}"))?;
        if open {
            bail!("triage {id} is still running; wait for proposals before applying");
        }
        if !data["settledAt"].is_null() {
            bail!("triage {id} has already been dismissed or superseded");
        }
        if data["status"] != "ready" {
            let st = data["status"].as_str().unwrap_or("unknown");
            bail!("triage {id} has no reviewable proposals (status: {st})");
        }
        let mut changes: Vec<TriageChange> = serde_json::from_value(data["changes"].clone()).unwrap_or_default();
        let selected: Vec<usize> = if args.all {
            changes.iter().enumerate().filter_map(|(i, c)| (c.status == "pending").then_some(i)).collect()
        } else {
            let mut idxs = Vec::new();
            let mut seen = HashSet::new();
            for cid in &args.change_ids {
                let Some(pos) = changes.iter().position(|c| c.id == *cid) else {
                    bail!("triage {id}: unknown change {cid:?}");
                };
                if seen.insert(pos) {
                    idxs.push(pos);
                }
            }
            idxs
        };

        let needs_graph_cycle_check = selected.iter().any(|&i| {
            matches!(changes[i].status.as_str(), "pending" | "error")
                && matches!(changes[i].kind.as_str(), BLOCKED_BY | PARENT)
        });
        let mut blocked_adj: HashMap<String, HashSet<String>> = HashMap::new();
        let mut parent_adj: HashMap<String, HashSet<String>> = HashMap::new();
        if needs_graph_cycle_check {
            let live_flow = self.flow_view(true).await?;
            if let Some(g) = live_flow.graph {
                for e in g.edges {
                    if let (Some(fk), Some(tk)) = (self.canon_issue_key(&e.from), self.canon_issue_key(&e.to)) {
                        if e.kind == BLOCKED_BY {
                            blocked_adj.entry(fk).or_default().insert(tk);
                        } else if e.kind == PARENT {
                            parent_adj.entry(fk).or_default().insert(tk);
                        }
                    }
                }
            }
        }

        let mut caches = WireCaches::new();
        let mut results = Vec::with_capacity(selected.len());
        let mut applied = 0usize;
        let mut already_set = 0usize;
        let mut failed = 0usize;
        let mut any_mutated = false;

        for idx in selected {
            let cur_status = changes[idx].status.clone();
            if matches!(cur_status.as_str(), "applied" | "alreadySet" | "dismissed") {
                if cur_status == "alreadySet" {
                    already_set += 1;
                }
                let c = &changes[idx];
                results.push(TriageApplyResult {
                    id: c.id.clone(),
                    kind: c.kind.clone(),
                    issue: c.issue.clone(),
                    target: c.target.clone(),
                    value: c.value.clone(),
                    status: c.status.clone(),
                    error: c.error.clone(),
                    permission: c.permission,
                    conflict: c.conflict,
                    existing_parent: c.existing_parent.clone(),
                });
                continue;
            }

            let c = changes[idx].clone();
            let Some(parsed_issue) = parse_github_issue_only(&c.issue) else {
                changes[idx].status = "error".into();
                changes[idx].error = Some(format!("invalid issue reference {:?}", c.issue));
                failed += 1;
                results.push(TriageApplyResult {
                    id: c.id,
                    kind: c.kind,
                    issue: c.issue,
                    target: c.target,
                    value: c.value,
                    status: "error".into(),
                    error: changes[idx].error.clone(),
                    permission: false,
                    conflict: false,
                    existing_parent: None,
                });
                continue;
            };
            let (owner, repo, number) = parsed_issue.github_parts().expect("GitHub issue has parts");
            let from_repo = format!("{owner}/{repo}");
            let is_label_edit = matches!(c.kind.as_str(), "track" | "priority");
            let _issue_guard =
                if is_label_edit { Some(self.issue_edit_lock(&from_repo, number).lock_owned().await) } else { None };
            if is_label_edit {
                caches.nodes.remove(&node_cache_key(&from_repo, number));
                let canon_repo = self.canonical_github_repo(&from_repo);
                caches.nodes.remove(&node_cache_key(&canon_repo, number));
            }
            let from_node = match self.resolve_issue_cached(&mut caches, &from_repo, number).await {
                Ok(n) => n,
                Err(e) => {
                    changes[idx].status = "error".into();
                    changes[idx].error = Some(e.message.clone());
                    changes[idx].permission = e.permission;
                    failed += 1;
                    results.push(TriageApplyResult {
                        id: c.id,
                        kind: c.kind,
                        issue: c.issue,
                        target: c.target,
                        value: c.value,
                        status: "error".into(),
                        error: Some(e.message),
                        permission: e.permission,
                        conflict: false,
                        existing_parent: None,
                    });
                    continue;
                }
            };

            let mut new_status = "applied".to_string();
            let mut err_msg: Option<String> = None;
            let mut perm = false;
            let mut conflict = false;
            let mut existing_parent: Option<String> = None;

            match c.kind.as_str() {
                BLOCKED_BY => {
                    let Some(target) = c.target.as_deref() else { continue };
                    let Ok((t_repo, t_num)) = parse_github_ref(target, &from_node.repo, "triage", "target") else {
                        new_status = "error".into();
                        err_msg = Some(format!("invalid blocker reference {target:?}"));
                        changes[idx].status = new_status.clone();
                        changes[idx].error = err_msg.clone();
                        failed += 1;
                        results.push(TriageApplyResult {
                            id: c.id,
                            kind: c.kind,
                            issue: c.issue,
                            target: c.target,
                            value: c.value,
                            status: new_status,
                            error: err_msg,
                            permission: false,
                            conflict: false,
                            existing_parent: None,
                        });
                        continue;
                    };
                    let from_key = node_key(&self.canonical_github_repo(&from_node.repo), from_node.number);
                    let to_key = match self.resolve_issue_cached(&mut caches, &t_repo, t_num).await {
                        Ok(to_node) => {
                            let tk = node_key(&self.canonical_github_repo(&to_node.repo), to_node.number);
                            for b in &to_node.blocked_by {
                                blocked_adj
                                    .entry(tk.clone())
                                    .or_default()
                                    .insert(node_key(&self.canonical_github_repo(&b.repo), b.number));
                            }
                            tk
                        }
                        Err(_) => node_key(&self.canonical_github_repo(&t_repo), t_num),
                    };
                    if from_key == to_key || can_reach(&blocked_adj, &to_key, &from_key) {
                        new_status = "error".into();
                        err_msg = Some(format!(
                            "would create a blockedBy cycle between {} and {}",
                            ref_slug(&from_node.repo, from_node.number),
                            ref_slug(&t_repo, t_num)
                        ));
                        failed += 1;
                    } else {
                        let plan = WiringPlan {
                            blocked_by: vec![(t_repo.clone(), t_num)],
                            parent: None,
                            replace_parent: false,
                            labels: Vec::new(),
                        };
                        let report = self.wire_issue_node(&from_node, &plan, &mut caches).await;
                        let lr = report.blocked_by.into_iter().next().expect("blockedBy report");
                        if lr.status == "error" {
                            new_status = "error".into();
                            err_msg = lr.error;
                            perm = lr.permission;
                            failed += 1;
                        } else if lr.status == "alreadyLinked" {
                            new_status = "alreadySet".into();
                            already_set += 1;
                            blocked_adj.entry(from_key).or_default().insert(to_key);
                            let _ = self.clear_dismissed_edge(
                                BLOCKED_BY,
                                &from_node.repo,
                                from_node.number,
                                &t_repo,
                                t_num,
                            );
                        } else {
                            applied += 1;
                            any_mutated = true;
                            blocked_adj.entry(from_key).or_default().insert(to_key);
                            let _ = self.clear_dismissed_edge(
                                BLOCKED_BY,
                                &from_node.repo,
                                from_node.number,
                                &t_repo,
                                t_num,
                            );
                        }
                    }
                }
                PARENT => {
                    let Some(target) = c.target.as_deref() else { continue };
                    let Ok((t_repo, t_num)) = parse_github_ref(target, &from_node.repo, "triage", "target") else {
                        new_status = "error".into();
                        err_msg = Some(format!("invalid parent reference {target:?}"));
                        changes[idx].status = new_status.clone();
                        changes[idx].error = err_msg.clone();
                        failed += 1;
                        results.push(TriageApplyResult {
                            id: c.id,
                            kind: c.kind,
                            issue: c.issue,
                            target: c.target,
                            value: c.value,
                            status: new_status,
                            error: err_msg,
                            permission: false,
                            conflict: false,
                            existing_parent: None,
                        });
                        continue;
                    };
                    let from_key = node_key(&self.canonical_github_repo(&from_node.repo), from_node.number);
                    let to_key = match self.resolve_issue_cached(&mut caches, &t_repo, t_num).await {
                        Ok(to_node) => {
                            let tk = node_key(&self.canonical_github_repo(&to_node.repo), to_node.number);
                            if let Some(p) = &to_node.parent {
                                parent_adj
                                    .entry(tk.clone())
                                    .or_default()
                                    .insert(node_key(&self.canonical_github_repo(&p.repo), p.number));
                            }
                            tk
                        }
                        Err(_) => node_key(&self.canonical_github_repo(&t_repo), t_num),
                    };
                    if from_key == to_key || can_reach(&parent_adj, &to_key, &from_key) {
                        new_status = "error".into();
                        err_msg = Some(format!(
                            "would create a parent cycle between {} and {}",
                            ref_slug(&from_node.repo, from_node.number),
                            ref_slug(&t_repo, t_num)
                        ));
                        failed += 1;
                    } else {
                        let plan = WiringPlan {
                            blocked_by: Vec::new(),
                            parent: Some((t_repo.clone(), t_num)),
                            replace_parent: args.replace_parent,
                            labels: Vec::new(),
                        };
                        let report = self.wire_issue_node(&from_node, &plan, &mut caches).await;
                        let lr = report.parent.expect("parent report");
                        if lr.status == "error" {
                            new_status = "error".into();
                            let msg = lr.error.unwrap_or_else(|| "GitHub returned an error".to_string());
                            let existing = from_node.parent.as_ref().map(|p| ref_slug(&p.repo, p.number));
                            conflict =
                                !lr.permission && (github::is_parent_conflict_message(&msg) || existing.is_some());
                            existing_parent = conflict.then_some(existing).flatten();
                            perm = lr.permission;
                            err_msg = Some(msg);
                            failed += 1;
                        } else if lr.status == "alreadyLinked" {
                            new_status = "alreadySet".into();
                            already_set += 1;
                            if args.replace_parent {
                                parent_adj.insert(from_key, HashSet::from([to_key]));
                            } else {
                                parent_adj.entry(from_key).or_default().insert(to_key);
                            }
                            let _ =
                                self.clear_dismissed_edge(PARENT, &from_node.repo, from_node.number, &t_repo, t_num);
                        } else {
                            applied += 1;
                            any_mutated = true;
                            if args.replace_parent {
                                parent_adj.insert(from_key, HashSet::from([to_key]));
                            } else {
                                parent_adj.entry(from_key).or_default().insert(to_key);
                            }
                            let _ =
                                self.clear_dismissed_edge(PARENT, &from_node.repo, from_node.number, &t_repo, t_num);
                        }
                    }
                }
                "track" => {
                    let Some(val) = c.value.as_deref() else { continue };
                    let (lbls, errs) =
                        self.apply_track_and_priority(&from_node, Some(Some(val)), None, &mut caches).await;
                    if lbls.iter().any(|l| matches!(l.status, "added" | "createdAndAdded" | "removed")) {
                        any_mutated = true;
                    }
                    if let Some(first_err) = errs.first() {
                        new_status = "error".into();
                        err_msg = Some(first_err.message.clone());
                        perm = errs.iter().any(|e| e.permission);
                        failed += 1;
                    } else if !lbls.is_empty() && lbls.iter().all(|l| l.status == "alreadyApplied") {
                        new_status = "alreadySet".into();
                        already_set += 1;
                    } else {
                        applied += 1;
                    }
                }
                "priority" => {
                    let Some(val) = c.value.as_deref() else { continue };
                    let (lbls, errs) =
                        self.apply_track_and_priority(&from_node, None, Some(Some(val)), &mut caches).await;
                    if lbls.iter().any(|l| matches!(l.status, "added" | "createdAndAdded" | "removed")) {
                        any_mutated = true;
                    }
                    if let Some(first_err) = errs.first() {
                        new_status = "error".into();
                        err_msg = Some(first_err.message.clone());
                        perm = errs.iter().any(|e| e.permission);
                        failed += 1;
                    } else if !lbls.is_empty() && lbls.iter().all(|l| l.status == "alreadyApplied") {
                        new_status = "alreadySet".into();
                        already_set += 1;
                    } else {
                        applied += 1;
                    }
                }
                _ => {
                    new_status = "error".into();
                    err_msg = Some(format!("unsupported triage change kind {:?}", c.kind));
                    failed += 1;
                }
            }

            // Invalidate mutated issue from node cache so a later change on the same issue reads fresh state.
            caches.nodes.remove(&node_cache_key(&from_node.repo, from_node.number));
            caches.nodes.remove(&node_cache_key(&from_repo, number));

            changes[idx].status = new_status.clone();
            changes[idx].error = err_msg.clone();
            changes[idx].permission = perm;
            changes[idx].conflict = conflict;
            changes[idx].existing_parent = existing_parent.clone();

            results.push(TriageApplyResult {
                id: c.id,
                kind: c.kind,
                issue: c.issue,
                target: c.target,
                value: c.value,
                status: new_status,
                error: err_msg,
                permission: perm,
                conflict,
                existing_parent,
            });
        }

        data["changes"] = serde_json::to_value(&changes)?;
        self.store.ask_update(seq, false, &data)?;
        self.changed();

        let (flow, refresh_error) = if any_mutated || applied > 0 || already_set > 0 {
            match self.flow_view(true).await {
                Ok(fresh) => (Some(serde_json::to_value(fresh)?), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            }
        } else {
            (None, None)
        };

        Ok(TriageApplyResponse {
            ok: failed == 0,
            applied,
            already_set,
            failed,
            results,
            triage: triage_view(seq, &data),
            flow,
            refresh_error,
        })
    }

    pub async fn dismiss_flow_triage(&self, id: &str, args: TriageDismissArgs) -> Result<Value> {
        let seq = triage_seq(id)?;
        let _guard = self.triage_lock.lock().await;
        let (open, mut data) = self.store.triage_get(seq)?.ok_or_else(|| anyhow!("no triage {id:?}"))?;

        if !args.all && !args.change_ids.is_empty() {
            if open {
                bail!("triage {id} is still running; dismiss the whole run to cancel it");
            }
            let mut changes: Vec<TriageChange> = serde_json::from_value(data["changes"].clone()).unwrap_or_default();
            for cid in &args.change_ids {
                let Some(slot) = changes.iter_mut().find(|c| c.id == *cid) else {
                    bail!("triage {id}: unknown change {cid:?}");
                };
                if matches!(slot.status.as_str(), "pending" | "error") {
                    slot.status = "dismissed".into();
                    slot.error = None;
                    slot.permission = false;
                    slot.conflict = false;
                    slot.existing_parent = None;
                }
            }
            data["changes"] = serde_json::to_value(&changes)?;
            self.store.ask_update(seq, false, &data)?;
            self.changed();
            return Ok(json!({"ok": true, "dismissed": false, "id": id, "triage": triage_view(seq, &data)}));
        }

        let now = self.now();
        if open {
            data["status"] = json!("dismissed");
            data["finishedAt"] = json!(&now);
            if let Some(thread) = data["thread"].as_str()
                && !thread.is_empty()
            {
                let (stopped, archived) = self.stop_triage_thread(thread, "triage dismissed").await;
                data["threadStopped"] = json!(stopped);
                data["archived"] = json!(archived);
            } else {
                data["threadStopped"] = json!(true);
            }
        } else {
            let mut changes: Vec<TriageChange> = serde_json::from_value(data["changes"].clone()).unwrap_or_default();
            for c in &mut changes {
                if c.status == "pending" {
                    c.status = "dismissed".into();
                }
            }
            data["changes"] = serde_json::to_value(&changes)?;
        }
        data["settledAt"] = json!(&now);
        self.store.ask_update(seq, false, &data)?;
        self.changed();
        Ok(json!({"ok": true, "dismissed": true, "id": id, "triage": Value::Null}))
    }

    pub(crate) async fn stop_triage_thread(&self, thread: &str, reason: &str) -> (bool, bool) {
        let stopped = match self
            .t3
            .call(
                "t3_thread_interrupt",
                json!({
                    "threadId": thread,
                    "stop": true,
                    "reason": reason,
                    "clientRequestId": format!("t3cm-triage-stop-{}", random_id()),
                }),
            )
            .await
        {
            Ok(_) => match self.t3.call("t3_thread_read", json!({"threadId": thread, "limit": 1})).await {
                Ok(r) => crate::wf::thread_is_stopped(&r["thread"]),
                Err(e) if e.is_not_found() => true,
                Err(_) => false,
            },
            Err(e) if e.is_not_found() => true,
            Err(_) => false,
        };
        let archived =
            self.t3.call("t3_thread_organize", json!({"threadId": thread, "action": "archive"})).await.is_ok();
        (stopped, archived)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> Vec<TriageSnapshotNode> {
        vec![
            TriageSnapshotNode {
                id: "Acme/widgets#1".into(),
                repo: "Acme/widgets".into(),
                number: 1,
                label: "widgets#1".into(),
                title: "Auth epic".into(),
                parent: None,
                blocked_by: Vec::new(),
                track: "auth".into(),
                track_by: "label".into(),
                priority: Some("high".into()),
            },
            TriageSnapshotNode {
                id: "Acme/widgets#2".into(),
                repo: "Acme/widgets".into(),
                number: 2,
                label: "widgets#2".into(),
                title: "Add login API".into(),
                parent: Some("Acme/widgets#1".into()),
                blocked_by: Vec::new(),
                track: "auth".into(),
                track_by: "epic".into(),
                priority: None,
            },
            TriageSnapshotNode {
                id: "Acme/widgets#3".into(),
                repo: "Acme/widgets".into(),
                number: 3,
                label: "widgets#3".into(),
                title: "Add login UI".into(),
                parent: None,
                blocked_by: vec!["Acme/widgets#2".into()],
                track: "Acme/widgets: Independent".into(),
                track_by: "independent".into(),
                priority: Some("low".into()),
            },
        ]
    }

    #[test]
    fn parses_fenced_and_prose_json_and_filters_noops() {
        let snap = sample_snapshot();
        let raw = r##"Here is the analysis:
```json
{
  "summary": "Link UI to epic and align track/priority.",
  "changes": [
    {"kind": "blockedBy", "issue": "#3", "target": "#2", "reason": "already blocked - should be filtered"},
    {"kind": "blockedBy", "issue": "#3", "target": "#3", "reason": "self loop - filtered"},
    {"kind": "blockedBy", "issue": "Acme/widgets#2", "target": "#1", "reason": "needs epic setup"},
    {"kind": "blockedBy", "issue": "Acme/widgets#2", "target": "Acme/widgets#1", "reason": "duplicate - filtered"},
    {"kind": "parent", "issue": "#3", "target": "#1", "reason": "belongs to auth epic"},
    {"kind": "parent", "issue": "#2", "target": "#1", "reason": "already has parent #1 - filtered"},
    {"kind": "track", "issue": "#1", "value": "auth", "reason": "already explicit auth - filtered"},
    {"kind": "track", "issue": "#3", "value": "track: auth", "reason": "put on auth track"},
    {"kind": "priority", "issue": "#2", "value": "high", "reason": "unblocks #3"},
    {"kind": "priority", "issue": "#3", "value": "low", "reason": "already low - filtered"},
    {"kind": "priority", "issue": "#3", "value": "urgent", "reason": "invalid priority - filtered"},
    {"kind": "track", "issue": "#999", "value": "auth", "reason": "unknown issue - filtered"}
  ]
}
```"##;
        let (summary, changes) = parse_triage_proposal(raw, &snap, Some("Acme/widgets")).unwrap();
        assert_eq!(summary.as_deref(), Some("Link UI to epic and align track/priority."));
        assert_eq!(changes.len(), 4);

        assert_eq!(changes[0].id, "c1");
        assert_eq!(changes[0].kind, "blockedBy");
        assert_eq!(changes[0].issue, "Acme/widgets#2");
        assert_eq!(changes[0].target.as_deref(), Some("Acme/widgets#1"));

        assert_eq!(changes[1].id, "c2");
        assert_eq!(changes[1].kind, "parent");
        assert_eq!(changes[1].issue, "Acme/widgets#3");
        assert_eq!(changes[1].target.as_deref(), Some("Acme/widgets#1"));
        assert_eq!(changes[1].current, None);

        assert_eq!(changes[2].id, "c3");
        assert_eq!(changes[2].kind, "track");
        assert_eq!(changes[2].issue, "Acme/widgets#3");
        assert_eq!(changes[2].value.as_deref(), Some("auth"));
        assert_eq!(changes[2].current, None);

        assert_eq!(changes[3].id, "c4");
        assert_eq!(changes[3].kind, "priority");
        assert_eq!(changes[3].issue, "Acme/widgets#2");
        assert_eq!(changes[3].value.as_deref(), Some("high"));
        assert_eq!(changes[3].current, None);
    }

    #[test]
    fn extracts_json_wrapped_in_plain_prose_without_fences() {
        let snap = sample_snapshot();
        let raw = "Done inspecting.\n{\"summary\": \"All clear.\", \"changes\": []}\nThanks!";
        let (summary, changes) = parse_triage_proposal(raw, &snap, Some("Acme/widgets")).unwrap();
        assert_eq!(summary.as_deref(), Some("All clear."));
        assert!(changes.is_empty());
    }

    #[test]
    fn rejects_non_json_reply() {
        let snap = sample_snapshot();
        let err = parse_triage_proposal("Sorry, I could not analyze the repo.", &snap, Some("Acme/widgets"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("valid JSON"), "{err}");
    }
}
