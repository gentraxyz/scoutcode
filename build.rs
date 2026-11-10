// Puts the icon and version info into the .exe, and the web UI's permissions into the app. Also records the git commit
// the exe was built from (SCOUT_COMMIT), shown in Settings and signed into the release manifest, so a release can be
// traced to its source.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=SCOUT_COMMIT");
    let commit = std::env::var("SCOUT_COMMIT").ok().filter(|c| !c.is_empty()).or_else(|| {
        let head = git(&["rev-parse", "HEAD"])?;
        // Uncommitted changes to the app's own files mean the exe isn't exactly that commit.
        let dirty = git(&["status", "--porcelain", "--", "."]).is_some_and(|s| !s.is_empty());
        Some(if dirty { format!("{head}-dirty") } else { head })
    });
    // Rebuilt when HEAD moves, not on every edit: the release script checks for a clean tree itself.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{r}");
        }
        println!("cargo:rerun-if-changed={dir}/packed-refs");
    }
    println!("cargo:rustc-env=SCOUT_COMMIT={}", commit.unwrap_or_else(|| "unknown".into()));
    tauri_build::build();
}
