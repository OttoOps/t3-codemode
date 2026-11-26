use crate::common::*;
use serde_json::{Value, json};
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

const PEER_SEEDS: u64 = 64;
const PEER_PAIRS: usize = 8;
const W2: &str = "mcp:worker2";

struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Clone, Copy, Debug)]
enum Serve {
    Tick,
    Reconcile,
}

#[derive(Clone, Copy, Debug)]
enum Stdio {
    Open(&'static str, &'static str),
    Clear(&'static str),
    Note(&'static str),
    Event(&'static str),
    Reply(&'static str),
}

async fn two_workers(h: &H) {
    let mut th = t3_codemode::fake::thread(W2, "idle", T0);
    th["link"] = json!("https://t3/mcp:worker2");
    h.fake.put(th);
    link_ticket_worker(h).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 2/2"})).await;
    h.wf(W2, "join", json!({"parent": TICKET, "label": "#41 PR 2/2", "role": "worker"})).await;
}

async fn serve(h: &H, op: Serve) {
    match op {
        Serve::Tick => drop(h.app.wf.tick().await.unwrap()),
        Serve::Reconcile => drop(h.app.wf.reconcile_ran().await.unwrap()),
    }
}

async fn call(peer: &t3_codemode::app::App, me: &'static str, method: &'static str, args: Value) -> Value {
    peer.wf.dispatch(method, Some(me), args).await.unwrap_or_else(|e| panic!("{method} as {me}: {e:#}"))
}

async fn stdio(peer: &t3_codemode::app::App, op: Stdio) -> Option<String> {
    let call = |me, method, args| call(peer, me, method, args);
    match op {
        Stdio::Open(w, gate) => drop(call(w, "gate", json!({"open": gate})).await),
        Stdio::Clear(w) => drop(call(w, "gate", json!({"clear": true})).await),
        Stdio::Note(w) => {
            return call(w, "send", json!({"kind": "note", "body": format!("note from {w}")})).await["handle"]
                .as_str()
                .map(String::from);
        }
        Stdio::Event(w) => {
            return call(w, "send", json!({"kind": "event", "body": format!("event from {w}")})).await["handle"]
                .as_str()
                .map(String::from);
        }
        Stdio::Reply(w) => drop(call(TICKET, "reply", json!({"to": w, "body": "carry on"})).await),
    }
    None
}

#[tokio::test(start_paused = true)]
async fn serve_and_stdio_sharing_a_store_lose_no_gate_note_or_wake() {
    let workers = [WORKER, W2];
    let mut merged = 0;
    for seed in 0..PEER_SEEDS {
        let h = H::new();
        two_workers(&h).await;
        let peer = h.peer();
        h.fake.set_jitter(Some(seed));
        let mut rng = Mix(seed);
        let mut gates: std::collections::BTreeMap<&str, Option<&str>> = workers.iter().map(|w| (*w, None)).collect();
        let mut notes = Vec::new();
        let mut events = Vec::new();
        let mut trace = Vec::new();
        for pair in 0..PEER_PAIRS {
            h.clock.set(&at(pair as i64 + 1));
            let a = if rng.pick(2) == 0 { Serve::Tick } else { Serve::Reconcile };
            let w = workers[rng.pick(2)];
            let b = match rng.pick(5) {
                0 => Stdio::Open(w, ["review", "deploy"][rng.pick(2)]),
                1 => Stdio::Clear(w),
                2 => Stdio::Note(w),
                3 => Stdio::Event(w),
                _ => Stdio::Reply(w),
            };
            trace.push(format!("{a:?} | {b:?}"));
            let ((), handle) = tokio::join!(serve(&h, a), stdio(&peer, b));
            match b {
                Stdio::Open(w, g) => drop(gates.insert(w, Some(g))),
                Stdio::Clear(w) => drop(gates.insert(w, None)),
                Stdio::Note(w) => notes.extend(handle.map(|h| (w, h))),
                Stdio::Event(_) => events.extend(handle),
                Stdio::Reply(_) => {}
            }
        }
        h.fake.set_jitter(None);
        h.clock.set(&at(60));
        h.wf(WORKER, "send", json!({"kind": "event", "body": "flush the digest"})).await;
        h.app.wf.tick().await.unwrap();
        let what = format!("seed {seed}: {trace:#?}");
        for w in workers {
            let ln = h.app.store.link_get(w).unwrap().unwrap();
            assert_eq!(ln.gate.as_deref(), gates[w], "{w}'s gate under {what}");
            assert!(!ln.closed, "{w} closed under {what}");
        }
        let sent = h.sent();
        let to_ticket: Vec<&str> =
            sent.iter().filter(|s| s["threadId"] == TICKET).filter_map(|s| s["message"].as_str()).collect();
        let shown = |handle: &str| {
            to_ticket.iter().any(|m| {
                ["]", " "].iter().any(|end| m.contains(&format!("msg={handle}{end}")))
                    || m.contains(&format!("[{handle}]"))
            })
        };
        for handle in &events {
            assert!(shown(handle), "event {handle} never reached the ticket under {what}\nsent: {to_ticket:#?}");
        }
        for (i, (w, handle)) in notes.iter().enumerate() {
            let folded = notes[i..].iter().filter(|(x, _)| x == w).any(|(_, later)| shown(later));
            assert!(folded, "note {handle} from {w} never reached the ticket's digest under {what}");
        }
        for w in workers {
            let unread = h.app.store.link_get(w).unwrap().unwrap().unread;
            assert!(unread.is_empty(), "{w} kept unread notes {unread:?} under {what}");
        }
        assert!(h.app.store.outbox_all().unwrap().is_empty(), "wakes left queued under {what}");
        merged += h.app.wf.reach().count(t3_codemode::reach::ReachPoint::LinkSaveMerged)
            + peer.wf.reach().count(t3_codemode::reach::ReachPoint::LinkSaveMerged);
    }
    assert!(merged > 0, "no seed raced a link save across the two processes");
}
