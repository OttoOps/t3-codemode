use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::codehost::ChangeRef;
use t3_codemode::config::Config;
use t3_codemode::fake::{github_pr, gitlab_mr, merge_request, pull_request};
use t3_codemode::reconcile::change_refs;

const HOST: &str = "gitlab.acme.example";
const PATH: &str = "acme/platform/widgets";
const URL: &str = "https://gitlab.acme.example/acme/platform/widgets/-/merge_requests/7";
const HEAD: &str = "1111111111111111111111111111111111111111";
const SQUASHED: &str = "5555555555555555555555555555555555555555";
const MERGED: &str = "3333333333333333333333333333333333333333";

fn config(forges: Value) -> Config {
    Config::from_value(json!({"forges": forges})).unwrap()
}

fn gitlab_h() -> H {
    H::build(Vec::new(), config(json!({HOST: {"kind": "gitlab"}})))
}

async fn ticket_with_worker(h: &H) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.fake.set_prs(WORKER, vec![merge_request(HOST, PATH, 7, "open")]);
    h.tick(0).await;
}

async fn check(h: &H, minutes: i64) -> Vec<String> {
    h.clock.set(&at(minutes));
    h.app.wf.reconcile().await.unwrap()
}

fn events(h: &H, from: &str) -> Vec<String> {
    h.sent()
        .into_iter()
        .filter(|s| s["threadId"] == TICKET)
        .filter_map(|s| s["message"].as_str().map(String::from))
        .filter(|m| m.contains(&format!("[t3cm event from={from}")))
        .collect()
}

fn open_mr(h: &H, squash: bool) {
    let mut mr = gitlab_mr(HEAD, "ci_still_running");
    mr["squash"] = json!(squash);
    mr["squash_on_merge"] = json!(squash);
    h.gitlab.set_mr(HOST, PATH, 7, mr);
    h.gitlab.set_pipeline(HOST, PATH, 7, "running");
}

fn worker_pr(h: &H) -> Value {
    h.app.wf.tree().unwrap()[0]["workers"][0]["pr"].clone()
}

#[test]
fn forges_name_gitlab_hosts_and_refuse_github() {
    let c = config(json!({"GitLab.Acme.Example": {"kind": "gitlab"}}));
    assert_eq!(c.forges["GitLab.Acme.Example"].cli, "glab");
    let bad = [
        (json!({"github.com": {"kind": "gitlab"}}), "github.com is built in"),
        (json!({"https://gitlab.acme.example": {"kind": "gitlab"}}), "without scheme"),
        (json!({HOST: {"kind": "gitlab"}, "GITLAB.acme.example": {"kind": "gitlab"}}), "same host"),
        (json!({HOST: {"kind": "gitlab", "cli": " "}}), "must name the CLI"),
        (json!({HOST: {"kind": "gitea"}}), "unknown variant"),
        (json!({HOST: {"kind": "gitlab", "token": "x"}}), "unknown field"),
    ];
    for (forges, want) in bad {
        let err = format!("{:#}", Config::from_value(json!({"forges": forges.clone()})).unwrap_err());
        assert!(err.contains(want), "{forges}: {err}");
    }
}

#[test]
fn merge_request_links_on_a_listed_host_are_tracked_with_their_full_path() {
    let forges = config(json!({HOST: {"kind": "gitlab"}})).forges;
    let text = format!(
        "see {URL}. and https://github.com/acme/widgets/pull/7, {URL}/diffs, \
         https://other.example/acme/app/-/merge_requests/3, https://gitlab.acme.example/solo/-/merge_requests/4, \
         https://evil.example/gitlab.acme.example/acme/app/-/merge_requests/5, \
         https://GITLAB.acme.example/Acme/App/-/merge_requests/6, \
         https://www.gitlab.acme.example/acme/app/-/merge_requests/8"
    );
    let refs = change_refs(&text, &forges);
    assert_eq!(
        refs,
        [
            ChangeRef::github("acme/widgets", 7),
            ChangeRef::gitlab(HOST, PATH, 7),
            ChangeRef::gitlab(HOST, "acme/app", 6)
        ]
    );
    assert_eq!(refs[1].key(), "gitlab.acme.example/acme/platform/widgets");
    assert_eq!(ChangeRef::of_key(&refs[1].key(), 7), refs[1]);
    assert_eq!(ChangeRef::of_key("acme/widgets", 7), refs[0]);
    assert!(change_refs(URL, &Config::default().forges).is_empty());
}

#[tokio::test]
async fn a_squash_merge_through_the_merge_train_wakes_the_ticket_with_the_squash_commit() {
    let h = gitlab_h();
    ticket_with_worker(&h).await;
    open_mr(&h, true);
    assert!(check(&h, 1).await.is_empty());
    assert_eq!(worker_pr(&h)["readiness"]["checks"], "pending");

    h.gitlab.set_pipeline(HOST, PATH, 7, "success");
    h.gitlab.join_train(HOST, PATH, 7, 0);
    check(&h, 3).await;
    assert_eq!(worker_pr(&h)["state"], "queued");
    assert_eq!(worker_pr(&h)["readiness"]["checks"], "passing", "judged on the MR pipeline, not the train's");

    h.gitlab.merge(HOST, PATH, 7, SQUASHED, &at(6));
    assert_eq!(h.gitlab.mr(HOST, PATH, 7)["merge_commit_sha"], Value::Null);
    check(&h, 8).await;

    let got = events(&h, "gitlab");
    assert_eq!(got.len(), 1, "{got:?}");
    let want = format!("{HOST}/{PATH}#7 merged on GitLab at 5555555");
    assert!(got[0].contains(&want), "{}", got[0]);
    assert!(got[0].contains(&format!("(merge commit {SQUASHED})")), "{}", got[0]);
    assert!(got[0].contains(URL), "{}", got[0]);
    assert!(events(&h, "github").is_empty());

    check(&h, 20).await;
    assert_eq!(events(&h, "gitlab").len(), 1, "the merge wakes the ticket once");
}

#[tokio::test]
async fn a_train_merge_with_a_merge_commit_reports_that_commit_without_a_rest_read() {
    let h = gitlab_h();
    ticket_with_worker(&h).await;
    open_mr(&h, false);
    h.gitlab.set_pipeline(HOST, PATH, 7, "success");
    h.gitlab.join_train(HOST, PATH, 7, 2);
    check(&h, 1).await;

    h.gitlab.merge(HOST, PATH, 7, MERGED, &at(4));
    let before = h.gitlab.operations().len();
    check(&h, 5).await;

    let got = events(&h, "gitlab");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains(&format!("(merge commit {MERGED})")), "{}", got[0]);
    assert_eq!(h.gitlab.operations()[before..], ["reconcile"]);
}

#[tokio::test]
async fn a_squash_merge_by_hand_reads_the_squash_commit_over_rest() {
    let h = gitlab_h();
    ticket_with_worker(&h).await;
    open_mr(&h, true);
    check(&h, 1).await;

    h.gitlab.merge(HOST, PATH, 7, SQUASHED, &at(4));
    check(&h, 5).await;

    let got = events(&h, "gitlab");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains(&format!("(merge commit {SQUASHED})")), "{}", got[0]);
    let calls = h.gitlab.calls();
    let read = calls.iter().find(|c| c["operation"] == "mergeRequest").unwrap();
    assert_eq!(read["path"], "projects/acme%2Fplatform%2Fwidgets/merge_requests/7");
    assert_eq!(read["host"], HOST);
    assert_eq!(read["cli"], "glab");
}

#[tokio::test]
async fn a_closed_merge_request_wakes_the_ticket_once_at_its_closing_time() {
    let h = gitlab_h();
    ticket_with_worker(&h).await;
    open_mr(&h, false);
    check(&h, 1).await;

    h.gitlab.close(HOST, PATH, 7, &at(3));
    check(&h, 5).await;
    check(&h, 15).await;

    let got = events(&h, "gitlab");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains(&format!("{HOST}/{PATH}#7 was closed on GitLab without merging")), "{}", got[0]);
}

#[tokio::test]
async fn the_same_repo_and_number_on_two_hosts_are_tracked_apart() {
    let h = H::build(Vec::new(), config(json!({HOST: {"kind": "gitlab", "cli": "glab-acme"}})));
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.fake
        .set_prs(WORKER, vec![pull_request("acme/widgets", 7, "open"), merge_request(HOST, "acme/widgets", 7, "open")]);
    h.tick(0).await;
    h.github.set_pr("acme/widgets", 7, github_pr(HEAD, "CLEAN", false));
    h.gitlab.set_mr(HOST, "acme/widgets", 7, gitlab_mr(HEAD, "mergeable"));
    check(&h, 1).await;

    h.gitlab.merge(HOST, "acme/widgets", 7, MERGED, &at(3));
    check(&h, 5).await;

    let got = events(&h, "gitlab");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains(&format!("{HOST}/acme/widgets#7 merged on GitLab")), "{}", got[0]);
    assert!(events(&h, "github").is_empty(), "the GitHub pull request is still open");
    assert!(h.gitlab.calls().iter().all(|c| c["cli"] == "glab-acme"));

    h.github.update_pr(
        "acme/widgets",
        7,
        json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(6)}),
    );
    check(&h, 7).await;
    assert_eq!(events(&h, "github").len(), 1);
    assert_eq!(events(&h, "gitlab").len(), 1);
}

#[tokio::test]
async fn glab_failing_leaves_github_reconciling_and_names_the_merge_request() {
    let h = gitlab_h();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 7, "open"), merge_request(HOST, PATH, 7, "open")]);
    h.tick(0).await;
    h.github.set_pr("acme/widgets", 7, github_pr(HEAD, "CLEAN", false));
    open_mr(&h, false);
    h.app.wf.reconcile().await.unwrap();

    h.gitlab.set_down(true);
    h.github.update_pr(
        "acme/widgets",
        7,
        json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(2)}),
    );
    h.clock.set(&at(3));
    let err = format!("{:#}", h.app.wf.reconcile().await.unwrap_err());

    assert!(err.contains(&format!("{HOST}/{PATH}#7")) && err.contains("not authenticated"), "{err}");
    assert_eq!(events(&h, "github").len(), 1);
}

#[tokio::test]
async fn a_merge_request_on_an_unlisted_host_is_not_tracked() {
    let h = H::new();
    ticket_with_worker(&h).await;
    open_mr(&h, false);
    check(&h, 1).await;
    h.gitlab.merge(HOST, PATH, 7, MERGED, &at(2));
    check(&h, 3).await;
    assert!(h.gitlab.calls().is_empty());
    assert!(events(&h, "gitlab").is_empty());
}

#[tokio::test]
async fn a_merge_request_gone_from_gitlab_is_dropped_from_tracking() {
    let h = gitlab_h();
    ticket_with_worker(&h).await;
    open_mr(&h, false);
    check(&h, 1).await;
    assert_eq!(worker_pr(&h)["readiness"]["mergeStateStatus"], "BLOCKED");

    h.gitlab.remove_mr(HOST, PATH, 7);
    check(&h, 3).await;
    assert_eq!(worker_pr(&h)["readiness"], Value::Null);
    assert!(events(&h, "gitlab").is_empty());
}
