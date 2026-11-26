use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::store::{Bind, Link, Store};
use t3_codemode::wf::LAST_PASS;

async fn link_ticket_worker(h: &H) -> String {
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
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
    let r = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "x", "reopen": true})).await;
    assert_eq!(r["delivery"], "sent");
    assert_eq!(h.sent().last().unwrap()["threadId"], WORKER);
    assert!(h.last_message().ends_with("\nx"), "{}", h.last_message());
}

#[tokio::test]
async fn adopt_moves_children_silently() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(BOSS, "init", json!({"role": "ticket", "root": true})).await;
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
async fn a_paused_link_is_never_nudged_escalated_or_reported_until_it_resumes() {
    let h = H::new();
    link_ticket_worker(&h).await;
    assert_eq!(h.tick(45).await, vec!["nudge #41 PR 1/2 (1/2)"]);
    assert_eq!(h.wf(WORKER, "pause", json!({})).await, json!({"label": "#41 PR 1/2", "paused": true}));
    assert_eq!(link(&h, WORKER).nudges, 0);
    let n = h.sent().len();
    for m in [100, 200, 300] {
        assert!(h.tick(m).await.is_empty());
    }
    h.fake.update(WORKER, json!({"status": "waiting", "pendingRequestCount": 1}));
    assert!(h.tick(400).await.is_empty());
    h.fake.update(WORKER, json!({"status": "running", "pendingRequestCount": 0}));
    assert!(h.tick(500).await.is_empty());
    h.fake.update(WORKER, json!({"status": "failed"}));
    assert!(h.tick(501).await.is_empty());
    assert_eq!(h.sent().len(), n);
    let b = h.wf(TICKET, "board", json!({})).await;
    assert_eq!(b["children"][0]["paused"], true);

    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(501)}));
    assert_eq!(h.wf(WORKER, "pause", json!({"paused": false})).await["paused"], false);
    assert!(h.tick(520).await.is_empty());
    assert_eq!(h.tick(546).await, vec!["nudge #41 PR 1/2 (1/2)"]);
    assert_eq!(h.wf(TICKET, "board", json!({})).await["children"][0]["paused"], false);
}

#[tokio::test]
async fn a_parent_pauses_and_resumes_a_child_by_label() {
    let h = H::new();
    link_ticket_worker(&h).await;
    assert_eq!(h.wf(TICKET, "pause", json!({"child": "#41 PR 1/2"})).await["paused"], true);
    assert!(link(&h, WORKER).paused_at.is_some());
    assert!(h.tick(300).await.is_empty());
    h.wf(TICKET, "pause", json!({"child": WORKER, "paused": false})).await;
    assert_eq!(link(&h, WORKER).paused_at, None);
    assert_eq!(h.tick(301).await, vec!["nudge #41 PR 1/2 (1/2)"]);
    let err = h.wf_err(TICKET, "pause", json!({})).await;
    assert!(err.contains("no parent link to pause"), "{err}");
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    let err = h.wf_err(TICKET, "pause", json!({"child": WORKER})).await;
    assert!(err.contains("closed"), "{err}");
}

#[tokio::test]
async fn a_rejoin_keeps_the_pause() {
    let h = H::new();
    let call = link_ticket_worker(&h).await;
    h.wf(WORKER, "pause", json!({})).await;
    h.exec_as(WORKER, &call).await;
    assert!(link(&h, WORKER).paused_at.is_some());
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
    h.fake.update(WORKER, json!({"status": "running", "updatedAt": at(1)}));
    assert!(h.tick(1).await.is_empty());
    h.fake.update(WORKER, json!({"status": "idle", "pendingBackgroundTasks": [{"id": "b1"}], "updatedAt": at(100)}));
    assert!(h.tick(200).await.is_empty());
    let out = h.tick(241).await;
    assert!(out.iter().any(|a| a.starts_with("report to mcp:ticket")), "{out:?}");
    assert!(h.last_message().contains("sent nothing for 240 min"), "{}", h.last_message());
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
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
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
    h.fake.fail_tool("t3_thread_send");
    let r = h.wf(WORKER, "send", json!({"kind": "blocker", "body": "need a token"})).await;
    assert_eq!(r["delivery"], "queued");
    assert!(r["detail"].as_str().unwrap().contains("delivery failed"));
    h.fake.clear_faults_on("t3_thread_send");
    h.fake.update(WORKER, json!({"status": "running"}));
    assert_eq!(h.tick(3).await, vec!["delivered m1 to mcp:ticket"]);
}

#[tokio::test]
async fn a_failed_delivery_keeps_the_digest_for_the_retry() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.wf(WORKER, "send", json!({"kind": "note", "body": "review clean"})).await;
    h.fake.fail_tool("t3_thread_send");
    h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    h.fake.clear_faults_on("t3_thread_send");
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

#[tokio::test]
async fn init_puts_a_ticket_on_the_board_and_its_gate_becomes_a_card() {
    let h = H::new();
    h.fake.update(TICKET, json!({"title": "widgets#41: fix the sprockets"}));
    let out = h.exec_as(TICKET, "async () => wf.init({ role: 'ticket' })").await;
    let r = result(&out);
    assert_eq!(r["root"], false);
    assert_eq!(r["parent"], "board");
    assert_eq!(r["label"], "widgets#41: fix the sprockets");
    let ln = link(&h, TICKET);
    assert_eq!(ln.params.down, 75);
    assert_eq!(ln.parent, "board");
    assert_eq!(ln.parent_role.as_deref(), Some("overseer"));
    assert_eq!(ln.link.as_deref(), Some("https://t3/mcp:ticket"));

    let b = h.wf(BOSS, "board", json!({"parent": "board"})).await;
    let kids = b["children"].as_array().unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0]["thread"], TICKET);
    assert_eq!(kids[0]["label"], "widgets#41: fix the sprockets");

    let r =
        h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": "reply \"merge\" to merge #81"})).await;
    assert_eq!(r["delivery"], "board");
    assert_eq!(r["card"], "a1");
    assert_eq!(h.wf(BOSS, "cards", json!({})).await[0]["ticket"], TICKET);
    assert!(h.sent().is_empty(), "{:?}", h.sent());
}

#[tokio::test]
async fn init_takes_a_label_and_a_second_init_keeps_the_link() {
    let h = H::new();
    let r = h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    assert_eq!(r["label"], "widgets#41");
    let r = h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    assert_eq!(r["root"], false);
    assert_eq!(r["parent"], "board");
    assert_eq!(r["label"], "widgets#41");
    assert_eq!(h.wf(BOSS, "board", json!({"parent": "board"})).await["children"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn init_stays_a_root_for_overseers_workers_and_root_true() {
    let h = H::new();
    for (me, args) in [
        (BOSS, json!({"role": "overseer"})),
        (TICKET, json!({"role": "ticket", "root": true})),
        (WORKER, json!({"role": "worker"})),
    ] {
        let r = h.wf(me, "init", args).await;
        assert_eq!(r["root"], true, "{r}");
        assert!(r.get("parent").is_none(), "{r}");
        assert!(h.app.store.link_get(me).unwrap().is_none());
    }
    assert!(h.wf(BOSS, "board", json!({"parent": "board"})).await["children"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn init_of_a_joined_ticket_keeps_its_overseer() {
    let h = H::new();
    link_boss_ticket(&h).await;
    let r = h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    assert_eq!(r["root"], false);
    assert_eq!(r["parent"], BOSS);
    assert_eq!(link(&h, TICKET).parent, BOSS);
}

#[tokio::test]
async fn init_keeps_a_link_another_process_opened() {
    let h = H::new();
    link_boss_ticket(&h).await;
    let peer = h.peer();
    let r = peer.wf.dispatch("init", Some(TICKET), json!({"role": "ticket"})).await.unwrap();
    assert_eq!(r["root"], false);
    assert_eq!(r["parent"], BOSS);
    assert_eq!(r["label"], "widgets#41");
    assert_eq!(link(&h, TICKET).parent, BOSS);
}

fn bare_link(child: &str, parent: &str, label: &str) -> Link {
    Link {
        child: child.into(),
        parent: parent.into(),
        label: label.into(),
        role: "ticket".into(),
        parent_role: None,
        params: t3_codemode::config::LinkParams { down: 75, miss: 2, stall: 480 },
        link: None,
        opened: T0.into(),
        last_up: T0.into(),
        last_seen: None,
        last_nudge: None,
        nudges: 0,
        escalated: false,
        reported: Vec::new(),
        gate: None,
        gate_since: None,
        gate_pr: None,
        closed: false,
        closed_at: None,
        closed_how: None,
        unread: Vec::new(),
        bad_run: None,
        settled_at: None,
        restored_at: None,
        base: Default::default(),
        project: None,
        paused_at: None,
        busy_since: None,
        identity: Default::default(),
        brief: None,
        pr: None,
    }
}

#[test]
fn a_join_from_another_handle_during_a_bind_survives() {
    let dir = tempfile::tempdir().unwrap();
    let attach = Store::open(dir.path()).unwrap();
    let joiner = Store::open(dir.path()).unwrap();
    let racer = rusqlite::Connection::open(attach.path()).unwrap();
    racer.busy_timeout(std::time::Duration::ZERO).unwrap();
    let bound = attach
        .link_bind(TICKET, false, |current| {
            assert!(current.is_none());
            let err = racer
                .execute("INSERT INTO links (child, parent, closed, data) VALUES (?1, ?2, 0, '{}')", [TICKET, BOSS])
                .unwrap_err();
            assert_eq!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy), "{err}");
            Bind::Saved(bare_link(TICKET, "board", TICKET))
        })
        .unwrap();
    assert!(matches!(bound, Bind::Saved(_)));
    let joined = joiner.link_bind(TICKET, true, |_| Bind::Saved(bare_link(TICKET, BOSS, "widgets#41"))).unwrap();
    assert!(matches!(joined, Bind::Saved(_)));
    assert_eq!(attach.link_get(TICKET).unwrap().unwrap().parent, BOSS);

    let kept = attach
        .link_bind(TICKET, false, |current| match current {
            Some(l) if !l.closed => Bind::Kept(l),
            _ => Bind::Saved(bare_link(TICKET, "board", TICKET)),
        })
        .unwrap();
    assert!(matches!(kept, Bind::Kept(l) if l.parent == BOSS));
    assert_eq!(joiner.link_get(TICKET).unwrap().unwrap().parent, BOSS);
}

#[tokio::test]
async fn init_leaves_another_launchs_pending_row_alone() {
    for thread in [None, Some("launch:7")] {
        let h = H::new();
        h.app
            .store
            .pending_put(&t3_codemode::store::Pending {
                parent: "board".into(),
                label: "widgets#41".into(),
                role: "ticket".into(),
                params: t3_codemode::config::LinkParams { down: 75, miss: 2, stall: 480 },
                at: T0.into(),
                reported: false,
                thread: thread.map(String::from),
                link: thread.map(|t| format!("https://t3/{t}")),
                project: None,
                identity: t3_codemode::store::Identity { issue: Some("acme/widgets#41".into()), ..Default::default() },
                brief: None,
            })
            .unwrap();
        h.fake.update(TICKET, json!({"title": "widgets#41"}));
        let r = h.wf(TICKET, "init", json!({"role": "ticket"})).await;
        assert_eq!(r["label"], "widgets#41", "{r}");
        assert_eq!(link(&h, TICKET).identity.issue, None, "launched as {thread:?}");
        assert!(h.tick(76).await.contains(&"report never-joined widgets#41".to_string()), "launched as {thread:?}");
    }
}

#[tokio::test]
async fn a_child_launched_from_execute_is_named_in_its_never_joined_report() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
    let old = h.wf(TICKET, "openChild", json!({"role": "worker", "label": "old"})).await["brief"].clone();
    let old_join = old.as_str().unwrap().lines().find(|l| l.contains("wf.join(")).unwrap().to_string();
    let brief = h.wf(TICKET, "openChild", json!({"role": "worker", "label": "lost"})).await["brief"].clone();
    let message = json!(format!("The last worker ran {old_join}\n\n{}", brief.as_str().unwrap()));
    let code =
        format!("async () => t3.call('t3_thread_launch', {{ projectId: 'p', title: 'lost', message: {message} }})");
    assert_eq!(result(&h.exec_as(TICKET, &code).await)["threadId"], "launch:1");
    h.tick(46).await;
    let m = h.last_message();
    assert!(m.contains("lost was opened 46 min ago and never joined. Its thread: https://t3/launch:1."), "{m}");
    assert!(m.contains("old was opened 46 min ago and never joined. Check its launch"), "{m}");
}

#[tokio::test]
async fn init_reopening_a_closed_board_link_drops_its_old_gate() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": "reply \"merge\""})).await;
    let mut ln = link(&h, TICKET);
    ln.closed = true;
    h.app.store.link_save(&ln).unwrap();
    h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    let ln = link(&h, TICKET);
    assert!(!ln.closed);
    assert_eq!(ln.gate, None);
    assert_eq!(ln.gate_since, None);
}

#[tokio::test]
async fn init_with_a_blank_label_falls_back_to_the_title() {
    let h = H::new();
    h.fake.update(TICKET, json!({"title": "Fix sprockets"}));
    let r = h.wf(TICKET, "init", json!({"role": "ticket", "label": "  "})).await;
    assert_eq!(r["label"], "Fix sprockets");
}

#[tokio::test]
async fn init_reopening_a_closed_board_link_forgets_old_reports() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    let mut ln = link(&h, TICKET);
    ln.closed = true;
    ln.reported = vec!["asks@x".into()];
    h.app.store.link_save(&ln).unwrap();
    h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    assert!(link(&h, TICKET).reported.is_empty());
}

#[tokio::test]
async fn options_on_a_send_to_a_thread_parent_are_checked_and_leave_the_body_alone() {
    let h = H::new();
    link_ticket_worker(&h).await;
    let body = "**ACTION NEEDED:** reply \"merge\" to merge #81, here: https://x/pull/81";
    let out = h
        .exec_as(
            WORKER,
            &format!(
                "async () => wf.send({{ kind: 'gate', gate: 'merge', body: {}, options: ['merge'] }})",
                json!(body)
            ),
        )
        .await;
    assert_eq!(result(&out)["delivery"], "sent");
    assert!(h.last_message().ends_with(body), "{}", h.last_message());
    let e = h
        .wf_err(WORKER, "send", json!({"kind": "gate", "gate": "review", "body": "pick", "options": ["a", "A"]}))
        .await;
    assert!(e.contains("options[1]") && e.contains("repeats"), "{e}");
    assert_eq!(link(&h, WORKER).gate.as_deref(), Some("merge"));
}

async fn board_ticket_with_named_worker(h: &H) -> String {
    h.wf(TICKET, "join", json!({"parent": "board", "label": "widgets#105", "role": "ticket"})).await;
    let r = h
        .wf(
            TICKET,
            "openChild",
            json!({"role": "worker", "label": "#115 (#105 1/4)", "issue": "acme/widgets#115",
                   "parentIssue": "https://github.com/acme/widgets/issues/105", "part": "1/4"}),
        )
        .await;
    let brief = r["brief"].as_str().unwrap().to_string();
    let call = brief.lines().find(|l| l.starts_with("async () => wf.join(")).expect("join call in brief").to_string();
    result(&h.exec_as(WORKER, &call).await);
    call
}

#[tokio::test]
async fn a_worker_opened_with_its_issue_shows_it_on_the_board_and_in_the_state() {
    let h = H::new();
    let call = board_ticket_with_named_worker(&h).await;
    assert!(
        call.ends_with(r#"stall: 240, issue: "acme/widgets#115", parentIssue: "acme/widgets#105", part: "1/4" })"#),
        "{call}"
    );
    let want = (json!("acme/widgets#115"), json!("acme/widgets#105"), json!("1/4"));
    let kid = &h.wf(TICKET, "board", json!({})).await["children"][0];
    assert_eq!((kid["issue"].clone(), kid["parentIssue"].clone(), kid["part"].clone()), want);
    let state = h.app.wf.snapshot().unwrap();
    let w = &state["tree"][0]["workers"][0];
    assert_eq!((w["issue"].clone(), w["parentIssue"].clone(), w["part"].clone()), want);
    assert_eq!(state["tree"][0]["issue"], Value::Null);
    let tree = &h.wf(TICKET, "board", json!({"parent": "board"})).await;
    assert_eq!(tree["children"][0]["workers"][0]["part"], "1/4", "{tree}");
}

#[tokio::test]
async fn a_rejoin_without_identity_keeps_the_links_issue() {
    let h = H::new();
    board_ticket_with_named_worker(&h).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#115 (#105 1/4)", "role": "worker"})).await;
    let ln = link(&h, WORKER);
    assert_eq!(ln.identity.issue.as_deref(), Some("acme/widgets#115"));
    assert_eq!(ln.identity.parent_issue.as_deref(), Some("acme/widgets#105"));
    assert_eq!(ln.identity.part.as_deref(), Some("1/4"));
}

#[tokio::test]
async fn a_rejoin_keeps_its_issue_over_another_launch_under_the_same_label() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": "board", "label": "widgets#105", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "w", "role": "worker", "issue": "acme/widgets#115"})).await;
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "w", "issue": "acme/widgets#999", "parentIssue": "acme/widgets#9", "part": "2/2"}),
    )
    .await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "w", "role": "worker"})).await;
    let id = link(&h, WORKER).identity;
    assert_eq!(id.issue.as_deref(), Some("acme/widgets#115"));
    assert_eq!((id.parent_issue, id.part), (None, None));
}

#[tokio::test]
async fn init_stores_the_issue_on_the_board_link_it_joins() {
    let h = H::new();
    h.fake.update(TICKET, json!({"title": "widgets#105"}));
    h.wf(TICKET, "init", json!({"role": "ticket", "issue": "acme/widgets#105"})).await;
    assert_eq!(link(&h, TICKET).identity.issue.as_deref(), Some("acme/widgets#105"));
}

#[tokio::test]
async fn a_malformed_issue_or_part_is_refused() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
    let e = h.wf_err(TICKET, "openChild", json!({"role": "worker", "label": "w", "issue": "widgets 115"})).await;
    assert!(e.contains("issue \"widgets 115\" is not a supported issue reference"), "{e}");
    let e = h.wf_err(TICKET, "openChild", json!({"role": "worker", "label": "w", "part": "0/4"})).await;
    assert!(e.contains("part \"0/4\" must be n/N"), "{e}");
    let e = h
        .wf_err(
            WORKER,
            "join",
            json!({"parent": TICKET, "label": "w", "role": "worker", "parentIssue": "acme/widgets/pull/9"}),
        )
        .await;
    assert!(e.contains("parentIssue"), "{e}");
    assert!(h.app.store.link_get(WORKER).unwrap().is_none());
}

#[test]
fn a_link_row_from_before_identity_still_reads() {
    let mut row = serde_json::to_value(bare_link(WORKER, TICKET, "#41 PR 1/2")).unwrap();
    for k in ["issue", "parentIssue", "part"] {
        assert!(row.get(k).is_none(), "{row}");
    }
    row.as_object_mut().unwrap().remove("project");
    let ln: Link = serde_json::from_value(row).unwrap();
    assert_eq!(ln.identity, Default::default());
}

#[tokio::test]
async fn a_held_reply_to_an_active_worker_is_dropped_when_it_hands_off() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.fake.update(WORKER, json!({"status": "running"}));

    let r = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "wait for CI"})).await;
    assert_eq!(r["delivery"], "queued");
    assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);
    assert!(h.sent().is_empty());

    let handoff = h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert_eq!(handoff["closed"], true);
    assert!(h.app.store.outbox_all().unwrap().is_empty());
    assert_eq!(h.app.wf.reach().count(t3_codemode::reach::ReachPoint::OutboxHeldThenClosed), 1);

    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(1)}));
    h.tick(2).await;

    let to_worker: Vec<_> = h.sent().into_iter().filter(|s| s["threadId"] == WORKER).collect();
    assert!(to_worker.is_empty(), "closed worker received stale reply: {to_worker:?}");
    assert!(h.app.store.outbox_all().unwrap().is_empty());
    assert_eq!(h.sent().len(), 1);
    assert_eq!(h.sent()[0]["threadId"], TICKET);
}

#[tokio::test]
async fn wf_close_drops_queued_outbox_wakes_for_the_closed_child() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.fake.update(WORKER, json!({"status": "running"}));

    let r = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "check lint"})).await;
    assert_eq!(r["delivery"], "queued");
    assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);

    h.wf(TICKET, "close", json!({"child": WORKER})).await;
    assert!(h.app.store.outbox_all().unwrap().is_empty());
    assert_eq!(h.app.wf.reach().count(t3_codemode::reach::ReachPoint::OutboxHeldThenClosed), 1);

    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(1)}));
    h.tick(2).await;

    assert!(h.sent().is_empty(), "{:?}", h.sent());
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn terminal_tick_closures_drop_queued_outbox_wakes() {
    for archive in [false, true] {
        let h = H::new();
        link_ticket_worker(&h).await;
        h.app.store.meta_set(LAST_PASS, T0).unwrap();
        h.fake.update(WORKER, json!({"status": "running"}));

        let r = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "still there?"})).await;
        assert_eq!(r["delivery"], "queued");
        assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);

        if archive {
            h.fake.update(WORKER, json!({"status": "idle", "archived": true}));
        } else {
            h.fake.remove(WORKER);
        }
        h.tick(2).await;

        assert!(link(&h, WORKER).closed);
        assert!(h.sent().is_empty(), "archive={archive}: {:?}", h.sent());
        assert!(h.app.store.outbox_all().unwrap().is_empty());
        assert_eq!(h.app.wf.reach().count(t3_codemode::reach::ReachPoint::OutboxHeldThenClosed), 1);
    }
}

#[tokio::test]
async fn flush_outbox_drops_unreopened_wakes_for_closed_links_but_delivers_reopened_ones() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();

    // Passing reopen: true while the link is still open must NOT exempt the wake from a later close.
    h.fake.update(WORKER, json!({"status": "running"}));
    let held = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "before close", "reopen": true})).await;
    assert_eq!(held["delivery"], "queued");
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert!(h.app.store.outbox_all().unwrap().is_empty());

    // Defense-in-depth: a stale outbox row for an already-closed link is dropped on tick.
    h.app.store.outbox_push(WORKER, "[t3cm reply from=t3cm msg=m99]\nstale", "m99", T0).unwrap();
    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(1)}));
    h.tick(2).await;
    assert_eq!(h.app.wf.reach().count(t3_codemode::reach::ReachPoint::OutboxHeldThenClosed), 2);
    let to_worker: Vec<_> = h.sent().into_iter().filter(|s| s["threadId"] == WORKER).collect();
    assert!(to_worker.is_empty(), "stale row delivered to closed worker: {to_worker:?}");
    assert!(h.app.store.outbox_all().unwrap().is_empty());

    // Explicit reopen: true on a closed link still delivers, even if held while ACTIVE and flushed on tick.
    h.fake.update(WORKER, json!({"status": "running", "updatedAt": at(2)}));
    let reopened = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "one more thing", "reopen": true})).await;
    assert_eq!(reopened["delivery"], "queued");
    assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);
    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(3)}));
    h.tick(4).await;
    let to_worker: Vec<_> = h.sent().into_iter().filter(|s| s["threadId"] == WORKER).collect();
    assert_eq!(to_worker.len(), 1, "{to_worker:?}");
    assert!(to_worker[0]["message"].as_str().unwrap().ends_with("\none more thing"));
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn close_ticket_drops_queued_outbox_wakes() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/1"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.fake.update(TICKET, json!({"status": "running"}));

    let wake = h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    assert_eq!(wake["delivery"], "queued");
    assert_eq!(h.app.store.outbox_all().unwrap().len(), 1);

    h.fake.update(TICKET, json!({"status": "idle", "updatedAt": at(1)}));
    h.app.wf.close_ticket(TICKET, false).await.unwrap();
    assert!(h.app.store.outbox_all().unwrap().is_empty());

    h.tick(2).await;
    assert!(h.sent().is_empty(), "{:?}", h.sent());
}

#[tokio::test]
async fn wake_to_a_closed_parent_is_dropped_and_not_reported_as_queued() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/1"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.wf(TICKET, "send", json!({"kind": "handoff", "body": "ticket done"})).await;

    let r = h.wf(WORKER, "send", json!({"kind": "handoff", "body": "worker done"})).await;
    assert_eq!(r["closed"], true);
    assert_eq!(r["delivery"], "noted", "{r}");
    assert!(r["detail"].as_str().unwrap_or("").contains("closed"), "{r}");
    assert!(h.sent().is_empty(), "{:?}", h.sent());
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn dropped_wake_reports_noted_even_when_a_queued_reopen_reply_flushes_in_the_same_batch() {
    let h = H::new();
    link_boss_ticket(&h).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/1"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();

    h.wf(BOSS, "close", json!({"child": TICKET})).await;
    h.fake.update(TICKET, json!({"status": "running"}));
    let reopened = h.wf(BOSS, "reply", json!({"to": TICKET, "body": "reopen ticket", "reopen": true})).await;
    assert_eq!(reopened["delivery"], "queued", "{reopened}");

    h.fake.update(TICKET, json!({"status": "idle", "updatedAt": at(1)}));
    let r = h.wf(WORKER, "send", json!({"kind": "handoff", "body": "worker done"})).await;
    assert_eq!(r["closed"], true);
    assert_eq!(r["delivery"], "noted", "{r}");
    assert!(r["detail"].as_str().unwrap_or("").contains("closed"), "{r}");
    let to_ticket: Vec<_> = h.sent().into_iter().filter(|s| s["threadId"] == TICKET).collect();
    assert_eq!(to_ticket.len(), 1, "{to_ticket:?}");
    let msg = to_ticket[0]["message"].as_str().unwrap();
    assert!(msg.ends_with("\nreopen ticket") && !msg.contains("worker done"), "{msg}");
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn reopened_reply_held_alongside_a_stale_row_drops_only_the_stale_row() {
    let h = H::new();
    link_ticket_worker(&h).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
    h.wf(TICKET, "close", json!({"child": WORKER})).await;

    h.app.store.outbox_push(WORKER, "[t3cm reply from=parent msg=m98]\nstale", "m98", T0).unwrap();
    h.fake.update(WORKER, json!({"status": "running"}));

    let r = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "reopened", "reopen": true})).await;
    assert_eq!(r["delivery"], "queued", "{r}");
    let outbox = h.app.store.outbox_all().unwrap();
    assert_eq!(outbox.len(), 1, "{outbox:?}");
    assert!(outbox[0].allow_closed);
    assert!(outbox[0].text.ends_with("\nreopened"), "{}", outbox[0].text);

    h.fake.update(WORKER, json!({"status": "idle", "updatedAt": at(1)}));
    h.tick(2).await;
    let to_worker: Vec<_> = h.sent().into_iter().filter(|s| s["threadId"] == WORKER).collect();
    assert_eq!(to_worker.len(), 1, "{to_worker:?}");
    let msg = to_worker[0]["message"].as_str().unwrap();
    assert!(msg.ends_with("\nreopened") && !msg.contains("stale"), "{msg}");
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[test]
fn opening_a_legacy_store_migrates_outbox_allow_closed_to_false() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE outbox (id INTEGER PRIMARY KEY AUTOINCREMENT, to_id TEXT NOT NULL, \
         text TEXT NOT NULL, handle TEXT NOT NULL, queued TEXT NOT NULL);
         INSERT INTO outbox (to_id, text, handle, queued) VALUES ('mcp:worker', 'stale', 'm1', '2026-11-17T12:00:00Z');",
    )
    .unwrap();
    drop(conn);

    let store = Store::open(dir.path()).unwrap();
    let rows = store.outbox_all().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].to, WORKER);
    assert!(!rows[0].allow_closed);
}

#[tokio::test]
async fn answering_a_card_for_a_closed_ticket_errors_and_keeps_the_card_open() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    let card = h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": "reply \"merge\""})).await;
    let id = card["card"].as_str().unwrap().to_string();

    let mut ln = link(&h, TICKET);
    ln.closed = true;
    h.app.store.link_save(&ln).unwrap();

    let err = h.wf_err(BOSS, "answer", json!({"id": id, "text": "merge"})).await;
    assert!(err.contains("closed") && err.contains("dismiss it instead"), "{err}");
    let open = h.wf(BOSS, "cards", json!({})).await;
    assert_eq!(open.as_array().unwrap().len(), 1, "{open}");
    assert_eq!(open[0]["id"], id);
    assert!(h.sent().is_empty(), "{:?}", h.sent());
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

fn handoff_args() -> Value {
    json!({"kind": "handoff", "body": "## Handoff\n\nPR merged."})
}

#[tokio::test]
async fn a_crash_at_any_write_of_a_handoff_still_hands_off_to_the_parent() {
    use futures_util::FutureExt;
    let clean = H::new();
    link_ticket_worker(&clean).await;
    let before = clean.app.store.writes();
    clean.app.wf.dispatch("send", Some(WORKER), handoff_args()).await.unwrap();
    let writes = clean.app.store.writes() - before;
    assert!(writes > 1, "a handoff writes {writes} times");

    for k in 1..=writes {
        let mut h = H::new();
        link_ticket_worker(&h).await;
        h.app.store.set_fault(Some(h.app.store.writes() + k));
        let send = h.app.wf.dispatch("send", Some(WORKER), handoff_args());
        assert!(
            std::panic::AssertUnwindSafe(send).catch_unwind().await.is_err(),
            "write {k} of {writes} did not crash"
        );
        h.reopen();
        let _ = h.app.wf.dispatch("send", Some(WORKER), handoff_args()).await;
        h.app.wf.tick().await.unwrap();
        let handoffs = h
            .sent()
            .iter()
            .filter(|s| s["threadId"] == TICKET && s["message"].as_str().is_some_and(|m| m.contains("[t3cm handoff")))
            .count();
        assert!(handoffs >= 1, "a crash at handoff write {k} of {writes} lost the handoff");
        assert!(link(&h, WORKER).closed, "a crash at handoff write {k} left the link open");
    }
}
