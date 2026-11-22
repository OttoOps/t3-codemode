use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::App;

const CATALOG_EVERY: Duration = Duration::from_secs(600);

pub fn spawn(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    let every = Duration::from_secs(app.config().watcher_interval_secs.max(5));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut catalog_at: Option<Instant> = None;
        let mut upstreams: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            ticker.tick().await;
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
