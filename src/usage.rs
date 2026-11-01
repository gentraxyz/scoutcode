//! Tokens used per day, kept in ~/.scoutcode/usage.json. The old CLI wrote the same file, so the chart counts its days too.
//! Days are UTC, like the monthly credit. The first time the file is made, the days before are filled in from saved
//! chats, each counted on the day it was last used, so they're only estimates.

use crate::config::home;
use crate::session::{Session, civil, now_ms};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Day {
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub requests: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    /// "YYYY-MM-DD" (UTC) to that day's use.
    #[serde(default)]
    pub days: BTreeMap<String, Day>,
    /// The day the log was started. Days before it were estimated from saved chats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded_on: Option<String>,
}

fn file() -> PathBuf {
    home().join("usage.json")
}

/// "YYYY-MM-DD" (UTC) for a time in milliseconds.
pub fn day_of(ms: u64) -> String {
    let (y, m, d, ..) = civil(ms / 1000);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn today() -> String {
    day_of(now_ms())
}

/// The log, or an empty one when there's none yet.
pub fn load() -> Log {
    std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(log: &Log) {
    let _ = std::fs::create_dir_all(home());
    let tmp = file().with_extension("json.tmp");
    if let Ok(text) = serde_json::to_vec_pretty(log) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file());
        }
    }
}

/// A new log with what the saved chats used, each on the day it was last used.
fn seed() -> Log {
    let mut log = Log { days: BTreeMap::new(), seeded_on: Some(today()) };
    for e in std::fs::read_dir(home().join("sessions")).into_iter().flatten().flatten() {
        let Ok(s) = std::fs::read(e.path()).map(|b| serde_json::from_slice::<Session>(&b)) else { continue };
        let Ok(s) = s else { continue };
        let d = log.days.entry(day_of(s.updated)).or_default();
        d.tokens += s.totals.prompt_tokens + s.totals.completion_tokens;
        d.cost += s.totals.cost;
        d.requests += s.totals.requests;
    }
    log
}

/// Adds one request to today. Call it before the request is added to the chat's totals, so a first-time seed
/// doesn't count it twice.
pub fn record(tokens: u64, cost: f64) {
    let mut log = if file().exists() { load() } else { seed() };
    let d = log.days.entry(today()).or_default();
    d.tokens += tokens;
    if cost.is_finite() {
        d.cost += cost;
    }
    d.requests += 1;
    save(&log);
}

/// The last `n` days, oldest first, ending today; days without use are zero.
pub fn last_days(log: &Log, n: usize) -> Vec<(String, Day)> {
    let now = now_ms();
    (0..n)
        .rev()
        .map(|i| {
            let day = day_of(now.saturating_sub(i as u64 * 86_400_000));
            let d = log.days.get(&day).copied().unwrap_or_default();
            (day, d)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_days_ends_today_and_fills_gaps() {
        let mut log = Log::default();
        log.days.insert(today(), Day { tokens: 5, cost: 0.1, requests: 1 });
        let days = last_days(&log, 3);
        assert_eq!(days.len(), 3);
        assert_eq!(days[2].0, today());
        assert_eq!(days[2].1.tokens, 5);
        assert_eq!(days[0].1, Day::default());
        assert!(days[0].0 < days[1].0 && days[1].0 < days[2].0);
    }

    #[test]
    fn reads_the_cli_format() {
        let log: Log = serde_json::from_str(r#"{"days":{"2026-10-01":{"tokens":10,"cost":0.5,"requests":2}},"seededOn":"2026-10-02"}"#).unwrap();
        assert_eq!(log.days["2026-10-01"].tokens, 10);
        assert_eq!(log.seeded_on.as_deref(), Some("2026-10-02"));
    }
}
