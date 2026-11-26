use crate::common::*;
use serde_json::json;
use t3_codemode::fake::{Gate, Rule};
use t3_codemode::reach::ReachPoint;

async fn link_ticket_worker(h: &H) {
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/2"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
}

#[tokio::test(start_paused = true)]
async fn a_tick_keeps_a_gate_a_peer_process_opened_while_it_read_the_thread() {
    let h = H::new();
    link_ticket_worker(&h).await;
    let peer = h.peer();
    let gate = Gate::new();
    h.fake.fault(Rule::on("t3_thread_read").target(WORKER).call(1).gate(&gate));
    h.clock.set(&at(10));

    let (ticked, ()) = tokio::join!(h.app.wf.tick(), async {
        gate.arrived().await;
        let body = "**ACTION NEEDED:** decide each open review finding";
        peer.wf.dispatch("send", Some(WORKER), json!({"kind": "gate", "gate": "review", "body": body})).await.unwrap();
        gate.release();
    });
    ticked.unwrap();

    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert_eq!(ln.gate.as_deref(), Some("review"), "the tick's save erased the peer's gate");
    assert!(ln.last_seen.is_some(), "the tick's own fields still land: {ln:?}");
    assert_eq!(h.app.wf.reach().count(ReachPoint::LinkSaveMerged), 1);
}
