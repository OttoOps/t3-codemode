mod common;

use common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::fake::github_pr;
use t3_codemode::github::{errors, gh_response};

const REPO: &str = "acme/widgets";
const HEAD: &str = "1111111111111111111111111111111111111111";
const MOVED: &str = "2222222222222222222222222222222222222222";

async fn merge(h: &H, args: Value) -> Value {
    let code = format!("async () => wf.merge({args})");
    result(&h.exec_as(TICKET, &code).await)
}

fn args(pr: u64) -> Value {
    json!({"repo": REPO, "pr": pr, "head": HEAD})
}

fn wait_status(h: &H) -> Vec<Value> {
    h.app.store.merge_waits().unwrap().iter().map(|w| serde_json::to_value(w).unwrap()["status"].clone()).collect()
}

fn mutations(h: &H) -> Vec<String> {
    h.github.operations().into_iter().filter(|op| op != "read").collect()
}

#[tokio::test]
async fn a_clean_pr_on_a_merge_queue_repo_is_enqueued_at_the_approved_head() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.set_queue_length(2);

    let r = merge(&h, args(7)).await;

    assert_eq!(r, json!({"status": "queued", "repo": REPO, "pr": 7, "head": HEAD, "position": 3}));
    assert_eq!(h.github.operations(), ["read", "enqueuePullRequest"]);
    let enqueue = &h.github.calls()[1]["variables"];
    assert_eq!(enqueue["head"], HEAD);
    assert_eq!(enqueue["id"], h.github.pr(REPO, 7)["id"]);
    assert_eq!(wait_status(&h), ["sent"]);
}

#[tokio::test]
async fn a_merge_queue_pr_that_is_not_ready_waits_and_is_never_set_to_auto_merge() {
    let mut h = H::new();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#7", "role": "ticket"})).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));

    let r = merge(&h, json!({"repo": "Acme/Widgets", "pr": 7, "head": HEAD.to_uppercase()})).await;

    assert_eq!(r, json!({"status": "waiting", "repo": REPO, "pr": 7, "head": HEAD, "mergeState": "BLOCKED"}));
    assert!(mutations(&h).is_empty(), "no mutation while waiting: {:?}", h.github.operations());
    h.reopen();
    let waits = h.app.store.merge_waits().unwrap();
    assert_eq!(waits.len(), 1);
    let w = serde_json::to_value(&waits[0]).unwrap();
    assert_eq!(w["repo"], REPO);
    assert_eq!(w["pr"], 7);
    assert_eq!(w["head"], HEAD);
    assert_eq!(w["pullRequestId"], h.github.pr(REPO, 7)["id"]);
    assert_eq!(w["thread"], TICKET);
    assert_eq!(w["label"], "widgets#7");
    assert_eq!(w["mergeState"], "BLOCKED");
    assert_eq!(w["at"], T0);
}

#[tokio::test]
async fn a_waiting_pr_is_queued_and_its_record_marked_sent_once_it_is_clean() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "DIRTY", true));
    assert_eq!(merge(&h, json!({"repo": "ACME/widgets", "pr": 7, "head": HEAD})).await["status"], "waiting");
    assert_eq!(h.app.store.merge_waits().unwrap().len(), 1);

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    let r = merge(&h, args(7)).await;

    assert_eq!(r["status"], "queued");
    assert_eq!(r["position"], 1);
    assert_eq!(wait_status(&h), ["sent"]);
    assert_eq!(mutations(&h), ["enqueuePullRequest"]);
}

#[tokio::test]
async fn a_pr_already_in_the_queue_reports_its_position_without_a_mutation() {
    let h = H::new();
    let mut pr = github_pr(HEAD, "BLOCKED", true);
    pr["isInMergeQueue"] = json!(true);
    pr["mergeQueueEntry"] = json!({"position": 4});
    h.github.set_pr(REPO, 7, pr);

    let r = merge(&h, args(7)).await;

    assert_eq!(r["status"], "queued");
    assert_eq!(r["position"], 4);
    assert!(mutations(&h).is_empty());
}

#[tokio::test]
async fn without_a_merge_queue_the_pr_merges_with_the_repo_method_at_the_head() {
    let h = H::new();
    h.github.set_pr(REPO, 8, github_pr(HEAD, "CLEAN", false));
    h.github.set_default_method(REPO, "REBASE");

    let r = merge(&h, args(8)).await;

    assert_eq!(r["status"], "merged");
    assert_eq!(r["method"], "rebase");
    assert_eq!(r["commit"], h.github.pr(REPO, 8)["mergeCommit"]["oid"]);
    assert_eq!(h.github.operations(), ["read", "mergePullRequest"]);
    let vars = &h.github.calls()[1]["variables"];
    assert_eq!(vars["head"], HEAD);
    assert_eq!(vars["method"], "REBASE");
}

#[tokio::test]
async fn an_explicit_method_overrides_the_repo_default() {
    let h = H::new();
    h.github.set_pr(REPO, 8, github_pr(HEAD, "UNSTABLE", false));

    let r = merge(&h, json!({"repo": REPO, "pr": 8, "head": HEAD, "method": "squash"})).await;

    assert_eq!(r["status"], "merged");
    assert_eq!(h.github.calls()[1]["variables"]["method"], "SQUASH");
}

#[tokio::test]
async fn a_moved_head_is_refused_before_any_mutation() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(MOVED, "CLEAN", true));
    h.github.set_pr(REPO, 8, github_pr(MOVED, "CLEAN", false));

    for pr in [7, 8] {
        let r = merge(&h, args(pr)).await;
        assert_eq!(r["status"], "refused");
        assert!(r["reason"].as_str().unwrap().contains(MOVED), "{r}");
    }
    assert!(mutations(&h).is_empty());
    assert!(h.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
async fn prs_that_cannot_merge_are_refused_with_a_reason() {
    let h = H::new();
    let mut draft = github_pr(HEAD, "CLEAN", true);
    draft["isDraft"] = json!(true);
    h.github.set_pr(REPO, 1, draft);
    let mut closed = github_pr(HEAD, "CLEAN", false);
    closed["state"] = json!("CLOSED");
    h.github.set_pr(REPO, 2, closed);
    h.github.set_pr(REPO, 3, github_pr(HEAD, "DIRTY", false));
    h.github.set_pr(REPO, 4, github_pr(HEAD, "BLOCKED", false));

    let mut reasons = Vec::new();
    for pr in [1, 2, 3, 4] {
        let r = merge(&h, args(pr)).await;
        assert_eq!(r["status"], "refused", "{r}");
        reasons.push(r["reason"].as_str().unwrap().to_string());
    }

    assert!(reasons[0].contains("draft"), "{reasons:?}");
    assert!(reasons[1].contains("closed"), "{reasons:?}");
    assert!(reasons[2].contains("DIRTY"), "{reasons:?}");
    assert!(reasons[3].contains("BLOCKED"), "{reasons:?}");
    assert!(mutations(&h).is_empty());
    assert!(h.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
async fn an_already_merged_pr_reports_its_merge_commit() {
    let h = H::new();
    let mut pr = github_pr(HEAD, "UNKNOWN", true);
    pr["state"] = json!("MERGED");
    pr["mergeCommit"] = json!({"oid": MOVED});
    h.github.set_pr(REPO, 7, pr);

    let r = merge(&h, args(7)).await;

    assert_eq!(r, json!({"status": "merged", "repo": REPO, "pr": 7, "head": HEAD, "commit": MOVED}));
    assert_eq!(h.github.operations(), ["read"]);
}

#[tokio::test]
async fn a_github_refusal_of_the_mutation_comes_back_as_refused() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.fail_operation("enqueuePullRequest", "Pull request is not mergeable");

    let r = merge(&h, args(7)).await;

    assert_eq!(r["status"], "refused");
    assert_eq!(r["reason"], "Pull request is not mergeable");
}

#[tokio::test]
async fn an_unknown_repo_or_pr_is_refused() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));

    let r = merge(&h, json!({"repo": "acme/gadgets", "pr": 7, "head": HEAD})).await;
    assert_eq!(r["status"], "refused");
    assert!(r["reason"].as_str().unwrap().contains("Could not resolve"), "{r}");

    let r = merge(&h, args(99)).await;
    assert_eq!(r["status"], "refused");
    assert!(r["reason"].as_str().unwrap().contains("not a pull request"), "{r}");
}

#[tokio::test(start_paused = true)]
async fn an_unknown_merge_state_is_read_again_before_deciding() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.set_unknown_reads(2);

    let r = merge(&h, args(7)).await;

    assert_eq!(r["status"], "queued");
    assert_eq!(h.github.operations(), ["read", "read", "read", "enqueuePullRequest"]);
}

#[tokio::test]
async fn a_gh_failure_throws_instead_of_returning_an_outcome() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.set_down(true);

    let out = h.exec_as(TICKET, &format!("async () => wf.merge({})", args(7))).await;

    assert!(err_text(&out).contains("not logged in"), "{}", out.text);
}

#[tokio::test]
async fn bad_arguments_are_rejected_before_github_is_called() {
    let h = H::new();
    for bad in [
        json!({"repo": "widgets", "pr": 7, "head": HEAD}),
        json!({"repo": REPO, "pr": 0, "head": HEAD}),
        json!({"repo": REPO, "pr": 7, "head": "abc123"}),
        json!({"repo": REPO, "pr": 7, "head": HEAD, "method": "fast-forward"}),
        json!({"repo": REPO, "pr": 7, "head": HEAD, "auto": true}),
    ] {
        let msg = h.wf_err(TICKET, "merge", bad.clone()).await;
        assert!(msg.starts_with("wf.merge"), "{bad}: {msg}");
    }
    assert!(h.github.calls().is_empty());
}

#[tokio::test]
async fn wf_merge_needs_no_caller_thread() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));

    let r = result(&h.exec(&format!("async () => wf.merge({})", args(7))).await);

    assert_eq!(r["status"], "waiting");
    let w = serde_json::to_value(&h.app.store.merge_waits().unwrap()[0]).unwrap();
    assert_eq!(w["thread"], Value::Null);
    assert_eq!(w["label"], Value::Null);
}

#[tokio::test]
async fn the_catalog_documents_wf_merge() {
    let h = H::new();
    let out = h.app.search(r#"async () => catalog.find(t => t.name === "wf.merge")"#).await;
    let entry = result(&out);
    assert_eq!(entry["kind"], "wf");
    let sig = entry["signature"].as_str().unwrap();
    for part in ["wf.merge(input: {", "head: string", "\"queued\"", "\"merged\"", "\"waiting\"", "\"refused\""] {
        assert!(sig.contains(part), "{part} missing from {sig}");
    }
}

#[test]
fn gh_output_with_graphql_errors_still_parses() {
    let out = br#"{"data":{"repository":null},"errors":[{"message":"Could not resolve"}]}"#;
    let v = gh_response(false, out, b"gh: Could not resolve").unwrap();
    assert_eq!(errors(&v).as_deref(), Some("Could not resolve"));
    let e = gh_response(false, b"", b"gh: not logged in").unwrap_err();
    assert!(e.to_string().contains("not logged in"), "{e}");
}
