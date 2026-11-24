use std::collections::{BTreeMap, BTreeSet};

use super::{GhChecks, GhPrModel, HEADS, REPO};
use crate::common::{CardView, LinkView, SeenView, Snapshot, WaitView};

const BOARD: &str = "board";
const GITHUB: &str = "github";
const T3CM: &str = "t3cm";
const MAX_FAILURES: u32 = 3;
const EXIT_WINDOW_MINS: i64 = 30;
const WAKE_KINDS: &[&str] = &["event", "gate", "blocker", "handoff", "closed", "report", "escalate", "reply"];

pub type Wake = (String, String, String);
type Tracked = BTreeMap<u64, (String, Vec<(String, String)>, Vec<u64>)>;

fn ts(s: &str) -> jiff::Timestamp {
    s.parse().unwrap_or_else(|e| panic!("model timestamp {s:?}: {e}"))
}

fn later(event: &str, since: &str) -> bool {
    ts(event) >= ts(since)
}

fn same_head(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

fn earliest(a: &str, b: &str) -> bool {
    ts(a) < ts(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Plain,
    Held,
    NoReason,
}

#[derive(Clone, Debug)]
pub struct CardM {
    pub seq: u64,
    pub kind: &'static str,
    pub ticket: Option<String>,
    pub worker: Option<(String, String)>,
    pub pr: Option<u64>,
    pub head: Option<String>,
    pub ready: bool,
    pub at: String,
    pub tag: Tag,
    pub options: Option<Vec<String>>,
}

impl CardM {
    fn own_gate(&self, t: &str) -> bool {
        self.kind == "gate" && self.worker.is_none() && self.ticket.as_deref() == Some(t)
    }

    fn view(&self) -> CardView {
        CardView {
            kind: self.kind.into(),
            ticket: self.ticket.clone(),
            worker: self.worker.clone(),
            pr: self.pr.map(|n| (REPO.into(), n)),
            head: self.head.clone(),
            ready: self.ready,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrM {
    pub number: u64,
    pub head: String,
    pub waiting: bool,
    pub no_ci: bool,
}

#[derive(Clone, Debug)]
pub struct LinkM {
    pub parent: String,
    pub open: bool,
    pub gate: Option<String>,
    pub gate_pr: Option<u64>,
    pub pr: Option<PrM>,
    pub label: String,
    pub opened: String,
    pub worker: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum St {
    Waiting,
    Sent,
}

#[derive(Clone, Debug)]
pub struct WaitM {
    pub head: String,
    pub id_ok: bool,
    pub thread: String,
    pub at: String,
    pub status: St,
    pub approval: String,
    pub failures: u32,
}

#[derive(Clone, Debug, Default)]
struct SeenM {
    state: String,
    head: String,
    queued: bool,
    since: String,
    tickets: Vec<(String, String)>,
    left: Option<String>,
    queued_at: Option<String>,
    draft: bool,
    merge_state: String,
    checks: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cs {
    Passed,
    Pending,
    Failing,
    Unread,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ready {
    Yes,
    Failing,
    Not,
}

fn gh_head(m: &GhPrModel) -> String {
    HEADS[m.head_idx].to_string()
}

fn check_state(m: &GhPrModel) -> Cs {
    match m.checks {
        GhChecks::Pending => Cs::Pending,
        GhChecks::Passing | GhChecks::Cancelled => Cs::Passed,
        GhChecks::Failing => Cs::Failing,
        GhChecks::Unread => Cs::Unread,
    }
}

fn seen_checks(m: &GhPrModel) -> Option<&'static str> {
    match check_state(m) {
        Cs::Passed => Some("passing"),
        Cs::Pending => Some("pending"),
        Cs::Failing => Some("failing"),
        Cs::Unread => None,
    }
}

fn readiness(m: &GhPrModel) -> Ready {
    match (m.merge_state, check_state(m)) {
        ("CLEAN", Cs::Failing) => Ready::Failing,
        ("CLEAN", Cs::Pending) => Ready::Not,
        ("CLEAN", _) => Ready::Yes,
        _ => Ready::Not,
    }
}

fn commit_of(m: &GhPrModel) -> String {
    m.merge_commit.clone().unwrap_or_else(|| HEADS[(m.head_idx + 1) % HEADS.len()].to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Src {
    Direct,
    Reconcile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proj {
    pub links: BTreeMap<String, LinkView>,
    pub cards: Vec<CardView>,
    pub waits: BTreeMap<(String, u64), WaitView>,
    pub events: BTreeSet<(String, u64, String, String)>,
    pub seen: BTreeMap<(String, u64), SeenView>,
    pub outbox: BTreeMap<String, usize>,
}

pub struct Predicted {
    pub gh: BTreeMap<u64, GhPrModel>,
    pub ok: bool,
    pub attempts: u64,
}

pub fn project(s: &Snapshot) -> Proj {
    Proj {
        links: s.links.clone(),
        cards: s.cards.iter().filter(|c| c.kind != "escalate").cloned().collect(),
        waits: s.merge_waits.clone(),
        events: s.pr_events.clone(),
        seen: s.pr_seen.clone(),
        outbox: s.outbox.clone(),
    }
}

struct Code {
    ticket: String,
}

#[derive(Clone)]
pub struct Model {
    pub links: BTreeMap<String, LinkM>,
    pub cards: Vec<CardM>,
    seq: u64,
    pub waits: BTreeMap<u64, WaitM>,
    seen: BTreeMap<u64, SeenM>,
    pub events: BTreeSet<(u64, String, String)>,
    live: BTreeMap<String, BTreeSet<u64>>,
    fake_prs: BTreeMap<String, BTreeSet<u64>>,
    merges: u64,
    floor: String,
    now: String,
    pub wakes: Vec<Wake>,
    pub retired: BTreeSet<(String, u64)>,
}

impl Model {
    pub fn new(floor: &str) -> Model {
        Model {
            links: BTreeMap::new(),
            cards: Vec::new(),
            seq: 0,
            waits: BTreeMap::new(),
            seen: BTreeMap::new(),
            events: BTreeSet::new(),
            live: BTreeMap::new(),
            fake_prs: BTreeMap::new(),
            merges: 0,
            floor: floor.into(),
            now: floor.into(),
            wakes: Vec::new(),
            retired: BTreeSet::new(),
        }
    }

    pub fn step(&mut self, now: &str) {
        self.now = now.into();
        self.wakes.clear();
    }

    pub fn proj(&self) -> Proj {
        let links = self
            .links
            .iter()
            .map(|(id, l)| {
                let pr = l.pr.as_ref().map(|p| (REPO.into(), p.number, p.head.clone(), p.waiting));
                let view = LinkView {
                    parent: l.parent.clone(),
                    closed: !l.open,
                    gate: l.gate.clone(),
                    gate_pr: l.gate_pr.map(|n| (REPO.into(), n)),
                    pr,
                };
                (id.clone(), view)
            })
            .collect();
        let waits = self
            .waits
            .iter()
            .map(|(n, w)| {
                ((REPO.into(), *n), (format!("{:?}", w.status), w.head.clone(), Some(w.thread.clone()), w.failures))
            })
            .collect();
        Proj {
            links,
            cards: self.cards.iter().map(CardM::view).collect(),
            waits,
            events: self.events.iter().map(|(n, e, t)| (REPO.into(), *n, e.clone(), t.clone())).collect(),
            seen: self
                .seen
                .iter()
                .map(|(n, r)| ((REPO.into(), *n), (r.queued, r.since.clone(), r.left.clone(), r.queued_at.clone())))
                .collect(),
            outbox: BTreeMap::new(),
        }
    }

    pub fn label(&self, id: &str) -> String {
        self.links[id].label.clone()
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.links.get(id).is_some_and(|l| l.open)
    }

    fn deliver(&mut self, to: &str, kind: &str, from: &str) {
        if self.is_open(to) {
            self.wakes.push((to.into(), kind.into(), from.into()));
        }
    }

    fn link(&mut self, id: &str) -> &mut LinkM {
        self.links.get_mut(id).unwrap_or_else(|| panic!("model has no link {id}"))
    }

    fn add_card(&mut self, mut c: CardM) {
        self.seq += 1;
        c.seq = self.seq;
        c.at = self.now.clone();
        self.cards.push(c);
    }

    fn resolve(&mut self, mut pred: impl FnMut(&CardM) -> bool) -> Vec<CardM> {
        let (gone, kept): (Vec<CardM>, Vec<CardM>) = std::mem::take(&mut self.cards).into_iter().partition(|c| pred(c));
        self.cards = kept;
        gone
    }

    fn release(&mut self, tickets: &[String]) {
        for t in tickets {
            let held = self.cards.iter().any(|c| c.own_gate(t));
            if let Some(l) = self.links.get_mut(t).filter(|l| l.open && l.gate.as_deref() == Some("merge"))
                && !held
            {
                l.gate = None;
                l.gate_pr = None;
            }
        }
    }

    fn sync_merge_gate_pr(&mut self, t: &str) {
        let Some(l) = self.links.get(t).filter(|l| l.open && l.gate.as_deref() == Some("merge")) else { return };
        let Some(n) = l.gate_pr else { return };
        if self.cards.iter().any(|c| c.own_gate(t) && c.pr == Some(n)) {
            return;
        }
        let next = self.cards.iter().find(|c| c.own_gate(t) && c.ready).and_then(|c| c.pr);
        self.link(t).gate_pr = next;
    }

    fn sync_relay_gate(&mut self, t: &str) {
        let Some(l) = self.links.get(t).filter(|l| l.open) else { return };
        if l.gate.as_deref().is_some_and(|g| !g.starts_with("relay:")) {
            return;
        }
        let held = self.cards.iter().rev().filter(|c| c.ticket.as_deref() == Some(t)).find_map(|c| c.worker.clone());
        self.link(t).gate = held.map(|(_, g)| format!("relay:{g}"));
    }

    fn release_and_sync(&mut self, tickets: &[String]) {
        self.release(tickets);
        for t in tickets {
            self.sync_merge_gate_pr(t);
            self.sync_relay_gate(t);
        }
    }

    fn take_merge_gate(&mut self, t: &str, pr: u64) {
        let l = self.link(t);
        if l.gate.as_deref().is_none_or(|g| g == "merge" || g.starts_with("relay:")) {
            if l.gate.as_deref() != Some("merge") {
                l.gate_pr = Some(pr);
            }
            l.gate = Some("merge".into());
        }
    }

    fn board_card(&mut self, c: CardM) {
        if c.kind == "gate"
            && let Some(t) = c.ticket.clone()
        {
            let from = c.worker.as_ref().map(|w| w.0.clone());
            self.resolve(|o| {
                o.kind == "gate"
                    && o.ticket.as_deref() == Some(t.as_str())
                    && o.worker.as_ref().map(|w| &w.0) == from.as_ref()
                    && !o.ready
            });
        }
        self.add_card(c);
    }

    fn card(kind: &'static str, ticket: Option<&str>, worker: Option<(String, String)>) -> CardM {
        CardM {
            seq: 0,
            kind,
            ticket: ticket.map(String::from),
            worker,
            pr: None,
            head: None,
            ready: false,
            at: String::new(),
            tag: Tag::Plain,
            options: None,
        }
    }

    pub fn join_ticket(&mut self, t: &str, label: &str) {
        let opened = self.now.clone();
        let l = LinkM {
            parent: BOARD.into(),
            open: true,
            gate: None,
            gate_pr: None,
            pr: None,
            label: label.into(),
            opened,
            worker: false,
        };
        self.links.insert(t.into(), l);
    }

    pub fn join_worker(&mut self, w: &str, parent: &str, label: &str) {
        let opened = self.now.clone();
        let l = LinkM {
            parent: parent.into(),
            open: true,
            gate: None,
            gate_pr: None,
            pr: None,
            label: label.into(),
            opened,
            worker: true,
        };
        self.links.insert(w.into(), l);
    }

    pub fn parent(&self, w: &str) -> String {
        self.links[w].parent.clone()
    }

    fn ticket_ok(&self, w: &str) -> bool {
        let parent = &self.links[w].parent;
        self.links.get(parent).is_some_and(|t| t.open && t.parent == BOARD)
    }

    fn remember(&mut self, w: &str, number: u64, head: &str, waiting: bool, ready: bool, opened: Option<bool>) {
        let l = self.link(w);
        match l.pr.as_mut() {
            Some(p) if p.number == number => {
                if ready || !same_head(&p.head, head) {
                    p.head = head.into();
                    p.waiting = waiting;
                } else if head.len() > p.head.len() {
                    p.head = head.into();
                }
                if let Some(no_ci) = opened {
                    p.no_ci = no_ci;
                }
            }
            Some(_) => panic!("model: chaos workers never send a follow-up PR"),
            None => {
                l.pr = Some(PrM { number, head: head.into(), waiting, no_ci: opened.unwrap_or(false) });
            }
        }
    }

    fn worker_send(&mut self, w: &str, kind: &str, gate: Option<&str>, expect: Option<Option<PrM>>) -> &'static str {
        let parent = self.parent(w);
        if !self.is_open(w) {
            return if expect.is_some() { "superseded" } else { "error" };
        }
        if expect.as_ref().is_some_and(|seen| self.links[w].pr != *seen) {
            return "superseded";
        }
        if expect.is_some()
            && let Some(p) = self.link(w).pr.as_mut()
        {
            p.waiting = false;
        }
        if kind == "gate" {
            let l = self.link(w);
            l.gate = Some(gate.unwrap_or("user").into());
            l.gate_pr = None;
        }
        let closing = matches!(kind, "handoff" | "closed");
        if closing {
            self.drop_relayed(w, &parent);
        }
        let gate_now = self.links[w].gate.clone();
        let relays = kind == "blocker"
            || (kind == "gate" && !gate_now.as_deref().is_some_and(|g| g == "merge" || g.starts_with("delivery:")));
        if kind == "gate" && !relays {
            let gone = self.resolve(|c| c.kind == "gate" && c.worker.as_ref().is_some_and(|x| x.0 == w));
            if !gone.is_empty() {
                self.sync_relay_gate(&parent);
            }
        }
        let wake = WAKE_KINDS.contains(&kind);
        if wake && relays && self.links.get(&parent).is_some_and(|t| t.open && t.parent == BOARD) {
            let g = if kind == "gate" { gate_now.unwrap_or_default() } else { kind.to_string() };
            self.board_card(Self::card(
                if kind == "gate" { "gate" } else { "blocker" },
                Some(&parent),
                Some((w.into(), g)),
            ));
            self.sync_relay_gate(&parent);
            return "board";
        }
        if closing {
            let l = self.link(w);
            l.open = false;
            l.gate = None;
            l.gate_pr = None;
        }
        let label = self.label(w);
        self.deliver(&parent, kind, &label);
        "ticket"
    }

    fn drop_relayed(&mut self, w: &str, t: &str) {
        let gone = self.resolve(|c| c.worker.as_ref().is_some_and(|x| x.0 == w));
        if !gone.is_empty() {
            self.sync_relay_gate(t);
        }
    }

    fn reply_worker(&mut self, w: &str) {
        self.deliver(w, "reply", T3CM);
    }

    pub fn pr_opened(&mut self, w: &str, number: u64, sha: &str, no_ci: bool, gh: &BTreeMap<u64, GhPrModel>) -> String {
        let ok = self.ticket_ok(w);
        self.remember(w, number, sha, false, false, Some(no_ci));
        if ok {
            self.refresh(w);
        }
        if !ok || !gh[&number].exists || no_ci {
            self.worker_send(w, "event", None, None);
            return "ticket".into();
        }
        "open".into()
    }

    pub fn pr_updated(&mut self, w: &str, number: u64, sha: &str) -> String {
        let no_ci = self.links[w].pr.as_ref().is_some_and(|p| p.no_ci);
        if !self.ticket_ok(w) || no_ci {
            self.remember(w, number, sha, false, false, None);
            self.worker_send(w, "event", None, None);
            return "ticket".into();
        }
        let moved = self.links[w].pr.as_ref().is_none_or(|p| !same_head(&p.head, sha));
        self.remember(w, number, sha, false, false, None);
        let t = self.parent(w);
        let stale = self.resolve(|c| {
            c.own_gate(&t) && c.pr == Some(number) && c.head.as_deref().is_some_and(|h| !same_head(h, sha))
        });
        if !stale.is_empty() {
            self.release_and_sync(std::slice::from_ref(&t));
        }
        if moved && self.links[w].gate.as_deref() == Some("merge") {
            self.link(w).gate = None;
        }
        "recorded".into()
    }

    pub fn pr_ready(&mut self, w: &str, number: u64, sha: &str, gh: &BTreeMap<u64, GhPrModel>) -> String {
        let no_ci = self.links[w].pr.as_ref().is_some_and(|p| p.no_ci);
        if !self.ticket_ok(w) || no_ci {
            self.worker_send(w, "gate", Some("merge"), None);
            return "ticket".into();
        }
        self.ready_check(w, number, sha, gh, Src::Direct).into()
    }

    fn refuse(&mut self, w: &str, src: Src) -> &'static str {
        if src == Src::Reconcile {
            self.reply_worker(w);
            if let Some(p) = self.link(w).pr.as_mut() {
                p.waiting = false;
            }
        }
        "refused"
    }

    fn ready_check(
        &mut self,
        w: &str,
        number: u64,
        sha: &str,
        gh: &BTreeMap<u64, GhPrModel>,
        src: Src,
    ) -> &'static str {
        let m = &gh[&number];
        let seen = self.links[w].pr.clone();
        if !m.exists {
            return self.fallback(w, seen);
        }
        let head = gh_head(m);
        if m.state != "OPEN" || m.draft || !same_head(&head, sha) {
            return self.refuse(w, src);
        }
        let cs = check_state(m);
        match cs {
            Cs::Failing => {
                if src == Src::Reconcile {
                    self.reply_worker(w);
                }
                self.remember(w, number, &head, false, true, None);
                return "failing";
            }
            Cs::Unread => return self.fallback(w, seen),
            _ => {}
        }
        if cs == Cs::Pending || m.merge_state == "UNKNOWN" {
            self.remember(w, number, &head, true, true, None);
            return "waiting";
        }
        if m.merge_state == "DIRTY" {
            return self.refuse(w, src);
        }
        self.refresh(w);
        let t = self.parent(w);
        self.resolve(|c| c.own_gate(&t) && c.pr == Some(number));
        let mut c = Self::card("gate", Some(&t), None);
        c.pr = Some(number);
        c.head = Some(head.clone());
        c.ready = true;
        self.add_card(c);
        self.take_merge_gate(&t, number);
        self.remember(w, number, &head, false, true, None);
        let l = self.link(w);
        if l.gate.is_none() {
            l.gate = Some("merge".into());
        }
        "gated"
    }

    fn fallback(&mut self, w: &str, seen: Option<PrM>) -> &'static str {
        match self.worker_send(w, "gate", Some("merge"), Some(seen)) {
            "superseded" => "superseded",
            _ => "ticket",
        }
    }

    pub fn adopt(&mut self, to: &str, from: &str) {
        let children: Vec<String> =
            self.links.iter().filter(|(_, l)| l.open && l.parent == from).map(|(k, _)| k.clone()).collect();
        let prs: Vec<u64> = children.iter().filter_map(|c| self.links[c].pr.as_ref().map(|p| p.number)).collect();
        let moved: Vec<u64> = self
            .cards
            .iter()
            .filter(|c| {
                c.ticket.as_deref() == Some(from)
                    && (c.worker.as_ref().is_some_and(|w| children.contains(&w.0))
                        || (c.ready && c.pr.is_some_and(|p| prs.contains(&p))))
            })
            .map(|c| c.seq)
            .collect();
        let moved_cards: Vec<CardM> = self.cards.iter().filter(|c| moved.contains(&c.seq)).cloned().collect();
        for c in self.cards.iter_mut().filter(|c| moved.contains(&c.seq)) {
            c.ticket = Some(to.into());
        }
        let mut seen_prs: Vec<u64> = prs.clone();
        for c in &children {
            for p in self.live.get(c).cloned().unwrap_or_default() {
                if !seen_prs.contains(&p) {
                    seen_prs.push(p);
                }
            }
        }
        for c in &moved_cards {
            if let Some(p) = c.pr.filter(|p| !seen_prs.contains(p)) {
                seen_prs.push(p);
            }
        }
        for c in &children {
            self.link(c).parent = to.into();
        }
        for p in &seen_prs {
            let Some(seen) = self.seen.get(p) else { continue };
            let Some(pos) = seen.tickets.iter().position(|(x, _)| x == from) else { continue };
            let still =
                self.cards.iter().any(|c| c.worker.is_none() && c.ticket.as_deref() == Some(from) && c.pr == Some(*p))
                    || self.waits.get(p).is_some_and(|w| w.thread == from);
            if still {
                continue;
            }
            let seen = self.seen.get_mut(p).unwrap();
            let (_, since) = seen.tickets.remove(pos);
            match seen.tickets.iter_mut().find(|(x, _)| x == to) {
                Some((_, s)) if earliest(&since, s) => *s = since,
                Some(_) => {}
                None => seen.tickets.push((to.into(), since)),
            }
            self.retired.insert((from.into(), *p));
        }
        let moved_merge_pr = moved_cards.iter().filter(|c| c.worker.is_none()).find_map(|c| c.pr).or_else(|| {
            self.cards.iter().find_map(|c| {
                (c.ticket.as_deref() == Some(to) && c.worker.is_none() && c.ready)
                    .then_some(c.pr)
                    .flatten()
                    .filter(|p| prs.contains(p))
            })
        });
        if let Some(pr) = moved_merge_pr {
            self.release(&[from.to_string()]);
            self.sync_merge_gate_pr(from);
            if self.is_open(to) {
                self.take_merge_gate(to, pr);
            }
        }
        if !moved.is_empty() || moved_merge_pr.is_some() {
            self.sync_relay_gate(from);
            self.sync_relay_gate(to);
        }
    }

    pub fn ticket_gate_clear(&mut self, t: &str) {
        let l = self.link(t);
        let was_merge = l.gate.as_deref() == Some("merge");
        let prev = l.gate_pr;
        l.gate = None;
        l.gate_pr = None;
        self.resolve(|c| c.own_gate(t) && (!c.ready || (was_merge && prev.is_some() && c.pr == prev)));
        let next = self.cards.iter().find(|c| c.own_gate(t) && c.ready).and_then(|c| c.pr);
        match next {
            Some(n) => {
                let l = self.link(t);
                l.gate = Some("merge".into());
                l.gate_pr = Some(n);
            }
            None => self.sync_relay_gate(t),
        }
    }

    pub fn ticket_gate_open(&mut self, t: &str, name: &str) {
        let l = self.link(t);
        l.gate = Some(name.into());
        l.gate_pr = None;
    }

    pub fn ticket_send_gate(&mut self, t: &str, name: &str) {
        let l = self.link(t);
        l.gate = Some(name.into());
        l.gate_pr = None;
        self.board_card(Self::card("gate", Some(t), None));
    }

    pub fn worker_gate(&mut self, w: &str, name: &str, options: &[&str]) {
        if self.worker_send(w, "gate", Some(name), None) == "board" {
            self.cards.last_mut().unwrap().options = Some(options.iter().map(|o| o.to_string()).collect());
        }
    }

    pub fn close(&mut self, w: &str) {
        let parent = self.parent(w);
        let l = self.link(w);
        l.open = false;
        l.gate = None;
        l.gate_pr = None;
        self.drop_relayed(w, &parent);
    }

    pub fn handoff(&mut self, w: &str) {
        self.worker_send(w, "handoff", None, None);
    }

    pub fn answer(&mut self, seq: Option<u64>, ticket: Option<&str>, text: &str, shown: &[String]) {
        let Some(seq) = seq else {
            if let Some(t) = ticket {
                self.deliver(t, "reply", BOARD);
            }
            return;
        };
        let Some(card) = self.cards.iter().find(|c| c.seq == seq).cloned() else { return };
        let Some(t) = card.ticket.clone() else { return };
        if let Some(want) = &card.options {
            assert_eq!(
                shown,
                want.as_slice(),
                "the board shows other options than the worker sent for {:?}",
                card.view()
            );
        }
        let options = card.options.clone().unwrap_or_else(|| shown.to_vec());
        if let Some((w, _)) = &card.worker {
            let merge = text.get(..5).is_some_and(|x| x.eq_ignore_ascii_case("merge"));
            let passes =
                !merge && (text.eq_ignore_ascii_case("hold") || options.iter().any(|o| o.eq_ignore_ascii_case(text)));
            if passes && self.is_open(w) {
                self.deliver(&w.clone(), "reply", BOARD);
                self.resolve(|c| c.seq == card.seq);
                self.sync_relay_gate(&t);
                return;
            }
        }
        self.deliver(&t, "reply", BOARD);
        self.resolve(|c| c.seq == card.seq);
        if card.kind == "gate" && card.worker.is_none() && card.ready {
            let l = self.link(&t);
            if l.open && l.gate.as_deref() == Some("merge") && l.gate_pr.is_some() {
                l.gate_pr = None;
            }
        }
        if card.worker.is_some() {
            self.sync_relay_gate(&t);
        }
    }

    pub fn dismiss(&mut self, seq: Option<u64>, ticket: Option<&str>, relayed: bool) {
        if let Some(seq) = seq {
            self.resolve(|c| c.seq == seq);
        }
        if let Some(t) = ticket {
            if relayed {
                self.sync_relay_gate(t);
            } else {
                self.sync_merge_gate_pr(t);
            }
        }
    }

    pub fn tick_may(&self) -> Vec<Wake> {
        let mut may = Vec::new();
        for (id, l) in &self.links {
            if !l.open || l.gate.is_some() {
                continue;
            }
            if l.pr.as_ref().is_some_and(|p| p.waiting) {
                continue;
            }
            may.push((id.clone(), "nudge".into(), "watcher".into()));
            if l.parent != BOARD {
                may.push((l.parent.clone(), "escalate".into(), l.label.clone()));
            }
        }
        may
    }

    fn refresh(&mut self, w: &str) {
        let prs = self.fake_prs.get(w).cloned().unwrap_or_default();
        self.live.insert(w.into(), prs);
    }

    pub fn tick(&mut self) {
        let open: Vec<String> = self.links.iter().filter(|(_, l)| l.open && l.worker).map(|(k, _)| k.clone()).collect();
        for w in open {
            self.refresh(&w);
        }
    }

    pub fn set_prs(&mut self, w: &str, pr: u64) {
        self.fake_prs.insert(w.into(), BTreeSet::from([pr]));
    }

    pub fn wait_put(&mut self, pr: u64, w: WaitM) {
        self.waits.insert(pr, w);
    }

    fn mark(&mut self, pr: u64, event: &str, to: &str) -> bool {
        self.events.insert((pr, event.into(), to.into()))
    }

    fn raise(&mut self, pr: u64, ticket: Option<&str>, kind: &'static str, head: &str, event: Option<&str>, tag: Tag) {
        let open = ticket.filter(|t| self.is_open(t)).map(String::from);
        let mut c = Self::card(kind, open.as_deref(), None);
        c.pr = Some(pr);
        c.head = Some(head.into());
        c.tag = tag;
        if open.is_none() {
            c.kind = "report";
        }
        self.add_card(c);
        if let Some(t) = &open
            && kind == "gate"
        {
            self.take_merge_gate(t, pr);
        }
        if let Some(e) = event {
            let to = open.clone().or(ticket.map(String::from)).unwrap_or_else(|| BOARD.into());
            self.mark(pr, e, &to);
        }
    }

    fn tracked(&self) -> Tracked {
        let mut map: Tracked = BTreeMap::new();
        fn add(map: &mut Tracked, pr: u64, ticket: Option<&str>, since: &str) {
            let e = map.entry(pr).or_default();
            if e.0.is_empty() || earliest(since, &e.0) {
                e.0 = since.into();
            }
            let Some(t) = ticket.filter(|t| *t != BOARD) else { return };
            match e.1.iter_mut().find(|(x, _)| x == t) {
                Some((_, s)) if earliest(since, s) => *s = since.into(),
                Some(_) => {}
                None => e.1.push((t.into(), since.into())),
            }
        }
        for (id, l) in self.links.iter().filter(|(_, l)| l.open && l.worker) {
            for p in self.live.get(id).cloned().unwrap_or_default() {
                add(&mut map, p, Some(&l.parent), &l.opened);
            }
            if let Some(p) = l.pr.as_ref().filter(|p| p.waiting) {
                add(&mut map, p.number, Some(&l.parent), &l.opened);
            }
        }
        for c in self.cards.iter().filter(|c| c.worker.is_none()) {
            if let Some(p) = c.pr {
                add(&mut map, p, c.ticket.as_deref(), &c.at);
                map.get_mut(&p).unwrap().2.push(c.seq);
            }
        }
        for (p, w) in &self.waits {
            add(&mut map, *p, Some(&w.thread), &w.at);
        }
        for (p, row) in &self.seen {
            let live = (row.queued || row.left.is_some()) && row.state == "OPEN";
            if !live && !map.contains_key(p) {
                continue;
            }
            add(&mut map, *p, None, &row.since);
            for (t, s) in row.tickets.clone() {
                add(&mut map, *p, Some(&t), &s);
            }
        }
        let floor = self.floor.clone();
        for e in map.values_mut() {
            if earliest(&e.0, &floor) {
                e.0 = floor.clone();
            }
            for (_, s) in e.1.iter_mut() {
                if earliest(s, &floor) {
                    *s = floor.clone();
                }
            }
        }
        map
    }

    fn live_tickets(&self, pr: u64, seqs: &[u64]) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add = |t: &str, since: &str| {
            if t == BOARD {
                return;
            }
            match out.iter_mut().find(|(x, _)| x == t) {
                Some((_, s)) if earliest(since, s) => *s = since.into(),
                Some(_) => {}
                None => out.push((t.into(), since.into())),
            }
        };
        for c in self.cards.iter().filter(|c| seqs.contains(&c.seq)) {
            if let Some(t) = &c.ticket {
                add(t, &c.at);
            }
        }
        if let Some(w) = self.waits.get(&pr) {
            add(&w.thread, &w.at);
        }
        for (id, l) in self.links.iter().filter(|(_, l)| l.open && l.worker) {
            let ours =
                l.pr.as_ref().is_some_and(|p| p.number == pr) || self.live.get(id).is_some_and(|s| s.contains(&pr));
            if ours {
                add(&l.parent, &l.opened);
            }
        }
        if let Some(row) = self.seen.get(&pr) {
            for (t, s) in &row.tickets {
                add(t, s);
            }
        }
        for (_, s) in out.iter_mut() {
            if earliest(s, &self.floor) {
                *s = self.floor.clone();
            }
        }
        out
    }

    pub fn reconcile(&mut self, gh: &BTreeMap<u64, GhPrModel>) -> Predicted {
        let mut gh = gh.clone();
        let tracked = self.tracked();
        let mut unread = Vec::new();
        for (&pr, (t_since, _, seqs)) in &tracked {
            let m = gh[&pr].clone();
            if !m.exists {
                self.seen.remove(&pr);
                if let Some(w) = self.waits.get(&pr).cloned() {
                    self.failed(pr, w, None);
                }
                continue;
            }
            if m.read_err {
                unread.push(pr);
                continue;
            }
            self.fix(pr, &m, t_since, seqs, &mut gh);
        }
        self.seen.retain(|p, _| tracked.contains_key(p));
        let waiting: Vec<(String, PrM)> = self
            .links
            .iter()
            .filter(|(_, l)| l.open)
            .filter_map(|(k, l)| l.pr.clone().filter(|p| p.waiting).map(|p| (k.clone(), p)))
            .collect();
        for (w, p) in waiting {
            if unread.contains(&p.number) {
                continue;
            }
            let due = match self.seen.get(&p.number) {
                None => true,
                Some(s) => {
                    s.state != "OPEN"
                        || s.draft
                        || !same_head(&s.head, &p.head)
                        || matches!(s.checks, None | Some("failing"))
                        || (s.checks != Some("pending") && s.merge_state != "UNKNOWN")
                }
            };
            if !due {
                continue;
            }
            if self.links[&w].pr.as_ref() != Some(&p) {
                continue;
            }
            if !self.ticket_ok(&w) {
                self.fallback(&w, Some(p.clone()));
                continue;
            }
            self.ready_check(&w, p.number, &p.head, &gh, Src::Reconcile);
        }
        Predicted { gh, ok: unread.is_empty(), attempts: self.merges }
    }

    pub fn attempts(&self) -> u64 {
        self.merges
    }

    fn fix(&mut self, pr: u64, m: &GhPrModel, t_since: &str, seqs: &[u64], gh: &mut BTreeMap<u64, GhPrModel>) {
        let head = gh_head(m);
        let wait = self.waits.get(&pr).cloned();
        let sent = wait.as_ref().filter(|w| w.status == St::Sent).map(|w| w.at.clone());
        let status = wait.as_ref().map(|w| w.status);
        let prev = self.seen.get(&pr).cloned();
        let live = self.live_tickets(pr, seqs);
        let since = live
            .iter()
            .map(|(_, s)| s.clone())
            .chain(std::iter::once(t_since.to_string()))
            .min_by_key(|s| ts(s))
            .unwrap();
        let left = if m.state != "OPEN" || m.queued {
            None
        } else if prev.as_ref().is_some_and(|p| p.queued) {
            Some(self.now.clone())
        } else {
            let kept = prev.as_ref().and_then(|p| p.left.clone());
            match &sent {
                Some(approved) => kept.filter(|l| later(l, approved)).or_else(|| Some(self.now.clone())),
                None => kept,
            }
        };
        let after = match prev.as_ref().and_then(|p| p.queued_at.clone()) {
            Some(q) if later(&q, &since) => q,
            _ => since.clone(),
        };
        let mut code = None;
        let mut advance = None;
        match m.state {
            "MERGED" | "CLOSED" => code = self.finished(pr, m, seqs, wait.as_ref(), &live, &since),
            _ => advance = self.open(pr, m, seqs, wait, &live, &after, left.as_deref()),
        }
        let queued_at = if m.state == "OPEN" && m.queued {
            Some(self.now.clone())
        } else {
            prev.as_ref().and_then(|p| p.queued_at.clone())
        };
        let left = match left {
            Some(l) => self.exited(pr, m, &l, &live, &after, status),
            None => None,
        };
        let row = SeenM {
            state: m.state.into(),
            head: head.clone(),
            queued: m.queued,
            since: since.clone(),
            tickets: live,
            left,
            queued_at,
            draft: m.draft,
            merge_state: m.merge_state.into(),
            checks: seen_checks(m),
        };
        self.seen.insert(pr, row);
        if let Some(c) = code {
            self.after_merge(pr, &c.ticket);
        }
        if let Some(w) = advance {
            self.advance(pr, m, w, gh);
        }
    }

    fn finished(
        &mut self,
        pr: u64,
        m: &GhPrModel,
        seqs: &[u64],
        wait: Option<&WaitM>,
        live: &[(String, String)],
        since: &str,
    ) -> Option<Code> {
        let merged = m.state == "MERGED";
        let end = m.ended_at.clone().unwrap_or_else(|| self.now.clone());
        let head = gh_head(m);
        let cards: Vec<CardM> = self.cards.iter().filter(|c| seqs.contains(&c.seq)).cloned().collect();
        let stale: Vec<CardM> = cards.iter().filter(|c| !earliest(&end, &c.at)).cloned().collect();
        let code = if merged && later(&end, since) {
            cards
                .iter()
                .find(|c| {
                    c.kind == "gate"
                        && c.worker.is_none()
                        && c.ready
                        && c.head.as_deref().is_some_and(|h| same_head(h, &head))
                })
                .and_then(|c| c.ticket.clone())
                .filter(|t| self.links.get(t).is_some_and(|l| l.open && l.parent == BOARD))
                .filter(|t| {
                    self.links
                        .values()
                        .any(|l| l.open && l.worker && l.parent == *t && l.pr.as_ref().is_some_and(|p| p.number == pr))
                })
                .map(|ticket| Code { ticket })
        } else {
            None
        };
        let event = if merged { format!("merged:{}", commit_of(m)) } else { format!("closed:{end}") };
        let targets: Vec<String> = live.iter().filter(|(_, s)| later(&end, s)).map(|(t, _)| t.clone()).collect();
        let mut told = 0;
        for t in &targets {
            if code.as_ref().is_some_and(|c| c.ticket == *t) {
                self.mark(pr, &event, t);
            } else if self.is_open(t) && self.mark(pr, &event, t) {
                self.deliver(t, "event", GITHUB);
                told += 1;
            }
        }
        if told == 0 && code.is_none() && later(&end, since) && (wait.is_some() || !stale.is_empty()) {
            self.mark(pr, &event, BOARD);
        }
        let stale_seqs: Vec<u64> = stale.iter().map(|c| c.seq).collect();
        let gated: Vec<String> = stale.iter().filter(|c| c.kind == "gate").filter_map(|c| c.ticket.clone()).collect();
        self.resolve(|c| stale_seqs.contains(&c.seq));
        self.release_and_sync(&gated);
        if wait.is_some() {
            self.waits.remove(&pr);
        }
        code
    }

    fn after_merge(&mut self, pr: u64, ticket: &str) {
        let worker = self
            .links
            .iter()
            .find(|(_, l)| l.open && l.worker && l.parent == ticket && l.pr.as_ref().is_some_and(|p| p.number == pr))
            .map(|(k, _)| k.clone())
            .unwrap();
        if self.events.contains(&(pr, "merged-in-code".into(), worker.clone())) {
            return;
        }
        let t = self.parent(&worker);
        if !self.events.contains(&(pr, "merged-in-code:ticket".into(), t.clone())) {
            if self.is_open(&t) {
                self.deliver(&t, "event", T3CM);
            }
            self.mark(pr, "merged-in-code:ticket", &t);
        }
        let own: Vec<u64> = self.cards.iter().filter(|c| c.own_gate(&t) && c.pr == Some(pr)).map(|c| c.seq).collect();
        self.resolve(|c| own.contains(&c.seq));
        self.release_and_sync(std::slice::from_ref(&t));
        self.waits.remove(&pr);
        self.mark(pr, "merged-in-code", &worker);
    }

    #[allow(clippy::too_many_arguments)]
    fn open(
        &mut self,
        pr: u64,
        m: &GhPrModel,
        seqs: &[u64],
        mut wait: Option<WaitM>,
        live: &[(String, String)],
        after: &str,
        left: Option<&str>,
    ) -> Option<WaitM> {
        let head = gh_head(m);
        if self.dropped(pr, m, live, after) {
            wait.take();
            self.waits.remove(&pr);
        }
        self.resolve(|c| {
            c.kind == "report"
                && c.pr == Some(pr)
                && c.tag == Tag::Held
                && c.head.as_deref().is_some_and(|h| !same_head(&head, h))
        });
        let ready = readiness(m);
        if m.queued || ready == Ready::Yes {
            let queued = m.queued;
            self.resolve(|c| {
                c.kind == "report" && c.pr == Some(pr) && (c.tag == Tag::Held || (queued && c.tag == Tag::NoReason))
            });
        }
        let mut moved: Vec<(String, String, Vec<u64>)> = Vec::new();
        let gates: Vec<CardM> =
            self.cards.iter().filter(|c| seqs.contains(&c.seq) && c.kind == "gate").cloned().collect();
        for c in gates {
            let Some(t) = c.ticket.clone() else { continue };
            if !self.cards.iter().any(|x| x.seq == c.seq) {
                continue;
            }
            let ready_at = c.head.clone().filter(|_| c.ready);
            if ready_at.is_some_and(|b| {
                !same_head(&head, &b) && !wait.as_ref().is_some_and(|w| w.thread == t && same_head(&w.head, &b))
            }) {
                self.resolve(|x| x.seq == c.seq);
                self.release_and_sync(std::slice::from_ref(&t));
                let told: Vec<String> = self
                    .links
                    .iter()
                    .filter(|(_, l)| {
                        l.open
                            && l.worker
                            && l.parent == t
                            && l.gate.as_deref() == Some("merge")
                            && l.pr.as_ref().is_some_and(|p| p.number == pr && !same_head(&head, &p.head))
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for w in told {
                    self.link(&w).gate = None;
                    self.reply_worker(&w);
                }
                continue;
            }
            if !self.links.get(&t).is_some_and(|l| l.open && l.gate.as_deref() == Some("merge")) {
                continue;
            }
            let Some(base) = c.head.clone() else {
                let x = self.cards.iter_mut().find(|x| x.seq == c.seq).unwrap();
                x.head = Some(head.clone());
                x.ready = false;
                continue;
            };
            if same_head(&head, &base) {
                continue;
            }
            match moved.iter_mut().find(|(x, _, _)| *x == t) {
                Some((_, _, s)) => s.push(c.seq),
                None => moved.push((t, base, vec![c.seq])),
            }
        }
        if let Some(w) = &wait
            && w.head != head
            && !moved.iter().any(|(x, _, _)| *x == w.thread)
        {
            moved.push((w.thread.clone(), w.head.clone(), Vec::new()));
        }
        for (t, old) in moved.into_iter().map(|(t, _, old)| (t, old)) {
            if wait.as_ref().is_some_and(|w| w.thread == t && w.head == head) {
                self.resolve(|c| old.contains(&c.seq));
                continue;
            }
            let kind = if m.queued { "report" } else { "gate" };
            self.raise(pr, Some(&t), kind, &head, None, Tag::Plain);
            self.resolve(|c| old.contains(&c.seq));
            if kind == "report" {
                self.release_and_sync(std::slice::from_ref(&t));
            }
            if wait.as_ref().is_some_and(|w| w.thread == t) {
                wait = None;
                self.waits.remove(&pr);
            }
        }
        let mut w = wait.filter(|w| w.head == head)?;
        match (w.status, m.queued) {
            (St::Waiting, true) => {
                w.status = St::Sent;
                self.waits.insert(pr, w);
                None
            }
            (St::Waiting, false) if !m.draft && ready != Ready::Yes => {
                if ready == Ready::Failing {
                    let event = format!("checks:{}", w.approval);
                    if !self.events.contains(&(pr, event.clone(), w.thread.clone())) {
                        self.raise(pr, Some(&w.thread.clone()), "report", &head, Some(&event), Tag::Held);
                    }
                }
                None
            }
            (St::Waiting, false) if !m.draft => {
                if w.id_ok {
                    return Some(w);
                }
                self.raise(pr, Some(&w.thread.clone()), "report", &head, None, Tag::Plain);
                self.waits.remove(&pr);
                None
            }
            (St::Sent, true) if w.failures > 0 => {
                w.failures = 0;
                self.waits.insert(pr, w);
                None
            }
            (St::Sent, false) => {
                let event = left.map(|l| format!("left:{l}"));
                self.failed(pr, w, event.as_deref());
                None
            }
            _ => None,
        }
    }

    fn failed(&mut self, pr: u64, mut w: WaitM, event: Option<&str>) {
        w.failures += 1;
        if w.failures < MAX_FAILURES {
            self.waits.insert(pr, w);
            return;
        }
        let head = w.head.clone();
        self.raise(pr, Some(&w.thread), "report", &head, event, Tag::Plain);
        self.waits.remove(&pr);
    }

    fn dropped(&mut self, pr: u64, m: &GhPrModel, live: &[(String, String)], after: &str) -> bool {
        let Some((_, at)) = m.removed.clone().filter(|(r, _)| !m.queued && *r != "merged") else { return false };
        if !later(&at, after) {
            return false;
        }
        if self.waits.get(&pr).is_some_and(|w| earliest(&at, &w.at)) {
            return false;
        }
        let event = format!("removed:{at}");
        if self.report_exit(pr, m, live, &at, &event, Tag::Plain) {
            self.resolve(|c| c.kind == "report" && c.pr == Some(pr) && c.tag == Tag::NoReason);
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn report_exit(
        &mut self,
        pr: u64,
        m: &GhPrModel,
        live: &[(String, String)],
        at: &str,
        event: &str,
        tag: Tag,
    ) -> bool {
        let mut to: Vec<Option<String>> =
            live.iter().filter(|(_, s)| later(at, s)).map(|(t, _)| Some(t.clone())).collect();
        if to.is_empty() {
            to.push(None);
        }
        let head = gh_head(m);
        let mut raised = false;
        for t in to {
            let key = t.clone().unwrap_or_else(|| BOARD.into());
            if self.events.contains(&(pr, event.into(), key)) {
                continue;
            }
            self.raise(pr, t.as_deref(), "report", &head, Some(event), tag);
            raised = true;
        }
        raised
    }

    #[allow(clippy::too_many_arguments)]
    fn exited(
        &mut self,
        pr: u64,
        m: &GhPrModel,
        left: &str,
        live: &[(String, String)],
        after: &str,
        status: Option<St>,
    ) -> Option<String> {
        if m.removed.as_ref().is_some_and(|(_, at)| later(at, after)) {
            return None;
        }
        let mins = (ts(&self.now).as_second() - ts(left).as_second()) / 60;
        if status == Some(St::Sent) || mins < EXIT_WINDOW_MINS {
            return Some(left.into());
        }
        if status.is_some() {
            return None;
        }
        self.report_exit(pr, m, live, left, &format!("left:{left}"), Tag::NoReason);
        None
    }

    fn advance(&mut self, pr: u64, m: &GhPrModel, mut w: WaitM, gh: &mut BTreeMap<u64, GhPrModel>) {
        self.merges += 1;
        let g = gh.get_mut(&pr).unwrap();
        if m.queue_enabled {
            g.queued = true;
        } else {
            g.state = "MERGED";
            g.queued = false;
            g.merge_commit = Some(format!("{:040x}", self.merges));
            g.ended_at = None;
        }
        w.status = St::Sent;
        w.failures = 0;
        self.waits.insert(pr, w);
        if m.queue_enabled
            && let Some(seen) = self.seen.get_mut(&pr)
        {
            seen.queued = true;
            seen.queued_at = Some(self.now.clone());
        }
    }

    fn tracks(&self, t: &str, pr: u64) -> bool {
        self.cards.iter().any(|c| c.ticket.as_deref() == Some(t) && c.pr == Some(pr))
            || self.waits.get(&pr).is_some_and(|w| w.thread == t)
            || self.links.iter().any(|(id, l)| {
                l.open
                    && l.worker
                    && l.parent == t
                    && (l.pr.as_ref().is_some_and(|p| p.number == pr)
                        || self.live.get(id).is_some_and(|s| s.contains(&pr)))
            })
    }

    pub fn forbidden_github(&mut self) -> BTreeSet<(String, u64)> {
        let regained: Vec<(String, u64)> = self.retired.iter().filter(|(t, p)| self.tracks(t, *p)).cloned().collect();
        for r in regained {
            self.retired.remove(&r);
        }
        let mut out = self.retired.clone();
        for (pr, event, t) in &self.events {
            if event == "merged-in-code:ticket" {
                out.insert((t.clone(), *pr));
            }
        }
        out
    }

    pub fn card_ref(&self, s: &Snapshot, index: usize) -> Option<u64> {
        if s.cards[index].kind == "escalate" {
            return None;
        }
        let rank = s.cards[..index].iter().filter(|c| c.kind != "escalate").count();
        Some(self.cards.get(rank).unwrap_or_else(|| panic!("model has no card at rank {rank}")).seq)
    }
}

pub fn wakes_of(all: &[(String, String, String)]) -> BTreeMap<Wake, usize> {
    let mut m = BTreeMap::new();
    for w in all {
        *m.entry(w.clone()).or_insert(0) += 1;
    }
    m
}
