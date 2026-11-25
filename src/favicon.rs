//! The icon to suggest for a provider the user adds: the favicon of the site its address is on. An API's own host
//! (api.anthropic.com) rarely has one, so the site's main domain (anthropic.com) is tried as well.

use base64::Engine;
use regex::Regex;
use std::sync::OnceLock;

/// The biggest icon kept: it's saved in the settings and sent to the window with them.
const MAX_ICON: usize = 256 * 1024;

/// The sites to look on for `address`: its own host, then its main domain, and that with "www.".
fn sites(address: &str) -> Vec<url::Url> {
    let with_scheme = if address.contains("://") { address.to_string() } else { format!("https://{address}") };
    let Ok(u) = url::Url::parse(&with_scheme) else { return vec![] };
    let Some(host) = u.host_str().map(str::to_string) else { return vec![] };
    let scheme = if u.scheme() == "http" { "http" } else { "https" };
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    let mut hosts = vec![host.clone()];
    let labels: Vec<&str> = host.split('.').collect();
    // Not for an IP address or a single-label host such as localhost.
    if labels.len() > 2 && host.parse::<std::net::IpAddr>().is_err() {
        let main = labels[labels.len() - 2..].join(".");
        hosts.push(main.clone());
        hosts.push(format!("www.{main}"));
    }
    hosts.dedup();
    hosts.into_iter().filter_map(|h| url::Url::parse(&format!("{scheme}://{h}{port}/")).ok()).collect()
}

fn link_tags() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?is)<link\b[^>]*>").unwrap())
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"(?is)\b{name}\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)).ok()?;
    let c = re.captures(tag)?;
    c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str().trim().to_string())
}

/// The icons a page names, best first: an Apple touch icon or SVG (both large and sharp), then the rest by size.
fn icons_in(page: &str, base: &url::Url) -> Vec<url::Url> {
    let mut found: Vec<(i32, url::Url)> = vec![];
    for tag in link_tags().find_iter(page).map(|m| m.as_str()) {
        let Some(rel) = attr(tag, "rel").map(|r| r.to_lowercase()) else { continue };
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon" || r == "apple-touch-icon-precomposed") {
            continue;
        }
        let Some(href) = attr(tag, "href").filter(|h| !h.is_empty()) else { continue };
        let Ok(at) = base.join(&href.replace("&amp;", "&")) else { continue };
        let size = attr(tag, "sizes").and_then(|s| s.split(['x', 'X']).next().and_then(|n| n.parse::<i32>().ok())).unwrap_or(16);
        let svg = attr(tag, "type").is_some_and(|t| t.contains("svg")) || at.path().ends_with(".svg");
        let score = if rel.contains("apple-touch-icon") { 1000 + size } else if svg { 900 } else { size };
        found.push((score, at));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, u)| u).collect()
}

/// The picture at `at` as a data: URL, if it is one and isn't too big.
fn fetch_image(at: &url::Url) -> Option<String> {
    let res = crate::api::http().get(at.as_str()).call().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let declared = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_lowercase();
    let bytes = res.into_body().with_config().limit(MAX_ICON as u64).read_to_vec().ok()?;
    if bytes.is_empty() {
        return None;
    }
    // Some servers call every file text/html or octet-stream, so the file's own start says what it is.
    let sniffed = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        "image/x-icon"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase().contains("<svg") {
        "image/svg+xml"
    } else if declared.starts_with("image/") {
        declared.as_str()
    } else {
        return None;
    };
    Some(format!("data:{sniffed};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

/// The suggested icon for a provider at `address`, as a data: URL.
pub fn suggest(address: &str) -> Result<String, String> {
    let sites = sites(address);
    if sites.is_empty() {
        return Err("Type the provider's address first.".into());
    }
    for site in &sites {
        if let Ok(res) = crate::api::http().get(site.as_str()).call() {
            if res.status().is_success() {
                // Where the page ended up, after any redirect, is what its links are relative to.
                let base = res.headers().get("content-location").and_then(|v| v.to_str().ok()).and_then(|l| site.join(l).ok()).unwrap_or_else(|| site.clone());
                let page = res.into_body().with_config().limit(1024 * 1024).read_to_string().unwrap_or_default();
                for icon in icons_in(&page, &base).iter().take(4) {
                    if let Some(data) = fetch_image(icon) {
                        return Ok(data);
                    }
                }
            }
        }
        if let Some(data) = site.join("/favicon.ico").ok().as_ref().and_then(fetch_image) {
            return Ok(data);
        }
    }
    Err("No icon found on that site.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_api_host_also_tries_its_main_domain() {
        let s: Vec<String> = sites("https://api.anthropic.com/v1").iter().map(|u| u.to_string()).collect();
        assert_eq!(s, vec!["https://api.anthropic.com/", "https://anthropic.com/", "https://www.anthropic.com/"]);
        assert_eq!(sites("http://localhost:11434/v1").iter().map(|u| u.to_string()).collect::<Vec<_>>(), vec!["http://localhost:11434/"]);
        assert!(sites("").is_empty());
    }

    #[test]
    #[ignore = "reaches the internet"]
    fn live_icons_for_the_presets() {
        for site in ["https://api.openai.com/v1", "https://api.anthropic.com/v1"] {
            let icon = suggest(site).unwrap_or_else(|e| panic!("{site}: {e}"));
            assert!(icon.starts_with("data:image/"), "{site}: {}", &icon[..40.min(icon.len())]);
        }
    }

    #[test]
    fn page_icons_come_best_first() {
        let base = url::Url::parse("https://example.com/").unwrap();
        let page = r#"<link rel="icon" href="/favicon-16.png" sizes="16x16"><link rel='icon' type="image/png" sizes="32x32" href="f32.png"><link rel="apple-touch-icon" href="https://cdn.example.com/apple.png"><link rel="stylesheet" href="a.css">"#;
        let got: Vec<String> = icons_in(page, &base).iter().map(|u| u.to_string()).collect();
        assert_eq!(got, vec!["https://cdn.example.com/apple.png", "https://example.com/f32.png", "https://example.com/favicon-16.png"]);
    }
}
