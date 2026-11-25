use std::time::Duration;

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{Failure, Rule, check_run, github_pr, project, pull_request};
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

fn wait_status(h: &H) -> Vec<Value> {
    h.app.store.merge_waits().unwrap().iter().map(|w| serde_json::to_value(w).unwrap()["status"].clone()).collect()
}

fn worker_pr_state(h: &H) -> Value {
    h.app.wf.tree().unwrap()[0]["workers"][0]["pr"]["state"].clone()
}

fn worker_readiness(h: &H) -> Value {
    h.app.wf.tree().unwrap()[0]["workers"][0]["pr"]["readiness"].clone()
}

fn readiness(draft: bool, merge_state: &str, review: Value, checks: Value) -> Value {
    json!({"isDraft": draft, "mergeStateStatus": merge_state, "reviewDecision": review, "checks": checks})
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
    assert_eq!(wait_status(&h), ["sent"]);
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
async fn a_head_move_leaves_a_relayed_review_card_alone() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    let body = format!("**ACTION NEEDED:** reply \"fix it\" or \"ship as is\", here: {URL}");
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": body})).await;

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 10).await;

    let review: Vec<Value> = cards(&h, false).into_iter().filter(|c| c["worker"].is_object()).collect();
    assert_eq!(review.len(), 1, "{review:?}");
    assert_eq!(review[0]["options"], json!(["fix it", "ship as is"]));

    h.github.update_pr(REPO, 7, json!({"state": "CLOSED"}));
    check(&h, 20).await;
    assert_eq!(cards(&h, false).into_iter().filter(|c| c["worker"].is_object()).count(), 1);
}

#[tokio::test]
async fn a_finished_pr_releases_the_merge_gate_past_a_relayed_gate() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    let body = format!("**ACTION NEEDED:** reply \"fix it\" or \"ship as is\", here: {URL}");
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": body})).await;
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));

    h.github.update_pr(REPO, 7, json!({"state": "CLOSED"}));
    check(&h, 10).await;

    let open = cards(&h, false);
    assert!(open.iter().all(|c| c["worker"].is_object()), "{open:?}");
    assert_eq!(open.len(), 1);
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("relay:review"));
}

#[tokio::test]
async fn a_new_merge_card_takes_the_gate_from_a_relay_hold() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    let body = format!("**ACTION NEEDED:** reply \"fix it\" or \"ship as is\", here: {URL}");
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": body})).await;
    let gate = |h: &H| h.app.store.link_get(TICKET).unwrap().unwrap().gate;
    assert_eq!(gate(&h).as_deref(), Some("relay:review"));
    wf_merge(&h, 7, HEAD).await;

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED, "mergeStateStatus": "CLEAN"}));
    check(&h, 2).await;
    assert_eq!(gate(&h).as_deref(), Some("merge"));

    h.wf(WORKER, "gate", json!({"clear": true})).await;
    assert_eq!(gate(&h).as_deref(), Some("merge"));
    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert!(open[0]["worker"].is_null(), "{open:?}");
}

async fn crash_while_clearing_the_merge_gate(h: &mut H, minutes: i64) {
    let db = rusqlite::Connection::open(h.app.store.path()).unwrap();
    db.execute_batch(
        "CREATE TRIGGER crash_on_insert BEFORE INSERT ON links WHEN json_extract(NEW.data, '$.gate') IS NULL
           BEGIN SELECT RAISE(ABORT, 'crashed clearing the gate'); END;
         CREATE TRIGGER crash_on_update BEFORE UPDATE ON links WHEN json_extract(NEW.data, '$.gate') IS NULL
           BEGIN SELECT RAISE(ABORT, 'crashed clearing the gate'); END;",
    )
    .unwrap();
    h.clock.set(&at(minutes));
    let err = h.app.wf.reconcile().await.unwrap_err();
    assert!(format!("{err:#}").contains("crashed clearing the gate"), "{err:#}");
    db.execute_batch("DROP TRIGGER crash_on_insert; DROP TRIGGER crash_on_update;").unwrap();
    h.reopen();
}

fn open_gate_cards(h: &H) -> usize {
    cards(h, false).iter().filter(|c| c["kind"] == "gate").count()
}

#[tokio::test]
async fn a_crash_while_clearing_a_merged_prs_gate_leaves_no_gate_without_a_card() {
    let mut h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(4)}));

    crash_while_clearing_the_merge_gate(&mut h, 5).await;

    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));
    assert_eq!(open_gate_cards(&h), 1, "the gate is still set, so its card is still open");
    check(&h, 7).await;
    let link = h.app.store.link_get(TICKET).unwrap().unwrap();
    assert_eq!((link.gate, link.gate_since), (None, None));
    assert!(cards(&h, false).is_empty());
    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_crash_while_clearing_a_moved_prs_gate_leaves_no_gate_without_a_card() {
    let mut h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    h.github.update_pr(
        REPO,
        7,
        json!({"headRefOid": MOVED, "isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}),
    );

    crash_while_clearing_the_merge_gate(&mut h, 5).await;

    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));
    assert_eq!(open_gate_cards(&h), 1, "the gate is still set, so its card is still open");
    check(&h, 7).await;
    let link = h.app.store.link_get(TICKET).unwrap().unwrap();
    assert_eq!((link.gate, link.gate_since), (None, None));
    assert_eq!(open_gate_cards(&h), 0);
    assert!(!cards(&h, false).is_empty(), "the moved report stays");
}

#[tokio::test]
async fn a_merge_keeps_the_gate_while_another_prs_gate_card_is_open() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.set_pr(REPO, 8, github_pr(HEAD, "BLOCKED", true));
    merge_gate(&h, Some(HEAD)).await;
    assert_eq!(wf_merge(&h, 8, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 8, json!({"headRefOid": MOVED}));
    check(&h, 2).await;
    assert_eq!(open_gate_cards(&h), 2);

    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(4)}));
    check(&h, 5).await;

    assert_eq!(open_gate_cards(&h), 1);
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));
}

#[tokio::test]
async fn a_merge_leaves_a_delivery_gate_alone() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let body = format!("**ACTION NEEDED:** reply \"deploy\" to deploy #7, here: {URL}");
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "delivery:deploy", "body": body})).await;
    check(&h, 0).await;

    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(4)}));
    check(&h, 5).await;

    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("delivery:deploy"));
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
    pr["mergedAt"] = json!(at(-60));
    h.github.set_pr(REPO, 7, pr);
    check(&h, 0).await;
    assert!(github_events(&h).is_empty());
    assert!(feed_heads(&h).iter().all(|f| !f.contains("merged on GitHub")));
}

#[tokio::test]
async fn a_waiting_pr_github_refuses_three_times_becomes_a_card() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    wf_merge(&h, 7, HEAD).await;
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    h.github.fault(
        Rule::on("enqueuePullRequest").fail(Failure::refused("Pull request is in clean status but is not mergeable")),
    );

    check(&h, 2).await;
    check(&h, 4).await;
    assert!(cards(&h, false).is_empty());
    assert_eq!(wait_status(&h), ["waiting"]);
    check(&h, 6).await;

    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let open = cards(&h, false);
    assert_eq!(open.len(), 1);
    assert!(open[0]["text"].as_str().unwrap().contains("GitHub would not queue acme/widgets#7"), "{}", open[0]);
    assert!(open[0]["text"].as_str().unwrap().contains("is not mergeable"), "{}", open[0]);
}

#[tokio::test]
async fn a_refusal_card_keeps_its_lowercase_label_after_a_long_multiline_error() {
    let long = "Repository rule violations found for this merge. ".repeat(20);
    for error in [format!("{long}\nRequired status check is expected.\n{long}"), format!("Refused:\n\n{long}")] {
        let h = H::new();
        ticket_with_worker(&h, 0).await;
        h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
        let code = format!("async () => wf.merge({})", json!({"repo": "Acme/Widgets", "pr": 7, "head": HEAD}));
        assert_eq!(result(&h.exec_as(TICKET, &code).await)["status"], "waiting");
        h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
        h.github.fault(Rule::on("enqueuePullRequest").fail(Failure::refused(&error)));

        check(&h, 2).await;
        check(&h, 4).await;
        check(&h, 6).await;

        let open = cards(&h, false);
        assert_eq!(open.len(), 1, "{open:?}");
        let text = open[0]["text"].as_str().unwrap();
        assert!(text.starts_with("GitHub would not queue acme/widgets#7 ("), "{text}");
        assert!(text.chars().count() <= 403 && !text.contains('\n'), "{text}");
    }
}

#[tokio::test]
async fn github_down_changes_nothing_and_the_board_pass_still_runs() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    h.github.unreachable();

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

    h.app.wf.dismiss(full[0]["id"].as_str().unwrap()).await.unwrap();
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

async fn second_ticket(h: &H) {
    h.wf("mcp:ticket2", "join", json!({"parent": BOARD, "label": "widgets#42", "role": "ticket"})).await;
    let body = format!("**ACTION NEEDED:** reply \"merge\" to merge #7, here: {URL}");
    h.wf("mcp:ticket2", "send", json!({"kind": "gate", "gate": "merge", "body": body})).await;
}

fn events_to(h: &H, ticket: &str) -> usize {
    h.sent()
        .iter()
        .filter(|s| s["threadId"] == ticket)
        .filter(|s| s["message"].as_str().is_some_and(|m| m.contains("[t3cm event from=github")))
        .count()
}

fn merged_at(minutes: i64) -> Value {
    json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": MERGED}, "mergedAt": at(minutes)})
}

#[tokio::test]
async fn one_failing_pr_does_not_stop_the_others() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 8, "open")]);
    h.tick(0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    h.github.set_pr(REPO, 8, github_pr(HEAD, "CLEAN", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    for i in 0..36 {
        let body = format!("**ACTION NEEDED:** filler {i}");
        if h.app.wf.dispatch("send", Some(TICKET), json!({"kind": "report", "body": body})).await.is_err() {
            break;
        }
    }
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.github.update_pr(REPO, 8, merged_at(1));
    h.clock.set(&at(2));

    let err = h.app.wf.reconcile().await.unwrap_err();

    assert!(format!("{err:#}").contains("acme/widgets#7"), "{err:#}");
    assert_eq!(github_events(&h).len(), 1);
    assert!(github_events(&h)[0].contains("acme/widgets#8 merged"), "{:?}", github_events(&h));
    assert_eq!(h.app.store.merge_waits().unwrap().len(), 1);
}

#[tokio::test]
async fn a_merge_on_github_releases_the_tickets_merge_gate() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));

    h.github.update_pr(REPO, 7, merged_at(3));
    check(&h, 4).await;

    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate, None);
}

#[tokio::test]
async fn a_new_approval_after_a_queue_removal_keeps_its_wait() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(1));
    h.clock.set(&at(2));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");

    check(&h, 3).await;

    assert_eq!(wait_status(&h), ["waiting"]);
    assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
}

#[tokio::test]
async fn a_queue_exit_with_no_timeline_stays_tracked_until_the_removal_shows() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let out = h.app.wf.dispatch("merge", None, json!({"repo": REPO, "pr": 7, "head": HEAD})).await.unwrap();
    assert_eq!(out["status"], "queued");
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "timelineItems": {"nodes": null}}));
    check(&h, 1).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(1));

    check(&h, 2).await;

    assert_eq!(reconciles(&h), 2);
    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("removed by hand"), "{open:?}");
}

fn dismiss_all(h: &H) {
    for c in cards(h, false) {
        h.app.store.waiting_resolve(BOARD, c["id"].as_str().unwrap(), Some("dismissed"), &at(0)).unwrap();
    }
}

async fn queued_then_left_with_no_reason(h: &H) {
    ticket_with_worker(h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    check(h, 0).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(h, 2).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    check(h, 4).await;
}

#[tokio::test]
async fn a_queue_removal_recorded_late_in_the_window_reports_its_reason() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    check(&h, 14).await;
    check(&h, 24).await;
    assert!(cards(&h, false).is_empty());
    h.github.drop_from_queue(REPO, 7, "manual", &at(3));

    check(&h, 34).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["ticket"], TICKET);
    assert!(open[0]["text"].as_str().unwrap().contains("dropped from the merge queue (removed by hand)"), "{open:?}");
    assert!(h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().left.is_none());
}

#[tokio::test]
async fn a_queue_exit_with_no_reason_by_the_window_close_gets_a_card() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    check(&h, 14).await;
    check(&h, 24).await;

    check(&h, 34).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["kind"], "report");
    assert_eq!(open[0]["ticket"], TICKET);
    let text = open[0]["text"].as_str().unwrap();
    assert!(text.contains("left the merge queue at") && text.contains("no reason recorded"), "{text}");
    assert_eq!(open[0]["options"], json!(["requeue", "hold"]));
    check(&h, 44).await;
    assert_eq!(cards(&h, false).len(), 1);
}

#[tokio::test]
async fn a_reason_recorded_after_the_window_replaces_the_no_reason_card() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    check(&h, 34).await;
    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(3));

    check(&h, 44).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("dropped from the merge queue (failed checks)"), "{open:?}");
    dismiss_all(&h);
    check(&h, 54).await;
    assert!(h.app.store.pr_seen_get(REPO, 7).unwrap().is_none());
}

#[tokio::test]
async fn a_requeue_resolves_the_no_reason_card_before_the_next_exit_is_reported() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    check(&h, 34).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));

    check(&h, 36).await;

    assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(37));
    check(&h, 38).await;
    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("dropped from the merge queue (failed checks)"), "{open:?}");
}

#[tokio::test]
async fn an_older_removal_does_not_count_for_a_later_queue_exit() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 0).await;
    h.github.drop_from_queue(REPO, 7, "merge_conflict", &at(1));
    check(&h, 2).await;
    assert_eq!(cards(&h, false).len(), 1);
    dismiss_all(&h);
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 4).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    check(&h, 6).await;

    check(&h, 36).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("no reason recorded"), "{open:?}");
}

#[tokio::test]
async fn a_removal_from_an_earlier_queue_stay_that_shows_late_is_not_reported_for_the_next_exit() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 2).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    check(&h, 4).await;
    check(&h, 34).await;
    dismiss_all(&h);
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 36).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    check(&h, 38).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(3));

    check(&h, 40).await;
    assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
    check(&h, 68).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("no reason recorded"), "{open:?}");
}

#[tokio::test]
async fn a_reason_recorded_after_an_approved_exit_was_reported_is_still_read() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
    check(&h, 1).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    for m in [2, 4, 6] {
        check(&h, m).await;
    }
    assert_eq!(cards(&h, false).len(), 1);
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    dismiss_all(&h);
    check(&h, 16).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(2));

    check(&h, 26).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("dropped from the merge queue (removed by hand)"), "{open:?}");
}

#[tokio::test]
async fn an_approved_exit_before_the_first_read_is_tracked_and_reported_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    for m in [2, 4, 6] {
        check(&h, m).await;
    }
    assert_eq!(cards(&h, false).len(), 1);
    dismiss_all(&h);
    let reads = reconciles(&h);

    for m in [16, 26, 36] {
        check(&h, m).await;
        assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
    }

    assert_eq!(reconciles(&h), reads + 3);
    check(&h, 46).await;
    assert_eq!(reconciles(&h), reads + 3);
}

#[tokio::test]
async fn an_approved_exit_still_waiting_on_its_checks_at_the_window_close_gets_one_report() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
    check(&h, 1).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));

    for m in [2, 33, 64, 95] {
        check(&h, m).await;
    }

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("recorded no reason"), "{open:?}");
}

#[tokio::test]
async fn an_exit_stays_tracked_when_a_later_waiting_approval_is_retired() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "BLOCKED"}));
    h.clock.set(&at(10));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    check(&h, 10).await;
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 12).await;
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    dismiss_all(&h);
    h.github.drop_from_queue(REPO, 7, "manual", &at(3));

    check(&h, 22).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("removed by hand"), "{open:?}");
}

#[tokio::test]
async fn a_waiting_approval_at_the_window_close_gets_no_no_reason_card() {
    let h = H::new();
    queued_then_left_with_no_reason(&h).await;
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "BLOCKED"}));
    h.clock.set(&at(10));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");

    check(&h, 34).await;

    assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
}

#[tokio::test]
async fn a_second_approved_exit_wakes_a_ticket_under_another_thread_again() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": "mcp:boss", "label": "widgets#41", "role": "ticket"})).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let reports = || {
        h.sent()
            .iter()
            .filter(|s| s["threadId"] == TICKET)
            .filter(|s| s["message"].as_str().is_some_and(|m| m.contains("[t3cm report from=github")))
            .count()
    };
    for (round, start) in [(1, 0), (2, 10)] {
        h.clock.set(&at(start));
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
        h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
        for m in [2, 4, 6] {
            check(&h, start + m).await;
        }
        assert_eq!(reports(), round);
    }
}

#[tokio::test]
async fn a_pr_queued_at_another_head_gets_one_mismatch_report_even_with_a_gate_open() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    merge_gate(&h, Some(HEAD)).await;
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED, "isInMergeQueue": true}));

    check(&h, 2).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["kind"], "report");
    assert!(open[0]["text"].as_str().unwrap().contains("not at its approved head 1111111"), "{open:?}");
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate, None);
    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")));
}

#[tokio::test]
async fn each_ticket_tracking_a_pr_is_woken_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    second_ticket(&h).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    check(&h, 0).await;

    h.github.update_pr(REPO, 7, merged_at(1));
    check(&h, 2).await;
    check(&h, 4).await;

    assert_eq!(events_to(&h, TICKET), 1);
    assert_eq!(events_to(&h, "mcp:ticket2"), 1);
}

#[tokio::test]
async fn a_pr_closed_reopened_and_closed_again_tells_the_ticket_twice() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"state": "CLOSED", "closedAt": at(1)}));
    check(&h, 2).await;
    h.github.update_pr(REPO, 7, json!({"state": "OPEN"}));
    check(&h, 3).await;
    h.github.update_pr(REPO, 7, json!({"state": "CLOSED", "closedAt": at(4)}));
    check(&h, 5).await;
    check(&h, 6).await;

    assert_eq!(github_events(&h).len(), 2);
}

#[tokio::test]
async fn a_worker_pr_that_merges_before_its_first_check_still_wakes_the_ticket() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    let mut pr = github_pr(HEAD, "UNKNOWN", true);
    pr["state"] = json!("MERGED");
    pr["mergeCommit"] = json!({"oid": MERGED});
    pr["mergedAt"] = json!(at(1));
    h.github.set_pr(REPO, 7, pr);

    check(&h, 2).await;

    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn events_from_before_the_first_check_after_install_stay_quiet() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 7, "open"), pull_request(REPO, 8, "open")]);
    h.tick(0).await;
    h.app.store.meta_set(t3_codemode::reconcile::SINCE, &at(30)).unwrap();
    let mut old = github_pr(HEAD, "UNKNOWN", true);
    old["state"] = json!("MERGED");
    old["mergeCommit"] = json!({"oid": MERGED});
    old["mergedAt"] = json!(at(10));
    h.github.set_pr(REPO, 7, old);
    h.github.set_pr(REPO, 8, github_pr(HEAD, "CLEAN", true));
    check(&h, 40).await;
    assert!(github_events(&h).is_empty());

    h.github.update_pr(REPO, 8, merged_at(45));
    check(&h, 50).await;

    let events = github_events(&h);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains("acme/widgets#8"), "{events:?}");
}

#[tokio::test]
async fn a_waiting_pr_that_github_cannot_find_is_reported_and_leaves_the_fast_cadence() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    let out = h.app.wf.dispatch("merge", None, json!({"repo": REPO, "pr": 7, "head": HEAD})).await.unwrap();
    assert_eq!(out["status"], "waiting");
    h.github.remove_pr(REPO, 7);

    for m in [2, 4, 6] {
        check(&h, m).await;
    }

    h.clock.set(&at(9));
    h.app.wf.reconcile_if_due().await.unwrap();
    assert_eq!(reconciles(&h), 3);
    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("no pull request"), "{open:?}");
    assert!(h.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
async fn a_waiting_approval_merges_directly_once_the_queue_is_turned_off() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    h.github.set_default_method(REPO, "REBASE");
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"isMergeQueueEnabled": false, "mergeStateStatus": "CLEAN"}));

    check(&h, 2).await;
    let last = h.github.calls().last().unwrap().clone();
    assert_eq!(last["operation"], "mergePullRequest");
    assert_eq!(last["variables"]["head"], HEAD);
    assert_eq!(last["variables"]["method"], "REBASE");
    assert!(github_events(&h).is_empty());

    check(&h, 4).await;
    check(&h, 6).await;
    assert_eq!(github_events(&h).len(), 1);
    assert!(h.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
async fn a_wake_that_t3_cannot_take_is_kept_and_delivered_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    merge_gate(&h, Some(HEAD)).await;
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, merged_at(1));
    h.fake.fail_tool("t3_thread_send");

    check(&h, 2).await;
    assert!(github_events(&h).is_empty());
    assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);

    h.fake.clear_faults_on("t3_thread_send");
    h.tick(3).await;
    check(&h, 4).await;
    h.tick(5).await;
    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_card_from_before_this_version_is_tracked_from_its_message() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let body = format!("**ACTION NEEDED:** reply \"merge\" to merge #7, here: {URL}");
    let handle = h.app.store.msg_new(TICKET, BOARD, "gate", &body, &at(0)).unwrap();
    let item = t3_codemode::store::WaitingItem {
        id: "a1".into(),
        text: "merge #7".into(),
        from: Some("widgets#41".into()),
        link: None,
        at: at(0),
    };
    let data = json!({"kind": "gate", "ticket": TICKET, "handle": handle, "options": ["merge"]});
    h.app.store.waiting_insert(BOARD, &item, Some(&data)).unwrap();
    check(&h, 0).await;

    h.github.update_pr(REPO, 7, merged_at(1));
    check(&h, 2).await;

    assert!(cards(&h, false).is_empty());
    assert_eq!(github_events(&h).len(), 1);
}

#[tokio::test]
async fn a_gate_whose_pr_link_is_past_the_card_text_is_still_tracked() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let body = format!("**ACTION NEEDED:** reply \"merge\" to merge #7\n\n{}\n\n{URL}", "Context. ".repeat(80));
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": body})).await;
    assert!(!cards(&h, false)[0]["text"].as_str().unwrap().contains(URL));
    check(&h, 0).await;

    h.github.update_pr(REPO, 7, merged_at(1));
    check(&h, 2).await;

    assert!(cards(&h, false).is_empty());
}

#[tokio::test]
async fn a_ticket_that_starts_tracking_after_a_close_is_never_told_of_it() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"state": "CLOSED", "closedAt": at(1)}));
    check(&h, 2).await;
    assert_eq!(events_to(&h, TICKET), 1);

    h.clock.set(&at(5));
    second_ticket(&h).await;
    check(&h, 6).await;
    check(&h, 8).await;

    assert_eq!(events_to(&h, "mcp:ticket2"), 0);
    assert_eq!(events_to(&h, TICKET), 1);
}

#[tokio::test]
async fn a_read_error_for_one_pr_leaves_its_approval_alone() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.fault(
        Rule::on("reconcile.pr").pr(REPO, 7).fail(Failure::refused("Something went wrong while executing your query")),
    );

    for m in [2, 4, 6] {
        h.clock.set(&at(m));
        let err = h.app.wf.reconcile().await.unwrap_err();
        assert!(format!("{err:#}").contains("Something went wrong"), "{err:#}");
    }

    assert!(cards(&h, false).is_empty());
    let waits = h.app.store.merge_waits().unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].failures, 0);
}

#[tokio::test]
async fn a_pr_that_leaves_the_queue_before_its_removal_shows_is_still_reported() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    check(&h, 0).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}));
    check(&h, 2).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "queued"})).await;
    h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
    check(&h, 4).await;
    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(3));

    check(&h, 14).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("failed checks"), "{open:?}");
}

#[tokio::test]
async fn a_waiting_approval_is_never_sent_to_another_pr_under_the_same_number() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"id": "PR_other", "mergeStateStatus": "CLEAN"}));

    check(&h, 2).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")));
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("no longer the pull request that was approved"), "{open:?}");
}

#[tokio::test]
async fn a_read_with_a_field_error_is_retried_and_wakes_the_ticket_once() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 7, "open")]);
    h.tick(0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    h.github.update_pr(REPO, 7, merged_at(1));
    h.github.fault(
        Rule::on("reconcile.pr")
            .pr(REPO, 7)
            .fail(Failure::field("mergeCommit", "Something went wrong while resolving mergeCommit")),
    );
    h.clock.set(&at(2));

    let err = h.app.wf.reconcile().await.unwrap_err();
    h.github.clear_faults_on("reconcile.pr");
    check(&h, 4).await;

    assert!(format!("{err:#}").contains("resolving mergeCommit"), "{err:#}");
    assert_eq!(github_events(&h).len(), 1, "{:?}", github_events(&h));
}

#[tokio::test]
async fn a_waiting_approval_with_no_github_node_id_is_never_sent() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    h.github.update_pr(REPO, 7, json!({"id": ""}));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));

    check(&h, 2).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")));
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    assert_eq!(cards(&h, false).len(), 1);
}

#[tokio::test]
async fn a_waiting_pr_with_a_failed_optional_check_is_held_with_one_card_until_the_check_passes() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));
    h.github.set_checks(REPO, 7, json!([check_run("CodeRabbit", "CANCELLED"), check_run("lint", "FAILURE")]));
    check(&h, 2).await;
    check(&h, 4).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")), "{:?}", h.github.operations());
    assert_eq!(wait_status(&h), ["waiting"]);
    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["kind"], "report");
    let text = open[0]["text"].as_str().unwrap();
    assert!(text.contains("acme/widgets#7 has a failing check at its approved head 1111111: lint (FAILURE)"), "{text}");
    assert!(!text.contains("CodeRabbit"), "{text}");
    assert_eq!(open[0]["options"], json!(["fix"]));
    assert!(feed_heads(&h).iter().any(|f| f.starts_with("acme/widgets#7 is held: a check failed")));

    h.github.set_checks(REPO, 7, json!([check_run("CodeRabbit", "CANCELLED"), check_run("lint", "SUCCESS")]));
    check(&h, 6).await;

    let mutations: Vec<String> = h.github.operations().into_iter().filter(|o| o.ends_with("PullRequest")).collect();
    assert_eq!(mutations, ["enqueuePullRequest"]);
    assert_eq!(h.github.calls().last().unwrap()["variables"]["head"], HEAD);
    assert_eq!(wait_status(&h), ["sent"]);
    assert!(cards(&h, false).is_empty(), "the held card resolves once the pull request is queued");
}

#[tokio::test]
async fn a_waiting_pr_unstable_only_from_a_cancelled_bot_check_is_queued() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    wf_merge(&h, 7, HEAD).await;

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));
    h.github.set_checks(REPO, 7, json!([check_run("build", "SUCCESS"), check_run("CodeRabbit", "CANCELLED")]));
    check(&h, 2).await;

    let mutations: Vec<String> = h.github.operations().into_iter().filter(|o| o.ends_with("PullRequest")).collect();
    assert_eq!(mutations, ["enqueuePullRequest"]);
    assert_eq!(wait_status(&h), ["sent"]);
    assert!(cards(&h, false).is_empty());
}

#[tokio::test]
async fn a_waiting_pr_with_a_running_check_at_unstable_is_neither_queued_nor_reported() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    wf_merge(&h, 7, HEAD).await;

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));
    let running = json!({"__typename": "CheckRun", "name": "e2e", "status": "QUEUED", "conclusion": null});
    h.github.set_checks(REPO, 7, json!([running]));
    check(&h, 2).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")), "{:?}", h.github.operations());
    assert_eq!(wait_status(&h), ["waiting"]);
    assert!(cards(&h, false).is_empty());
}

#[tokio::test]
async fn a_held_card_resolves_when_the_pr_is_queued_outside_the_board() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "UNSTABLE", true));
    h.github.set_checks(REPO, 7, json!([{"__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS"}]));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.set_checks(REPO, 7, json!([check_run("lint", "FAILURE")]));
    check(&h, 2).await;
    assert_eq!(cards(&h, false).len(), 1);

    h.github.set_checks(REPO, 7, json!([check_run("lint", "SUCCESS")]));
    h.github.update_pr(
        REPO,
        7,
        json!({"mergeStateStatus": "CLEAN", "isInMergeQueue": true, "mergeQueueEntry": {"position": 1}}),
    );
    check(&h, 4).await;

    assert!(cards(&h, false).is_empty());
    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")), "{:?}", h.github.operations());
    assert_eq!(wait_status(&h), ["sent"]);
}

async fn held_on_a_failing_check(h: &H) {
    ticket_with_worker(h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));
    h.github.set_checks(REPO, 7, json!([check_run("lint", "FAILURE")]));
    check(h, 2).await;
    assert!(cards(h, false)[0]["text"].as_str().unwrap().contains("has a failing check"));
}

fn push_with_a_pending_check(h: &H, head: &str) {
    h.github.update_pr(REPO, 7, json!({"headRefOid": head}));
    let running = json!({"__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS"});
    h.github.set_checks(REPO, 7, json!([running]));
}

#[tokio::test]
async fn a_push_from_a_held_head_to_a_pending_head_resolves_the_held_card() {
    let h = H::new();
    held_on_a_failing_check(&h).await;

    push_with_a_pending_check(&h, MOVED);
    check(&h, 4).await;

    let open = cards(&h, false);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["kind"], "gate");
    assert!(open[0]["text"].as_str().unwrap().contains("moved to head 2222222"), "{open:?}");
    let all = cards(&h, true);
    let held = all.iter().find(|c| c["kind"] == "report").unwrap();
    assert!(held["resolution"].as_str().unwrap().starts_with("head moved to 2222222"), "{held}");
}

#[tokio::test]
async fn a_new_approval_at_a_pushed_head_resolves_the_old_held_card() {
    let h = H::new();
    held_on_a_failing_check(&h).await;

    push_with_a_pending_check(&h, MOVED);
    assert_eq!(wf_merge(&h, 7, MOVED).await["status"], "waiting");
    check(&h, 4).await;

    assert!(cards(&h, false).is_empty(), "{:?}", cards(&h, false));
    assert_eq!(wait_status(&h), ["waiting"]);
}

#[tokio::test]
async fn a_held_card_stays_while_its_check_reruns_at_the_same_head() {
    let h = H::new();
    held_on_a_failing_check(&h).await;

    push_with_a_pending_check(&h, HEAD);
    check(&h, 4).await;

    assert_eq!(cards(&h, false).len(), 1);
}

#[tokio::test]
async fn a_reconcile_card_carries_its_approvals_id_and_unrelated_cards_do_not() {
    let h = H::new();
    let root = "mcp:root-ticket";
    h.wf(root, "init", json!({"role": "ticket", "root": true})).await;
    h.wf("mcp:other", "join", json!({"parent": BOARD, "label": "other#1", "role": "ticket"})).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));

    let first = result(
        &h.exec_as(root, &format!("async () => wf.merge({})", json!({"repo": REPO, "pr": 7, "head": HEAD}))).await,
    );
    assert_eq!(first["status"], "waiting");
    let a1 = first["approval"].as_str().unwrap().to_string();
    assert!(!a1.is_empty());

    // An unrelated board card whose text names the same PR carries no approval id.
    let unrelated = format!("**ACTION NEEDED:** check {REPO}#7 notes before release, here: {URL}");
    h.wf("mcp:other", "send", json!({"kind": "report", "body": unrelated})).await;
    let via_js = result(&h.exec("async () => wf.cards()").await);
    let open = via_js.as_array().unwrap();
    assert_eq!(open.len(), 1);
    assert!(open[0]["text"].as_str().unwrap().contains("acme/widgets#7"));
    assert!(open[0].get("approval").is_none(), "unrelated card must not carry an approval id: {}", open[0]);

    // Three enqueue refusals retire approval A1 and stamp A1 on the reconcile report card.
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    h.github.fault(
        Rule::on("enqueuePullRequest").fail(Failure::refused("Pull request is in clean status but is not mergeable")),
    );
    check(&h, 2).await;
    check(&h, 4).await;
    check(&h, 6).await;

    let via_js = result(&h.exec("async () => wf.cards()").await);
    let open = via_js.as_array().unwrap();
    let matching: Vec<&Value> = open.iter().filter(|c| c["approval"].as_str() == Some(a1.as_str())).collect();
    assert_eq!(matching.len(), 1, "{open:?}");
    assert!(matching[0]["text"].as_str().unwrap().contains("GitHub would not queue acme/widgets#7"), "{}", matching[0]);
    assert_eq!(open.iter().filter(|c| c.get("approval").is_none()).count(), 1, "{open:?}");

    // A second approval A2 held on a failing optional check gets its own distinct approval id.
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "BLOCKED"}));
    let second = result(
        &h.exec_as(root, &format!("async () => wf.merge({})", json!({"repo": REPO, "pr": 7, "head": HEAD}))).await,
    );
    assert_eq!(second["status"], "waiting");
    let a2 = second["approval"].as_str().unwrap().to_string();
    assert_ne!(a1, a2);
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));
    h.github.set_checks(REPO, 7, json!([check_run("lint", "FAILURE")]));
    check(&h, 8).await;

    let via_js = result(&h.exec("async () => wf.cards()").await);
    let open = via_js.as_array().unwrap();
    let a2_cards: Vec<&Value> = open.iter().filter(|c| c["approval"].as_str() == Some(a2.as_str())).collect();
    assert_eq!(a2_cards.len(), 1, "{open:?}");
    assert!(a2_cards[0]["text"].as_str().unwrap().contains("has a failing check"), "{}", a2_cards[0]);
}

#[tokio::test]
async fn the_reconcile_read_reports_each_open_prs_readiness_with_no_extra_call() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    h.github.update_pr(REPO, 7, json!({"isDraft": true, "reviewDecision": "REVIEW_REQUIRED"}));
    let running = json!({"__typename": "CheckRun", "name": "e2e", "status": "IN_PROGRESS", "conclusion": null});
    h.github.set_checks(REPO, 7, json!([check_run("build", "SUCCESS"), running]));
    assert_eq!(worker_readiness(&h), Value::Null, "no readiness before the first read");

    check(&h, 1).await;

    assert_eq!(h.github.operations(), ["reconcile"]);
    let query = h.github.calls()[0]["query"].as_str().unwrap().to_string();
    assert!(query.contains(" reviewDecision "), "{query}");
    assert_eq!(worker_pr_state(&h), "open");
    assert_eq!(worker_readiness(&h), readiness(true, "BLOCKED", json!("REVIEW_REQUIRED"), json!("pending")));

    let rev = h.app.wf.revision();
    check(&h, 2).await;
    assert_eq!(h.app.wf.revision(), rev, "an unchanged read leaves the board revision alone");
    h.github.update_pr(REPO, 7, json!({"isDraft": false, "mergeStateStatus": "CLEAN", "reviewDecision": "APPROVED"}));
    h.github.set_checks(REPO, 7, json!([check_run("build", "SUCCESS"), check_run("CodeRabbit", "SKIPPED")]));
    check(&h, 3).await;
    assert!(h.app.wf.revision() > rev, "a readiness change bumps the board revision");
    assert_eq!(worker_readiness(&h), readiness(false, "CLEAN", json!("APPROVED"), json!("passing")));

    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE", "reviewDecision": null}));
    h.github.set_checks(REPO, 7, json!([check_run("build", "FAILURE")]));
    check(&h, 5).await;
    assert_eq!(worker_readiness(&h), readiness(false, "UNSTABLE", Value::Null, json!("failing")));
    assert_eq!(h.github.operations(), ["reconcile"; 4]);
}

#[tokio::test]
async fn a_pr_with_no_checks_at_its_head_reports_no_check_rollup() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", false));
    check(&h, 1).await;
    assert_eq!(worker_readiness(&h)["checks"], Value::Null);

    h.github.set_checks(REPO, 7, json!([check_run("build", "SUCCESS")]));
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 3).await;
    assert_eq!(worker_readiness(&h)["checks"], Value::Null, "checks read at an older head are not this head's");
}

#[tokio::test]
async fn a_merged_pr_carries_no_readiness() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", false));
    check(&h, 1).await;
    assert_eq!(worker_readiness(&h)["checks"], Value::Null);
    assert_eq!(worker_readiness(&h)["mergeStateStatus"], "CLEAN");

    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MERGED}, "mergedAt": at(2)}));
    check(&h, 3).await;
    assert_eq!(worker_readiness(&h), Value::Null);
}

#[tokio::test]
async fn a_row_saved_before_readiness_was_read_shows_no_readiness_but_keeps_queued() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.app
        .store
        .pr_seen_put(&t3_codemode::store::PrSeen {
            repo: REPO.into(),
            pr: 7,
            state: "OPEN".into(),
            head: HEAD.into(),
            queued: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(worker_pr_state(&h), "queued");
    assert_eq!(worker_readiness(&h), Value::Null);
}

fn closing(issues: &[(&str, u64)]) -> Value {
    let nodes: Vec<Value> =
        issues.iter().map(|(repo, n)| json!({"number": n, "repository": {"nameWithOwner": repo}})).collect();
    json!({"nodes": nodes})
}

#[tokio::test]
async fn a_worker_opened_without_an_issue_takes_it_from_its_prs_closing_reference() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    let mut pr = github_pr(HEAD, "BLOCKED", true);
    pr["closingIssuesReferences"] = closing(&[("acme/widgets", 115), ("acme/widgets", 116)]);
    h.github.set_pr(REPO, 7, pr);
    assert!(h.app.store.link_get(WORKER).unwrap().unwrap().identity.issue.is_none());

    check(&h, 1).await;

    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert_eq!(ln.identity.issue.as_deref(), Some("acme/widgets#115"));
    assert_eq!(h.app.wf.snapshot().unwrap()["tree"][0]["workers"][0]["issue"], "acme/widgets#115");
}

#[tokio::test]
async fn a_worker_takes_its_issue_from_the_pr_it_shows_now() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 3, "closed"), pull_request(REPO, 7, "open")]);
    h.tick(0).await;
    let mut old = github_pr(HEAD, "BLOCKED", true);
    old["state"] = json!("CLOSED");
    old["closingIssuesReferences"] = closing(&[("acme/widgets", 114)]);
    h.github.set_pr(REPO, 3, old);
    let mut pr = github_pr(HEAD, "BLOCKED", true);
    pr["closingIssuesReferences"] = closing(&[("acme/widgets", 115)]);
    h.github.set_pr(REPO, 7, pr);
    assert!(worker_pr_url(&h).ends_with("/pull/7"), "{}", worker_pr_url(&h));

    check(&h, 1).await;

    assert_eq!(h.app.store.link_get(WORKER).unwrap().unwrap().identity.issue.as_deref(), Some("acme/widgets#115"));
}

fn worker_pr_url(h: &H) -> String {
    h.app.wf.tree().unwrap()[0]["workers"][0]["pr"]["url"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_worker_opened_with_an_issue_keeps_it_over_its_prs_closing_reference() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#105", "role": "ticket"})).await;
    h.wf(
        WORKER,
        "join",
        json!({"parent": TICKET, "label": "#115 (#105 1/4)", "role": "worker", "issue": "acme/widgets#115"}),
    )
    .await;
    h.fake.set_prs(WORKER, vec![pull_request(REPO, 7, "open")]);
    h.tick(0).await;
    let mut pr = github_pr(HEAD, "BLOCKED", true);
    pr["closingIssuesReferences"] = closing(&[("acme/widgets", 999)]);
    h.github.set_pr(REPO, 7, pr);

    check(&h, 1).await;

    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert_eq!(ln.identity.issue.as_deref(), Some("acme/widgets#115"));
}

#[tokio::test]
async fn a_queue_drop_between_worker_open_and_ready_card_is_idempotent_on_first_reconcile() {
    let h = H::new();
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    h.fake.update(TICKET, json!({"projectId": "proj:widgets"}));
    h.fake.update(WORKER, json!({"projectId": "proj:widgets"}));
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        WORKER,
        "join",
        json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker", "issue": "acme/widgets#41", "part": "1/1"}),
    )
    .await;

    let mut pr = github_pr(HEAD, "CLEAN", true);
    pr["body"] = json!("## Summary\n\ntest\n\nRefs #41\n");
    h.github.set_pr(REPO, 7, pr);
    h.github.set_checks(REPO, 7, json!([check_run("ci", "SUCCESS")]));
    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(3));

    h.clock.set(&at(5));
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert_eq!(r["status"], "gated", "{r}");

    // Worker opened at T0, queue drop happened at T0+3, ready card opened at T0+5.
    // The first reconcile must fold the worker's open time into `since` so the drop is
    // reported immediately rather than surfacing only on a second reconcile.
    check(&h, 6).await;
    let after_first: Vec<String> = cards(&h, false).into_iter().map(|c| c["id"].as_str().unwrap().into()).collect();
    assert!(
        cards(&h, false).iter().any(|c| c["text"].as_str().is_some_and(|t| t.contains("failed checks"))),
        "first reconcile should report the queue drop: {:?}",
        cards(&h, false)
    );

    check(&h, 6).await;
    let after_second: Vec<String> = cards(&h, false).into_iter().map(|c| c["id"].as_str().unwrap().into()).collect();
    assert_eq!(after_second, after_first, "second reconcile must be idempotent");
}

#[tokio::test]
async fn a_pre_tracking_merge_with_a_later_ready_card_does_not_trigger_after_merge() {
    let h = H::new();
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    h.fake.update(TICKET, json!({"projectId": "proj:widgets"}));
    h.fake.update(WORKER, json!({"projectId": "proj:widgets"}));
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        WORKER,
        "join",
        json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker", "issue": "acme/widgets#41", "part": "1/1"}),
    )
    .await;

    let mut pr = github_pr(HEAD, "CLEAN", false);
    pr["body"] = json!("## Summary\n\ntest\n\nRefs #41\n");
    h.github.set_pr(REPO, 7, pr);
    h.github.set_checks(REPO, 7, json!([check_run("ci", "SUCCESS")]));

    h.clock.set(&at(5));
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert_eq!(r["status"], "gated", "{r}");

    h.github.update_pr(
        REPO,
        7,
        json!({
            "state": "MERGED",
            "mergeCommit": {"oid": MERGED},
            "mergedAt": at(-10),
            "headRef": {"id": "REF_acme/widgets_7", "target": {"oid": HEAD}}
        }),
    );

    let actions = check(&h, 6).await;
    assert!(
        actions.iter().all(|a| !a.contains("merged on GitHub")),
        "pre-tracking merge must not be reported: {actions:?}"
    );
    assert!(
        !h.github.operations().iter().any(|op| op == "deleteRef"),
        "pre-tracking merge must not delete the branch: {:?}",
        h.github.operations()
    );
    assert!(h.sent().is_empty(), "pre-tracking merge must not wake worker or ticket: {:?}", h.sent());
    assert_eq!(cards(&h, false).len(), 1, "ready card created after the pre-tracking merge must stay untouched");
    assert_eq!(h.app.wf.reach().count(t3_codemode::reach::ReachPoint::MergeEventPredatesTracking), 1);
}

#[tokio::test]
async fn answering_a_moved_approval_card_clears_gate_pr_so_gate_clear_keeps_later_ready_card() {
    let h = H::new();
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    h.fake.update(TICKET, json!({"projectId": "proj:widgets"}));
    h.fake.update(WORKER, json!({"projectId": "proj:widgets"}));
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        WORKER,
        "join",
        json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker", "issue": "acme/widgets#41", "part": "1/1"}),
    )
    .await;

    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");

    let mut moved_pr = github_pr(MOVED, "CLEAN", false);
    moved_pr["body"] = json!("## Summary\n\ntest\n\nRefs #41\n");
    h.github.set_pr(REPO, 7, moved_pr);
    h.github.set_checks(REPO, 7, json!([check_run("ci", "SUCCESS")]));
    check(&h, 2).await;

    let open = result(&h.exec("async () => wf.cards()").await);
    let moved_id = open.as_array().unwrap()[0]["id"].as_str().unwrap().to_string();
    assert!(open[0]["text"].as_str().unwrap().contains("moved to head 2222222"), "{open}");

    h.app.wf.answer(&moved_id, "merge").await.unwrap();
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate_pr, None);
    let ready = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": MOVED})).await;
    assert_eq!(ready["status"], "gated", "{ready}");
    let ready_id = ready["card"].as_str().unwrap().to_string();

    let cleared = h.wf(TICKET, "gate", json!({"clear": true})).await;
    assert_eq!(cleared["gate"], "merge", "later unanswered ready merge card must hold the merge gate: {cleared}");
    let remaining = result(&h.exec("async () => wf.cards()").await);
    let remaining = remaining.as_array().unwrap();
    assert_eq!(remaining.len(), 1, "gate clear must not wipe the later ready merge card: {remaining:?}");
    assert_eq!(remaining[0]["id"], ready_id);
}

#[tokio::test]
async fn answering_a_moved_approval_card_then_clearing_gate_keeps_another_prs_ready_card() {
    for ready_first in [false, true] {
        let h = H::new();
        let repo = h.dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
        let worker2 = "mcp:worker-2";
        let mut th2 = t3_codemode::fake::thread(worker2, "idle", T0);
        th2["projectId"] = json!("proj:widgets");
        h.fake.put(th2);
        h.fake.update(TICKET, json!({"projectId": "proj:widgets"}));
        h.fake.update(WORKER, json!({"projectId": "proj:widgets"}));
        h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
        h.wf(
            WORKER,
            "join",
            json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker", "issue": "acme/widgets#41", "part": "1/2"}),
        )
        .await;
        h.wf(
            worker2,
            "join",
            json!({"parent": TICKET, "label": "#41 PR 2/2", "role": "worker", "issue": "acme/widgets#41", "part": "2/2"}),
        )
        .await;

        let mut pr8 = github_pr(HEAD, "CLEAN", false);
        pr8["body"] = json!("## Summary\n\ntest\n\nRefs #41\n");
        h.github.set_pr(REPO, 8, pr8);
        h.github.set_checks(REPO, 8, json!([check_run("ci", "SUCCESS")]));

        let ready_id = if ready_first {
            let ready = h.wf(worker2, "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": HEAD})).await;
            assert_eq!(ready["status"], "gated", "{ready}");
            Some(ready["card"].as_str().unwrap().to_string())
        } else {
            None
        };

        h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
        h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
        check(&h, 2).await;

        let open = result(&h.exec("async () => wf.cards()").await);
        let moved_id = open
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["text"].as_str().is_some_and(|t| t.contains("moved to head 2222222")))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let ready_id = match ready_id {
            Some(id) => id,
            None => {
                let ready = h.wf(worker2, "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": HEAD})).await;
                assert_eq!(ready["status"], "gated", "{ready}");
                ready["card"].as_str().unwrap().to_string()
            }
        };

        h.app.wf.answer(&moved_id, "merge").await.unwrap();
        assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate_pr, None, "ready_first={ready_first}");
        let cleared = h.wf(TICKET, "gate", json!({"clear": true})).await;

        assert_eq!(
            cleared["gate"], "merge",
            "other PR's unanswered ready merge card must hold the merge gate (ready_first={ready_first}): {cleared}"
        );
        let remaining = result(&h.exec("async () => wf.cards()").await);
        let remaining = remaining.as_array().unwrap();
        assert_eq!(
            remaining.len(),
            1,
            "unanswered PR #8 ready card must survive (ready_first={ready_first}): {remaining:?}"
        );
        assert_eq!(remaining[0]["id"], ready_id);
    }
}

#[tokio::test]
async fn superseding_a_moved_approval_card_with_a_newer_approval_releases_the_merge_gate() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");

    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    check(&h, 2).await;
    assert_eq!(h.app.wf.tree().unwrap()[0]["gate"], "merge");

    let review = format!("**ACTION NEEDED:** reply \"fix\" or \"ship\", here: {URL}");
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": review})).await;
    assert_eq!(h.app.wf.tree().unwrap()[0]["gate"], "merge");

    h.github.update_pr(REPO, 7, json!({"headRefOid": MERGED}));
    assert_eq!(wf_merge(&h, 7, MERGED).await["status"], "waiting");
    check(&h, 4).await;

    let open = result(&h.exec("async () => wf.cards()").await);
    let open = open.as_array().unwrap();
    assert_eq!(open.len(), 1, "only the relayed worker review card should remain open: {open:?}");
    assert_eq!(open[0]["worker"]["gate"], "review");
    let all = cards(&h, true);
    let superseded = all.iter().find(|c| c["resolution"] == "superseded by a newer merge approval");
    assert!(superseded.is_some(), "expected moved-approval card superseded by newer approval: {all:?}");
    assert_eq!(
        h.app.wf.tree().unwrap()[0]["gate"],
        "relay:review",
        "releasing the superseded merge gate must restore the relayed worker gate"
    );
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate_pr, None);

    h.app.wf.dismiss(open[0]["id"].as_str().unwrap()).await.unwrap();
    assert_eq!(h.app.wf.tree().unwrap()[0]["gate"], Value::Null);
}

#[tokio::test]
async fn a_queue_drop_superseded_by_a_reassigned_approval_is_not_reported_on_a_later_reconcile() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;

    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(10));
    h.clock.set(&at(11));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"id": "PR_other", "mergeStateStatus": "CLEAN"}));

    check(&h, 12).await;

    assert!(h.github.operations().iter().all(|o| !o.ends_with("PullRequest")));
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let after_first = cards(&h, false);
    assert_eq!(after_first.len(), 1, "{after_first:?}");
    assert!(
        after_first[0]["text"].as_str().unwrap().contains("no longer the pull request that was approved"),
        "{after_first:?}"
    );
    let sent_after_first = h.sent().len();

    let actions = check(&h, 14).await;

    assert!(actions.is_empty(), "second reconcile must reach a fixpoint: {actions:?}");
    assert_eq!(cards(&h, false), after_first, "second reconcile must not report the superseded drop");
    assert_eq!(h.sent().len(), sent_after_first, "second reconcile must send no messages");

    // A later queue drop at a new timestamp is still reported normally.
    h.github.drop_from_queue(REPO, 7, "merge_conflict", &at(15));
    check(&h, 16).await;
    let after_fresh = cards(&h, false);
    assert_eq!(after_fresh.len(), 2, "{after_fresh:?}");
    assert!(after_fresh.iter().any(|c| c["text"].as_str().unwrap().contains("a merge conflict")), "{after_fresh:?}");
}

#[tokio::test]
async fn a_queue_drop_superseded_by_a_wait_that_retires_on_head_move_stays_quiet_across_adopt() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.wf("mcp:ticket2", "join", json!({"parent": BOARD, "label": "widgets#42", "role": "ticket"})).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;

    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(10));
    h.clock.set(&at(11));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));

    check(&h, 12).await;

    assert!(h.app.store.merge_waits().unwrap().is_empty());
    let after_first = cards(&h, false);
    assert_eq!(after_first.len(), 1, "{after_first:?}");
    assert_eq!(after_first[0]["kind"], "gate");
    assert!(after_first[0]["text"].as_str().unwrap().contains("moved to head 2222222"), "{after_first:?}");
    let sent_after_first = h.sent().len();

    let actions = check(&h, 14).await;
    assert!(actions.is_empty(), "second reconcile must reach a fixpoint: {actions:?}");
    assert_eq!(cards(&h, false), after_first);
    assert_eq!(h.sent().len(), sent_after_first);

    h.wf("mcp:ticket2", "adopt", json!({"from": TICKET})).await;
    assert_eq!(h.app.store.link_get(WORKER).unwrap().unwrap().parent, "mcp:ticket2");
    let actions_after_adopt = check(&h, 16).await;
    assert!(actions_after_adopt.is_empty(), "superseded drop must stay quiet after adopt: {actions_after_adopt:?}");
    assert_eq!(cards(&h, false).len(), 1, "{:?}", cards(&h, false));
}

#[tokio::test]
async fn a_moved_ready_card_with_a_matching_merge_wait_under_a_non_merge_gate_reconciles_idempotently() {
    for (queued, ready_first) in [(false, false), (true, false), (false, true)] {
        let h = H::new();
        let repo = h.dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
        h.fake.update(TICKET, json!({"projectId": "proj:widgets"}));
        h.fake.update(WORKER, json!({"projectId": "proj:widgets"}));
        h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
        h.wf(
            WORKER,
            "join",
            json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker", "issue": "acme/widgets#41", "part": "1/1"}),
        )
        .await;

        let mut pr = github_pr(HEAD, "BLOCKED", true);
        pr["body"] = json!("## Summary\n\ntest\n\nRefs #41\n");
        h.github.set_pr(REPO, 7, pr);
        h.github.set_checks(REPO, 7, json!([check_run("ci", "SUCCESS")]));

        let (verify_id, ready_id) = if ready_first {
            // Ready card opens first, then a non-merge delivery:verify gate referencing the
            // same PR and head opens on top of it (board_card preserves open ready cards).
            let ready = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEAD})).await;
            assert_eq!(ready["status"], "gated", "{ready}");
            let verify = h
                .wf(
                    TICKET,
                    "send",
                    json!({
                        "kind": "gate",
                        "gate": "delivery:verify",
                        "body": format!("**ACTION NEEDED:** verify #7 at head {}: reply \"ok\", here: {URL}", &HEAD[..7]),
                        "options": ["ok"]
                    }),
                )
                .await;
            (verify["card"].as_str().unwrap().to_string(), ready["card"].as_str().unwrap().to_string())
        } else {
            // Exact #282 sequence: delivery:verify opens first, then worker opens a ready
            // merge card at HEAD while the ticket keeps delivery:verify.
            let verify = h
                .wf(
                    TICKET,
                    "send",
                    json!({
                        "kind": "gate",
                        "gate": "delivery:verify",
                        "body": "**ACTION NEEDED:** verify: reply \"ok\"",
                        "options": ["ok"]
                    }),
                )
                .await;
            let ready = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEAD})).await;
            assert_eq!(ready["status"], "gated", "{ready}");
            (verify["card"].as_str().unwrap().to_string(), ready["card"].as_str().unwrap().to_string())
        };
        assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("delivery:verify"));

        // 3. Ticket records a MergeWait for PR #7 at HEAD.
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
        assert_eq!(h.app.store.merge_waits().unwrap().len(), 1);

        // 4. PR #7 advances to MOVED (optionally queued).
        h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED, "isInMergeQueue": queued}));

        // 5. First reconcile retires the wait, replaces the ready card in the same pass,
        //    preserves delivery:verify (even when its body references PR #7), and sends no
        //    worker wake.
        let sent_before = h.sent().len();
        check(&h, 5).await;

        assert!(
            h.app.store.merge_waits().unwrap().is_empty(),
            "queued={queued} ready_first={ready_first}: MergeWait must retire on pass 1"
        );
        let open_after_first = cards(&h, false);
        assert!(
            open_after_first.iter().any(|c| c["id"] == verify_id),
            "queued={queued} ready_first={ready_first}: non-merge gate card must stay open: {open_after_first:?}"
        );
        assert!(
            open_after_first.iter().all(|c| c["id"] != ready_id),
            "queued={queued} ready_first={ready_first}: stale ready merge card must resolve on pass 1: {open_after_first:?}"
        );
        assert_eq!(open_after_first.len(), 2, "queued={queued} ready_first={ready_first}: {open_after_first:?}");
        let replacement = open_after_first.iter().find(|c| c["id"] != verify_id).unwrap();
        let expected_kind = if queued { "report" } else { "gate" };
        assert_eq!(replacement["kind"], expected_kind, "queued={queued} ready_first={ready_first}: {replacement}");
        assert!(
            replacement["text"].as_str().unwrap().contains("2222222"),
            "queued={queued} ready_first={ready_first}: {replacement}"
        );
        let resolved_ready = cards(&h, true).into_iter().find(|c| c["id"] == ready_id).unwrap();
        assert!(
            resolved_ready["resolution"]
                .as_str()
                .is_some_and(|r| r.starts_with("head moved to 2222222 after this approval")),
            "queued={queued} ready_first={ready_first}: {resolved_ready}"
        );
        let tlink = h.app.store.link_get(TICKET).unwrap().unwrap();
        assert_eq!(tlink.gate.as_deref(), Some("delivery:verify"), "queued={queued} ready_first={ready_first}");
        assert_eq!(tlink.gate_pr, None, "queued={queued} ready_first={ready_first}");
        assert_eq!(
            h.sent().len(),
            sent_before,
            "queued={queued} ready_first={ready_first}: pass 1 must not wake worker: {:?}",
            h.sent()
        );

        // Second reconcile is a complete no-op (idempotent, no extra worker wake).
        let actions_second = check(&h, 6).await;
        assert!(
            actions_second.is_empty(),
            "queued={queued} ready_first={ready_first}: pass 2 must be idempotent: {actions_second:?}"
        );
        assert_eq!(
            cards(&h, false),
            open_after_first,
            "queued={queued} ready_first={ready_first}: pass 2 changed cards"
        );
        assert_eq!(
            h.sent().len(),
            sent_before,
            "queued={queued} ready_first={ready_first}: pass 2 sent extra messages: {:?}",
            h.sent()
        );
    }
}

#[tokio::test]
async fn a_sent_wait_retired_by_head_move_after_queue_exit_clears_left_and_reconciles_idempotently() {
    for move_at in [37, 10] {
        let h = H::new();
        ticket_with_worker(&h, 0).await;
        h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
        check(&h, 1).await;

        h.github.update_pr(
            REPO,
            7,
            json!({"isMergeQueueEnabled": false, "isInMergeQueue": false, "mergeQueueEntry": null}),
        );
        check(&h, 2).await;
        assert!(h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().left.is_some());
        assert!(cards(&h, false).is_empty());

        h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
        let sent_before = h.sent().len();
        check(&h, move_at).await;

        assert!(h.app.store.merge_waits().unwrap().is_empty(), "move_at={move_at}: Sent wait must retire");
        assert_eq!(
            h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().left,
            None,
            "move_at={move_at}: head move superseding Sent wait must clear pr_seen.left"
        );
        let open_after_first = result(&h.exec("async () => wf.cards()").await);
        let open_after_first = open_after_first.as_array().unwrap().clone();
        assert_eq!(open_after_first.len(), 1, "move_at={move_at}: {open_after_first:?}");
        assert_eq!(open_after_first[0]["kind"], "gate");
        assert!(
            open_after_first[0]["text"].as_str().unwrap().contains("moved to head 2222222"),
            "move_at={move_at}: {open_after_first:?}"
        );
        assert!(
            !open_after_first[0]["text"].as_str().unwrap().contains("no reason recorded"),
            "move_at={move_at}: {open_after_first:?}"
        );

        let actions_second = check(&h, move_at).await;
        assert!(actions_second.is_empty(), "move_at={move_at}: pass 2 must be idempotent: {actions_second:?}");
        let open_after_second = result(&h.exec("async () => wf.cards()").await);
        assert_eq!(open_after_second.as_array().unwrap(), &open_after_first);
        assert_eq!(h.sent().len(), sent_before);

        // Even well past EXIT_WINDOW_MINS, no spurious no-reason exit card appears later.
        let actions_later = check(&h, move_at + 35).await;
        assert!(actions_later.is_empty(), "move_at={move_at}: later pass must stay quiet: {actions_later:?}");
        let open_later = result(&h.exec("async () => wf.cards()").await);
        assert_eq!(open_later.as_array().unwrap(), &open_after_first);
        assert_eq!(h.sent().len(), sent_before);
    }
}

#[tokio::test]
async fn a_sent_wait_retired_by_failed_after_exit_window_reconciles_idempotently() {
    // Single ticket: 3rd failed check past EXIT_WINDOW_MINS retires the Sent wait,
    // raises one refusal card, clears pr_seen.left in pass 1, and pass 2 is a no-op.
    {
        let h = H::new();
        ticket_with_worker(&h, 0).await;
        h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
        check(&h, 1).await;

        h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
        check(&h, 2).await;
        check(&h, 33).await;
        assert!(cards(&h, false).is_empty());

        check(&h, 64).await;
        assert!(h.app.store.merge_waits().unwrap().is_empty());
        assert_eq!(
            h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().left,
            None,
            "pass 1 must clear pr_seen.left once failed retires the Sent wait past EXIT_WINDOW_MINS"
        );
        let after_first = result(&h.exec("async () => wf.cards()").await);
        let after_first = after_first.as_array().unwrap().clone();
        assert_eq!(after_first.len(), 1, "{after_first:?}");
        assert!(after_first[0]["text"].as_str().unwrap().contains("GitHub would not keep acme/widgets#7"));

        let actions_second = check(&h, 64).await;
        assert!(actions_second.is_empty(), "pass 2 must be idempotent: {actions_second:?}");
        let after_second = result(&h.exec("async () => wf.cards()").await);
        assert_eq!(after_second.as_array().unwrap(), &after_first);
    }

    // Two live tickets tracking the same PR: pass 1 raises the refusal card for the
    // approving ticket, raises the no-reason exit card for the second ticket in the
    // same pass, clears pr_seen.left, and pass 2 is a complete no-op.
    {
        let h = H::new();
        ticket_with_worker(&h, 0).await;
        h.fake.put(t3_codemode::fake::thread("mcp:ticket2", "idle", T0));
        h.wf("mcp:ticket2", "join", json!({"parent": BOARD, "label": "widgets#42", "role": "ticket"})).await;
        let worker2 = "mcp:worker-2";
        h.fake.put(t3_codemode::fake::thread(worker2, "idle", T0));
        h.wf(worker2, "join", json!({"parent": "mcp:ticket2", "label": "#42 PR 1/1", "role": "worker"})).await;
        h.fake.set_prs(worker2, vec![pull_request(REPO, 7, "open")]);
        h.tick(0).await;

        h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
        assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "queued");
        check(&h, 1).await;

        h.github.update_pr(REPO, 7, json!({"isInMergeQueue": false, "mergeQueueEntry": null}));
        check(&h, 2).await;
        check(&h, 33).await;
        assert!(cards(&h, false).is_empty());

        let sent_before = h.sent().len();
        check(&h, 64).await;
        assert!(h.app.store.merge_waits().unwrap().is_empty());
        assert_eq!(h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap().left, None);

        let after_first = result(&h.exec("async () => wf.cards()").await);
        let after_first = after_first.as_array().unwrap().clone();
        assert_eq!(after_first.len(), 2, "{after_first:?}");
        let t1_cards: Vec<&Value> = after_first.iter().filter(|c| c["ticket"] == TICKET).collect();
        let t2_cards: Vec<&Value> = after_first.iter().filter(|c| c["ticket"] == "mcp:ticket2").collect();
        assert_eq!(t1_cards.len(), 1, "approving ticket must get only the refusal card: {after_first:?}");
        assert!(t1_cards[0]["text"].as_str().unwrap().contains("GitHub would not keep acme/widgets#7"));
        assert_eq!(t2_cards.len(), 1, "second ticket must get its no-reason exit card on pass 1: {after_first:?}");
        assert!(t2_cards[0]["text"].as_str().unwrap().contains("no reason recorded"));

        let actions_second = check(&h, 64).await;
        assert!(actions_second.is_empty(), "pass 2 must be idempotent: {actions_second:?}");
        let after_second = result(&h.exec("async () => wf.cards()").await);
        assert_eq!(after_second.as_array().unwrap(), &after_first);
        assert_eq!(h.sent().len(), sent_before);
    }
}
