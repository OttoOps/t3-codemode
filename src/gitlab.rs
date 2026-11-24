use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::t3::BoxFut;

const GLAB_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    pub host: &'a str,
    pub cli: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rest {
    pub status: u16,
    pub body: Value,
}

pub trait GitLab: Send + Sync {
    fn graphql<'a>(&'a self, to: Target<'a>, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>>;

    fn get<'a>(&'a self, to: Target<'a>, path: &'a str) -> BoxFut<'a, Result<Rest>>;

    fn send<'a>(&'a self, to: Target<'a>, method: &'a str, path: &'a str, body: Value) -> BoxFut<'a, Result<Rest>>;
}

pub fn project_id(path: &str) -> String {
    path.replace('/', "%2F")
}

pub struct GlabCli;

impl GlabCli {
    async fn run(to: Target<'_>, args: &[&str], stdin: Option<Vec<u8>>) -> Result<std::process::Output> {
        let mut cmd = tokio::process::Command::new(to.cli);
        cmd.args(["api", "--hostname", to.host])
            .args(args)
            .env("NO_PROMPT", "1")
            .env("GLAB_NO_PROMPT", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| anyhow!("cannot run {}: {e}", to.cli))?;
        let input = child.stdin.take();
        let run = async move {
            if let (Some(mut pipe), Some(body)) = (input, stdin) {
                pipe.write_all(&body).await?;
            }
            child.wait_with_output().await
        };
        tokio::time::timeout(GLAB_DEADLINE, run)
            .await
            .map_err(|_| anyhow!("{} api timed out after {}s", to.cli, GLAB_DEADLINE.as_secs()))?
            .map_err(Into::into)
    }
}

impl GitLab for GlabCli {
    fn graphql<'a>(&'a self, to: Target<'a>, query: &'a str, variables: Value) -> BoxFut<'a, Result<Value>> {
        Box::pin(async move {
            let body = serde_json::to_vec(&json!({"query": query, "variables": variables}))?;
            let out =
                GlabCli::run(to, &["graphql", "-H", "Content-Type: application/json", "--input", "-"], Some(body))
                    .await?;
            graphql_response(to.cli, out.status.success(), &out.stdout, &out.stderr)
        })
    }

    fn get<'a>(&'a self, to: Target<'a>, path: &'a str) -> BoxFut<'a, Result<Rest>> {
        Box::pin(async move {
            let out = GlabCli::run(to, &["--include", path], None).await?;
            rest_response(to.cli, &out.stdout, &out.stderr)
        })
    }

    fn send<'a>(&'a self, to: Target<'a>, method: &'a str, path: &'a str, body: Value) -> BoxFut<'a, Result<Rest>> {
        Box::pin(async move {
            let out = match body {
                Value::Null => GlabCli::run(to, &["--include", "-X", method, path], None).await?,
                body => {
                    let args =
                        ["--include", "-X", method, "-H", "Content-Type: application/json", "--input", "-", path];
                    GlabCli::run(to, &args, Some(serde_json::to_vec(&body)?)).await?
                }
            };
            rest_response(to.cli, &out.stdout, &out.stderr)
        })
    }
}

pub fn graphql_response(cli: &str, ok: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(stdout)
        && (v.get("data").is_some() || v.get("errors").is_some())
    {
        return Ok(v);
    }
    let err = String::from_utf8_lossy(stderr).trim().to_string();
    if ok {
        bail!("{cli} api graphql returned no GraphQL response");
    }
    bail!("{cli} api graphql failed: {}", if err.is_empty() { "no output" } else { &err })
}

pub fn rest_response(cli: &str, stdout: &[u8], stderr: &[u8]) -> Result<Rest> {
    let text = String::from_utf8_lossy(stdout);
    let failed = || {
        let err = String::from_utf8_lossy(stderr).trim().to_string();
        anyhow!("{cli} api failed: {}", if err.is_empty() { "no HTTP response" } else { &err })
    };
    let Some(head) = text.strip_prefix("HTTP/") else { return Err(failed()) };
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()).ok_or_else(failed)?;
    let body = ["\r\n\r\n", "\n\n"].iter().filter_map(|sep| text.split_once(sep)).map(|(_, b)| b).next();
    let body = match body.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => serde_json::from_str(b).map_err(|e| anyhow!("{cli} api returned a body that is not JSON: {e}"))?,
        None => Value::Null,
    };
    Ok(Rest { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_included_rest_responses() {
        let ok = b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"iid\": 4}";
        assert_eq!(rest_response("glab", ok, b"").unwrap(), Rest { status: 200, body: json!({"iid": 4}) });
        let missing = b"HTTP/1.1 404 Not Found\nContent-Type: application/json\n\n{\"message\": \"404 Not found\"}";
        assert_eq!(rest_response("glab", missing, b"glab: 404").unwrap().status, 404);
        let err = rest_response("glab", b"", b"glab: not authenticated").unwrap_err();
        assert!(format!("{err:#}").contains("not authenticated"));
        assert_eq!(project_id("group/sub/project"), "group%2Fsub%2Fproject");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sends_writes_with_a_json_body_and_deletes_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let cli = dir.path().join("glab");
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> {log}\ncat >> {log}\necho >> {log}\nprintf 'HTTP/2.0 200 OK\\r\\n\\r\\n{{\"state\": \"merged\"}}'\n",
            log = log.display()
        );
        crate::fake::write_executable(&cli, &script);
        let to = Target { host: "gitlab.acme.example", cli: cli.to_str().unwrap() };

        let put =
            GlabCli.send(to, "PUT", "projects/a%2Fb/merge_requests/7/merge", json!({"sha": "abc"})).await.unwrap();
        let delete =
            GlabCli.send(to, "DELETE", "projects/a%2Fb/repository/branches/fix%2F7", Value::Null).await.unwrap();

        assert_eq!(put, Rest { status: 200, body: json!({"state": "merged"}) });
        assert_eq!(delete.status, 200);
        let lines: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(String::from).collect();
        assert_eq!(
            lines,
            [
                "api --hostname gitlab.acme.example --include -X PUT -H Content-Type: application/json --input - projects/a%2Fb/merge_requests/7/merge",
                "{\"sha\":\"abc\"}",
                "api --hostname gitlab.acme.example --include -X DELETE projects/a%2Fb/repository/branches/fix%2F7",
                "",
            ]
        );
    }

    #[test]
    fn takes_graphql_errors_as_a_response() {
        let v = graphql_response("glab", false, br#"{"errors": [{"message": "x"}]}"#, b"").unwrap();
        assert_eq!(v["errors"][0]["message"], "x");
        assert!(graphql_response("glab", false, b"", b"boom").unwrap_err().to_string().contains("boom"));
    }
}
