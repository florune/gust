//! Run configurations: what to build, where to run it, and with what.
//!
//! A run builds the package into an executable with `go build`, then starts
//! that executable itself, so stopping it stops the program and not just a
//! `go run` wrapper that would leave it behind.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    pub name: String,
    /// The package to build, as `go build` takes it, from the project root:
    /// `.`, `./cmd/server`, or a single `main.go`.
    pub package: String,
    /// Where the program runs, from the project root; empty is the root.
    pub dir: String,
    /// Arguments for the program, written as in a shell.
    pub args: String,
    /// `KEY=VALUE` lines.
    pub env: String,
    /// Extra flags for `go build`, such as `-race` or `-tags dev`.
    pub build_flags: String,
    /// Where `go build` writes the executable; empty is a temporary file.
    pub output: String,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            name: "main".to_string(),
            package: ".".to_string(),
            dir: String::new(),
            args: String::new(),
            env: String::new(),
            build_flags: String::new(),
            output: String::new(),
        }
    }
}

/// A program and its arguments, ready to start.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    pub env: HashMap<String, String>,
}

impl Command {
    /// The command as it would be typed, for showing what runs.
    pub fn display(&self) -> String {
        let mut line = quote(&self.program);
        for arg in &self.args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        line
    }
}

fn quote(word: &str) -> String {
    if !word.is_empty() && !word.contains([' ', '\t', '"', '\'']) {
        word.to_string()
    } else {
        format!("\"{}\"", word.replace('"', "\\\""))
    }
}

/// Split a command line into words: whitespace separates them, and single
/// or double quotes keep spaces inside one; `\"` is a quote inside double quotes.
pub fn split_args(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('"'), '\\') if chars.peek() == Some(&'"') => {
                word.push('"');
                chars.next();
            }
            (Some(open), ch) if ch == open => quote = None,
            (Some(_), ch) => word.push(ch),
            (None, '"' | '\'') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, ch) => {
                word.push(ch);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// `KEY=VALUE` lines; blank lines and `#` comments are skipped.
pub fn parse_env(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim().trim_start_matches("export ").trim();
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            (!key.is_empty()).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

/// A path from the config, taken from the project root unless it is absolute.
fn from_root(root: &Path, path: &str) -> PathBuf {
    let path = path.trim();
    if path.is_empty() {
        root.to_path_buf()
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    }
}

impl RunConfig {
    /// The executable `go build` writes.
    pub fn executable(&self, root: &Path) -> PathBuf {
        if !self.output.trim().is_empty() {
            return from_root(root, &self.output);
        }
        let name: String = self
            .name
            .chars()
            .map(|ch| if ch.is_alphanumeric() || ch == '-' { ch } else { '_' })
            .collect();
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        std::env::temp_dir()
            .join("gust-build")
            .join(format!("{name}{suffix}"))
    }

    /// `go build` for this configuration, run in the project root.
    pub fn build(&self, root: &Path) -> Command {
        let mut args = vec!["build".to_string()];
        args.extend(split_args(&self.build_flags));
        args.push("-o".to_string());
        args.push(self.executable(root).to_string_lossy().into_owned());
        let package = self.package.trim();
        args.push(if package.is_empty() { "." } else { package }.to_string());
        Command {
            program: "go".to_string(),
            args,
            dir: root.to_path_buf(),
            env: parse_env(&self.env),
        }
    }

    /// The built program, in its working directory with its arguments.
    pub fn run(&self, root: &Path) -> Command {
        Command {
            program: self.executable(root).to_string_lossy().into_owned(),
            args: split_args(&self.args),
            dir: from_root(root, &self.dir),
            env: parse_env(&self.env),
        }
    }
}

/// The `file.go:12` reference in `line` that byte `at` falls in.
pub fn file_reference_at(line: &str, at: usize) -> Option<(String, u32, u32)> {
    let mut from = 0;
    while let Some(found) = line[from..].find(".go:") {
        let dot = from + found;
        let start = line[..dot]
            .rfind(|ch: char| ch.is_whitespace() || ch == '(' || ch == '"' || ch == '\'')
            .map_or(0, |ix| ix + 1);
        let end = line[dot + 4..]
            .find(|ch: char| !ch.is_ascii_digit() && ch != ':')
            .map_or(line.len(), |ix| dot + 4 + ix);
        if (start..end).contains(&at) {
            return file_reference(&line[start..end]);
        }
        from = dot + 4;
    }
    None
}

/// `file.go:12` or `file.go:12:5` in a line of output: the path as written,
/// and the zero-based row and byte column.
pub fn file_reference(line: &str) -> Option<(String, u32, u32)> {
    let at = line.find(".go:")?;
    let path_start = line[..at]
        .rfind(|ch: char| ch.is_whitespace() || ch == '(' || ch == '"' || ch == '\'')
        .map_or(0, |ix| ix + 1);
    let path = &line[path_start..at + 3];
    let mut numbers = line[at + 4..].split(':');
    let row: u32 = numbers
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    let col: u32 = numbers
        .next()
        .and_then(|col| {
            col.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(1);
    Some((path.to_string(), row.saturating_sub(1), col.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_split_like_a_shell_does() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("-port 8080  -v"), ["-port", "8080", "-v"]);
        assert_eq!(
            split_args(r#"-name "my game" -dir 'C:\some dir' -q "a \"b\"""#),
            ["-name", "my game", "-dir", r"C:\some dir", "-q", r#"a "b""#]
        );
        assert_eq!(split_args(r#"-empty "" x"#), ["-empty", "", "x"]);
    }

    #[test]
    fn environment_lines_become_variables() {
        let env = parse_env("# comment\nAPP_ENV=dev\n\n export PORT = 8080\nDSN=\"a=b c\"\nbroken\n");
        assert_eq!(env.len(), 3);
        assert_eq!(env["APP_ENV"], "dev");
        assert_eq!(env["PORT"], "8080");
        assert_eq!(env["DSN"], "a=b c");
    }

    #[test]
    fn a_config_builds_its_package_then_runs_the_executable_in_its_folder() {
        let root = Path::new("/work/game");
        let config = RunConfig {
            name: "server".into(),
            package: "./cmd/server".into(),
            dir: "configs".into(),
            args: "-c dev.yaml".into(),
            env: "APP_ENV=dev".into(),
            build_flags: "-race -tags dev".into(),
            output: "bin/server".into(),
        };
        let build = config.build(root);
        assert_eq!(build.program, "go");
        let output = root.join("bin/server").to_string_lossy().into_owned();
        assert_eq!(
            build.args,
            ["build", "-race", "-tags", "dev", "-o", output.as_str(), "./cmd/server"]
        );
        assert_eq!(build.dir, root);

        let run = config.run(root);
        assert_eq!(run.program, output);
        assert_eq!(run.args, ["-c", "dev.yaml"]);
        assert_eq!(run.dir, root.join("configs"));
        assert_eq!(run.env["APP_ENV"], "dev");
    }

    #[test]
    fn without_an_output_the_executable_goes_to_a_temporary_folder() {
        let config = RunConfig {
            name: "my server".into(),
            ..Default::default()
        };
        let exe = config.executable(Path::new("/work"));
        assert!(exe.starts_with(std::env::temp_dir()));
        assert!(exe.to_string_lossy().contains("my_server"));
    }

    #[test]
    fn compiler_output_points_at_files() {
        assert_eq!(
            file_reference("internal/game/room.go:359:12: undefined: addChip"),
            Some(("internal/game/room.go".to_string(), 358, 11))
        );
        assert_eq!(
            file_reference("    D:/work/main_test.go:14 +0x1d"),
            Some(("D:/work/main_test.go".to_string(), 13, 0))
        );
        assert_eq!(
            file_reference("panic at (C:\\x\\a.go:3)"),
            Some(("C:\\x\\a.go".to_string(), 2, 0))
        );
        assert_eq!(file_reference("no file here"), None);
        let line = "a.go:1 then b/c.go:20:3 end";
        assert_eq!(file_reference_at(line, 2), Some(("a.go".to_string(), 0, 0)));
        assert_eq!(file_reference_at(line, 14), Some(("b/c.go".to_string(), 19, 2)));
        assert_eq!(file_reference_at(line, 25), None);
        assert_eq!(file_reference("see go.mod:3"), None);
    }
}
