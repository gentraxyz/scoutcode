//! What each provider last said about its limits, for Settings > Usage and the composer's free-request count. Kept in
//! memory only: limits change on their own, so a saved one would be stale by the next start.
//!
//! It comes from wherever a provider says it: ScoutCode's free-model headers and `GET /v1/free`, the
//! `x-ratelimit-*` headers OpenAI-style APIs send (OpenRouter, NVIDIA NIM, Kilo, additional providers), OpenRouter's
//! `GET /key`, Copilot's quota snapshots, Claude Code's usage call and the limits its adapter passes along with each
//! answer, Codex's app server, and Grok Build's billing call. Devin and OpenCode only send token counts over ACP, and Kiro's
//! usage call hasn't been wired up yet.

use crate::session::now_ms;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// One limit: how much is left of how much, and when it comes back.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    /// "Free requests today", "Requests", "Premium requests".
    pub label: String,
    pub left: Option<f64>,
    pub limit: Option<f64>,
    /// "requests", "tokens", "USD" or "%".
    pub unit: &'static str,
    /// When it resets, in milliseconds since 1970.
    pub resets_at: Option<u64>,
    /// Anything else worth saying, like "Unlimited".
    pub note: Option<String>,
}

impl Meter {
    fn new(label: &str, left: Option<f64>, limit: Option<f64>, unit: &'static str, resets_at: Option<u64>) -> Meter {
        Meter { label: label.into(), left, limit, unit, resets_at, note: None }
    }
}

/// What one provider (or one of its keys) last said.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub provider: String,
    /// Which key, for a provider with more than one (OpenRouter).
    pub account: Option<String>,
    /// ScoutCode's free tier: "guest", "desktop", "starter" or "paying".
    pub tier: Option<String>,
    pub meters: Vec<Meter>,
    /// When it was heard, in milliseconds since 1970.
    pub at: u64,
}

static STORE: Mutex<BTreeMap<(String, String), Snapshot>> = Mutex::new(BTreeMap::new());

/// Saves what a provider said. Meters replace the ones with the same label; the others stay until they're said again.
pub fn record(provider: &str, account: Option<&str>, tier: Option<String>, meters: Vec<Meter>) {
    if meters.is_empty() {
        return;
    }
    let mut store = STORE.lock().unwrap();
    let s = store.entry((provider.to_string(), account.unwrap_or("").to_string())).or_insert_with(|| Snapshot {
        provider: provider.into(),
        account: account.map(String::from),
        tier: None,
        meters: vec![],
        at: 0,
    });
    for m in meters {
        match s.meters.iter_mut().find(|x| x.label == m.label) {
            Some(x) => *x = m,
            None => s.meters.push(m),
        }
    }
    if tier.is_some() {
        s.tier = tier;
    }
    s.at = now_ms();
}

/// Everything heard so far, by provider.
pub fn all() -> Vec<Snapshot> {
    STORE.lock().unwrap().values().cloned().collect()
}

/// Forgets a provider, as when its account is signed out.
pub fn forget(provider: &str) {
    STORE.lock().unwrap().retain(|(p, _), _| p != provider);
}

// ---------------------------------------------------------------- time

/// Milliseconds since 1970 for "2026-11-05T00:00:00.000Z" (or with an offset, or just a date). None when it isn't one.
pub fn parse_iso(s: &str) -> Option<u64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.as_bytes().get(4) != Some(&b'-') || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (h, mi, sec) = if s.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    // An offset after the seconds (and any fraction): "+02:00" means two hours ahead of UTC.
    let rest = s.get(19..).unwrap_or("");
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.as_bytes().first() {
        Some(b'+') | Some(b'-') => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let oh = rest.get(1..3)?.parse::<i64>().ok()?;
            let om = rest.get(4..6).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
            sign * (oh * 3600 + om * 60)
        }
        _ => 0,
    };
    // Days since 1970 from a civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset;
    u64::try_from(secs).ok().map(|s| s * 1000)
}

/// Milliseconds in "1m30s", "2m59.56s", "450ms", "6h" or a plain number of seconds. None when it isn't one.
fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<f64>() {
        return (n >= 0.0).then(|| (n * 1000.0) as u64);
    }
    let (mut total, mut num) = (0f64, String::new());
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        let ms = match c {
            'h' => 3_600_000.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                1.0
            }
            'm' => 60_000.0,
            's' => 1000.0,
            _ => return None,
        };
        total += n * ms;
        any = true;
    }
    (any && num.is_empty()).then_some(total as u64)
}

/// When a reset header's value comes round: a time (seconds or milliseconds since 1970, or ISO) or a wait from now.
fn reset_at(v: &str, now: u64) -> Option<u64> {
    let v = v.trim();
    if let Ok(n) = v.parse::<f64>() {
        // Big numbers are times; small ones are waits.
        return Some(if n > 1e12 { n as u64 } else if n > 1e9 { (n * 1000.0) as u64 } else { now + (n * 1000.0) as u64 });
    }
    parse_iso(v).or_else(|| parse_duration(v).map(|d| now + d))
}

// ---------------------------------------------------------------- headers

/// What ScoutCode's free-model headers say: requests left today and this minute. Empty on paid answers.
pub fn scoutcode_meters(get: &dyn Fn(&str) -> Option<String>, status: u16) -> (Option<String>, Vec<Meter>) {
    let n = |h: &str| get(h).and_then(|v| v.trim().parse::<f64>().ok());
    let tier = get("x-scoutcode-free-tier");
    let now = now_ms();
    let resets = get("x-scoutcode-free-resets").and_then(|v| parse_iso(&v));
    let mut out = vec![];
    if let Some(left) = n("x-scoutcode-free-left-today") {
        out.push(Meter::new("Free requests today", Some(left), n("x-scoutcode-free-limit-today"), "requests", resets));
    }
    if let Some(left) = n("x-scoutcode-free-left-minute") {
        out.push(Meter::new("Free requests this minute", Some(left), n("x-scoutcode-free-limit-minute"), "requests", Some(now + 60_000)));
    }
    // A free model refused for its limit: a wait of an hour or more means the day's are gone, otherwise the minute's.
    if status == 429 && tier.is_some() && out.is_empty() {
        let wait = n("retry-after").unwrap_or(0.0);
        let at = Some(now + (wait * 1000.0) as u64);
        out.push(if wait >= 3600.0 { Meter::new("Free requests today", Some(0.0), None, "requests", at) } else { Meter::new("Free requests this minute", Some(0.0), None, "requests", at) });
    }
    (tier, out)
}

/// What the `x-ratelimit-*` headers say. OpenAI, Groq, NVIDIA and most OpenAI-style APIs send `-requests` and `-tokens`;
/// OpenRouter and some others send them without a suffix.
pub fn ratelimit_meters(get: &dyn Fn(&str) -> Option<String>) -> Vec<Meter> {
    let n = |h: &str| get(h).and_then(|v| v.trim().parse::<f64>().ok());
    let now = now_ms();
    let mut out = vec![];
    for (suffix, label, unit) in [("-requests", "Requests", "requests"), ("-tokens", "Tokens", "tokens"), ("", "Requests", "requests")] {
        let Some(left) = n(&format!("x-ratelimit-remaining{suffix}")) else { continue };
        if out.iter().any(|m: &Meter| m.label == label) {
            continue;
        }
        let resets = get(&format!("x-ratelimit-reset{suffix}")).and_then(|v| reset_at(&v, now));
        out.push(Meter::new(label, Some(left), n(&format!("x-ratelimit-limit{suffix}")), unit, resets));
    }
    out
}

/// Records what a response's headers say, for the provider at `base_url`. `account` names the key when there are several.
pub fn observe(base_url: &str, account: Option<&str>, status: u16, get: &dyn Fn(&str) -> Option<String>) {
    // ScoutCode's own answers say so; that also covers a team's providers, which go through its server.
    if get("x-scoutcode-free-tier").is_some() || get("x-scoutcode-cost").is_some() {
        let (tier, meters) = scoutcode_meters(get, status);
        record("scoutcode", None, tier, meters);
        return;
    }
    let Some(provider) = provider_at(base_url) else { return };
    record(&provider, account, None, ratelimit_meters(get));
}

/// Which provider a base URL belongs to.
fn provider_at(base_url: &str) -> Option<String> {
    use crate::config::{ADDITIONAL, HERMES_URL, NVIDIA_URL, OPENROUTER_URL, kilo_url};
    let url = base_url.trim_end_matches('/');
    if url == OPENROUTER_URL {
        return Some("openrouter".into());
    }
    if url == NVIDIA_URL {
        return Some("nvidia".into());
    }
    if url == kilo_url() {
        return Some("kilo".into());
    }
    if url == HERMES_URL {
        return Some("hermes".into());
    }
    ADDITIONAL.iter().find(|a| a.url.trim_end_matches('/') == url).map(|a| a.id.to_string())
}

// ---------------------------------------------------------------- answers

/// What ScoutCode's `GET /v1/free` says.
pub fn record_scoutcode_free(v: &Value) {
    let n = |k: &str| v.get(k).and_then(Value::as_f64);
    let tier = v.get("tier").and_then(Value::as_str).map(String::from);
    let resets = v.get("resetsAt").and_then(Value::as_str).and_then(parse_iso);
    let meters = if tier.is_none() {
        // Not counted: the account is unlimited.
        vec![Meter { note: Some("Unlimited".into()), ..Meter::new("Free requests today", None, None, "requests", None) }]
    } else {
        vec![
            Meter::new("Free requests today", n("leftToday"), n("perDay"), "requests", resets),
            Meter::new("Free requests this minute", n("leftMinute"), n("perMinute"), "requests", None),
        ]
    };
    record("scoutcode", None, tier, meters);
}

/// What OpenRouter's `GET /key` says about one key: its credit limit and what it has spent.
pub fn record_openrouter_key(name: &str, v: &Value) {
    let d = v.get("data").unwrap_or(v);
    let n = |k: &str| d.get(k).and_then(Value::as_f64);
    let mut m = match n("limit") {
        Some(limit) => Meter::new("Credit on this key", n("limit_remaining").or_else(|| Some((limit - n("usage").unwrap_or(0.0)).max(0.0))), Some(limit), "USD", None),
        None => Meter { note: Some(format!("No limit on this key. {} spent.", money(n("usage").unwrap_or(0.0)))), ..Meter::new("Credit on this key", None, None, "USD", None) },
    };
    if d.get("is_free_tier").and_then(Value::as_bool) == Some(true) {
        let note = "Free tier: OpenRouter limits free models per day until the account buys credits.";
        m.note = Some(m.note.map_or(note.to_string(), |n| format!("{n} {note}")));
    }
    record("openrouter", Some(name), None, vec![m]);
}

/// Seconds or milliseconds since 1970, as milliseconds.
fn unix_ms(v: &Value) -> Option<u64> {
    let n = v.as_f64().filter(|n| *n > 0.0)?;
    Some(if n > 1e12 { n as u64 } else { (n * 1000.0) as u64 })
}

/// A window the provider measures in percent used.
fn used_percent(label: &str, used: f64, resets_at: Option<u64>) -> Meter {
    Meter::new(label, Some((100.0 - used).clamp(0.0, 100.0)), Some(100.0), "%", resets_at)
}

/// "pro" -> "Pro", "SUPERGROK" stays as it is.
fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Claude's windows by their key, with the labels both of its sources use, so each replaces the other.
const CLAUDE_WINDOWS: [(&str, &str); 4] = [("five_hour", "5-hour limit"), ("seven_day", "Weekly limit"), ("seven_day_opus", "Weekly limit, Opus"), ("seven_day_sonnet", "Weekly limit, Sonnet")];

/// What Claude Code's usage call says: the plan, and how much of each window is used, in percent.
pub fn record_claude_usage(v: &Value) {
    let tier = v.get("subscription_type").and_then(Value::as_str).map(|p| format!("Claude {}", title(p)));
    let mut meters = vec![];
    let r = v.get("rate_limits").unwrap_or(&Value::Null);
    let window = |w: &Value| Some((w.get("utilization")?.as_f64()?, w.get("resets_at").and_then(Value::as_str).and_then(parse_iso)));
    for (key, label) in CLAUDE_WINDOWS {
        if let Some((used, resets)) = r.get(key).and_then(window) {
            meters.push(used_percent(label, used, resets));
        }
    }
    for w in r.get("model_scoped").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(name), Some((used, resets))) = (w.get("display_name").and_then(Value::as_str), window(w)) {
            meters.push(used_percent(&format!("Weekly limit, {name}"), used, resets));
        }
    }
    if let Some(e) = r.get("extra_usage").filter(|e| e.get("is_enabled").and_then(Value::as_bool) == Some(true)) {
        if let Some(used) = e.get("utilization").and_then(Value::as_f64) {
            meters.push(used_percent("Extra usage this month", used, None));
        }
    }
    if meters.is_empty() && v.get("rate_limits_available").and_then(Value::as_bool) == Some(false) {
        meters.push(Meter { note: Some("This sign-in has no plan limits to show.".into()), ..Meter::new("Plan limits", None, None, "%", None) });
    }
    record("claude", None, tier, meters);
}

/// What Claude Code's adapter passes along with an answer: the window the answer counted against, and every window's
/// share used when it says them all. Shares here are fractions, not percents.
pub fn record_claude_rate_limit(info: &Value) {
    let mut meters = vec![];
    let windows = info.get("unifiedWindows").unwrap_or(&Value::Null);
    for (key, label) in CLAUDE_WINDOWS {
        if let Some(w) = windows.get(key) {
            if let Some(used) = w.get("utilization").and_then(Value::as_f64) {
                meters.push(used_percent(label, used * 100.0, w.get("resetsAt").and_then(unix_ms)));
            }
        }
    }
    if meters.is_empty() {
        let kind = info.get("rateLimitType").and_then(Value::as_str).unwrap_or("");
        if let (Some((_, label)), Some(used)) = (CLAUDE_WINDOWS.iter().find(|(k, _)| *k == kind), info.get("utilization").and_then(Value::as_f64)) {
            meters.push(used_percent(label, used * 100.0, info.get("resetsAt").and_then(unix_ms)));
        }
    }
    record("claude", None, None, meters);
}

/// "5-hour limit", "Weekly limit", "30-day limit", from a window's length in minutes.
fn window_label(mins: Option<f64>) -> String {
    match mins.map(|m| m.round() as u64) {
        Some(300) => "5-hour limit".into(),
        Some(10080) => "Weekly limit".into(),
        Some(m) if m >= 1440 && m % 1440 == 0 => format!("{}-day limit", m / 1440),
        Some(m) if m >= 60 && m % 60 == 0 => format!("{}-hour limit", m / 60),
        Some(m) => format!("{m}-minute limit"),
        None => "Limit".into(),
    }
}

/// What Codex's app server says (`account/rateLimits/read`): each limit's windows, in percent used, and any credits.
pub fn record_codex(v: &Value) {
    let mut snapshots: Vec<&Value> = v.get("rateLimitsByLimitId").and_then(Value::as_object).map(|m| m.values().collect()).unwrap_or_default();
    if snapshots.is_empty() {
        snapshots.extend(v.get("rateLimits"));
    }
    let mut tier = None;
    let mut meters = vec![];
    for s in snapshots {
        let id = s.get("limitId").and_then(Value::as_str).unwrap_or("codex");
        // The plan's own limit is "codex"; others, such as a model's, say whose they are.
        let prefix = if id == "codex" { String::new() } else { format!("{} ", s.get("limitName").and_then(Value::as_str).unwrap_or(id)) };
        for w in ["primary", "secondary"].iter().filter_map(|k| s.get(*k)).filter(|w| w.is_object()) {
            let Some(used) = w.get("usedPercent").and_then(Value::as_f64) else { continue };
            let label = window_label(w.get("windowDurationMins").and_then(Value::as_f64));
            meters.push(used_percent(&format!("{prefix}{label}"), used, w.get("resetsAt").and_then(unix_ms)));
        }
        if let Some(c) = s.get("credits").filter(|c| c.get("hasCredits").and_then(Value::as_bool) == Some(true)) {
            let label = format!("{prefix}Credits");
            meters.push(if c.get("unlimited").and_then(Value::as_bool) == Some(true) {
                Meter { note: Some("Unlimited".into()), ..Meter::new(&label, None, None, "credits", None) }
            } else {
                Meter::new(&label, c.get("balance").and_then(|b| b.as_f64().or_else(|| b.as_str()?.trim().parse().ok())), None, "credits", None)
            });
        }
        if tier.is_none() {
            tier = s.get("planType").and_then(Value::as_str).map(|p| format!("ChatGPT {}", title(p)));
        }
    }
    record("codex", None, tier, meters);
}

/// What Grok Build's billing call says: the share of this period's credit used, and when the period ends.
pub fn record_grok_billing(v: &Value) {
    let c = v.get("config").unwrap_or(&Value::Null);
    let tier = v.get("subscription_tier").and_then(Value::as_str).map(String::from);
    let Some(used) = c.get("creditUsagePercent").and_then(Value::as_f64) else {
        record("grok", None, tier, vec![]);
        return;
    };
    let period = c.get("currentPeriod").unwrap_or(&Value::Null);
    let label = match period.get("type").and_then(Value::as_str).unwrap_or("") {
        t if t.ends_with("DAILY") => "Credit today",
        t if t.ends_with("WEEKLY") => "Credit this week",
        t if t.ends_with("MONTHLY") => "Credit this month",
        _ => "Credit this period",
    };
    let ends = period.get("end").or_else(|| c.get("billingPeriodEnd")).and_then(Value::as_str).and_then(parse_iso);
    record("grok", None, tier, vec![used_percent(label, used, ends)]);
}

fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// What Copilot's quota snapshots say, keyed by kind: "chat", "completions", "premium_interactions".
pub fn record_copilot(snapshots: &Value) {
    let Some(map) = snapshots.as_object() else { return };
    let mut meters = vec![];
    for (kind, s) in map {
        let label = match kind.as_str() {
            "chat" => "Chat requests".to_string(),
            "completions" => "Completions".to_string(),
            "premium_interactions" => "Premium requests".to_string(),
            other => other.replace('_', " "),
        };
        let n = |k: &str| s.get(k).and_then(Value::as_f64);
        let resets = s.get("resetDate").and_then(Value::as_str).and_then(parse_iso);
        let unlimited = s.get("isUnlimitedEntitlement").and_then(Value::as_bool) == Some(true) || n("entitlementRequests") == Some(-1.0);
        let m = if unlimited {
            Meter { note: Some("Unlimited".into()), ..Meter::new(&label, None, None, "requests", resets) }
        } else if s.get("tokenBasedBilling").and_then(Value::as_bool) == Some(true) || n("entitlementRequests").is_none() {
            Meter::new(&label, n("remainingPercentage"), Some(100.0), "%", resets)
        } else {
            let limit = n("entitlementRequests").unwrap_or(0.0);
            let mut m = Meter::new(&label, Some((limit - n("usedRequests").unwrap_or(0.0)).max(0.0)), Some(limit), "requests", resets);
            if let Some(o) = n("overage").filter(|o| *o > 0.0) {
                m.note = Some(format!("{o} more past the allowance"));
            }
            m
        };
        meters.push(m);
    }
    record("copilot", None, None, meters);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(list: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let list: Vec<(String, String)> = list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |h: &str| list.iter().find(|(k, _)| k == h).map(|(_, v)| v.clone())
    }

    #[test]
    fn reads_iso_times() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-11-05T00:00:00.000Z"), Some(1_793_836_800_000));
        assert_eq!(parse_iso("2026-11-05T02:00:00+02:00"), Some(1_793_836_800_000));
        assert_eq!(parse_iso("2026-11-05"), Some(1_793_836_800_000));
        assert_eq!(parse_iso("soon"), None);
    }

    #[test]
    fn reads_waits() {
        assert_eq!(parse_duration("1m30s"), Some(90_000));
        assert_eq!(parse_duration("2m59.5s"), Some(179_500));
        assert_eq!(parse_duration("450ms"), Some(450));
        assert_eq!(parse_duration("6h"), Some(21_600_000));
        assert_eq!(parse_duration("12"), Some(12_000));
        assert_eq!(parse_duration("later"), None);
        assert_eq!(reset_at("1793836800000", 5), Some(1_793_836_800_000));
        assert_eq!(reset_at("1793836800", 5), Some(1_793_836_800_000));
        assert_eq!(reset_at("2s", 5), Some(2005));
    }

    #[test]
    fn reads_scoutcodes_free_headers() {
        let get = headers(&[
            ("x-scoutcode-cost", "0"),
            ("x-scoutcode-free-tier", "starter"),
            ("x-scoutcode-free-left-today", "399"),
            ("x-scoutcode-free-limit-today", "400"),
            ("x-scoutcode-free-left-minute", "14"),
            ("x-scoutcode-free-limit-minute", "15"),
            ("x-scoutcode-free-resets", "2026-11-05T00:00:00.000Z"),
        ]);
        let (tier, m) = scoutcode_meters(&get, 200);
        assert_eq!(tier.as_deref(), Some("starter"));
        assert_eq!((m[0].left, m[0].limit, m[0].resets_at), (Some(399.0), Some(400.0), Some(1_793_836_800_000)));
        assert_eq!((m[1].left, m[1].limit), (Some(14.0), Some(15.0)));
        // A server from before the limit headers still says what's left.
        let (_, m) = scoutcode_meters(&headers(&[("x-scoutcode-free-tier", "paying"), ("x-scoutcode-free-left-today", "7")]), 200);
        assert_eq!((m[0].left, m[0].limit), (Some(7.0), None));
        // Refused for the day.
        let (_, m) = scoutcode_meters(&headers(&[("x-scoutcode-free-tier", "desktop"), ("retry-after", "7200")]), 429);
        assert_eq!((m[0].label.as_str(), m[0].left), ("Free requests today", Some(0.0)));
        // A paid answer says nothing about free requests.
        assert!(scoutcode_meters(&headers(&[("x-scoutcode-cost", "0.01")]), 200).1.is_empty());
    }

    #[test]
    fn reads_ratelimit_headers() {
        let get = headers(&[
            ("x-ratelimit-limit-requests", "1000"),
            ("x-ratelimit-remaining-requests", "998"),
            ("x-ratelimit-reset-requests", "1m"),
            ("x-ratelimit-limit-tokens", "6000"),
            ("x-ratelimit-remaining-tokens", "5000"),
        ]);
        let m = ratelimit_meters(&get);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].label.as_str(), m[0].left, m[0].limit), ("Requests", Some(998.0), Some(1000.0)));
        assert!(m[0].resets_at.is_some());
        assert_eq!((m[1].unit, m[1].left), ("tokens", Some(5000.0)));
        // OpenRouter's, without a suffix.
        let m = ratelimit_meters(&headers(&[("x-ratelimit-limit", "20"), ("x-ratelimit-remaining", "19"), ("x-ratelimit-reset", "1793836800000")]));
        assert_eq!((m[0].left, m[0].limit, m[0].resets_at), (Some(19.0), Some(20.0), Some(1_793_836_800_000)));
        assert!(ratelimit_meters(&headers(&[])).is_empty());
    }

    #[test]
    fn reads_copilot_snapshots() {
        record_copilot(&json!({
            "premium_interactions": { "isUnlimitedEntitlement": false, "entitlementRequests": 300, "usedRequests": 120, "remainingPercentage": 60, "overage": 0, "resetDate": "2026-12-01T00:00:00Z" },
            "chat": { "isUnlimitedEntitlement": true, "entitlementRequests": -1, "usedRequests": 4, "remainingPercentage": 100, "overage": 0 },
        }));
        let s = all().into_iter().find(|s| s.provider == "copilot").unwrap();
        let premium = s.meters.iter().find(|m| m.label == "Premium requests").unwrap();
        assert_eq!((premium.left, premium.limit), (Some(180.0), Some(300.0)));
        assert!(premium.resets_at.is_some());
        let chat = s.meters.iter().find(|m| m.label == "Chat requests").unwrap();
        assert_eq!(chat.note.as_deref(), Some("Unlimited"));
    }

    #[test]
    fn reads_openrouter_keys() {
        record_openrouter_key("Work", &json!({ "data": { "limit": 10.0, "usage": 2.5, "limit_remaining": 7.5, "is_free_tier": false } }));
        record_openrouter_key("Free", &json!({ "data": { "limit": null, "usage": 0, "is_free_tier": true } }));
        let all = all();
        let work = all.iter().find(|s| s.account.as_deref() == Some("Work")).unwrap();
        assert_eq!((work.meters[0].left, work.meters[0].limit), (Some(7.5), Some(10.0)));
        let free = all.iter().find(|s| s.account.as_deref() == Some("Free")).unwrap();
        assert!(free.meters[0].note.as_deref().unwrap().contains("Free tier"));
    }

    #[test]
    fn later_meters_replace_earlier_ones_of_the_same_name() {
        record("test-merge", None, None, vec![Meter::new("A", Some(1.0), None, "requests", None), Meter::new("B", Some(1.0), None, "requests", None)]);
        record("test-merge", None, None, vec![Meter::new("A", Some(0.0), None, "requests", None)]);
        let s = all().into_iter().find(|s| s.provider == "test-merge").unwrap();
        assert_eq!(s.meters.iter().map(|m| (m.label.as_str(), m.left)).collect::<Vec<_>>(), vec![("A", Some(0.0)), ("B", Some(1.0))]);
        forget("test-merge");
        assert!(!all().iter().any(|s| s.provider == "test-merge"));
    }

    fn meter(provider: &str, label: &str) -> Meter {
        let s = all().into_iter().find(|s| s.provider == provider).unwrap();
        s.meters.into_iter().find(|m| m.label == label).unwrap_or_else(|| panic!("no {label}"))
    }

    /// Both of Claude Code's sources, as its CLI and adapter send them: a usage call in percent, then an answer's
    /// limits in fractions, which replace the same windows.
    #[test]
    fn reads_claudes_limits() {
        record_claude_usage(&json!({
            "subscription_type": "pro", "rate_limits_available": true,
            "rate_limits": {
                "five_hour": { "utilization": 50, "resets_at": "2026-11-05T00:20:00.352557+00:00" },
                "seven_day": { "utilization": 92, "resets_at": "2026-11-07T17:00:00.352577+00:00" },
                "seven_day_opus": null,
                "extra_usage": { "is_enabled": false, "utilization": null },
            },
        }));
        let s = all().into_iter().find(|s| s.provider == "claude").unwrap();
        assert_eq!(s.tier.as_deref(), Some("Claude Pro"));
        assert_eq!(s.meters.len(), 2);
        let five = meter("claude", "5-hour limit");
        assert_eq!((five.left, five.limit, five.unit, five.resets_at), (Some(50.0), Some(100.0), "%", parse_iso("2026-11-05T00:20:00Z")));
        assert_eq!(meter("claude", "Weekly limit").left, Some(8.0));

        record_claude_rate_limit(&json!({
            "status": "allowed_warning", "rateLimitType": "seven_day", "utilization": 0.95, "resetsAt": 1794243600,
            "unifiedWindows": { "five_hour": { "utilization": 0.6, "resetsAt": 1794270000 }, "seven_day": { "utilization": 0.95, "resetsAt": 1794243600 } },
        }));
        let five = meter("claude", "5-hour limit");
        assert!((five.left.unwrap() - 40.0).abs() < 1e-9);
        assert_eq!(five.resets_at, Some(1_794_270_000_000));
        assert!((meter("claude", "Weekly limit").left.unwrap() - 5.0).abs() < 1e-9);
        // Without every window, just the one the answer counted against.
        record_claude_rate_limit(&json!({ "status": "allowed", "rateLimitType": "five_hour", "utilization": 0.7, "resetsAt": 1794270000 }));
        assert!((meter("claude", "5-hour limit").left.unwrap() - 30.0).abs() < 1e-9);
        assert_eq!(all().into_iter().find(|s| s.provider == "claude").unwrap().tier.as_deref(), Some("Claude Pro"), "the plan stays");
    }

    /// As Codex's app server answers on a free plan, and with a weekly window and credits.
    #[test]
    fn reads_codexs_limits() {
        let one = |id: &str| json!({ "limitId": id, "limitName": null, "primary": { "usedPercent": 1, "windowDurationMins": 43200, "resetsAt": 1796323775 }, "secondary": null, "credits": { "hasCredits": false, "unlimited": false, "balance": null }, "planType": "free" });
        record_codex(&json!({ "rateLimits": one("codex"), "rateLimitsByLimitId": { "codex": one("codex") } }));
        let s = all().into_iter().find(|s| s.provider == "codex").unwrap();
        assert_eq!(s.tier.as_deref(), Some("ChatGPT Free"));
        assert_eq!(s.meters.len(), 1);
        let m = meter("codex", "30-day limit");
        assert_eq!((m.left, m.resets_at), (Some(99.0), Some(1_796_323_775_000)));

        record_codex(&json!({ "rateLimits": {
            "limitId": "codex", "planType": "plus",
            "primary": { "usedPercent": 40, "windowDurationMins": 300, "resetsAt": 1794270000 },
            "secondary": { "usedPercent": 10, "windowDurationMins": 10080, "resetsAt": 1794700000 },
            "credits": { "hasCredits": true, "unlimited": false, "balance": "12.5" },
        } }));
        assert_eq!(meter("codex", "5-hour limit").left, Some(60.0));
        assert_eq!(meter("codex", "Weekly limit").left, Some(90.0));
        assert_eq!(meter("codex", "Credits").left, Some(12.5));
        assert_eq!(window_label(Some(90.0)), "90-minute limit");
    }

    /// Asks the real CLIs signed in on this PC, reading only: `cargo test -- --ignored live_limits --nocapture`.
    #[test]
    #[ignore]
    fn live_limits() {
        for (name, f) in [("claude", crate::claude::fetch_limits as fn() -> Result<(), String>), ("codex", crate::codex::fetch_limits), ("grok", crate::grok::fetch_limits)] {
            println!("{name}: {:?}", f());
        }
        for s in all() {
            println!("{} {:?}", s.provider, s.tier);
            for m in s.meters {
                println!("  {}: {:?} of {:?} {} resets {:?} {:?}", m.label, m.left, m.limit, m.unit, m.resets_at, m.note);
            }
        }
    }

    /// As Grok Build's `_x.ai/billing` answers for SuperGrok.
    #[test]
    fn reads_groks_billing() {
        record_grok_billing(&json!({ "config": {
            "creditUsagePercent": 29,
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": "2026-11-02T23:44:11.434+00:00", "end": "2026-11-09T23:44:11.434+00:00" },
            "prepaidBalance": { "val": 0 }, "isUnifiedBillingUser": true,
        }, "subscription_tier": "SuperGrok" }));
        let s = all().into_iter().find(|s| s.provider == "grok").unwrap();
        assert_eq!(s.tier.as_deref(), Some("SuperGrok"));
        let m = meter("grok", "Credit this week");
        assert_eq!((m.left, m.resets_at), (Some(71.0), parse_iso("2026-11-09T23:44:11.434Z")));
    }
}
