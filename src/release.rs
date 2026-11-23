use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const REPO: &str = "OttoOps/t3-codemode";
pub const RELEASES: &str = "https://github.com/OttoOps/t3-codemode/releases";
pub const RELEASES_ENV: &str = "T3CM_RELEASES_URL";
pub const NO_CHECK_ENV: &str = "T3CM_NO_UPDATE_CHECK";
pub const CLI_FRESH: SignedDuration = SignedDuration::from_hours(24);
pub const SERVICE_EVERY: Duration = Duration::from_secs(12 * 3600);
const CACHE_FILE: &str = "update-check.json";
const CHECK_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (s, None),
        };
        let mut parts = core
            .split('.')
            .map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { None } else { p.parse::<u64>().ok() });
        let v = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()??, pre };
        parts.next().is_none().then_some(v)
    }

    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver")
    }

    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| match (&self.pre, &o.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => pre_cmp(a, b),
        })
    }
}

fn pre_cmp(a: &str, b: &str) -> Ordering {
    let num =
        |s: &str| (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.parse::<u128>().ok()).flatten();
    let mut x = a.split('.');
    let mut y = b.split('.');
    loop {
        let o = match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => match (num(p), num(q)) {
                (Some(m), Some(n)) => m.cmp(&n),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => p.cmp(q),
            },
        };
        if o != Ordering::Equal {
            return o;
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

pub fn releases_url() -> Result<String> {
    let url = std::env::var(RELEASES_ENV).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| RELEASES.to_string());
    checked_releases(&url)
}

fn checked_releases(url: &str) -> Result<String> {
    let url = url.trim_end_matches('/').to_string();
    let Ok(parsed) = reqwest::Url::parse(&url) else { bail!("{RELEASES_ENV} is not a URL") };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("{RELEASES_ENV} must not carry a user name or password");
    }
    if !(url.starts_with("https://") || is_loopback(&url)) {
        bail!("{RELEASES_ENV} must be https (plain http only on 127.0.0.1 or localhost): {url}");
    }
    Ok(url)
}

pub fn is_loopback(url: &str) -> bool {
    reqwest::Url::parse(url)
        .is_ok_and(|u| u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost")))
}

pub fn notes_url(releases: &str, v: &Version) -> String {
    format!("{releases}/tag/{}", v.tag())
}

pub fn client(releases: &str, follow: bool) -> Result<reqwest::Client> {
    let local = is_loopback(releases);
    let policy = if follow {
        reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() >= 10 {
                a.error("too many redirects")
            } else if a.url().scheme() == "https" || (local && is_loopback(a.url().as_str())) {
                a.follow()
            } else {
                a.error("refused a redirect off https")
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    let mut b = reqwest::Client::builder()
        .redirect(policy)
        .user_agent(concat!("t3cm/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10));
    if local {
        b = b.no_proxy();
    }
    Ok(b.build()?)
}

pub async fn latest(releases: &str) -> Result<Version> {
    let url = format!("{releases}/latest");
    let resp = tokio::time::timeout(CHECK_DEADLINE, client(releases, false)?.get(&url).send())
        .await
        .map_err(|_| anyhow!("{url} did not answer within {}s", CHECK_DEADLINE.as_secs()))?
        .with_context(|| format!("could not reach {url}"))?;
    if !resp.status().is_redirection() {
        bail!("{url} answered {} instead of redirecting to the latest release", resp.status());
    }
    let loc = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("{url} redirected without a Location"))?;
    let tag = loc.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    match tag.strip_prefix('v').and_then(Version::parse) {
        Some(v) => Ok(v),
        None => bail!("no published release at {releases} yet"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    pub latest: String,
    pub checked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_by: Option<String>,
}

impl Checked {
    pub fn now(latest: &Version) -> Checked {
        Checked {
            latest: latest.to_string(),
            checked_at: Timestamp::now().to_string(),
            tried_at: None,
            checked_by: Some(env!("CARGO_PKG_VERSION").to_string()),
        }
    }

    pub fn by_this_install(&self) -> bool {
        self.checked_by.as_deref() == Some(env!("CARGO_PKG_VERSION"))
    }

    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.latest)
    }

    pub fn age(&self) -> Option<SignedDuration> {
        since(&self.checked_at)
    }

    pub fn fresh(&self, within: SignedDuration) -> bool {
        recent(self.age(), within)
    }

    pub fn tried_recently(&self, within: SignedDuration) -> bool {
        recent(self.tried_at.as_deref().and_then(since), within)
    }
}

fn since(at: &str) -> Option<SignedDuration> {
    let at: Timestamp = at.parse().ok()?;
    Some(Timestamp::now().duration_since(at))
}

fn recent(age: Option<SignedDuration>, within: SignedDuration) -> bool {
    age.is_some_and(|a| a >= SignedDuration::ZERO && a < within)
}

pub fn cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join(CACHE_FILE)
}

pub fn read_cache(state_dir: &Path) -> Option<Checked> {
    serde_json::from_str(&std::fs::read_to_string(cache_path(state_dir)).ok()?).ok()
}

pub fn read_own_cache(state_dir: &Path) -> Option<Checked> {
    read_cache(state_dir).filter(Checked::by_this_install)
}

pub fn write_cache(state_dir: &Path, c: &Checked) -> Result<()> {
    std::fs::create_dir_all(state_dir).with_context(|| format!("creating {}", state_dir.display()))?;
    let path = cache_path(state_dir);
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(c)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
}

pub fn check_disabled(config_allows: bool) -> bool {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"));
    !config_allows || set(NO_CHECK_ENV) || set("CI")
}

pub fn notice(current: &Version, latest: &Version, releases: &str) -> Option<String> {
    (latest > current).then(|| {
        format!(
            "A new t3cm is available: {current} -> {latest}. Run `t3cm update` to install it.\nRelease notes: {}",
            notes_url(releases, latest)
        )
    })
}

pub async fn known_latest(state_dir: &Path, releases: &str, within: SignedDuration) -> Option<Version> {
    let prior = read_own_cache(state_dir);
    if let Some(c) = &prior
        && (c.fresh(within) || c.tried_recently(within))
    {
        return c.version();
    }
    let mut attempt = prior.unwrap_or_else(|| Checked {
        latest: String::new(),
        checked_at: String::new(),
        ..Checked::now(&Version::current())
    });
    attempt.tried_at = Some(Timestamp::now().to_string());
    let _ = write_cache(state_dir, &attempt);
    let v = latest(releases).await.ok()?;
    let _ = write_cache(state_dir, &Checked::now(&v));
    Some(v)
}

pub const UPDATE_COMMAND: &str =
    if cfg!(windows) { r#"& "$env:LOCALAPPDATA\Programs\t3-codemode\t3cm.exe" update"# } else { "t3cm update" };

pub fn view(current: &Version, latest: Option<&Version>, releases: &str) -> Value {
    match latest {
        Some(l) => json!({
            "current": current.to_string(),
            "latest": l.max(current).to_string(),
            "available": l > current,
            "notes": notes_url(releases, l),
            "command": UPDATE_COMMAND,
        }),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(v("v0.4.0"), Version { major: 0, minor: 4, patch: 0, pre: None });
        assert_eq!(v("1.2.3-rc.1").pre.as_deref(), Some("rc.1"));
        for bad in ["", "v1", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "v", "1..3", "+1.2.3"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
        assert!(v("1.0.0-rc.1") > v("1.0.0-rc"));
        assert!(v("1.0.0-alpha") > v("1.0.0-1"));
        assert!(v("1.0.0-beta.11") > v("1.0.0-beta.2"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha.1"));
        assert_eq!(v("1.0.0-rc.1").cmp(&v("1.0.0-rc.1")), Ordering::Equal);
        assert_eq!(v("v2.0.0").tag(), "v2.0.0");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_notice_names_both_versions_and_the_release_notes() {
        let r = "https://example.test/releases";
        let n = notice(&v("0.4.0"), &v("0.5.0"), r).unwrap();
        assert!(n.contains("0.4.0 -> 0.5.0") && n.contains("t3cm update"), "{n}");
        assert!(n.contains("https://example.test/releases/tag/v0.5.0"), "{n}");
        assert!(notice(&v("0.5.0"), &v("0.5.0"), r).is_none());
        assert!(notice(&v("0.6.0"), &v("0.5.0"), r).is_none());
        assert_eq!(view(&v("0.4.0"), Some(&v("0.5.0")), r)["available"], true);
        assert_eq!(view(&v("0.5.0"), Some(&v("0.5.0")), r)["available"], false);
        assert!(view(&v("0.5.0"), None, r).is_null());
    }

    #[test]
    fn an_older_published_release_is_never_reported_as_latest() {
        let r = "https://example.test/releases";
        let shown = view(&v("0.7.0"), Some(&v("0.6.0")), r);
        assert_eq!(shown["latest"], "0.7.0", "{shown}");
        assert_eq!(shown["available"], false, "{shown}");
        assert_eq!(shown["notes"], "https://example.test/releases/tag/v0.6.0", "{shown}");
        assert!(notice(&v("0.7.0"), &v("0.6.0"), r).is_none());
    }

    #[tokio::test]
    async fn a_cache_left_by_another_install_is_checked_again() {
        let dir = tempfile::tempdir().unwrap();
        let older = Checked { checked_by: Some("0.0.1".into()), ..Checked::now(&v("0.0.1")) };
        write_cache(dir.path(), &older).unwrap();
        assert!(read_own_cache(dir.path()).is_none());
        let unsigned = Checked { checked_by: None, ..older };
        write_cache(dir.path(), &unsigned).unwrap();
        assert!(read_own_cache(dir.path()).is_none());
        let app = axum::Router::new().route(
            "/releases/latest",
            axum::routing::get(|| async { (axum::http::StatusCode::FOUND, [("location", "/releases/tag/v99.0.0")]) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}/releases", listener.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(known_latest(dir.path(), &url, CLI_FRESH).await, Some(v("99.0.0")));
        let back = read_own_cache(dir.path()).expect("the new check is recorded as this install's");
        assert_eq!(back.version(), Some(v("99.0.0")));
    }

    #[test]
    fn the_cache_round_trips_and_ages() {
        let dir = std::env::temp_dir().join(format!("t3cm-release-test-{}", std::process::id()));
        let c = Checked::now(&v("1.2.3"));
        write_cache(&dir, &c).unwrap();
        let back = read_cache(&dir).unwrap();
        assert_eq!(back, c);
        assert!(back.fresh(CLI_FRESH));
        let old = Checked { checked_at: "2020-01-01T00:00:00Z".into(), ..c };
        assert!(!old.fresh(CLI_FRESH));
        let ahead = Checked { checked_at: "2999-01-01T00:00:00Z".into(), ..old };
        assert!(!ahead.fresh(CLI_FRESH));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_failed_check_waits_out_the_interval_too() {
        let dir = tempfile::tempdir().unwrap();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{closed}/releases");
        assert!(known_latest(dir.path(), &url, CLI_FRESH).await.is_none());
        let c = read_cache(dir.path()).expect("the failed attempt is recorded");
        assert!(c.tried_recently(CLI_FRESH) && !c.fresh(CLI_FRESH) && c.version().is_none());
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/releases", silent.local_addr().unwrap().port());
        let again = tokio::time::timeout(Duration::from_secs(2), known_latest(dir.path(), &url, CLI_FRESH)).await;
        assert_eq!(again.expect("no second lookup inside the interval"), None);
    }

    #[test]
    fn only_https_or_loopback_release_urls() {
        assert!(is_loopback("http://127.0.0.1:9/x"));
        assert!(is_loopback("http://localhost/x"));
        assert!(!is_loopback("http://127.0.0.1.evil.test/x"));
        assert!(!is_loopback("https://127.0.0.1/x"));
        assert!(!is_loopback("http://localhost@evil.test/x"));
        assert!(!is_loopback("http://evil.test\\@127.0.0.1/x"));
        assert!(is_loopback("http://LOCALHOST:8/x"));
    }

    #[test]
    fn release_urls_with_credentials_are_refused_without_echoing_them() {
        assert_eq!(checked_releases("https://example.test/r/").unwrap(), "https://example.test/r");
        for url in [
            "https://me:hunter2@example.test/r",
            "http://me:hunter2@localhost/r",
            "https://tok@example.test/r",
            "hunter2",
        ] {
            let err = checked_releases(url).unwrap_err().to_string();
            assert!(!err.contains("hunter2") && !err.contains("tok@"), "{err}");
        }
    }
}
