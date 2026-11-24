use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, card_text, reply_options};
use t3_codemode::config::Config;
use t3_codemode::fake::{gitlab_mr, gitlab_pipeline, project, thread};

const HOST: &str = "gitlab.acme.example";
const PATH: &str = "acme/platform/widgets";
const KEY: &str = "gitlab.acme.example/acme/platform/widgets";
const URL: &str = "https://gitlab.acme.example/acme/platform/widgets/-/merge_requests/7";
const HEAD: &str = "1111111111111111111111111111111111111111";
const PUBLISHING: &str = "Publishing: pr skill; passes: none configured";
const BODY: &str = "## Summary\n\n```text\nwf.pr\n  gitlab\n```\n\n## Evidence\n\n- **Before:** none\n  **After:** green\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\nRefs acme/widgets#41\n";

fn setup() -> H {
    let h = H::build(Vec::new(), Config::from_value(json!({"forges": {HOST: {"kind": "gitlab"}}})).unwrap());
    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let delivery =
        json!({"delivery": {"steps": [{"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"}]}});
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, delivery.to_string()).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    let mut th = thread(WORKER, "idle", T0);
    th["projectId"] = json!("proj:widgets");
    h.fake.put(th);
    h
}

async fn ticket_and_worker(h: &H, issue: &str) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 1/1", "issue": issue, "part": "1/1", "pr": {"format": "pr"}}),
    )
    .await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
}

fn open_mr(h: &H, body: &str, files: usize, pipeline: Option<&str>) {
    let mut mr = gitlab_mr(HEAD, "mergeable");
    mr["description"] = json!(body);
    h.gitlab.set_mr(HOST, PATH, 7, mr);
    let names: Vec<String> = (0..files).map(|i| format!("src/f{i}.rs")).collect();
    h.gitlab.set_files(HOST, PATH, 7, &names.iter().map(String::as_str).collect::<Vec<_>>());
    if let Some(status) = pipeline {
        h.gitlab.set_pipeline(HOST, PATH, 7, status);
    }
}

fn opened(repo: &str) -> Value {
    json!({"event": "opened", "repo": repo, "head": HEAD, "publishing": PUBLISHING, "checks": "pending"})
}

fn ready() -> Value {
    json!({"event": "ready", "repo": KEY, "pr": 7, "head": HEAD})
}

fn cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

#[tokio::test]
async fn a_merge_request_url_is_checked_and_its_ready_opens_a_merge_card() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 1, Some("success"));

    let r = h.wf(WORKER, "pr", opened(URL)).await;
    assert_eq!(r["status"], "open", "{r}");
    assert_eq!(h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap().repo, KEY);

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "gated", "{r}");
    let open = cards(&h);
    assert_eq!(open.len(), 1, "{open:?}");
    let pr = h.app.wf.cards().unwrap().remove(0).pr.unwrap();
    assert_eq!((pr.repo.as_str(), pr.number, pr.ready), (KEY, 7, true));
    let shown = h.wf(TICKET, "show", json!({"handle": open[0]["handle"]})).await;
    let body = shown["body"].as_str().unwrap();
    assert!(body.starts_with("**ACTION NEEDED:** merge #7 at head 1111111"), "{body}");
    assert!(body.contains(&format!("here: {URL}")), "{body}");
    let shown = card_text(body);
    let note = shown.find("t3cm sends your answer to the ticket, which merges it on GitLab");
    assert!(note.is_some_and(|i| i < shown.find("reply").unwrap()), "the note leads the reply clause: {shown}");
    assert_eq!(reply_options(body), ["merge"]);
    let paths: Vec<String> = h.gitlab.calls().iter().filter_map(|c| c["path"].as_str().map(String::from)).collect();
    assert!(paths.iter().any(|p| p.ends_with("/merge_requests/7/diffs?per_page=100&page=1")), "{paths:?}");
}

#[tokio::test]
async fn refs_with_a_bare_number_resolve_against_the_merge_requests_project() {
    let h = setup();
    ticket_and_worker(&h, &format!("{KEY}#41")).await;
    open_mr(&h, &BODY.replace("Refs acme/widgets#41", "Refs #41"), 1, Some("success"));
    let mut args = opened(KEY);
    args["pr"] = json!(7);
    let r = h.wf(WORKER, "pr", args).await;
    assert_eq!(r["status"], "open", "{r}");
}

#[tokio::test]
async fn ready_with_no_pipeline_at_the_head_goes_to_the_ticket() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 1, None);
    h.wf(WORKER, "pr", opened(URL)).await;

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().starts_with("GitLab shows no finished pipeline at head 1111111"), "{r}");
    assert!(h.app.wf.cards().unwrap().iter().all(|c| c.pr.as_ref().is_none_or(|p| !p.ready)));
}

#[tokio::test]
async fn the_merge_request_pipeline_counts_past_a_page_of_train_pipelines() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 1, None);
    let mut pipelines: Vec<Value> =
        (0..100).map(|i| gitlab_pipeline(1000 - i, HEAD, "refs/merge-requests/7/train", "failed")).collect();
    pipelines.push(gitlab_pipeline(400, HEAD, "refs/merge-requests/7/head", "running"));
    pipelines.push(gitlab_pipeline(300, HEAD, "refs/merge-requests/7/head", "failed"));
    h.gitlab.update_mr(HOST, PATH, 7, json!({"pipelines": pipelines}));
    h.wf(WORKER, "pr", opened(URL)).await;

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "waiting", "the newest MR pipeline is still running: {r}");
    assert_eq!(r["pending"], "checks still running: pipeline 400", "{r}");
}

#[tokio::test]
async fn a_merge_request_past_a_page_of_changed_files_goes_to_the_ticket() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 101, Some("success"));
    h.wf(WORKER, "pr", opened(URL)).await;

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("more than 100 files"), "{r}");
}

#[tokio::test]
async fn wf_pr_names_what_is_wrong_with_a_merge_request_ref() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    let numbered = |repo: &str| {
        let mut a = opened(repo);
        a["pr"] = json!(7);
        a
    };
    let err = h.wf_err(WORKER, "pr", opened(KEY)).await;
    assert!(err.contains("pr must be"), "{err}");
    let err = h.wf_err(WORKER, "pr", numbered("gitlab.other.example/acme/widgets")).await;
    assert!(err.contains("gitlab.other.example is not listed under forges"), "{err}");
    let mut mismatched = opened(URL);
    mismatched["pr"] = json!(8);
    let err = h.wf_err(WORKER, "pr", mismatched).await;
    assert!(err.contains("pr 8 does not match"), "{err}");
    let err = h.wf_err(WORKER, "pr", numbered("acme")).await;
    assert!(err.contains("merge request URL"), "{err}");
}

#[tokio::test]
async fn a_diff_list_shorter_than_the_changes_count_goes_to_the_ticket() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 3, Some("success"));
    h.gitlab.update_mr(HOST, PATH, 7, json!({"changes_count": "4"}));
    h.wf(WORKER, "pr", opened(URL)).await;

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("more than 100 files"), "{r}");
}

#[tokio::test]
async fn a_head_that_moves_while_t3cm_reads_the_files_goes_to_the_ticket() {
    let h = setup();
    ticket_and_worker(&h, "acme/widgets#41").await;
    open_mr(&h, BODY, 1, Some("success"));
    h.wf(WORKER, "pr", opened(URL)).await;
    h.gitlab.patch_after("diffs", json!({"sha": "2222222222222222222222222222222222222222"}));

    let r = h.wf(WORKER, "pr", ready()).await;
    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("head moved while t3cm read it"), "{r}");
    assert!(h.app.wf.cards().unwrap().iter().all(|c| c.pr.as_ref().is_none_or(|p| !p.ready)));
}
