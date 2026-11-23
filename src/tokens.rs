use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::clock::{iso, parse_ts, plus_minutes};
use crate::store::RunTokenRecord;
use crate::t3::BoxFut;
use crate::wf::Wf;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunTokens {
    pub input: u64,
    pub output: u64,
}

pub trait TokenSource: Send + Sync {
    fn reports(&self) -> bool;
    fn tokens<'a>(&'a self, thread: &'a str, run: &'a Value) -> BoxFut<'a, Result<Option<RunTokens>>>;
}

/// T3's MCP tools carry no token counts, so this source reports none until they do.
pub struct T3Tokens;

impl TokenSource for T3Tokens {
    fn reports(&self) -> bool {
        false
    }

    fn tokens<'a>(&'a self, _thread: &'a str, _run: &'a Value) -> BoxFut<'a, Result<Option<RunTokens>>> {
        Box::pin(async { Ok(None) })
    }
}

const STATUSES: &[&str] = &[
    "idle",
    "preparing",
    "queued",
    "starting",
    "running",
    "waiting",
    "completed",
    "interrupted",
    "failed",
    "cancelled",
];
const DONE: &[&str] = &["completed", "interrupted", "failed", "cancelled", "rolled_back"];
const LOOKBACK_MINUTES: f64 = 24.0 * 60.0;
const KEEP_DAYS: f64 = 400.0;
const RUN_LIMIT: u64 = 20;

impl Wf {
    pub fn set_token_source(&self, source: Arc<dyn TokenSource>) {
        *self.tokens.lock().unwrap_or_else(|e| e.into_inner()) = source;
    }

    pub(crate) fn token_source(&self) -> Arc<dyn TokenSource> {
        self.tokens.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub async fn record_tokens(&self) -> Result<Vec<String>> {
        let source = self.token_source();
        if !source.reports() {
            return Ok(Vec::new());
        }
        let now = self.clock.now();
        self.store.run_tokens_prune(&iso(plus_minutes(now, -KEEP_DAYS * 24.0 * 60.0)))?;
        let mut actions = Vec::new();
        let retry_before = self.token_retry.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut retry = HashSet::new();
        for t in self.threads_with(STATUSES).await? {
            let Some(thread) = t["threadId"].as_str() else { continue };
            let recent = self.mins(t["updatedAt"].as_str()).is_some_and(|m| m <= LOOKBACK_MINUTES);
            let seen = match t["latestRunId"].as_str() {
                Some(run) => self.store.run_tokens_seen(thread, run)?,
                None => true,
            };
            if !recent || (seen && !retry_before.contains(thread)) {
                continue;
            }
            let read = json!({"threadId": thread, "limit": 1, "maxCharsPerItem": 1, "runLimit": RUN_LIMIT});
            let runs = match self.t3.call("t3_thread_read", read).await {
                Ok(r) => r["recentRuns"].as_array().cloned().unwrap_or_default(),
                Err(e) => {
                    tracing::warn!("token count skipped thread {thread}: {}", self.t3.redact(format!("{e:#}")));
                    if retry_before.contains(thread) {
                        retry.insert(thread.to_string());
                    }
                    continue;
                }
            };
            for run in &runs {
                let (Some(id), Some(instance), Some(at)) = (
                    run["runId"].as_str(),
                    run["providerInstanceId"].as_str(),
                    run["completedAt"].as_str().and_then(parse_ts),
                ) else {
                    continue;
                };
                if !DONE.contains(&run["status"].as_str().unwrap_or(""))
                    || self.mins(Some(&iso(at))).is_none_or(|m| m > LOOKBACK_MINUTES)
                    || self.store.run_tokens_seen(thread, id)?
                {
                    continue;
                }
                let tokens = match source.tokens(thread, run).await {
                    Ok(tokens) => tokens,
                    Err(e) => {
                        tracing::warn!("token count skipped run {id}: {}", self.t3.redact(format!("{e:#}")));
                        retry.insert(thread.to_string());
                        continue;
                    }
                };
                let record = RunTokenRecord {
                    thread: thread.into(),
                    run: id.into(),
                    instance: instance.into(),
                    driver: self.driver_of(instance).await,
                    at: iso(at),
                    tokens,
                };
                self.store.run_tokens_put(&record)?;
                if let Some(n) = tokens {
                    actions.push(format!("tokens {id} on {instance}: {} in, {} out", n.input, n.output));
                }
            }
        }
        *self.token_retry.lock().unwrap_or_else(|e| e.into_inner()) = retry;
        if !actions.is_empty() {
            self.changed();
        }
        Ok(actions)
    }
}
