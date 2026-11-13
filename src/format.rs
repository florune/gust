use std::path::Path;
use std::sync::OnceLock;

use lsp_types::{Position, Range, TextEdit};

use crate::proc;

/// Format `text` for `language`, or `Ok(None)` when the language has no formatter.
///
/// An `Err` carries a one-line reason fit for the status bar.
pub fn format(language: &str, text: &str, dir: Option<&Path>) -> Result<Option<String>, String> {
    match language {
        "go" => format_go(text, dir).map(Some),
        "json" => format_json(text).map(Some),
        _ => Ok(None),
    }
}

/// `goimports` when it is installed (it also fixes the import block), else `gofmt`.
fn go_formatter() -> &'static str {
    static FORMATTER: OnceLock<&'static str> = OnceLock::new();
    FORMATTER.get_or_init(|| {
        if proc::on_path("goimports") {
            "goimports"
        } else {
            "gofmt"
        }
    })
}

fn format_go(text: &str, dir: Option<&Path>) -> Result<String, String> {
    let formatter = go_formatter();
    let output = proc::run_with_stdin(formatter, &[], dir, text)
        .map_err(|err| format!("无法运行 {formatter}：{err}"))?;
    if output.status.success() {
        return String::from_utf8(output.stdout).map_err(|_| "格式化结果不是 UTF-8".to_string());
    }
    // gofmt reports `<standard input>:12:5: expected …`, one error per line.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = stderr.lines().next().unwrap_or("格式化失败");
    Err(first
        .trim_start_matches("<standard input>:")
        .trim()
        .to_string())
}

/// Re-indent JSON without parsing its values, so numbers, key order, and
/// escapes come out exactly as they went in. `//` and `/* */` comments are kept.
pub fn format_json(text: &str) -> Result<String, String> {
    const INDENT: &str = "  ";
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut stack: Vec<char> = Vec::new();
    let mut chars = text.chars().peekable();
    // Whether the container just opened is still empty, so `{}` stays on one line.
    let mut just_opened = false;

    fn newline(out: &mut String, depth: usize) {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(INDENT);
        }
    }

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => continue,
            '{' | '[' => {
                if just_opened {
                    newline(&mut out, stack.len());
                }
                out.push(ch);
                stack.push(ch);
                just_opened = true;
                continue;
            }
            '}' | ']' => {
                let open = stack.pop().ok_or("JSON 括号不匹配")?;
                if (open == '{') != (ch == '}') {
                    return Err("JSON 括号不匹配".into());
                }
                if !just_opened {
                    newline(&mut out, stack.len());
                }
                out.push(ch);
            }
            ',' => {
                out.push(',');
                newline(&mut out, stack.len());
                just_opened = false;
                continue;
            }
            ':' => out.push_str(": "),
            '"' => {
                if just_opened {
                    newline(&mut out, stack.len());
                }
                out.push('"');
                let mut closed = false;
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => {
                            closed = true;
                            break;
                        }
                        _ => {}
                    }
                }
                if !closed {
                    return Err("JSON 字符串没有结束".into());
                }
            }
            '/' if matches!(chars.peek(), Some('/') | Some('*')) => {
                if just_opened {
                    newline(&mut out, stack.len());
                }
                out.push('/');
                if chars.next() == Some('*') {
                    out.push('*');
                    let mut previous = ' ';
                    for c in chars.by_ref() {
                        out.push(c);
                        if previous == '*' && c == '/' {
                            break;
                        }
                        previous = c;
                    }
                } else {
                    out.push('/');
                    while let Some(c) = chars.next_if(|c| *c != '\n') {
                        out.push(c);
                    }
                }
                newline(&mut out, stack.len());
                just_opened = false;
                continue;
            }
            _ => {
                if just_opened {
                    newline(&mut out, stack.len());
                }
                out.push(ch);
            }
        }
        just_opened = false;
    }
    if !stack.is_empty() {
        return Err("JSON 括号不匹配".into());
    }
    let mut out = out.trim_end().to_string();
    out.push('\n');
    Ok(out)
}

/// The line-level edits that turn `old` into `new`, last edit first.
///
/// The editor applies edits one at a time against the live text; ordering them
/// from the bottom up keeps every remaining position valid. Applying edits
/// instead of replacing the whole text keeps the caret, scroll, and undo
/// history where the user left them.
pub fn line_edits(old: &str, new: &str) -> Vec<TextEdit> {
    let diff = similar::TextDiff::from_lines(old, new);
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut edits = Vec::new();
    for op in diff.ops() {
        if matches!(op, similar::DiffOp::Equal { .. }) {
            continue;
        }
        let old_range = op.old_range();
        let new_range = op.new_range();
        edits.push(TextEdit {
            range: Range {
                start: Position::new(old_range.start as u32, 0),
                end: Position::new(old_range.end as u32, 0),
            },
            new_text: new_lines[new_range].concat(),
        });
    }
    edits.reverse();
    edits
}

/// Where a caret at `position` in `old` belongs in `new`.
///
/// On a line the formatter left alone the column is kept; on one it rewrote,
/// the distance to the end of the line is kept, since formatting mostly
/// changes indentation.
pub fn map_position(old: &str, new: &str, position: Position) -> Position {
    let diff = similar::TextDiff::from_lines(old, new);
    let line = position.line as usize;
    let width = |text: &str, line: usize| {
        text.split('\n')
            .nth(line)
            .map_or(0, |line| line.encode_utf16().count() as u32)
    };
    for op in diff.ops() {
        let (old_range, new_range) = (op.old_range(), op.new_range());
        if !old_range.contains(&line) {
            continue;
        }
        if matches!(op, similar::DiffOp::Equal { .. }) {
            let new_line = new_range.start + (line - old_range.start);
            return Position::new(new_line as u32, position.character);
        }
        if new_range.is_empty() {
            return Position::new(new_range.start as u32, 0);
        }
        let new_line = new_range.start + (line - old_range.start).min(new_range.len() - 1);
        let from_end = width(old, line).saturating_sub(position.character);
        return Position::new(
            new_line as u32,
            width(new, new_line).saturating_sub(from_end),
        );
    }
    // Past the last line the diff knows about: keep the distance to the end.
    let lines = |text: &str| text.split('\n').count();
    let new_line = (line + lines(new)).saturating_sub(lines(old));
    Position::new(new_line as u32, position.character.min(width(new, new_line)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caret_follows_its_line_through_formatting() {
        let old = "package a\n\n\n\nfunc  f( ) {\nx:=1\n}\n";
        let new = "package a\n\nfunc f() {\n\tx := 1\n}\n";
        // On an untouched line: same column, shifted up by the removed lines.
        assert_eq!(map_position(old, new, Position::new(6, 1)), Position::new(4, 1));
        // On a rewritten line: same distance from its end.
        assert_eq!(map_position(old, new, Position::new(5, 4)), Position::new(3, 7));
        assert_eq!(map_position(old, new, Position::new(5, 1)), Position::new(3, 4));
        assert_eq!(map_position(old, new, Position::new(0, 3)), Position::new(0, 3));
    }

    #[test]
    fn json_is_reindented_without_touching_values() {
        let formatted = format_json(r#"{"b":1.0,"a":[1,2,{"x":"a\"b, {}"}],"e":{},"n":1e10}"#);
        assert_eq!(
            formatted.unwrap(),
            "{\n  \"b\": 1.0,\n  \"a\": [\n    1,\n    2,\n    {\n      \"x\": \"a\\\"b, {}\"\n    }\n  ],\n  \"e\": {},\n  \"n\": 1e10\n}\n"
        );
    }

    #[test]
    fn json_with_unbalanced_brackets_is_rejected() {
        assert!(format_json("{\"a\": [1, 2}").is_err());
        assert!(format_json("{\"a\": 1").is_err());
        assert!(format_json("{\"a").is_err());
    }

    #[test]
    fn formatted_json_is_stable() {
        let once = format_json("[ {\"a\" : true , \"b\":null} , [] ]").unwrap();
        assert_eq!(format_json(&once).unwrap(), once);
    }

    /// Apply edits the way the editor does: one at a time, in the given order.
    fn apply(old: &str, edits: &[TextEdit]) -> String {
        let mut text = old.to_string();
        for edit in edits {
            let offset = |position: &Position| -> usize {
                text.split_inclusive('\n')
                    .take(position.line as usize)
                    .map(str::len)
                    .sum::<usize>()
                    + position.character as usize
            };
            let range = offset(&edit.range.start)..offset(&edit.range.end);
            text.replace_range(range, &edit.new_text);
        }
        text
    }

    #[test]
    fn line_edits_rebuild_the_new_text() {
        let old = "package a\n\nfunc  f( ) {\nreturn\n}\n\nvar x=1\n";
        let new = "package a\n\nfunc f() {\n\treturn\n}\n\nvar x = 1\n";
        let edits = line_edits(old, new);
        assert_eq!(edits.len(), 2);
        assert_eq!(apply(old, &edits), new);
    }

    #[test]
    fn line_edits_handle_a_missing_final_newline() {
        let old = "a\nb";
        let new = "a\nb\nc\n";
        assert_eq!(apply(old, &line_edits(old, new)), new);
    }
}
