//! Finding things in the project: files by name, and text in files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files larger than this are not searched for text.
const MAX_SEARCHED_BYTES: u64 = 1024 * 1024;

/// One row of results.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Path from the project root, with `/` separators.
    pub path: String,
    /// Zero-based line and byte column to put the caret at.
    pub row: u32,
    pub col: u32,
    /// The matching line, for text results.
    pub preview: String,
    /// A label shown with the row, such as "实现" for an implementation.
    pub tag: &'static str,
}

/// Every file in the project that is not ignored by git or hidden by the
/// project's settings, relative to `root`.
pub fn project_files(root: &Path, hidden: &[String]) -> Vec<String> {
    let hidden = hidden.to_vec();
    let walk_root = root.to_path_buf();
    let mut files: Vec<String> = ignore::WalkBuilder::new(root)
        // Dotfiles such as `.gitignore` and `.env.example` are part of a
        // project; only the repository's own store is not.
        .hidden(false)
        .filter_entry(move |entry| {
            if entry.file_name() == ".git" || entry.file_name() == crate::settings::LOCAL_DIR {
                return false;
            }
            let relative = entry
                .path()
                .strip_prefix(&walk_root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            relative.is_empty() || !crate::settings::is_hidden(&hidden, &relative)
        })
        .build()
        .flatten()
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?;
            Some(relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    files.sort();
    files
}

/// How well `path` answers `query` typed as a loose sequence of letters;
/// higher is better, `None` is no match.
fn score(path: &str, query: &str) -> Option<i32> {
    let name_start = path.rfind('/').map_or(0, |ix| ix + 1);
    let lower_path = path.to_lowercase();
    let lower_query = query.to_lowercase();

    // The file name matters most: a query found whole in it beats anything
    // spread over the folders.
    if let Some(at) = lower_path[name_start..].find(&lower_query) {
        let mut score = 1000 - at as i32 * 2 - (path.len() - name_start) as i32;
        if at == 0 {
            score += 200;
        }
        return Some(score);
    }
    if let Some(at) = lower_path.find(&lower_query) {
        return Some(500 - at as i32 - path.len() as i32 / 4);
    }

    // Otherwise the letters in order, rewarding runs and word starts.
    let path_chars: Vec<char> = lower_path.chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for wanted in lower_query.chars() {
        let found = (at..path_chars.len()).find(|&ix| path_chars[ix] == wanted)?;
        let word_start = found == 0 || matches!(path_chars[found - 1], '/' | '_' | '-' | '.');
        score += match previous {
            Some(previous) if previous + 1 == found => 12,
            _ if word_start => 10,
            _ => 1,
        };
        previous = Some(found);
        at = found + 1;
    }
    Some(score - path.len() as i32 / 4)
}

/// The `limit` files that best answer `query`; with no query, the first files.
pub fn find_files(files: &[String], query: &str, limit: usize) -> Vec<Hit> {
    let query: String = query.split_whitespace().collect();
    let hit = |path: &String| Hit {
        path: path.clone(),
        row: 0,
        col: 0,
        preview: String::new(),
        tag: "",
    };
    if query.is_empty() {
        return files.iter().take(limit).map(hit).collect();
    }
    let mut ranked: Vec<(i32, &String)> = files
        .iter()
        .filter_map(|path| Some((score(path, &query)?, path)))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    ranked.into_iter().take(limit).map(|(_, path)| hit(path)).collect()
}

/// Lines containing `query` in the project's text files, up to `limit`.
///
/// Letter case is ignored unless the query itself has an uppercase letter.
/// Returns early with what it has once `cancelled` is set.
pub fn find_text(
    root: &Path,
    files: &[String],
    query: &str,
    limit: usize,
    cancelled: &AtomicBool,
) -> Vec<Hit> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    let exact_case = query.chars().any(char::is_uppercase);
    let needle = if exact_case {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    for path in files {
        if cancelled.load(Ordering::Relaxed) || hits.len() >= limit {
            break;
        }
        let full: PathBuf = root.join(path);
        let small = std::fs::metadata(&full).is_ok_and(|meta| meta.len() <= MAX_SEARCHED_BYTES);
        if !small {
            continue;
        }
        // Whatever is not UTF-8 text is a binary file.
        let Some(text) = std::fs::read(&full)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };
        for (row, line) in text.lines().enumerate() {
            let col = if exact_case {
                line.find(&needle)
            } else {
                // Lowercasing can change byte lengths; ASCII lowercasing cannot.
                line.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
            };
            let Some(col) = col else {
                continue;
            };
            hits.push(Hit {
                path: path.clone(),
                row: row as u32,
                col: col as u32,
                preview: line.trim().chars().take(160).collect(),
                tag: "",
            });
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(hits: Vec<Hit>) -> Vec<String> {
        hits.into_iter().map(|hit| hit.path).collect()
    }

    fn files() -> Vec<String> {
        [
            "cmd/demo/main.go",
            "internal/server/server.go",
            "internal/server/server_test.go",
            "internal/store/main_store.go",
            "docs/maintenance.md",
            "go.mod",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn a_file_name_beats_a_folder_and_a_prefix_beats_the_middle() {
        assert_eq!(
            names(find_files(&files(), "main", 10)),
            [
                "cmd/demo/main.go",
                "internal/store/main_store.go",
                "docs/maintenance.md",
            ]
        );
        assert_eq!(
            names(find_files(&files(), "server", 2)),
            ["internal/server/server.go", "internal/server/server_test.go"]
        );
    }

    #[test]
    fn letters_in_order_find_a_path_across_its_folders() {
        assert_eq!(
            names(find_files(&files(), "isvt", 1)),
            ["internal/server/server_test.go"]
        );
        assert_eq!(names(find_files(&files(), "cdm", 1)), ["cmd/demo/main.go"]);
        assert!(find_files(&files(), "zzz", 10).is_empty());
    }

    #[test]
    fn no_query_lists_files_as_they_are() {
        assert_eq!(names(find_files(&files(), "  ", 2)), ["cmd/demo/main.go", "internal/server/server.go"]);
    }

    #[test]
    fn text_is_found_with_its_line_and_column_ignoring_git_ignored_files() {
        let root = std::env::temp_dir().join(format!("gust-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        // The `ignore` walker reads `.gitignore` only inside a repository.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::write(root.join("src/a.go"), "package a\n\n\tfunc Hello() {}\n").unwrap();
        std::fs::write(root.join("src/b.go"), "package a\n// say hello\n").unwrap();
        std::fs::write(root.join("build/out.go"), "hello\n").unwrap();
        std::fs::write(root.join("blob.bin"), [0xff, 0xfe, b'h', b'e', b'l', b'l', b'o']).unwrap();

        let files = project_files(&root, &[]);
        assert_eq!(project_files(&root, &["src".to_string()]), [".gitignore", "blob.bin"]);
        assert_eq!(files, [".gitignore", "blob.bin", "src/a.go", "src/b.go"]);

        let cancelled = AtomicBool::new(false);
        let found: Vec<(String, u32, u32, String)> =
            find_text(&root, &files, "hello", 10, &cancelled)
                .into_iter()
                .map(|hit| (hit.path, hit.row, hit.col, hit.preview))
                .collect();
        assert_eq!(
            found,
            [
                ("src/a.go".to_string(), 2, 6, "func Hello() {}".to_string()),
                ("src/b.go".to_string(), 1, 7, "// say hello".to_string()),
            ]
        );
        // An uppercase letter in the query makes the search exact.
        assert_eq!(find_text(&root, &files, "Hello", 10, &cancelled).len(), 1);
        assert_eq!(find_text(&root, &files, "hello", 1, &cancelled).len(), 1);

        cancelled.store(true, Ordering::Relaxed);
        assert!(find_text(&root, &files, "hello", 10, &cancelled).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
