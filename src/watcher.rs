use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::config::Config;

const CATALOG_EVERY: Duration = Duration::from_secs(600);

fn period(config: &Config) -> Duration {
    Duration::from_secs(config.watcher_interval_secs.max(5))
}

fn ticker_of(every: Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

pub fn spawn(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    let mut changes = app.wf.config_changes();
    let mut every = period(&changes.borrow_and_update());
    tokio::spawn(async move {
        let mut ticker = ticker_of(every);
        let mut catalog_at: Option<Instant> = None;
        let mut upstreams: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                Ok(()) = changes.changed() => {
                    let next = period(&changes.borrow_and_update());
                    if next != every {
                        every = next;
                        ticker = ticker_of(every);
                    }
                    continue;
                }
            }
            match app.wf.tick().await {
                Ok(actions) if actions.is_empty() => tracing::debug!("watcher pass: no action"),
                Ok(actions) => tracing::info!(?actions, "watcher pass"),
                Err(e) => tracing::warn!("watcher pass failed: {e:#}"),
            }
            if let Err(e) = app.wf.board_tick().await {
                tracing::warn!("board pass failed: {}", app.t3.redact(format!("{e:#}")));
            }
            let wf = app.wf.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || wf.refresh_skills()).await {
                tracing::warn!("skills check failed: {e}");
            }
            match app.wf.mark_limits().await {
                Ok(actions) if actions.is_empty() => {}
                Ok(actions) => tracing::info!(?actions, "pool limits"),
                Err(e) => tracing::warn!("pool limit pass failed: {}", app.t3.redact(format!("{e:#}"))),
            }
            match app.wf.record_tokens().await {
                Ok(actions) if actions.is_empty() => {}
                Ok(actions) => tracing::debug!(?actions, "token counts"),
                Err(e) => tracing::warn!("token count pass failed: {}", app.t3.redact(format!("{e:#}"))),
            }
            if let Err(e) = app.wf.refresh_routing().await {
                tracing::warn!("routing view not refreshed: {}", app.t3.redact(format!("{e:#}")));
            }
            if catalog_at.is_none_or(|t| t.elapsed() >= CATALOG_EVERY) {
                catalog_at = Some(Instant::now());
                if upstreams.as_ref().is_none_or(|h| h.is_finished()) {
                    let app = app.clone();
                    upstreams = Some(tokio::spawn(async move { app.refresh_upstreams().await }));
                }
                match app.refresh_t3_catalog().await {
                    Ok(n) => tracing::debug!(tools = n, "catalog refreshed"),
                    Err(e) => tracing::warn!("catalog refresh failed: {}", app.t3.redact(e)),
                }
            }
        }
    })
}
