use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::codehost::{
    Branch, Change, ChangeRef, CodeHost, Detail, Forge, Landed, Landing, Read, Rollup, State, Viewed,
};
use crate::github::errors;
use crate::gitlab::{GitLab, Rest, Target, project_id};
use crate::t3::BoxFut;

const BATCH: usize = 10;
const MR_FIELDS: &str = "id state draft diffHeadSha detailedMergeStatus mergedAt squashOnMerge mergeCommitSha \
headPipeline { status ref } mergeTrainCar { status index }";
const QUEUED: &[&str] = &["idle", "fresh", "stale", "merging"];
const PENDING: &[&str] =
    &["created", "waiting_for_resource", "preparing", "pending", "running", "scheduled", "waiting_for_callback"];
const CAR: &str = "query MergeTrainCar($p: ID!, $i: String!) { project(fullPath: $p) { mergeRequest(iid: $i) { \
mergeTrainCar { status index } } } }";
const PER_PAGE: usize = 100;
const PIPELINE_PAGES: usize = 5;
const DIFF_PAGES: usize = 1;

pub type CliFor = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct GitlabHost {
    lab: Arc<dyn GitLab>,
    cli: CliFor,
}

impl GitlabHost {
    pub fn new(lab: Arc<dyn GitLab>, cli: CliFor) -> GitlabHost {
        GitlabHost { lab, cli }
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_ascii_lowercase()
}

fn state(mr: &Value) -> std::result::Result<State, String> {
    match text(&mr["state"]).as_str() {
        "opened" | "locked" => Ok(State::Open),
        "merged" => Ok(State::Merged),
        "closed" => Ok(State::Closed),
        _ => Err(format!("GitLab returned an unknown merge request state {}", mr["state"])),
    }
}

pub fn merge_state(detailed: &str) -> &'static str {
    match detailed.to_ascii_lowercase().as_str() {
        "mergeable" => "CLEAN",
        "conflict" => "DIRTY",
        "need_rebase" => "BEHIND",
        "draft_status" => "DRAFT",
        "" | "checking" | "unchecked" | "preparing" | "approvals_syncing" => "UNKNOWN",
        _ => "BLOCKED",
    }
}

pub fn rollup(pipelines: &Value, head: &str, iid: u64) -> Rollup {
    let train = format!("refs/merge-requests/{iid}/train");
    let own = pipelines
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["sha"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(head)) && p["ref"].as_str() != Some(train.as_str())
        })
        .max_by_key(|p| p["id"].as_u64());
    let Some(p) = own else { return Rollup::default() };
    let name = format!("pipeline {}", p["id"]);
    let status = text(&p["status"]);
    let mut r = Rollup { complete: true, ..Rollup::default() };
    match status.as_str() {
        "success" => {}
        "skipped" | "manual" => r.settled.push(format!("{name} ({status})")),
        s if PENDING.contains(&s) => r.pending.push(name),
        s => r.failing.push(format!("{name} ({s})")),
    }
    r
}

pub fn all_listed(mr: &Value, listed: usize) -> bool {
    mr["changes_count"].as_str().and_then(|n| n.trim().parse::<usize>().ok()).is_some_and(|n| n <= listed)
}

pub fn changed_files(diffs: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in diffs {
        for k in ["new_path", "old_path"] {
            if let Some(p) = d[k].as_str().filter(|p| !p.is_empty())
                && !out.iter().any(|o| o == p)
            {
                out.push(p.to_string());
            }
        }
    }
    out
}

fn gid(mr: &Value) -> Value {
    mr["id"].as_u64().map_or(Value::Null, |n| json!(format!("gid://gitlab/MergeRequest/{n}")))
}

fn message(body: &Value) -> String {
    match &body["message"] {
        Value::String(m) => m.clone(),
        Value::Null => body["error"].as_str().map(String::from).unwrap_or_else(|| "no message".into()),
        other => other.to_string(),
    }
}

pub fn encode(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn graphql_shape(mr: &Value) -> Value {
    json!({
        "id": gid(mr), "squashOnMerge": mr["squash_on_merge"], "state": mr["state"], "draft": mr["draft"], "diffHeadSha": mr["sha"],
        "detailedMergeStatus": mr["detailed_merge_status"], "mergedAt": mr["merged_at"],
        "mergeCommitSha": mr["merge_commit_sha"], "mergeTrainCar": null,
    })
}

fn batch_query(chunk: &[&ChangeRef]) -> (String, Value) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    let mut vars = serde_json::Map::new();
    for (i, r) in chunk.iter().enumerate() {
        params.push(format!("$p{i}: ID!, $i{i}: String!"));
        fields.push(format!("m{i}: project(fullPath: $p{i}) {{ mergeRequest(iid: $i{i}) {{ {MR_FIELDS} }} }}"));
        vars.insert(format!("p{i}"), json!(r.repo));
        vars.insert(format!("i{i}"), json!(r.number.to_string()));
    }
    (format!("query Reconcile({}) {{ {} }}", params.join(", "), fields.join(" ")), Value::Object(vars))
}

impl GitlabHost {
    async fn rest(&self, to: Target<'_>, r: &ChangeRef, tail: &str) -> std::result::Result<Value, String> {
        let path = format!("projects/{}/merge_requests/{}{tail}", project_id(&r.repo), r.number);
        match self.lab.get(to, &path).await {
            Ok(rest) if rest.status == 200 => Ok(rest.body),
            Ok(rest) => Err(format!("GitLab answered HTTP {} for {path}", rest.status)),
            Err(e) => Err(format!("{e:#}")),
        }
    }

    async fn pages(
        &self,
        to: Target<'_>,
        r: &ChangeRef,
        tail: &str,
        max: usize,
    ) -> std::result::Result<(Vec<Value>, bool), String> {
        let list = |body: Value| match body {
            Value::Array(items) => Ok(items),
            _ => Err(format!("GitLab returned no list for {tail}")),
        };
        let mut out = Vec::new();
        for page in 1..=max {
            let items = list(self.rest(to, r, &format!("{tail}?per_page={PER_PAGE}&page={page}")).await?)?;
            let full = items.len() >= PER_PAGE;
            out.extend(items);
            if !full {
                return Ok((out, false));
            }
        }
        let beyond = max * PER_PAGE + 1;
        let more = !list(self.rest(to, r, &format!("{tail}?per_page=1&page={beyond}")).await?)?.is_empty();
        Ok((out, more))
    }

    async fn change(
        &self,
        to: Target<'_>,
        r: &ChangeRef,
        mr: &Value,
        full: Option<&Value>,
    ) -> std::result::Result<Change, String> {
        let state = state(mr)?;
        let head = text(&mr["diffHeadSha"]);
        let car = mr["mergeTrainCar"].as_object();
        let car_status = car.map(|c| text(&c["status"])).unwrap_or_default();
        let mut commit = mr["mergeCommitSha"].as_str().filter(|c| !c.is_empty()).map(String::from);
        let mut ended = mr["mergedAt"].as_str().map(String::from).filter(|_| state == State::Merged);
        if state == State::Closed || (state == State::Merged && commit.is_none()) {
            let full = match full {
                Some(f) => f.clone(),
                None => self.rest(to, r, "").await?,
            };
            if state == State::Merged {
                commit = ["merge_commit_sha", "squash_commit_sha"]
                    .iter()
                    .find_map(|k| full[*k].as_str().filter(|c| !c.is_empty()).map(String::from));
                ended = ended.or_else(|| full["merged_at"].as_str().map(String::from));
            } else {
                ended = ["closed_at", "updated_at"].iter().find_map(|k| full[*k].as_str().map(String::from));
            }
        }
        let checks = match state {
            State::Open => {
                let (pipelines, _) = self.pages(to, r, "/pipelines", PIPELINE_PAGES).await?;
                Some(rollup(&Value::Array(pipelines), &head, r.number))
            }
            _ => None,
        };
        Ok(Change {
            id: mr["id"].as_str().map(String::from),
            state,
            draft: mr["draft"] == true,
            head,
            merge_state: merge_state(mr["detailedMergeStatus"].as_str().unwrap_or_default()).into(),
            review_decision: None,
            checks,
            queue: car.is_some(),
            queued: state == State::Open && QUEUED.contains(&car_status.as_str()),
            position: car.and_then(|c| c["index"].as_u64()).map(|i| i + 1),
            commit: commit.map(|c| c.to_ascii_lowercase()),
            ended,
            removal: None,
            method: Some(if mr["squashOnMerge"] == true { "SQUASH" } else { "MERGE" }.into()),
            closing_issue: None,
        })
    }

    fn target(&self, r: &ChangeRef) -> std::result::Result<(String, String), String> {
        let Forge::Gitlab { host } = &r.forge else {
            return Err(format!("{}#{} is not on GitLab", r.repo, r.number));
        };
        let cli = (self.cli)(host).ok_or_else(|| format!("{host} is not listed under forges in config.json"))?;
        Ok((host.clone(), cli))
    }

    async fn trains(&self, to: Target<'_>, r: &ChangeRef) -> std::result::Result<bool, String> {
        let path = format!("projects/{}", project_id(&r.repo));
        match self.lab.get(to, &path).await {
            Ok(rest) if rest.status == 200 => Ok(rest.body["merge_trains_enabled"] == true),
            Ok(rest) => Err(format!("GitLab answered HTTP {} for {path}", rest.status)),
            Err(e) => Err(format!("{e:#}")),
        }
    }

    async fn car(&self, to: Target<'_>, r: &ChangeRef) -> Option<(String, Option<u64>)> {
        let vars = json!({"p": r.repo, "i": r.number.to_string()});
        let resp = self.lab.graphql(to, CAR, vars).await.ok()?;
        if resp["errors"].as_array().is_some_and(|e| !e.is_empty()) {
            return None;
        }
        let car = resp["data"]["project"]["mergeRequest"]["mergeTrainCar"].as_object()?;
        Some((text(&car["status"]), car["index"].as_u64().map(|i| i + 1)))
    }

    async fn cancel(&self, to: Target<'_>, r: &ChangeRef, why: &str) -> Landed {
        let base = format!("projects/{}/merge_requests/{}", project_id(&r.repo), r.number);
        let mut sent = self.lab.send(to, "POST", &format!("{base}/cancel_auto_merge"), json!({})).await;
        if matches!(&sent, Ok(rest) if rest.status == 404) {
            sent = self.lab.send(to, "POST", &format!("{base}/cancel_merge_when_pipeline_succeeds"), json!({})).await;
        }
        match sent {
            Ok(rest)
                if matches!(rest.status, 406 | 409)
                    || rest.body["http_status"] == 406
                    || ((200..300).contains(&rest.status) && rest.body["status"] != "error") =>
            {
                Landed::Waiting(format!("{why}; t3cm took back GitLab's auto-merge and holds the approval itself"))
            }
            Ok(rest) => Landed::Waiting(format!(
                "{why}, and GitLab will merge it on its own: t3cm could not cancel the auto-merge (HTTP {}: {})",
                rest.status,
                message(&rest.body)
            )),
            Err(e) => Landed::Waiting(format!(
                "{why}, and GitLab will merge it on its own: t3cm could not cancel the auto-merge ({e:#})"
            )),
        }
    }

    async fn landed(&self, to: Target<'_>, r: &ChangeRef, head: &str, rest: Rest) -> Landed {
        let msg = message(&rest.body);
        let short = head.get(..7).unwrap_or(head);
        match rest.status {
            200 | 201 if text(&rest.body["state"]) == "merged" => Landed::Merged {
                commit: ["merge_commit_sha", "squash_commit_sha"]
                    .iter()
                    .find_map(|k| rest.body[*k].as_str().filter(|c| !c.is_empty()))
                    .map(|c| c.to_ascii_lowercase()),
            },
            200 | 201 => match self.car(to, r).await {
                Some((status, position)) if QUEUED.contains(&status.as_str()) => Landed::Queued { position },
                _ if rest.status == 201 => Landed::Queued { position: None },
                _ => self.cancel(to, r, "GitLab set the merge request to merge once its pipeline passes").await,
            },
            202 => self.cancel(to, r, "GitLab scheduled it to join the merge train once its pipeline passes").await,
            405 => Landed::Waiting(format!("GitLab cannot merge it yet ({msg})")),
            409 => Landed::Refused(format!("head moved: GitLab no longer has the merge request at {short} ({msg})")),
            422 => Landed::Refused(format!("GitLab could not merge it ({msg})")),
            401 | 403 => Landed::Refused(format!(
                "t3cm's GitLab login has no permission to merge it (HTTP {}: {msg})",
                rest.status
            )),
            400 => Landed::Refused(format!("GitLab refused the merge, and may require the head sha (HTTP 400: {msg})")),
            404 => Landed::Refused(format!("GitLab shows no merge request !{} in {}", r.number, r.repo)),
            s => Landed::Refused(format!("GitLab answered HTTP {s}: {msg}")),
        }
    }

    async fn deleted(&self, to: Target<'_>, r: &ChangeRef) -> Branch {
        let mr = match self.rest(to, r, "").await {
            Ok(mr) => mr,
            Err(e) => return Branch::Unread(e),
        };
        let branch = mr["source_branch"].as_str().unwrap_or("of the merge request").to_string();
        if text(&mr["state"]) != "merged" {
            return Branch::NotMerged(branch);
        }
        if mr["source_project_id"] != mr["target_project_id"] {
            return Branch::Fork(branch);
        }
        let path = format!("projects/{}/repository/branches/{}", project_id(&r.repo), encode(&branch));
        let tip = match self.lab.get(to, &path).await {
            Ok(rest) if rest.status == 404 => return Branch::Gone(branch),
            Ok(rest) if rest.status == 200 => text(&rest.body["commit"]["id"]),
            Ok(rest) => return Branch::Unread(format!("GitLab answered HTTP {} for {path}", rest.status)),
            Err(e) => return Branch::Unread(format!("{e:#}")),
        };
        if tip != text(&mr["sha"]) {
            return Branch::TipMoved { branch, tip };
        }
        match self.lab.send(to, "DELETE", &path, Value::Null).await {
            Ok(rest) if (200..300).contains(&rest.status) => Branch::Deleted(branch),
            Ok(rest) if rest.status == 404 => Branch::Gone(branch),
            Ok(rest) => Branch::Failed { why: format!("HTTP {}: {}", rest.status, message(&rest.body)), branch },
            Err(e) => Branch::Failed { branch, why: format!("{e:#}") },
        }
    }

    async fn read_one(&self, to: Target<'_>, r: &ChangeRef, resp: &Value, i: usize) -> Read {
        let alias = format!("m{i}");
        let errs: Vec<&str> = resp["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["path"][0] == alias.as_str())
            .filter_map(|e| e["message"].as_str())
            .collect();
        if !errs.is_empty() {
            return Read::Failed(format!("GitLab returned an incomplete read: {}", errs.join("; ")));
        }
        let mr = &resp["data"][&alias]["mergeRequest"];
        if !mr.is_object() {
            return Read::NotFound(format!("GitLab shows no merge request !{} in {}", r.number, r.repo));
        }
        match self.change(to, r, mr, None).await {
            Ok(c) => Read::Found(Box::new(c)),
            Err(e) => Read::Failed(e),
        }
    }

    async fn read_host(&self, host: &str, refs: &[&ChangeRef]) -> Vec<Read> {
        let Some(cli) = (self.cli)(host) else {
            let why = format!("{host} is not listed under forges in config.json");
            return refs.iter().map(|_| Read::Failed(why.clone())).collect();
        };
        let to = Target { host, cli: &cli };
        let mut out = Vec::with_capacity(refs.len());
        for chunk in refs.chunks(BATCH) {
            let (query, vars) = batch_query(chunk);
            let resp = match self.lab.graphql(to, &query, vars).await {
                Ok(v) if v["data"].is_object() => v,
                Ok(v) => {
                    let why = errors(&v).unwrap_or_else(|| "no data".into());
                    out.extend(chunk.iter().map(|_| Read::Failed(format!("GitLab reconcile query failed: {why}"))));
                    continue;
                }
                Err(e) => {
                    out.extend(chunk.iter().map(|_| Read::Failed(format!("{e:#}"))));
                    continue;
                }
            };
            for (i, r) in chunk.iter().enumerate() {
                out.push(self.read_one(to, r, &resp, i).await);
            }
        }
        out
    }
}

impl CodeHost for GitlabHost {
    fn read<'a>(&'a self, refs: &'a [ChangeRef]) -> BoxFut<'a, Result<Vec<Read>>> {
        Box::pin(async move {
            let mut out: Vec<Option<Read>> = refs.iter().map(|_| None).collect();
            let mut hosts: Vec<&str> = Vec::new();
            for r in refs {
                if let Forge::Gitlab { host } = &r.forge
                    && !hosts.contains(&host.as_str())
                {
                    hosts.push(host);
                }
            }
            for host in hosts {
                let at: Vec<usize> = (0..refs.len())
                    .filter(|&i| matches!(&refs[i].forge, Forge::Gitlab { host: h } if h == host))
                    .collect();
                let mine: Vec<&ChangeRef> = at.iter().map(|&i| &refs[i]).collect();
                for (i, read) in at.into_iter().zip(self.read_host(host, &mine).await) {
                    out[i] = Some(read);
                }
            }
            Ok(out
                .into_iter()
                .zip(refs)
                .map(|(read, r)| {
                    read.unwrap_or_else(|| Read::Failed(format!("{}#{} is not on GitLab", r.repo, r.number)))
                })
                .collect())
        })
    }

    fn view<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Result<Viewed>> {
        Box::pin(async move {
            let Forge::Gitlab { host } = &r.forge else {
                return Ok(Viewed::Refused(format!("{}#{} is not on GitLab", r.repo, r.number)));
            };
            let cli = (self.cli)(host).ok_or_else(|| anyhow!("{host} is not listed under forges in config.json"))?;
            let to = Target { host, cli: &cli };
            let path = format!("projects/{}/merge_requests/{}", project_id(&r.repo), r.number);
            let mr = match self.lab.get(to, &path).await? {
                rest if rest.status == 404 => return Ok(Viewed::NotFound),
                rest if rest.status == 200 => rest.body,
                rest => return Err(anyhow!("GitLab answered HTTP {} for {path}", rest.status)),
            };
            let mut change = self.change(to, r, &graphql_shape(&mr), Some(&mr)).await.map_err(|e| anyhow!(e))?;
            if change.state == State::Open && self.trains(to, r).await.map_err(|e| anyhow!(e))? {
                change.queue = true;
                if let Some((status, position)) = self.car(to, r).await {
                    change.queued = QUEUED.contains(&status.as_str());
                    change.position = position.filter(|_| change.queued);
                }
            }
            let (diffs, more) = self.pages(to, r, "/diffs", DIFF_PAGES).await.map_err(|e| anyhow!(e))?;
            let after = self.rest(to, r, "").await.map_err(|e| anyhow!(e))?;
            if text(&after["sha"]) != change.head {
                return Err(anyhow!("the merge request head moved while t3cm read it"));
            }
            Ok(Viewed::Found(Box::new(Detail {
                change,
                body: mr["description"].as_str().unwrap_or_default().to_string(),
                files: changed_files(&diffs),
                more_files: more || !all_listed(&after, diffs.len()),
                change_requests: Vec::new(),
            })))
        })
    }

    fn merge<'a>(
        &'a self,
        r: &'a ChangeRef,
        _id: &'a str,
        head: &'a str,
        how: Landing<'a>,
    ) -> BoxFut<'a, Result<Landed>> {
        Box::pin(async move {
            let (host, cli) = match self.target(r) {
                Ok(t) => t,
                Err(e) => return Ok(Landed::Refused(e)),
            };
            let to = Target { host: &host, cli: &cli };
            let project = project_id(&r.repo);
            let trains = self.trains(to, r).await.map_err(|e| anyhow!(e))?;
            let (verb, path, mut body) = if trains {
                let path = format!("projects/{project}/merge_trains/merge_requests/{}", r.number);
                ("POST", path, json!({"sha": head, "auto_merge": true}))
            } else {
                let path = format!("projects/{project}/merge_requests/{}/merge", r.number);
                ("PUT", path, json!({"sha": head, "auto_merge": true, "should_remove_source_branch": true}))
            };
            let (Landing::Queue { method } | Landing::Merge { method }) = how;
            body["squash"] = json!(method.eq_ignore_ascii_case("SQUASH"));
            let rest = self.lab.send(to, verb, &path, body).await?;
            Ok(self.landed(to, r, head, rest).await)
        })
    }

    fn delete_branch<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Branch> {
        Box::pin(async move {
            let (host, cli) = match self.target(r) {
                Ok(t) => t,
                Err(e) => return Branch::Unread(e),
            };
            self.deleted(Target { host: &host, cli: &cli }, r).await
        })
    }

    fn url(&self, r: &ChangeRef) -> String {
        let host = match &r.forge {
            Forge::Gitlab { host } => host.as_str(),
            Forge::Github => "github.com",
        };
        format!("https://{host}/{}/-/merge_requests/{}", r.repo, r.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_detailed_merge_status_and_pipelines() {
        assert_eq!(merge_state("MERGEABLE"), "CLEAN");
        assert_eq!(merge_state("ci_still_running"), "BLOCKED");
        assert_eq!(merge_state("NEED_REBASE"), "BEHIND");
        assert_eq!(merge_state("conflict"), "DIRTY");
        assert_eq!(merge_state("checking"), "UNKNOWN");
        assert_eq!(merge_state(""), "UNKNOWN");
        let head = "a".repeat(40);
        let pipelines = json!([
            {"id": 9, "sha": "f".repeat(40), "ref": "refs/merge-requests/4/train", "status": "running"},
            {"id": 8, "sha": head, "ref": "refs/merge-requests/4/train", "status": "failed"},
            {"id": 7, "sha": head, "ref": "refs/merge-requests/4/head", "status": "success"},
        ]);
        assert_eq!(rollup(&pipelines, &head, 4), Rollup { complete: true, ..Rollup::default() });
        let branch = json!([{"id": 6, "sha": head, "ref": "release/train", "status": "success"}]);
        assert_eq!(rollup(&branch, &head, 4), Rollup { complete: true, ..Rollup::default() });
        let failed = json!([{"id": 3, "sha": head, "ref": "feat", "status": "canceled"}]);
        assert_eq!(rollup(&failed, &head, 4).failing, ["pipeline 3 (canceled)"]);
        assert!(!rollup(&json!([]), &head, 4).complete);
        let grouped = json!([
            {"id": 7, "sha": head, "ref": "refs/merge-requests/4/head", "status": "success"},
            {"id": 12, "sha": head, "ref": "feat", "status": "failed"},
        ]);
        assert_eq!(rollup(&grouped, &head, 4).failing, ["pipeline 12 (failed)"]);
    }

    #[test]
    fn trusts_the_diff_list_only_up_to_the_changes_count() {
        assert!(all_listed(&json!({"changes_count": "3"}), 3));
        assert!(!all_listed(&json!({"changes_count": "4"}), 3));
        assert!(!all_listed(&json!({"changes_count": "1000+"}), 100));
        assert!(!all_listed(&json!({"changes_count": null}), 3));
    }

    #[test]
    fn lists_new_and_renamed_paths_once() {
        let diffs = [
            json!({"old_path": "src/a.rs", "new_path": "src/a.rs"}),
            json!({"old_path": "src/old.rs", "new_path": "src/new.rs", "renamed_file": true}),
        ];
        assert_eq!(changed_files(&diffs), ["src/a.rs", "src/new.rs", "src/old.rs"]);
    }
}
