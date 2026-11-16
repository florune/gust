//! Compile errors and `go vet` findings of a package, read from the `go`
//! tool's output. Nothing stays running: each check is one `go build`, and
//! `go vet` when it builds.

use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: PathBuf,
    /// Zero-based line and byte column.
    pub row: u32,
    pub col: u32,
    pub message: String,
    /// A compile error rather than a warning.
    pub error: bool,
}

/// The location at the start of `line` (`a/b.go:12:5: message`), and the message.
fn split_line(line: &str) -> Option<(&str, u32, u32, &str)> {
    let dot = line.find(".go:")?;
    let path = &line[..dot + 3];
    let mut rest = &line[dot + 4..];
    let number = |text: &mut &str| -> Option<u32> {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        let value = text[..digits].parse().ok()?;
        *text = &text[digits..];
        Some(value)
    };
    let row = number(&mut rest)?;
    let mut col = 1;
    if let Some(after) = rest.strip_prefix(':')
        && after.starts_with(|ch: char| ch.is_ascii_digit())
    {
        rest = after;
        col = number(&mut rest)?;
    }
    let message = rest.strip_prefix(':')?.trim();
    Some((path, row.saturating_sub(1), col.saturating_sub(1), message))
}

/// Problems in `go build` or `go vet` output; paths are read from `dir`.
pub fn parse(output: &str, dir: &Path, errors: bool) -> Vec<Problem> {
    let mut problems: Vec<Problem> = Vec::new();
    for line in output.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        // `go vet` reports a package that does not compile this way.
        let (line, error) = match line
            .strip_prefix("vet.exe: ")
            .or_else(|| line.strip_prefix("vet: "))
        {
            Some(rest) => (rest, true),
            None => (line, errors),
        };
        match split_line(line) {
            Some((path, row, col, message)) => {
                let path = Path::new(path);
                problems.push(Problem {
                    file: if path.is_absolute() { path.to_path_buf() } else { dir.join(path) },
                    row,
                    col,
                    message: message.to_string(),
                    error,
                });
            }
            // An indented line goes on with the problem before it.
            None if line.starts_with([' ', '\t']) => {
                if let Some(last) = problems.last_mut() {
                    last.message.push(' ');
                    last.message.push_str(line.trim());
                }
            }
            None => {}
        }
    }
    problems
}

fn go(dir: &Path, args: &[&str]) -> Option<(bool, String)> {
    for attempt in 0..3 {
        let output = crate::proc::command("go")
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        // On Windows another `go` working at the same time can hold an entry
        // of the build cache for a moment; that is not about the code.
        let busy = !output.status.success()
            && (text.contains("Access is denied") || text.contains("being used by another process"))
            && !text.contains(".go:");
        if busy && attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            continue;
        }
        return Some((output.status.success(), text));
    }
    None
}

/// The problems of the package in `package_dir`: compile errors, or when it
/// compiles, what `go vet` finds (which covers its tests too).
pub fn check(package_dir: &Path) -> Vec<Problem> {
    let Some(module) = package_dir.ancestors().find(|dir| dir.join("go.mod").is_file()) else {
        return Vec::new();
    };
    let relative = package_dir
        .strip_prefix(module)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let target = if relative.is_empty() { ".".to_string() } else { format!("./{relative}") };
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let Some((built, output)) = go(module, &["build", "-gcflags=-e", "-o", null, &target]) else {
        return Vec::new();
    };
    if !built {
        return parse(&output, module, true);
    }
    go(module, &["vet", &target])
        .map(|(_, output)| parse(&output, module, false))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_and_vet_output_become_problems() {
        let output = "# example.com/v/pkg\n\
            pkg\\b.go:4:9: undefined: undefinedThing\n\
            pkg\\b.go:7:16: declared and not used: unused\n\
            vet.exe: pkg\\t_test.go:3:1: missing return\n\
            pkg/a.go:6:14: cannot use x (variable of type int) as string value\n\
            \thave int\n";
        let dir = Path::new("/m");
        let problems = parse(output, dir, false);
        let short: Vec<(String, u32, u32, bool)> = problems
            .iter()
            .map(|p| {
                let file = p.file.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                (file, p.row, p.col, p.error)
            })
            .collect();
        assert_eq!(
            short,
            [
                ("pkg/b.go".to_string(), 3, 8, false),
                ("pkg/b.go".to_string(), 6, 15, false),
                ("pkg/t_test.go".to_string(), 2, 0, true),
                ("pkg/a.go".to_string(), 5, 13, false),
            ]
        );
        assert_eq!(problems[0].message, "undefined: undefinedThing");
        assert_eq!(
            problems[3].message,
            "cannot use x (variable of type int) as string value have int"
        );
    }

    #[test]
    fn a_real_package_is_checked() {
        let base = std::env::temp_dir().join(format!("gust-problems-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("pkg")).unwrap();
        std::fs::write(base.join("go.mod"), "module example.com/v\n\ngo 1.22\n").unwrap();
        std::fs::write(
            base.join("pkg/a.go"),
            "package pkg\n\nfunc B() int {\n\treturn missing\n}\n\nfunc C() { var unused int }\n",
        )
        .unwrap();
        let problems = check(&base.join("pkg"));
        let messages: Vec<&str> = problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(problems.len(), 2, "{messages:?}");
        assert!(problems.iter().all(|p| p.error && p.file.ends_with("a.go")));

        // Once it compiles, vet speaks.
        std::fs::write(
            base.join("pkg/a.go"),
            "package pkg\n\nimport \"fmt\"\n\nfunc A() { fmt.Printf(\"%d\", \"x\") }\n",
        )
        .unwrap();
        let problems = check(&base.join("pkg"));
        assert_eq!(problems.len(), 1);
        assert!(!problems[0].error);
        assert_eq!((problems[0].row, problems[0].col), (4, 23));
        let _ = std::fs::remove_dir_all(&base);
    }
}
