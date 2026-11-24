use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{gitlab_gid, gitlab_mr, project, thread};
use t3_codemode::store::WaitStatus;

const HOST: &str = "gitlab.acme.example";
const PATH: &str = "acme/platform/widgets";
const KEY: &str = "gitlab.acme.example/acme/platform/widgets";
const URL: &str = "https://gitlab.acme.example/acme/platform/widgets/-/merge_requests/7";
const HEAD: &str = "1111111111111111111111111111111111111111";
const MOVED: &str = "2222222222222222222222222222222222222222";
const BODY: &str = "## Summary\n\n```text\nwf.merge\n  gitlab\n```\n\n## Evidence\n\n- **Before:** none\n  **After:** green\n\n\
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

fn open_mr(h: &H, detailed: &str, pipeline: &str) {
    let mut mr = gitlab_mr(HEAD, detailed);
    mr["description"] = json!(BODY);
    h.gitlab.set_mr(HOST, PATH, 7, mr);
    h.gitlab.set_files(HOST, PATH, 7, &["src/lib.rs"]);
    h.gitlab.set_pipeline(HOST, PATH, 7, pipeline);
}

fn merge(repo: &str) -> Value {
    json!({"repo": repo, "pr": 7, "head": HEAD})
}

fn writes(h: &H) -> Vec<Value> {
    h.gitlab.calls().into_iter().filter(|c| c["method"].is_string()).collect()
}

fn ops(h: &H) -> Vec<String> {
    writes(h).iter().map(|c| c["operation"].as_str().unwrap_or_default().to_string()).collect()
}

fn to(h: &H, thread: &str) -> Vec<String> {
    h.sent().into_iter().filter(|s| s["threadId"] == thread).map(|s| s["message"].as_str().unwrap().into()).collect()
}

async fn ticket_and_worker(h: &H) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 1/1", "issue": "acme/widgets#41", "part": "1/1",
               "pr": {"format": "pr", "method": "squash"}}),
    )
    .await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
}

#[tokio::test]
async fn a_mergeable_merge_request_merges_directly_and_drops_its_branch() {
    let h = setup();
    open_mr(&h, "mergeable", "success");

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "merged", "{r}");
    assert_eq!((r["repo"].as_str(), r["pr"].as_u64()), (Some(KEY), Some(7)));
    let commit = format!("{:040x}", 1);
    assert_eq!(r["commit"], commit.as_str());
    assert_eq!(r["method"], "merge");
    let put = &writes(&h)[0];
    assert_eq!(put["path"], "projects/acme%2Fplatform%2Fwidgets/merge_requests/7/merge");
    assert_eq!(put["method"], "PUT");
    assert_eq!(
        put["body"],
        json!({"sha": HEAD, "squash": false, "auto_merge": true, "should_remove_source_branch": true})
    );
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["state"], "merged");
    assert!(h.app.store.merge_wait_get(KEY, 7).unwrap().is_none());
}

#[tokio::test]
async fn the_merge_request_url_and_its_squash_setting_carry_through() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.update_mr(HOST, PATH, 7, json!({"squash_on_merge": true}));

    let r = h.wf(TICKET, "merge", json!({"repo": URL, "head": HEAD})).await;

    assert_eq!(r["status"], "merged", "{r}");
    assert_eq!(r["method"], "squash");
    assert_eq!(writes(&h)[0]["body"]["squash"], true);
    let mr = h.gitlab.mr(HOST, PATH, 7);
    assert_eq!((mr["merge_commit_sha"].is_null(), &mr["squash_commit_sha"]), (true, &r["commit"]));
}

#[tokio::test]
async fn a_merge_train_project_joins_the_train_and_reports_its_position() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.set_trains(HOST, PATH, true);

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "queued", "{r}");
    assert_eq!(r["position"], 1);
    let post = &writes(&h)[0];
    assert_eq!(
        (post["method"].as_str(), post["path"].as_str()),
        (Some("POST"), Some("projects/acme%2Fplatform%2Fwidgets/merge_trains/merge_requests/7"))
    );
    assert_eq!(post["body"], json!({"sha": HEAD, "auto_merge": true, "squash": false}));
    assert!(h.gitlab.operations().contains(&"trainCar".to_string()));
    let w = h.app.store.merge_wait_get(KEY, 7).unwrap().unwrap();
    assert_eq!((w.status, w.pull_request_id.as_str()), (WaitStatus::Sent, gitlab_gid(7).as_str()));

    let again = h.wf(TICKET, "merge", merge(KEY)).await;
    assert_eq!(again["status"], "queued", "{again}");
    assert_eq!(ops(&h), ["joinTrain"], "an MR already on the train is not added again");
}

#[tokio::test]
async fn a_running_pipeline_waits_and_the_reconcile_merges_once_it_passes() {
    let h = setup();
    open_mr(&h, "ci_still_running", "running");

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "waiting", "{r}");
    assert_eq!(r["mergeState"], "BLOCKED");
    assert!(ops(&h).is_empty(), "nothing is sent to GitLab while the pipeline runs: {:?}", ops(&h));
    let w = h.app.store.merge_wait_get(KEY, 7).unwrap().unwrap();
    assert_eq!(w.status, WaitStatus::Waiting);

    h.gitlab.set_pipeline(HOST, PATH, 7, "success");
    h.gitlab.update_mr(HOST, PATH, 7, json!({"detailed_merge_status": "mergeable"}));
    h.app.wf.reconcile().await.unwrap();

    assert_eq!(ops(&h), ["merge"]);
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["state"], "merged");
}

#[tokio::test]
async fn the_reconcile_joins_a_train_for_a_waiting_approval() {
    let h = setup();
    open_mr(&h, "ci_still_running", "running");
    h.gitlab.set_trains(HOST, PATH, true);
    assert_eq!(h.wf(TICKET, "merge", merge(KEY)).await["status"], "waiting");

    h.gitlab.set_pipeline(HOST, PATH, 7, "success");
    h.gitlab.update_mr(HOST, PATH, 7, json!({"detailed_merge_status": "mergeable"}));
    let actions = h.app.wf.reconcile().await.unwrap();

    assert_eq!(ops(&h), ["joinTrain"]);
    assert!(actions.iter().any(|a| a.contains("queued at its approved head 1111111 (position 1)")), "{actions:?}");
    let seen = h.app.store.pr_seen_get(KEY, 7).unwrap().unwrap();
    assert!(seen.queued);
    assert_eq!(h.app.store.merge_wait_get(KEY, 7).unwrap().unwrap().status, WaitStatus::Sent);
}

#[tokio::test]
async fn an_auto_merge_gitlab_holds_is_taken_back_so_the_approval_stays_with_t3cm() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.patch_after("pipelines", json!({"detailed_merge_status": "ci_still_running"}));

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "waiting", "{r}");
    assert!(r["reason"].as_str().unwrap().contains("t3cm took back GitLab's auto-merge"), "{r}");
    assert_eq!(ops(&h), ["merge", "cancelAutoMerge"]);
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["merge_when_pipeline_succeeds"], false);
    assert_eq!(h.app.store.merge_wait_get(KEY, 7).unwrap().unwrap().status, WaitStatus::Waiting);
}

#[tokio::test]
async fn an_older_gitlab_cancels_through_the_merge_when_pipeline_succeeds_route() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.patch_after("pipelines", json!({"detailed_merge_status": "ci_still_running"}));
    h.gitlab.fail_rest("/cancel_auto_merge", 404);

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert!(r["reason"].as_str().unwrap().contains("t3cm took back GitLab's auto-merge"), "{r}");
    let paths: Vec<String> = writes(&h).iter().map(|c| c["path"].as_str().unwrap().to_string()).collect();
    assert!(paths[2].ends_with("/merge_requests/7/cancel_merge_when_pipeline_succeeds"), "{paths:?}");
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["merge_when_pipeline_succeeds"], false);
}

#[tokio::test]
async fn a_cancel_that_finds_nothing_armed_leaves_the_approval_with_t3cm() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.patch_after("pipelines", json!({"detailed_merge_status": "ci_still_running"}));
    h.gitlab.fail_rest("/cancel_auto_merge", 409);

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert!(r["reason"].as_str().unwrap().contains("t3cm took back GitLab's auto-merge"), "{r}");
}

#[tokio::test]
async fn a_cancel_gitlab_answers_with_an_error_status_says_gitlab_will_merge_on_its_own() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.patch_after("pipelines", json!({"detailed_merge_status": "ci_still_running", "cancel_fails": true}));
    h.gitlab.fail_rest("/cancel_auto_merge", 404);

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "waiting", "{r}");
    assert!(r["reason"].as_str().unwrap().contains("GitLab will merge it on its own"), "{r}");
}

#[tokio::test]
async fn a_merge_gitlab_keeps_answering_405_stays_waiting_in_the_reconcile() {
    let h = setup();
    open_mr(&h, "ci_still_running", "running");
    assert_eq!(h.wf(TICKET, "merge", merge(KEY)).await["status"], "waiting");
    h.gitlab.set_pipeline(HOST, PATH, 7, "success");
    h.gitlab.update_mr(HOST, PATH, 7, json!({"detailed_merge_status": "mergeable"}));
    h.gitlab.fail_rest("/merge_requests/7/merge", 405);

    for _ in 0..4 {
        h.app.wf.reconcile().await.unwrap();
    }

    let w = h.app.store.merge_wait_get(KEY, 7).unwrap().expect("the approval is kept");
    assert_eq!((w.status, w.failures), (WaitStatus::Waiting, 0));
    assert!(w.last_error.unwrap().starts_with("GitLab cannot merge it yet"));
    assert!(h.app.wf.cards().unwrap().is_empty(), "no would-not-merge card");
}

#[tokio::test]
async fn a_train_that_schedules_the_merge_request_leaves_it_waiting() {
    let h = setup();
    open_mr(&h, "mergeable", "success");
    h.gitlab.set_trains(HOST, PATH, true);
    h.gitlab.patch_after("pipelines", json!({"detailed_merge_status": "ci_still_running"}));

    let r = h.wf(TICKET, "merge", merge(KEY)).await;

    assert_eq!(r["status"], "waiting", "{r}");
    assert!(r["reason"].as_str().unwrap().starts_with("GitLab scheduled it to join the merge train"), "{r}");
    assert_eq!(ops(&h), ["joinTrain", "cancelAutoMerge"]);
}

#[tokio::test]
async fn gitlab_merge_answers_map_to_wf_merge_outcomes() {
    let cases: [(u16, &str, &str); 6] = [
        (405, "waiting", "GitLab cannot merge it yet"),
        (409, "refused", "head moved: GitLab no longer has the merge request at 1111111"),
        (422, "refused", "GitLab could not merge it"),
        (401, "refused", "t3cm's GitLab login has no permission to merge it (HTTP 401"),
        (400, "refused", "GitLab refused the merge, and may require the head sha"),
        (500, "refused", "GitLab answered HTTP 500"),
    ];
    for (status, outcome, reason) in cases {
        let h = setup();
        open_mr(&h, "mergeable", "success");
        h.gitlab.fail_rest("/merge_requests/7/merge", status);

        let r = h.wf(TICKET, "merge", merge(KEY)).await;

        assert_eq!(r["status"], outcome, "HTTP {status}: {r}");
        assert!(r["reason"].as_str().unwrap_or_default().starts_with(reason), "HTTP {status}: {r}");
        assert_eq!(h.app.store.merge_wait_get(KEY, 7).unwrap().is_some(), outcome == "waiting", "HTTP {status}");
    }
}

#[tokio::test]
async fn wf_merge_refuses_merge_requests_it_cannot_merge() {
    let h = setup();
    open_mr(&h, "conflict", "success");
    let r = h.wf(TICKET, "merge", merge(KEY)).await;
    assert_eq!(r["status"], "refused", "{r}");
    assert!(r["reason"].as_str().unwrap().contains("merge state DIRTY"), "{r}");

    h.gitlab.update_mr(HOST, PATH, 7, json!({"detailed_merge_status": "mergeable", "sha": MOVED}));
    let r = h.wf(TICKET, "merge", merge(KEY)).await;
    assert_eq!(r["reason"], format!("head moved: the merge request is at {MOVED}"), "{r}");

    h.gitlab.close(HOST, PATH, 7, T0);
    let r = h.wf(TICKET, "merge", merge(KEY)).await;
    assert_eq!(r["reason"], "the merge request is closed", "{r}");

    let r = h.wf(TICKET, "merge", json!({"repo": KEY, "pr": 9, "head": HEAD})).await;
    assert_eq!(r["reason"], format!("{KEY}#9 is not a merge request"), "{r}");
    assert!(ops(&h).is_empty(), "{:?}", ops(&h));

    let err = h.wf_err(TICKET, "merge", json!({"repo": "other.example/acme/widgets", "pr": 7, "head": HEAD})).await;
    assert!(err.contains("wf.merge: other.example is not listed under forges"), "{err}");
}

#[tokio::test]
async fn a_board_merge_answer_merges_the_merge_request_and_hands_the_worker_off() {
    let h = setup();
    ticket_and_worker(&h).await;
    open_mr(&h, "mergeable", "success");
    let opened = json!({"event": "opened", "repo": URL, "head": HEAD, "checks": "pending",
                        "publishing": "Publishing: pr skill; passes: none configured"});
    assert_eq!(h.wf(WORKER, "pr", opened).await["status"], "open");
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": KEY, "pr": 7, "head": HEAD})).await;
    assert_eq!(r["status"], "gated", "{r}");
    let card = r["card"].as_str().unwrap().to_string();

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "merged", "{out}");
    assert_eq!(writes(&h)[0]["body"]["squash"], true, "the brief's squash method reaches GitLab");
    let commit = out["merge"]["commit"].as_str().unwrap().to_string();
    let worker = to(&h, WORKER);
    assert_eq!(worker.len(), 1, "{worker:?}");
    assert!(worker[0].contains(&format!("Merged #7 at {commit}. Hand off.")), "{}", worker[0]);
    assert!(h.app.wf.cards().unwrap().is_empty());
    let handled = digest(&h).await.join("\n");
    assert!(handled.contains(&format!("{URL} merged at")), "{handled}");
    assert!(handled.contains("found branch fix/7 already deleted on GitLab"), "{handled}");
}

#[tokio::test]
async fn the_source_branch_is_deleted_only_at_the_merged_head() {
    let h = setup();
    ticket_and_worker(&h).await;
    open_mr(&h, "mergeable", "success");
    let opened = json!({"event": "opened", "repo": URL, "head": HEAD, "checks": "pending",
                        "publishing": "Publishing: pr skill; passes: none configured"});
    h.wf(WORKER, "pr", opened).await;
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": KEY, "pr": 7, "head": HEAD})).await;
    let card = r["card"].as_str().unwrap().to_string();
    h.gitlab.set_trains(HOST, PATH, true);

    let out = h.app.wf.answer(&card, "merge").await.unwrap();
    assert_eq!(out["merge"]["status"], "queued", "{out}");
    assert_eq!(writes(&h)[0]["body"]["squash"], true, "the brief's squash method reaches the train");

    h.gitlab.merge(HOST, PATH, 7, &"c".repeat(40), T0);
    h.gitlab.update_mr(HOST, PATH, 7, json!({"branch_tip": MOVED}));
    h.app.wf.reconcile().await.unwrap();

    assert!(!ops(&h).contains(&"deleteBranch".to_string()), "{:?}", ops(&h));
    let handled = digest(&h).await.join("\n");
    assert!(handled.contains("left branch fix/7, as its tip 2222222 is not the merged head"), "{handled}");
}

#[tokio::test]
async fn a_train_merge_deletes_the_source_branch_afterwards() {
    let h = setup();
    ticket_and_worker(&h).await;
    open_mr(&h, "mergeable", "success");
    let opened = json!({"event": "opened", "repo": URL, "head": HEAD, "checks": "pending",
                        "publishing": "Publishing: pr skill; passes: none configured"});
    h.wf(WORKER, "pr", opened).await;
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": KEY, "pr": 7, "head": HEAD})).await;
    let card = r["card"].as_str().unwrap().to_string();
    h.gitlab.set_trains(HOST, PATH, true);
    assert_eq!(h.app.wf.answer(&card, "merge").await.unwrap()["merge"]["status"], "queued");

    h.gitlab.merge(HOST, PATH, 7, &"c".repeat(40), T0);
    h.app.wf.reconcile().await.unwrap();

    let delete = writes(&h).into_iter().find(|c| c["operation"] == "deleteBranch").expect("a branch delete");
    assert_eq!(delete["path"], "projects/acme%2Fplatform%2Fwidgets/repository/branches/fix%2F7");
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["branch_deleted"], true);
    assert!(to(&h, WORKER).iter().any(|m| m.contains(&format!("Merged #7 at {}. Hand off.", "c".repeat(40)))));
    let handled = digest(&h).await.join("\n");
    assert!(handled.contains("deleted branch fix/7"), "{handled}");
}

async fn digest(h: &H) -> Vec<String> {
    h.wf(WORKER, "send", json!({"kind": "event", "body": "wake the ticket"})).await;
    let woke = to(h, TICKET);
    let last = woke.last().expect("a ticket wake");
    let notes = last.split_once("Notes since your last wake").map(|(_, d)| d).unwrap_or_default();
    let mut out = Vec::new();
    for line in notes.lines().filter(|l| l.starts_with("- handled for you")) {
        let handle = line.rsplit_once('[').map(|(_, h)| h.trim_end_matches(']')).unwrap_or_default();
        out.push(h.wf(TICKET, "show", json!({"handle": handle})).await["body"].as_str().unwrap_or(line).to_string());
    }
    out
}
