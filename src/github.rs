use std::fmt;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::t3::BoxFut;

const GH_DEADLINE: Duration = Duration::from_secs(30);

pub const SUB_ISSUES_HEADERS: &[(&str, &str)] = &[("GraphQL-Features", "sub_issues")];

pub const REPO_ID: &str = "query RepoId($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    id
    nameWithOwner
  }
}";

pub const ISSUE_NODE: &str = "query IssueNode($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    id
    nameWithOwner
    issue(number: $number) {
      id
      number
      url
      title
      parent { id number repository { nameWithOwner } }
      blockedBy(first: 100) {
        nodes { id number repository { nameWithOwner } }
      }
      labels(first: 100) {
        nodes { id name }
      }
    }
  }
}";

pub const REPO_LABEL: &str = "query RepoLabel($owner: String!, $name: String!, $label: String!) {
  repository(owner: $owner, name: $name) {
    id
    label(name: $label) { id name }
  }
}";

pub const CREATE_LABEL: &str =
    "mutation CreateLabel($repoId: ID!, $name: String!, $color: String!, $description: String) {
  createLabel(input: { repositoryId: $repoId, name: $name, color: $color, description: $description }) {
    label { id name }
  }
}";

pub const ADD_LABELS: &str = "mutation AddLabelsToLabelable($labelableId: ID!, $labelIds: [ID!]!) {
  addLabelsToLabelable(input: { labelableId: $labelableId, labelIds: $labelIds }) {
    labelable { __typename }
  }
}";

pub const ADD_BLOCKED_BY: &str = "mutation AddBlockedBy($issueId: ID!, $blockingIssueId: ID!) {
  addBlockedBy(input: { issueId: $issueId, blockingIssueId: $blockingIssueId }) {
    issue { id number }
    blockingIssue { id number }
  }
}";

pub const REMOVE_BLOCKED_BY: &str = "mutation RemoveBlockedBy($issueId: ID!, $blockingIssueId: ID!) {
  removeBlockedBy(input: { issueId: $issueId, blockingIssueId: $blockingIssueId }) {
    issue { id number }
    blockingIssue { id number }
  }
}";

pub const ADD_SUB_ISSUE: &str = "mutation AddSubIssue($issueId: ID!, $subIssueId: ID!, $replaceParent: Boolean) {
  addSubIssue(input: { issueId: $issueId, subIssueId: $subIssueId, replaceParent: $replaceParent }) {
    issue { id number }
    subIssue { id number }
  }
}";

pub const REMOVE_SUB_ISSUE: &str = "mutation RemoveSubIssue($issueId: ID!, $subIssueId: ID!) {
  removeSubIssue(input: { issueId: $issueId, subIssueId: $subIssueId }) {
    issue { id number }
    subIssue { id number }
  }
}";

pub const CREATE_ISSUE: &str = "mutation CreateIssue($repoId: ID!, $title: String!, $body: String, $labelIds: [ID!]) {
  createIssue(input: { repositoryId: $repoId, title: $title, body: $body, labelIds: $labelIds }) {
    issue { id number url title }
  }
}";

pub const FLOW_OPEN_PRS: &str = "query FlowOpenPrs($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    defaultBranchRef { name }
    pullRequests(states: OPEN, first: 100, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title url body baseRefName headRefName headRefOid isCrossRepository
        closingIssuesReferences(first: 20) { nodes { number repository { nameWithOwner } } }
        files(first: 100) { nodes { path } }
      }
    }
  }
}";

pub trait GitHub: Send + Sync {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>>;

    fn graphql_with_headers<'a>(
        &'a self,
        query: &'a str,
        variables: Value,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFut<'a, Result<Value>> {
        let _ = headers;
        self.graphql(query, variables)
    }
}

pub struct GhCli;

impl GitHub for GhCli {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>> {
        self.graphql_with_headers(query, variables, &[])
    }

    fn graphql_with_headers<'a>(
        &'a self,
        query: &'a str,
        variables: Value,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFut<'a, Result<Value>> {
        Box::pin(async move {
            let body = serde_json::to_vec(&json!({"query": query, "variables": variables}))?;
            let mut cmd = tokio::process::Command::new("gh");
            cmd.args(["api", "graphql"]);
            for (k, v) in headers {
                cmd.args(["-H", &format!("{k}: {v}")]);
            }
            let mut child = cmd
                .args(["--input", "-"])
                .env("GH_PROMPT_DISABLED", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| anyhow!("cannot run gh: {e}"))?;
            let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("gh has no stdin"))?;
            let run = async move {
                stdin.write_all(&body).await?;
                drop(stdin);
                child.wait_with_output().await
            };
            let out = tokio::time::timeout(GH_DEADLINE, run)
                .await
                .map_err(|_| anyhow!("gh api graphql timed out after {}s", GH_DEADLINE.as_secs()))??;
            gh_response(out.status.success(), &out.stdout, &out.stderr)
        })
    }
}

pub fn gh_response(ok: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(stdout)
        && (v.get("data").is_some() || v.get("errors").is_some())
    {
        return Ok(v);
    }
    let err = String::from_utf8_lossy(stderr).trim().to_string();
    if ok {
        bail!("gh api graphql returned no GraphQL response");
    }
    bail!("gh api graphql failed: {}", if err.is_empty() { "no output" } else { &err })
}

pub fn errors(resp: &Value) -> Option<String> {
    let errs = resp["errors"].as_array().filter(|a| !a.is_empty())?;
    let msgs: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
    Some(if msgs.is_empty() { "GitHub returned an error".into() } else { msgs.join("; ") })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphQlError {
    pub message: String,
    pub permission: bool,
}

impl GraphQlError {
    pub fn new(message: impl Into<String>, permission: bool) -> Self {
        Self { message: message.into(), permission }
    }

    pub fn from_transport(err: anyhow::Error) -> Self {
        let message = err.to_string();
        let permission = is_permission_message(&message);
        Self { message, permission }
    }
}

impl fmt::Display for GraphQlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GraphQlError {}

pub fn is_permission_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    [
        "permission",
        "forbidden",
        "not accessible",
        "insufficient",
        "must have push",
        "must have write",
        "must have triage",
        "must have admin",
        "viewer does not have",
        "cannot modify",
        "unauthorized",
        "http 403",
        "status 403",
        "not logged in",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn graphql_error(resp: &Value) -> Option<GraphQlError> {
    let errs = resp["errors"].as_array().filter(|a| !a.is_empty())?;
    let msgs: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
    let message = if msgs.is_empty() { "GitHub returned an error".to_string() } else { msgs.join("; ") };
    let typed_perm = errs.iter().any(|e| {
        e["type"]
            .as_str()
            .is_some_and(|t| t.eq_ignore_ascii_case("FORBIDDEN") || t.eq_ignore_ascii_case("INSUFFICIENT_SCOPES"))
    });
    let permission = typed_perm || is_permission_message(&message);
    Some(GraphQlError { message, permission })
}

fn is_already_blocked_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("already blocked")
        || lower.contains("already exists")
        || lower.contains("already linked")
        || lower.contains("has already been added")
        || lower.contains("duplicate")
        || lower.contains("relationship already")
}

pub fn is_parent_conflict_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("already has a parent")
        || lower.contains("only have one parent")
        || lower.contains("different parent")
        || lower.contains("parent conflict")
}

fn is_already_sub_issue_error(msg: &str) -> bool {
    // Do NOT treat "already has a parent" (different parent conflict) as idempotent success.
    if is_parent_conflict_message(msg) {
        return false;
    }
    let lower = msg.to_ascii_lowercase();
    lower.contains("already a sub-issue")
        || lower.contains("already a sub issue")
        || lower.contains("duplicate sub-issue")
        || lower.contains("duplicate sub issue")
        || lower.contains("sub-issue is already")
        || lower.contains("already exists")
        || lower.contains("already linked")
        || lower.contains("parent already has")
}

fn is_not_blocked_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("not blocked")
        || lower.contains("does not block")
        || lower.contains("dependency does not exist")
        || lower.contains("relationship does not exist")
        || lower.contains("no blocking relationship")
}

fn is_not_sub_issue_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("not a sub-issue")
        || lower.contains("not a sub issue")
        || lower.contains("sub-issue does not exist")
        || lower.contains("sub-issue relationship does not exist")
}

fn is_label_exists_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("already exists") || lower.contains("already been taken")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoNode {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub name_with_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueNodeRef {
    pub id: String,
    pub repo: String,
    pub number: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelNode {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueNode {
    pub id: String,
    pub repo_id: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub parent: Option<IssueNodeRef>,
    pub blocked_by: Vec<IssueNodeRef>,
    pub labels: Vec<LabelNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedIssue {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked,
    AlreadyLinked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlinkOutcome {
    Removed,
    NotLinked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnsureLabelOutcome {
    Existing(LabelNode),
    Created(LabelNode),
}

#[derive(Clone, Copy, Debug)]
pub struct LabelSpec<'a> {
    pub name: &'a str,
    pub alt_name: Option<&'a str>,
    pub color: &'a str,
    pub description: Option<&'a str>,
}

fn parse_issue_node_ref(v: &Value) -> Option<IssueNodeRef> {
    let id = v["id"].as_str()?.to_string();
    let number = v["number"].as_u64()?;
    let repo = v["repository"]["nameWithOwner"].as_str()?.to_string();
    Some(IssueNodeRef { id, repo, number })
}

pub async fn read_repo(gh: &dyn GitHub, owner: &str, name: &str) -> std::result::Result<RepoNode, GraphQlError> {
    let resp =
        gh.graphql(REPO_ID, json!({"owner": owner, "name": name})).await.map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        return Err(err);
    }
    let repo = &resp["data"]["repository"];
    let id = repo["id"]
        .as_str()
        .ok_or_else(|| GraphQlError::new(format!("repository {owner}/{name} was not found"), false))?
        .to_string();
    let name_with_owner = repo["nameWithOwner"].as_str().map_or_else(|| format!("{owner}/{name}"), String::from);
    let (canon_owner, canon_name) = name_with_owner.split_once('/').unwrap_or((owner, name));
    Ok(RepoNode { id, owner: canon_owner.to_string(), name: canon_name.to_string(), name_with_owner })
}

pub async fn read_issue_node(
    gh: &dyn GitHub,
    owner: &str,
    name: &str,
    number: u64,
) -> std::result::Result<IssueNode, GraphQlError> {
    if number == 0 || number > i32::MAX as u64 {
        return Err(GraphQlError::new(format!("invalid GitHub issue number #{number}"), false));
    }
    let resp = gh
        .graphql(ISSUE_NODE, json!({"owner": owner, "name": name, "number": number}))
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        return Err(err);
    }
    let repo = &resp["data"]["repository"];
    let repo_id = repo["id"]
        .as_str()
        .ok_or_else(|| GraphQlError::new(format!("repository {owner}/{name} was not found"), false))?
        .to_string();
    let repo_name = repo["nameWithOwner"].as_str().map_or_else(|| format!("{owner}/{name}"), String::from);
    let issue = &repo["issue"];
    let id = issue["id"]
        .as_str()
        .ok_or_else(|| GraphQlError::new(format!("{repo_name}#{number} was not found"), false))?
        .to_string();
    let num = issue["number"].as_u64().unwrap_or(number);
    let url =
        issue["url"].as_str().map_or_else(|| format!("https://github.com/{repo_name}/issues/{num}"), String::from);
    let title = issue["title"].as_str().unwrap_or_default().to_string();
    let parent = parse_issue_node_ref(&issue["parent"]);
    let blocked_by =
        issue["blockedBy"]["nodes"].as_array().into_iter().flatten().filter_map(parse_issue_node_ref).collect();
    let labels = issue["labels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| Some(LabelNode { id: l["id"].as_str()?.to_string(), name: l["name"].as_str()?.to_string() }))
        .collect();
    Ok(IssueNode { id, repo_id, repo: repo_name, number: num, url, title, parent, blocked_by, labels })
}

pub async fn find_label(
    gh: &dyn GitHub,
    owner: &str,
    name: &str,
    label: &str,
) -> std::result::Result<Option<LabelNode>, GraphQlError> {
    let resp = gh
        .graphql(REPO_LABEL, json!({"owner": owner, "name": name, "label": label}))
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        return Err(err);
    }
    let node = &resp["data"]["repository"]["label"];
    let Some(id) = node["id"].as_str() else {
        return Ok(None);
    };
    let found_name = node["name"].as_str().unwrap_or(label).to_string();
    Ok(Some(LabelNode { id: id.to_string(), name: found_name }))
}

pub async fn ensure_label(
    gh: &dyn GitHub,
    repo: &RepoNode,
    spec: &LabelSpec<'_>,
) -> std::result::Result<EnsureLabelOutcome, GraphQlError> {
    if let Some(found) = find_label(gh, &repo.owner, &repo.name, spec.name).await? {
        return Ok(EnsureLabelOutcome::Existing(found));
    }
    if let Some(alt) = spec.alt_name
        && !alt.eq_ignore_ascii_case(spec.name)
        && let Some(found) = find_label(gh, &repo.owner, &repo.name, alt).await?
    {
        return Ok(EnsureLabelOutcome::Existing(found));
    }
    let resp = gh
        .graphql(
            CREATE_LABEL,
            json!({
                "repoId": repo.id,
                "name": spec.name,
                "color": spec.color,
                "description": spec.description,
            }),
        )
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        if is_label_exists_error(&err.message)
            && let Some(found) = find_label(gh, &repo.owner, &repo.name, spec.name).await?
        {
            return Ok(EnsureLabelOutcome::Existing(found));
        }
        return Err(err);
    }
    let node = &resp["data"]["createLabel"]["label"];
    let id = node["id"]
        .as_str()
        .ok_or_else(|| GraphQlError::new(format!("GitHub returned no label id for {:?}", spec.name), false))?
        .to_string();
    let name = node["name"].as_str().unwrap_or(spec.name).to_string();
    Ok(EnsureLabelOutcome::Created(LabelNode { id, name }))
}

pub async fn add_labels(
    gh: &dyn GitHub,
    labelable_id: &str,
    label_ids: &[String],
) -> std::result::Result<(), GraphQlError> {
    if label_ids.is_empty() {
        return Ok(());
    }
    let resp = gh
        .graphql(ADD_LABELS, json!({"labelableId": labelable_id, "labelIds": label_ids}))
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        return Err(err);
    }
    Ok(())
}

pub async fn add_blocked_by(
    gh: &dyn GitHub,
    issue_id: &str,
    blocking_issue_id: &str,
) -> std::result::Result<LinkOutcome, GraphQlError> {
    let resp = gh
        .graphql(ADD_BLOCKED_BY, json!({"issueId": issue_id, "blockingIssueId": blocking_issue_id}))
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        if is_already_blocked_error(&err.message) {
            return Ok(LinkOutcome::AlreadyLinked);
        }
        return Err(err);
    }
    Ok(LinkOutcome::Linked)
}

pub async fn remove_blocked_by(
    gh: &dyn GitHub,
    issue_id: &str,
    blocking_issue_id: &str,
) -> std::result::Result<UnlinkOutcome, GraphQlError> {
    let resp = gh
        .graphql(REMOVE_BLOCKED_BY, json!({"issueId": issue_id, "blockingIssueId": blocking_issue_id}))
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        if is_not_blocked_error(&err.message) {
            return Ok(UnlinkOutcome::NotLinked);
        }
        return Err(err);
    }
    Ok(UnlinkOutcome::Removed)
}

pub async fn add_sub_issue(
    gh: &dyn GitHub,
    parent_id: &str,
    sub_issue_id: &str,
    replace_parent: bool,
) -> std::result::Result<LinkOutcome, GraphQlError> {
    let resp = gh
        .graphql_with_headers(
            ADD_SUB_ISSUE,
            json!({"issueId": parent_id, "subIssueId": sub_issue_id, "replaceParent": replace_parent}),
            SUB_ISSUES_HEADERS,
        )
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        if is_already_sub_issue_error(&err.message) {
            return Ok(LinkOutcome::AlreadyLinked);
        }
        return Err(err);
    }
    Ok(LinkOutcome::Linked)
}

pub async fn remove_sub_issue(
    gh: &dyn GitHub,
    parent_id: &str,
    sub_issue_id: &str,
) -> std::result::Result<UnlinkOutcome, GraphQlError> {
    let resp = gh
        .graphql_with_headers(
            REMOVE_SUB_ISSUE,
            json!({"issueId": parent_id, "subIssueId": sub_issue_id}),
            SUB_ISSUES_HEADERS,
        )
        .await
        .map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        if is_not_sub_issue_error(&err.message) {
            return Ok(UnlinkOutcome::NotLinked);
        }
        return Err(err);
    }
    Ok(UnlinkOutcome::Removed)
}

pub async fn create_issue(
    gh: &dyn GitHub,
    repo: &RepoNode,
    title: &str,
    body: &str,
    label_ids: &[String],
) -> std::result::Result<CreatedIssue, GraphQlError> {
    let mut vars = json!({"repoId": repo.id, "title": title, "body": body});
    if !label_ids.is_empty() {
        vars["labelIds"] = json!(label_ids);
    }
    let resp = gh.graphql(CREATE_ISSUE, vars).await.map_err(GraphQlError::from_transport)?;
    if let Some(err) = graphql_error(&resp) {
        return Err(err);
    }
    let issue = &resp["data"]["createIssue"]["issue"];
    let id = issue["id"]
        .as_str()
        .ok_or_else(|| GraphQlError::new("GitHub returned no created issue id", false))?
        .to_string();
    let number =
        issue["number"].as_u64().ok_or_else(|| GraphQlError::new("GitHub returned no created issue number", false))?;
    let url = issue["url"]
        .as_str()
        .map_or_else(|| format!("https://github.com/{}/issues/{number}", repo.name_with_owner), String::from);
    let out_title = issue["title"].as_str().unwrap_or(title).to_string();
    Ok(CreatedIssue { id, repo: repo.name_with_owner.clone(), number, url, title: out_title })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_permission_and_idempotent_errors() {
        let forbidden = json!({"errors": [{"type": "FORBIDDEN", "message": "Resource not accessible by integration"}]});
        let err = graphql_error(&forbidden).unwrap();
        assert!(err.permission);

        let not_found = json!({"errors": [{"type": "NOT_FOUND", "message": "Could not resolve to an Issue"}]});
        let err = graphql_error(&not_found).unwrap();
        assert!(!err.permission);

        assert!(is_already_blocked_error("Issue is already blocked by this issue"));
        assert!(!is_already_blocked_error("Viewer does not have permission"));

        assert!(is_already_sub_issue_error("Sub-issue is already a sub-issue of this issue"));
        assert!(
            !is_already_sub_issue_error("Sub-issue already has a parent"),
            "different parent conflict must not be treated as already linked"
        );
        assert!(
            !is_already_sub_issue_error("Issue may only have one parent"),
            "single-parent conflict must not be treated as already linked"
        );

        assert!(is_not_blocked_error("Issue is not blocked by this issue"));
        assert!(is_not_sub_issue_error("Issue is not a sub-issue of this issue"));
        assert!(is_label_exists_error("Name has already been taken"));
    }
}
