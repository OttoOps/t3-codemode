use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::board::BaseSpec;
use crate::clock::{iso, local_clock, parse_ts, plus_minutes};
use crate::config::Config;
use crate::store::{KEEP_PICKS, PickRecord};
use crate::wf::Wf;

pub const DEFAULT_WINDOW: &str = "24h";
const MAX_WINDOW_MINUTES: i64 = 366 * 24 * 60;
const BUCKETS: usize = 24;
const RECENT: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub label: String,
    pub minutes: Option<i64>,
}

pub fn parse_window(raw: Option<&str>) -> Result<Window> {
    let w = raw.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_WINDOW);
    if w == "all" {
        return Ok(Window { label: w.into(), minutes: None });
    }
    let bad = || {
        anyhow!(
            "wf.usageStats: window {w:?} is not a duration; use \"all\" or a count of m, h or d, such as 30m, 24h or 7d"
        )
    };
    let unit = w.chars().last().ok_or_else(bad)?;
    let per = match unit {
        'm' => 1,
        'h' => 60,
        'd' => 24 * 60,
        _ => return Err(bad()),
    };
    let n: i64 = w[..w.len() - unit.len_utf8()].parse().map_err(|_| bad())?;
    if n < 1 {
        return Err(bad());
    }
    let minutes = n.checked_mul(per).filter(|m| *m <= MAX_WINDOW_MINUTES);
    let Some(minutes) = minutes else { bail!("wf.usageStats: window {w:?} is longer than 366d") };
    Ok(Window { label: w.into(), minutes: Some(minutes) })
}

pub fn pick_kind(p: &PickRecord) -> String {
    if p.moved_from.is_some() {
        "moved".into()
    } else if p.reason.contains("; last resort: ") {
        "last-resort".into()
    } else {
        p.strategy.clone()
    }
}

pub fn skip_kind(reason: &str) -> &'static str {
    if reason.starts_with("limited until") || reason == "has no usage left" {
        "limited"
    } else if reason.starts_with("held at its") {
        "reserve"
    } else if reason == "disabled" {
        "disabled"
    } else if reason.starts_with("constrained") {
        "constrained"
    } else if reason.starts_with("does not list model") {
        "missing-model"
    } else if reason.starts_with("does not offer effort") {
        "missing-effort"
    } else {
        "other"
    }
}

fn status(m: &Value) -> &'static str {
    match m["state"].as_str() {
        Some("ready") => {
            let reserve = m["reserve"].as_f64().unwrap_or(0.0);
            if reserve > 0.0 && m["usageLeft"].as_f64().is_none_or(|left| left <= reserve) {
                "reserve"
            } else {
                "ready"
            }
        }
        Some("limited") => "limited",
        Some("unknown") | None => "unknown",
        Some(_) => "disabled",
    }
}

fn bump(map: &mut Map<String, Value>, key: &str) {
    let n = map.get(key).and_then(Value::as_u64).unwrap_or(0);
    map.insert(key.into(), json!(n + 1));
}

fn share(part: f64, whole: f64) -> Value {
    if whole > 0.0 { json!((part / whole * 1000.0).round() / 1000.0) } else { Value::Null }
}

fn since_of(window: &Window, now: Timestamp) -> Option<Timestamp> {
    window.minutes.map(|m| plus_minutes(now, -(m as f64)))
}

fn series(picks: &[PickRecord], ids: &[String], start: Timestamp, now: Timestamp) -> Value {
    let span = (now.as_second() - start.as_second()).max(60) as f64 / 60.0;
    let width = span / BUCKETS as f64;
    let zero: Map<String, Value> = ids.iter().map(|id| (id.clone(), json!(0))).collect();
    let mut counts = vec![zero; BUCKETS];
    for p in picks {
        let Some(at) = parse_ts(&p.at) else { continue };
        let mins = (at.as_second() - start.as_second()) as f64 / 60.0;
        let i = ((mins / width).floor().max(0.0) as usize).min(BUCKETS - 1);
        bump(&mut counts[i], &p.instance_id);
    }
    let buckets: Vec<Value> = counts
        .into_iter()
        .enumerate()
        .map(|(i, c)| json!({"start": iso(plus_minutes(start, width * i as f64)), "counts": c}))
        .collect();
    json!({"start": iso(start), "bucketMinutes": (width * 1000.0).round() / 1000.0, "buckets": buckets})
}

pub fn fingerprint(v: &Value) -> Value {
    let members: Vec<Value> = v["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            json!([
                m["instanceId"],
                m["status"],
                m["state"],
                m["reason"],
                m["share"],
                m["picks"],
                m["running"],
                m["limitedUntil"],
                m["usageLeft"],
                m["model"],
                m["effort"]
            ])
        })
        .collect();
    json!([
        v["mode"],
        v["source"],
        v["pool"],
        v["strategy"],
        v["error"],
        v["picks"],
        v["shareBasis"],
        members,
        v["running"],
        v["runningError"],
        v["window"]
    ])
}

impl Wf {
    pub async fn usage_stats(&self, window: Option<&str>) -> Result<Value> {
        let window = parse_window(window)?;
        let config = self.config();
        let running = self.running_by_instance().await;
        let base = match self.resolve_project_slots(None).await {
            Ok(r) => Self::user_ticket_base(&config, &r),
            Err(e) => Err(("user".into(), format!("{e:#}"))),
        };
        let now = self.clock.now();
        let mut out = match base {
            Ok(BaseSpec::Pool { pool, source }) => self.pool_usage(&pool, &source, &window, now, &running).await?,
            Ok(b) => self.single_usage(&config, b, &running).await?,
            Err((source, error)) => json!({"mode": "single", "source": source, "pool": null, "strategy": null,
                                           "error": error, "warnings": [], "members": []}),
        };
        let since = since_of(&window, now);
        out["window"] = json!(window.label);
        out["since"] = json!(since.map(iso));
        out["sinceLocal"] = json!(since.map(|s| local_clock(&iso(s))));
        out["at"] = json!(iso(now));
        for key in ["picks", "shareBasis", "reasons", "series", "recent", "former"] {
            if out.get(key).is_none() {
                out[key] = match key {
                    "picks" => json!(0),
                    "shareBasis" => json!("single"),
                    "reasons" => json!({}),
                    "series" => Value::Null,
                    _ => json!([]),
                };
            }
        }
        match &running {
            Ok(r) => {
                out["running"] =
                    json!(r.iter().map(|(id, n)| json!({"instanceId": id, "threads": n})).collect::<Vec<_>>());
                out["runningError"] = Value::Null;
            }
            Err(e) => {
                out["running"] = Value::Null;
                out["runningError"] = json!(self.t3.redact(format!("{e:#}")));
            }
        }
        Ok(crate::upstream::redact_json(out, &|s| self.t3.redact(s)))
    }

    async fn pool_usage(
        &self,
        name: &str,
        source: &str,
        window: &Window,
        now: Timestamp,
        running: &Result<BTreeMap<String, u64>>,
    ) -> Result<Value> {
        let pool = match self.pool(name) {
            Ok(p) => p,
            Err(e) => {
                return Ok(json!({"mode": "pool", "source": source, "pool": name, "strategy": null,
                                 "error": format!("{e:#}"), "warnings": [], "members": []}));
            }
        };
        let limits = self.store.limits()?;
        let ids: Vec<String> = pool.members.iter().map(|m| m.instance_id.clone()).collect();
        let reported = self.usage_of(&ids).await;
        let mut entry = self.pool_entry(name, &pool, running, &limits, &reported).await?;
        let since = since_of(window, now);
        let picks: Vec<PickRecord> = self
            .store
            .picks(name, KEEP_PICKS as usize)?
            .into_iter()
            .filter(|p| since.is_none_or(|s| parse_ts(&p.at).is_some_and(|a| a >= s)))
            .collect();
        let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
        let mut kinds: BTreeMap<&str, Map<String, Value>> = BTreeMap::new();
        let mut skips: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        let mut reasons = Map::new();
        let mut former: BTreeMap<&str, u64> = BTreeMap::new();
        for p in &picks {
            let kind = pick_kind(p);
            bump(&mut reasons, &kind);
            if ids.contains(&p.instance_id) {
                *counts.entry(&p.instance_id).or_default() += 1;
                bump(kinds.entry(&p.instance_id).or_default(), &kind);
            } else {
                *former.entry(&p.instance_id).or_default() += 1;
            }
            for s in p.skipped.as_array().into_iter().flatten() {
                let (Some(id), Some(why)) = (s["instanceId"].as_str(), s["reason"].as_str()) else { continue };
                bump(skips.entry(id.to_string()).or_default(), skip_kind(why));
            }
        }
        let total = picks.len() as u64;
        let weights: f64 = pool.members.iter().map(|m| m.weight as f64).sum();
        let basis = if total > 0 { "picks" } else { "weight" };
        if let Some(ms) = entry["members"].as_array_mut() {
            for (m, cfg) in ms.iter_mut().zip(&pool.members) {
                let id = cfg.instance_id.as_str();
                let n = counts.get(id).copied().unwrap_or(0);
                m["status"] = json!(status(m));
                m["picks"] = json!(n);
                m["share"] = if total > 0 { share(n as f64, total as f64) } else { share(cfg.weight as f64, weights) };
                m["weightShare"] = share(cfg.weight as f64, weights);
                m["reasons"] = Value::Object(kinds.remove(id).unwrap_or_default());
                m["skips"] = Value::Object(skips.remove(id).unwrap_or_default());
            }
        }
        let start = since.unwrap_or_else(|| picks.last().and_then(|p| parse_ts(&p.at)).unwrap_or(now));
        let recent: Vec<Value> = picks
            .iter()
            .take(RECENT)
            .map(|p| {
                json!({"at": p.at, "atLocal": local_clock(&p.at), "instanceId": p.instance_id, "kind": pick_kind(p),
                       "reason": p.reason, "thread": p.thread})
            })
            .collect();
        entry["mode"] = json!("pool");
        entry["source"] = json!(source);
        entry["picks"] = json!(total);
        entry["shareBasis"] = json!(basis);
        entry["reasons"] = Value::Object(reasons);
        entry["series"] = series(&picks, &ids, start, now);
        entry["recent"] = json!(recent);
        entry["former"] =
            json!(former.into_iter().map(|(id, n)| json!({"instanceId": id, "picks": n})).collect::<Vec<_>>());
        Ok(entry)
    }

    async fn single_usage(
        &self,
        config: &Config,
        base: BaseSpec,
        running: &Result<BTreeMap<String, u64>>,
    ) -> Result<Value> {
        let (mut caps, caps_err) = match self.t3.call("orchestrator_capabilities", json!({})).await {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(self.t3.redact(e.to_string()))),
        };
        let t = self.inspect_base_spec(config, base, caps.as_mut(), caps_err.as_deref()).await;
        let id = t["instanceId"].as_str().map(String::from);
        let driver = caps.as_ref().zip(id.as_deref()).and_then(|(c, id)| {
            c["providers"].as_array()?.iter().find(|p| p["providerInstanceId"] == id)?["driverKind"]
                .as_str()
                .map(String::from)
        });
        let now = self.clock.now();
        let limit = match id.as_deref() {
            Some(i) => self.store.limits()?.remove(i).filter(|l| parse_ts(&l.until).is_some_and(|u| u > now)),
            None => None,
        };
        let error = t["error"].as_str().map(String::from);
        let (state, reason) = match (&error, &limit) {
            (Some(e), _) => {
                let unknown = caps_err.is_some() || e.starts_with("T3 capabilities unreachable");
                (if unknown { "unknown" } else { "disabled" }, Some(e.clone()))
            }
            (None, Some(l)) => ("limited", Some(format!("limited until {}", local_clock(&l.until)))),
            (None, None) => ("ready", None),
        };
        let member = json!({
            "instanceId": id,
            "driver": driver,
            "model": t["model"],
            "effort": t["effort"],
            "state": state,
            "status": state,
            "reason": reason,
            "limitedUntil": limit.as_ref().map(|l| l.until.clone()),
            "limitedUntilLocal": limit.as_ref().map(|l| local_clock(&l.until)),
            "limitReason": limit.as_ref().and_then(|l| l.reason.clone()),
            "running": match (running, id.as_deref()) {
                (Ok(r), Some(i)) => json!(r.get(i).copied().unwrap_or(0)),
                _ => Value::Null,
            },
            "picks": 0,
            "share": if id.is_some() { json!(1) } else { Value::Null },
        });
        Ok(json!({"mode": "single", "source": t["source"], "pool": null, "strategy": null, "error": error,
                  "warnings": [], "members": [member]}))
    }

    pub fn routing_view(&self) -> Value {
        self.routing.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or(Value::Null)
    }

    pub async fn refresh_routing(&self) -> Result<bool> {
        let v = self.usage_stats(None).await?;
        let changed = {
            let mut g = self.routing.lock().unwrap_or_else(|e| e.into_inner());
            let changed = g.as_ref().is_none_or(|old| fingerprint(old) != fingerprint(&v));
            *g = Some(v);
            changed
        };
        if changed {
            self.changed();
        }
        Ok(changed)
    }
}
