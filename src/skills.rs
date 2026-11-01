//! Skills: folders with a SKILL.md of instructions, kept in ~/.scoutcode/skills/<name>. Typing @name in a message sends
//! that skill along with it (see attach.rs). They come from the user's own files or from the skills.sh directory. A
//! skill is only ever text for the model: installing one runs nothing, and what it asks for goes through permissions.

use crate::api::http;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const MANIFEST: &str = "SKILL.md";
/// Where a skill came from, in its folder.
const SOURCE_FILE: &str = ".source";
const MAX_FILES: usize = 300;
const MAX_FILE_BYTES: u64 = 5_000_000;
const MAX_TOTAL_BYTES: u64 = 25_000_000;
/// The most of a SKILL.md sent with a message; the model can read the rest from the file.
const MAX_BODY_CHARS: usize = 100_000;
/// Supporting files named in a message.
const MAX_LISTED: usize = 50;
const SKIP_DIRS: [&str; 3] = [".git", "node_modules", "__pycache__"];
/// How much of a skill's description Settings and the picker show.
pub const SHORT_DESCRIPTION: usize = 140;

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    /// Its folder's name, which is also what comes after the @.
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    /// "owner/repo/skill" or "domain/skill" for one from the directory.
    pub source: Option<String>,
}

impl Skill {
    pub fn short_description(&self) -> String {
        let first = self.description.lines().next().unwrap_or("");
        if first.chars().count() <= SHORT_DESCRIPTION {
            return first.to_string();
        }
        format!("{}…", first.chars().take(SHORT_DESCRIPTION).collect::<String>().trim_end())
    }
}

pub fn root() -> PathBuf {
    crate::config::home().join("skills")
}

/// A name that's safe as a folder and works after an @: lower case, without spaces or path characters.
fn clean_name(s: &str) -> String {
    let out: String = s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    out.trim_matches('-').to_string()
}

/// `name` and `description` from a SKILL.md's frontmatter.
fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return (None, None);
    }
    let block: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let value = |key: &str| -> Option<String> {
        let at = block.iter().position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))?;
        let v = block[at][key.len() + 1..].trim();
        // A block scalar: the lines indented under it.
        if v.is_empty() || v.starts_with('>') || v.starts_with('|') {
            let more: Vec<&str> = block[at + 1..].iter().take_while(|l| l.starts_with([' ', '\t'])).map(|l| l.trim()).collect();
            return Some(more.join(" ")).filter(|s| !s.is_empty());
        }
        Some(v.trim_matches(|c| c == '"' || c == '\'').to_string())
    };
    (value("name"), value("description"))
}

fn read_skill(dir: &Path) -> Option<Skill> {
    let name = dir.file_name()?.to_string_lossy().to_string();
    if name.starts_with('.') {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let source = std::fs::read_to_string(dir.join(SOURCE_FILE)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some(Skill { name, description: frontmatter(&text).1.unwrap_or_default(), dir: dir.to_path_buf(), source })
}

fn list_in(root: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = std::fs::read_dir(root).into_iter().flatten().flatten().filter_map(|e| read_skill(&e.path())).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The installed skills, by name.
pub fn list() -> Vec<Skill> {
    list_in(&root())
}

/// The skills named with @ in `text`, once each, in the order they come.
fn mentioned_in(root: &Path, text: &str) -> Vec<Skill> {
    if !text.contains('@') {
        return vec![];
    }
    let installed = list_in(root);
    let mut out: Vec<Skill> = vec![];
    for word in text.split_whitespace() {
        let Some(w) = word.strip_prefix('@') else { continue };
        let w = w.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if let Some(s) = installed.iter().find(|s| s.name == w) {
            if !out.iter().any(|o| o.name == s.name) {
                out.push(s.clone());
            }
        }
    }
    out
}

/// How a skill goes in a message, like attach.rs's `<file>` blocks.
fn block(s: &Skill) -> String {
    let text = std::fs::read_to_string(s.dir.join(MANIFEST)).unwrap_or_default().replace("\r\n", "\n");
    let total = text.chars().count();
    let mut body: String = text.chars().take(MAX_BODY_CHARS).collect();
    if total > MAX_BODY_CHARS {
        body.push_str(&format!("\n\n(Cut at {MAX_BODY_CHARS} characters. The rest is in {MANIFEST} in this skill's folder.)"));
    }
    let mut files = vec![];
    let _ = collect(&s.dir, "", &mut files, false);
    let mut names: Vec<String> = files.into_iter().map(|(rel, _)| rel).filter(|r| r != MANIFEST && r != SOURCE_FILE).collect();
    names.sort();
    let more = names.len().saturating_sub(MAX_LISTED);
    names.truncate(MAX_LISTED);
    let list = if names.is_empty() {
        String::new()
    } else {
        format!("\n\nSupporting files in this skill's folder, to read when the instructions call for them:\n{}{}", names.join("\n"), if more > 0 { format!("\n… {more} more") } else { String::new() })
    };
    format!("<skill name=\"{}\" path=\"{}\">\nThe user asked for this skill with @{}. Follow it for this request. It doesn't change what needs the user's permission.\n\n{body}{list}\n</skill>", s.name, s.dir.display(), s.name)
}

/// The blocks for the skills `text` mentions.
pub fn blocks(text: &str) -> Vec<String> {
    mentioned_in(&root(), text).iter().map(block).collect()
}

/// Removes an installed skill. Only ever a folder directly under the skills folder.
pub fn remove(name: &str) -> Result<(), String> {
    remove_in(&root(), name)
}

fn remove_in(root: &Path, name: &str) -> Result<(), String> {
    if safe_rel(name).is_none_or(|n| n.contains('/')) {
        return Err("That isn't a skill's name.".into());
    }
    std::fs::remove_dir_all(root.join(name)).map_err(|e| format!("Could not remove {name}: {e}"))
}

// ---------- installing ----------

/// A relative path with forward slashes, if it stays inside its folder.
fn safe_rel(p: &str) -> Option<String> {
    let p = p.replace('\\', "/");
    if p.contains(':') || p.starts_with('/') {
        return None;
    }
    let path = Path::new(&p);
    path.components().all(|c| matches!(c, Component::Normal(_))).then(|| p.clone()).filter(|p| !p.is_empty())
}

/// Reads a folder's files into memory, keeping to the limits. Doesn't follow links, so a skill can't pull in other
/// files. With `read` false it only lists them.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>, read: bool) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                collect(&e.path(), &rel, out, read)?;
            }
        } else if kind.is_file() {
            if out.len() >= MAX_FILES {
                return Err(format!("That's more than {MAX_FILES} files, too many for a skill."));
            }
            let bytes = if read {
                if e.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
                    return Err(format!("{rel} is over {} MB, too large for a skill.", MAX_FILE_BYTES / 1_000_000));
                }
                std::fs::read(e.path()).map_err(|e| format!("Could not read {rel}: {e}"))?
            } else {
                vec![]
            };
            out.push((rel, bytes));
        }
    }
    Ok(())
}

/// Writes one skill to a temporary folder, then swaps it in, so a failure can't leave half of one.
fn install_one(root: &Path, name: &str, source: Option<&str>, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    if name.is_empty() {
        return Err("This skill has no usable name.".into());
    }
    let total: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
    if total > MAX_TOTAL_BYTES || files.len() > MAX_FILES {
        return Err(format!("{name} is too large for a skill."));
    }
    let temp = root.join(format!(".tmp-{name}-{}", crate::session::now_ms()));
    let write = || -> Result<(), String> {
        for (rel, bytes) in files {
            let rel = safe_rel(rel).ok_or_else(|| format!("{name} has a file with an unsafe path: {rel}"))?;
            let path = temp.join(&rel);
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("Could not write {rel}: {e}"))?;
        }
        if let Some(s) = source {
            std::fs::write(temp.join(SOURCE_FILE), s).map_err(|e| e.to_string())?;
        }
        let dest = root.join(name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| format!("Could not replace {name}: {e}"))?;
        }
        std::fs::rename(&temp, &dest).map_err(|e| e.to_string())
    };
    let r = write();
    if r.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    r
}

/// Installs every skill in a set of files: the folders with a SKILL.md, without any nested inside another. Returns
/// their names.
fn install_tree(root: &Path, files: Vec<(String, Vec<u8>)>, fallback: &str, source: Option<&str>) -> Result<Vec<String>, String> {
    let mut roots: Vec<String> = vec![];
    let mut dirs: Vec<String> = files
        .iter()
        .filter(|(p, _)| p.rsplit('/').next() == Some(MANIFEST))
        .map(|(p, _)| p.strip_suffix(MANIFEST).unwrap_or("").to_string())
        .collect();
    dirs.sort_by_key(String::len);
    for d in dirs {
        if !roots.iter().any(|r| d.starts_with(r.as_str())) {
            roots.push(d);
        }
    }
    if roots.is_empty() {
        return Err(format!("There's no {MANIFEST} in there, so it isn't a skill."));
    }
    let mut names = vec![];
    for r in &roots {
        let inside: Vec<(String, Vec<u8>)> = files.iter().filter_map(|(p, b)| p.strip_prefix(r.as_str()).map(|rel| (rel.to_string(), b.clone()))).collect();
        let manifest = inside.iter().find(|(p, _)| p == MANIFEST).map(|(_, b)| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
        let folder = r.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let name = [frontmatter(&manifest).0.unwrap_or_default().as_str(), folder, fallback].iter().map(|n| clean_name(n)).find(|n| !n.is_empty()).unwrap_or_default();
        install_one(root, &name, source, &inside)?;
        names.push(name);
    }
    Ok(names)
}

fn read_zip(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Could not open the zip: {e}"))?;
    let mut out = vec![];
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuses paths that leave the folder.
        let Some(rel) = entry.enclosed_name() else { continue };
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.split('/').any(|p| SKIP_DIRS.contains(&p) || p == "__MACOSX") {
            continue;
        }
        if out.len() >= MAX_FILES {
            return Err(format!("That's more than {MAX_FILES} files, too many for a skill."));
        }
        // By what comes out, not what the zip claims.
        let mut bytes = vec![];
        entry.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Could not read {rel}: {e}"))?;
        total += bytes.len() as u64;
        if bytes.len() as u64 > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err(format!("{rel} makes it too large for a skill."));
        }
        out.push((rel, bytes));
    }
    Ok(out)
}

/// Installs from what the user picked: a folder, a .zip, or a single SKILL.md. Their original is left alone.
pub fn upload(path: &Path) -> Result<Vec<String>, String> {
    upload_to(&root(), path)
}

fn upload_to(root: &Path, path: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if path.is_dir() {
        let mut files = vec![];
        collect(path, "", &mut files, true)?;
        return install_tree(root, files, &stem, None);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" | "skill" => install_tree(root, read_zip(path)?, &stem, None),
        "md" | "markdown" => {
            let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
            if meta.len() > MAX_FILE_BYTES {
                return Err("That file is too large for a skill.".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            // A file called SKILL.md is named by its folder unless its frontmatter says.
            let fallback = if stem.eq_ignore_ascii_case("skill") { path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default() } else { stem };
            install_tree(root, vec![(MANIFEST.to_string(), bytes)], &fallback, None)
        }
        _ => Err("Choose a skill's folder, a .zip, or a SKILL.md.".into()),
    }
}

// ---------- the skills.sh directory ----------

const DIRECTORY: &str = "https://skills.sh";
/// The well-known paths a site publishes skills under, newest first.
const WELL_KNOWN: [&str; 2] = [".well-known/agent-skills", ".well-known/skills"];

/// A skill listed in the directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// "owner/repo" on GitHub, or a domain.
    pub source: String,
    pub id: String,
    pub name: String,
    pub installs: u64,
}

impl Listing {
    fn from_json(v: &Value) -> Option<Listing> {
        let id = v["skillId"].as_str()?.to_string();
        Some(Listing { source: v["source"].as_str()?.to_string(), name: v["name"].as_str().unwrap_or(&id).to_string(), id, installs: v["installs"].as_u64().unwrap_or(0) })
    }

    /// What's saved with an installed skill.
    pub fn key(&self) -> String {
        format!("{}/{}", self.source, self.id)
    }

    /// Its page on skills.sh, where its contents and security audits are.
    pub fn url(&self) -> String {
        if self.source.contains('/') { format!("{DIRECTORY}/{}/{}", self.source, self.id) } else { format!("{DIRECTORY}/site/{}/{}", self.source, self.id) }
    }
}

fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, (u16, String)> {
    let res = http().get(url).config().timeout_global(Some(Duration::from_secs(60))).build().call().map_err(|e| (0, e.to_string()))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err((status, format!("HTTP {status}")));
    }
    res.into_body().with_config().limit(limit).read_to_vec().map_err(|e| (0, e.to_string()))
}

fn parse_listings(body: &[u8]) -> Result<Vec<Listing>, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("Couldn't read the directory's answer: {e}"))?;
    Ok(v["skills"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(Listing::from_json).collect())
}

/// The directory's matches for `query`, or its most installed skills if it's blank. These are the endpoints the
/// site and its `skills` CLI use; its documented /api/v1 only takes a Vercel token.
pub fn search(query: &str) -> Result<Vec<Listing>, String> {
    let q = query.trim();
    let fail = |(_, e): (u16, String)| format!("Couldn't reach skills.sh: {e}");
    if q.chars().count() < 2 {
        if !q.is_empty() {
            return Err("Search for at least two letters.".into());
        }
        let mut all = parse_listings(&fetch(&format!("{DIRECTORY}/api/skills/all-time/0"), 5_000_000).map_err(fail)?)?;
        all.truncate(30);
        return Ok(all);
    }
    let res = http().get(&format!("{DIRECTORY}/api/search")).query("q", q).query("limit", "30").config().timeout_global(Some(Duration::from_secs(30))).build().call().map_err(|e| format!("Couldn't reach skills.sh: {e}"))?;
    let status = res.status().as_u16();
    if status != 200 {
        return Err(format!("skills.sh answered HTTP {status}."));
    }
    let body = res.into_body().with_config().limit(5_000_000).read_to_vec().map_err(|e| e.to_string())?;
    parse_listings(&body)
}

/// Each path segment percent-encoded, for a URL.
fn encode_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Which folder in a repo's list of SKILL.md folders is the skill: the one named for it, or else the only one, or
/// else the one whose frontmatter names it (`read` gets a SKILL.md's text). "" is the repo's top level.
fn pick_folder(id: &str, mut dirs: Vec<String>, mut read: impl FnMut(&str) -> Option<String>) -> Option<String> {
    dirs.sort_by_key(String::len);
    let base = |d: &str| d.rsplit('/').next().unwrap_or("").to_lowercase();
    let want = id.to_lowercase();
    if let Some(d) = dirs.iter().find(|d| base(d) == want) {
        return Some(d.clone());
    }
    if dirs.len() == 1 {
        return dirs.pop();
    }
    dirs.into_iter().take(40).find(|d| read(d).and_then(|t| frontmatter(&t).0).is_some_and(|n| clean_name(&n) == clean_name(id)))
}

fn install_github(root: &Path, l: &Listing) -> Result<String, String> {
    let ok = |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !l.source.split('/').all(ok) || l.source.matches('/').count() != 1 {
        return Err(format!("{} isn't a GitHub repository.", l.source));
    }
    let tree: Value = {
        let url = format!("https://api.github.com/repos/{}/git/trees/HEAD?recursive=1", l.source);
        let body = fetch(&url, 50_000_000).map_err(|(status, e)| match status {
            403 | 429 => "GitHub's limit for anonymous downloads (60 an hour) is used up. Try again later.".to_string(),
            404 => format!("{} isn't on GitHub any more, or is private.", l.source),
            _ => format!("Couldn't reach GitHub: {e}"),
        })?;
        serde_json::from_slice(&body).map_err(|e| e.to_string())?
    };
    // Blobs only, which leaves out links, with their sizes.
    let blobs: Vec<(String, u64)> = tree["tree"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter(|e| e["type"] == "blob" && e["mode"] != "120000").filter_map(|e| Some((e["path"].as_str()?.to_string(), e["size"].as_u64().unwrap_or(0)))).collect();
    let dirs: Vec<String> = blobs.iter().filter(|(p, _)| p.rsplit('/').next() == Some(MANIFEST)).map(|(p, _)| p.strip_suffix(MANIFEST).unwrap_or("").trim_end_matches('/').to_string()).collect();
    let raw = |path: &str| format!("https://raw.githubusercontent.com/{}/HEAD/{}", l.source, encode_path(path));
    let found = pick_folder(&l.id, dirs, |d| {
        let path = if d.is_empty() { MANIFEST.to_string() } else { format!("{d}/{MANIFEST}") };
        fetch(&raw(&path), MAX_FILE_BYTES).ok().map(|b| String::from_utf8_lossy(&b).to_string())
    });
    let Some(folder) = found else {
        return Err(format!("Couldn't find {} in {}.", l.id, l.source));
    };
    let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
    let wanted: Vec<&(String, u64)> = blobs
        .iter()
        .filter(|(p, _)| {
            let Some(rel) = p.strip_prefix(prefix.as_str()) else { return false };
            // A skill at the top of a repo: not the whole repo, only the folders the skill format defines.
            !folder.is_empty() || rel == MANIFEST || ["references/", "scripts/", "assets/"].iter().any(|d| rel.starts_with(d))
        })
        .collect();
    if wanted.len() > MAX_FILES || wanted.iter().map(|(_, s)| *s).sum::<u64>() > MAX_TOTAL_BYTES || wanted.iter().any(|(_, s)| *s > MAX_FILE_BYTES) {
        return Err(format!("{} is too large to install.", l.id));
    }
    let mut files = vec![];
    for (path, _) in wanted {
        let bytes = fetch(&raw(path), MAX_FILE_BYTES).map_err(|(_, e)| format!("Couldn't download {path}: {e}"))?;
        files.push((path[prefix.len()..].to_string(), bytes));
    }
    let name = clean_name(&l.id);
    install_one(root, &name, Some(&l.key()), &files)?;
    Ok(name)
}

/// One skill's files from a site's index entry, whichever version of the format it's in.
fn well_known_files(base: &str, entry: &Value) -> Result<Vec<(String, Vec<u8>)>, String> {
    let name = entry["name"].as_str().unwrap_or("");
    let get = |url: &str| fetch(url, MAX_FILE_BYTES).map_err(|(_, e)| format!("Couldn't download {url}: {e}"));
    if let Some(list) = entry["files"].as_array() {
        if list.len() > MAX_FILES {
            return Err(format!("{name} has too many files."));
        }
        let mut files = vec![];
        for f in list.iter().filter_map(Value::as_str) {
            let rel = safe_rel(f).ok_or_else(|| format!("{name} lists a file with an unsafe path: {f}"))?;
            files.push((rel.clone(), get(&format!("{base}/{}/{}", encode_path(name), encode_path(&rel)))?));
        }
        return Ok(files);
    }
    if entry["type"] != "skill-md" {
        return Err(format!("{name} is published as an archive, which Scout can't install yet. Download it from its page, then upload it here."));
    }
    let url = entry["url"].as_str().unwrap_or("");
    let url = if url.starts_with("https://") {
        url.to_string()
    } else if url.contains("://") || url.starts_with("//") {
        return Err(format!("{name} isn't on a secure address."));
    } else {
        format!("{base}/{}", url.trim_start_matches('/'))
    };
    let bytes = get(&url)?;
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if entry["digest"].as_str() != Some(format!("sha256:{digest}").as_str()) {
        return Err(format!("{name} doesn't match the checksum its site gives, so it wasn't installed."));
    }
    Ok(vec![(MANIFEST.to_string(), bytes)])
}

fn install_site(root: &Path, l: &Listing) -> Result<String, String> {
    if l.source.is_empty() || !l.source.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Err(format!("{} isn't a site.", l.source));
    }
    for path in WELL_KNOWN {
        let base = format!("https://{}/{path}", l.source);
        // Sites without one answer with a web page, so what doesn't parse just means the next place.
        let Ok(body) = fetch(&format!("{base}/index.json"), 5_000_000) else { continue };
        let Ok(index) = serde_json::from_slice::<Value>(&body) else { continue };
        let Some(entry) = index["skills"].as_array().and_then(|s| s.iter().find(|e| e["name"].as_str() == Some(l.id.as_str()))) else { continue };
        let files = well_known_files(&base, entry)?;
        if !files.iter().any(|(p, _)| p == MANIFEST) {
            return Err(format!("{} has no {MANIFEST}.", l.id));
        }
        let name = clean_name(&l.id);
        install_one(root, &name, Some(&l.key()), &files)?;
        return Ok(name);
    }
    Err(format!("{} doesn't publish {} any more.", l.source, l.id))
}

/// Downloads a listed skill. Returns its name. An installed copy is replaced.
pub fn install(l: &Listing) -> Result<String, String> {
    let root = root();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    if l.source.contains('/') { install_github(&root, l) } else { install_site(&root, l) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scout-skills-{name}-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SKILL: &str = "---\nname: My Skill\ndescription: >-\n  Does things\n  well.\n---\n\n# Body\n";

    #[test]
    fn reads_frontmatter() {
        assert_eq!(frontmatter(SKILL), (Some("My Skill".into()), Some("Does things well.".into())));
        assert_eq!(frontmatter("---\r\nname: \"a\"\r\ndescription: 'b: c'\r\n---\r\n"), (Some("a".into()), Some("b: c".into())));
        assert_eq!(frontmatter("# no frontmatter"), (None, None));
    }

    #[test]
    fn refuses_paths_that_leave_the_folder() {
        for bad in ["../x", "a/../../x", "/etc/x", "C:/x", "C:x", "a\\..\\x", ""] {
            assert_eq!(safe_rel(bad), None, "{bad}");
        }
        assert_eq!(safe_rel("refs\\a.md").as_deref(), Some("refs/a.md"));
        let root = temp("unsafe");
        assert!(install_one(&root, "x", None, &[("../evil".into(), vec![])]).is_err());
        assert!(!root.parent().unwrap().join("evil").exists());
        // Nothing half done is left behind.
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        assert!(remove_in(&root, "../..").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn uploads_a_folder_a_file_and_a_zip() {
        let (root, src) = (temp("root"), temp("src"));
        // A folder, with its supporting files. A folder holding several skills installs each.
        let many = src.join("many");
        std::fs::create_dir_all(many.join("one/refs")).unwrap();
        std::fs::create_dir_all(many.join("two/node_modules/x")).unwrap();
        std::fs::write(many.join("one/SKILL.md"), SKILL).unwrap();
        std::fs::write(many.join("one/refs/a.md"), "ref").unwrap();
        std::fs::write(many.join("two/SKILL.md"), "plain, named by its folder").unwrap();
        std::fs::write(many.join("two/node_modules/x/SKILL.md"), "skipped").unwrap();
        assert_eq!(upload_to(&root, &many).unwrap(), vec!["my-skill", "two"]);
        assert_eq!(std::fs::read_to_string(root.join("my-skill/refs/a.md")).unwrap(), "ref");
        assert!(!root.join("two/node_modules").exists());
        // Uploading it again replaces it.
        std::fs::write(many.join("one/SKILL.md"), format!("{SKILL}v2")).unwrap();
        upload_to(&root, &many).unwrap();
        assert!(std::fs::read_to_string(root.join("my-skill/SKILL.md")).unwrap().ends_with("v2"));
        // A lone file.
        let single = src.join("reviewer.md");
        std::fs::write(&single, "review things").unwrap();
        assert_eq!(upload_to(&root, &single).unwrap(), vec!["reviewer"]);
        // A zip wrapped in a top folder, with a path trying to escape.
        let zip_path = src.join("pack.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (p, body) in [("pack/SKILL.md", "---\nname: zipped\n---\nz"), ("pack/scripts/run.py", "print()"), ("../../evil.txt", "no")] {
            z.start_file(p, opts).unwrap();
            std::io::Write::write_all(&mut z, body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        assert_eq!(upload_to(&root, &zip_path).unwrap(), vec!["zipped"]);
        assert!(root.join("zipped/scripts/run.py").exists());
        assert!(!root.parent().unwrap().join("evil.txt").exists());
        // Not skills.
        std::fs::create_dir_all(src.join("empty")).unwrap();
        assert!(upload_to(&root, &src.join("empty")).is_err());
        let names: Vec<String> = list_in(&root).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["my-skill", "reviewer", "two", "zipped"]);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(src);
    }

    #[test]
    fn finds_mentions_and_builds_their_blocks() {
        let root = temp("mention");
        install_one(&root, "review", None, &[(MANIFEST.into(), b"---\ndescription: d\n---\nrules".to_vec()), ("refs/a.md".into(), vec![])]).unwrap();
        install_one(&root, "other", None, &[(MANIFEST.into(), vec![])]).unwrap();
        let names = |t: &str| mentioned_in(&root, t).into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names("use @review, then @review and @other."), vec!["review", "other"]);
        assert!(names("mail me@review or @reviews or review or @src/review").is_empty());
        let b = block(&mentioned_in(&root, "@review")[0]);
        assert!(b.starts_with("<skill name=\"review\" path=\"") && b.ends_with("</skill>"), "{b}");
        assert!(b.contains("rules") && b.contains("refs/a.md") && !b.contains(".source"), "{b}");
        remove_in(&root, "other").unwrap();
        assert!(!root.join("other").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn picks_the_right_folder_in_a_repo() {
        let dirs = |d: &[&str]| d.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let none = |_: &str| None;
        assert_eq!(pick_folder("a", dirs(&["skills/b", ".claude/skills/a", "a"]), none).as_deref(), Some("a"));
        assert_eq!(pick_folder("x", dirs(&["only/one"]), none).as_deref(), Some("only/one"));
        assert_eq!(pick_folder("x", dirs(&[""]), none).as_deref(), Some(""));
        assert_eq!(pick_folder("x", dirs(&["a", "b"]), none), None);
        let by_name = |d: &str| (d == "b").then(|| "---\nname: X\n---".to_string());
        assert_eq!(pick_folder("x", dirs(&["a", "b"]), by_name).as_deref(), Some("b"));
    }

    #[test]
    fn reads_the_directorys_answer() {
        let l = parse_listings(br#"{"skills":[{"id":"o/r/s","source":"o/r","skillId":"s","name":"S","installs":5},{"source":"x.dev","skillId":"t","installs":1},{"bad":1}]}"#).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].key().as_str(), l[0].url().as_str(), l[0].installs), ("o/r/s", "https://skills.sh/o/r/s", 5));
        assert_eq!((l[1].name.as_str(), l[1].url().as_str()), ("t", "https://skills.sh/site/x.dev/t"));
    }

    #[test]
    fn well_known_v2_checks_its_checksum() {
        let entry = serde_json::json!({"name": "x", "type": "archive", "url": "x.tar.gz", "digest": "sha256:0"});
        assert!(well_known_files("https://x.dev", &entry).unwrap_err().contains("archive"));
        let entry = serde_json::json!({"name": "x", "type": "skill-md", "url": "http://x.dev/SKILL.md", "digest": "sha256:0"});
        assert!(well_known_files("https://x.dev", &entry).unwrap_err().contains("secure"));
    }

    /// The real directory and real downloads: `cargo test -- --ignored live_skills --nocapture`.
    #[test]
    #[ignore]
    fn live_skills_directory() {
        let root = temp("live");
        let top = search("").unwrap();
        println!("top: {:?}", top.iter().take(3).collect::<Vec<_>>());
        assert!(!top.is_empty());
        let found = search("find-skills").unwrap();
        let github = found.iter().find(|l| l.key() == "vercel-labs/skills/find-skills").expect("in the search results");
        assert_eq!(install_github(&root, github).unwrap(), "find-skills");
        // A site's skill, with supporting files.
        let site = Listing { source: "open.feishu.cn".into(), id: "lark-doc".into(), name: "lark-doc".into(), installs: 0 };
        assert_eq!(install_site(&root, &site).unwrap(), "lark-doc");
        let installed = list_in(&root);
        println!("{installed:#?}");
        assert_eq!(installed.len(), 2);
        assert!(installed.iter().all(|s| s.source.is_some() && !s.description.is_empty()));
        println!("{}", block(&installed[0]).chars().take(400).collect::<String>());
        let _ = std::fs::remove_dir_all(root);
    }
}
