//! Git worktrees: more checkouts of one repository, each on its own branch
//! in its own folder, so several changes can be worked on side by side.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::git::git;

#[derive(Clone, Debug, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    /// The branch checked out; `None` when the head is detached.
    pub branch: Option<String>,
    /// The repository's own folder, which the others were added to.
    pub main: bool,
    pub locked: bool,
    /// Its folder is gone; git forgets it at the next prune.
    pub prunable: bool,
}

impl Worktree {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

/// A path as git prints it, `C:/work/repo`, in this system's spelling.
fn native(path: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.replace('/', "\\"))
    } else {
        PathBuf::from(path)
    }
}

/// Whether two paths name the same folder, however each is spelled: short
/// or long names, either slash, any case on Windows.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let spell = |path: &Path| {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let text = path.to_string_lossy().replace('\\', "/");
        let text = text.trim_start_matches("//?/");
        let text = text.trim_end_matches('/').to_string();
        if cfg!(windows) { text.to_lowercase() } else { text }
    };
    spell(a) == spell(b)
}

/// The worktrees of the repository containing `dir`, the main one first.
pub fn list(dir: &Path) -> Vec<Worktree> {
    let Ok(output) = git(dir, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut out: Vec<Worktree> = Vec::new();
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(Worktree {
                path: native(path),
                branch: None,
                main: out.is_empty(),
                locked: false,
                prunable: false,
            });
            continue;
        }
        let Some(current) = out.last_mut() else {
            continue;
        };
        if let Some(branch) = line.strip_prefix("branch ") {
            current.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
        } else if line == "locked" || line.starts_with("locked ") {
            current.locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            current.prunable = true;
        }
    }
    // A bare repository lists itself first, with nothing checked out.
    out.retain(|worktree| !(worktree.main && worktree.branch.is_none() && !worktree.path.join(".git").exists()));
    out
}

/// What is going on in a worktree, compared with the branch it is based on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// Files with uncommitted changes.
    pub changes: usize,
    /// Commits of its own, and commits of the base it does not have.
    pub ahead: u32,
    pub behind: u32,
    /// When a file last changed, or the last commit when nothing is uncommitted.
    pub last: Option<SystemTime>,
    /// Its branch has not moved since it was made: nothing was done in it yet.
    pub fresh: bool,
}

impl Activity {
    /// Work was done in it, and the base has all of it.
    pub fn merged(&self) -> bool {
        self.changes == 0 && self.ahead == 0 && !self.fresh
    }
}

pub fn activity(worktree: &Path, base: &str) -> Activity {
    let mut out = Activity::default();
    let status = git(worktree, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
    let changed: Vec<&str> = status.lines().filter(|line| line.len() > 3).map(|line| &line[3..]).collect();
    out.changes = changed.len();
    out.last = changed
        .iter()
        .map(|path| path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"'))
        .filter_map(|path| std::fs::metadata(worktree.join(path)).ok()?.modified().ok())
        .max();
    if out.last.is_none() {
        out.last = git(worktree, &["log", "-1", "--format=%ct"])
            .ok()
            .and_then(|seconds| seconds.trim().parse::<u64>().ok())
            .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
    }
    // A branch's log holds one entry, its creation, until it moves.
    out.fresh = git(worktree, &["symbolic-ref", "--short", "HEAD"])
        .and_then(|branch| git(worktree, &["reflog", "show", "--format=%H", branch.trim(), "--"]))
        .is_ok_and(|entries| entries.lines().count() <= 1);
    let range = format!("{base}...HEAD");
    if let Ok(counts) = git(worktree, &["rev-list", "--left-right", "--count", &range]) {
        let mut parts = counts.split_whitespace().map(|count| count.parse().unwrap_or(0));
        out.behind = parts.next().unwrap_or(0);
        out.ahead = parts.next().unwrap_or(0);
    }
    out
}

/// Where a new worktree for `branch` goes by default: beside the repository,
/// not inside it, where the file tree, search, and the Go tools would see it.
pub fn default_location(main: &Path, branch: &str) -> PathBuf {
    let name = main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    let folder: String = branch
        .trim()
        .chars()
        .map(|ch| if ch == '/' || ch == '\\' || ch.is_whitespace() { '-' } else { ch })
        .collect();
    main.parent()
        .unwrap_or(main)
        .join(format!("{name}.worktrees"))
        .join(folder)
}

/// Check out `branch` in a new worktree at `path`; a branch that does not
/// exist yet is made from `base`.
pub fn add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<(), String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("请填写分支名称".to_string());
    }
    if path.exists() && std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(format!("{} 已存在且不是空文件夹", path.display()));
    }
    let target = path.to_string_lossy().into_owned();
    let exists = git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    if exists {
        git(repo, &["worktree", "add", &target, branch])
    } else {
        git(repo, &["worktree", "add", "-b", branch, &target, base])
    }
    .map(|_| ())
}

/// Take the worktree at `path` away; `force` drops its uncommitted changes.
/// Its branch is kept.
pub fn remove(repo: &Path, path: &Path, force: bool) -> Result<(), String> {
    let target = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&target);
    let removed = git(repo, &args).map(|_| ());
    let _ = git(repo, &["worktree", "prune"]);
    removed
}

/// Whether `name` matches `pattern`, where `*` stands for any run of characters.
fn matches(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || !name[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// Files of the worktree at `from` that git does not keep, whose names match
/// one of `patterns`: settings and secrets a checkout needs to run.
pub fn local_files(from: &Path, patterns: &[String]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for args in [
        &["ls-files", "--others", "--exclude-standard"][..],
        &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory"][..],
    ] {
        let Ok(output) = git(from, args) else {
            continue;
        };
        files.extend(
            output
                .lines()
                .filter(|path| !path.ends_with('/'))
                .filter(|path| {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    patterns.iter().any(|pattern| matches(pattern.trim(), name))
                })
                .map(str::to_string),
        );
    }
    files.sort();
    files.dedup();
    files
}

/// Copy `files`, paths from `from`, to the same places under `to`.
pub fn copy_files(from: &Path, to: &Path, files: &[String]) -> Result<usize, String> {
    for file in files {
        let target = to.join(file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("无法复制 {file}：{err}"))?;
        }
        std::fs::copy(from.join(file), &target).map_err(|err| format!("无法复制 {file}：{err}"))?;
    }
    Ok(files.len())
}

/// The commit the worktree's branch grew from: where it and `base` last met.
pub fn fork_point(worktree: &Path, base: &str) -> Option<String> {
    git(worktree, &["merge-base", base, "HEAD"]).ok().map(|hash| hash.trim().to_string())
}

/// Every file of the worktree that differs from commit `since`: what was
/// committed on its branch and what is not committed yet, new files included.
pub fn changed_since(worktree: &Path, since: &str) -> Vec<crate::history::ChangedFile> {
    let names = git(worktree, &["diff", "--name-status", "-M", "-z", since, "--"]).unwrap_or_default();
    let mut files = crate::history::parse_name_status(&names);
    let untracked = git(worktree, &["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        if !files.iter().any(|file| file.path == path) {
            files.push(crate::history::ChangedFile {
                path: path.to_string(),
                old_path: None,
                status: 'A',
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The subjects of the commits on `branch` that `base` does not have, oldest first.
pub fn subjects(repo: &Path, base: &str, branch: &str) -> Vec<String> {
    git(repo, &["log", "--reverse", "--format=%s", &format!("{base}..{branch}")])
        .map(|output| output.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// All of the branch's changes as one new commit on the base.
    Squash,
    /// A merge commit that keeps the branch's commits.
    Merge,
    /// The branch's commits replayed on the base, which then moves up to them.
    Rebase,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Merged {
    Done,
    /// Stopped with conflicts in the repository's folder, to be resolved there.
    Conflicts,
}

/// Bring `branch`, checked out in the worktree at `worktree`, into `base`,
/// checked out in `repo`. With `commit_first`, uncommitted changes in the
/// worktree are committed with that message before anything else.
pub fn merge(
    repo: &Path,
    worktree: &Path,
    branch: &str,
    strategy: Strategy,
    message: &str,
    commit_first: Option<&str>,
) -> Result<Merged, String> {
    if let Some(wip) = commit_first {
        git(worktree, &["add", "--all"])?;
        if git(worktree, &["diff", "--cached", "--quiet"]).is_err() {
            git(worktree, &["commit", "--quiet", "-m", wip])?;
        }
    }
    let conflicted = |repo: &Path| {
        git(repo, &["diff", "--name-only", "--diff-filter=U"]).is_ok_and(|names| !names.trim().is_empty())
    };
    match strategy {
        Strategy::Squash => {
            if let Err(error) = git(repo, &["merge", "--squash", branch]) {
                return if conflicted(repo) { Ok(Merged::Conflicts) } else { Err(error) };
            }
            if git(repo, &["diff", "--cached", "--quiet"]).is_ok() {
                return Err("没有需要合并的改动".to_string());
            }
            git(repo, &["commit", "--quiet", "-m", message])?;
        }
        Strategy::Merge => {
            if let Err(error) = git(repo, &["merge", "--no-ff", "-m", message, branch]) {
                return if conflicted(repo) { Ok(Merged::Conflicts) } else { Err(error) };
            }
        }
        Strategy::Rebase => {
            let base = git(repo, &["symbolic-ref", "--short", "HEAD"])?.trim().to_string();
            if let Err(error) = git(worktree, &["rebase", &base]) {
                let _ = git(worktree, &["rebase", "--abort"]);
                return Err(if conflicted(worktree) || error.contains("CONFLICT") || error.contains("conflict") {
                    "变基时有冲突，已撤销变基；请改用压缩或合并提交，在主仓库中解决冲突".to_string()
                } else {
                    error
                });
            }
            git(repo, &["merge", "--ff-only", branch])?;
        }
    }
    Ok(Merged::Done)
}

/// Delete a local branch; `force` also when git cannot tell it was merged,
/// as after a squash.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> Result<(), String> {
    git(repo, &["branch", if force { "-D" } else { "-d" }, branch]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap();
    }

    fn repo(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("gust-worktree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        run(&root, &["init", "--quiet", "--initial-branch=main"]);
        run(&root, &["config", "user.email", "test@example.com"]);
        run(&root, &["config", "user.name", "Test"]);
        run(&root, &["config", "core.autocrlf", "false"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
        run(&root, &["add", "."]);
        run(&root, &["commit", "--quiet", "-m", "first"]);
        root
    }

    #[test]
    fn patterns_match_names() {
        assert!(matches(".env", ".env"));
        assert!(matches(".env.*", ".env.local"));
        assert!(!matches(".env.*", ".env"));
        assert!(matches("*.local.*", "config.local.yaml"));
        assert!(!matches("*.local.*", "config.yaml"));
    }

    #[test]
    fn a_worktree_sits_beside_the_repository() {
        let main = Path::new("D:/work/gust");
        assert_eq!(default_location(main, "feat/login"), Path::new("D:/work/gust.worktrees/feat-login"));
    }

    #[test]
    fn worktrees_are_added_listed_and_removed() {
        let root = repo("cycle");
        let place = root.parent().unwrap().join("repo.worktrees").join("feature");
        std::fs::write(root.join(".env"), "KEY=1\n").unwrap();

        add(&root, &place, "feature", "main").unwrap();
        let found = list(&root);
        assert_eq!(found.len(), 2);
        assert!(found[0].main && same_path(&found[0].path, &root));
        assert_eq!(found[1].branch.as_deref(), Some("feature"));
        assert!(same_path(&found[1].path, &place));

        // Local settings come along; the worktree starts level with its base.
        let files = local_files(&root, &[".env".to_string()]);
        assert_eq!(files, [".env"]);
        copy_files(&root, &place, &files).unwrap();
        assert_eq!(std::fs::read_to_string(place.join(".env")).unwrap(), "KEY=1\n");
        let fresh = activity(&place, "main");
        assert!(fresh.fresh && !fresh.merged());

        // Work in it shows up, and keeps it from being removed without force.
        std::fs::write(place.join("a.txt"), "two\n").unwrap();
        run(&place, &["commit", "--quiet", "-am", "change"]);
        std::fs::write(place.join("b.txt"), "new\n").unwrap();
        let busy = activity(&place, "main");
        assert_eq!((busy.changes, busy.ahead, busy.behind, busy.fresh), (1, 1, 0, false));
        assert!(remove(&root, &place, false).is_err());
        remove(&root, &place, true).unwrap();
        assert_eq!(list(&root).len(), 1);
        assert!(!place.exists());

        // An existing branch is checked out rather than made again.
        add(&root, &place, "feature", "main").unwrap();
        assert_eq!(std::fs::read_to_string(place.join("a.txt")).unwrap(), "two\n");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn a_worktree_is_reviewed_and_merged_back_three_ways() {
        for (name, strategy) in [("squash", Strategy::Squash), ("merge", Strategy::Merge), ("rebase", Strategy::Rebase)] {
            let root = repo(&format!("merge-{name}"));
            let place = root.parent().unwrap().join("repo.worktrees").join("work");
            add(&root, &place, "work", "main").unwrap();
            std::fs::write(place.join("a.txt"), "one\ntwo\n").unwrap();
            run(&place, &["commit", "--quiet", "-am", "add two"]);
            std::fs::write(place.join("new.txt"), "fresh\n").unwrap();
            // Meanwhile the base moves on, in another file.
            std::fs::write(root.join("b.txt"), "base\n").unwrap();
            run(&root, &["add", "."]);
            run(&root, &["commit", "--quiet", "-m", "base work"]);

            let fork = fork_point(&place, "main").unwrap();
            let changed: Vec<(String, char)> =
                changed_since(&place, &fork).into_iter().map(|file| (file.path, file.status)).collect();
            assert_eq!(changed, [("a.txt".to_string(), 'M'), ("new.txt".to_string(), 'A')]);
            assert_eq!(subjects(&root, "main", "work"), ["add two"]);

            let merged = merge(&root, &place, "work", strategy, "work: done", Some("wip")).unwrap();
            assert_eq!(merged, Merged::Done, "{name}");
            assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\ntwo\n", "{name}");
            assert_eq!(std::fs::read_to_string(root.join("new.txt")).unwrap(), "fresh\n", "{name}");
            assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "base\n", "{name}");
            remove(&root, &place, false).unwrap();
            delete_branch(&root, "work", strategy == Strategy::Squash).unwrap();
            let _ = std::fs::remove_dir_all(root.parent().unwrap());
        }
    }

    #[test]
    fn conflicting_work_stops_in_the_repository() {
        let root = repo("merge-conflict");
        let place = root.parent().unwrap().join("repo.worktrees").join("work");
        add(&root, &place, "work", "main").unwrap();
        std::fs::write(place.join("a.txt"), "theirs\n").unwrap();
        run(&place, &["commit", "--quiet", "-am", "theirs"]);
        std::fs::write(root.join("a.txt"), "ours\n").unwrap();
        run(&root, &["commit", "--quiet", "-am", "ours"]);
        assert_eq!(merge(&root, &place, "work", Strategy::Squash, "m", None).unwrap(), Merged::Conflicts);
        assert!(std::fs::read_to_string(root.join("a.txt")).unwrap().contains("<<<<<<<"));
        let _ = git(&root, &["merge", "--abort"]);
        let _ = git(&root, &["reset", "--hard", "--quiet"]);
        // A rebase that conflicts is undone, leaving the worktree as it was.
        assert!(merge(&root, &place, "work", Strategy::Rebase, "m", None).is_err());
        assert_eq!(std::fs::read_to_string(place.join("a.txt")).unwrap(), "theirs\n");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }
}
