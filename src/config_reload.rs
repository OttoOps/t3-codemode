use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;
use crate::config_edit::Document;
use crate::wf::Wf;

#[derive(Debug, Serialize)]
pub struct Reloaded {
    pub restart: Vec<&'static str>,
}

impl Wf {
    pub async fn reload_config(&self) -> Result<Reloaded> {
        let restart = {
            let _g = self.config_lock.lock().await;
            let path = self.config_path();
            let mut next: Config = Document::read(&path)
                .and_then(|file| Ok(file.config()?))
                .with_context(|| format!("config {}", path.display()))?;
            let restart = next.hold_restart_only(&self.config());
            self.config.send_replace(Arc::new(next));
            self.models_rev.fetch_add(1, Ordering::AcqRel);
            restart
        };
        self.changed();
        tracing::info!(?restart, "config reloaded");
        Ok(Reloaded { restart })
    }
}
