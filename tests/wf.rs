mod common;

use common::*;
use serde_json::{Value, json};
use t3_codemode::wf::LAST_PASS;

async fn link_ticket_worker(h: &H) -> String {
    h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    let r = h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/2"})).await;
    let brief = r["brief"].as_str().unwrap().to_string();
    let call = brief.lines().find(|l| l.starts_with("async () => wf.join(")).expect("join call in brief").to_string();
    let out = h.exec_as(WORKER, &call).await;
    assert_eq!(result(&out)["parent"], TICKET);
    call
}

async fn link_boss_ticket(h: &H) {
    h.wf(BOSS, "init", json!({"role": "overseer"})).await;
    let r = h.wf(BOSS, "openChild", json!({"role": "ticket", "label": "widgets#41"})).await;
    assert_eq!(r["params"], json!({"down": 75, "miss": 2, "stall": 480}));
    h.wf(
        TICKET,
        "join",
        json!({"parent": BOSS, "label": "widgets#41", "role": "ticket", "down": 75, "miss": 2, "stall": 480}),
    )
    .await;
}

fn link(h: &H, child: &str) -> t3_codemode::store::Link {
    h.app.store.link_get(child).unwrap().unwrap()
}

#[tokio::test]
async fn join_binds_the_link_without_parent_work() {
    let h = H::new();
    let call = link_ticket_worker(&h).await;
    assert!(call.contains("down: 45, miss: 2, stall: 240"), "{call}");
    let ln = link(&h, WORKER);
    assert_eq!(ln.parent, TICKET);
    assert_eq!(ln.parent_role.as_deref(), Some("ticket"));
    assert_eq!(ln.link.as_deref(), Some("https://t3/mcp:worker"));
    assert!(h.app.store.pending_all().unwrap().is_empty());
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn a_note_never_wakes_and_rides_the_next_wake_as_a_digest() {
    let h = H::new();
    link_ticket_worker(&h).await;
    let r = h.wf(WORKER, "send", json!({"kind": "note", "body": "built d4db97c, starting review"})).await;
    assert_eq!(r["delivery"], "noted");
    assert_eq!(r["handle"], "m1");
    h.wf(WORKER, "send", json!({"kind": "note", "body": "review clean"})).await;
    assert!(h.sent().is_empty());
    let r = h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened: https://x/pull/42"})).await;
    assert_eq!(r["delivery"], "sent");
    let s = h.sent();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0]["threadId"], TICKET);
    assert_eq!(s[0]["mode"], "queue");
    let msg = s[0]["message"].as_str().unwrap();
    assert!(msg.starts_with("[t3cm event from=\"#41 PR 1/2\" msg=m3]\nPR opened"), "{msg}");
    assert!(msg.contains("review clean [m2] (+1 earlier)"), "{msg}");
    assert_eq!(h.wf(TICKET, "inbox", json!({})).await, json!([]));
}

#[tokio::test]
async fn inbox_lists_notes_then_clears() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "note", "body": "tests green"})).await;
    let inbox = h.wf(TICKET, "inbox", json!({})).await;
    assert_eq!(inbox[0]["handle"], "m1");
    assert_eq!(inbox[0]["head"], "tests green");
    assert_eq!(inbox[0]["label"], "#41 PR 1/2");
    assert_eq!(h.wf(TICKET, "inbox", json!({})).await, json!([]));
}

#[tokio::test]
async fn a_big_body_travels_as_an_excerpt_plus_handle() {
    let h = H::new();
    link_ticket_worker(&h).await;
    let body = "x".repeat(5000);
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": body})).await;
    let msg = h.last_message();
    assert!(msg.len() < 1800, "{}", msg.len());
    assert!(msg.contains("full: wf.show(\"m1\")"), "{msg}");
    let shown = result(&h.exec_as(TICKET, "async () => wf.show('m1')").await);
    assert_eq!(shown["body"].as_str().unwrap(), body);
    assert_eq!(shown["kind"], "blocker");
}

#[tokio::test]
async fn handoff_closes_the_link_and_later_sends_fail() {
    let h = H::new();
    link_ticket_worker(&h).await;
    let r = h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert_eq!(r["closed"], true);
    assert_eq!(r["delivery"], "sent");
    assert!(link(&h, WORKER).closed);
    assert!(h.wf_err(WORKER, "send", json!({"kind": "note", "body": "late"})).await.contains("closed"));
}

#[tokio::test]
async fn reply_reaches_the_child_and_a_closed_link_needs_reopen() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(TICKET, "reply", json!({"to": "#41 PR 1/2", "body": "The user says merge."})).await;
    assert_eq!(h.sent().last().unwrap()["threadId"], WORKER);
    assert!(h.last_message().starts_with("[t3cm reply from=parent msg=m1]"));
    h.wf(TICKET, "close", json!({"child": WORKER})).await;
    assert!(h.wf_err(TICKET, "reply", json!({"to": WORKER, "body": "x"})).await.contains("reopen"));
    h.wf(TICKET, "reply", json!({"to": WORKER, "body": "x", "reopen": true})).await;
}

#[tokio::test]
async fn adopt_moves_children_silently() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(BOSS, "init", json!({"role": "ticket"})).await;
    let r = h.wf(BOSS, "adopt", json!({"from": TICKET})).await;
    assert_eq!(r["adopted"], json!(["#41 PR 1/2"]));
    assert_eq!(link(&h, WORKER).parent, BOSS);
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn sweep_names_open_links() {
    let h = H::new();
    link_ticket_worker(&h).await;
    assert_eq!(h.wf(TICKET, "sweep", json!({})).await["open"], json!(["#41 PR 1/2"]));
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert_eq!(h.wf(TICKET, "sweep", json!({})).await["open"], json!([]));
}

#[tokio::test]
async fn ticket_events_never_wake_the_overseer_and_its_bodies_cap_at_600() {
    let h = H::new();
    link_boss_ticket(&h).await;
    let r = h.wf(TICKET, "send", json!({"kind": "event", "body": "PR 1/2 merged"})).await;
    assert_eq!(r["delivery"], "noted");
    assert!(h.sent().is_empty());
    h.wf(TICKET, "send", json!({"kind": "gate", "body": "y".repeat(3000)})).await;
    let msg = h.last_message();
    assert!(msg.len() < 1000, "{}", msg.len());
    assert!(msg.contains("gate=user"), "{msg}");
    assert!(msg.contains("(2400 more chars; full: wf.show(\"m2\"))"), "{msg}");
    assert!(msg.contains("PR 1/2 merged [m1]"), "{msg}");
    let r = h.wf(TICKET, "send", json!({"kind": "event", "body": "urgent", "wake": true})).await;
    assert_eq!(r["delivery"], "sent");
}

#[tokio::test]
async fn board_is_one_object_per_child_with_live_status() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "note", "body": "building"})).await;
    h.fake.update(WORKER, json!({"status": "running", "pendingRequestCount": 1}));
    let b = h.wf(TICKET, "board", json!({})).await;
    let kids = b["children"].as_array().unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0]["label"], "#41 PR 1/2");
    assert_eq!(kids[0]["status"], "running");
    assert_eq!(kids[0]["asks"], 1);
    assert_eq!(kids[0]["unread"], 1);
    assert_eq!(kids[0]["last"]["head"], "building");
    let b = h.wf(WORKER, "board", json!({})).await;
    assert_eq!(b["parent"]["thread"], TICKET);
}

#[tokio::test]
async fn wf_without_a_caller_explains_how_to_identify() {
    let h = H::new();
    let out = h.exec("async () => wf.init({ role: 'ticket' })").await;
    assert!(err_text(&out).contains("X-T3-Thread"), "{}", out.text);
    let out = h.exec_as(WORKER, "async () => wf.send({ kind: 'note', body: 'x', bogus: 1 })").await;
    assert!(err_text(&out).contains("unknown field `bogus`"), "{}", out.text);
}

#[tokio::test]
async fn an_idle_child_is_nudged_twice_then_escalated_once() {
    let h = H::new();
    link_ticket_worker(&h).await;
    assert!(h.tick(30).await.is_empty());
    assert_eq!(h.tick(45).await, vec!["nudge #41 PR 1/2 (1/2)"]);
    assert_eq!(h.sent().last().unwrap()["threadId"], WORKER);
    assert!(h.last_message().starts_with("[t3cm nudge from=watcher msg="), "{}", h.last_message());
    assert!(h.tick(60).await.is_empty());
    assert_eq!(h.tick(90).await, vec!["nudge #41 PR 1/2 (2/2)"]);
    let out = h.tick(135).await;
    assert!(out.iter().any(|a| a.starts_with("escalate #41 PR 1/2")), "{out:?}");
    let esc = h.sent().last().unwrap().clone();
    assert_eq!(esc["threadId"], TICKET);
    let msg = esc["message"].as_str().unwrap();
    assert!(msg.starts_with("[t3cm escalate"), "{msg}");
    assert!(msg.contains("https://t3/mcp:worker"), "{msg}");
    let n = h.sent().len();
    h.tick(200).await;
    h.tick(400).await;
    assert_eq!(h.sent().len(), n);
}

#[tokio::test]
async fn a_send_after_a_nudge_resets_the_count() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.tick(45).await;
    h.clock.set(&at(50));
    h.wf(WORKER, "send", json!({"kind": "note", "body": "still on it"})).await;
    h.fake.update(WORKER, json!({"updatedAt": at(50)}));
    assert_eq!(link(&h, WORKER).nudges, 0);
    assert_eq!(h.tick(95).await, vec!["nudge #41 PR 1/2 (1/2)"]);
}

#[tokio::test]
async fn an_open_gate_suppresses_nudges() {
    let h = H::new();
    link_ticket_worker(&h).await;
    assert_eq!(h.wf(WORKER, "gate", json!({"open": "merge"})).await["gate"], "merge");
    assert!(h.tick(300).await.is_empty());
    h.wf(WORKER, "gate", json!({"clear": true})).await;
    assert_eq!(h.tick(301).await, vec!["nudge #41 PR 1/2 (1/2)"]);
}

#[tokio::test]
async fn the_gate_kind_opens_the_gate_and_wakes() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "gate", "gate": "merge", "body": "reply merge for #42"})).await;
    assert!(h.last_message().contains("gate=merge"));
    assert_eq!(link(&h, WORKER).gate.as_deref(), Some("merge"));
}

#[tokio::test]
async fn a_busy_child_is_left_alone_but_a_long_silence_is_reported_once() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.update(WORKER, json!({"status": "running", "updatedAt": at(200)}));
    assert!(h.tick(200).await.is_empty());
    h.fake.update(WORKER, json!({"status": "idle", "pendingBackgroundTasks": [{"id": "b1"}], "updatedAt": at(100)}));
    assert!(h.tick(200).await.is_empty());
    let out = h.tick(241).await;
    assert!(out.iter().any(|a| a.starts_with("report to mcp:ticket")), "{out:?}");
    assert!(h.last_message().contains("sent nothing for 241 min"), "{}", h.last_message());
    let n = h.sent().len();
    h.tick(400).await;
    assert_eq!(h.sent().len(), n);
}

#[tokio::test]
async fn a_failed_run_is_nudged_at_once_and_only_once_per_run() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.update(WORKER, json!({"status": "failed", "updatedAt": at(2)}));
    assert_eq!(h.tick(3).await, vec!["nudge #41 PR 1/2 (1/2)"]);
    assert!(h.last_message().contains("Your last run ended failed."));
    assert!(h.tick(4).await.is_empty());
}

#[tokio::test]
async fn a_pending_question_is_reported_once_not_nudged() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.update(WORKER, json!({"status": "waiting", "pendingRequestCount": 1}));
    let out = h.tick(50).await;
    assert!(out.iter().any(|a| a.starts_with("report")), "{out:?}");
    assert_eq!(h.sent().last().unwrap()["threadId"], TICKET);
    assert!(h.last_message().contains("waiting on an answer"));
    let n = h.sent().len();
    h.tick(120).await;
    assert_eq!(h.sent().len(), n);
}

#[tokio::test]
async fn a_never_joined_child_is_reported() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "lost"})).await;
    assert!(h.tick(44).await.is_empty());
    assert!(h.tick(46).await.contains(&"report never-joined lost".to_string()));
    assert!(h.last_message().contains("never joined"));
    let n = h.sent().len();
    h.tick(200).await;
    assert_eq!(h.sent().len(), n);
}

#[tokio::test]
async fn a_gone_thread_closes_its_link() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.remove(WORKER);
    assert_eq!(h.tick(10).await, vec!["closed #41 PR 1/2: thread gone"]);
    let ln = link(&h, WORKER);
    assert!(ln.closed);
    assert_eq!(ln.closed_how.as_deref(), Some("thread-gone"));
    let inbox = h.wf(TICKET, "inbox", json!({})).await;
    assert!(inbox[0]["head"].as_str().unwrap().contains("thread is gone"));
}

#[tokio::test]
async fn an_archived_thread_closes_its_link() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.update(WORKER, json!({"archived": true}));
    assert_eq!(h.tick(10).await, vec!["closed #41 PR 1/2: archived"]);
}

#[tokio::test]
async fn wakes_to_a_busy_parent_coalesce_into_one_message() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.fake.update(TICKET, json!({"status": "running"}));
    let r1 = h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    let r2 = h.wf(WORKER, "send", json!({"kind": "event", "body": "checks green"})).await;
    assert_eq!((r1["delivery"].clone(), r2["delivery"].clone()), (json!("queued"), json!("queued")));
    assert!(r1["detail"].as_str().unwrap().contains("mid-turn"));
    assert!(h.sent().is_empty());
    h.fake.update(TICKET, json!({"status": "idle", "updatedAt": at(1)}));
    h.fake.update(WORKER, json!({"status": "running", "updatedAt": at(1)}));
    let out = h.tick(2).await;
    assert_eq!(out, vec!["delivered m1+m2 to mcp:ticket"]);
    let s = h.sent();
    assert_eq!(s.len(), 1);
    let msg = s[0]["message"].as_str().unwrap();
    assert!(msg.contains("PR opened") && msg.contains("checks green"), "{msg}");
}

#[tokio::test]
async fn a_busy_parent_gets_held_wakes_after_the_max_hold() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.fake.update(TICKET, json!({"status": "running"}));
    h.fake.update(WORKER, json!({"status": "running"}));
    h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    assert!(h.tick(29).await.is_empty());
    assert_eq!(h.tick(31).await, vec!["delivered m1 to mcp:ticket"]);
}

#[tokio::test]
async fn a_failed_delivery_stays_queued_and_the_watcher_retries() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.fake.set_send_fails(true);
    let r = h.wf(WORKER, "send", json!({"kind": "blocker", "body": "need a token"})).await;
    assert_eq!(r["delivery"], "queued");
    assert!(r["detail"].as_str().unwrap().contains("delivery failed"));
    h.fake.set_send_fails(false);
    h.fake.update(WORKER, json!({"status": "running"}));
    assert_eq!(h.tick(3).await, vec!["delivered m1 to mcp:ticket"]);
}

#[tokio::test]
async fn a_failed_delivery_keeps_the_digest_for_the_retry() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "note", "body": "review clean"})).await;
    h.fake.set_send_fails(true);
    h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    h.fake.set_send_fails(false);
    h.fake.update(WORKER, json!({"status": "running"}));
    h.tick(3).await;
    assert!(h.last_message().contains("review clean [m1]"), "{}", h.last_message());
    assert_eq!(h.wf(TICKET, "inbox", json!({})).await, json!([]));
}

#[tokio::test]
async fn waiting_list_ids_cycle_and_skip_open_ones() {
    let h = H::new();
    let add = |text: String| h.wf(BOSS, "waiting.add", json!({"text": text}));
    let mut ids = Vec::new();
    for i in 0..36 {
        ids.push(add(format!("item {i}")).await["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids[..3], ["a1", "a2", "a3"]);
    assert_eq!(ids[9], "b1");
    assert_eq!(ids[35], "d9");
    assert!(h.wf_err(BOSS, "waiting.add", json!({"text": "full"})).await.contains("full"));
    h.wf(BOSS, "waiting.resolve", json!({"id": "b3", "note": "merged"})).await;
    h.wf(BOSS, "waiting.resolve", json!({"id": "a2"})).await;
    assert_eq!(add("next".into()).await["id"], "a2");
    assert_eq!(add("after".into()).await["id"], "b3");
    let open: Vec<Value> = h.wf(BOSS, "waiting.list", json!({})).await.as_array().unwrap().clone();
    assert_eq!(open.len(), 36);
    assert!(h.wf_err(BOSS, "waiting.resolve", json!({"id": "zz"})).await.contains("no open"));
    let mine = result(
        &h.exec_as(TICKET, "async () => (await wf.waiting.add({ text: 'ticket item', link: 'https://t3/x' })).id")
            .await,
    );
    assert_eq!(mine, "a1");
    let res = result(&h.exec_as(TICKET, "async () => wf.waiting.resolve('a1', 'done')").await);
    assert_eq!(res, json!({"id": "a1", "text": "ticket item", "resolved": true}));
}

#[tokio::test]
async fn wf_state_survives_a_restart() {
    let mut h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "note", "body": "before restart"})).await;
    h.reopen();
    let inbox = h.wf(TICKET, "inbox", json!({})).await;
    assert_eq!(inbox[0]["head"], "before restart");
}
