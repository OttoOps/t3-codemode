mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{github_pr, pull_request};
use t3_codemode::server;

const REPO: &str = "acme/widgets";
const URL: &str = "https://github.com/acme/widgets/pull/7";
const HEAD: &str = "1111111111111111111111111111111111111111";
const MOVED: &str = "2222222222222222222222222222222222222222";
const MERGED: &str = "3333333333333333333333333333333333333333";

async fn ticket_with_worker(h: &H, minutes: i64) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 7, "open")]);
    h.tick(minutes).await;
}

async fn merge_gate(h: &H, head: Option<&str>) {
    let tail = head.map(|s| format!("\n\nPR 1/1 ready: head {}, checks passed", &s[..7])).unwrap_or_default();
    let body = format!("**ACTION NEEDED:** reply \"merge\" to merge #7, here: {URL}{tail}");
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": body})).await;
}

async fn check(h: &H, minutes: i64) -> Vec<String> {
    h.clock.set(&at(minutes));
    h.app.wf.reconcile().await.unwrap()
}

async fn wf_merge(h: &H, pr: u64, head: &str) -> Value {
    let code = format!("async () => wf.merge({})", json!({"repo": REPO, "pr": pr, "head": head}));
    result(&h.exec_as(TICKET, &code).await)
}

fn cards(h: &H, all: bool) -> Vec<Value> {
    h.app.wf.cards_with(all).unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn github_events(h: &H) -> Vec<String> {
    h.sent()
        .into_iter()
        .filter(|s| s["threadId"] == TICKET)
        .filter_map(|s| s["message"].as_str().map(String::from))
        .filter(|m| m.contains("[t3cm event from=github"))
        .collect()
}

fn reconciles(h: &H) -> usize {
    h.github.operations().iter().filter(|op| *op == "reconcile").count()
}

fn feed_heads(h: &H) -> Vec<String> {
    h.app.wf.feed(50).unwrap().iter().map(|f| f["head"].as_str().unwrap_or_default().to_string()).collect()
}

fn worker_pr_state(h: &H) -> Value {
    h.app.wf.tree().unwrap()[0]["workers"][0]["pr"]["state"].clone()
}

#[tokio::test]
async fn a_pr_merged_by_hand_resolves_its_gate_card_and_wakes_the_ticket_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    assert!(check(&h, 0).await.is_empty());
    assert_eq!(cards(&h, false).len(), 1);

    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(4), "closedAt": at(4)}),
    );
    h.clock.set(&at(10));
    h.app.wf.board_tick().await.unwrap();

    assert!(cards(&h, false).is_empty());
    let resolution = cards(&h, true)[0]["resolution"].as_str().unwrap().to_string();
    assert!(resolution.starts_with("merged on GitHub at 3333333, "), "{resolution}");
    let events = github_events(&h);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains("acme/widgets#7 merged on GitHub at 3333333"), "{}", events[0]);
    assert!(events[0].contains("post-merge"), "{}", events[0]);
    assert!(feed_heads(&h).iter().any(|f| f.starts_with("acme/widgets#7 merged on GitHub")));

    assert!(check(&h, 20).await.is_empty());
    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_pr_closed_without_merging_resolves_its_card_and_tells_the_ticket() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    merge_gate(&h, None).await;
    check(&h, 0).await;

    h.github.update_pr(REPO, 7, json!({"state": "CLOSED", "closedAt": at(3)}));
    let actions = check(&h, 5).await;

    assert_eq!(actions.len(), 1, "{actions:?}");
    let resolution = cards(&h, true)[0]["resolution"].as_str().unwrap().to_string();
    assert!(resolution.starts_with("closed on GitHub without merging"), "{resolution}");
    let events = github_events(&h);
    assert_eq!(events.len(), 1);
    assert!(events[0].contains("acme/widgets#7 was closed on GitHub without merging"), "{}", events[0]);
    assert!(check(&h, 15).await.is_empty());
    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_pr_dropped_from_the_queue_opens_one_card_with_the_reason() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
    check(&h, 1).await;
    assert!(cards(&h, false).is_empty());

    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(2));
    check(&h, 3).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["kind"], "report");
    assert_eq!(open[0]["ticket"], TICKET);
    assert!(open[0]["text"].as_str().unwrap().contains("dropped from the merge queue (failed checks)"), "{}", open[0]);
    assert_eq!(open[0]["options"], json!(["requeue", "hold"]));
    assert!(feed_heads(&h).iter().any(|f| f.contains("dropped from the merge queue: failed checks")));

    check(&h, 5).await;
    assert_eq!(cards(&h, false).len(), 1);
}

#[tokio::test]
async fn a_removal_that_predates_tracking_opens_no_card() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    let mut pr = github_pr(HEAD, "BLOCKED", true);
    pr["timelineItems"] = json!({"nodes": [{"reason": "merge_conflict", "createdAt": at(-60)}]});
    h.github.set_pr(REPO, 7, pr);

    check(&h, 0).await;
    check(&h, 10).await;

    assert!(cards(&h, false).is_empty());
}

#[tokio::test]
async fn a_waiting_pr_is_queued_once_it_is_clean_at_the_approved_head() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    check(&h, 2).await;
    assert_eq!(h.app.store.merge_waits().unwrap().len(), 1);

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    check(&h, 4).await;

    let mutations: Vec<String> = h.github.operations().into_iter().filter(|o| o.ends_with("PullRequest")).collect();
    assert_eq!(mutations, ["enqueuePullRequest"]);
    assert_eq!(h.github.calls().last().unwrap()["variables"]["head"], HEAD);
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    assert!(feed_heads(&h).iter().any(|f| f.starts_with("acme/widgets#7 queued at its approved head 1111111")));
    assert_eq!(worker_pr_state(&h), "queued");
    assert!(github_events(&h).is_empty());
}

#[tokio::test]
async fn a_waiting_pr_whose_head_moved_gets_a_new_gate_and_is_not_queued() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    wf_merge(&h, 7, HEAD).await;

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED, "mergeStateStatus": "CLEAN"}));
    check(&h, 2).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")), "{:?}", h.github.operations());
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["kind"], "gate");
    assert!(open[0]["text"].as_str().unwrap().contains("moved to head 2222222"), "{}", open[0]);
    assert_eq!(open[0]["options"], json!(["merge", "hold"]));
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));
}

#[tokio::test]
async fn a_merge_gate_whose_head_moved_is_replaced_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    let first = cards(&h, false)[0]["id"].clone();

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 10).await;

    let all = cards(&h, true);
    let old = all.iter().find(|c| c["resolved"].is_string()).unwrap();
    assert_eq!(old["id"], first);
    assert!(old["resolution"].as_str().unwrap().starts_with("head moved to 2222222"), "{old}");
    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert!(open[0]["text"].as_str().unwrap().contains("moved to head 2222222"));

    check(&h, 20).await;
    assert_eq!(cards(&h, true).len(), 2);
}

#[tokio::test]
async fn a_gate_that_names_no_head_is_held_to_the_head_first_seen() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, None).await;
    check(&h, 0).await;
    check(&h, 10).await;
    assert_eq!(cards(&h, true).len(), 1);

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 20).await;
    assert_eq!(cards(&h, true).len(), 2);
}

#[tokio::test]
async fn one_batched_query_checks_every_tracked_pr() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", false));
    h.github.set_pr(REPO, 8, github_pr(HEAD, "BLOCKED", true));
    h.github.set_pr("acme/gadgets", 9, github_pr(HEAD, "CLEAN", true));
    wf_merge(&h, 8, HEAD).await;
    let gate = "**ACTION NEEDED:** reply \"merge\" to merge it, here: https://github.com/acme/gadgets/pull/9";
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": gate})).await;

    check(&h, 1).await;

    assert_eq!(reconciles(&h), 1);
    let vars = h.github.calls().last().unwrap()["variables"].clone();
    let mut prs: Vec<(String, u64)> = (0..3)
        .map(|i| {
            let repo =
                format!("{}/{}", vars[format!("o{i}")].as_str().unwrap(), vars[format!("r{i}")].as_str().unwrap());
            (repo, vars[format!("n{i}")].as_u64().unwrap())
        })
        .collect();
    prs.sort();
    assert_eq!(prs, [("acme/gadgets".into(), 9), (REPO.into(), 7), (REPO.into(), 8)]);
    assert!(vars.get("o3").is_none());
}

#[tokio::test]
async fn nothing_tracked_means_no_github_call() {
    let h = H::new();
    assert!(check(&h, 0).await.is_empty());
    assert!(h.github.calls().is_empty());
}

#[tokio::test]
async fn the_check_runs_every_ten_minutes_and_every_two_while_a_pr_waits() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    let due = |m: i64| {
        let h = &h;
        async move {
            h.clock.set(&at(m));
            h.app.wf.reconcile_if_due().await.unwrap();
            reconciles(h)
        }
    };
    assert_eq!(due(0).await, 1);
    assert_eq!(due(5).await, 1);
    assert_eq!(due(10).await, 2);
    wf_merge(&h, 7, HEAD).await;
    assert_eq!(due(11).await, 2);
    assert_eq!(due(12).await, 3);
    assert_eq!(due(13).await, 3);
    assert_eq!(due(14).await, 4);
}

#[tokio::test]
async fn reconcile_can_be_turned_off() {
    let config = Config::from_value(json!({"board": {"reconcileIdleMinutes": 0}})).unwrap();
    let h = H::build(Vec::new(), config);
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    h.app.wf.reconcile_if_due().await.unwrap();
    h.app.wf.reconcile_soon().await.unwrap();
    assert_eq!(reconciles(&h), 0);
    let bad = Config::from_value(json!({"board": {"reconcileActiveMinutes": 20}})).unwrap_err();
    assert!(format!("{bad:#}").contains("reconcileActiveMinutes"), "{bad:#}");
}

#[tokio::test]
async fn a_pr_merged_by_wf_merge_does_not_wake_the_ticket_again() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", false));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "merged");
    check(&h, 1).await;
    check(&h, 11).await;
    assert!(github_events(&h).is_empty());
}

#[tokio::test]
async fn a_queued_pr_that_merges_wakes_its_ticket_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    wf_merge(&h, 7, HEAD).await;
    check(&h, 1).await;
    assert_eq!(worker_pr_state(&h), "queued");

    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": MERGED}, "mergedAt": at(2),
               "timelineItems": {"nodes": [{"reason": "merged", "createdAt": at(2)}]}}),
    );
    check(&h, 3).await;
    check(&h, 5).await;

    assert_eq!(github_events(&h).len(), 1);
    assert!(cards(&h, false).is_empty());
}

#[tokio::test]
async fn a_pr_already_merged_when_first_seen_with_no_card_stays_quiet() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    let mut pr = github_pr(HEAD, "UNKNOWN", true);
    pr["state"] = json!("MERGED");
    pr["mergeCommit"] = json!({"oid": MERGED});
    h.github.set_pr(REPO, 7, pr);
    check(&h, 0).await;
    assert!(github_events(&h).is_empty());
    assert!(feed_heads(&h).iter().all(|f| !f.contains("merged on GitHub")));
}

#[tokio::test]
async fn a_waiting_pr_github_refuses_to_queue_becomes_a_card() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    wf_merge(&h, 7, HEAD).await;
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    h.github.fail_operation("enqueuePullRequest", "Pull request is in clean status but is not mergeable");

    check(&h, 2).await;

    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert!(open[0]["text"].as_str().unwrap().contains("GitHub would not queue acme/widgets#7"), "{}", open[0]);
    assert!(open[0]["text"].as_str().unwrap().contains("is not mergeable"), "{}", open[0]);
}

#[tokio::test]
async fn github_down_changes_nothing_and_the_board_pass_still_runs() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    h.github.set_down(true);

    assert!(h.app.wf.reconcile().await.is_err());
    h.clock.set(&at(10));
    h.app.wf.board_tick().await.unwrap();

    assert_eq!(cards(&h, false).len(), 1);
    assert!(github_events(&h).is_empty());
}

#[tokio::test]
async fn loading_the_board_page_runs_a_check() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    let github = h.github.clone();
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = h.app.clone();
    tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let count = || github.operations().iter().filter(|o| *o == "reconcile").count();

    assert_eq!(http.get(format!("http://{addr}/")).send().await.unwrap().status(), 200);
    for _ in 0..100 {
        if count() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(count(), 1);
    http.get(format!("http://{addr}/")).send().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count(), 1);
    let _ = tx.send(());
}

#[tokio::test]
async fn a_merged_pr_that_leaves_tracking_does_not_fire_again_for_a_later_card() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(4)}));
    check(&h, 5).await;
    assert_eq!(github_events(&h).len(), 1);

    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "merged"})).await;
    check(&h, 15).await;
    h.clock.set(&at(20));
    let deploy = format!("**ACTION NEEDED:** reply \"deploy\" to deploy #7, here: {URL}");
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "deploy", "body": deploy})).await;
    check(&h, 25).await;

    assert_eq!(github_events(&h).len(), 1);
    assert_eq!(cards(&h, false).len(), 1);
}

#[tokio::test]
async fn a_pr_queued_outside_t3_still_wakes_its_ticket_after_the_worker_hands_off() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 2).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;

    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": MERGED}}));
    check(&h, 4).await;

    assert_eq!(github_events(&h).len(), 1);
}

#[test]
fn only_github_hosted_pr_links_with_an_int_sized_number_are_tracked() {
    let one = vec![("acme".to_string(), "widgets".to_string(), 7)];
    assert_eq!(t3_codemode::reconcile::pr_refs(URL), one);
    assert_eq!(t3_codemode::reconcile::pr_refs("see https://www.github.com/acme/widgets/pull/7."), one);
    assert_eq!(t3_codemode::reconcile::pr_refs("merge github.com/acme/widgets/pull/7 now"), one);
    assert!(t3_codemode::reconcile::pr_refs("https://example.org/github.com/acme/widgets/pull/7").is_empty());
    assert!(t3_codemode::reconcile::pr_refs("https://notgithub.com/acme/widgets/pull/7").is_empty());
    assert!(t3_codemode::reconcile::pr_refs("https://github.com/acme/widgets/pull/2147483648").is_empty());
}

#[tokio::test]
async fn a_wf_merge_refused_for_an_older_head_does_not_swallow_the_merge_event() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;

    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "headRefOid": MOVED, "mergeCommit": {"oid": MERGED}, "mergedAt": at(4)}),
    );
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "refused");
    check(&h, 5).await;

    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_moved_head_waits_for_a_free_board_slot_and_then_gets_one_gate() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    for i in 0..36 {
        let body = format!("**ACTION NEEDED:** filler {i}");
        if h.app.wf.dispatch("send", Some(TICKET), json!({"kind": "report", "body": body})).await.is_err() {
            break;
        }
    }
    let full = cards(&h, false);
    assert_eq!(full.len(), 36);

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.clock.set(&at(2));
    let err = h.app.wf.reconcile().await.unwrap_err();
    assert!(format!("{err:#}").contains("full"), "{err:#}");
    assert_eq!(h.app.store.merge_waits().unwrap().len(), 1);

    h.app.wf.dismiss(full[0]["id"].as_str().unwrap()).unwrap();
    check(&h, 4).await;

    let gates: Vec<Value> = cards(&h, false).into_iter().filter(|c| c["kind"] == "gate").collect();
    assert_eq!(gates.len(), 1, "{gates:?}");
    assert!(gates[0]["text"].as_str().unwrap().contains("moved to head 2222222"), "{}", gates[0]);
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    check(&h, 6).await;
    assert_eq!(cards(&h, false).iter().filter(|c| c["kind"] == "gate").count(), 1);
}

#[tokio::test]
async fn a_dropped_pr_no_ticket_tracks_gets_a_card_with_no_reply_buttons() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let out = h.app.wf.dispatch("merge", None, json!({"repo": REPO, "pr": 7, "head": HEAD})).await.unwrap();
    assert_eq!(out["status"], "queued");
    check(&h, 1).await;

    h.github.drop_from_queue(REPO, 7, "merge_conflict", &at(2));
    check(&h, 3).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["options"], json!([]));
    let text = open[0]["text"].as_str().unwrap();
    assert!(text.contains("dropped from the merge queue (a merge conflict)"), "{text}");
    assert!(text.contains("no ticket tracks it"), "{text}");
    assert!(!text.contains("requeue"), "{text}");
}
