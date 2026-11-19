use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clock::{local_clock, parse_ts};
use crate::store::{Limit, PickRecord};
use crate::wf::{ACTIVE, Wf};

pub const STRATEGIES: &[&str] = &["weighted", "priority"];
const EFFORT_IDS: &[&str] = &["effort", "reasoningEffort", "thinking_level"];
const THREAD_PAGE: u64 = 200;

fn weighted() -> String {
    "weighted".into()
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolConfig {
    pub driver: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default = "weighted")]
    pub strategy: String,
    pub members: Vec<PoolMember>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolMember {
    pub instance_id: String,
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default)]
    pub reserve: f64,
}

impl PoolConfig {
    pub fn validate(&self) -> Result<()> {
        if self.driver.trim().is_empty() {
            bail!("driver is empty");
        }
        if self.model.trim().is_empty() {
            bail!("model is empty");
        }
        match self.strategy.as_str() {
            s if STRATEGIES.contains(&s) => {}
            "headroom" => {
                bail!("strategy headroom needs a usage source, which t3cm does not have yet; use weighted or priority")
            }
            s => bail!("unknown strategy {s:?}; use {}", STRATEGIES.join(" or ")),
        }
        if self.members.is_empty() {
            bail!("a pool needs at least one member");
        }
        for (i, m) in self.members.iter().enumerate() {
            if m.instance_id.trim().is_empty() {
                bail!("member {} has an empty instanceId", i + 1);
            }
            if self.members[..i].iter().any(|o| o.instance_id == m.instance_id) {
                bail!("member {} is listed twice", m.instance_id);
            }
            if m.weight == 0 {
                bail!("member {}: weight must be at least 1", m.instance_id);
            }
            if !(0.0..=100.0).contains(&m.reserve) {
                bail!("member {}: reserve must be between 0 and 100, got {}", m.instance_id, m.reserve);
            }
        }
        Ok(())
    }
}

pub fn check_value(name: &str, raw: &Value) -> Result<PoolConfig> {
    let pool: PoolConfig = serde_json::from_value(raw.clone()).with_context(|| format!("pool {name}"))?;
    pool.validate().with_context(|| format!("pool {name}"))?;
    Ok(pool)
}

fn providers(caps: &Value) -> &[Value] {
    caps["providers"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn provider<'a>(caps: &'a Value, id: &str) -> Option<&'a Value> {
    providers(caps).iter().find(|p| p["providerInstanceId"] == id)
}

pub fn check_members(name: &str, pool: &PoolConfig, caps: &Value) -> Result<()> {
    for m in &pool.members {
        let Some(p) = provider(caps, &m.instance_id) else {
            let known: Vec<&str> = providers(caps).iter().filter_map(|p| p["providerInstanceId"].as_str()).collect();
            bail!(
                "pool {name}: member {} is not a T3 provider instance; the instances are {}",
                m.instance_id,
                if known.is_empty() { "none".into() } else { known.join(", ") }
            );
        };
        let driver = p["driverKind"].as_str().unwrap_or_default();
        if driver != pool.driver {
            bail!("pool {name}: member {} is a {driver} instance, not {}", m.instance_id, pool.driver);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Ready,
    Limited(String),
    Disabled,
    Constrained(String),
    MissingModel,
    MissingEffort,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Ready => "ready",
            State::Limited(_) => "limited",
            State::Disabled => "disabled",
            State::Constrained(_) => "constrained",
            State::MissingModel => "missing-model",
            State::MissingEffort => "missing-effort",
        }
    }

    fn why(&self, pool: &PoolConfig) -> Option<String> {
        match self {
            State::Ready => None,
            State::Limited(until) => Some(format!("limited until {}", local_clock(until))),
            State::Disabled => Some("disabled".into()),
            State::Constrained(c) => Some(format!("constrained: {c}")),
            State::MissingModel => Some(format!("does not list model {}", pool.model)),
            State::MissingEffort => {
                Some(format!("does not offer effort {} for {}", pool.effort.as_deref().unwrap_or_default(), pool.model))
            }
        }
    }
}

fn active_limit<'a>(limits: &'a BTreeMap<String, Limit>, id: &str, now: Timestamp) -> Option<&'a Limit> {
    limits.get(id).filter(|l| parse_ts(&l.until).is_some_and(|u| u > now))
}

fn member_state(pool: &PoolConfig, p: &Value, limit: Option<&Limit>) -> State {
    let constraints: Vec<&str> = p["constraints"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if constraints.iter().any(|c| c.to_ascii_lowercase().contains("disabled")) {
        return State::Disabled;
    }
    if !constraints.is_empty() {
        return State::Constrained(constraints.join("; "));
    }
    if !p["models"].as_array().is_some_and(|ms| ms.iter().any(|m| m["id"] == pool.model.as_str())) {
        return State::MissingModel;
    }
    if pool.effort.is_some() && effort_options(pool, p).is_none() {
        return State::MissingEffort;
    }
    match limit {
        Some(l) => State::Limited(l.until.clone()),
        None => State::Ready,
    }
}

fn effort_options(pool: &PoolConfig, p: &Value) -> Option<Value> {
    let Some(effort) = &pool.effort else { return Some(json!([])) };
    let options = p["models"]
        .as_array()
        .and_then(|ms| ms.iter().find(|m| m["id"] == pool.model.as_str()))
        .and_then(|m| m["options"].as_array())?;
    let option = EFFORT_IDS.iter().find_map(|id| options.iter().find(|o| o["id"] == *id))?;
    let offered = option["values"].as_array().is_some_and(|vs| vs.iter().any(|v| v == effort.as_str()));
    offered.then(|| json!([{"id": option["id"], "value": effort}]))
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub instance_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub instance_id: String,
    pub options: Value,
    pub reason: String,
    pub skipped: Vec<Skipped>,
    pub credits: BTreeMap<String, i64>,
}

pub struct Inputs<'a> {
    pub name: &'a str,
    pub pool: &'a PoolConfig,
    pub caps: &'a Value,
    pub running: &'a BTreeMap<String, u64>,
    pub limits: &'a BTreeMap<String, Limit>,
    pub credits: &'a BTreeMap<String, i64>,
    pub now: Timestamp,
}

fn with_skips(base: String, skipped: &[Skipped]) -> String {
    if skipped.is_empty() {
        return base;
    }
    let list: Vec<String> = skipped.iter().map(|s| format!("{} {}", s.instance_id, s.reason)).collect();
    format!("{base}; skipped {}", list.join(", "))
}

pub fn choose(inp: &Inputs) -> Result<Choice> {
    let Inputs { name, pool, caps, running, limits, credits, now } = *inp;
    check_members(name, pool, caps)?;
    let mut skipped = Vec::new();
    let mut ready = Vec::new();
    for (i, m) in pool.members.iter().enumerate() {
        let p = provider(caps, &m.instance_id).expect("checked members");
        let state = member_state(pool, p, active_limit(limits, &m.instance_id, now));
        match state.why(pool) {
            Some(why) => skipped.push(Skipped { instance_id: m.instance_id.clone(), reason: why }),
            None => ready.push(i),
        }
    }
    let (open, held): (Vec<usize>, Vec<usize>) = ready.iter().partition(|&&i| pool.members[i].reserve <= 0.0);
    let last_resort = open.is_empty() && !held.is_empty();
    let candidates = if open.is_empty() { held } else { open };
    if candidates.is_empty() {
        let list: Vec<String> = skipped.iter().map(|s| format!("{} {}", s.instance_id, s.reason)).collect();
        bail!("pool {name} has no member that can run: {}", list.join("; "));
    }
    if !last_resort {
        for &i in &ready {
            let m = &pool.members[i];
            if !candidates.contains(&i) {
                skipped.push(Skipped {
                    instance_id: m.instance_id.clone(),
                    reason: format!("held at its {}% reserve until every other member is out", m.reserve),
                });
            }
        }
    }
    let mut credits = credits.clone();
    let (chosen, base) = match pool.strategy.as_str() {
        "priority" => (candidates[0], "priority: the first member that can run".to_string()),
        _ => {
            let total: i64 = candidates.iter().map(|&i| pool.members[i].weight as i64).sum();
            for &i in &candidates {
                let m = &pool.members[i];
                *credits.entry(m.instance_id.clone()).or_default() += m.weight as i64;
            }
            let load = |i: usize| running.get(&pool.members[i].instance_id).copied().unwrap_or(0);
            let credit = |i: usize| credits[&pool.members[i].instance_id];
            let best = candidates
                .iter()
                .copied()
                .reduce(|a, b| {
                    if (credit(b), std::cmp::Reverse(load(b))) > (credit(a), std::cmp::Reverse(load(a))) {
                        b
                    } else {
                        a
                    }
                })
                .expect("candidates is not empty");
            *credits.get_mut(&pool.members[best].instance_id).expect("credited") -= total;
            (best, format!("weighted: weight {} of {total}", pool.members[best].weight))
        }
    };
    let m = &pool.members[chosen];
    let base = if last_resort {
        format!("last resort: {} holds a {}% reserve and every other member is out; {base}", m.instance_id, m.reserve)
    } else {
        base
    };
    let p = provider(caps, &m.instance_id).expect("checked members");
    Ok(Choice {
        instance_id: m.instance_id.clone(),
        options: effort_options(pool, p).expect("a ready member offers the effort"),
        reason: with_skips(base, &skipped),
        skipped,
        credits,
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    pub pool: String,
    pub instance_id: String,
    pub model: String,
    pub options: Value,
    pub reason: String,
    pub skipped: Vec<Skipped>,
    pub model_selection: Value,
    #[serde(skip)]
    pub seq: i64,
}

impl Picked {
    pub fn summary(&self) -> String {
        format!("on {} ({})", self.instance_id, self.reason)
    }
}

pub fn model_selection(instance_id: &str, model: &str, options: &Value) -> Value {
    let mut m = json!({"instanceId": instance_id, "model": model});
    if options.as_array().is_some_and(|o| !o.is_empty()) {
        m["options"] = options.clone();
    }
    m
}

impl Wf {
    pub(crate) fn pool(&self, name: &str) -> Result<&PoolConfig> {
        self.config.pools.get(name).ok_or_else(|| {
            let known: Vec<&str> = self.config.pools.keys().map(String::as_str).collect();
            anyhow!(
                "no pool {name:?}; configured pools: {}",
                if known.is_empty() { "none".into() } else { known.join(", ") }
            )
        })
    }

    async fn capabilities(&self, model: Option<&str>) -> Result<Value> {
        let input = match model {
            Some(m) => json!({"search": m}),
            None => json!({}),
        };
        Ok(self.t3.call("orchestrator_capabilities", input).await?)
    }

    async fn running_by_instance(&self) -> Result<BTreeMap<String, u64>> {
        let mut out = BTreeMap::new();
        let mut cursor: Option<u64> = None;
        loop {
            let mut input = json!({"allProjects": true, "statuses": ACTIVE, "limit": THREAD_PAGE});
            if let Some(c) = cursor {
                input["cursor"] = json!(c);
            }
            let r = self.t3.call("t3_thread_list", input).await?;
            let threads = r["threads"].as_array().map(Vec::as_slice).unwrap_or_default();
            for t in threads {
                if let Some(id) = t["providerInstanceId"].as_str() {
                    *out.entry(id.to_string()).or_default() += 1;
                }
            }
            match r["nextCursor"].as_u64() {
                Some(c) if !threads.is_empty() && cursor.is_none_or(|p| c > p) => cursor = Some(c),
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn check_pools(&self) -> Result<()> {
        if self.config.pools.is_empty() {
            return Ok(());
        }
        let caps = match self.capabilities(None).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "pools not checked at start, T3 did not answer ({}); each pick checks them",
                    self.t3.redact(format!("{e:#}"))
                );
                return Ok(());
            }
        };
        for (name, pool) in &self.config.pools {
            check_members(name, pool, &caps)?;
        }
        Ok(())
    }

    pub async fn pick(&self, name: &str) -> Result<Picked> {
        let pool = self.pool(name)?;
        let caps = self.capabilities(Some(&pool.model)).await?;
        check_members(name, pool, &caps)?;
        let running = match pool.strategy.as_str() {
            "priority" => BTreeMap::new(),
            _ => self.running_by_instance().await?,
        };
        let _g = self.pool_lock.lock().await;
        let limits = self.store.limits()?;
        let credits = self.store.pool_credits(name)?;
        let now = self.clock.now();
        let c =
            choose(&Inputs { name, pool, caps: &caps, running: &running, limits: &limits, credits: &credits, now })?;
        let at = self.now();
        self.store.pool_credits_put(name, &c.credits)?;
        let seq = self.store.pick_add(&PickRecord {
            pool: name.into(),
            instance_id: c.instance_id.clone(),
            model: pool.model.clone(),
            strategy: pool.strategy.clone(),
            reason: c.reason.clone(),
            skipped: serde_json::to_value(&c.skipped)?,
            at,
            handle: None,
            thread: None,
        })?;
        Ok(Picked {
            pool: name.into(),
            model_selection: model_selection(&c.instance_id, &pool.model, &c.options),
            instance_id: c.instance_id,
            model: pool.model.clone(),
            options: c.options,
            reason: c.reason,
            skipped: c.skipped,
            seq,
        })
    }

    pub async fn pools(&self) -> Result<Value> {
        let running = self.running_by_instance().await;
        let limits = self.store.limits()?;
        let now = self.clock.now();
        let mut out = Vec::new();
        for (name, pool) in &self.config.pools {
            let caps = match &running {
                Ok(_) => self.capabilities(Some(&pool.model)).await,
                Err(e) => Err(anyhow!("{e:#}")),
            };
            let error = match &caps {
                Ok(c) => check_members(name, pool, c).err().map(|e| format!("{e:#}")),
                Err(e) => Some(format!("T3 did not answer: {e:#}")),
            };
            let stats = self.store.pick_stats(name)?;
            let members: Vec<Value> = pool
                .members
                .iter()
                .map(|m| {
                    let limit = active_limit(&limits, &m.instance_id, now);
                    let p = caps.as_ref().ok().and_then(|c| provider(c, &m.instance_id));
                    let state = match (p, limit) {
                        (Some(p), _) if error.is_none() => Some(member_state(pool, p, limit)),
                        (_, Some(l)) => Some(State::Limited(l.until.clone())),
                        _ => None,
                    };
                    let (picks, last) = stats.get(&m.instance_id).cloned().unwrap_or((0, None));
                    json!({
                        "instanceId": m.instance_id,
                        "weight": m.weight,
                        "reserve": m.reserve,
                        "state": state.as_ref().map_or("unknown", State::name),
                        "reason": state.as_ref().and_then(|s| s.why(pool)),
                        "limitedUntil": limit.map(|l| l.until.clone()),
                        "limitedUntilLocal": limit.map(|l| local_clock(&l.until)),
                        "running": running.as_ref().ok().map(|r| r.get(&m.instance_id).copied().unwrap_or(0)),
                        "picks": picks,
                        "lastPick": last,
                        "lastPickLocal": last.as_deref().map(local_clock),
                    })
                })
                .collect();
            out.push(json!({
                "pool": name,
                "driver": pool.driver,
                "model": pool.model,
                "effort": pool.effort,
                "strategy": pool.strategy,
                "error": error,
                "members": members,
            }));
        }
        Ok(Value::Array(out))
    }
}
