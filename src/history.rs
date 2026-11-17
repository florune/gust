//! Repository history: commits, branches, and what each commit changed, read
//! from `git`, and the lanes that draw the commit graph.

use std::path::Path;
use std::process::Stdio;

use crate::proc;

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Author time, in seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    pub refs: Vec<Ref>,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }
}

/// A name pointing at a commit, as shown beside it.
#[derive(Clone, Debug, PartialEq)]
pub enum Ref {
    /// The branch that is checked out.
    Head(String),
    /// HEAD without a branch.
    Detached,
    Local(String),
    Tag(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    /// Full ref name, such as `refs/heads/main`: what `git log` is given.
    pub refname: String,
    /// Short name, such as `main` or `origin/main`.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    /// The upstream's short name, for a local branch that has one.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// Which history to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every branch, local and remote.
    All,
    /// What is checked out.
    Head,
    /// One branch, by full ref name.
    Branch(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangedFile {
    /// Path in the commit, with `/` separators.
    pub path: String,
    /// The path before a rename or copy.
    pub old_path: Option<String>,
    /// `A`, `M`, `D`, `R`, `C`, or `T`.
    pub status: char,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub hash: String,
    pub message: String,
    pub committer: String,
    pub commit_time: i64,
    pub files: Vec<ChangedFile>,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = proc::command("git")
        .args(["-c", "core.quotepath=false", "--no-optional-locks"])
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("无法运行 git：{err}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("git 命令失败")
        .trim()
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string())
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// Up to `limit` commits of `scope`, newest first, children before parents.
pub fn log(root: &Path, scope: &Scope, limit: usize) -> Result<Vec<Commit>, String> {
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e";
    let limit = format!("--max-count={limit}");
    let mut args = vec!["log", "--topo-order", format, &limit];
    match scope {
        // A stash is a commit too, but not one anybody means by "all branches".
        Scope::All => args.extend(["--exclude=refs/stash", "--all"]),
        Scope::Head => args.push("HEAD"),
        Scope::Branch(refname) => args.push(refname),
    }
    args.push("--");
    match git(root, &args) {
        Ok(output) => Ok(parse_log(&output)),
        // A repository without a commit has no history to show.
        Err(reason) if reason.contains("does not have any commits") => Ok(Vec::new()),
        Err(reason) => Err(reason),
    }
}

fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split(RECORD)
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            if record.is_empty() {
                return None;
            }
            let mut fields = record.splitn(7, FIELD);
            let mut next = || fields.next().unwrap_or_default();
            let hash = next().to_string();
            let parents = next().split_whitespace().map(str::to_string).collect();
            let author = next().to_string();
            let email = next().to_string();
            let time = next().parse().unwrap_or(0);
            let refs = parse_refs(next());
            let subject = next().to_string();
            (!hash.is_empty()).then_some(Commit {
                hash,
                parents,
                author,
                email,
                time,
                subject,
                refs,
            })
        })
        .collect()
}

/// Parse `%D`: `HEAD -> main, origin/main, tag: v1.0`.
fn parse_refs(decoration: &str) -> Vec<Ref> {
    decoration
        .split(", ")
        .filter(|name| !name.is_empty())
        .filter_map(|name| {
            if let Some(branch) = name.strip_prefix("HEAD -> ") {
                return Some(Ref::Head(branch.to_string()));
            }
            if name == "HEAD" {
                return Some(Ref::Detached);
            }
            if let Some(tag) = name.strip_prefix("tag: ") {
                return Some(Ref::Tag(tag.to_string()));
            }
            // `origin/HEAD` only repeats which branch the remote considers default.
            if name.ends_with("/HEAD") {
                return None;
            }
            Some(Ref::Local(name.to_string()))
        })
        .collect()
}

/// Local and remote branches, local ones first.
pub fn branches(root: &Path) -> Vec<Branch> {
    let format = "--format=%(refname)%1f%(refname:short)%1f%(HEAD)%1f%(upstream:short)%1f%(upstream:track,nobracket)";
    let Ok(output) = git(root, &["for-each-ref", format, "refs/heads", "refs/remotes"]) else {
        return Vec::new();
    };
    let mut branches: Vec<Branch> = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(FIELD);
            let refname = fields.next()?.to_string();
            let name = fields.next()?.to_string();
            let current = fields.next()? == "*";
            let upstream = fields.next().filter(|u| !u.is_empty()).map(str::to_string);
            let track = fields.next().unwrap_or_default();
            if refname.ends_with("/HEAD") {
                return None;
            }
            let count = |word: &str| {
                track
                    .split(", ")
                    .find_map(|part| part.strip_prefix(word))
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0)
            };
            Some(Branch {
                remote: refname.starts_with("refs/remotes/"),
                refname,
                name,
                current,
                upstream,
                ahead: count("ahead "),
                behind: count("behind "),
            })
        })
        .collect();
    branches.sort_by(|a, b| (a.remote, &a.name).cmp(&(b.remote, &b.name)));
    branches
}

/// The full message and the changed files of one commit, against its first parent.
pub fn details(root: &Path, commit: &Commit) -> Result<Details, String> {
    let header = git(
        root,
        &["show", "-s", "--format=%cn%x1f%ct%x1f%B", &commit.hash, "--"],
    )?;
    let mut fields = header.splitn(3, FIELD);
    let committer = fields.next().unwrap_or_default().to_string();
    let commit_time = fields
        .next()
        .and_then(|time| time.trim().parse().ok())
        .unwrap_or(0);
    let message = fields.next().unwrap_or_default().trim_end().to_string();

    let names = match commit.parents.first() {
        Some(parent) => git(
            root,
            &["diff", "--name-status", "-M", "-z", parent, &commit.hash, "--"],
        )?,
        None => git(
            root,
            &["diff-tree", "--root", "-r", "--name-status", "-M", "-z", &commit.hash, "--"],
        )?,
    };
    Ok(Details {
        hash: commit.hash.clone(),
        message,
        committer,
        commit_time,
        files: parse_name_status(&names),
    })
}

/// Parse `--name-status -z`: a status, then one path, or two for a rename or copy.
pub(crate) fn parse_name_status(output: &str) -> Vec<ChangedFile> {
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        // `diff-tree` starts with the commit's own hash.
        if status.len() >= 40 && status.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let kind = status.chars().next().unwrap_or('M');
        let Some(first) = fields.next() else { break };
        if matches!(kind, 'R' | 'C') {
            let Some(second) = fields.next() else { break };
            files.push(ChangedFile {
                path: second.to_string(),
                old_path: Some(first.to_string()),
                status: kind,
            });
        } else {
            files.push(ChangedFile {
                path: first.to_string(),
                old_path: None,
                status: kind,
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The content of `path` as of `rev`, or `None` when it has no such file.
pub fn file_at(root: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let output = proc::command("git")
        .args(["-c", "core.quotepath=false", "--no-optional-locks", "show"])
        .arg(format!("{rev}:{path}"))
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Who last changed a line, and in which commit.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameEntry {
    pub hash: String,
    pub author: String,
    /// Author time, in seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
}

impl BlameEntry {
    /// A line changed since the last commit.
    pub fn uncommitted(&self) -> bool {
        self.hash.bytes().all(|byte| byte == b'0')
    }
}

/// One entry per line of the file.
pub type Blame = Vec<Option<std::sync::Arc<BlameEntry>>>;

/// Who last changed each line of `contents`, the text of `path` as it is in
/// the editor; lines edited since the last commit say so.
pub fn blame(root: &Path, path: &str, contents: &str) -> Option<Blame> {
    let output = proc::run_with_stdin(
        "git",
        &[
            "-c",
            "core.quotepath=false",
            "--no-optional-locks",
            "blame",
            "--porcelain",
            "--contents",
            "-",
            "--",
            path,
        ],
        Some(root),
        contents,
    )
    .ok()?;
    output
        .status
        .success()
        .then(|| parse_blame(&String::from_utf8_lossy(&output.stdout)))
}

/// Read `git blame --porcelain`: a header line per line of the file, with the
/// commit's details after the first line that names it.
fn parse_blame(output: &str) -> Blame {
    use std::collections::HashMap;
    use std::sync::Arc;

    let mut details: HashMap<String, BlameEntry> = HashMap::new();
    let mut lines: Vec<(usize, String)> = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for line in output.lines() {
        if line.starts_with('\t') {
            if let Some((hash, row)) = current.take() {
                lines.push((row, hash));
            }
            continue;
        }
        let mut words = line.splitn(2, ' ');
        let (key, value) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
        if key.len() == 40 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let row = value
                .split(' ')
                .nth(1)
                .and_then(|row| row.parse::<usize>().ok())
                .unwrap_or(1);
            details.entry(key.to_string()).or_insert_with(|| BlameEntry {
                hash: key.to_string(),
                author: String::new(),
                time: 0,
                summary: String::new(),
            });
            current = Some((key.to_string(), row.saturating_sub(1)));
            continue;
        }
        let Some((hash, _)) = &current else {
            continue;
        };
        let Some(entry) = details.get_mut(hash) else {
            continue;
        };
        match key {
            "author" => entry.author = value.to_string(),
            "author-time" => entry.time = value.parse().unwrap_or(0),
            "summary" => entry.summary = value.to_string(),
            _ => {}
        }
    }
    let shared: HashMap<String, Arc<BlameEntry>> = details
        .into_iter()
        .map(|(hash, entry)| (hash, Arc::new(entry)))
        .collect();
    let count = lines.iter().map(|(row, _)| row + 1).max().unwrap_or(0);
    let mut blame: Blame = vec![None; count];
    for (row, hash) in lines {
        blame[row] = shared.get(&hash).cloned();
    }
    blame
}

/// Bring remote branches up to date.
pub fn fetch(root: &Path) -> Result<(), String> {
    git(root, &["fetch", "--all", "--prune", "--quiet"]).map(|_| ())
}

// ---------------------------------------------------------------- graph

/// One line drawn in a commit's row of the graph. Lanes are columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// A branch passing by, top to bottom in its lane.
    Through { lane: usize, color: usize },
    /// From the top of `lane` into this row's commit.
    In { lane: usize, color: usize },
    /// From this row's commit down to the top of `lane` in the next row.
    Out { lane: usize, color: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    /// The lane the commit's dot is in.
    pub lane: usize,
    pub color: usize,
    pub segments: Vec<Segment>,
    /// How many lanes the row uses, which sets how wide the graph is drawn.
    pub width: usize,
}

/// Lay out the graph for commits listed children-first.
///
/// Each lane holds the commit it is waiting to reach. A commit takes the lane
/// waiting for it, or a new one; its first parent continues in that lane and
/// any other parent gets a lane of its own, so a branch keeps its column and
/// its color for as long as it runs.
pub fn graph(commits: &[Commit]) -> Vec<GraphRow> {
    // Lane contents: the hash awaited and the lane's color.
    let mut lanes: Vec<Option<(&str, usize)>> = Vec::new();
    let mut next_color = 0;
    let mut new_color = || {
        next_color += 1;
        next_color - 1
    };
    let free_lane = |lanes: &mut Vec<Option<(&str, usize)>>| {
        lanes.iter().position(Option::is_none).unwrap_or_else(|| {
            lanes.push(None);
            lanes.len() - 1
        })
    };

    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let hash = commit.hash.as_str();
        let mut segments = Vec::new();
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.is_some_and(|(h, _)| h == hash))
            .map(|(ix, _)| ix)
            .collect();
        let (lane, color) = match waiting.first() {
            Some(&lane) => (lane, lanes[lane].unwrap().1),
            None => {
                let lane = free_lane(&mut lanes);
                (lane, new_color())
            }
        };
        for (ix, slot) in lanes.iter().enumerate() {
            match slot {
                Some((h, c)) if *h == hash => segments.push(Segment::In { lane: ix, color: *c }),
                Some((_, c)) => segments.push(Segment::Through { lane: ix, color: *c }),
                None => {}
            }
        }
        let width_before = lanes.len();
        for ix in waiting {
            lanes[ix] = None;
        }

        for (nth, parent) in commit.parents.iter().enumerate() {
            let parent = parent.as_str();
            let existing = lanes
                .iter()
                .position(|slot| slot.is_some_and(|(h, _)| h == parent));
            // The first parent awaited further right: this lane keeps going and
            // the other bends into it at the parent, so main lines stay left.
            if nth == 0
                && lanes[lane].is_none()
                && existing.is_some_and(|existing| existing > lane)
            {
                lanes[lane] = Some((parent, color));
                segments.push(Segment::Out { lane, color });
                continue;
            }
            // A parent some lane already waits for: join that lane.
            if let Some(existing) = existing {
                let color = lanes[existing].unwrap().1;
                segments.push(Segment::Out { lane: existing, color });
                continue;
            }
            let (target, target_color) = if nth == 0 && lanes[lane].is_none() {
                (lane, color)
            } else {
                (free_lane(&mut lanes), new_color())
            };
            lanes[target] = Some((parent, target_color));
            segments.push(Segment::Out {
                lane: target,
                color: target_color,
            });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(GraphRow {
            lane,
            color,
            segments,
            width: width_before.max(lanes.len()).max(lane + 1),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_gives_each_line_its_commit() {
        let a = "a".repeat(40);
        let zero = "0".repeat(40);
        let output = [
            format!("{a} 1 1 2"),
            "author Ann".into(),
            "author-time 1700000000".into(),
            "summary First".into(),
            "filename f.go".into(),
            "\tline one".into(),
            format!("{a} 2 2"),
            "\tline two".into(),
            format!("{zero} 3 3 1"),
            "author Not Committed Yet".into(),
            "author-time 1800000000".into(),
            "summary Version of f.go from f.go".into(),
            "filename f.go".into(),
            "\tline three".into(),
        ]
        .join("\n");
        let blame = parse_blame(&output);
        assert_eq!(blame.len(), 3);
        let first = blame[0].as_ref().unwrap();
        assert_eq!((first.author.as_str(), first.time, first.summary.as_str()), ("Ann", 1700000000, "First"));
        assert!(std::sync::Arc::ptr_eq(first, blame[1].as_ref().unwrap()));
        assert!(blame[2].as_ref().unwrap().uncommitted());
        assert!(!first.uncommitted());
    }

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            subject: String::new(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn a_straight_history_stays_in_one_lane() {
        let rows = graph(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert!(rows.iter().all(|row| row.lane == 0 && row.width == 1));
        assert_eq!(rows[0].segments, [Segment::Out { lane: 0, color: 0 }]);
        assert_eq!(rows[1].segments, [Segment::In { lane: 0, color: 0 }, Segment::Out { lane: 0, color: 0 }]);
        assert_eq!(rows[2].segments, [Segment::In { lane: 0, color: 0 }]);
    }

    #[test]
    fn a_merged_branch_gets_its_own_lane_and_color_until_it_forks_off() {
        // m merges f into the line ... b; f and b both come from a.
        let rows = graph(&[
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ]);
        let lanes: Vec<(usize, usize)> = rows.iter().map(|row| (row.lane, row.color)).collect();
        assert_eq!(lanes, [(0, 0), (1, 1), (0, 0), (0, 0)]);
        assert_eq!(
            rows[0].segments,
            [Segment::Out { lane: 0, color: 0 }, Segment::Out { lane: 1, color: 1 }]
        );
        // f runs beside b's lane down to a, where it bends back in.
        assert_eq!(
            rows[1].segments,
            [
                Segment::Through { lane: 0, color: 0 },
                Segment::In { lane: 1, color: 1 },
                Segment::Out { lane: 1, color: 1 },
            ]
        );
        assert_eq!(
            rows[3].segments,
            [Segment::In { lane: 0, color: 0 }, Segment::In { lane: 1, color: 1 }]
        );
        assert_eq!(rows[1].width, 2);
    }

    #[test]
    fn unrelated_tips_start_lanes_side_by_side() {
        let rows = graph(&[commit("x", &["a"]), commit("y", &["a"]), commit("a", &[])]);
        assert_eq!((rows[0].lane, rows[1].lane, rows[2].lane), (0, 1, 0));
        assert_eq!(
            rows[2].segments,
            [Segment::In { lane: 0, color: 0 }],
            "y's lane already joined a's at y"
        );
    }

    #[test]
    fn decorations_name_branches_tags_and_head() {
        assert_eq!(
            parse_refs("HEAD -> main, origin/main, origin/HEAD, tag: v1.0, feature"),
            [
                Ref::Head("main".into()),
                Ref::Local("origin/main".into()),
                Ref::Tag("v1.0".into()),
                Ref::Local("feature".into()),
            ]
        );
        assert_eq!(parse_refs("HEAD"), [Ref::Detached]);
        assert!(parse_refs("").is_empty());
    }

    #[test]
    fn renames_keep_both_paths() {
        let files = parse_name_status("M\0a.go\0R100\0old.go\0new.go\0A\0b.go\0");
        assert_eq!(
            files,
            [
                ChangedFile { path: "a.go".into(), old_path: None, status: 'M' },
                ChangedFile { path: "b.go".into(), old_path: None, status: 'A' },
                ChangedFile { path: "new.go".into(), old_path: Some("old.go".into()), status: 'R' },
            ]
        );
    }

    /// A repository with history on two branches, made with the real `git`.
    struct Repo(std::path::PathBuf);

    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("gust-history-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let repo = Self(root);
            repo.git(&["init", "--quiet", "--initial-branch=main"]);
            repo.git(&["config", "user.email", "a@example.com"]);
            repo.git(&["config", "user.name", "Ann"]);
            repo.git(&["config", "core.autocrlf", "false"]);
            repo.commit("a.go", "package a\n", "first");
            repo.git(&["switch", "--quiet", "-c", "feature"]);
            repo.git(&["mv", "a.go", "b.go"]);
            repo.git(&["commit", "--quiet", "-m", "rename"]);
            repo.git(&["switch", "--quiet", "main"]);
            repo.commit("c.go", "package a\n", "second\n\nWith a body.");
            repo.git(&["tag", "v1"]);
            repo
        }

        fn git(&self, args: &[&str]) {
            git(&self.0, args).unwrap();
        }

        fn commit(&self, file: &str, content: &str, message: &str) {
            std::fs::write(self.0.join(file), content).unwrap();
            self.git(&["add", file]);
            self.git(&["commit", "--quiet", "-m", message]);
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn history_branches_and_details_come_from_git() {
        let repo = Repo::new();
        let subjects = |scope: &Scope| -> Vec<String> {
            log(&repo.0, scope, 100)
                .unwrap()
                .into_iter()
                .map(|commit| commit.subject)
                .collect()
        };
        assert_eq!(subjects(&Scope::Head), ["second", "first"]);
        assert_eq!(
            subjects(&Scope::Branch("refs/heads/feature".into())),
            ["rename", "first"]
        );
        let mut all = subjects(&Scope::All);
        all.sort();
        assert_eq!(all, ["first", "rename", "second"]);

        let head = &log(&repo.0, &Scope::Head, 1).unwrap()[0];
        assert_eq!(head.author, "Ann");
        assert_eq!(head.refs, [Ref::Head("main".into()), Ref::Tag("v1".into())]);

        let names: Vec<(String, bool)> = branches(&repo.0)
            .into_iter()
            .map(|branch| (branch.name, branch.current))
            .collect();
        assert_eq!(names, [("feature".to_string(), false), ("main".to_string(), true)]);

        let details_of = |scope: Scope| {
            let commit = log(&repo.0, &scope, 1).unwrap().remove(0);
            (details(&repo.0, &commit).unwrap(), commit)
        };
        let (second, second_commit) = details_of(Scope::Head);
        assert_eq!(second.message, "second\n\nWith a body.");
        assert_eq!(second.files, [ChangedFile { path: "c.go".into(), old_path: None, status: 'A' }]);
        assert_eq!(
            file_at(&repo.0, &second_commit.hash, "c.go").as_deref(),
            Some(&b"package a\n"[..])
        );

        let (rename, rename_commit) = details_of(Scope::Branch("refs/heads/feature".into()));
        assert_eq!(rename.files[0].old_path.as_deref(), Some("a.go"));
        // The old side of a rename is read under its old name.
        let parent = &rename_commit.parents[0];
        assert!(file_at(&repo.0, parent, "a.go").is_some());
        assert!(file_at(&repo.0, parent, "b.go").is_none());

        let first = log(&repo.0, &Scope::Head, 2).unwrap().remove(1);
        assert!(first.parents.is_empty());
        let root_details = details(&repo.0, &first).unwrap();
        assert_eq!(root_details.files.len(), 1);
        assert!(first.parents.is_empty());
    }
}
