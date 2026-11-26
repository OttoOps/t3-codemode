use serde_json::json;
use t3_codemode::board::CardRef;
use t3_codemode::fake::{Failure, Gate, Rule};
use t3_codemode::reach::{ReachPoint, ReachSnapshot};
use t3_codemode::wf::LAST_PASS;

use super::races::{ready_card, setup_waiting_merge};
use super::sweeps::{Inject, reach_of};
use super::{HEADS, REPO};
use crate::common::*;

struct Class {
    id: &'static str,
    fault: &'static str,
    points: &'static [ReachPoint],
}

const CLASSES: [Class; 9] = [
    Class { id: "F1", fault: "an external call fails before its effect", points: &[ReachPoint::OutboxSendFailed] },
    Class { id: "F2", fault: "an external call fails after its effect", points: &[ReachPoint::LandedAfterError] },
    Class {
        id: "F3",
        fault: "a call is held at a gate while another op runs",
        points: &[ReachPoint::ReconcileDroppedUntracked],
    },
    Class { id: "F4", fault: "one pull request of a batched read fails", points: &[ReachPoint::ReconcilePrReadFailed] },
    Class { id: "F5", fault: "the store crashes at a write and reopens", points: &[ReachPoint::InCodeMergeResumed] },
    Class {
        id: "F6",
        fault: "a thread's run holds, stalls, or ends badly",
        points: &[ReachPoint::OutboxHeld, ReachPoint::StallReported, ReachPoint::BadEndNudged],
    },
    Class {
        id: "F7",
        fault: "a thread is archived or removed",
        points: &[ReachPoint::WorkerArchived, ReachPoint::WorkerThreadGone],
    },
    Class {
        id: "F8",
        fault: "GitHub state moves under an approval",
        points: &[ReachPoint::ReadyCardDroppedHeadMoved, ReachPoint::ReassignedIdRejected],
    },
    Class { id: "F9", fault: "a peer process saves the same link", points: &[ReachPoint::LinkSaveMerged] },
];

async fn link_worker(h: &H) {
    h.wf(TICKET, "init", json!({"role": "ticket", "root": true})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/2"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
    h.app.store.meta_set(LAST_PASS, T0).unwrap();
}

async fn gated() -> ReachSnapshot {
    let (h, ctx) = ready_card(false).await;
    let gate = Gate::new();
    h.github.fault(Rule::on("reconcile").call(1).gate(&gate));
    let (ran, ()) = tokio::join!(h.app.wf.reconcile_ran(), async {
        gate.arrived().await;
        let card = CardRef { at: None, token: Some(&ctx["token"]) };
        h.app.wf.dismiss_at(&ctx["ready"], &card).await.unwrap();
        gate.release();
    });
    ran.unwrap();
    h.app.wf.reach().snapshot()
}

async fn batch_read_fails() -> ReachSnapshot {
    let (h, _) = ready_card(false).await;
    h.github.fault(Rule::on("reconcile.pr").pr(REPO, 7).call(1).fail(Failure::refused("injected read error")));
    let _ = h.app.wf.reconcile_ran().await;
    h.app.wf.reach().snapshot()
}

async fn run_lifecycle() -> ReachSnapshot {
    let h = H::new();
    link_worker(&h).await;
    h.fake.start_run(WORKER, None);
    h.wf(TICKET, "reply", json!({"to": WORKER, "body": "hold this"})).await;
    h.tick(1).await;
    h.tick(241).await;
    h.clock.set(&at(242));
    h.fake.finish_run(WORKER, "failed");
    h.tick(243).await;
    h.app.wf.reach().snapshot()
}

async fn threads_lost() -> ReachSnapshot {
    let mut reach = ReachSnapshot::default();
    for archive in [true, false] {
        let h = H::new();
        link_worker(&h).await;
        if archive {
            h.fake.archive(WORKER);
        } else {
            h.fake.remove(WORKER);
        }
        h.tick(1).await;
        reach.merge(&h.app.wf.reach().snapshot());
    }
    reach
}

async fn github_drifts() -> ReachSnapshot {
    let mut reach = ReachSnapshot::default();
    let (h, _) = ready_card(false).await;
    h.github.push(REPO, 7, HEADS[1]);
    h.app.wf.reconcile().await.unwrap();
    reach.merge(&h.app.wf.reach().snapshot());
    let (h, _) = setup_waiting_merge(false).await;
    h.github.reassign_id(REPO, 7);
    h.app.wf.reconcile().await.unwrap();
    reach.merge(&h.app.wf.reach().snapshot());
    reach
}

async fn peer_saves() -> ReachSnapshot {
    let h = H::new();
    link_worker(&h).await;
    let peer = h.peer();
    let gate = Gate::new();
    h.fake.fault(Rule::on("t3_thread_read").target(WORKER).call(1).gate(&gate));
    let (ticked, ()) = tokio::join!(h.app.wf.tick(), async {
        gate.arrived().await;
        peer.wf.dispatch("gate", Some(WORKER), json!({"open": "review"})).await.unwrap();
        gate.release();
    });
    ticked.unwrap();
    h.app.wf.reach().snapshot()
}

#[tokio::test(start_paused = true)]
async fn all_nine_fault_classes_reach_the_branches_that_handle_them() {
    for class in &CLASSES {
        let reach = match class.id {
            "F1" => reach_of(|r| r.t3_calls, Inject::T3Fail).await,
            "F2" => reach_of(|r| r.gh_calls, Inject::GitHubFailAfterEffect).await,
            "F3" => gated().await,
            "F4" => batch_read_fails().await,
            "F5" => reach_of(|r| r.writes, Inject::Crash).await,
            "F6" => run_lifecycle().await,
            "F7" => threads_lost().await,
            "F8" => github_drifts().await,
            "F9" => peer_saves().await,
            other => panic!("no injection for {other}"),
        };
        for &point in class.points {
            assert!(reach.get(point) > 0, "{} ({}) never reached {}:\n{reach}", class.id, class.fault, point.name());
        }
    }
}
