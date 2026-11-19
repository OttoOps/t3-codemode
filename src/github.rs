use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::t3::BoxFut;

const GH_DEADLINE: Duration = Duration::from_secs(30);

pub trait GitHub: Send + Sync {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>>;
}

pub struct GhCli;

impl GitHub for GhCli {
    fn graphql<'a>(&'a self, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>> {
        Box::pin(async move {
            let body = serde_json::to_vec(&json!({"query": query, "variables": variables}))?;
            let mut child = tokio::process::Command::new("gh")
                .args(["api", "graphql", "--input", "-"])
                .env("GH_PROMPT_DISABLED", "1")
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| anyhow!("cannot run gh: {e}"))?;
            let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("gh has no stdin"))?;
            let run = async move {
                stdin.write_all(&body).await?;
                drop(stdin);
                child.wait_with_output().await
            };
            let out = tokio::time::timeout(GH_DEADLINE, run)
                .await
                .map_err(|_| anyhow!("gh api graphql timed out after {}s", GH_DEADLINE.as_secs()))??;
            gh_response(out.status.success(), &out.stdout, &out.stderr)
        })
    }
}

pub fn gh_response(ok: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(stdout)
        && (v.get("data").is_some() || v.get("errors").is_some())
    {
        return Ok(v);
    }
    let err = String::from_utf8_lossy(stderr).trim().to_string();
    if ok {
        bail!("gh api graphql returned no GraphQL response");
    }
    bail!("gh api graphql failed: {}", if err.is_empty() { "no output" } else { &err })
}

pub fn errors(resp: &Value) -> Option<String> {
    let errs = resp["errors"].as_array().filter(|a| !a.is_empty())?;
    let msgs: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
    Some(if msgs.is_empty() { "GitHub returned an error".into() } else { msgs.join("; ") })
}
