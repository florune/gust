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
    /// More about the row, such as a declaration's signature.
    pub detail: String,
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

/// A `line` or `line:column` after the last colon of `word`: zero-based.
fn line_and_column(word: &str) -> Option<(&str, u32, u32)> {
    let word = word.trim_end_matches([')', ',', '.', ';', '\'', '"']);
    let number = |text: &str| -> Option<u32> {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (rest, last) = word.rsplit_once(':')?;
    let last = number(last)?;
    if let Some((path, line)) = rest.rsplit_once(':')
        && let Some(line) = number(line)
        && !path.is_empty()
    {
        return Some((path, line.saturating_sub(1), last.saturating_sub(1)));
    }
    (!rest.is_empty()).then(|| (rest, last.saturating_sub(1), 0))
}

/// A query as found in logs and stack traces, `kafka/consumer.go:159` or
/// `/app/kafka/consumer.go:159 +0x1d`: the path, and the line to go to.
pub fn split_location(query: &str) -> (String, Option<(u32, u32)>) {
    for word in query.split_whitespace() {
        // A drive letter alone is not a path with a line.
        if let Some((path, row, col)) = line_and_column(word)
            && path.len() > 1
        {
            return (path.replace('\\', "/"), Some((row, col)));
        }
    }
    (query.split_whitespace().collect(), None)
}

/// Files whose path ends the way `query` does, most folders in common first.
fn by_suffix<'a>(files: &'a [String], query: &str) -> Vec<&'a String> {
    let wanted: Vec<&str> = query
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let Some(name) = wanted.last() else {
        return Vec::new();
    };
    let mut ranked: Vec<(usize, &String)> = files
        .iter()
        .filter_map(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            if !parts.last()?.eq_ignore_ascii_case(name) {
                return None;
            }
            let common = parts
                .iter()
                .rev()
                .zip(wanted.iter().rev())
                .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
                .count();
            Some((common, path))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
    let best = ranked.first().map_or(0, |(common, _)| *common);
    // With folders in common, only the files that share the most of them.
    ranked
        .into_iter()
        .filter(|(common, _)| best <= 1 || *common == best)
        .map(|(_, path)| path)
        .collect()
}

/// The `limit` files that best answer `query`; with no query, the first files.
///
/// A query with a line, such as `kafka/consumer.go:159`, finds the file by
/// how its path ends and puts the line on each result.
pub fn find_files(files: &[String], query: &str, limit: usize) -> Vec<Hit> {
    let (query, location) = split_location(query);
    let query = query.trim_start_matches("./").to_string();
    let hit = |path: &String| {
        let (row, col) = location.unwrap_or((0, 0));
        Hit {
            path: path.clone(),
            row,
            col,
            // Shown after the name: the line it opens at.
            preview: location.map_or(String::new(), |(row, _)| format!(":{}", row + 1)),
            tag: "",
            detail: String::new(),
        }
    };
    if query.is_empty() {
        return files.iter().take(limit).map(hit).collect();
    }
    if query.contains('/') || location.is_some() {
        let found = by_suffix(files, &query);
        if !found.is_empty() {
            return found.into_iter().take(limit).map(hit).collect();
        }
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
                detail: String::new(),
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

    #[test]
    fn a_path_with_a_line_finds_the_file_by_its_ending() {
        let files: Vec<String> = [
            "internal/kafka/consumer.go",
            "internal/rabbit/consumer.go",
            "cmd/kafka/main.go",
        ]
        .map(String::from)
        .to_vec();
        let found = |query: &str| -> Vec<(String, u32, u32)> {
            find_files(&files, query, 10)
                .into_iter()
                .map(|hit| (hit.path, hit.row, hit.col))
                .collect()
        };
        let kafka = ("internal/kafka/consumer.go".to_string(), 158, 0);
        assert_eq!(found("kafka/consumer.go:159"), [kafka.clone()]);
        // A server's own absolute path, as in a Go panic.
        assert_eq!(found("\t/app/internal/kafka/consumer.go:159 +0x1d"), [kafka.clone()]);
        assert_eq!(
            found(r"internal\kafka\consumer.go:159:7"),
            [("internal/kafka/consumer.go".to_string(), 158, 6)]
        );
        // Only the name: every file of that name.
        assert_eq!(found("consumer.go:3").len(), 2);
        // Without a line, the usual loose search.
        assert_eq!(found("kafmain")[0].0, "cmd/kafka/main.go");
    }

    #[test]
    fn locations_are_read_from_log_lines() {
        assert_eq!(
            split_location("kafka/consumer.go:159"),
            ("kafka/consumer.go".into(), Some((158, 0)))
        );
        assert_eq!(split_location("at main.go:12:5,"), ("main.go".into(), Some((11, 4))));
        assert_eq!(
            split_location(r"C:\work\main.go:3"),
            ("C:/work/main.go".into(), Some((2, 0)))
        );
        assert_eq!(split_location("consumer"), ("consumer".into(), None));
    }


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
