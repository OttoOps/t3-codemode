use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::App;

const CATALOG_EVERY: Duration = Duration::from_secs(600);

pub fn spawn(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    let every = Duration::from_secs(app.config.watcher_interval_secs.max(5));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut catalog_at: Option<Instant> = None;
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
            if catalog_at.is_none_or(|t| t.elapsed() >= CATALOG_EVERY) {
                catalog_at = Some(Instant::now());
                match app.refresh_catalog().await {
                    Ok(n) => tracing::debug!(tools = n, "catalog refreshed"),
                    Err(e) => tracing::warn!("catalog refresh failed: {}", app.t3.redact(e)),
                }
            }
        }
    })
}
