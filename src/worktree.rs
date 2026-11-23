//! Git worktrees for new chats: when the user turns it on before the first message, the chat runs in its own checkout
//! on a new branch, at `<repo>/.scout/worktrees/<name>`, so its changes stay apart from the folder they opened.
//! A chat there still belongs to the repository's project.

use std::path::{Path, PathBuf};

/// Where the worktrees go, under the repository's own folder.
const DIR: [&str; 2] = [".scout", "worktrees"];

/// Whether `dir` is in a git repository: it or a folder above it has `.git` (a folder, or a file in a worktree).
pub fn in_repo(dir: &str) -> bool {
    !dir.is_empty() && Path::new(dir).ancestors().any(|a| a.join(".git").exists())
}

/// The folder a Scout worktree was made from, when `dir` is one (`<repo>/.scout/worktrees/<name>`).
pub fn home_of(dir: &str) -> Option<String> {
    let p = Path::new(dir);
    let name = p.file_name()?;
    let parent = p.parent()?;
    let is = |q: &Path, n: &str| q.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n));
    if name.is_empty() || !is(parent, DIR[1]) {
        return None;
    }
    let scout = parent.parent()?;
    if !is(scout, DIR[0]) {
        return None;
    }
    scout.parent().map(|h| h.to_string_lossy().to_string())
}

/// The folder a chat in `dir` is listed under: the repository for a Scout worktree, else `dir` itself.
pub fn project_dir(dir: &str) -> String {
    home_of(dir).unwrap_or_else(|| dir.to_string())
}

/// Makes a worktree of the repository `dir` is in, on a new branch from what's checked out, and returns its folder.
pub fn create(dir: &str) -> Result<String, String> {
    let git = |cwd: &str, args: &[&str]| crate::prompt::git(cwd, args);
    let root = git(dir, &["rev-parse", "--show-toplevel"]).ok_or("this folder isn't in a git repository")?;
    // A worktree made from inside another one goes beside it, under the same repository.
    let root = home_of(&root).unwrap_or(root);
    let base: PathBuf = DIR.iter().fold(PathBuf::from(&root), |p, d| p.join(d));
    std::fs::create_dir_all(&base).map_err(|e| format!("couldn't make {}: {e}", base.display()))?;
    keep_out_of_git(&root);
    let stamp = format!("{:x}", crate::session::now_ms());
    let name = format!("scout-{}", &stamp[stamp.len().saturating_sub(6)..]);
    let path = base.join(&name);
    let branch = format!("scout/{name}");
    let path_s = path.to_string_lossy().to_string();
    let mut c = std::process::Command::new("git");
    c.args(["worktree", "add", "-b", &branch, &path_s]).current_dir(&root);
    crate::tools::no_window(&mut c);
    let out = c.output().map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "git worktree add failed".into() } else { err });
    }
    Ok(path_s)
}

/// Adds the worktrees' folder to the repository's own ignore list (`.git/info/exclude`), so they don't show as new
/// files in it. That list isn't committed.
fn keep_out_of_git(root: &str) {
    let Some(common) = crate::prompt::git(root, &["rev-parse", "--git-common-dir"]) else { return };
    let common = if Path::new(&common).is_absolute() { PathBuf::from(common) } else { Path::new(root).join(common) };
    let file = common.join("info").join("exclude");
    let line = "/.scout/worktrees/";
    let now = std::fs::read_to_string(&file).unwrap_or_default();
    if now.lines().any(|l| l.trim() == line) {
        return;
    }
    let _ = std::fs::create_dir_all(common.join("info"));
    let sep = if now.is_empty() || now.ends_with('\n') { "" } else { "\n" };
    let _ = std::fs::write(&file, format!("{now}{sep}{line}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_folders_belong_to_their_repo() {
        let sep = std::path::MAIN_SEPARATOR;
        let repo = format!("{sep}code{sep}app");
        let wt = format!("{repo}{sep}.scout{sep}worktrees{sep}scout-abc123");
        assert_eq!(home_of(&wt).as_deref(), Some(repo.as_str()));
        assert_eq!(project_dir(&wt), repo);
        assert_eq!(home_of(&repo), None);
        assert_eq!(home_of(&format!("{repo}{sep}worktrees{sep}x")), None);
    }

    #[test]
    fn makes_a_worktree_on_a_new_branch() {
        let dir = std::env::temp_dir().join(format!("scout-wt-test-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        let run = |a: &[&str]| assert!(std::process::Command::new("git").args(a).current_dir(&dir).output().unwrap().status.success());
        run(&["init", "-q"]);
        run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
        assert!(in_repo(&d));
        let wt = create(&d).unwrap();
        assert!(Path::new(&wt).join(".git").exists());
        assert!(session_same(&project_dir(&wt), &d));
        let exclude = std::fs::read_to_string(dir.join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.contains("/.scout/worktrees/"));
        let _ = std::process::Command::new("git").args(["worktree", "remove", "--force", &wt]).current_dir(&dir).output();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn session_same(a: &str, b: &str) -> bool {
        crate::session::same_dir(a, b)
    }
}
