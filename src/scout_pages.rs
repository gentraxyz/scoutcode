//! What web_search adds to the server's result list, all fetched on this computer: library docs from Context7 and the
//! READMEs of matching GitHub repositories. Their full text is kept here as pages at `scout://` addresses, which only
//! web_fetch understands. A `scout://` address is never a real URL: nothing is sent anywhere to read one, and a page
//! lives only as long as this run of Scout.

use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const SCHEME: &str = "scout://";
/// Searches whose pages are kept; older ones are dropped.
const KEPT: usize = 40;
/// How long each extra source may take, so a slow one never holds up the results.
const TIMEOUT: Duration = Duration::from_secs(12);
/// How much of a page goes into the search result itself; the rest is a web_fetch away.
const EXCERPT: usize = 700;
const DOC_TOKENS: u32 = 4000;

struct Search {
    n: usize,
    /// (path under `scout://search/<n>/`, title, text). The combined page has the empty path.
    pages: Vec<(String, String, String)>,
}

fn store() -> &'static Mutex<(usize, VecDeque<Search>)> {
    static S: OnceLock<Mutex<(usize, VecDeque<Search>)>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn is_scout_url(url: &str) -> bool {
    url.get(..SCHEME.len()).is_some_and(|p| p.eq_ignore_ascii_case(SCHEME))
}

/// The text of a `scout://` page, or why there isn't one.
pub fn read(url: &str) -> Result<(String, String), String> {
    let rest = url[SCHEME.len()..].trim_end_matches('/');
    let Some(rest) = rest.strip_prefix("search/") else {
        return Err(format!("{url} isn't a page Scout made. scout:// pages come only from web_search results (scout://search/<n>/...)."));
    };
    let (n, path) = rest.split_once('/').unwrap_or((rest, ""));
    let n: usize = n.parse().map_err(|_| format!("{url} isn't a scout:// page address."))?;
    let s = store().lock().unwrap();
    let search = s.1.iter().find(|x| x.n == n).ok_or_else(|| format!("{url} has expired or never existed. Run web_search again to get fresh pages."))?;
    let (_, title, text) = search
        .pages
        .iter()
        .find(|(p, _, _)| p == path)
        .ok_or_else(|| format!("{url} isn't one of that search's pages. It has: {}", search.pages.iter().map(|(p, _, _)| page_url(n, p)).collect::<Vec<_>>().join(", ")))?;
    Ok((title.clone(), text.clone()))
}

fn page_url(n: usize, path: &str) -> String {
    if path.is_empty() { format!("{SCHEME}search/{n}") } else { format!("{SCHEME}search/{n}/{path}") }
}

/// One extra result: where it came from, its title and its full text.
pub struct Extra {
    kind: &'static str,
    title: String,
    source: String,
    text: String,
}

fn get(url: &str, accept: &str) -> Option<String> {
    let res = crate::api::http().get(url).config().timeout_global(Some(TIMEOUT)).build().header("Accept", accept).call().ok()?;
    if res.status().as_u16() >= 400 {
        return None;
    }
    res.into_body().with_config().limit(4 * 1024 * 1024).read_to_string().ok()
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out += &format!("%{b:02X}"),
        }
    }
    out
}

/// Library docs for the query: Context7's best-matching libraries, then the snippets on the query's topic.
pub fn context7(query: &str, max: usize) -> Vec<Extra> {
    let Some(found) = get(&format!("https://context7.com/api/v1/search?query={}", enc(query)), "application/json") else { return vec![] };
    let Ok(found) = serde_json::from_str::<Value>(&found) else { return vec![] };
    let libs: Vec<(String, String)> = found
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        // Unmaintained or tiny libraries are noise.
        .filter(|r| r.get("trustScore").and_then(Value::as_f64).unwrap_or(0.0) >= 5.0 && r.get("totalSnippets").and_then(Value::as_u64).unwrap_or(0) > 0)
        .filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), r.get("title").and_then(Value::as_str).unwrap_or("").to_string())))
        .take(max)
        .collect();
    std::thread::scope(|sc| {
        let jobs: Vec<_> = libs
            .iter()
            .map(|(id, title)| {
                sc.spawn(move || {
                    let url = format!("https://context7.com/api/v1{id}?type=txt&topic={}&tokens={DOC_TOKENS}", enc(query));
                    let text = get(&url, "text/plain")?;
                    let text = text.trim();
                    (!text.is_empty() && !text.starts_with('{')).then(|| Extra {
                        kind: "docs",
                        title: if title.is_empty() { id.trim_start_matches('/').to_string() } else { title.clone() },
                        source: format!("https://context7.com{id}"),
                        text: text.to_string(),
                    })
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    })
}

/// The READMEs of the GitHub repositories that best match the query.
pub fn github(query: &str, max: usize) -> Vec<Extra> {
    let Some(found) = get(&format!("https://api.github.com/search/repositories?q={}&per_page={max}", enc(query)), "application/vnd.github+json") else { return vec![] };
    let Ok(found) = serde_json::from_str::<Value>(&found) else { return vec![] };
    let repos: Vec<(String, String, u64)> = found
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r.get("full_name")?.as_str()?.to_string(), r.get("description").and_then(Value::as_str).unwrap_or("").to_string(), r.get("stargazers_count").and_then(Value::as_u64).unwrap_or(0))))
        .take(max)
        .collect();
    std::thread::scope(|sc| {
        let jobs: Vec<_> = repos
            .iter()
            .map(|(name, about, stars)| {
                sc.spawn(move || {
                    let readme = get(&format!("https://api.github.com/repos/{name}/readme"), "application/vnd.github.raw")?;
                    let head = if about.is_empty() { format!("{stars} stars") } else { format!("{about} ({stars} stars)") };
                    Some(Extra { kind: "readme", title: name.clone(), source: format!("https://github.com/{name}"), text: format!("{head}\n\n{}", readme.trim()) })
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    })
}

fn excerpt(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= EXCERPT { flat } else { flat.chars().take(EXCERPT).collect::<String>() + "…" }
}

/// Keeps the pages for one search and writes the part of the result that lists them. `web` is the server's result
/// list as already written, which goes on the combined page too.
pub fn publish(query: &str, web: &str, extras: Vec<Extra>) -> String {
    if extras.is_empty() {
        return String::new();
    }
    let mut s = store().lock().unwrap();
    s.0 += 1;
    let n = s.0;
    let mut pages = vec![];
    let mut combined = format!("Everything web_search found for \"{query}\", made by Scout on this computer.\n\n{web}");
    let mut listing = String::new();
    for (kind, heading) in [("docs", "Library docs (Context7)"), ("readme", "GitHub READMEs")] {
        let of_kind: Vec<&Extra> = extras.iter().filter(|e| e.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        listing += &format!("\n{heading}:\n");
        combined += &format!("\n\n# {heading}\n");
        for (i, e) in of_kind.iter().enumerate() {
            let path = format!("{kind}/{}", i + 1);
            let url = page_url(n, &path);
            listing += &format!("\n- {}\n  {url} (from {})\n  > {}\n", e.title, e.source, excerpt(&e.text));
            combined += &format!("\n## {} ({})\n\n{}\n", e.title, e.source, e.text);
            pages.push((path, format!("{} ({})", e.title, e.source), e.text.clone()));
        }
    }
    pages.insert(0, (String::new(), format!("Search: {query}"), combined));
    s.1.push_back(Search { n, pages });
    while s.1.len() > KEPT {
        s.1.pop_front();
    }
    format!(
        "{listing}\nThe scout:// addresses are pages Scout keeps on this computer (not real websites). Read one with web_fetch, or {} for all of it.\n",
        page_url(n, "")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_kept_and_read_back() {
        let extras = vec![Extra { kind: "readme", title: "a/b".into(), source: "https://github.com/a/b".into(), text: "hello readme".into() }];
        let listing = publish("q", "web results", extras);
        let n = store().lock().unwrap().0;
        assert!(listing.contains(&format!("scout://search/{n}/readme/1")));
        assert_eq!(read(&format!("scout://search/{n}/readme/1")).unwrap().1, "hello readme");
        assert!(read(&format!("scout://search/{n}")).unwrap().1.contains("web results"));
        assert!(read("scout://search/999999").is_err());
        assert!(read("scout://evil.com/x").is_err());
    }

    #[test]
    fn scheme_check_is_exact() {
        assert!(is_scout_url("scout://search/1"));
        assert!(is_scout_url("SCOUT://search/1"));
        assert!(!is_scout_url("https://scout.dev"));
        assert!(!is_scout_url("scout:/x"));
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn live_sources() {
        let d = super::context7("tauri window", 2);
        let g = super::github("tauri window vibrancy", 2);
        for e in d.iter().chain(&g) {
            println!("{} | {} | {} chars", e.kind, e.title, e.text.len());
        }
        println!("{}", super::publish("tauri window", "web", d.into_iter().chain(g).collect()));
    }
}
