//! What the folder picker shows: the places to start from, and the folders inside one. Only folders, never files.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    name: String,
    path: String,
    icon: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Places {
    home: String,
    places: Vec<Place>,
    drives: Vec<Place>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    name: String,
    path: String,
    hidden: bool,
    /// Has a `.git` folder: likely a project.
    repo: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    path: String,
    parent: Option<String>,
    folders: Vec<Folder>,
    error: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn place(name: &str, path: &Path, icon: &'static str) -> Place {
    Place { name: name.to_string(), path: path.to_string_lossy().to_string(), icon }
}

pub fn places() -> Places {
    let home = home();
    let mut places = vec![place("Home", &home, "home")];
    for (name, icon) in [("Desktop", "desktop"), ("Documents", "files"), ("Downloads", "download")] {
        // OneDrive moves these when it backs them up.
        let found = [home.join(name), home.join("OneDrive").join(name)].into_iter().find(|p| p.is_dir());
        if let Some(p) = found {
            places.push(place(name, &p, icon));
        }
    }
    for name in ["Scout", "source/repos", "Projects", "projects", "dev", "code", "src"] {
        let p = home.join(name);
        if p.is_dir() && !places.iter().any(|x| Path::new(&x.path) == p) {
            places.push(place(name.rsplit('/').next().unwrap_or(name), &p, "folder"));
        }
    }
    Places { home: home.to_string_lossy().to_string(), places, drives: drives() }
}

#[cfg(windows)]
fn drives() -> Vec<Place> {
    (b'A'..=b'Z')
        .filter_map(|l| {
            let root = format!("{}:\\", l as char);
            Path::new(&root).is_dir().then(|| Place { name: format!("{}:", l as char), path: root, icon: "drive" })
        })
        .collect()
}

#[cfg(not(windows))]
fn drives() -> Vec<Place> {
    vec![place("/", Path::new("/"), "drive")]
}

#[cfg(windows)]
fn is_hidden(e: &std::fs::DirEntry, name: &str) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    name.starts_with('.') || e.metadata().map(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0).unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden(_: &std::fs::DirEntry, name: &str) -> bool {
    name.starts_with('.')
}

pub fn list(dir: &str) -> Listing {
    let path = PathBuf::from(dir);
    let parent = path.parent().map(|p| p.to_string_lossy().to_string()).filter(|p| !p.is_empty());
    let shown = path.to_string_lossy().to_string();
    match std::fs::read_dir(&path) {
        Err(e) => Listing {
            path: shown,
            parent,
            folders: vec![],
            error: Some(match e.kind() {
                std::io::ErrorKind::NotFound => "This folder doesn't exist.".into(),
                std::io::ErrorKind::PermissionDenied => "Scout isn't allowed to open this folder.".into(),
                _ => e.to_string(),
            }),
        },
        Ok(rd) => {
            let mut folders: Vec<Folder> = rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let p = e.path();
                    Folder { hidden: is_hidden(&e, &name), repo: p.join(".git").exists(), path: p.to_string_lossy().to_string(), name }
                })
                .collect();
            folders.sort_by_key(|f| f.name.to_lowercase());
            Listing { path: shown, parent, folders, error: None }
        }
    }
}

/// Makes a folder inside `parent`. Returns its path.
pub fn create(parent: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("That isn't a name a folder can have.".into());
    }
    let p = Path::new(parent).join(name);
    if p.exists() {
        return Err("There's already something with that name here.".into());
    }
    std::fs::create_dir(&p).map_err(|e| e.to_string())?;
    Ok(p.to_string_lossy().to_string())
}
