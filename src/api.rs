use crate::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
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
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(p) => p
                .iter()
                .map(|x| x.get("text").and_then(Value::as_str).unwrap_or("[image]"))
                .collect::<Vec<_>>()
                .join(" "),
        }
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
    pub model: Option<String>,
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
    Meta(&'a Meta),
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
    fn new(message: impl Into<String>, status: u16) -> Self {
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

fn header(res: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    res.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

fn error_from(mut res: ureq::http::Response<ureq::Body>) -> ApiError {
    let status = res.status().as_u16();
    let retry = header(&res, "retry-after").and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0);
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let mut message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|b| b.pointer("/error/message").or_else(|| b.get("message")).and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| if text.is_empty() { format!("HTTP {status}") } else { text.chars().take(500).collect() });
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
        let inputs = list_at(m, "/architecture/input_modalities");
        let params: Vec<String> = list("supported_parameters");
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
        })
    }

    /// The name to show: the server's, without a parenthetical, or the id without its vendor prefix.
    pub fn label(&self) -> String {
        match &self.name {
            Some(n) => n.split(" (").next().unwrap_or(n).trim().to_string(),
            None => short_id(&self.id),
        }
    }
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
fn money(usd: f64) -> String {
    let s = format!("{usd:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The models the key can use, and the username it belongs to when the server says.
pub fn list_models(cfg: &Config) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
    let res = http()
        .get(&format!("{}/models", cfg.base_url))
        .header("Authorization", &format!("Bearer {}", key(cfg)?))
        .call()
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let user = header(&res, "x-scoutcode-user").filter(|u| !u.is_empty());
    let body: Value = res.into_body().read_json().map_err(transport)?;
    let models = body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(ModelInfo::from_json).collect())
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
    let res = http()
        .post(&format!("{}/chat/completions", cfg.base_url))
        .header("Authorization", &format!("Bearer {}", key(cfg)?))
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(&body).unwrap_or_default())
        .map_err(transport)?;
    if !res.status().is_success() {
        return Err(error_from(res));
    }
    let num = |h: &str| header(&res, h).and_then(|v| v.parse::<f64>().ok());
    let meta = Meta { model: header(&res, "x-scoutcode-model"), cost: num("x-scoutcode-cost"), balance: num("x-scoutcode-balance") };
    on(Delta::Meta(&meta));
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
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
        if let Some(err) = chunk.get("error") {
            let code = err.get("code").and_then(Value::as_u64).unwrap_or(500) as u16;
            return Err(ApiError::new(err.get("message").and_then(Value::as_str).unwrap_or("Stream error"), code));
        }
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            out.usage = Some(usage_from(u));
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
                let c = calls[i].get_or_insert_with(|| Partial { id: format!("call_{i}"), name: String::new(), args: String::new() });
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    c.id = id.to_string();
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
            function: FunctionCall { name: c.name, arguments: if c.args.is_empty() { "{}".into() } else { c.args } },
        })
        .collect();
    Ok(out)
}
