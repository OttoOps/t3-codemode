use std::sync::{Mutex, OnceLock};

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

pub struct FakeClock(Mutex<Timestamp>);

impl FakeClock {
    pub fn at(iso: &str) -> FakeClock {
        FakeClock(Mutex::new(parse_ts(iso).expect("valid timestamp")))
    }

    pub fn set(&self, iso: &str) {
        *self.0.lock().unwrap() = parse_ts(iso).expect("valid timestamp");
    }

    pub fn advance_minutes(&self, minutes: i64) {
        let mut t = self.0.lock().unwrap();
        *t = plus_minutes(*t, minutes as f64);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

pub fn iso(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn parse_ts(s: &str) -> Option<Timestamp> {
    s.parse::<Timestamp>().ok()
}

pub fn plus_minutes(t: Timestamp, minutes: f64) -> Timestamp {
    t.checked_add(SignedDuration::from_secs_f64(minutes * 60.0)).unwrap_or(t)
}

pub fn minutes_since(clock: &dyn Clock, s: Option<&str>) -> Option<f64> {
    let t = parse_ts(s?)?;
    Some(clock.now().duration_since(t).as_secs_f64() / 60.0)
}

fn local_zone() -> &'static TimeZone {
    static TZ: OnceLock<TimeZone> = OnceLock::new();
    TZ.get_or_init(|| {
        let name = std::env::var("T3CM_TZ").unwrap_or_else(|_| "America/Denver".into());
        TimeZone::get(&name).unwrap_or(TimeZone::UTC)
    })
}

pub fn local_clock(s: &str) -> String {
    match parse_ts(s) {
        Some(t) => t.to_zoned(local_zone().clone()).strftime("%H:%M %Z").to_string(),
        None => "-".into(),
    }
}

pub fn local_datetime(t: Timestamp) -> String {
    t.to_zoned(local_zone().clone()).strftime("%Y-%m-%d %H:%M %Z").to_string()
}
