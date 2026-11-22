use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::config_edit::{self, Document, MEMBER_CHECK_BUDGET, Rejected, Revision, effective};
use crate::wf::Wf;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    pub revision: Revision,
    pub file: Map<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct Saved {
    pub revision: Revision,
    pub restart: Option<Vec<&'static str>>,
    pub warnings: Vec<String>,
}

impl Wf {
    pub async fn settings(&self) -> Result<Value> {
        let _g = self.config_lock.lock().await;
        let path = self.config_path();
        let file = Document::read(&path)?;
        let running = self.config();
        let (values, restart, errors) = match file.config() {
            Ok(mut saved) => (Some(effective(&saved)), Some(saved.hold_restart_only(&running)), Vec::new()),
            Err(rejected) => (None, None, rejected.errors),
        };
        Ok(json!({
            "path": path.to_string_lossy(),
            "revision": file.revision(),
            "file": file.value().ok(),
            "values": values,
            "running": effective(&running),
            "restart": restart,
            "errors": errors,
        }))
    }

    pub async fn save_settings(&self, body: SettingsBody) -> Result<Saved> {
        let path = self.config_path();
        let file = Document::read(&path)?;
        file.check_revision(&body.revision)?;
        let draft = file.rewrite(Value::Object(body.file))?;
        let mut warnings = Vec::new();
        if let Some(why) = self.check_changed_pools(draft.config(), &draft.changed_pools()).await? {
            warnings.push(format!("pool members not checked against T3 ({why}); each pick checks them"));
        }

        let _g = self.config_lock.lock().await;
        let revision = draft.save()?;
        // No await from the save to the reload, so a dropped request cannot leave a saved change unapplied.
        let restart = match self.reload_locked() {
            Ok(reloaded) => Some(reloaded.restart),
            Err(e) => {
                let why = self.t3.redact(format!("{e:#}"));
                warnings.push(format!("saved, but the service kept the config it was running: {why}"));
                None
            }
        };
        Ok(Saved { revision, restart, warnings })
    }

    async fn check_changed_pools(&self, config: &Config, pools: &[&str]) -> Result<Option<String>, Rejected> {
        if pools.is_empty() {
            return Ok(None);
        }
        let asked = self.t3.call("orchestrator_capabilities", json!({}));
        let unchecked = match tokio::time::timeout(MEMBER_CHECK_BUDGET, asked).await {
            Ok(Ok(caps)) if caps["providers"].is_array() => {
                config_edit::check_members(config, pools, &caps)?;
                return Ok(None);
            }
            Ok(Ok(_)) => "T3 listed no providers".to_string(),
            Ok(Err(e)) => self.t3.redact(e.to_string()),
            Err(_) => "T3 did not answer in time".to_string(),
        };
        Ok(Some(unchecked))
    }
}
