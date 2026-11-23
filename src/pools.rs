use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clock::{iso, local_clock, parse_ts, plus_minutes};
use crate::config::Config;
use crate::field::{self, invalid, within};
use crate::store::{Limit, LimitStop, PickRecord};
use crate::usage::{Usage, UsageSource};
use crate::wf::{ACTIVE, Wf};

pub const STRATEGIES: &[&str] = &["weighted", "priority", "cheapest"];
const EFFORT_IDS: &[&str] = &["effort", "reasoningEffort", "thinking_level", "thought_level"];
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

fn unit_cost() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolConfig {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mixed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default = "weighted")]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "MoveOnLimit::is_ask")]
    pub move_on_limit: MoveOnLimit,
    pub members: Vec<PoolMember>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MoveOnLimit {
    #[default]
    Ask,
    Auto,
    Never,
}

impl MoveOnLimit {
    fn is_ask(&self) -> bool {
        *self == Self::Ask
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolMember {
    pub instance_id: String,
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default)]
    pub reserve: f64,
    #[serde(default = "unit_cost")]
    pub cost: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

fn blank(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| s.trim().is_empty())
}

impl PoolConfig {
    pub fn validate(&self) -> Result<()> {
        let fail = |key: &str, message: String| Err(invalid(key, message));
        if self.mixed {
            for (key, set) in [("driver", &self.driver), ("model", &self.model), ("effort", &self.effort)] {
                if set.is_some() {
                    return fail(
                        key,
                        format!("a mixed pool has no pool-level {key}; set model and effort on each member"),
                    );
                }
            }
        } else {
            match self.driver.as_deref() {
                None => {
                    return fail(
                        "driver",
                        "driver is missing; set it, or set \"mixed\": true for members of several drivers".into(),
                    );
                }
                Some(d) if d.trim().is_empty() => return fail("driver", "driver is empty".into()),
                Some(_) => {}
            }
            match self.model.as_deref() {
                None => return fail("model", "model is missing".into()),
                Some(m) if m.trim().is_empty() => return fail("model", "model is empty".into()),
                Some(_) => {}
            }
            if blank(&self.effort) {
                return fail("effort", "effort is empty".into());
            }
        }
        match self.strategy.as_str() {
            s if STRATEGIES.contains(&s) => {}
            "headroom" => {
                return fail(
                    "strategy",
                    format!(
                        "strategy headroom needs a usage source, which t3cm does not have yet; use one of {}",
                        STRATEGIES.join(", ")
                    ),
                );
            }
            s => return fail("strategy", format!("unknown strategy {s:?}; use one of {}", STRATEGIES.join(", "))),
        }
        if self.members.is_empty() {
            return fail("members", "a pool needs at least one member".into());
        }
        for (i, m) in self.members.iter().enumerate() {
            let fail = |key: &str, message: String| Err(invalid(format!("members.{i}.{key}"), message));
            let id = &m.instance_id;
            if id.trim().is_empty() {
                return fail("instanceId", format!("member {} has an empty instanceId", i + 1));
            }
            if self.members[..i].iter().any(|o| o.instance_id == *id) {
                return fail("instanceId", format!("member {id} is listed twice"));
            }
            if m.weight == 0 {
                return fail("weight", format!("member {id}: weight must be at least 1"));
            }
            if !(0.0..=100.0).contains(&m.reserve) {
                return fail("reserve", format!("member {id}: reserve must be between 0 and 100, got {}", m.reserve));
            }
            if !m.cost.is_finite() || m.cost < 0.0 {
                return fail("cost", format!("member {id}: cost must be a finite number of 0 or more, got {}", m.cost));
            }
            if !self.mixed {
                for (key, set) in [("model", &m.model), ("effort", &m.effort)] {
                    if set.is_some() {
                        return fail(
                            key,
                            format!(
                                "member {id}: only a mixed pool's members set their own {key}; set it on the pool, or set \"mixed\": true"
                            ),
                        );
                    }
                }
            } else {
                match m.model.as_deref() {
                    None => return fail("model", format!("member {id}: a mixed pool's member needs a model")),
                    Some(s) if s.trim().is_empty() => return fail("model", format!("member {id}: model is empty")),
                    Some(_) => {}
                }
                if blank(&m.effort) {
                    return fail("effort", format!("member {id}: effort is empty"));
                }
            }
        }
        Ok(())
    }

    pub fn model_of<'a>(&'a self, m: &'a PoolMember) -> &'a str {
        m.model.as_deref().or(self.model.as_deref()).unwrap_or_default()
    }

    pub fn effort_of<'a>(&'a self, m: &'a PoolMember) -> Option<&'a str> {
        m.effort.as_deref().or(self.effort.as_deref())
    }
}

pub fn check_value(name: &str, raw: &Value) -> Result<PoolConfig> {
    let at = field::display(&["pools", name]);
    let label = format!("pool {name}");
    let pool: PoolConfig =
        field::deserialize(raw.clone()).map_err(|(key, e)| invalid(field::join(&at, &key), format!("{label}: {e}")))?;
    pool.validate().map_err(|e| within(e, &at, &label))?;
    Ok(pool)
}

fn providers(caps: &Value) -> &[Value] {
    caps["providers"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn provider<'a>(caps: &'a Value, id: &str) -> Option<&'a Value> {
    providers(caps).iter().find(|p| p["providerInstanceId"] == id)
}

pub fn check_members(name: &str, pool: &PoolConfig, caps: &Value) -> Result<()> {
    for (i, m) in pool.members.iter().enumerate() {
        let index = i.to_string();
        let at = field::display(&["pools", name, "members", &index, "instanceId"]);
        let Some(p) = provider(caps, &m.instance_id) else {
            let known: Vec<&str> = providers(caps).iter().filter_map(|p| p["providerInstanceId"].as_str()).collect();
            return Err(invalid(
                at,
                format!(
                    "pool {name}: member {} is not a T3 provider instance; the instances are {}",
                    m.instance_id,
                    if known.is_empty() { "none".into() } else { known.join(", ") }
                ),
            ));
        };
        let driver = p["driverKind"].as_str().unwrap_or_default();
        if let Some(want) = pool.driver.as_deref().filter(|w| *w != driver) {
            return Err(invalid(
                at,
                format!(
                    "pool {name}: member {} is a {driver} instance, not {want}; set \"mixed\": true to mix drivers",
                    m.instance_id
                ),
            ));
        }
    }
    Ok(())
}

fn lists_model(p: &Value, model: &str) -> bool {
    p["models"].as_array().is_some_and(|ms| ms.iter().any(|m| m["id"] == model))
}

fn can_run(pool: &PoolConfig, m: &PoolMember, p: &Value) -> bool {
    let model = pool.model_of(m);
    lists_model(p, model) && effort_options(model, pool.effort_of(m), p).is_some()
}

fn reserve_hint(m: &PoolMember) -> String {
    if m.reserve > 0.0 {
        String::new()
    } else {
        format!(" Set a reserve on {} to keep room for the work only it can run.", m.instance_id)
    }
}

pub fn sole_warnings(pool: &PoolConfig, caps: &Value) -> Vec<String> {
    let mut groups: BTreeMap<&str, Vec<&PoolMember>> = BTreeMap::new();
    for m in &pool.members {
        let Some(p) = provider(caps, &m.instance_id).filter(|p| can_run(pool, m, p)) else { continue };
        let key = if pool.mixed { p["driverKind"].as_str().unwrap_or_default() } else { pool.model_of(m) };
        groups.entry(key).or_default().push(m);
    }
    groups
        .into_iter()
        .filter_map(|(key, ms)| match ms.as_slice() {
            [m] if pool.mixed => Some(format!(
                "{id} is the only {key} member that can run its model, {model}, so the pool runs no {key} launch while {id} is out.{hint}",
                id = m.instance_id,
                model = pool.model_of(m),
                hint = reserve_hint(m)
            )),
            [m] => Some(format!(
                "only {id} can run {key}, so the pool launches nothing while {id} is out.{hint}",
                id = m.instance_id,
                hint = reserve_hint(m)
            )),
            _ => None,
        })
        .collect()
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

    fn why(&self, pool: &PoolConfig, m: &PoolMember) -> Option<String> {
        match self {
            State::Ready => None,
            State::Limited(until) => Some(format!("limited until {}", local_clock(until))),
            State::NoUsage => Some("has no usage left".into()),
            State::Disabled => Some("disabled".into()),
            State::Constrained(c) => Some(format!("constrained: {c}")),
            State::MissingModel => Some(format!("does not list model {}", pool.model_of(m))),
            State::MissingEffort => Some(format!(
                "does not offer effort {} for {}",
                pool.effort_of(m).unwrap_or_default(),
                pool.model_of(m)
            )),
        }
    }
}

fn current(usage: BTreeMap<String, Usage>, now: Timestamp) -> BTreeMap<String, Usage> {
    usage.into_iter().filter(|(_, u)| u.resets_at.as_deref().and_then(parse_ts).is_none_or(|r| r > now)).collect()
}

fn active_limit<'a>(limits: &'a BTreeMap<String, Limit>, id: &str, now: Timestamp) -> Option<&'a Limit> {
    limits.get(id).filter(|l| parse_ts(&l.until).is_some_and(|u| u > now))
}

fn member_state(pool: &PoolConfig, m: &PoolMember, p: &Value, limit: Option<&Limit>, usage: Option<&Usage>) -> State {
    let constraints: Vec<&str> = p["constraints"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if constraints.iter().any(|c| c.to_ascii_lowercase().contains("disabled")) {
        return State::Disabled;
    }
    if !constraints.is_empty() {
        return State::Constrained(constraints.join("; "));
    }
    let model = pool.model_of(m);
    if !lists_model(p, model) {
        return State::MissingModel;
    }
    if effort_options(model, pool.effort_of(m), p).is_none() {
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

fn effort_options(model: &str, effort: Option<&str>, p: &Value) -> Option<Value> {
    let Some(effort) = effort else { return Some(json!([])) };
    let options = p["models"]
        .as_array()
        .and_then(|ms| ms.iter().find(|m| m["id"] == model))
        .and_then(|m| m["options"].as_array())?;
    let option = EFFORT_IDS.iter().find_map(|id| options.iter().find(|o| o["id"] == *id))?;
    let offered = option["values"].as_array().is_some_and(|vs| vs.iter().any(|v| v == effort));
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
    pub driver: String,
    pub model: String,
    pub effort: Option<String>,
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
        let state = member_state(pool, m, p, active_limit(limits, &m.instance_id, now), left);
        if let Some(why) = state.why(pool, m) {
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
        "cheapest" => {
            let cost = |i: usize| pool.members[i].cost;
            let low = candidates.iter().map(|&i| cost(i)).reduce(f64::min).expect("candidates is not empty");
            let group: Vec<usize> = candidates.iter().copied().filter(|&i| cost(i) == low).collect();
            let (best, total) = by_weight(pool, &group, &mut credits, running);
            (best, format!("cheapest: cost {low}, weighted: weight {} of {total}", pool.members[best].weight))
        }
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
    let driver = p["driverKind"].as_str().unwrap_or_default().to_string();
    let (model, effort) = (pool.model_of(m), pool.effort_of(m));
    let ran = format!("{driver} {model}, effort {}", effort.unwrap_or("default"));
    Ok(Choice {
        instance_id: m.instance_id.clone(),
        driver,
        model: model.to_string(),
        effort: effort.map(String::from),
        options: effort_options(model, effort, p).expect("a ready member offers the effort"),
        reason: with_skips(format!("{ran}; {base}"), &skipped),
        skipped,
        credits,
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    pub pool: String,
    pub instance_id: String,
    pub driver: String,
    pub model: String,
    pub effort: Option<String>,
    pub options: Value,
    pub reason: String,
    pub skipped: Vec<Skipped>,
    pub model_selection: Value,
    #[serde(rename = "pick")]
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
    pub(crate) fn pool(&self, name: &str) -> Result<PoolConfig> {
        let config = self.config();
        config.pools.get(name).cloned().ok_or_else(|| {
            let known: Vec<&str> = config.pools.keys().map(String::as_str).collect();
            anyhow!(
                "no pool {name:?}; configured pools: {}",
                if known.is_empty() { "none".into() } else { known.join(", ") }
            )
        })
    }

    pub fn set_usage_source(&self, source: Option<Arc<dyn UsageSource>>) {
        *self.usage.lock().unwrap_or_else(|e| e.into_inner()) = source;
    }

    pub(crate) async fn usage_of(&self, instances: &[String]) -> BTreeMap<String, Usage> {
        let source = self.usage.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(source) = source else { return BTreeMap::new() };
        match source.usage(instances).await {
            Ok(u) => u,
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

    async fn pool_capabilities(&self, pool: &PoolConfig) -> Result<Value> {
        let models: BTreeSet<&str> = pool.members.iter().map(|m| pool.model_of(m)).collect();
        let mut merged: Option<Value> = None;
        for model in models {
            let caps = self.capabilities(Some(model)).await?;
            let Some(all) = merged.as_mut() else {
                merged = Some(caps);
                continue;
            };
            for p in providers(&caps) {
                let Some(id) = p["providerInstanceId"].as_str() else { continue };
                let Some(have) = all["providers"]
                    .as_array_mut()
                    .and_then(|ps| ps.iter_mut().find(|q| q["providerInstanceId"] == id))
                else {
                    continue;
                };
                if let Some(ms) = have["models"].as_array_mut() {
                    ms.extend(p["models"].as_array().into_iter().flatten().cloned());
                }
            }
        }
        merged.ok_or_else(|| anyhow!("pool has no members"))
    }

    pub(crate) async fn threads_with(&self, statuses: &[&str]) -> Result<Vec<Value>> {
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

    pub(crate) async fn running_by_instance(&self) -> Result<BTreeMap<String, u64>> {
        let mut out = BTreeMap::new();
        for t in self.threads_with(ACTIVE).await? {
            if let Some(id) = t["providerInstanceId"].as_str() {
                *out.entry(id.to_string()).or_default() += 1;
            }
        }
        Ok(out)
    }

    pub async fn check_pools(&self) -> Result<()> {
        let config = self.config();
        if config.pools.is_empty() {
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
        for (name, pool) in &config.pools {
            check_members(name, pool, &caps)?;
        }
        Ok(())
    }

    pub async fn pick(&self, name: &str) -> Result<Picked> {
        self.pick_avoiding(name, None).await
    }

    pub(crate) async fn pick_avoiding(&self, name: &str, avoid: Option<(&str, &str)>) -> Result<Picked> {
        let mut pool = self.pool(name)?;
        let caps = self.pool_capabilities(&pool).await?;
        check_members(name, &pool, &caps)?;
        let mut avoided = Vec::new();
        if let Some((id, why)) = avoid {
            let driver_of = |i: &str| provider(&caps, i).and_then(|p| p["driverKind"].as_str()).map(String::from);
            let driver = driver_of(id);
            pool.members.retain(|m| {
                let reason = if m.instance_id == id {
                    Some(why.to_string())
                } else {
                    let d = driver_of(&m.instance_id);
                    driver.as_ref().filter(|want| d.as_ref() != Some(want)).map(|want| {
                        format!("is a {} instance, not {want} like the thread", d.as_deref().unwrap_or("unknown"))
                    })
                };
                let keep = reason.is_none();
                avoided.extend(reason.map(|reason| Skipped { instance_id: m.instance_id.clone(), reason }));
                keep
            });
            if pool.members.is_empty() {
                let like = driver.map(|d| format!(" that is a {d} instance")).unwrap_or_default();
                bail!("pool {name} has no member other than {id}{like}");
            }
        }
        let pool = &pool;
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
        let usage = current(usage, now);
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
        let mut c = c;
        for s in avoided {
            let join = if c.skipped.is_empty() { "; skipped" } else { "," };
            c.reason = format!("{}{join} {} {}", c.reason, s.instance_id, s.reason);
            c.skipped.push(s);
        }
        let at = self.now();
        self.store.pool_credits_put(name, &c.credits)?;
        let seq = self.store.pick_add(&PickRecord {
            pool: name.into(),
            instance_id: c.instance_id.clone(),
            model: c.model.clone(),
            strategy: pool.strategy.clone(),
            reason: c.reason.clone(),
            skipped: serde_json::to_value(&c.skipped)?,
            at,
            handle: None,
            thread: None,
            moved_from: None,
        })?;
        Ok(Picked {
            pool: name.into(),
            model_selection: model_selection(&c.instance_id, &c.model, &c.options),
            instance_id: c.instance_id,
            driver: c.driver,
            model: c.model,
            effort: c.effort,
            options: c.options,
            reason: c.reason,
            skipped: c.skipped,
            seq,
        })
    }

    pub fn picked(&self, seq: i64, thread: &str) -> Result<Value> {
        let thread = thread.trim();
        if thread.is_empty() {
            bail!("wf.picked: empty thread");
        }
        let p =
            self.store.pick_get(seq)?.ok_or_else(|| anyhow!("wf.picked: no pick {seq}; pass the pick from wf.pick"))?;
        match p.thread.as_deref() {
            Some(t) if t != thread => {
                bail!("wf.picked: pick {seq} already launched thread {t}; pick again for another launch")
            }
            Some(_) => {}
            None => self.store.pick_bind(seq, p.handle.as_deref(), thread, None)?,
        }
        Ok(json!({"pick": seq, "pool": p.pool, "instanceId": p.instance_id, "thread": thread}))
    }

    pub(crate) fn members(config: &Config) -> BTreeSet<String> {
        config.pools.values().flat_map(|p| p.members.iter().map(|m| m.instance_id.clone())).collect()
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
                let hold = self.config().limit_hold_minutes;
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
        let config = self.config();
        if config.pools.is_empty() {
            return Ok(Vec::new());
        }
        let members = Self::members(&config);
        let mut want: BTreeMap<String, Limit> = BTreeMap::new();
        let mut keep = |id: String, l: Limit| match want.get(&id) {
            Some(w) if !later(&l.until, &w.until) => {}
            _ => {
                want.insert(id, l);
            }
        };
        let failed = self.threads_with(&["failed"]).await?;
        for t in &failed {
            match self.limit_from_thread(t, &members).await {
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
        let usage = self.usage_of(&ids).await;
        let now = self.clock.now();
        for (id, u) in current(usage, now) {
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
        drop(_g);
        actions.extend(self.offer_moves(&failed).await);
        if !actions.is_empty() {
            self.changed();
        }
        Ok(actions)
    }

    pub async fn pools(&self) -> Result<Value> {
        let running = self.running_by_instance().await;
        let limits = self.store.limits()?;
        let config = self.config();
        let reported = self.usage_of(&Self::members(&config).into_iter().collect::<Vec<_>>()).await;
        let mut out = Vec::new();
        for (name, pool) in &config.pools {
            out.push(self.pool_entry(name, pool, &running, &limits, &reported).await?);
        }
        Ok(Value::Array(out))
    }

    pub(crate) async fn pool_entry(
        &self,
        name: &str,
        pool: &PoolConfig,
        running: &Result<BTreeMap<String, u64>>,
        limits: &BTreeMap<String, Limit>,
        reported: &BTreeMap<String, Usage>,
    ) -> Result<Value> {
        let caps = match running {
            Ok(_) => self.pool_capabilities(pool).await,
            Err(e) => Err(anyhow!("{e:#}")),
        };
        let now = self.clock.now();
        let usage = current(reported.clone(), now);
        let error = match &caps {
            Ok(c) => check_members(name, pool, c).err().map(|e| format!("{e:#}")),
            Err(e) => Some(format!("T3 did not answer: {e:#}")),
        };
        let warnings = match (&caps, &error) {
            (Ok(c), None) => sole_warnings(pool, c),
            _ => Vec::new(),
        };
        let stats = self.store.pick_stats(name)?;
        let members: Vec<Value> = pool
            .members
            .iter()
            .map(|m| {
                let limit = active_limit(limits, &m.instance_id, now);
                let left = usage.get(&m.instance_id);
                let p = caps.as_ref().ok().and_then(|c| provider(c, &m.instance_id));
                let state = match (p, limit) {
                    (Some(p), _) if error.is_none() => Some(member_state(pool, m, p, limit, left)),
                    (_, Some(l)) => Some(State::Limited(l.until.clone())),
                    _ => None,
                };
                let (picks, last) = stats.get(&m.instance_id).cloned().unwrap_or((0, None));
                let driver = p.and_then(|p| p["driverKind"].as_str()).or(pool.driver.as_deref());
                json!({
                    "instanceId": m.instance_id,
                    "driver": driver,
                    "model": pool.model_of(m),
                    "effort": pool.effort_of(m),
                    "weight": m.weight,
                    "reserve": m.reserve,
                    "cost": m.cost,
                    "state": state.as_ref().map_or("unknown", State::name),
                    "reason": state.as_ref().and_then(|s| s.why(pool, m)),
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
        Ok(json!({
            "pool": name,
            "mixed": pool.mixed,
            "driver": pool.driver,
            "model": pool.model,
            "effort": pool.effort,
            "strategy": pool.strategy,
            "moveOnLimit": pool.move_on_limit,
            "error": error,
            "warnings": warnings,
            "members": members,
        }))
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use arbitrary::Unstructured;
    use arbtest::arbtest;

    const NOW_STR: &str = "2026-11-17T12:00:00Z";
    const PAST_STR: &str = "2026-11-17T11:00:00Z";
    const FUTURE_STR: &str = "2026-11-17T18:00:00Z";
    const DRIVERS: &[&str] = &["claudeAgent", "codex", "gemini"];
    const MODELS: &[&str] = &["model-a", "model-b"];
    const EFFORTS: &[Option<&str>] = &[None, Some("low"), Some("medium"), Some("high")];
    const COSTS: &[f64] = &[0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 5.0];
    const RESERVES: &[f64] = &[0.0, 0.0, 0.0, 10.0, 25.0, 50.0, 80.0, 100.0];
    const ALL_STRATEGIES: &[&str] = &["weighted", "priority", "cheapest", "headroom"];

    fn now() -> Timestamp {
        NOW_STR.parse().expect("valid timestamp")
    }

    fn prop<P>(budget_ms: u64, property: P)
    where
        P: FnMut(&mut Unstructured<'_>) -> arbitrary::Result<()>,
    {
        let mut test = arbtest(property).size_min(32).size_max(4096);
        if std::env::var_os("ARBTEST_BUDGET_MS").is_none() {
            test = test.budget_ms(budget_ms);
        }
        test.run();
    }

    #[derive(Clone, Debug)]
    struct Scenario {
        pool: PoolConfig,
        caps: Value,
        running: BTreeMap<String, u64>,
        limits: BTreeMap<String, Limit>,
        usage: BTreeMap<String, Usage>,
        credits: BTreeMap<String, i64>,
    }

    impl Scenario {
        fn inputs(&self) -> Inputs<'_> {
            Inputs {
                name: "test-pool",
                pool: &self.pool,
                caps: &self.caps,
                running: &self.running,
                limits: &self.limits,
                usage: &self.usage,
                credits: &self.credits,
                now: now(),
            }
        }
    }

    fn provider_json(
        id: &str,
        driver: &str,
        constraints: &[&str],
        listed_model: Option<&str>,
        effort_opt_id: &str,
        offered_efforts: &[&str],
    ) -> Value {
        let models = match listed_model {
            Some(m) => json!([{
                "id": m,
                "options": [{"id": effort_opt_id, "values": offered_efforts}]
            }]),
            None => json!([]),
        };
        json!({
            "providerInstanceId": id,
            "driverKind": driver,
            "displayName": id,
            "models": models,
            "constraints": constraints,
        })
    }

    fn gen_scenario(u: &mut Unstructured<'_>, all_ready: bool) -> arbitrary::Result<Scenario> {
        let mixed = u.arbitrary::<bool>()?;
        let strategy = (*u.choose(ALL_STRATEGIES)?).to_string();
        let pool_driver = if mixed { None } else { Some((*u.choose(DRIVERS)?).to_string()) };
        let pool_model = if mixed { None } else { Some((*u.choose(MODELS)?).to_string()) };
        let pool_effort = if mixed { None } else { (*u.choose(EFFORTS)?).map(String::from) };
        let n = u.int_in_range(1..=8usize)?;

        let mut members = Vec::with_capacity(n);
        let mut providers_vec = Vec::with_capacity(n);
        let mut running = BTreeMap::new();
        let mut limits = BTreeMap::new();
        let mut usage = BTreeMap::new();
        let mut credits = BTreeMap::new();

        let seed_credits = u.arbitrary::<bool>()?;
        if seed_credits && u.arbitrary::<bool>()? {
            credits.insert("stale-member".to_string(), u.int_in_range(-100..=100)?);
        }

        for i in 0..n {
            let id = format!("m{i}");
            let driver = if mixed { *u.choose(DRIVERS)? } else { pool_driver.as_deref().unwrap() };
            let m_model = if mixed { Some((*u.choose(MODELS)?).to_string()) } else { None };
            let m_effort = if mixed { (*u.choose(EFFORTS)?).map(String::from) } else { None };
            let target_model = m_model.as_deref().or(pool_model.as_deref()).unwrap();
            let target_effort = m_effort.as_deref().or(pool_effort.as_deref());
            let weight = u.int_in_range(1..=16u32)?;
            let reserve = if all_ready { 0.0 } else { *u.choose(RESERVES)? };
            let cost = *u.choose(COSTS)?;

            let (constraints, listed_model, offered_efforts): (&[&str], Option<&str>, &[&str]) = if all_ready {
                (&[], Some(target_model), &["low", "medium", "high"])
            } else {
                let c: &[&str] = match u.int_in_range(0..=9u8)? {
                    0 => &["Provider instance is DISABLED."],
                    1 => &["Account paused"],
                    _ => &[],
                };
                let lm = if u.int_in_range(0..=7u8)? == 0 { None } else { Some(target_model) };
                let oe: &[&str] = match target_effort {
                    Some(e) if u.int_in_range(0..=7u8)? == 0 => {
                        if e == "high" {
                            &["low", "medium"]
                        } else {
                            &["high"]
                        }
                    }
                    _ => &["low", "medium", "high"],
                };
                (c, lm, oe)
            };
            let effort_opt_id = *u.choose(EFFORT_IDS)?;
            providers_vec.push(provider_json(&id, driver, constraints, listed_model, effort_opt_id, offered_efforts));

            let run_count = u.int_in_range(0..=4u64)?;
            if run_count > 0 || u.arbitrary::<bool>()? {
                running.insert(id.clone(), run_count);
            }

            if !all_ready {
                match u.int_in_range(0..=9u8)? {
                    0 | 1 => {
                        limits.insert(
                            id.clone(),
                            Limit { until: FUTURE_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                        );
                    }
                    2 => {
                        limits.insert(
                            id.clone(),
                            Limit { until: NOW_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                        );
                    }
                    3 => {
                        limits.insert(
                            id.clone(),
                            Limit { until: PAST_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                        );
                    }
                    4 => {
                        limits.insert(
                            id.clone(),
                            Limit { until: "not-a-ts".into(), reason: None, at: PAST_STR.into(), thread: None },
                        );
                    }
                    _ => {}
                }

                match u.int_in_range(0..=9u8)? {
                    0..=3 => {}
                    4 => {
                        let left = *u.choose(&[0.0, -5.0])?;
                        let resets_at = if u.arbitrary::<bool>()? { Some(FUTURE_STR.into()) } else { None };
                        usage.insert(id.clone(), Usage { left, resets_at });
                    }
                    5 => {
                        let left = if reserve > 0.0 {
                            *u.choose(&[reserve, (reserve * 0.5).max(1.0)])?
                        } else {
                            *u.choose(&[10.0, 40.0, 75.0])?
                        };
                        usage.insert(id.clone(), Usage { left, resets_at: None });
                    }
                    _ => {
                        let base = *u.choose(&[10.0, 25.0, 50.0, 70.0, 95.0, 100.0])?;
                        let left = if reserve > 0.0 && reserve < 100.0 && u.arbitrary::<bool>()? {
                            (reserve + 10.0).min(100.0)
                        } else {
                            base
                        };
                        usage.insert(id.clone(), Usage { left, resets_at: None });
                    }
                }
            }

            if seed_credits && u.arbitrary::<bool>()? {
                credits.insert(id.clone(), u.int_in_range(-200..=200)?);
            }

            members.push(PoolMember { instance_id: id, weight, reserve, cost, model: m_model, effort: m_effort });
        }

        let pool = PoolConfig {
            mixed,
            driver: pool_driver,
            model: pool_model,
            effort: pool_effort,
            strategy,
            move_on_limit: MoveOnLimit::Ask,
            members,
        };
        Ok(Scenario { pool, caps: json!({ "providers": providers_vec }), running, limits, usage, credits })
    }

    fn classify_members(sc: &Scenario) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let mut open = Vec::new();
        let mut held = Vec::new();
        let mut hard_skipped = Vec::new();
        let t = now();
        for (i, m) in sc.pool.members.iter().enumerate() {
            let p = provider(&sc.caps, &m.instance_id).expect("generated provider");
            let constraints_non_empty = p["constraints"].as_array().is_some_and(|cs| !cs.is_empty());
            let model = sc.pool.model_of(m);
            let effort = sc.pool.effort_of(m);
            let has_model = lists_model(p, model);
            let has_effort = effort_options(model, effort, p).is_some();
            let limited = sc.limits.get(&m.instance_id).and_then(|l| parse_ts(&l.until)).is_some_and(|u| u > t);
            let u_obs = sc.usage.get(&m.instance_id);
            let exhausted = u_obs.is_some_and(|u| u.left <= 0.0);
            if constraints_non_empty || !has_model || !has_effort || limited || exhausted {
                hard_skipped.push(i);
                continue;
            }
            match u_obs {
                _ if m.reserve <= 0.0 => open.push(i),
                Some(u) if u.left > m.reserve => open.push(i),
                Some(_) => hard_skipped.push(i),
                None => held.push(i),
            }
        }
        (open, held, hard_skipped)
    }

    fn competing_group(sc: &Scenario, candidates: &[usize]) -> Vec<usize> {
        match sc.pool.strategy.as_str() {
            "priority" => Vec::new(),
            "cheapest" => {
                let low = candidates.iter().map(|&i| sc.pool.members[i].cost).reduce(f64::min).unwrap();
                candidates.iter().copied().filter(|&i| sc.pool.members[i].cost == low).collect()
            }
            "headroom" => {
                let left = |i: usize| sc.usage.get(&sc.pool.members[i].instance_id).map(|u| u.left);
                match candidates.iter().filter_map(|&i| left(i)).reduce(f64::max) {
                    Some(most) => candidates.iter().copied().filter(|&i| left(i) == Some(most)).collect(),
                    None => candidates.to_vec(),
                }
            }
            _ => candidates.to_vec(),
        }
    }

    #[test]
    fn choose_respects_eligibility_limits_reserves_and_strategy_bounds() {
        prop(40, |u| {
            let sc = gen_scenario(u, false)?;
            if sc.pool.strategy != "headroom" {
                assert!(sc.pool.validate().is_ok(), "generated invalid pool: {:?}", sc.pool);
            }

            let (open, held, _hard_skipped) = classify_members(&sc);
            let res = choose(&sc.inputs());

            if open.is_empty() && held.is_empty() {
                let err = format!("{:#}", res.expect_err("no eligible members must fail"));
                assert!(err.contains("pool test-pool has no member that can run"), "{err}");
                for m in &sc.pool.members {
                    assert!(err.contains(&m.instance_id), "missing {} in {err}", m.instance_id);
                }
                return Ok(());
            }

            let c = res.expect("eligible members must yield a choice");
            let candidates = if !open.is_empty() { &open } else { &held };
            let chosen_idx = sc
                .pool
                .members
                .iter()
                .position(|m| m.instance_id == c.instance_id)
                .expect("chosen member must exist in pool");
            assert!(candidates.contains(&chosen_idx), "chosen {} not in candidates {:?}", c.instance_id, candidates);

            // Skipped set is exact complement of candidates with no duplicates and excludes chosen.
            let skipped_ids: Vec<&str> = c.skipped.iter().map(|s| s.instance_id.as_str()).collect();
            let skipped_set: BTreeSet<&str> = skipped_ids.iter().copied().collect();
            assert_eq!(skipped_ids.len(), skipped_set.len(), "duplicate skipped entries: {:?}", c.skipped);
            assert!(!skipped_set.contains(c.instance_id.as_str()));
            let cand_set: BTreeSet<&str> =
                candidates.iter().map(|&i| sc.pool.members[i].instance_id.as_str()).collect();
            assert_eq!(cand_set.len() + skipped_set.len(), sc.pool.members.len());
            assert!(cand_set.is_disjoint(&skipped_set));

            // Limit, usage, reserve, and model/driver/effort invariants on chosen member.
            let m = &sc.pool.members[chosen_idx];
            assert!(active_limit(&sc.limits, &m.instance_id, now()).is_none());
            if let Some(obs) = sc.usage.get(&m.instance_id) {
                assert!(obs.left > 0.0, "chose exhausted member: {obs:?}");
                if m.reserve > 0.0 {
                    assert!(obs.left > m.reserve, "chose member below reserve: {obs:?} <= {}", m.reserve);
                }
            }
            let p = provider(&sc.caps, &m.instance_id).unwrap();
            assert_eq!(c.driver, p["driverKind"].as_str().unwrap());
            assert_eq!(c.model, sc.pool.model_of(m));
            assert_eq!(c.effort.as_deref(), sc.pool.effort_of(m));
            assert_eq!(c.options, effort_options(&c.model, c.effort.as_deref(), p).unwrap());

            // Strategy optimality bounds.
            match sc.pool.strategy.as_str() {
                "priority" => {
                    assert_eq!(chosen_idx, candidates[0]);
                    assert_eq!(c.credits, sc.credits);
                }
                "cheapest" => {
                    let min_cost = candidates.iter().map(|&i| sc.pool.members[i].cost).reduce(f64::min).unwrap();
                    assert_eq!(m.cost, min_cost);
                }
                "headroom" => {
                    let max_left = candidates
                        .iter()
                        .filter_map(|&i| sc.usage.get(&sc.pool.members[i].instance_id).map(|u| u.left))
                        .reduce(f64::max);
                    if let Some(best_left) = max_left {
                        assert_eq!(sc.usage.get(&m.instance_id).map(|u| u.left), Some(best_left));
                    }
                }
                _ => {}
            }
            Ok(())
        });
    }

    #[test]
    fn choose_conserves_credits_and_meets_exact_weighted_cycle_bounds() {
        prop(45, |u| {
            // Part A: Single-step conservation and exact deltas across arbitrary initial credits.
            let sc = gen_scenario(u, false)?;
            let (open, held, _) = classify_members(&sc);
            let candidates = if !open.is_empty() { open } else { held };
            if let Ok(c) = choose(&sc.inputs()) {
                let sum_before: i64 = sc.credits.values().sum();
                let sum_after: i64 = c.credits.values().sum();
                assert_eq!(sum_after, sum_before, "credit sum not conserved");

                let group = competing_group(&sc, &candidates);
                let group_ids: BTreeSet<&str> =
                    group.iter().map(|&i| sc.pool.members[i].instance_id.as_str()).collect();
                let total_w: i64 = group.iter().map(|&i| sc.pool.members[i].weight as i64).sum();

                for (k, &before) in &sc.credits {
                    if !group_ids.contains(k.as_str()) {
                        assert_eq!(c.credits.get(k), Some(&before), "non-competing credit mutated for {k}");
                    }
                }
                for &i in &group {
                    let m = &sc.pool.members[i];
                    let before = sc.credits.get(&m.instance_id).copied().unwrap_or(0);
                    let expected = before + m.weight as i64 - if m.instance_id == c.instance_id { total_w } else { 0 };
                    assert_eq!(c.credits.get(&m.instance_id), Some(&expected));
                }
            }

            // Part B: Full W-pick smooth weighted round-robin cycle from zero credits with varying loads.
            let mut cyc = gen_scenario(u, true)?;
            cyc.pool.members.truncate(cyc.pool.members.len().min(6));
            let strat = *u.choose(&["weighted", "cheapest", "headroom"])?;
            cyc.pool.strategy = strat.to_string();
            if strat == "cheapest" {
                let common_cost = *u.choose(COSTS)?;
                for m in &mut cyc.pool.members {
                    m.cost = common_cost;
                }
            }
            cyc.credits.clear();
            cyc.usage.clear();

            let total_w: i64 = cyc.pool.members.iter().map(|m| m.weight as i64).sum();
            let k = cyc.pool.members.len() as i64;
            let max_neg = total_w - (total_w + k - 1) / k;
            let max_pos = (k - 1) * max_neg;
            let mut picks: BTreeMap<String, u32> = BTreeMap::new();

            for _ in 0..total_w {
                for m in &cyc.pool.members {
                    cyc.running.insert(m.instance_id.clone(), u.int_in_range(0..=3u64)?);
                }
                let c = choose(&cyc.inputs()).expect("all members ready");
                let count = picks.entry(c.instance_id.clone()).or_default();
                *count += 1;
                let w = cyc.pool.members.iter().find(|m| m.instance_id == c.instance_id).unwrap().weight;
                assert!(*count <= w, "member {} exceeded cycle weight {w} with count {}", c.instance_id, *count);
                assert_eq!(c.credits.values().sum::<i64>(), 0);
                for m in &cyc.pool.members {
                    let cr = c.credits[&m.instance_id];
                    assert!(
                        (-max_neg..=max_pos).contains(&cr),
                        "credit {cr} for {} out of [{}, {}] (W={total_w}, k={k})",
                        m.instance_id,
                        -max_neg,
                        max_pos
                    );
                }
                cyc.credits = c.credits;
            }
            for m in &cyc.pool.members {
                assert_eq!(picks.get(&m.instance_id).copied().unwrap_or(0), m.weight);
                assert_eq!(cyc.credits[&m.instance_id], 0);
            }
            Ok(())
        });
    }

    fn apply_avoid(base: &Scenario, avoid: &BTreeSet<usize>, mode: u8) -> Scenario {
        let mut sc = base.clone();
        match mode {
            0 => {
                for &i in avoid {
                    let id = sc.pool.members[i].instance_id.clone();
                    sc.limits.insert(
                        id,
                        Limit { until: FUTURE_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                    );
                }
            }
            1 => {
                if let Some(ps) = sc.caps["providers"].as_array_mut() {
                    for &i in avoid {
                        let id = &sc.pool.members[i].instance_id;
                        if let Some(p) = ps.iter_mut().find(|p| p["providerInstanceId"] == *id) {
                            p["constraints"] = json!(["Provider instance is disabled."]);
                        }
                    }
                }
            }
            _ => {
                sc.pool.members = sc
                    .pool
                    .members
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| !avoid.contains(i))
                    .map(|(_, m)| m)
                    .collect();
            }
        }
        sc
    }

    #[test]
    fn choose_avoid_set_is_monotonic_and_satisfies_iia() {
        prop(45, |u| {
            let sc = gen_scenario(u, false)?;
            let n = sc.pool.members.len();
            let mode = u.int_in_range(0..=2u8)?;

            let mut a1 = BTreeSet::new();
            let mut a2 = BTreeSet::new();
            for i in 0..n {
                match u.int_in_range(0..=2u8)? {
                    0 => {}
                    1 => {
                        a2.insert(i);
                    }
                    _ => {
                        a1.insert(i);
                        a2.insert(i);
                    }
                }
            }

            let sc1 = apply_avoid(&sc, &a1, mode);
            let sc2 = apply_avoid(&sc, &a2, mode);
            let (open1, _, _) = classify_members(&sc1);
            let (open2, _, _) = classify_members(&sc2);
            let r1 = choose(&sc1.inputs());
            let r2 = choose(&sc2.inputs());

            // Feasibility monotonicity: adding exclusions can never make an unsolvable pool solvable.
            if r1.is_err() {
                assert!(r2.is_err(), "superset avoid set succeeded when subset failed");
            }
            if let Ok(ref c1) = r1 {
                assert!(a1.iter().all(|&i| sc.pool.members[i].instance_id != c1.instance_id));
                if mode != 2 {
                    let sk1: BTreeSet<&str> = c1.skipped.iter().map(|s| s.instance_id.as_str()).collect();
                    for &i in &a1 {
                        assert!(sk1.contains(sc.pool.members[i].instance_id.as_str()));
                    }
                }
                // Avoiding the winner forces a different choice or Err.
                let winner_idx = sc.pool.members.iter().position(|m| m.instance_id == c1.instance_id).unwrap();
                let mut a_win = a1.clone();
                a_win.insert(winner_idx);
                let sc_win = apply_avoid(&sc, &a_win, 0);
                if let Ok(cw) = choose(&sc_win.inputs()) {
                    assert_ne!(cw.instance_id, c1.instance_id);
                    assert!(a_win.iter().all(|&i| sc.pool.members[i].instance_id != cw.instance_id));
                }
            }
            if let Ok(ref c2) = r2 {
                assert!(a2.iter().all(|&i| sc.pool.members[i].instance_id != c2.instance_id));
                if mode != 2 {
                    let sk2: BTreeSet<&str> = c2.skipped.iter().map(|s| s.instance_id.as_str()).collect();
                    for &i in &a1 {
                        assert!(sk2.contains(sc.pool.members[i].instance_id.as_str()));
                    }
                }
            }

            // IIA & strategy objective monotonicity when the open/last-resort regime does not flip.
            if let (Ok(c1), Ok(c2)) = (r1, r2)
                && open1.is_empty() == open2.is_empty()
            {
                let idx1 = sc.pool.members.iter().position(|m| m.instance_id == c1.instance_id).unwrap();
                let idx2 = sc.pool.members.iter().position(|m| m.instance_id == c2.instance_id).unwrap();
                if !a2.contains(&idx1) {
                    assert_eq!(c2.instance_id, c1.instance_id, "IIA violated under strategy {}", sc.pool.strategy);
                }
                match sc.pool.strategy.as_str() {
                    "priority" => assert!(idx2 >= idx1),
                    "cheapest" => assert!(sc.pool.members[idx2].cost >= sc.pool.members[idx1].cost),
                    "headroom" => {
                        if let (Some(u1), Some(u2)) = (sc.usage.get(&c1.instance_id), sc.usage.get(&c2.instance_id)) {
                            assert!(u2.left <= u1.left);
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        });
    }

    #[test]
    fn choose_tie_breaking_is_deterministic_and_prefers_lower_load_then_order() {
        prop(40, |u| {
            // Part A: Pure determinism on arbitrary inputs.
            let sc = gen_scenario(u, false)?;
            let r1 = choose(&sc.inputs());
            let r2 = choose(&sc.inputs());
            match (r1, r2) {
                (Ok(a), Ok(b)) => assert_eq!(a, b),
                (Err(a), Err(b)) => assert_eq!(format!("{a:#}"), format!("{b:#}")),
                _ => panic!("choose was non-deterministic"),
            }

            // Part B: Exact tie-breaking on equal (credit + weight): lowest running, then earliest index.
            let mut tie = gen_scenario(u, true)?;
            if tie.pool.members.len() < 2 {
                return Ok(());
            }
            let strat = *u.choose(&["weighted", "cheapest", "headroom"])?;
            tie.pool.strategy = strat.to_string();
            let target_score = u.int_in_range(-50..=50i64)?;
            let shared_cost = *u.choose(COSTS)?;
            let shared_left = *u.choose(&[20.0, 50.0, 80.0])?;
            tie.credits.clear();
            tie.usage.clear();
            tie.running.clear();
            for m in &mut tie.pool.members {
                m.cost = shared_cost;
                tie.credits.insert(m.instance_id.clone(), target_score - m.weight as i64);
                if strat == "headroom" {
                    tie.usage.insert(m.instance_id.clone(), Usage { left: shared_left, resets_at: None });
                }
                tie.running.insert(m.instance_id.clone(), u.int_in_range(0..=5u64)?);
            }
            let min_load = tie.pool.members.iter().map(|m| tie.running[&m.instance_id]).min().unwrap();
            let expected = tie.pool.members.iter().find(|m| tie.running[&m.instance_id] == min_load).unwrap();
            let c = choose(&tie.inputs()).unwrap();
            assert_eq!(c.instance_id, expected.instance_id);

            // Setting a later member to load 0 while all others have load >= 1 forces it to win.
            let target_idx = u.int_in_range(1..=(tie.pool.members.len() - 1))?;
            for (i, m) in tie.pool.members.iter().enumerate() {
                tie.running.insert(m.instance_id.clone(), if i == target_idx { 0 } else { 1 + (i as u64) });
            }
            assert_eq!(choose(&tie.inputs()).unwrap().instance_id, tie.pool.members[target_idx].instance_id);

            // Equal loads across all tied members always fall back to pool.members[0].
            let uniform_load = u.int_in_range(0..=5u64)?;
            for m in &tie.pool.members {
                tie.running.insert(m.instance_id.clone(), uniform_load);
            }
            assert_eq!(choose(&tie.inputs()).unwrap().instance_id, tie.pool.members[0].instance_id);

            // Part C: Priority is invariant to running, credits, weight, and cost.
            let mut prio = sc.clone();
            prio.pool.strategy = "priority".to_string();
            let base = choose(&prio.inputs()).ok().map(|c| c.instance_id);
            for m in &mut prio.pool.members {
                m.weight = u.int_in_range(1..=50)?;
                m.cost = *u.choose(COSTS)?;
                prio.running.insert(m.instance_id.clone(), u.int_in_range(0..=20)?);
                prio.credits.insert(m.instance_id.clone(), u.int_in_range(-500..=500)?);
            }
            let perturbed = choose(&prio.inputs()).ok().map(|c| c.instance_id);
            assert_eq!(base, perturbed);
            Ok(())
        });
    }

    #[test]
    fn choose_falls_back_to_last_resort_and_errors_gracefully_when_exhausted() {
        prop(40, |u| {
            let mut sc = gen_scenario(u, true)?;
            let n = sc.pool.members.len();

            // Part A: Last-resort fallback to held (reserve > 0, usage = None) vs floor (usage <= reserve).
            let held_count = u.int_in_range(1..=n.min(4))?;
            let mut held_ids = BTreeSet::new();
            let mut floor_ids = BTreeSet::new();
            let mut open_candidate: Option<usize> = None;

            for (i, m) in sc.pool.members.iter_mut().enumerate() {
                if i < held_count {
                    m.reserve = *u.choose(&[10.0, 25.0, 50.0, 100.0])?;
                    held_ids.insert(m.instance_id.clone());
                } else if u.arbitrary::<bool>()? {
                    // Usage at or below reserve floor: must never be picked, even as last resort.
                    m.reserve = *u.choose(&[20.0, 40.0, 60.0])?;
                    let left = *u.choose(&[m.reserve, (m.reserve - 5.0).max(1.0)])?;
                    sc.usage.insert(m.instance_id.clone(), Usage { left, resets_at: None });
                    floor_ids.insert(m.instance_id.clone());
                } else {
                    // Open member that we saturate via limit/exhaustion/disabled.
                    m.reserve = 0.0;
                    open_candidate.get_or_insert(i);
                    match u.int_in_range(0..=2u8)? {
                        0 => {
                            sc.limits.insert(
                                m.instance_id.clone(),
                                Limit { until: FUTURE_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                            );
                        }
                        1 => {
                            sc.usage.insert(m.instance_id.clone(), Usage { left: 0.0, resets_at: None });
                        }
                        _ => {
                            if let Some(p) = sc.caps["providers"]
                                .as_array_mut()
                                .unwrap()
                                .iter_mut()
                                .find(|p| p["providerInstanceId"] == m.instance_id)
                            {
                                p["constraints"] = json!(["Provider instance is disabled."]);
                            }
                        }
                    }
                }
            }

            let lr = choose(&sc.inputs()).expect("held members must provide last-resort fallback");
            assert!(
                held_ids.contains(&lr.instance_id),
                "expected last resort from {held_ids:?}, got {}",
                lr.instance_id
            );
            assert!(!floor_ids.contains(&lr.instance_id));
            assert!(lr.reason.contains("last resort:"), "{}", lr.reason);
            for fid in &floor_ids {
                assert!(lr.skipped.iter().any(|s| s.instance_id == *fid));
            }

            // Un-saturating a single open member immediately supersedes all last-resort held members.
            if let Some(oi) = open_candidate {
                let oid = sc.pool.members[oi].instance_id.clone();
                sc.limits.remove(&oid);
                sc.usage.remove(&oid);
                if let Some(p) =
                    sc.caps["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["providerInstanceId"] == oid)
                {
                    p["constraints"] = json!([]);
                }
                let recovered = choose(&sc.inputs()).expect("open member must win");
                assert_eq!(recovered.instance_id, oid);
                assert!(!recovered.reason.contains("last resort:"), "{}", recovered.reason);
                for hid in &held_ids {
                    assert!(recovered.skipped.iter().any(|s| s.instance_id == *hid && s.reason.contains("reserve")));
                }
            }

            // Part B: Every member saturated or excluded => graceful Err naming every member.
            let mut ex = gen_scenario(u, true)?;
            for m in &mut ex.pool.members {
                match u.int_in_range(0..=5u8)? {
                    0 => {
                        ex.limits.insert(
                            m.instance_id.clone(),
                            Limit { until: FUTURE_STR.into(), reason: None, at: PAST_STR.into(), thread: None },
                        );
                    }
                    1 => {
                        let resets_at = if u.arbitrary::<bool>()? { Some(FUTURE_STR.into()) } else { None };
                        ex.usage.insert(m.instance_id.clone(), Usage { left: 0.0, resets_at });
                    }
                    2 => {
                        m.reserve = 30.0;
                        ex.usage.insert(m.instance_id.clone(), Usage { left: 20.0, resets_at: None });
                    }
                    3 => {
                        let p = ex.caps["providers"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|p| p["providerInstanceId"] == m.instance_id)
                            .unwrap();
                        p["constraints"] = json!(["Provider instance is disabled."]);
                    }
                    4 => {
                        let p = ex.caps["providers"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|p| p["providerInstanceId"] == m.instance_id)
                            .unwrap();
                        p["constraints"] = json!(["Maintenance window"]);
                    }
                    _ => {
                        let p = ex.caps["providers"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|p| p["providerInstanceId"] == m.instance_id)
                            .unwrap();
                        p["models"] = json!([]);
                    }
                }
            }
            let err = format!("{:#}", choose(&ex.inputs()).expect_err("all excluded must error"));
            assert!(err.contains("pool test-pool has no member that can run"), "{err}");
            for m in &ex.pool.members {
                assert!(err.contains(&m.instance_id), "missing {} in {err}", m.instance_id);
            }
            Ok(())
        });
    }
}
