use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

pub const MODES: [&str; 5] = ["default", "acceptEdits", "auto", "plan", "yolo"];
pub const EFFORTS: [&str; 4] = ["none", "low", "medium", "high"];
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";
pub const NVIDIA_URL: &str = "https://integrate.api.nvidia.com/v1";
pub const HERMES_URL: &str = "https://inference-api.nousresearch.com/v1";
pub const KILO_URL: &str = "https://ai-gateway.kilo.ai/api/v1";
pub const KILO_DEFAULT_MODEL: &str = "kilo-auto/free";
pub const KILO_WARNING: &str = "Kilo's free models may log your prompts and train on them, so don't send them anything private. They allow 200 requests an hour per network, and every tool call is a request.";

/// A smaller provider, listed together under Additional Providers in the model menu. All of them speak OpenAI's API.
pub struct AdditionalProvider {
    /// What `Config::provider` says when a model runs here.
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    /// Without a key it can't be used at all. Otherwise a key is only for higher limits.
    pub needs_key: bool,
    pub note: &'static str,
    pub key_url: &'static str,
    pub key_link: &'static str,
    pub key_hint: &'static str,
    /// What to start on when there's nothing else to run on, if it's still listed.
    pub default_model: &'static str,
}

pub static ADDITIONAL: [AdditionalProvider; 2] = [
    AdditionalProvider {
        id: "ovh",
        name: "OVH Cloud",
        url: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1",
        needs_key: false,
        note: "Free, with no account or key. It allows 2 requests a minute per model, and every tool call is a request, so Scout often pauses to wait for the limit.",
        key_url: "https://www.ovhcloud.com/en/public-cloud/ai-endpoints/",
        key_link: "Get a key from OVH Cloud",
        key_hint: "OVH AI Endpoints key",
        default_model: "Qwen3.8-27B",
    },
    AdditionalProvider {
        id: "google",
        name: "Google AI Studio",
        url: "https://generativelanguage.googleapis.com/v1beta/openai",
        needs_key: true,
        note: "Gemini models, with your own key. Saved on this PC. On Google's free tier, your prompts may be used to improve its products.",
        key_url: "https://aistudio.google.com/apikey",
        key_link: "Make a key in Google AI Studio",
        key_hint: "AIza…",
        default_model: "gemini-flash-latest",
    },
];

pub fn additional_provider(id: &str) -> Option<&'static AdditionalProvider> {
    ADDITIONAL.iter().find(|a| a.id == id)
}

/// An additional provider the user has linked, with their key if it has one.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LinkedProvider {
    pub id: String,
    pub key: Option<String>,
}

/// The Scout Enterprise team the signed-in account is in, as the server says (GET /api/account/team). It holds no keys:
/// the team keeps those on the server, and its providers' requests go through /v1/team/<provider> with the user's own
/// sign-in. Saved so the app starts the same way offline, and replaced whenever the model list is read.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    pub name: String,
    pub role: String,
    /// The providers that go through the server, with the models members may use on each (None for every one).
    pub providers: Vec<TeamProvider>,
    /// The providers members sign in to on this PC themselves (Claude Code, Kiro…), which the team allows.
    pub acp: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TeamProvider {
    pub id: String,
    pub models: Option<Vec<String>>,
}

impl Team {
    /// Whether the team runs `provider` for its members.
    pub fn runs(&self, provider: &str) -> bool {
        self.providers.iter().any(|p| p.id == provider)
    }

    /// Whether members may use `model` (that provider's own id for it) on `provider`.
    pub fn allows(&self, provider: &str, model: &str) -> bool {
        self.providers.iter().find(|p| p.id == provider).is_some_and(|p| p.models.as_ref().is_none_or(|m| m.iter().any(|x| x == model)))
    }
}

/// Where Kilo is: KILO_BASE_URL overrides it for testing.
pub fn kilo_url() -> String {
    std::env::var("KILO_BASE_URL").map(|u| u.trim_end_matches('/').to_string()).unwrap_or_else(|_| KILO_URL.to_string())
}

pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "acceptEdits" => "Accept edits",
        "auto" => "Auto",
        "plan" => "Read Only",
        "yolo" => "Accept All",
        _ => "Ask first",
    }
}

/// ~/.scoutcode/config.json, the file the old CLI used too, so its login and allow list carry over.
#[derive(Clone, Debug)]
pub struct Config {
    pub api_key: Option<String>,
    /// The user has chosen to use ScoutCode's free models without an account. A key wins over this. Only set on the
    /// settings for ScoutCode itself: see `for_provider`.
    pub guest: bool,
    /// The user's own OpenRouter keys, in the order they're tried. For models on OpenRouter.
    pub openrouter_keys: Vec<OpenRouterKey>,
    /// The user's own NVIDIA key, for models on NVIDIA NIM.
    pub nvidia_key: Option<String>,
    /// The user has chosen Kilo Code. Its models are listed either way, since they need no key.
    pub kilo_enabled: bool,
    /// The user's own Kilo key. Only paid models need it.
    pub kilo_key: Option<String>,
    /// The user's own GitHub token for Copilot, from signing in or COPILOT_GITHUB_TOKEN.
    pub copilot_token: Option<String>,
    /// The GitHub account that token belongs to, for Settings.
    pub copilot_user: Option<String>,
    /// The user has signed in to Devin here. Its CLI keeps the token; Scout never sees it.
    pub devin_enabled: bool,
    /// The user has signed in to Kiro here. Its CLI keeps the token; Scout never sees it.
    pub kiro_enabled: bool,
    /// The user has signed in to Codex here. Its CLI keeps the token; Scout never sees it.
    pub codex_enabled: bool,
    /// The user has signed in to Claude Code here. Its CLI keeps the token; Scout never sees it.
    pub claude_enabled: bool,
    /// The user has turned on OpenCode here. Its free models need no account.
    pub opencode_enabled: bool,
    /// The user has signed in to an OpenCode account here. Its CLI keeps the token; Scout never sees it.
    pub opencode_signed_in: bool,
    /// The user has signed in to Grok here. Its CLI keeps the token; Scout never sees it.
    pub grok_signed_in: bool,
    /// The user's own xAI key, for Grok Build without a sign-in.
    pub grok_key: Option<String>,
    /// The user has signed in to Nous Portal here. The tokens are in their own file: see `hermes.rs`.
    pub hermes_signed_in: bool,
    /// The user's own Nous Portal key, used when not signed in.
    pub hermes_key: Option<String>,
    /// The additional providers the user has linked, in the order they linked them.
    pub additional: Vec<LinkedProvider>,
    /// Where the chosen model runs: "scoutcode", "openrouter", "nvidia", "kilo", "copilot", "devin", "kiro", "codex", "claude", "opencode", "grok", "hermes" or an additional provider's id. The model is that provider's id for it.
    pub provider: String,
    /// Who the key belongs to, for the sidebar. Saved at sign-in and refreshed from the model list.
    pub username: Option<String>,
    pub base_url: String,
    pub model: String,
    pub mode: String,
    pub effort: String,
    /// Shell command prefixes allowed without asking, per project directory.
    pub allow: HashMap<String, Vec<String>>,
    pub auto_compact_tokens: u64,
    pub prune_tokens: u64,
    pub subagent_model: String,
    pub temperature: Option<f64>,
    /// Desktop app only: models pinned to the top of the picker.
    pub starred_models: Vec<String>,
    /// Desktop app only: named folders that chats are grouped under.
    pub projects: Vec<Project>,
    /// Desktop app only: how to get the user's attention when a turn ends or needs them while the window is in the
    /// background. None until they choose, which the first run asks.
    pub notify_flash: Option<bool>,
    pub notify_toast: Option<bool>,
    /// Desktop app only: show "Using Scout" on the user's Discord profile. On unless they turn it off.
    pub discord_presence: bool,
    /// Desktop app only: include the open folder's name in that presence. Off until they turn it on.
    pub discord_project: bool,
    /// Desktop app only: download new versions in the background, ready for the next restart.
    pub auto_update: bool,
    /// Desktop app only: the version that last ran here, to tell when the app has been updated.
    pub last_version: Option<String>,
    /// Desktop app only: the models to try when the chosen one fails.
    pub fallbacks: Fallbacks,
    /// Experimental: the agent gets a generate_image tool, which makes images on the ScoutCode server. Off until turned on.
    pub experimental_images: bool,
    /// The server's image model generate_image uses. None for the server's default.
    pub image_model: Option<String>,
    /// Desktop app only, experimental: a game to play while Scout works.
    pub games: Games,
    /// The Scout Enterprise team the account is in. While there is one, its providers replace the user's own.
    pub team: Option<Team>,
    /// Desktop app only: ScoutMote, so the account's website can drive this Scout. Off until turned on.
    pub scout_mote: bool,
    /// Desktop app only: what ScoutMote calls this computer on the site, kept so it stays the same PC.
    pub scout_mote_id: Option<String>,
}

/// A model in the picker: its list key and where it runs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModelRef {
    pub key: String,
    pub provider: String,
}

/// The models to try, in order, when the chosen one fails.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Fallbacks {
    /// The user has answered the card the first failure shows, so a model without fallbacks just shows its error.
    pub chosen: bool,
    /// For every model without a list of its own.
    pub all: Vec<ModelRef>,
    /// For single models: each one and its own list.
    pub models: Vec<(ModelRef, Vec<ModelRef>)>,
}

impl Fallbacks {
    /// What to try after `failed`: its own list, or the one for every model, without itself.
    pub fn after(&self, failed: &ModelRef) -> Vec<ModelRef> {
        let list = self.models.iter().find(|(m, _)| m == failed).map(|(_, l)| l).unwrap_or(&self.all);
        list.iter().filter(|m| *m != failed).cloned().collect()
    }
}

/// Experimental games: where they show and when they open and close by themselves. Off until turned on.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Games {
    pub on: bool,
    /// "float", a card the user drags anywhere over the window, or "panel", a tab in the right-hand panel.
    pub place: String,
    /// Opens when Scout starts working.
    pub open_on_run: bool,
    /// Closes when Scout has finished, in every chat.
    pub auto_close: bool,
    /// Where the floating card was left: its top left, in window pixels. None for the corner above the message box.
    pub pos: Option<(f64, f64)>,
}

impl Default for Games {
    fn default() -> Self {
        Games { on: false, place: "float".into(), open_on_run: true, auto_close: true, pos: None }
    }
}

/// One of the user's OpenRouter keys, under the name they gave it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenRouterKey {
    pub name: String,
    pub key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub name: String,
    pub path: String,
}

pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("SCOUT_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    PathBuf::from(base).join(".scoutcode")
}

fn file() -> PathBuf {
    home().join("config.json")
}

fn read_saved() -> Map<String, Value> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The saved OpenRouter keys in the order they're tried. A file from before the list has one `openrouterKey`, which
/// becomes the only key.
fn saved_openrouter_keys(s: &Map<String, Value>) -> Vec<OpenRouterKey> {
    if let Some(list) = s
        .get("openrouterKeys")
        .and_then(|v| serde_json::from_value::<Vec<OpenRouterKey>>(v.clone()).ok())
    {
        return list.into_iter().filter(|k| !k.key.is_empty()).collect();
    }
    s.get("openrouterKey")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
        .map(|k| {
            vec![OpenRouterKey {
                name: "OpenRouter".into(),
                key: k.into(),
            }]
        })
        .unwrap_or_default()
}

pub fn load_config() -> Config {
    let s = read_saved();
    let str_ = |k: &str, d: &str| s.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let num = |k: &str, d: u64| s.get(k).and_then(Value::as_f64).map(|n| n as u64).unwrap_or(d);
    let allow = s
        .get("allow")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let mut cfg = Config {
        api_key: s.get("apiKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        guest: s.get("desktopGuest").and_then(Value::as_bool).unwrap_or(false),
        openrouter_keys: saved_openrouter_keys(&s),
        nvidia_key: s.get("nvidiaKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        kilo_enabled: s.get("kiloEnabled").and_then(Value::as_bool).unwrap_or(false),
        kilo_key: s.get("kiloKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        copilot_token: s.get("copilotToken").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        copilot_user: s.get("copilotUser").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        devin_enabled: s.get("devinEnabled").and_then(Value::as_bool).unwrap_or(false),
        kiro_enabled: s.get("kiroEnabled").and_then(Value::as_bool).unwrap_or(false),
        codex_enabled: s.get("codexEnabled").and_then(Value::as_bool).unwrap_or(false),
        claude_enabled: s.get("claudeEnabled").and_then(Value::as_bool).unwrap_or(false),
        opencode_enabled: s.get("opencodeEnabled").and_then(Value::as_bool).unwrap_or(false),
        opencode_signed_in: s.get("opencodeSignedIn").and_then(Value::as_bool).unwrap_or(false),
        grok_signed_in: s.get("grokSignedIn").and_then(Value::as_bool).unwrap_or(false),
        grok_key: s.get("grokKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        hermes_signed_in: crate::hermes::signed_in(),
        hermes_key: s.get("hermesKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        additional: s
            .get("additionalProviders")
            .and_then(|v| serde_json::from_value::<Vec<LinkedProvider>>(v.clone()).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|l| additional_provider(&l.id).is_some())
            .collect(),
        provider: str_("provider", "scoutcode"),
        username: s.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()).map(String::from),
        base_url: str_("baseUrl", "https://scoutcode.dev/v1"),
        model: str_("model", "auto"),
        mode: str_("mode", "default"),
        // A file from before "default" was removed falls back to low.
        effort: Some(str_("effort", "low")).filter(|e| EFFORTS.contains(&e.as_str())).unwrap_or_else(|| "low".into()),
        allow,
        auto_compact_tokens: num("autoCompactTokens", 90_000),
        prune_tokens: num("pruneTokens", 60_000),
        subagent_model: str_("subagentModel", "qwen3.8-27b"),
        temperature: s.get("temperature").and_then(Value::as_f64),
        starred_models: s.get("starredModels").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        projects: s.get("desktopProjects").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        notify_flash: s.get("desktopFlash").and_then(Value::as_bool),
        notify_toast: s.get("desktopToast").and_then(Value::as_bool),
        discord_presence: s.get("discordPresence").and_then(Value::as_bool).unwrap_or(true),
        discord_project: s.get("discordProject").and_then(Value::as_bool).unwrap_or(false),
        auto_update: s.get("desktopAutoUpdate").and_then(Value::as_bool).unwrap_or(true),
        last_version: s.get("desktopLastVersion").and_then(Value::as_str).map(str::to_string),
        fallbacks: s.get("modelFallbacks").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        experimental_images: s.get("experimentalImages").and_then(Value::as_bool).unwrap_or(false),
        image_model: s.get("imageModel").and_then(Value::as_str).filter(|m| !m.is_empty()).map(String::from),
        games: s.get("games").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        team: s.get("team").and_then(|v| serde_json::from_value(v.clone()).ok()),
        scout_mote: s.get("scoutMote").and_then(Value::as_bool).unwrap_or(false),
        scout_mote_id: s.get("scoutMoteId").and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from),
    };
    // Environment variables win over the file, so one-off runs need no login.
    if let Ok(k) = std::env::var("SCOUTCODE_API_KEY") {
        cfg.api_key = Some(k);
    }
    if let Ok(u) = std::env::var("SCOUTCODE_BASE_URL") {
        cfg.base_url = u;
    }
    if let Ok(k) = std::env::var("OPENROUTER_API_KEY") {
        cfg.openrouter_keys = vec![OpenRouterKey {
            name: "Environment".into(),
            key: k,
        }];
    }
    if let Ok(k) = std::env::var("NVIDIA_API_KEY") {
        cfg.nvidia_key = Some(k);
    }
    if let Ok(k) = std::env::var("KILO_API_KEY") {
        cfg.kilo_key = Some(k);
        cfg.kilo_enabled = true;
    }
    if let Ok(k) = std::env::var("COPILOT_GITHUB_TOKEN") {
        cfg.copilot_token = Some(k);
    }
    cfg.base_url = cfg.base_url.trim_end_matches('/').to_string();
    // With nothing else to run on, models run on Kilo.
    if cfg.provider == "scoutcode" && cfg.only_kilo() {
        cfg.provider = "kilo".into();
        cfg.model = KILO_DEFAULT_MODEL.into();
    }
    cfg
}

impl Config {
    pub fn kilo_on(&self) -> bool {
        self.kilo_enabled || self.kilo_key.is_some()
    }

    pub fn grok_on(&self) -> bool {
        self.grok_signed_in || self.grok_key.is_some()
    }

    pub fn hermes_on(&self) -> bool {
        self.hermes_signed_in || self.hermes_key.is_some()
    }

    /// Whether a provider members sign in to on this PC (Claude Code, Kiro…) may be used: always, outside a team.
    pub fn acp_allowed(&self, which: &str) -> bool {
        self.team.as_ref().is_none_or(|t| t.acp.iter().any(|a| a == which))
    }

    /// Whether Kilo is the only provider there's a way to use.
    pub fn only_kilo(&self) -> bool {
        self.team.is_none() && self.kilo_on() && self.api_key.is_none() && !self.guest && self.openrouter_keys.is_empty() && self.nvidia_key.is_none() && self.copilot_token.is_none() && !self.devin_enabled && !self.kiro_enabled && !self.codex_enabled && !self.claude_enabled && !self.opencode_enabled && !self.grok_on() && !self.hermes_on() && self.additional.is_empty()
    }

    /// Whether requests to ScoutCode go without an account.
    pub fn is_guest(&self) -> bool {
        self.guest && self.api_key.is_none()
    }

    /// The additional provider `id`, if the user has linked it.
    pub fn linked(&self, id: &str) -> Option<&LinkedProvider> {
        self.additional.iter().find(|l| l.id == id)
    }

    /// The same settings pointed at the provider a request goes to: ScoutCode's server, OpenRouter with its first key,
    /// or NVIDIA NIM, Kilo or an additional provider with its key.
    pub fn for_provider(&self, provider: &str) -> Config {
        let mut c = self.clone();
        c.guest = self.guest && provider == "scoutcode";
        // A team's providers go through ScoutCode's server, which adds the team's key, with the user's own sign-in.
        if provider != "scoutcode" && self.team.as_ref().is_some_and(|t| t.runs(provider)) {
            c.guest = false;
            c.base_url = format!("{}/team/{provider}", self.base_url);
            return c;
        }
        if provider == "openrouter" {
            c.base_url = OPENROUTER_URL.to_string();
            c.api_key = self.openrouter_keys.first().map(|k| k.key.clone());
        } else if provider == "nvidia" {
            c.base_url = NVIDIA_URL.to_string();
            c.api_key = self.nvidia_key.clone();
        } else if provider == "kilo" {
            c.base_url = kilo_url();
            c.api_key = self.kilo_key.clone();
        } else if provider == "hermes" {
            // A sign-in's token goes in just before each request: see `hermes::bearer`.
            c.base_url = HERMES_URL.to_string();
            c.api_key = self.hermes_key.clone();
        } else if let Some(a) = additional_provider(provider) {
            c.base_url = a.url.to_string();
            c.api_key = self.linked(a.id).and_then(|l| l.key.clone());
        }
        c
    }

    /// The same settings pointed at OpenRouter with one of the user's keys.
    pub fn for_openrouter_key(&self, key: &str) -> Config {
        let mut c = self.clone();
        c.guest = false;
        c.base_url = OPENROUTER_URL.to_string();
        c.api_key = Some(key.to_string());
        c
    }

    /// Copies the keys from `other`, leaving the rest (such as the allow list, which the agent saves) alone.
    pub fn take_keys(&mut self, other: &Config) {
        self.api_key = other.api_key.clone();
        self.guest = other.guest;
        self.openrouter_keys = other.openrouter_keys.clone();
        self.nvidia_key = other.nvidia_key.clone();
        self.kilo_enabled = other.kilo_enabled;
        self.kilo_key = other.kilo_key.clone();
        self.copilot_token = other.copilot_token.clone();
        self.copilot_user = other.copilot_user.clone();
        self.devin_enabled = other.devin_enabled;
        self.kiro_enabled = other.kiro_enabled;
        self.codex_enabled = other.codex_enabled;
        self.claude_enabled = other.claude_enabled;
        self.opencode_enabled = other.opencode_enabled;
        self.opencode_signed_in = other.opencode_signed_in;
        self.grok_signed_in = other.grok_signed_in;
        self.grok_key = other.grok_key.clone();
        self.hermes_signed_in = other.hermes_signed_in;
        self.hermes_key = other.hermes_key.clone();
        self.additional = other.additional.clone();
        self.experimental_images = other.experimental_images;
        self.image_model = other.image_model.clone();
        self.games = other.games.clone();
        self.team = other.team.clone();
    }
}

/// Saves the OpenRouter keys in order. The first is also kept as `openrouterKey`, for older builds.
pub fn save_openrouter_keys(keys: &[OpenRouterKey]) {
    let mut m = Map::new();
    m.insert("openrouterKeys".into(), json!(keys));
    m.insert(
        "openrouterKey".into(),
        json!(keys.first().map(|k| k.key.clone())),
    );
    save_config(m);
}

/// Saves the given fields, keeping anything else already in the file. Null removes a field.
pub fn save_config(patch: Map<String, Value>) {
    let mut saved = read_saved();
    for (k, v) in patch {
        if v.is_null() {
            saved.remove(&k);
        } else {
            saved.insert(k, v);
        }
    }
    let _ = std::fs::create_dir_all(home());
    if let Ok(text) = serde_json::to_string_pretty(&Value::Object(saved)) {
        let _ = std::fs::write(file(), text);
    }
}

pub fn save_one(key: &str, value: Value) {
    let mut m = Map::new();
    m.insert(key.to_string(), value);
    save_config(m);
}

pub fn is_sign_in(token: Option<&str>) -> bool {
    token.is_some_and(|t| t.starts_with("scout-oauth-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(key: &str) -> ModelRef {
        ModelRef { key: key.into(), provider: "scoutcode".into() }
    }

    #[test]
    fn a_teams_providers_go_through_the_server_with_the_users_own_sign_in() {
        let mut cfg = load_config();
        cfg.base_url = "https://scoutcode.dev/v1".into();
        cfg.api_key = Some("scout-oauth-me".into());
        cfg.openrouter_keys = vec![OpenRouterKey { name: "Mine".into(), key: "sk-or-mine".into() }];
        cfg.team = Some(Team {
            id: "t".into(),
            name: "Acme".into(),
            role: "member".into(),
            providers: vec![TeamProvider { id: "openrouter".into(), models: Some(vec!["a/b".into()]) }, TeamProvider { id: "scoutcode".into(), models: None }],
            acp: vec!["claude".into()],
        });
        let or = cfg.for_provider("openrouter");
        assert_eq!(or.base_url, "https://scoutcode.dev/v1/team/openrouter");
        assert_eq!(or.api_key.as_deref(), Some("scout-oauth-me"), "never the user's own key");
        // What the team doesn't run keeps going where it did, and ScoutCode is ScoutCode.
        assert_eq!(cfg.for_provider("scoutcode").base_url, "https://scoutcode.dev/v1");
        assert_eq!(cfg.for_provider("nvidia").base_url, NVIDIA_URL);
        let t = cfg.team.as_ref().unwrap();
        assert!(t.allows("openrouter", "a/b") && !t.allows("openrouter", "c/d") && t.allows("scoutcode", "anything") && !t.allows("nvidia", "x"));
        assert!(cfg.acp_allowed("claude") && !cfg.acp_allowed("kiro"));
        cfg.team = None;
        assert!(cfg.acp_allowed("kiro"));
    }

    #[test]
    fn a_models_own_fallbacks_win_over_the_ones_for_every_model() {
        let f = Fallbacks { chosen: true, all: vec![m("a"), m("b")], models: vec![(m("x"), vec![m("c")])] };
        assert_eq!(f.after(&m("x")), vec![m("c")]);
        assert_eq!(f.after(&m("y")), vec![m("a"), m("b")]);
        // A model isn't its own fallback.
        assert_eq!(f.after(&m("a")), vec![m("b")]);
        let saved: Fallbacks = serde_json::from_value(json!(f)).unwrap();
        assert_eq!(saved.after(&m("x")), vec![m("c")]);
    }
}
