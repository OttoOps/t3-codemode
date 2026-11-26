use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::reach::ReachPoint;
use t3_codemode::t3::T3Client;
use t3_codemode::wf::LAST_PASS;

async fn send(h: &H, id: &str, message: &str) {
    h.fake.call("t3_thread_send", json!({"threadId": id, "message": message})).await.unwrap();
}

async fn link_worker(h: &H) {
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/2"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
}

fn sent_to(h: &H, id: &str) -> Vec<String> {
    h.sent().into_iter().filter(|s| s["threadId"] == id).map(|s| s["message"].as_str().unwrap().to_string()).collect()
}

fn reach(h: &H, point: ReachPoint) -> u64 {
    h.app.wf.reach().count(point)
}

fn status(h: &H, id: &str) -> Value {
    h.fake.thread(id).unwrap()["status"].clone()
}

#[tokio::test]
async fn a_run_started_for_n_minutes_ends_idle_at_its_end_time() {
    let h = H::new();
    h.fake.start_run(WORKER, Some(30));
    let started = h.fake.thread(WORKER).unwrap();

    h.clock.set(&at(29));
    assert_eq!(status(&h, WORKER), "running");
    h.clock.set(&at(31));
    let ended = h.fake.thread(WORKER).unwrap();

    assert_eq!(started["latestRunId"], format!("run:{WORKER}:2"));
    assert_eq!(ended["status"], "idle");
    assert_eq!(ended["updatedAt"], at(30));
    assert_eq!(ended["latestRunId"], started["latestRunId"]);
}

#[tokio::test]
async fn a_send_to_a_running_recipient_extends_its_run() {
    let h = H::new();
    h.fake.run_on_send(WORKER, 20);
    send(&h, WORKER, "first").await;
    h.clock.set(&at(10));
    send(&h, WORKER, "second").await;

    h.clock.set(&at(39));
    assert_eq!(status(&h, WORKER), "running");
    h.clock.set(&at(40));
    assert_eq!(status(&h, WORKER), "idle");
    assert_eq!(h.fake.thread(WORKER).unwrap()["latestRunId"], format!("run:{WORKER}:2"));
}

#[tokio::test]
async fn a_reply_to_a_worker_running_on_send_is_held_until_its_run_ends() {
    let h = H::new();
    link_worker(&h).await;
    h.fake.run_on_send(WORKER, 20);

    let first = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "rebase please"})).await;
    let second = h.wf(TICKET, "reply", json!({"to": WORKER, "body": "and fix lint"})).await;
    h.tick(5).await;
    let held = sent_to(&h, WORKER).len();
    h.tick(21).await;

    assert_eq!(first["delivery"], "sent");
    assert_eq!(second["delivery"], "queued");
    assert_eq!(held, 1);
    assert!(reach(&h, ReachPoint::OutboxHeld) >= 1);
    let to_worker = sent_to(&h, WORKER);
    assert_eq!(to_worker.len(), 2, "{to_worker:?}");
    assert!(to_worker[1].contains("and fix lint"), "{}", to_worker[1]);
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn a_worker_busy_past_its_stall_minutes_is_reported_once() {
    let h = H::new();
    link_worker(&h).await;
    h.fake.start_run(WORKER, None);

    h.tick(1).await;
    h.tick(240).await;
    let early = reach(&h, ReachPoint::StallReported);
    h.tick(241).await;
    h.tick(300).await;

    assert_eq!(early, 0);
    assert_eq!(reach(&h, ReachPoint::StallReported), 1);
    let reports: Vec<String> =
        sent_to(&h, TICKET).into_iter().filter(|m| m.contains("busy but sent nothing")).collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(sent_to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn a_worker_woken_after_a_long_idle_gets_its_full_stall_minutes() {
    let h = H::new();
    link_worker(&h).await;
    h.tick(300).await;
    h.clock.set(&at(400));
    h.fake.start_run(WORKER, None);

    h.tick(401).await;
    h.tick(639).await;
    let early = reach(&h, ReachPoint::StallReported);
    h.tick(641).await;

    assert_eq!(early, 0);
    assert_eq!(reach(&h, ReachPoint::StallReported), 1);
    let reports: Vec<String> =
        sent_to(&h, TICKET).into_iter().filter(|m| m.contains("busy but sent nothing")).collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].contains("for 240 min"), "{}", reports[0]);
}

#[tokio::test]
async fn an_idle_worker_is_escalated_after_its_missed_nudges() {
    let h = H::new();
    link_worker(&h).await;

    h.tick(46).await;
    h.tick(92).await;
    let before = reach(&h, ReachPoint::IdleEscalated);
    h.tick(138).await;
    h.tick(184).await;

    let nudges = sent_to(&h, WORKER).into_iter().filter(|m| m.starts_with("[t3cm nudge")).count();
    assert_eq!(nudges, 2);
    assert_eq!(before, 0);
    assert_eq!(reach(&h, ReachPoint::IdleEscalated), 1);
    let escalations = sent_to(&h, TICKET).into_iter().filter(|m| m.starts_with("[t3cm escalate")).count();
    assert_eq!(escalations, 1);
}

#[tokio::test]
async fn a_run_that_ends_badly_is_nudged_once_at_once() {
    let h = H::new();
    link_worker(&h).await;
    h.fake.start_run(WORKER, Some(60));

    h.tick(5).await;
    h.clock.set(&at(6));
    h.fake.finish_run(WORKER, "failed");
    h.tick(7).await;
    h.tick(8).await;

    assert_eq!(reach(&h, ReachPoint::BadEndNudged), 1);
    let nudges: Vec<String> = sent_to(&h, WORKER).into_iter().filter(|m| m.starts_with("[t3cm nudge")).collect();
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0].contains("Your last run ended failed."), "{}", nudges[0]);
}

#[tokio::test]
async fn an_archived_worker_closes_its_link_and_drops_its_held_wakes() {
    let h = H::new();
    link_worker(&h).await;
    h.fake.start_run(WORKER, None);
    h.wf(TICKET, "reply", json!({"to": WORKER, "body": "hold this"})).await;

    h.fake.archive(WORKER);
    h.tick(1).await;

    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert!(ln.closed);
    assert_eq!(ln.closed_how.as_deref(), Some("archived"));
    assert_eq!(reach(&h, ReachPoint::WorkerArchived), 1);
    assert_eq!(reach(&h, ReachPoint::OutboxHeldThenClosed), 1);
    assert!(sent_to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn a_removed_worker_closes_its_link_as_thread_gone() {
    let h = H::new();
    link_worker(&h).await;

    h.fake.remove(WORKER);
    h.tick(1).await;

    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert!(ln.closed);
    assert_eq!(ln.closed_how.as_deref(), Some("thread-gone"));
    assert_eq!(reach(&h, ReachPoint::WorkerThreadGone), 1);
}

#[tokio::test]
async fn board_tickets_close_when_archived_or_removed() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(BOSS, "join", json!({"parent": BOARD, "label": "widgets#42", "role": "ticket"})).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();

    h.fake.archive(TICKET);
    h.fake.remove(BOSS);
    h.tick(1).await;

    assert_eq!(reach(&h, ReachPoint::TicketArchived), 1);
    assert_eq!(reach(&h, ReachPoint::TicketThreadGone), 1);
    for id in [TICKET, BOSS] {
        assert!(h.app.store.link_get(id).unwrap().unwrap().closed, "{id} still open");
    }
}
