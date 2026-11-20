mod common;

use common::*;
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
    h.wf(TICKET, "reply", json!({"to": WORKER, "body": "x", "reopen": true})).await;
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
        closed: false,
        closed_at: None,
        closed_how: None,
        unread: Vec::new(),
        bad_run: None,
        settled_at: None,
        restored_at: None,
        project: None,
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
            thread: None,
            link: None,
        })
        .unwrap();
    h.wf(TICKET, "init", json!({"role": "ticket", "label": "widgets#41"})).await;
    assert_eq!(h.app.store.pending_all().unwrap().len(), 1);
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
