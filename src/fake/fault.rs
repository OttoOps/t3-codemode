use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Down(String),
    Refused(String),
    Typed { kind: String, message: String },
    Field { field: String, message: String },
}

impl Failure {
    pub fn down(message: &str) -> Failure {
        Failure::Down(message.into())
    }

    pub fn refused(message: &str) -> Failure {
        Failure::Refused(message.into())
    }

    pub fn typed(kind: &str, message: &str) -> Failure {
        Failure::Typed { kind: kind.into(), message: message.into() }
    }

    pub fn field(field: &str, message: &str) -> Failure {
        Failure::Field { field: field.into(), message: message.into() }
    }

    pub fn message(&self) -> &str {
        match self {
            Failure::Down(m) | Failure::Refused(m) => m,
            Failure::Typed { message, .. } | Failure::Field { message, .. } => message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Any,
    Is(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nth {
    Every,
    Call(usize),
    From(usize),
}

impl Nth {
    fn fires(self, seen: usize) -> bool {
        match self {
            Nth::Every => true,
            Nth::Call(n) => seen == n,
            Nth::From(n) => seen >= n,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Fault {
    Fail(Failure),
    FailAfterEffect(Failure),
    Gate(Gate),
}

#[derive(Clone, Debug)]
pub struct Match {
    op: String,
    target: Target,
    nth: Nth,
}

impl Match {
    pub fn target(mut self, target: &str) -> Match {
        self.target = Target::Is(target.into());
        self
    }

    pub fn pr(self, repo: &str, number: u64) -> Match {
        self.target(&format!("{repo}#{number}"))
    }

    pub fn nth(mut self, nth: Nth) -> Match {
        self.nth = nth;
        self
    }

    pub fn call(self, n: usize) -> Match {
        self.nth(Nth::Call(n))
    }

    pub fn fail(self, failure: Failure) -> Rule {
        Rule { on: self, fault: Fault::Fail(failure) }
    }

    pub fn fail_after_effect(self, failure: Failure) -> Rule {
        Rule { on: self, fault: Fault::FailAfterEffect(failure) }
    }

    pub fn gate(self, gate: &Gate) -> Rule {
        Rule { on: self, fault: Fault::Gate(gate.clone()) }
    }

    fn matches(&self, ops: &[&str], target: Option<&str>) -> bool {
        let op = ops.iter().any(|o| *o == self.op || (self.op == "*" && !o.contains('.')));
        op && match &self.target {
            Target::Any => true,
            Target::Is(want) => target.is_some_and(|t| t.eq_ignore_ascii_case(want)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    on: Match,
    fault: Fault,
}

impl Rule {
    pub fn on(op: &str) -> Match {
        Match { op: op.into(), target: Target::Any, nth: Nth::Every }
    }

    pub fn op(&self) -> &str {
        &self.on.op
    }
}

#[derive(Default)]
pub(crate) struct Hit {
    pub(crate) gates: Vec<Gate>,
    pub(crate) fail: Option<Failure>,
    pub(crate) after_effect: bool,
}

impl Hit {
    pub(crate) async fn pass_gates(&self) {
        for g in &self.gates {
            g.pass().await;
        }
    }

    pub(crate) fn before(&self) -> Option<&Failure> {
        self.fail.as_ref().filter(|_| !self.after_effect)
    }

    pub(crate) fn after(&self) -> Option<&Failure> {
        self.fail.as_ref().filter(|_| self.after_effect)
    }
}

#[derive(Default)]
pub(crate) struct Faults {
    rules: Vec<(Rule, usize)>,
    jitter: Option<u64>,
}

impl Faults {
    pub(crate) fn add(&mut self, rule: Rule) {
        self.rules.push((rule, 0));
    }

    pub(crate) fn add_first(&mut self, rule: Rule) {
        self.rules.insert(0, (rule, 0));
    }

    pub(crate) fn clear(&mut self, op: Option<&str>) {
        self.rules.retain(|(r, _)| op.is_some_and(|op| r.op() != op));
    }

    pub(crate) fn set_jitter(&mut self, seed: Option<u64>) {
        self.jitter = seed;
    }

    pub(crate) fn hit(&mut self, ops: &[&str], target: Option<&str>) -> Hit {
        let mut hit = Hit::default();
        for (rule, seen) in &mut self.rules {
            if !rule.on.matches(ops, target) {
                continue;
            }
            *seen += 1;
            if !rule.on.nth.fires(*seen) {
                continue;
            }
            match &rule.fault {
                Fault::Gate(g) => hit.gates.push(g.clone()),
                Fault::Fail(f) if hit.fail.is_none() => hit.fail = Some(f.clone()),
                Fault::FailAfterEffect(f) if hit.fail.is_none() => {
                    hit.fail = Some(f.clone());
                    hit.after_effect = true;
                }
                _ => {}
            }
        }
        hit
    }

    pub(crate) fn yields(&mut self) -> u64 {
        let Some(state) = self.jitter.as_mut() else { return 0 };
        *state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (z ^ (z >> 31)) % 4
    }
}

pub(crate) async fn jitter(yields: u64) {
    for _ in 0..yields {
        tokio::task::yield_now().await;
    }
}

#[derive(Clone)]
pub struct Gate(Arc<GateState>);

struct GateState {
    arrived: watch::Sender<usize>,
    released: watch::Sender<bool>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate").field("arrivals", &self.count()).field("released", &self.is_released()).finish()
    }
}

impl Default for Gate {
    fn default() -> Gate {
        Gate::new()
    }
}

impl Gate {
    pub fn new() -> Gate {
        Gate(Arc::new(GateState { arrived: watch::Sender::new(0), released: watch::Sender::new(false) }))
    }

    pub async fn arrived(&self) {
        self.arrivals(1).await;
    }

    pub async fn arrivals(&self, n: usize) {
        let mut rx = self.0.arrived.subscribe();
        let _ = rx.wait_for(|seen| *seen >= n).await;
    }

    pub fn count(&self) -> usize {
        *self.0.arrived.borrow()
    }

    pub fn release(&self) {
        self.0.released.send_replace(true);
    }

    pub fn is_released(&self) -> bool {
        *self.0.released.borrow()
    }

    async fn pass(&self) {
        self.0.arrived.send_modify(|n| *n += 1);
        let mut rx = self.0.released.subscribe();
        let _ = rx.wait_for(|open| *open).await;
    }
}
