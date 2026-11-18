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
    /// The command as typed after a prompt that names its folder, relative
    /// to `root`.
    pub fn display_in(&self, root: &Path) -> String {
        let folder = match self.dir.strip_prefix(root) {
            Ok(rest) if rest.as_os_str().is_empty() => root
                .file_name()
                .map_or(".".to_string(), |name| name.to_string_lossy().into_owned()),
            Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
            Err(_) => self.dir.to_string_lossy().into_owned(),
        };
        format!("{folder}> {}", self.display())
    }

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

/// Options of `go build` written with their value as the next word, `-tags dev`.
const BUILD_FLAGS_WITH_VALUE: &[&str] = &[
    "C", "p", "o", "asmflags", "buildmode", "compiler", "gccgoflags", "gcflags", "installsuffix",
    "ldflags", "mod", "modfile", "overlay", "pgo", "pkgdir", "tags", "toolexec", "covermode",
    "coverpkg", "exec",
];

/// Whether `flags` are options of `go build` and nothing else. Anything that
/// is not would be taken for a package to build, which fails in ways that
/// do not say why; this says what is wrong instead.
pub fn check_build_flags(flags: &str) -> Result<(), String> {
    let words = split_args(flags);
    let mut words = words.iter();
    while let Some(word) = words.next() {
        let Some(option) = word.strip_prefix("--").or_else(|| word.strip_prefix('-')) else {
            return Err(format!(
                "构建参数中的「{word}」不是 go build 的选项。要在启动时传给程序的参数，请填写在「程序参数」中。"
            ));
        };
        if option == "o" {
            return Err("构建参数不能包含 -o：可执行文件的位置请填写在「输出文件」中。".to_string());
        }
        if !option.contains('=') && BUILD_FLAGS_WITH_VALUE.contains(&option) && words.next().is_none() {
            return Err(format!("构建参数中的 -{option} 缺少取值。"));
        }
    }
    Ok(())
}

/// `env` with the variables of `overrides` in place of its own of the same
/// names; both are `KEY=VALUE` lines.
pub fn override_env(env: &str, overrides: &str) -> String {
    let replaced = parse_env(overrides);
    if replaced.is_empty() {
        return env.to_string();
    }
    let mut lines: Vec<&str> = env
        .lines()
        .filter(|line| {
            let key = line.split_once('=').map(|(key, _)| key.trim().trim_start_matches("export ").trim());
            !key.is_some_and(|key| replaced.contains_key(key))
        })
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let added: Vec<&str> = overrides
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && line.contains('='))
        .collect();
    lines.extend(added);
    lines.join("
")
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

/// Folders never searched for modules.
const SKIPPED: &[&str] = &["node_modules", "vendor", "testdata", "target"];

/// Whether `dir` holds a `package main` file.
fn has_main(dir: &Path) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.ends_with(".go")
            && !name.ends_with("_test.go")
            && std::fs::read_to_string(entry.path()).is_ok_and(|text| {
                text.lines().any(|line| line.trim() == "package main")
            })
    })
}

/// Modules below `dir`, nearest first, a few levels deep.
fn modules_below(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for dir in level {
            let mut children: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    !name.starts_with('.') && !SKIPPED.contains(&name.as_str())
                })
                .map(|entry| entry.path())
                .collect();
            children.sort();
            for child in children {
                if child.join("go.mod").is_file() {
                    found.push(child);
                } else {
                    next.push(child);
                }
            }
        }
        level = next;
    }
    found
}

/// `path` from `base`, as `go` takes it: `.`, `./cmd/server`.
fn relative_target(base: &Path, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_string(),
        Ok(rest) => format!("./{}", rest.to_string_lossy().replace('\\', "/")),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// The Go module that `package` (from the project root) belongs to, and the
/// package as named from inside it. The module is the nearest `go.mod` above
/// the package; when there is none, as in a repository that keeps its Go
/// code in a folder, the module below it with a `main` package, else the first.
pub fn module_for(root: &Path, package: &str) -> Option<(PathBuf, String)> {
    let package = package.trim();
    let path = from_root(root, if package.is_empty() { "." } else { package });
    let (dir, file) = if path.extension().is_some_and(|ext| ext == "go") {
        (path.parent()?.to_path_buf(), Some(path.clone()))
    } else {
        (path.clone(), None)
    };
    if let Some(module) = dir.ancestors().find(|dir| dir.join("go.mod").is_file()) {
        let target = relative_target(module, file.as_deref().unwrap_or(&dir));
        return Some((module.to_path_buf(), target));
    }
    let modules = modules_below(&dir);
    let module = modules
        .iter()
        .find(|module| has_main(module))
        .or(modules.first())?
        .clone();
    Some((module, ".".to_string()))
}

impl RunConfig {
    /// Where the program runs: the folder set, or else the root of its module.
    pub fn work_dir(&self, root: &Path) -> PathBuf {
        if !self.dir.trim().is_empty() {
            return from_root(root, &self.dir);
        }
        module_for(root, &self.package).map_or(root.to_path_buf(), |(module, _)| module)
    }

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

    /// `go build` for this configuration, run in the package's module.
    pub fn build(&self, root: &Path) -> Command {
        let mut args = vec!["build".to_string()];
        args.extend(split_args(&self.build_flags));
        args.push("-o".to_string());
        args.push(self.executable(root).to_string_lossy().into_owned());
        let package = self.package.trim();
        let (dir, target) = module_for(root, package).unwrap_or_else(|| {
            let package = if package.is_empty() { "." } else { package };
            (root.to_path_buf(), package.to_string())
        });
        args.push(target);
        Command {
            program: "go".to_string(),
            args,
            dir,
            env: parse_env(&self.env),
        }
    }

    /// The built program, in its working directory with its arguments.
    pub fn run(&self, root: &Path) -> Command {
        Command {
            program: self.executable(root).to_string_lossy().into_owned(),
            args: split_args(&self.args),
            dir: self.work_dir(root),
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

    /// A repository whose Go module is in a subfolder.
    fn repository(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("gust-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in [
            ("README.md", "readme"),
            ("ui/package.json", "{}"),
            ("server/go.mod", "module server\n"),
            ("server/main.go", "package main\n\nfunc main() {}\n"),
            ("server/cmd/cli/main.go", "package main\n\nfunc main() {}\n"),
            ("server/app/app.go", "package app\n"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    #[test]
    fn a_module_in_a_folder_is_built_from_there() {
        let root = repository("module");
        let module = root.join("server");
        // The project root has no go.mod: the module below it is found.
        assert_eq!(module_for(&root, "."), Some((module.clone(), ".".to_string())));
        assert_eq!(module_for(&root, ""), Some((module.clone(), ".".to_string())));
        // Inside the module, the package is named from the module.
        assert_eq!(
            module_for(&root, "./server/cmd/cli"),
            Some((module.clone(), "./cmd/cli".to_string()))
        );
        assert_eq!(
            module_for(&root, "server/cmd/cli/main.go"),
            Some((module.clone(), "./cmd/cli/main.go".to_string()))
        );

        let config = RunConfig {
            package: ".".into(),
            args: "server -c config/settings.yml".into(),
            ..Default::default()
        };
        let build = config.build(&root);
        assert_eq!(build.dir, module);
        assert_eq!(build.args.last().map(String::as_str), Some("."));
        assert_eq!(build.display_in(&root).split_once('>').unwrap().0, "server");
        // Left empty, the program runs in its module, where its config files are.
        assert_eq!(config.run(&root).dir, module);
        let elsewhere = RunConfig {
            dir: "ui".into(),
            ..config
        };
        assert_eq!(elsewhere.run(&root).dir, root.join("ui"));
        let _ = std::fs::remove_dir_all(&root);
    }


    #[test]
    fn arguments_are_split_like_a_shell_does() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("-port 8080  -v"), ["-port", "8080", "-v"]);
        assert_eq!(
            split_args(r#"-name "my app" -dir 'C:\some dir' -q "a \"b\"""#),
            ["-name", "my app", "-dir", r"C:\some dir", "-q", r#"a "b""#]
        );
        assert_eq!(split_args(r#"-empty "" x"#), ["-empty", "", "x"]);
    }

    #[test]
    fn build_flags_are_options_of_go_build_only() {
        assert!(check_build_flags("").is_ok());
        assert!(check_build_flags("-race -tags dev -ldflags \"-s -w\" -gcflags=all=-N").is_ok());
        let wrong = check_build_flags("server -c config/settings.test.yml").unwrap_err();
        assert!(wrong.contains("「server」") && wrong.contains("程序参数"));
        assert!(check_build_flags("-o out.exe").unwrap_err().contains("输出文件"));
        assert!(check_build_flags("-race -tags").unwrap_err().contains("-tags"));
    }

    #[test]
    fn overrides_replace_variables_of_the_same_name() {
        assert_eq!(override_env("APP_ENV=dev
PORT=8080", "PORT=8081"), "APP_ENV=dev
PORT=8081");
        assert_eq!(override_env("", "PORT=8081
DEBUG=1"), "PORT=8081
DEBUG=1");
        assert_eq!(override_env("PORT=8080", ""), "PORT=8080");
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
        let root = Path::new("/work/app");
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
            file_reference("internal/store/store.go:359:12: undefined: loadItem"),
            Some(("internal/store/store.go".to_string(), 358, 11))
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
