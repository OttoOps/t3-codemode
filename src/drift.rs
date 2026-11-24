use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::platform;
use crate::release::Version;
use crate::skills::{self, Bundle, Foreign, Paths, SkillState, Status};
use crate::steps::{self, ConfigState, Env, Kind, Missing, Steps};

pub struct Home {
    pub paths: Paths,
    pub env: Env,
}

impl Home {
    pub fn from_env() -> Result<Home> {
        Ok(Home { paths: Paths::from_env()?, env: Env::from_env()? })
    }

    pub fn under(home: &Path) -> Home {
        Home { paths: Paths::under(home, None), env: Env::under(home, None) }
    }
}

pub fn current(st: &Status, setup: &Steps) -> bool {
    st.current && !setup.needs_update()
}

pub fn newer(installed: &str, release: &str) -> bool {
    matches!((Version::parse(installed), Version::parse(release)), (Some(i), Some(r)) if i > r)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drift {
    pub release: String,
    pub installed: Option<String>,
    pub present: bool,
    pub current: bool,
    pub fix: Option<String>,
    pub service_stale: bool,
    pub foreign: Option<Foreign>,
    pub missing_steps: Vec<Missing>,
    pub config_state: Option<ConfigState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    pub fixes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Drift {
    pub fn of(st: &Status, setup: &Steps) -> Drift {
        let service_stale = st.installed.as_deref().is_some_and(|i| newer(i, &st.release));
        let fix = if service_stale { Some(platform::SERVICE.restart.to_string()) } else { st.fix.clone() };
        let mut fixes: Vec<String> = match &fix {
            Some(f) => vec![f.clone()],
            None => setup.missing.iter().filter(|m| m.kind == Kind::Auto).map(|m| m.fix.clone()).take(1).collect(),
        };
        if !service_stale {
            fixes.extend(steps::setup_fix(setup));
        }
        Drift {
            release: st.release.clone(),
            installed: st.installed.clone(),
            present: st.installed.is_some()
                || st.foreign.is_some()
                || st.skills.iter().any(|k| k.state != SkillState::Missing),
            current: current(st, setup) && !service_stale,
            fix,
            service_stale,
            foreign: st.foreign.clone(),
            missing_steps: setup.missing.clone(),
            config_state: Some(setup.config_state),
            problem: setup.problem.clone(),
            fixes,
            error: None,
        }
    }

    pub fn failed(release: &str, error: String) -> Drift {
        Drift {
            release: release.to_string(),
            installed: None,
            present: true,
            current: false,
            fix: None,
            service_stale: false,
            foreign: None,
            missing_steps: Vec::new(),
            config_state: None,
            problem: None,
            fixes: Vec::new(),
            error: Some(error),
        }
    }
}

pub fn check(home: &Home, bundle: &Bundle) -> Drift {
    match (skills::status(&home.paths, bundle), steps::check(&home.env, bundle)) {
        (Ok(st), Ok(setup)) => Drift::of(&st, &setup),
        (Err(e), _) | (_, Err(e)) => Drift::failed(&bundle.release, format!("{e:#}")),
    }
}

pub struct Cache {
    bundle: Bundle,
    home: Mutex<Option<Home>>,
    last: Mutex<Option<Value>>,
}

impl Default for Cache {
    fn default() -> Cache {
        Cache { bundle: Bundle::embedded(), home: Mutex::new(None), last: Mutex::new(None) }
    }
}

impl Cache {
    pub fn set_home(&self, home: Home) {
        let mut slot = self.home.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(home);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn view(&self) -> Value {
        if let Some(v) = self.last.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return v.clone();
        }
        self.refresh().0
    }

    pub fn refresh(&self) -> (Value, bool) {
        let home = self.home.lock().unwrap_or_else(|e| e.into_inner());
        let drift = match home.as_ref() {
            Some(h) => check(h, &self.bundle),
            None => match Home::from_env() {
                Ok(h) => check(&h, &self.bundle),
                Err(e) => Drift::failed(&self.bundle.release, format!("{e:#}")),
            },
        };
        let v = serde_json::to_value(drift).unwrap_or(Value::Null);
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let changed = last.as_ref() != Some(&v);
        *last = Some(v.clone());
        (v, changed)
    }
}
