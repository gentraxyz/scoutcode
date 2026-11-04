use crate::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    /// Anything the provider attached to the call, which it wants back with it: Gemini's thought signature, for one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<Value>,
    pub function: FunctionCall,
}

fn function_type() -> String {
    "function".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// A user message is a string or a list of parts (text and images), as in the OpenAI format.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<Value>),
}

impl Content {
    /// What was written, without the images.
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(p) => p.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect(),
        }
    }

    /// The images sent as data URLs, as (media type, base64).
    pub fn images(&self) -> Vec<(String, String)> {
        let Content::Parts(p) = self else { return vec![] };
        p.iter()
            .filter_map(|x| x.pointer("/image_url/url")?.as_str()?.strip_prefix("data:")?.split_once(";base64,"))
            .map(|(mime, data)| (mime.to_string(), data.to_string()))
            .collect()
    }

    /// The same, with each image replaced by a note, for a model that can't see them.
    pub fn without_images(&self) -> Content {
        match self {
            Content::Text(s) => Content::Text(s.clone()),
            Content::Parts(p) => Content::Parts(p.iter().map(|x| if x.get("image_url").is_some() { json!({"type": "text", "text": "\n\n[An image was attached here, but this model can't see images.]"}) } else { x.clone() }).collect()),
        }
    }
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Content::Text(s)
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Content::Text(s.to_string())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: Content,
    },
    Assistant {
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Default, Clone, Debug)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
}

#[derive(Default, Clone, Debug)]
pub struct Meta {
    pub cost: Option<f64>,
    pub balance: Option<f64>,
}

#[derive(Default, Debug)]
pub struct StreamResult {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
    pub meta: Meta,
}

pub enum Delta<'a> {
    Text(&'a str),
    Reasoning(&'a str),
}

#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    /// 0 for network failures, which are retried like 5xx.
    pub status: u16,
    pub retry_after: Option<u64>,
    pub aborted: bool,
}

impl ApiError {
    pub fn new(message: impl Into<String>, status: u16) -> Self {
        ApiError { message: message.into(), status, retry_after: None, aborted: false }
    }
    pub fn aborted() -> Self {
        ApiError { message: "Interrupted".into(), status: 0, retry_after: None, aborted: true }
    }
    pub fn retryable(&self) -> bool {
        !self.aborted && [0, 408, 409, 429, 500, 502, 503, 504, 520, 522, 524].contains(&self.status)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub const USER_AGENT: &str = "scoutcode-desktop/0.1";

/// One connection pool for the whole app, so repeat requests skip the TLS handshake.
pub fn http() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(20)))
                .user_agent(USER_AGENT)
                .build(),
        )
    })
}

fn key(cfg: &Config) -> Result<&str, ApiError> {
    cfg.api_key.as_deref().ok_or_else(|| ApiError::new("Not signed in. Sign in or paste an API key first.", 401))
}

/// Whether requests with these settings go to Kilo. The provider can't say: auto mode's check goes to ScoutCode while a
/// model runs on Kilo.
fn is_kilo(cfg: &Config) -> bool {
    cfg.base_url == crate::config::kilo_url()
}

/// The key for a request that may go to Kilo. Its free models need no key, and it refuses a made-up one, so without a
/// key no header goes at all. The same goes for an additional provider with a free tier.
fn auth<B>(req: ureq::RequestBuilder<B>, cfg: &Config) -> Result<ureq::RequestBuilder<B>, ApiError> {
    // ScoutCode's free models without an account: no key to send, just the mark that says this is the desktop app.
    if cfg.is_guest() {
        return Ok(req.header("x-scoutcode-desktop-guest", "1"));
    }
    if cfg.api_key.is_none() && crate::config::ADDITIONAL.iter().any(|a| !a.needs_key && a.url == cfg.base_url) {
        return Ok(req);
    }
    if !is_kilo(cfg) {
        return Ok(req.header("Authorization", &format!("Bearer {}", key(cfg)?)));
    }
    let req = req.header("X-KiloCode-Version", env!("CARGO_PKG_VERSION"));
    Ok(match &cfg.api_key {
        Some(k) => req.header("Authorization", &format!("Bearer {k}")),
        None => req,
    })
}

fn header(res: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    res.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

fn error_from(mut res: ureq::http::Response<ureq::Body>) -> ApiError {
    let status = res.status().as_u16();
    let retry = header(&res, "retry-after").and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0);
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let mut message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|b| b.pointer("/error/message").or_else(|| b.get("message")).or_else(|| b.get("error")).or_else(|| b.get("detail")).and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| {
            // An HTML error page (a 404 from the site, a proxy's 502) says no more than its title.
            if text.trim_start().starts_with('<') {
                let title = text.split_once("<title>").and_then(|(_, t)| t.split_once("</title>")).map(|(t, _)| t.trim()).filter(|t| !t.is_empty());
                return title.map_or_else(|| format!("HTTP {status}"), String::from);
            }
            if text.is_empty() { format!("HTTP {status}") } else { text.chars().take(500).collect() }
        });
    if status == 401 {
        message += " (check your sign-in or API key)";
    }
    if status == 402 {
        message += " (out of money: ask an admin to top up your account)";
    }
    ApiError { message, status, retry_after: retry, aborted: false }
}

fn transport(e: ureq::Error) -> ApiError {
    ApiError::new(e.to_string(), 0)
}

/// A model the server offers, with whatever stats it reports (ScoutCode's own fields, or OpenRouter's shape).
#[derive(Clone, Debug, Default)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub context: Option<u64>,
    pub tools: Option<bool>,
    pub vision: Option<bool>,
    pub cost: Option<String>,
    pub providers: Vec<String>,
    pub routes_to: Vec<String>,
    pub available: Option<bool>,
    /// The edit tool it was trained on, as an admin set it on the server; str_replace when the server doesn't say.
    pub edit_tool: crate::tools::EditTool,
    /// The id OpenRouter knows it by, when OpenRouter serves it too. Set for OpenRouter's own entries as well.
    pub openrouter_id: Option<String>,
    /// The id NVIDIA NIM knows it by, when NIM serves it too. Set for NIM's own entries as well.
    pub nvidia_id: Option<String>,
    /// The id OpenCode knows it by. OpenCode's entries are their own too, under "opencode/…", for the same reason.
    pub opencode_id: Option<String>,
    /// The id Kilo knows it by, when Kilo serves it.
    pub kilo_id: Option<String>,
    /// Costs nothing on Kilo.
    pub kilo_free: bool,
    /// How expensive it is on Kilo.
    pub kilo_tier: Option<u8>,
    /// The id Copilot knows it by. Copilot's entries are their own, under "copilot/…": it decides what it serves per account.
    pub copilot_id: Option<String>,
    /// The id Devin knows it by. Devin's entries are their own too, under "devin/…", for the same reason.
    pub devin_id: Option<String>,
    /// The id Kiro knows it by. Kiro's entries are their own too, under "kiro/…", for the same reason.
    pub kiro_id: Option<String>,
    /// The id Codex knows it by. Its entries are their own too, under "codex/…": its models depend on the ChatGPT plan.
    pub codex_id: Option<String>,
    /// The id Claude Code knows it by. Its entries are their own too, under "claude/…": its models depend on the Claude plan.
    pub claude_id: Option<String>,
    /// The id Grok Build knows it by. Its entries are their own too, under "grok/…", for the same reason.
    pub grok_id: Option<String>,
    /// The id Nous Portal knows it by. Its entries are their own too, under "hermes/…": its prices and free models are its own.
    pub hermes_id: Option<String>,
    /// The additional provider that serves it, and the id it knows it by. These entries are their own too, under "ovh/…"
    /// and the like: the same name there can be a different build at a different price.
    pub additional: Option<(&'static str, String)>,
    /// ScoutCode serves it, from our own model list. False for models that are only on OpenRouter, NIM or Kilo.
    pub hosted: bool,
    /// Costs nothing on the route that reports it (no per-token price, or OpenRouter's ":free" variants).
    pub free: bool,
    /// How expensive it is, from 1 ($) to 4 ($$$$), on ScoutCode or for OpenRouter's own entries. None when it's free or varies.
    pub tier: Option<u8>,
    /// The same on OpenRouter, for a model both serve.
    pub openrouter_tier: Option<u8>,
    /// Costs nothing on OpenRouter, for a model both serve. Kept apart from `free`, which is ScoutCode's own price.
    pub openrouter_free: bool,
    /// The Intelligence Index an admin set on the server, from 1 (light) to 3 (frontier). None: the app guesses from the name.
    pub intelligence: Option<u8>,
}

/// The price tier for USD per million tokens, on the server's scale: what 10k tokens in and 1k out cost. None when free or unknown.
fn price_tier(input: f64, output: f64) -> Option<u8> {
    if input < 0.0 || output < 0.0 || input + output == 0.0 {
        return None;
    }
    let usd = (input * 10_000.0 + output * 1_000.0) / 1e6;
    Some(if usd < 0.01 { 1 } else if usd < 0.05 { 2 } else if usd < 0.2 { 3 } else { 4 })
}

impl ModelInfo {
    fn from_json(m: &Value) -> Option<Self> {
        let s = |k: &str| m.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let list = |k: &str| m.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
        // OpenRouter prices per token, as strings.
        let per_m = |k: &str| m.pointer(&format!("/pricing/{k}")).and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64())).map(|p| p * 1e6);
        let cost = s("cost").or_else(|| match (per_m("prompt"), per_m("completion")) {
            (Some(i), Some(o)) if i + o == 0.0 => Some("Free".into()),
            (Some(i), Some(o)) => Some(format!("${} in · ${} out / M tokens", money(i), money(o))),
            _ => None,
        });
        let tier = m.get("price_tier").and_then(Value::as_u64).map(|t| t as u8).or_else(|| price_tier(per_m("prompt")?, per_m("completion")?));
        let inputs = list_at(m, "/architecture/input_modalities");
        let params: Vec<String> = list("supported_parameters");
        let free = s("id").is_some_and(|id| id.ends_with(":free")) || cost.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("free"));
        Some(ModelInfo {
            id: s("id")?,
            name: s("name"),
            description: s("description"),
            context: m.get("context_length").and_then(Value::as_u64),
            tools: m.get("tools").and_then(Value::as_bool).or_else(|| (!params.is_empty()).then(|| params.iter().any(|p| p == "tools"))),
            vision: m.get("vision").and_then(Value::as_bool).or_else(|| (!inputs.is_empty()).then(|| inputs.iter().any(|i| i == "image"))),
            cost,
            providers: list("providers"),
            routes_to: list("routes_to"),
            available: m.get("available").and_then(Value::as_bool),
            edit_tool: crate::tools::EditTool::parse(m.get("edit_tool").and_then(Value::as_str).unwrap_or("")),
            openrouter_id: None,
            nvidia_id: None,
            opencode_id: None,
            kilo_id: None,
            kilo_free: false,
            kilo_tier: None,
            copilot_id: None,
            devin_id: None,
            kiro_id: None,
            codex_id: None,
            claude_id: None,
            grok_id: None,
            hermes_id: None,
            additional: None,
            hosted: false,
            free,
            tier,
            openrouter_tier: None,
            openrouter_free: false,
            intelligence: m.get("intelligence").and_then(Value::as_u64).filter(|n| (1..=3).contains(n)).map(|n| n as u8),
        })
    }

    /// The name to show: the server's, without a parenthetical, or the id without its vendor prefix.
    pub fn label(&self) -> String {
        match &self.name {
            Some(n) => n.split(" (").next().unwrap_or(n).trim().to_string(),
            None => short_id(&self.id),
        }
    }

    /// The id to send for this model to `provider`.
    pub fn request_id(&self, provider: &str) -> &str {
        if let ("opencode", Some(id)) = (provider, &self.opencode_id) {
            return id;
        }
        if let ("copilot", Some(id)) = (provider, &self.copilot_id) {
            return id;
        }
        if let ("devin", Some(id)) = (provider, &self.devin_id) {
            return id;
        }
        if let ("kiro", Some(id)) = (provider, &self.kiro_id) {
            return id;
        }
        if let ("codex", Some(id)) = (provider, &self.codex_id) {
            return id;
        }
        if let ("claude", Some(id)) = (provider, &self.claude_id) {
            return id;
        }
        if let ("grok", Some(id)) = (provider, &self.grok_id) {
            return id;
        }
        if let ("hermes", Some(id)) = (provider, &self.hermes_id) {
            return id;
        }
        if let Some((_, id)) = self.additional.as_ref().filter(|(p, _)| *p == provider) {
            return id;
        }
        match (provider, &self.openrouter_id, &self.nvidia_id, &self.kilo_id) {
            ("openrouter", Some(id), _, _) | ("nvidia", _, Some(id), _) | ("kilo", _, _, Some(id)) => id,
            _ => &self.id,
        }
    }
}

/// The name models are matched on across providers: no vendor ("OpenAI: "), no parenthetical, letters and digits only.
fn match_key(m: &ModelInfo) -> String {
    let label = m.label();
    let name = label.split_once(": ").map(|(_, n)| n).unwrap_or(&label);
    name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// One list from every provider: a model several serve shows once, on the first one's entry with the others' ids
/// attached; models only one of them serves follow ours.
pub fn merge_models(scout: Vec<ModelInfo>, openrouter: Vec<ModelInfo>, nvidia: Vec<ModelInfo>, kilo: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let mut out = scout;
    for o in openrouter {
        let key = match_key(&o);
        let same = out.iter().position(|m| m.hosted && m.openrouter_id.is_none() && (m.id == o.id || (!key.is_empty() && match_key(m) == key)));
        match same {
            Some(i) => {
                // Free on one says nothing about the other: each row shows its own provider's price.
                out[i].openrouter_free = o.free;
                out[i].openrouter_id = o.openrouter_id;
                out[i].openrouter_tier = o.tier;
            }
            None if out.iter().all(|m| m.id != o.id) => out.push(o),
            None => {}
        }
    }
    for n in nvidia {
        let key = match_key(&n);
        let same = out
            .iter()
            .position(|m| m.nvidia_id.is_none() && m.id == n.id)
            .or_else(|| out.iter().position(|m| m.nvidia_id.is_none() && !key.is_empty() && match_key(m) == key));
        match same {
            Some(i) => out[i].nvidia_id = n.nvidia_id,
            None if out.iter().all(|m| m.id != n.id) => out.push(n),
            None => {}
        }
    }
    // By id only: Kilo's "nvidia/…:free" and the like are its own routes, whatever else goes by that name.
    for k in kilo {
        match out.iter_mut().find(|m| m.id == k.id) {
            Some(m) => {
                m.kilo_id = k.kilo_id;
                m.kilo_free = k.kilo_free;
                m.kilo_tier = k.kilo_tier;
            }
            None => out.push(k),
        }
    }
    out
}

// ---------- the Intelligence Index admins set ----------

/// Letters and digits only, lowercased: how admins' keys and model names are compared.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn intelligence_file() -> std::path::PathBuf {
    crate::config::home().join("model-intelligence.json")
}

/// The levels admins set on the server, by model id or name, kept on disk for starts without a connection.
fn intelligence_levels() -> &'static Mutex<HashMap<String, u8>> {
    static LEVELS: OnceLock<Mutex<HashMap<String, u8>>> = OnceLock::new();
    LEVELS.get_or_init(|| {
        let saved: HashMap<String, u8> = std::fs::read_to_string(intelligence_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Mutex::new(saved)
    })
}

fn set_intelligence(levels: HashMap<String, u8>) {
    let levels: HashMap<String, u8> = levels.into_iter().map(|(k, v)| (squash(&k), v)).filter(|(k, _)| !k.is_empty()).collect();
    let mut cur = intelligence_levels().lock().unwrap();
    if *cur != levels {
        let _ = std::fs::write(intelligence_file(), serde_json::to_string(&levels).unwrap_or_default());
        *cur = levels;
    }
}

/// Gives each model the level an admin set for it, matched by its id on any provider (with or without the vendor
/// prefix) or by its name, so one setting covers the model wherever it runs.
pub fn apply_intelligence(models: &mut [ModelInfo]) {
    let levels = intelligence_levels().lock().unwrap();
    if levels.is_empty() {
        return;
    }
    for m in models.iter_mut() {
        let mut ids: Vec<&str> = vec![&m.id];
        ids.extend([&m.openrouter_id, &m.nvidia_id, &m.opencode_id, &m.kilo_id, &m.copilot_id, &m.devin_id, &m.kiro_id, &m.codex_id, &m.claude_id, &m.grok_id, &m.hermes_id].into_iter().flatten().map(String::as_str));
        if let Some((_, id)) = &m.additional {
            ids.push(id);
        }
        let mut keys: Vec<String> = ids.iter().flat_map(|id| [squash(id), squash(id.rsplit('/').next().unwrap_or(id))]).collect();
        keys.push(match_key(m));
        keys.push(squash(&m.label()));
        if let Some(level) = keys.iter().find_map(|k| levels.get(k)) {
            m.intelligence = Some(*level);
        }
    }
}

// ---------- the saved lists ----------

impl ModelInfo {
    fn to_cache(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "description": self.description, "context": self.context, "tools": self.tools, "vision": self.vision,
            "cost": self.cost, "providers": self.providers, "routesTo": self.routes_to, "available": self.available,
            "editTool": if self.edit_tool == crate::tools::EditTool::ApplyPatch { "apply_patch" } else { "str_replace" },
            "openrouterId": self.openrouter_id, "nvidiaId": self.nvidia_id, "opencodeId": self.opencode_id, "kiloId": self.kilo_id,
            "kiloFree": self.kilo_free, "kiloTier": self.kilo_tier, "copilotId": self.copilot_id, "devinId": self.devin_id, "kiroId": self.kiro_id,
            "codexId": self.codex_id, "claudeId": self.claude_id, "grokId": self.grok_id, "hermesId": self.hermes_id, "additional": self.additional, "hosted": self.hosted, "free": self.free,
            "tier": self.tier, "openrouterTier": self.openrouter_tier, "openrouterFree": self.openrouter_free, "intelligence": self.intelligence,
        })
    }

    fn from_cache(m: &Value) -> Option<Self> {
        let s = |k: &str| m.get(k).and_then(Value::as_str).map(String::from);
        let b = |k: &str| m.get(k).and_then(Value::as_bool);
        let n = |k: &str| m.get(k).and_then(Value::as_u64);
        let list = |k: &str| m.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
        // A provider this version doesn't know drops its models.
        let additional = match m.get("additional") {
            Some(Value::Array(a)) => {
                let p = crate::config::additional_provider(a.first()?.as_str()?)?;
                Some((p.id, a.get(1)?.as_str()?.to_string()))
            }
            _ => None,
        };
        Some(ModelInfo {
            id: s("id")?,
            name: s("name"),
            description: s("description"),
            context: n("context"),
            tools: b("tools"),
            vision: b("vision"),
            cost: s("cost"),
            providers: list("providers"),
            routes_to: list("routesTo"),
            available: b("available"),
            edit_tool: crate::tools::EditTool::parse(m.get("editTool").and_then(Value::as_str).unwrap_or("")),
            openrouter_id: s("openrouterId"),
            nvidia_id: s("nvidiaId"),
            opencode_id: s("opencodeId"),
            kilo_id: s("kiloId"),
            kilo_free: b("kiloFree").unwrap_or(false),
            kilo_tier: n("kiloTier").map(|t| t as u8),
            copilot_id: s("copilotId"),
            devin_id: s("devinId"),
            kiro_id: s("kiroId"),
            codex_id: s("codexId"),
            claude_id: s("claudeId"),
            grok_id: s("grokId"),
            hermes_id: s("hermesId"),
            additional,
            hosted: b("hosted").unwrap_or(false),
            free: b("free").unwrap_or(false),
            tier: n("tier").map(|t| t as u8),
            openrouter_tier: n("openrouterTier").map(|t| t as u8),
            openrouter_free: b("openrouterFree").unwrap_or(false),
            intelligence: n("intelligence").map(|t| t as u8),
        })
    }
}

fn model_cache_file() -> std::path::PathBuf {
    crate::config::home().join("model-lists.json")
}

/// Each provider's last model list, by the id `Config::provider` gives it, saved so the next start can show them at once.
pub fn load_model_cache() -> HashMap<String, Vec<ModelInfo>> {
    let Some(Value::Object(lists)) = std::fs::read_to_string(model_cache_file()).ok().and_then(|t| serde_json::from_str(&t).ok()) else { return HashMap::new() };
    lists
        .iter()
        .filter_map(|(source, list)| Some((source.clone(), list.as_array()?.iter().filter_map(ModelInfo::from_cache).collect())))
        .collect()
}

pub fn save_model_cache(lists: &HashMap<String, Vec<ModelInfo>>) {
    let body: serde_json::Map<String, Value> = lists.iter().map(|(source, list)| (source.clone(), Value::Array(list.iter().map(ModelInfo::to_cache).collect()))).collect();
    let path = model_cache_file();
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, Value::Object(body).to_string()).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}

/// Whether NIM's list entry can chat. The list also has embeddings, safety filters, document parsers and so on, with no
/// field to tell them apart.
fn nvidia_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 16] = ["embed", "rerank", "safety", "guard", "parse", "reward", "clip", "detector", "translate", "calibration", "diffusion", "deplot", "kosmos", "fuyu", "neva", "vila"];
    !NOT_CHAT.iter().any(|w| id.contains(w))
}

/// NIM's catalog. It's public, so reading it says nothing about the key.
pub fn list_nvidia(cfg: &Config) -> Result<Vec<ModelInfo>, ApiError> {
    let nv = cfg.for_provider("nvidia");
    let res = http()
        .get(&format!("{}/models", nv.base_url))
        .header("Authorization", &format!("Bearer {}", key(&nv)?))
        .call()
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    Ok(body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(ModelInfo::from_json).filter(|m| nvidia_chat_model(&m.id)).map(|m| ModelInfo { nvidia_id: Some(m.id.clone()), ..m }).collect())
        .unwrap_or_default())
}

/// Kilo's catalog, which is public. Models without tools can't run Scout, and without a key only the free ones work.
pub fn list_kilo(cfg: &Config) -> Result<Vec<ModelInfo>, ApiError> {
    let k = cfg.for_provider("kilo");
    let res = auth(http().get(&format!("{}/models", k.base_url)), &k)?.call().map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    let is_free = |m: &Value| m.get("isFree").and_then(Value::as_bool) == Some(true);
    Ok(body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|m| k.api_key.is_some() || is_free(m))
                .filter_map(|m| {
                    let free = is_free(m);
                    let info = ModelInfo::from_json(m).filter(|i| i.tools == Some(true))?;
                    Some(ModelInfo { kilo_id: Some(info.id.clone()), kilo_free: free, kilo_tier: info.tier, free, ..info })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Whether an additional provider's list entry can run Scout. The lists also have embeddings, speech, image and safety
/// models, and a few chat models that refuse tools, with no field to tell them apart.
fn additional_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 19] = ["embed", "bge-", "guard", "whisper", "tts", "diffusion", "-vl-", "image", "audio", "live", "veo", "imagen", "aqa", "gemma", "robotics", "computer-use", "learnlm", "nano-banana", "deep-research"];
    let id = id.to_lowercase();
    !NOT_CHAT.iter().any(|w| id.contains(w))
}

/// The entries of an additional provider's model list that can run Scout. With no key on a free tier, they all cost nothing.
fn additional_models(a: &crate::config::AdditionalProvider, keyed: bool, body: &Value) -> Vec<ModelInfo> {
    let free = !a.needs_key && !keyed;
    body.get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(ModelInfo::from_json)
                .filter(|m| additional_chat_model(&m.id))
                .map(|m| {
                    // Google's ids start "models/", which chat doesn't need.
                    let id = m.id.strip_prefix("models/").unwrap_or(&m.id).to_string();
                    ModelInfo {
                        id: format!("{}/{id}", a.id),
                        additional: Some((a.id, id)),
                        free: free || m.free,
                        tier: if free { None } else { m.tier },
                        cost: if free { Some("Free".into()) } else { m.cost },
                        ..m
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The models of a linked additional provider.
pub fn list_additional(cfg: &Config, a: &crate::config::AdditionalProvider) -> Result<Vec<ModelInfo>, ApiError> {
    let c = cfg.for_provider(a.id);
    let res = auth(http().get(&format!("{}/models", c.base_url)), &c)?.call().map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    Ok(additional_models(a, c.api_key.is_some(), &body))
}

/// The entries of Nous Portal's catalog that can run Scout, with the free ones first.
fn hermes_models(body: &Value) -> Vec<ModelInfo> {
    let mut models: Vec<ModelInfo> = body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(ModelInfo::from_json).filter(|m| m.tools == Some(true)).map(|m| ModelInfo { id: format!("hermes/{}", m.id), hermes_id: Some(m.id.clone()), ..m }).collect())
        .unwrap_or_default();
    models.sort_by_key(|m| !m.free);
    models
}

/// Nous Portal's catalog. It's public, so reading it says nothing about the sign-in or key.
pub fn list_hermes() -> Result<Vec<ModelInfo>, ApiError> {
    let res = http().get(&format!("{}/models", crate::config::HERMES_URL)).call().map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    Ok(hermes_models(&body))
}

/// Whether Kilo answers and, with a key, takes it, which costs one request. Without a key that request is left out, to
/// save the hourly limit. Says how it went either way.
pub fn check_kilo(cfg: &Config) -> Result<String, String> {
    let k = cfg.for_provider("kilo");
    let timeout = Some(Duration::from_secs(10));
    let unreachable = |e: String| format!("Could not reach Kilo: {e}");
    let free = list_kilo(&Config { kilo_key: None, ..cfg.clone() }).map_err(|e| unreachable(e.message))?.len();
    if k.api_key.is_some() {
        let res = auth(http().post(&format!("{}/chat/completions", k.base_url)), &k)
            .map_err(|e| e.message)?
            .config()
            .timeout_global(timeout)
            .build()
            .send_json(json!({ "model": crate::config::KILO_DEFAULT_MODEL, "messages": [{ "role": "user", "content": "hi" }], "max_tokens": 1 }));
        if res.is_ok_and(|r| r.status().as_u16() == 401) {
            return Err("Kilo rejected that key.".into());
        }
    }
    Ok(format!("Kilo is reachable, with {free} free models."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_models_come_back_the_same() {
        let m = ModelInfo {
            id: "ovh/qwen".into(),
            name: Some("Qwen".into()),
            context: Some(128_000),
            vision: Some(false),
            edit_tool: crate::tools::EditTool::ApplyPatch,
            openrouter_id: Some("qwen/qwen".into()),
            kilo_free: true,
            kilo_tier: Some(2),
            additional: Some(("ovh", "Qwen-32B".into())),
            hosted: true,
            intelligence: Some(3),
            ..Default::default()
        };
        let back = ModelInfo::from_cache(&m.to_cache()).unwrap();
        assert_eq!(format!("{back:?}"), format!("{m:?}"));
        // A provider this version doesn't know is dropped with its model.
        let mut gone = m.to_cache();
        gone["additional"] = json!(["nowhere", "x"]);
        assert!(ModelInfo::from_cache(&gone).is_none());
    }

    #[test]
    fn free_on_one_provider_stays_on_that_provider() {
        let model = |id: &str, free: bool| ModelInfo { id: id.into(), name: Some("Qwen 3".into()), free, ..Default::default() };
        let hosted = |free| ModelInfo { hosted: true, ..model("qwen3", free) };
        let or = |free| ModelInfo { openrouter_id: Some("qwen/qwen3".into()), ..model("qwen/qwen3", free) };
        let a = merge_models(vec![hosted(true)], vec![or(false)], vec![], vec![]);
        assert!(a[0].free && !a[0].openrouter_free, "free on ScoutCode, paid on OpenRouter");
        let b = merge_models(vec![hosted(false)], vec![or(true)], vec![], vec![]);
        assert!(!b[0].free && b[0].openrouter_free, "paid on ScoutCode, free on OpenRouter");
    }

    #[test]
    fn kilo_models_merge_by_id_only() {
        let model = |id: &str, name: &str| ModelInfo { id: id.into(), name: Some(name.into()), ..Default::default() };
        let nim = ModelInfo { nvidia_id: Some("nvidia/nemotron".into()), ..model("nvidia/nemotron", "Nemotron") };
        let or = ModelInfo { openrouter_id: Some("openrouter/free".into()), ..model("openrouter/free", "Free") };
        let kilo = |id: &str, name: &str| ModelInfo { kilo_id: Some(id.into()), kilo_free: true, ..model(id, name) };
        let out = merge_models(vec![], vec![or], vec![nim], vec![kilo("nvidia/nemotron:free", "Nemotron"), kilo("openrouter/free", "Free")]);
        assert_eq!(out.len(), 3);
        assert!(out.iter().find(|m| m.id == "nvidia/nemotron").unwrap().kilo_id.is_none(), "the same name isn't the same route");
        assert!(out.iter().find(|m| m.id == "openrouter/free").unwrap().kilo_free, "the same id is one entry");
    }

    #[test]
    fn hermes_lists_free_models_first_and_only_those_with_tools() {
        let entry = |id: &str, price: &str, params: &[&str]| json!({ "id": id, "name": id, "pricing": { "prompt": price, "completion": price }, "supported_parameters": params });
        let body = json!({ "data": [entry("paid/big", "0.000001", &["tools"]), entry("paid/no-tools", "0.000001", &["temperature"]), entry("free/priced-zero", "0", &["tools"]), entry("step/flash:free", "0", &["tools", "reasoning"]), entry("free/no-tools:free", "0", &[])] });
        let models = hermes_models(&body);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["hermes/free/priced-zero", "hermes/step/flash:free", "hermes/paid/big"]);
        assert!(models[0].free && models[1].free && !models[2].free);
        assert_eq!(models[1].request_id("hermes"), "step/flash:free");
    }

    #[test]
    #[ignore = "reads Nous Portal's public catalog"]
    fn live_hermes_catalog() {
        let models = list_hermes().unwrap();
        let free = models.iter().filter(|m| m.free).count();
        println!("{} models, {free} free, first: {}", models.len(), models[0].id);
        assert!(free > 0 && models[0].free);
    }

    #[test]
    fn additional_lists_keep_only_models_that_can_run_scout() {
        let ovh = crate::config::additional_provider("ovh").unwrap();
        let price = json!({ "prompt": "0.00000047", "completion": "0.00000319" });
        let listing = |ids: &[&str]| json!({ "data": ids.iter().map(|id| json!({ "id": id, "pricing": price })).collect::<Vec<_>>() });
        let body = listing(&["Qwen3.8-27B", "bge-m3", "Qwen3Guard-Gen-8B", "whisper-large-v3", "nvr-tts-en-us", "stable-diffusion-xl-base-v10", "Qwen2.5-VL-72B-Instruct", "Qwen3-Embedding-8B"]);
        let free = additional_models(ovh, false, &body);
        assert_eq!(free.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["ovh/Qwen3.8-27B"]);
        assert_eq!(free[0].request_id("ovh"), "Qwen3.8-27B");
        assert_eq!(free[0].label(), "Qwen3.8-27B");
        assert!(free[0].free && free[0].tier.is_none(), "no key is the free tier");
        let paid = additional_models(ovh, true, &body);
        assert!(!paid[0].free && paid[0].tier.is_some(), "a key is billed");

        let google = crate::config::additional_provider("google").unwrap();
        let body = listing(&["models/gemini-2.5-flash", "models/gemini-2.5-flash-preview-tts", "models/gemini-embedding-001", "models/gemini-2.5-flash-image", "models/gemma-3-27b-it", "models/gemini-2.5-flash-live"]);
        let listed = additional_models(google, true, &body);
        assert_eq!(listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["google/gemini-2.5-flash"]);
        assert_eq!(listed[0].request_id("google"), "gemini-2.5-flash");
        assert_eq!(listed[0].request_id("scoutcode"), "google/gemini-2.5-flash");
    }

    /// The ScoutCode key must never go to another provider, and OVH's free tier refuses nothing for lacking one.
    #[test]
    fn additional_providers_only_get_their_own_keys() {
        let mut cfg = crate::config::load_config();
        cfg.api_key = Some("scout-key".into());
        cfg.additional = vec![crate::config::LinkedProvider { id: "ovh".into(), key: None }, crate::config::LinkedProvider { id: "google".into(), key: Some("g-key".into()) }];
        let ovh = cfg.for_provider("ovh");
        assert_eq!(ovh.api_key, None);
        assert!(ovh.base_url.contains("ovh.net"));
        let google = cfg.for_provider("google");
        assert_eq!(google.api_key.as_deref(), Some("g-key"));
        assert!(auth(http().get("http://localhost"), &ovh).is_ok(), "OVH works with no key");
        cfg.additional.clear();
        assert_eq!(cfg.for_provider("google").api_key, None);
        assert!(auth(http().get("http://localhost"), &cfg.for_provider("google")).is_err(), "Google doesn't");
    }

    /// Going without an account is only for ScoutCode: the mark must never reach another provider, and a key wins over it.
    #[test]
    fn guest_mode_is_only_for_scoutcode() {
        let mut cfg = crate::config::load_config();
        cfg.api_key = None;
        cfg.guest = true;
        assert!(cfg.is_guest());
        assert!(cfg.for_provider("scoutcode").is_guest());
        for provider in ["openrouter", "nvidia", "kilo", "hermes", "ovh", "google", "copilot"] {
            assert!(!cfg.for_provider(provider).is_guest(), "{provider}");
        }
        assert!(!cfg.for_openrouter_key("sk-or").is_guest());
        assert!(auth(http().get("http://localhost"), &cfg).is_ok(), "needs no key");
        cfg.api_key = Some("scout-key".into());
        assert!(!cfg.is_guest());
    }

    #[test]
    #[ignore = "talks to OVH, and uses up a request of its free tier"]
    fn ovh_free_tier_lists_models_and_calls_tools_with_no_key() {
        let ovh = crate::config::additional_provider("ovh").unwrap();
        let mut cfg = crate::config::load_config();
        cfg.additional = vec![crate::config::LinkedProvider { id: "ovh".into(), key: None }];
        let models = list_additional(&cfg, ovh).unwrap();
        println!("{:?}", models.iter().map(|m| m.request_id("ovh")).collect::<Vec<_>>());
        assert!(models.iter().any(|m| m.request_id("ovh") == ovh.default_model), "the default model is gone");
        assert!(models.iter().all(|m| m.free));
        let tools = [json!({ "type": "function", "function": { "name": "ping", "description": "Ping", "parameters": { "type": "object", "properties": {} } } })];
        let messages = [Message::User { content: Content::Text("Call the ping tool now.".into()) }];
        let req = ChatRequest { model: ovh.default_model, messages: &messages, tools: Some(&tools), tool_choice_none: false, temperature: None, effort: None };
        let r = stream_chat(&cfg.for_provider("ovh"), req, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(r.tool_calls.first().map(|c| c.function.name.as_str()), Some("ping"), "{r:?}");
    }

    /// Gemini refuses a tool result whose call came back without the signature it attached.
    #[test]
    fn tool_calls_keep_what_the_provider_attached() {
        let signed = json!({ "id": "1", "type": "function", "function": { "name": "x", "arguments": "{}" }, "extra_content": { "google": { "thought_signature": "sig" } } });
        let call: ToolCall = serde_json::from_value(signed.clone()).unwrap();
        assert_eq!(serde_json::to_value(&call).unwrap(), signed);
        let plain = json!({ "id": "1", "type": "function", "function": { "name": "x", "arguments": "{}" } });
        let call: ToolCall = serde_json::from_value(plain.clone()).unwrap();
        assert_eq!(serde_json::to_value(&call).unwrap(), plain, "nothing extra for everyone else");
    }
}

/// OpenRouter's catalog, read with the user's own key.
pub fn list_openrouter(cfg: &Config) -> Result<Vec<ModelInfo>, ApiError> {
    let or = cfg.for_provider("openrouter");
    let res = http()
        .get(&format!("{}/models", or.base_url))
        .header("Authorization", &format!("Bearer {}", key(&or)?))
        .call()
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    Ok(body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(ModelInfo::from_json).map(|m| ModelInfo { openrouter_id: Some(m.id.clone()), ..m }).collect())
        .unwrap_or_default())
}

/// "openai/gpt-4o:free" → "gpt-4o"; "auto" → "Auto".
pub fn short_id(id: &str) -> String {
    let s = id.rsplit('/').next().unwrap_or(id);
    let s = s.split(':').next().unwrap_or(s);
    if s == "auto" { "Auto".into() } else { s.to_string() }
}

fn list_at(m: &Value, ptr: &str) -> Vec<String> {
    m.pointer(ptr).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// Up to four decimals, without trailing zeros.
pub fn money(usd: f64) -> String {
    let s = format!("{usd:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// How much of a monthly limit is left.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Plan {
    pub limit: f64,
    pub left: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BankedMoney {
    pub amount: f64,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BankedReset {
    pub expires_at: String,
}

/// An amount of money per month saved for a user.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Monthly {
    pub monthly: Option<f64>,
}

/// A user an admin can change, from the server's list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManagedUser {
    pub username: String,
    /// "default", "custom" or "unlimited".
    pub mode: String,
    /// What applies now; null when unlimited.
    pub limits: Option<Monthly>,
    pub cli_monthly: f64,
    /// The amount set for this user; null means what their plan gets.
    pub cli_limit: Option<f64>,
    pub balance: f64,
    pub banked: f64,
    pub spent_this_month: f64,
    pub harness_spent_this_month: f64,
}

/// What Settings > Usage shows: the account's limits this month, and for admins everyone they can change.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    pub username: String,
    pub admin: bool,
    pub resets_at: String,
    /// None when the account is unlimited.
    pub money: Option<Plan>,
    /// None when the account is unlimited.
    pub cli: Option<Plan>,
    pub balance: f64,
    pub banked_money: Vec<BankedMoney>,
    pub banked_resets: Vec<BankedReset>,
    #[serde(default)]
    pub users: Vec<ManagedUser>,
}

fn usage_url(cfg: &Config) -> String {
    format!("{}/api/account/usage", cfg.base_url.trim_end_matches("/v1"))
}

fn usage_reply(res: ureq::http::Response<ureq::Body>) -> Result<UsageInfo, ApiError> {
    match res.status().as_u16() {
        200 => res.into_body().read_json().map_err(transport),
        404 => Err(ApiError::new("This server doesn't have monthly limits yet.", 404)),
        _ => Err(error_from(res)),
    }
}

/// The Scout Enterprise team the signed-in account is in, if any. A server from before teams has none.
pub fn team_policy(cfg: &Config) -> Result<Option<crate::config::Team>, ApiError> {
    let url = format!("{}/api/account/team", cfg.base_url.trim_end_matches("/v1"));
    let res = http().get(&url).header("Authorization", &format!("Bearer {}", key(cfg)?)).call().map_err(transport)?;
    match res.status().as_u16() {
        200 => {
            let body: Value = res.into_body().read_json().map_err(transport)?;
            Ok(body.get("policy").filter(|p| !p.is_null()).and_then(|p| serde_json::from_value(p.clone()).ok()))
        }
        404 => Ok(None),
        _ => Err(error_from(res)),
    }
}

/// The signed-in account's limits this month.
pub fn usage_info(cfg: &Config) -> Result<UsageInfo, ApiError> {
    let res = http().get(&usage_url(cfg)).header("Authorization", &format!("Bearer {}", key(cfg)?)).call().map_err(transport)?;
    usage_reply(res)
}

/// Uses a banked reset or, for admins, changes limits and money. Returns the limits afterwards.
pub fn usage_action(cfg: &Config, body: &Value) -> Result<UsageInfo, ApiError> {
    let res = http()
        .post(&usage_url(cfg))
        .header("Authorization", &format!("Bearer {}", key(cfg)?))
        .header("x-scoutcode", "1")
        .send_json(body)
        .map_err(transport)?;
    usage_reply(res)
}

/// The models the key can use, and the username it belongs to when the server says.
pub fn list_models(cfg: &Config) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
    let res = auth(http().get(&format!("{}/models", cfg.base_url)), cfg)?.call().map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let user = header(&res, "x-scoutcode-user").filter(|u| !u.is_empty());
    let body: Value = res.into_body().read_json().map_err(transport)?;
    if let Some(Value::Object(levels)) = body.get("intelligence") {
        set_intelligence(levels.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64().filter(|n| (1..=3).contains(n))? as u8))).collect());
    }
    let models = body
        .get("data")
        .and_then(Value::as_array)
        // The server's "auto" router is not offered in the picker.
        .map(|a| a.iter().filter_map(ModelInfo::from_json).filter(|m| m.id != "auto").map(|m| ModelInfo { hosted: true, ..m }).collect())
        .unwrap_or_default();
    Ok((models, user))
}

fn usage_from(u: &Value) -> Usage {
    let n = |p: &str| u.pointer(p).and_then(Value::as_f64).unwrap_or(0.0) as u64;
    Usage {
        prompt: n("/prompt_tokens"),
        completion: n("/completion_tokens"),
        // DeepSeek calls it prompt_cache_hit_tokens.
        cached: n("/prompt_tokens_details/cached_tokens").max(n("/prompt_cache_hit_tokens")),
    }
}

/// Auto mode's verdict on a shell command, from the server's rules and Jev: why it's held, or None to run it.
pub fn review_command(cfg: &Config, command: &str, cwd: &str, shell: &str) -> Result<Option<String>, ApiError> {
    let res = http()
        .post(&format!("{}/review/command", cfg.base_url))
        .header("Authorization", &format!("Bearer {}", key(cfg)?))
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .send_json(json!({ "command": command, "cwd": cwd, "shell": shell }))
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    if body.get("held").and_then(Value::as_bool) == Some(false) {
        return Ok(None);
    }
    Ok(Some(body.get("reason").and_then(Value::as_str).unwrap_or("Flagged as dangerous.").to_string()))
}

/// Speech to text (Deepgram, through the server): the transcript of a WAV recording.
pub fn transcribe(cfg: &Config, wav: &[u8]) -> Result<String, ApiError> {
    let res = http()
        .post(&format!("{}/audio/transcriptions", cfg.base_url))
        .header("Authorization", &format!("Bearer {}", key(cfg)?))
        .header("Content-Type", "audio/wav")
        .config()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .send(wav)
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    Ok(body.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string())
}

/// Web search (Exa, through the server): the results' titles, URLs, dates and best-matching sentences, without the
/// pages themselves, which web_fetch reads on this computer. `body` is the request as /search takes it.
pub fn search(cfg: &Config, body: &Value) -> Result<Value, ApiError> {
    let req = http().post(&format!("{}/search", cfg.base_url)).config().timeout_global(Some(Duration::from_secs(60))).build();
    let res = auth(req, cfg)?.send_json(body).map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    res.into_body().read_json().map_err(transport)
}

/// Experimental image generation (an OpenRouter image model, through the server): the answer to /images/generations,
/// with each image as base64. `body` is the request as that endpoint takes it.
pub fn generate_image(cfg: &Config, body: &Value) -> Result<Value, ApiError> {
    let req = http().post(&format!("{}/images/generations", cfg.base_url)).config().timeout_global(Some(Duration::from_secs(300))).build();
    let res = auth(req, cfg)?.send_json(body).map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    res.into_body().with_config().limit(60 * 1024 * 1024).read_json().map_err(transport)
}

/// The server's image models, for Settings: (id, name, price).
pub fn image_models(cfg: &Config) -> Result<Vec<(String, String, String)>, ApiError> {
    let req = http().get(&format!("{}/images/models", cfg.base_url)).config().timeout_global(Some(Duration::from_secs(20))).build();
    let res = auth(req, cfg)?.call().map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let body: Value = res.into_body().read_json().map_err(transport)?;
    let s = |m: &Value, k: &str| m.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(body.get("data").and_then(Value::as_array).into_iter().flatten().map(|m| (s(m, "id"), s(m, "name"), s(m, "cost"))).collect())
}

pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [Message],
    pub tools: Option<&'a [Value]>,
    pub tool_choice_none: bool,
    pub temperature: Option<f64>,
    pub effort: Option<&'a str>,
}

/// Streams one chat completion, collecting text, reasoning and tool calls as they arrive. Checks `cancel` between chunks.
pub fn stream_chat(cfg: &Config, req: ChatRequest, on: &mut dyn FnMut(Delta), cancel: &AtomicBool) -> Result<StreamResult, ApiError> {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(t) = req.tools.filter(|t| !t.is_empty()) {
        body["tools"] = Value::Array(t.to_vec());
        body["tool_choice"] = json!(if req.tool_choice_none { "none" } else { "auto" });
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(e) = req.effort {
        body["reasoning_effort"] = json!(e);
    }
    let res = auth(http().post(&format!("{}/chat/completions", cfg.base_url)), cfg)?
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(&body).unwrap_or_default())
        .map_err(transport)?;
    if !res.status().is_success() {
        let mut e = error_from(res);
        if is_kilo(cfg) {
            e.message = e.message.replace("check your sign-in or API key", "paid Kilo models need a key: add one in Settings > Providers");
        }
        return Err(e);
    }
    let num = |h: &str| header(&res, h).and_then(|v| v.parse::<f64>().ok());
    let meta = Meta { cost: num("x-scoutcode-cost"), balance: num("x-scoutcode-balance") };
    let event_stream = header(&res, "content-type").is_some_and(|t| t.contains("event-stream"));
    let mut out = StreamResult { meta, ..Default::default() };

    // Some upstreams ignore stream: true; accept a plain JSON answer as well.
    if !event_stream {
        let data: Value = res.into_body().read_json().map_err(transport)?;
        let msg = data.pointer("/choices/0/message").cloned().unwrap_or(Value::Null);
        out.content = msg.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
        out.reasoning = msg.get("reasoning_content").or_else(|| msg.get("reasoning")).and_then(Value::as_str).unwrap_or_default().to_string();
        out.tool_calls = msg.get("tool_calls").and_then(|t| serde_json::from_value(t.clone()).ok()).unwrap_or_default();
        out.finish_reason = data.pointer("/choices/0/finish_reason").and_then(Value::as_str).map(String::from);
        out.usage = data.get("usage").map(usage_from);
        if !out.reasoning.is_empty() {
            on(Delta::Reasoning(&out.reasoning));
        }
        if !out.content.is_empty() {
            on(Delta::Text(&out.content));
        }
        return Ok(out);
    }

    struct Partial {
        id: String,
        name: String,
        args: String,
        extra: Option<Value>,
    }
    let mut calls: Vec<Option<Partial>> = Vec::new();
    let mut reader = BufReader::with_capacity(16 * 1024, res.into_body().into_reader());
    let mut line = String::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ApiError::aborted());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(ApiError::aborted());
                }
                return Err(ApiError::new(e.to_string(), 0));
            }
        }
        // "[DONE]" doesn't parse, so it is skipped; ScoutCode's cost chunk comes after it, so read on to the end.
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
        let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
        if let Some(err) = chunk.get("error") {
            // The code can be a name, like Kilo's PAID_MODEL_AUTH_REQUIRED, rather than a status.
            let code = err.get("code").and_then(|c| c.as_u64().or_else(|| c.as_str()?.parse().ok())).unwrap_or(500) as u16;
            return Err(ApiError::new(err.get("message").and_then(Value::as_str).unwrap_or("Stream error"), code));
        }
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            out.usage = Some(usage_from(u));
        }
        // What a token-priced answer cost, from ScoutCode, once the answer is done.
        if let Some(sc) = chunk.get("scoutcode") {
            let num = |k: &str| sc.get(k).and_then(Value::as_str).and_then(|v| v.parse::<f64>().ok());
            out.meta.cost = num("cost").or(out.meta.cost);
            out.meta.balance = num("balance").or(out.meta.balance);
        }
        let Some(choice) = chunk.pointer("/choices/0") else { continue };
        if let Some(d) = choice.get("delta") {
            if let Some(r) = d.get("reasoning_content").or_else(|| d.get("reasoning")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.reasoning.push_str(r);
                on(Delta::Reasoning(r));
            }
            if let Some(t) = d.get("content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.content.push_str(t);
                on(Delta::Text(t));
            }
            for tc in d.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let i = tc.get("index").and_then(Value::as_u64).map(|i| i as usize).unwrap_or(calls.len());
                if calls.len() <= i {
                    calls.resize_with(i + 1, || None);
                }
                let c = calls[i].get_or_insert_with(|| Partial { id: format!("call_{i}"), name: String::new(), args: String::new(), extra: None });
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    c.id = id.to_string();
                }
                if let Some(e) = tc.get("extra_content") {
                    c.extra = Some(e.clone());
                }
                if let Some(n) = tc.pointer("/function/name").and_then(Value::as_str) {
                    c.name.push_str(n);
                }
                if let Some(a) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                    c.args.push_str(a);
                }
            }
        }
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            out.finish_reason = Some(f.to_string());
        }
    }
    // Drain so the connection can go back to the pool.
    let _ = reader.read_to_end(&mut Vec::new());
    out.tool_calls = calls
        .into_iter()
        .flatten()
        .filter(|c| !c.name.is_empty())
        .map(|c| ToolCall {
            id: c.id,
            kind: "function".into(),
            extra_content: c.extra,
            function: FunctionCall { name: c.name, arguments: if c.args.is_empty() { "{}".into() } else { c.args } },
        })
        .collect();
    Ok(out)
}
