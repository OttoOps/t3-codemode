use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clock::{iso, local_clock, parse_ts, plus_minutes};
use crate::store::{Limit, LimitStop, PickRecord};
use crate::usage::{Usage, UsageSource};
use crate::wf::{ACTIVE, Wf};

pub const STRATEGIES: &[&str] = &["weighted", "priority"];
const EFFORT_IDS: &[&str] = &["effort", "reasoningEffort", "thinking_level"];
const THREAD_PAGE: u64 = 200;
const LIMIT_TITLE: &str = "Usage limit reached";
const LOOKBACK_MINUTES: f64 = 24.0 * 60.0;
const STOP_ITEMS: u64 = 20;
const LAST_POSITION: u64 = 1 << 40;

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
    NoUsage,
    Disabled,
    Constrained(String),
    MissingModel,
    MissingEffort,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Ready => "ready",
            State::Limited(_) | State::NoUsage => "limited",
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
            State::NoUsage => Some("has no usage left".into()),
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

fn member_state(pool: &PoolConfig, p: &Value, limit: Option<&Limit>, usage: Option<&Usage>) -> State {
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
    if let Some(l) = limit {
        return State::Limited(l.until.clone());
    }
    match usage {
        Some(u) if u.left <= 0.0 => u.resets_at.clone().map_or(State::NoUsage, State::Limited),
        _ => State::Ready,
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

fn pct(v: f64) -> String {
    format!("{}%", (v * 10.0).round() / 10.0)
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
    pub usage: &'a BTreeMap<String, Usage>,
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

fn by_weight(
    pool: &PoolConfig,
    among: &[usize],
    credits: &mut BTreeMap<String, i64>,
    running: &BTreeMap<String, u64>,
) -> (usize, i64) {
    let total: i64 = among.iter().map(|&i| pool.members[i].weight as i64).sum();
    for &i in among {
        let m = &pool.members[i];
        *credits.entry(m.instance_id.clone()).or_default() += m.weight as i64;
    }
    let load = |i: usize| running.get(&pool.members[i].instance_id).copied().unwrap_or(0);
    let credit = |i: usize| credits[&pool.members[i].instance_id];
    let best =
        among
            .iter()
            .copied()
            .reduce(|a, b| {
                if (credit(b), std::cmp::Reverse(load(b))) > (credit(a), std::cmp::Reverse(load(a))) { b } else { a }
            })
            .expect("among is not empty");
    *credits.get_mut(&pool.members[best].instance_id).expect("credited") -= total;
    (best, total)
}

pub fn choose(inp: &Inputs) -> Result<Choice> {
    let Inputs { name, pool, caps, running, limits, usage, credits, now } = *inp;
    check_members(name, pool, caps)?;
    let mut skipped = Vec::new();
    let mut open = Vec::new();
    let mut held = Vec::new();
    for (i, m) in pool.members.iter().enumerate() {
        let p = provider(caps, &m.instance_id).expect("checked members");
        let left = usage.get(&m.instance_id);
        let state = member_state(pool, p, active_limit(limits, &m.instance_id, now), left);
        if let Some(why) = state.why(pool) {
            skipped.push(Skipped { instance_id: m.instance_id.clone(), reason: why });
            continue;
        }
        match left {
            _ if m.reserve <= 0.0 => open.push(i),
            Some(u) if u.left > m.reserve => open.push(i),
            Some(u) => skipped.push(Skipped {
                instance_id: m.instance_id.clone(),
                reason: format!("held at its {}% reserve with {} left", m.reserve, pct(u.left)),
            }),
            None => held.push(i),
        }
    }
    let last_resort = open.is_empty() && !held.is_empty();
    let candidates = if open.is_empty() { held.clone() } else { open };
    if candidates.is_empty() {
        let list: Vec<String> = skipped.iter().map(|s| format!("{} {}", s.instance_id, s.reason)).collect();
        bail!("pool {name} has no member that can run: {}", list.join("; "));
    }
    if !last_resort {
        for &i in &held {
            let m = &pool.members[i];
            skipped.push(Skipped {
                instance_id: m.instance_id.clone(),
                reason: format!("held at its {}% reserve until every other member is out", m.reserve),
            });
        }
    }
    let mut credits = credits.clone();
    let (chosen, base) = match pool.strategy.as_str() {
        "priority" => (candidates[0], "priority: the first member that can run".to_string()),
        "headroom" => {
            let left = |i: usize| usage.get(&pool.members[i].instance_id).map(|u| u.left);
            match candidates.iter().filter_map(|&i| left(i)).reduce(f64::max) {
                Some(most) => {
                    let top: Vec<usize> = candidates.iter().copied().filter(|&i| left(i) == Some(most)).collect();
                    let (best, _) = by_weight(pool, &top, &mut credits, running);
                    (best, format!("headroom: most left, {}", pct(most)))
                }
                None => {
                    let (best, total) = by_weight(pool, &candidates, &mut credits, running);
                    let w = pool.members[best].weight;
                    (best, format!("headroom: no member reports usage, so weighted: weight {w} of {total}"))
                }
            }
        }
        _ => {
            let (best, total) = by_weight(pool, &candidates, &mut credits, running);
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

fn snooze_until(t: &Value) -> Option<String> {
    if t["snoozed"] != true {
        return None;
    }
    parse_ts(t["snoozedUntil"].as_str()?).map(iso)
}

fn later(a: &str, b: &str) -> bool {
    matches!((parse_ts(a), parse_ts(b)), (Some(a), Some(b)) if a > b)
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

    pub fn set_usage_source(&self, source: Option<Arc<dyn UsageSource>>) {
        *self.usage.lock().unwrap_or_else(|e| e.into_inner()) = source;
    }

    async fn usage_of(&self, instances: &[String]) -> BTreeMap<String, Usage> {
        let source = self.usage.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(source) = source else { return BTreeMap::new() };
        let now = self.clock.now();
        match source.usage(instances).await {
            Ok(u) => u
                .into_iter()
                .filter(|(_, u)| u.resets_at.as_deref().and_then(parse_ts).is_none_or(|r| r > now))
                .collect(),
            Err(e) => {
                tracing::warn!("usage source did not answer, picking without it: {e:#}");
                BTreeMap::new()
            }
        }
    }

    async fn capabilities(&self, model: Option<&str>) -> Result<Value> {
        let input = match model {
            Some(m) => json!({"search": m}),
            None => json!({}),
        };
        Ok(self.t3.call("orchestrator_capabilities", input).await?)
    }

    async fn threads_with(&self, statuses: &[&str]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor: Option<u64> = None;
        loop {
            let mut input = json!({"allProjects": true, "statuses": statuses, "limit": THREAD_PAGE});
            if let Some(c) = cursor {
                input["cursor"] = json!(c);
            }
            let r = self.t3.call("t3_thread_list", input).await?;
            let threads = r["threads"].as_array().cloned().unwrap_or_default();
            let empty = threads.is_empty();
            out.extend(threads);
            match r["nextCursor"].as_u64() {
                Some(c) if !empty && cursor.is_none_or(|p| c > p) => cursor = Some(c),
                _ => break,
            }
        }
        Ok(out)
    }

    async fn running_by_instance(&self) -> Result<BTreeMap<String, u64>> {
        let mut out = BTreeMap::new();
        for t in self.threads_with(ACTIVE).await? {
            if let Some(id) = t["providerInstanceId"].as_str() {
                *out.entry(id.to_string()).or_default() += 1;
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
        let ids: Vec<String> = pool.members.iter().map(|m| m.instance_id.clone()).collect();
        let usage = self.usage_of(&ids).await;
        let _g = self.pool_lock.lock().await;
        let limits = self.store.limits()?;
        let credits = self.store.pool_credits(name)?;
        let now = self.clock.now();
        let c = choose(&Inputs {
            name,
            pool,
            caps: &caps,
            running: &running,
            limits: &limits,
            usage: &usage,
            credits: &credits,
            now,
        })?;
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

    fn members(&self) -> BTreeSet<String> {
        self.config.pools.values().flat_map(|p| p.members.iter().map(|m| m.instance_id.clone())).collect()
    }

    async fn stopped_on_limit(&self, t: &Value, thread: &str, run: &str) -> Result<bool> {
        let end = t["itemCount"].as_u64().unwrap_or(LAST_POSITION);
        let r = self
            .t3
            .call(
                "t3_thread_read",
                json!({"threadId": thread, "view": "activity", "beforePosition": end,
                       "limit": STOP_ITEMS, "maxCharsPerItem": 300}),
            )
            .await?;
        let items = r["items"].as_array().map(Vec::as_slice).unwrap_or_default();
        Ok(items.iter().any(|i| i["type"] == "error" && i["title"] == LIMIT_TITLE && i["runId"] == run))
    }

    async fn limit_from_thread(&self, t: &Value, members: &BTreeSet<String>) -> Result<Option<(String, Limit)>> {
        let Some(instance) = t["providerInstanceId"].as_str().filter(|i| members.contains(*i)) else {
            return Ok(None);
        };
        let (Some(thread), Some(run)) = (t["threadId"].as_str(), t["latestRunId"].as_str()) else {
            return Ok(None);
        };
        if self.mins(t["updatedAt"].as_str()).is_none_or(|m| m > LOOKBACK_MINUTES) {
            return Ok(None);
        }
        let now = self.clock.now();
        let stop = match self.store.limit_stop(thread, run)? {
            Some(s) => s,
            None => {
                let limit = self.stopped_on_limit(t, thread, run).await?;
                let hold = self.config.limit_hold_minutes;
                let (until, how) = match snooze_until(t) {
                    _ if !limit => (None, None),
                    Some(u) => (Some(u), Some("reset time from T3's limit snooze".to_string())),
                    None => (
                        Some(iso(plus_minutes(now, hold as f64))),
                        Some(format!("reset time unknown, held {hold} min")),
                    ),
                };
                let s = LimitStop { limit, seen_at: iso(now), until, how };
                self.store.limit_stop_put(thread, run, &s)?;
                s
            }
        };
        let (Some(until), Some(how)) = (stop.until, stop.how) else {
            return Ok(None);
        };
        if parse_ts(&until).is_none_or(|u| u <= now) {
            return Ok(None);
        }
        let title = t["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(thread);
        let limit = Limit {
            until,
            reason: Some(format!("usage limit on thread {title}; {how}")),
            at: iso(now),
            thread: Some(thread.to_string()),
        };
        Ok(Some((instance.to_string(), limit)))
    }

    pub async fn mark_limits(&self) -> Result<Vec<String>> {
        if self.config.pools.is_empty() {
            return Ok(Vec::new());
        }
        let members = self.members();
        let mut want: BTreeMap<String, Limit> = BTreeMap::new();
        let mut keep = |id: String, l: Limit| match want.get(&id) {
            Some(w) if !later(&l.until, &w.until) => {}
            _ => {
                want.insert(id, l);
            }
        };
        for t in self.threads_with(&["failed"]).await? {
            match self.limit_from_thread(&t, &members).await {
                Ok(Some((id, l))) => keep(id, l),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    "limit check skipped thread {}: {}",
                    t["threadId"].as_str().unwrap_or("?"),
                    self.t3.redact(format!("{e:#}"))
                ),
            }
        }
        let ids: Vec<String> = members.iter().cloned().collect();
        let now = self.clock.now();
        for (id, u) in self.usage_of(&ids).await {
            if let Some(until) = u.resets_at.filter(|r| u.left <= 0.0 && parse_ts(r).is_some_and(|r| r > now)) {
                let reason = Some("the usage source reports no usage left".to_string());
                keep(id, Limit { until, reason, at: iso(now), thread: None });
            }
        }
        let mut actions = Vec::new();
        let _g = self.pool_lock.lock().await;
        let stored = self.store.limits()?;
        for (id, l) in &stored {
            if active_limit(&stored, id, now).is_none() {
                self.store.limit_clear(id)?;
                actions.push(format!("limit cleared {id}, reset {} passed", local_clock(&l.until)));
            }
        }
        for (id, l) in want {
            let replace = active_limit(&stored, &id, now).is_none_or(|s| later(&l.until, &s.until));
            if replace {
                actions.push(format!("limited {id} until {}", local_clock(&l.until)));
                self.store.limit_set(&id, &l)?;
            }
        }
        if !actions.is_empty() {
            self.changed();
        }
        Ok(actions)
    }

    pub async fn pools(&self) -> Result<Value> {
        let running = self.running_by_instance().await;
        let limits = self.store.limits()?;
        let usage = self.usage_of(&self.members().into_iter().collect::<Vec<_>>()).await;
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
                    let left = usage.get(&m.instance_id);
                    let p = caps.as_ref().ok().and_then(|c| provider(c, &m.instance_id));
                    let state = match (p, limit) {
                        (Some(p), _) if error.is_none() => Some(member_state(pool, p, limit, left)),
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
                        "limitReason": limit.and_then(|l| l.reason.clone()),
                        "usageLeft": left.map(|u| u.left),
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
