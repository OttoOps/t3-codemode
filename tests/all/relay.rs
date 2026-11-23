use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;

const REVIEW: &str = "**ACTION NEEDED:** decide each open review finding: reply \"fix it\" or \"ship as is\", \
here: https://github.com/acme/widgets/pull/7\n\nFinding 1: the retry loop never backs off.";
const BLOCKED: &str = "**ACTION NEEDED:** sign in to GitHub, then reply \"retry\", here: `gh auth login`";

async fn board_ticket_with_worker(h: &H) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
}

fn to(h: &H, thread: &str) -> Vec<String> {
    h.sent().into_iter().filter(|s| s["threadId"] == thread).map(|s| s["message"].as_str().unwrap().into()).collect()
}

fn open_cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn gate_of(h: &H, child: &str) -> Option<String> {
    h.app.store.link_get(child).unwrap().unwrap().gate
}

async fn review_gate(h: &H) -> Value {
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": REVIEW})).await
}

#[tokio::test]
async fn a_review_gate_goes_to_the_board_and_its_answer_back_to_the_worker() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let r = review_gate(&h).await;
    assert_eq!(r["delivery"], "board");
    assert_eq!(r["options"], json!(["fix it", "ship as is"]));
    let cards = open_cards(&h);
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card["id"], r["card"]);
    assert_eq!(card["kind"], "gate");
    assert_eq!(card["label"], "widgets#41");
    assert_eq!(card["ticket"], TICKET);
    assert_eq!(card["link"], format!("https://t3/{WORKER}"));
    assert_eq!(card["worker"]["thread"], WORKER);
    assert_eq!(card["worker"]["label"], "#41 PR 1/1");
    assert!(to(&h, TICKET).is_empty(), "the ticket must not wake for a relayed gate");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:review"));

    h.tick(100).await;
    assert!(to(&h, TICKET).is_empty(), "a ticket holding a relayed gate is not nudged");

    let id = card["id"].as_str().unwrap();
    let out = h.app.wf.answer(id, "ship as is").await.unwrap();
    assert_eq!(out["relayed"], "#41 PR 1/1");
    let worker = to(&h, WORKER);
    assert_eq!(worker.len(), 1);
    assert!(worker[0].starts_with("[t3cm reply from=board msg="), "{}", worker[0]);
    assert!(worker[0].contains(&format!("The user says \"ship as is\" about {id}")), "{}", worker[0]);
    assert!(to(&h, TICKET).is_empty(), "the answer must not wake the ticket");
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);

    h.wf(WORKER, "send", json!({"kind": "event", "body": "PR 1/1 opened: #7"})).await;
    let ticket = to(&h, TICKET);
    assert_eq!(ticket.len(), 1);
    let digest = ticket[0].split_once("Notes since your last wake").expect("a digest").1;
    let handled: Vec<&str> = digest.lines().filter(|l| l.starts_with("- handled for you")).collect();
    assert_eq!(handled.len(), 2, "{digest}");
    assert!(handled[0].contains(&format!("review gate from #41 PR 1/1 (m1) went straight to the board as card {id}")));
    assert!(
        handled[1].contains(&format!("the board's answer \"ship as is\" on card {id} went straight to #41 PR 1/1"))
    );
    assert_eq!(h.wf(TICKET, "inbox", json!({})).await, json!([]));
}

#[tokio::test]
async fn a_blocker_on_hold_reaches_the_worker_without_a_ticket_turn() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let r = h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    assert_eq!(r["delivery"], "board");
    let card = open_cards(&h).remove(0);
    assert_eq!(card["kind"], "blocker");
    assert_eq!(card["options"], json!(["retry"]));
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:blocker"));
    let id = card["id"].as_str().unwrap();
    h.app.wf.answer(id, "Hold").await.unwrap();
    assert!(to(&h, WORKER)[0].contains("The user says \"Hold\""));
    assert!(to(&h, TICKET).is_empty());

    let inbox = h.wf(TICKET, "inbox", json!({})).await;
    let kinds: Vec<&str> = inbox.as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["handled", "handled"], "{inbox}");
    assert!(inbox[0]["head"].as_str().unwrap().starts_with("blocker from #41 PR 1/1"));
}

#[tokio::test]
async fn a_free_text_answer_goes_to_the_ticket_with_the_relay_in_its_digest() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let id = review_gate(&h).await["card"].as_str().unwrap().to_string();
    h.app.wf.answer(&id, "fix the first one, then ship").await.unwrap();
    assert!(to(&h, WORKER).is_empty());
    let ticket = to(&h, TICKET);
    assert_eq!(ticket.len(), 1);
    assert!(ticket[0].contains("sent past you to the board"), "{}", ticket[0]);
    assert!(ticket[0].contains("wf.reply to \"#41 PR 1/1\""), "{}", ticket[0]);
    assert!(ticket[0].contains("- handled for you"), "{}", ticket[0]);
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
}

#[tokio::test]
async fn the_ticket_holds_its_relay_gate_until_the_last_relayed_card_is_answered() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let blocker = h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    let review = review_gate(&h).await;
    h.app.wf.answer(review["card"].as_str().unwrap(), "fix it").await.unwrap();
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:blocker"));
    h.app.wf.answer(blocker["card"].as_str().unwrap(), "retry").await.unwrap();
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(to(&h, WORKER).len(), 2);
    assert!(to(&h, TICKET).is_empty());
}

#[tokio::test]
async fn a_relayed_blocker_holds_the_worker_until_it_is_answered() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let id =
        h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await["card"].as_str().unwrap().to_string();
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("relay:blocker"));
    h.tick(200).await;
    assert!(h.sent().is_empty(), "neither the worker nor the ticket hears from the watcher: {:?}", h.sent());
    h.app.wf.answer(&id, "retry").await.unwrap();
    assert_eq!(gate_of(&h, WORKER), None);
}

#[tokio::test]
async fn dismissing_the_last_relayed_card_releases_the_ticket() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let id = review_gate(&h).await["card"].as_str().unwrap().to_string();
    h.app.wf.dismiss(&id).unwrap();
    assert_eq!(gate_of(&h, TICKET), None);
    let id =
        h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await["card"].as_str().unwrap().to_string();
    h.app.wf.dismiss_at(&id, &Default::default()).await.unwrap();
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("review"), "the worker's own gate is the worker's to clear");
}

#[tokio::test]
async fn the_relay_hold_comes_back_after_the_tickets_own_gate_clears() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    review_gate(&h).await;
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": "reply \"merge\" to merge #7"})).await;
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));
    let r = h.wf(TICKET, "gate", json!({"clear": true})).await;
    assert_eq!(r["gate"], "relay:review");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:review"));
}

#[tokio::test]
async fn a_worker_that_clears_its_gate_resolves_the_relayed_card() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    review_gate(&h).await;
    h.wf(WORKER, "gate", json!({"clear": true})).await;
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
}

#[tokio::test]
async fn relayed_and_ticket_gates_supersede_and_clear_apart() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    h.wf(TICKET, "gate", json!({"open": "merge"})).await;
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": "reply \"merge\" to merge #7"})).await;
    review_gate(&h).await;
    assert_eq!(open_cards(&h).len(), 3, "a relayed gate does not supersede the ticket's own gate");
    review_gate(&h).await;
    assert_eq!(open_cards(&h).len(), 3, "a newer relayed gate supersedes the worker's older one");
    h.wf(TICKET, "gate", json!({"clear": true})).await;
    let left: Vec<(Value, Value)> =
        open_cards(&h).iter().map(|c| (c["kind"].clone(), c["worker"]["label"].clone())).collect();
    assert_eq!(left, [(json!("blocker"), json!("#41 PR 1/1")), (json!("gate"), json!("#41 PR 1/1"))]);
}

#[tokio::test]
async fn ticket_owned_gates_from_the_worker_still_wake_the_ticket() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "merge", "body": "PR 1/1 ready to merge: #7"})).await;
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "delivery:deploy", "body": "reply \"deploy\""})).await;
    assert_eq!(to(&h, TICKET).len(), 2);
    assert!(open_cards(&h).is_empty());
}

#[tokio::test]
async fn a_custom_worker_gate_is_raised_and_answered_on_the_board() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let body = "**ACTION NEEDED:** reply \"retry\" to rerun the member check, or \"skip\", here: https://github.com/acme/widgets/pull/7";
    let r = h.wf(WORKER, "send", json!({"kind": "gate", "gate": "s1-member-retry", "body": body})).await;
    assert_eq!(r["delivery"], "board");
    let card = open_cards(&h).remove(0);
    assert_eq!(card["worker"]["gate"], "s1-member-retry");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:s1-member-retry"));
    let id = card["id"].as_str().unwrap();
    h.app.wf.answer(id, "skip").await.unwrap();
    assert!(to(&h, WORKER)[0].contains("The user says \"skip\""));
    assert!(to(&h, TICKET).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);

    h.wf(WORKER, "send", json!({"kind": "event", "body": "member check skipped"})).await;
    let ticket = to(&h, TICKET);
    let handled: Vec<&str> = ticket[0].lines().filter(|l| l.starts_with("- handled for you")).collect();
    assert_eq!(handled.len(), 2, "{}", ticket[0]);
    assert!(handled[0].contains("s1-member-retry gate from #41 PR 1/1"), "{}", handled[0]);
    assert!(handled[1].contains("the board's answer \"skip\""), "{}", handled[1]);
}

#[tokio::test]
async fn a_merge_gate_supersedes_the_workers_relayed_gate() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    review_gate(&h).await;
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "merge", "body": "PR 1/1 ready to merge: #7"})).await;
    let kinds: Vec<Value> = open_cards(&h).iter().map(|c| c["kind"].clone()).collect();
    assert_eq!(kinds, [json!("blocker")]);
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("relay:blocker"));
}

#[tokio::test]
async fn a_merge_answer_on_a_relayed_card_goes_to_the_ticket() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let body =
        "**ACTION NEEDED:** reply \"merge\" to merge #7, or \"hold\", here: https://github.com/acme/widgets/pull/7";
    let id = h.wf(WORKER, "send", json!({"kind": "gate", "gate": "user", "body": body})).await["card"]
        .as_str()
        .unwrap()
        .to_string();
    h.app.wf.answer(&id, "merge").await.unwrap();
    assert!(to(&h, WORKER).is_empty());
    let ticket = to(&h, TICKET);
    assert_eq!(ticket.len(), 1);
    assert!(ticket[0].contains("The user says \"merge\""), "{}", ticket[0]);
    assert!(open_cards(&h).is_empty());
}

#[tokio::test]
async fn a_ticket_under_an_overseer_thread_keeps_todays_path() {
    let h = H::new();
    h.wf(BOSS, "init", json!({"role": "overseer"})).await;
    h.wf(TICKET, "join", json!({"parent": BOSS, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    let r = h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    assert_eq!(r["delivery"], "sent");
    assert!(to(&h, TICKET)[0].starts_with("[t3cm blocker from=\"#41 PR 1/1\""));
    assert!(open_cards(&h).is_empty());
}

#[tokio::test]
async fn relay_off_on_the_worker_link_wakes_the_ticket() {
    let config = Config::from_value(json!({"roles": {"ticket": {"children": {"worker": {"relay": false}}}}})).unwrap();
    let h = H::build(Vec::new(), config);
    board_ticket_with_worker(&h).await;
    let r = review_gate(&h).await;
    assert_eq!(r["delivery"], "sent");
    assert!(to(&h, TICKET)[0].starts_with("[t3cm gate from=\"#41 PR 1/1\""));
    assert!(open_cards(&h).is_empty());
}

#[tokio::test]
async fn closing_the_worker_link_resolves_its_relayed_cards() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    review_gate(&h).await;
    h.wf(TICKET, "close", json!({"child": WORKER})).await;
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
}

#[tokio::test]
async fn a_closed_then_rejoined_worker_drops_its_blocker_hold() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    h.wf(TICKET, "close", json!({"child": WORKER})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    assert_eq!(gate_of(&h, WORKER), None);
}

#[tokio::test]
async fn an_archived_ticket_releases_its_blocked_worker() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    h.fake.update(TICKET, json!({"archived": true}));
    h.tick(1).await;
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, WORKER), None);
}

#[tokio::test]
async fn closing_a_board_ticket_releases_its_blocked_worker() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    h.app.wf.close_ticket(TICKET, false).await.unwrap();
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, WORKER), None);
}

#[tokio::test]
async fn an_adopting_ticket_takes_over_the_relayed_cards() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(BOSS, "join", json!({"parent": BOARD, "label": "widgets#41 again", "role": "ticket"})).await;
    let id =
        h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await["card"].as_str().unwrap().to_string();
    h.wf(BOSS, "adopt", json!({"from": TICKET})).await;
    assert_eq!(open_cards(&h)[0]["ticket"], BOSS);
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(gate_of(&h, BOSS).as_deref(), Some("relay:blocker"));
    h.app.wf.answer(&id, "retry").await.unwrap();
    assert_eq!(to(&h, WORKER).len(), 1);
    assert_eq!(gate_of(&h, BOSS), None);
    let inbox = h.wf(BOSS, "inbox", json!({})).await;
    assert!(inbox.as_array().unwrap().iter().any(|m| m["kind"] == "handled"), "{inbox}");
}

#[tokio::test]
async fn a_moved_worker_still_gets_its_relayed_answer() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    let fork = "mcp:worker-fork";
    h.fake.put(t3_codemode::fake::thread(fork, "idle", T0));
    let id = review_gate(&h).await["card"].as_str().unwrap().to_string();
    h.app
        .store
        .rebind(&t3_codemode::store::Rebind {
            old: WORKER,
            new: fork,
            link: Some("https://t3/fork"),
            board: BOARD,
            card: None,
            pick: 0,
            summary: "moved the worker",
            at: T0,
        })
        .unwrap();
    let card = open_cards(&h).remove(0);
    assert_eq!(card["worker"]["thread"], fork);
    assert_eq!(card["ticket"], TICKET);
    assert_eq!(card["link"], "https://t3/fork");
    h.app.wf.answer(&id, "ship as is").await.unwrap();
    assert_eq!(to(&h, fork).len(), 1);
    assert!(to(&h, TICKET).is_empty());
}

#[tokio::test]
async fn a_worker_handoff_resolves_its_open_relayed_cards() {
    let h = H::new();
    board_ticket_with_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": BLOCKED})).await;
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert!(open_cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(to(&h, TICKET).len(), 1);
}
