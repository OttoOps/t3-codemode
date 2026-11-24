use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::board::parse_github_issue_only;
use crate::codehost::{
    Branch, Change, ChangeRef, ChangeRequest, CodeHost, Detail, Forge, Landed, Landing, Read, Rollup, State, Viewed,
};
use crate::github::{GitHub, errors};
use crate::t3::BoxFut;

const BATCH: usize = 100;
const SETTLED: &[&str] = &["SUCCESS", "CANCELLED", "SKIPPED", "NEUTRAL"];

const CHECKS: &str = "fragment Checks on PullRequest { commits(last: 1) { nodes { commit { oid \
statusCheckRollup { contexts(first: 100) { pageInfo { hasNextPage } nodes { __typename \
... on CheckRun { name status conclusion } ... on StatusContext { context state } } } } } } } }";
const PR_FIELDS: &str = "fragment F on PullRequest { id state isDraft headRefOid mergeStateStatus reviewDecision \
isMergeQueueEnabled isInMergeQueue mergeQueueEntry { position } mergeCommit { oid } mergedAt closedAt \
timelineItems(last: 1, itemTypes: [REMOVED_FROM_MERGE_QUEUE_EVENT]) { nodes { ... on RemovedFromMergeQueueEvent { \
reason createdAt } } } closingIssuesReferences(first: 10) { nodes { number repository { nameWithOwner } } } ...Checks }";
const VIEW_PR: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { \
viewerDefaultMergeMethod pullRequest(number: $number) { id state isDraft headRefOid mergeStateStatus reviewDecision body \
isMergeQueueEnabled isInMergeQueue mergeQueueEntry { position } mergeCommit { oid } \
latestOpinionatedReviews(first: 100) { nodes { state author { login } commit { oid } } } \
files(first: 100) { pageInfo { hasNextPage } nodes { path } } ...Checks } } }";
const ENQUEUE_PR: &str = "mutation($id: ID!, $head: GitObjectID!) { enqueuePullRequest(input: { \
pullRequestId: $id, expectedHeadOid: $head }) { mergeQueueEntry { position } } }";
const MERGE_PR: &str = "mutation($id: ID!, $head: GitObjectID!, $method: PullRequestMergeMethod!) { \
mergePullRequest(input: { pullRequestId: $id, expectedHeadOid: $head, mergeMethod: $method }) { \
pullRequest { mergeCommit { oid } } } }";
const PR_BRANCH: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { \
pullRequest(number: $number) { state headRefName headRefOid isCrossRepository headRef { id target { oid } } } } }";
const DELETE_REF: &str = "mutation($ref: ID!) { deleteRef(input: { refId: $ref }) { clientMutationId } }";

pub struct GithubHost(Arc<dyn GitHub>);

impl GithubHost {
    pub fn new(github: Arc<dyn GitHub>) -> GithubHost {
        GithubHost(github)
    }
}

fn not_github(r: &ChangeRef) -> String {
    format!("{}#{} is not on GitHub ({:?})", r.repo, r.number, r.forge)
}

fn owner_name(r: &ChangeRef) -> (&str, &str) {
    r.repo.split_once('/').unwrap_or_default()
}

fn rollup(pr: &Value) -> Option<Rollup> {
    let commit = &pr["commits"]["nodes"][0]["commit"];
    let contexts = &commit["statusCheckRollup"]["contexts"];
    let head = pr["headRefOid"].as_str().unwrap_or_default();
    let at_head = commit["oid"].as_str().is_some_and(|o| !o.is_empty() && o.eq_ignore_ascii_case(head));
    let nodes = contexts["nodes"].as_array().filter(|_| at_head)?;
    let mut r = Rollup::default();
    for c in nodes {
        let text = |k: &str| c[k].as_str().unwrap_or("unknown");
        if c["__typename"] == "StatusContext" {
            match text("state") {
                "SUCCESS" => {}
                "PENDING" | "EXPECTED" => r.pending.push(text("context").to_string()),
                s => r.failing.push(format!("{} ({s})", text("context"))),
            }
        } else if c["status"] != "COMPLETED" {
            r.pending.push(text("name").to_string());
        } else {
            let conclusion = text("conclusion");
            if !SETTLED.contains(&conclusion) {
                r.failing.push(format!("{} ({conclusion})", text("name")));
            } else if conclusion != "SUCCESS" {
                r.settled.push(format!("{} ({conclusion})", text("name")));
            }
        }
    }
    r.complete = !nodes.is_empty() && contexts["pageInfo"]["hasNextPage"] != true;
    Some(r)
}

fn state(pr: &Value) -> std::result::Result<State, String> {
    match pr["state"].as_str() {
        Some("OPEN") => Ok(State::Open),
        Some("MERGED") => Ok(State::Merged),
        Some("CLOSED") => Ok(State::Closed),
        other => Err(format!("GitHub returned an unknown pull request state {}", json!(other))),
    }
}

fn closing_issue(pr: &Value) -> Option<String> {
    pr["closingIssuesReferences"]["nodes"].as_array()?.iter().find_map(|n| {
        let repo = n["repository"]["nameWithOwner"].as_str()?;
        parse_github_issue_only(&format!("{repo}#{}", n["number"].as_u64()?)).map(|i| i.slug())
    })
}

fn change(repo: &Value) -> std::result::Result<Change, String> {
    let pr = &repo["pullRequest"];
    let state = state(pr)?;
    let s = |k: &str| pr[k].as_str().unwrap_or_default().to_string();
    let removal = &pr["timelineItems"]["nodes"][0];
    Ok(Change {
        id: pr["id"].as_str().map(String::from),
        state,
        draft: pr["isDraft"] == true,
        head: s("headRefOid").to_ascii_lowercase(),
        merge_state: pr["mergeStateStatus"].as_str().unwrap_or("UNKNOWN").into(),
        review_decision: pr["reviewDecision"].as_str().map(String::from),
        checks: rollup(pr),
        queue: pr["isMergeQueueEnabled"] == true,
        queued: pr["isInMergeQueue"] == true,
        position: pr["mergeQueueEntry"]["position"].as_u64(),
        commit: pr["mergeCommit"]["oid"].as_str().map(String::from),
        ended: pr[if state == State::Merged { "mergedAt" } else { "closedAt" }].as_str().map(String::from),
        removal: removal["createdAt"]
            .as_str()
            .map(|at| (removal["reason"].as_str().unwrap_or_default().to_string(), at.to_string())),
        method: repo["viewerDefaultMergeMethod"].as_str().map(String::from),
        closing_issue: closing_issue(pr),
    })
}

fn detail(repo: &Value) -> std::result::Result<Detail, String> {
    let pr = &repo["pullRequest"];
    let reviews = pr["latestOpinionatedReviews"]["nodes"].as_array().into_iter().flatten();
    let change_requests = reviews
        .filter(|r| r["state"] == "CHANGES_REQUESTED")
        .map(|r| ChangeRequest {
            author: r["author"]["login"].as_str().map(String::from),
            commit: r["commit"]["oid"].as_str().filter(|c| !c.is_empty()).map(String::from),
        })
        .collect();
    let files = pr["files"]["nodes"].as_array().into_iter().flatten();
    Ok(Detail {
        change: change(repo)?,
        body: pr["body"].as_str().unwrap_or_default().to_string(),
        files: files.filter_map(|f| f["path"].as_str().map(String::from)).collect(),
        more_files: pr["files"]["pageInfo"]["hasNextPage"] == true,
        change_requests,
    })
}

fn batch_query(chunk: &[&ChangeRef]) -> (String, Value) {
    let mut params = Vec::new();
    let mut fields = Vec::new();
    let mut vars = serde_json::Map::new();
    for (i, r) in chunk.iter().enumerate() {
        let (owner, name) = owner_name(r);
        params.push(format!("$o{i}: String!, $r{i}: String!, $n{i}: Int!"));
        fields.push(format!(
            "p{i}: repository(owner: $o{i}, name: $r{i}) {{ viewerDefaultMergeMethod pullRequest(number: $n{i}) {{ ...F }} }}"
        ));
        vars.insert(format!("o{i}"), json!(owner));
        vars.insert(format!("r{i}"), json!(name));
        vars.insert(format!("n{i}"), json!(r.number));
    }
    let query = format!("query Reconcile({}) {{ {} }} {PR_FIELDS} {CHECKS}", params.join(", "), fields.join(" "));
    (query, Value::Object(vars))
}

fn batch_read(resp: &Value, i: usize) -> Read {
    let alias = format!("p{i}");
    let repo = &resp["data"][&alias];
    let errs: Vec<&Value> =
        resp["errors"].as_array().into_iter().flatten().filter(|e| e["path"][0] == alias.as_str()).collect();
    let why = errs.iter().filter_map(|e| e["message"].as_str()).collect::<Vec<_>>().join("; ");
    let found = repo["pullRequest"].is_object();
    if found && errs.is_empty() {
        match change(repo) {
            Ok(c) => Read::Found(Box::new(c)),
            Err(e) => Read::Failed(format!("GitHub returned an incomplete read: {e}")),
        }
    } else if !found && errs.iter().any(|e| e["type"] == "NOT_FOUND") {
        Read::NotFound(why)
    } else {
        Read::Failed(format!(
            "GitHub returned an incomplete read: {}",
            if why.is_empty() { "no error given" } else { &why }
        ))
    }
}

impl GithubHost {
    async fn read_github(&self, refs: &[&ChangeRef]) -> Result<Vec<Read>> {
        let mut out = Vec::with_capacity(refs.len());
        for chunk in refs.chunks(BATCH) {
            let (query, vars) = batch_query(chunk);
            let resp = self.0.graphql(&query, vars).await?;
            if !resp["data"].is_object() {
                bail!("GitHub reconcile query failed: {}", errors(&resp).unwrap_or_else(|| "no data".into()));
            }
            out.extend((0..chunk.len()).map(|i| batch_read(&resp, i)));
        }
        Ok(out)
    }

    async fn deleted(&self, r: &ChangeRef) -> Branch {
        let (owner, name) = owner_name(r);
        let read = match self.0.graphql(PR_BRANCH, json!({"owner": owner, "name": name, "number": r.number})).await {
            Ok(v) => v,
            Err(e) => return Branch::Unread(format!("{e:#}")),
        };
        if let Some(e) = errors(&read) {
            return Branch::Unread(e);
        }
        let p = &read["data"]["repository"]["pullRequest"];
        let branch = p["headRefName"].as_str().unwrap_or("of the PR").to_string();
        if p["state"] != "MERGED" {
            return Branch::NotMerged(branch);
        }
        if p["isCrossRepository"] == true {
            return Branch::Fork(branch);
        }
        let Some(id) = p["headRef"]["id"].as_str() else {
            return Branch::Gone(branch);
        };
        let tip = p["headRef"]["target"]["oid"].as_str().unwrap_or_default();
        if !tip.eq_ignore_ascii_case(p["headRefOid"].as_str().unwrap_or_default()) {
            return Branch::TipMoved { branch, tip: tip.into() };
        }
        match self.0.graphql(DELETE_REF, json!({"ref": id})).await {
            Ok(v) => match errors(&v) {
                None => Branch::Deleted(branch),
                Some(why) => Branch::Failed { branch, why },
            },
            Err(e) => Branch::Failed { branch, why: format!("{e:#}") },
        }
    }
}

impl CodeHost for GithubHost {
    fn read<'a>(&'a self, refs: &'a [ChangeRef]) -> BoxFut<'a, Result<Vec<Read>>> {
        Box::pin(async move {
            let ours: Vec<&ChangeRef> = refs.iter().filter(|r| r.forge == Forge::Github).collect();
            let mut read = self.read_github(&ours).await?.into_iter();
            Ok(refs
                .iter()
                .map(|r| match r.forge {
                    Forge::Github => read.next().unwrap_or_else(|| Read::Failed("GitHub returned no read".into())),
                    _ => Read::Failed(not_github(r)),
                })
                .collect())
        })
    }

    fn view<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Result<Viewed>> {
        Box::pin(async move {
            if r.forge != Forge::Github {
                bail!("{}", not_github(r));
            }
            let (owner, name) = owner_name(r);
            let query = format!("{VIEW_PR} {CHECKS}");
            let resp = self.0.graphql(&query, json!({"owner": owner, "name": name, "number": r.number})).await?;
            if let Some(e) = errors(&resp) {
                return Ok(Viewed::Refused(e));
            }
            let repo = &resp["data"]["repository"];
            if repo["pullRequest"].is_null() {
                return Ok(Viewed::NotFound);
            }
            Ok(match detail(repo) {
                Ok(d) => Viewed::Found(Box::new(d)),
                Err(e) => Viewed::Refused(e),
            })
        })
    }

    fn merge<'a>(
        &'a self,
        r: &'a ChangeRef,
        id: &'a str,
        head: &'a str,
        how: Landing<'a>,
    ) -> BoxFut<'a, Result<Landed>> {
        Box::pin(async move {
            if r.forge != Forge::Github {
                bail!("{}", not_github(r));
            }
            let (query, vars) = match how {
                Landing::Queue => (ENQUEUE_PR, json!({"id": id, "head": head})),
                Landing::Merge { method } => (MERGE_PR, json!({"id": id, "head": head, "method": method})),
            };
            let resp = self.0.graphql(query, vars).await?;
            if let Some(e) = errors(&resp) {
                return Ok(Landed::Refused(e));
            }
            Ok(match how {
                Landing::Queue => Landed::Queued {
                    position: resp["data"]["enqueuePullRequest"]["mergeQueueEntry"]["position"].as_u64(),
                },
                Landing::Merge { .. } => Landed::Merged {
                    commit: resp["data"]["mergePullRequest"]["pullRequest"]["mergeCommit"]["oid"]
                        .as_str()
                        .map(String::from),
                },
            })
        })
    }

    fn delete_branch<'a>(&'a self, r: &'a ChangeRef) -> BoxFut<'a, Branch> {
        Box::pin(async move {
            if r.forge != Forge::Github {
                return Branch::Unread(not_github(r));
            }
            self.deleted(r).await
        })
    }

    fn url(&self, r: &ChangeRef) -> String {
        format!("https://github.com/{}/pull/{}", r.repo, r.number)
    }
}

pub fn github_host(github: Arc<dyn GitHub>) -> Arc<dyn CodeHost> {
    Arc::new(GithubHost::new(github))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeGitHub, github_pr};

    #[tokio::test]
    async fn reads_github_refs_and_turns_other_forges_away() {
        let gh = Arc::new(FakeGitHub::default());
        gh.set_pr("acme/widgets", 7, github_pr(&"a".repeat(40), "CLEAN", false));
        let host = GithubHost::new(gh.clone());
        let lab = ChangeRef {
            forge: Forge::Gitlab { host: "gitlab.example.com".into() },
            ..ChangeRef::github("acme/widgets", 7)
        };
        let refs = [ChangeRef::github("acme/widgets", 7), lab.clone(), ChangeRef::github("acme/widgets", 8)];

        let reads = host.read(&refs).await.unwrap();

        assert!(matches!(&reads[0], Read::Found(c) if c.state == State::Open && c.merge_state == "CLEAN"));
        assert!(matches!(&reads[1], Read::Failed(why) if why.contains("not on GitHub")));
        assert!(matches!(&reads[2], Read::NotFound(_)));
        assert_eq!(gh.operations(), ["reconcile"]);
        assert!(host.view(&lab).await.is_err());
        assert_eq!(host.url(&refs[0]), "https://github.com/acme/widgets/pull/7");
    }
}
