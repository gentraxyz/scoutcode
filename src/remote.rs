//! ScoutMote: lets the account's website drive this Scout from a browser. While it's on, one thread long-polls the site
//! for commands (a prompt, stop, open a chat, answer a permission prompt), which go to the core as `Ev::Remote`, and
//! another posts what's on screen whenever the core publishes something new. See src/lib/scoutmote.ts on the server.
//! This computer takes no incoming connections.

use crate::api::http;
use crate::config::Config;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub enum RemoteEvent {
    /// What to show in Settings, and whether the site answered.
    Status(String, bool),
    /// A command from the website.
    Command(Value),
    /// It stopped by itself, and why.
    Ended(String),
}

pub struct Remote {
    stop: Arc<AtomicBool>,
    /// The latest state, and a number that goes up with each change.
    state: Arc<Mutex<(u64, Value)>>,
}

impl Remote {
    /// Hands the pusher what's on screen now. Sent only when it differs from the last one.
    pub fn publish(&self, state: Value) {
        let mut s = self.state.lock().unwrap();
        if s.1 != state {
            s.0 += 1;
            s.1 = state;
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The name the website shows for this computer.
pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Computer".into())
}

fn post(cfg: &Config, body: &Value, timeout: u64) -> Result<Value, (u16, String)> {
    let res = http()
        .post(&format!("{}/remote/device", cfg.base_url))
        .config()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .build()
        .header("Authorization", &format!("Bearer {}", cfg.api_key.as_deref().unwrap_or("")))
        .send_json(body)
        .map_err(|e| (0, e.to_string()))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err((status, format!("HTTP {status}")));
    }
    res.into_body().read_json().map_err(|e| (0, e.to_string()))
}

/// How long to wait before the next poll: what the site asks for (`next`, in ms), kept to a sane range. Sites from
/// before quick polling don't say, so a few seconds.
pub fn next_poll(reply: &Value) -> u64 {
    reply["next"].as_u64().unwrap_or(3000).clamp(500, 30_000)
}

fn nap(stop: &AtomicBool, ms: u64) {
    for _ in 0..ms / 100 {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Turns ScoutMote on as `device_id`, until it's stopped.
pub fn start(cfg: &Config, device_id: &str, tell: impl Fn(RemoteEvent) + Send + Sync + 'static) -> Remote {
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new((0u64, Value::Null)));
    let tell = Arc::new(tell);
    let name = computer_name();
    // The site lost what's on screen (it restarted), so send it again.
    let resend = Arc::new(AtomicBool::new(false));

    // Commands.
    {
        let (cfg, stop, tell, id, name, resend) = (cfg.clone(), stop.clone(), tell.clone(), device_id.to_string(), name.clone(), resend.clone());
        std::thread::spawn(move || {
            let mut failures = 0u32;
            let mut up = false;
            let ended = loop {
                if stop.load(Ordering::Relaxed) {
                    break None;
                }
                // Quick polls with a pause the site picks (`next`), rather than one held open: on shared hosting a held
                // request takes one of the few connections the host lets through, and the website waits for them.
                match post(&cfg, &json!({ "deviceId": id, "name": name }), 30) {
                    Ok(reply) => {
                        if failures > 0 || !up {
                            failures = 0;
                            up = true;
                            tell(RemoteEvent::Status("On. Open ScoutCode in a browser to use Scout here.".into(), true));
                        }
                        if reply["needState"] == true {
                            resend.store(true, Ordering::Relaxed);
                            // The site answers at once until it has the state, so give the pusher a moment.
                            nap(&stop, 1000);
                        }
                        if stop.load(Ordering::Relaxed) {
                            break None;
                        }
                        let commands = reply["commands"].as_array().cloned().unwrap_or_default();
                        let busy = !commands.is_empty();
                        for c in commands {
                            tell(RemoteEvent::Command(c));
                        }
                        // Right after a command another often follows (an answer to a prompt it raised), so look again soon.
                        nap(&stop, if busy { 300 } else { next_poll(&reply) });
                    }
                    Err((401, _)) => break Some("ScoutCode refused this sign-in. Sign in again.".to_string()),
                    Err((_, e)) => {
                        failures += 1;
                        tell(RemoteEvent::Status(format!("Can't reach ScoutCode ({e}). Trying again…"), false));
                        nap(&stop, 1000 * 2u64.saturating_pow(failures.min(5)).min(30));
                    }
                }
            };
            if let Some(why) = ended {
                stop.store(true, Ordering::Relaxed);
                tell(RemoteEvent::Ended(why));
            }
        });
    }

    // What's on screen.
    {
        let (cfg, stop, state, id, resend) = (cfg.clone(), stop.clone(), state.clone(), device_id.to_string(), resend.clone());
        std::thread::spawn(move || {
            let mut sent = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let (version, value) = {
                    let s = state.lock().unwrap();
                    (s.0, s.1.clone())
                };
                // Changes go at most about three times a second; the command poll keeps the PC online in between.
                if (version != sent || resend.swap(false, Ordering::Relaxed)) && !value.is_null() {
                    if post(&cfg, &json!({ "deviceId": id, "name": name, "state": value }), 20).is_ok() {
                        sent = version;
                    } else {
                        nap(&stop, 2000);
                    }
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            let _ = post(&cfg, &json!({ "deviceId": id, "done": true }), 5);
        });
    }

    Remote { stop, state }
}
