//! Editing whole lines, as GoLand's keys do: comment them, duplicate them,
//! delete them, move them up or down. Each takes the text and the selection
//! (byte ranges) and gives back the new text and what to select after.

use std::ops::Range;

#[derive(Debug, PartialEq)]
pub struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

/// The line comment of a language, if it has one.
pub fn comment_prefix(language: &str) -> Option<&'static str> {
    match language {
        "go" | "rust" | "javascript" | "typescript" | "proto" | "c" | "cpp" | "java" | "jsonc" => {
            Some("//")
        }
        "yaml" | "toml" | "bash" | "make" | "python" | "dockerfile" | "ini" | "env" => Some("#"),
        "sql" => Some("--"),
        _ => None,
    }
}

/// The byte range of whole lines the selection touches: from the start of
/// its first line to just past the newline of its last. A selection that
/// ends at the start of a line does not touch that line.
fn line_block(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = text[..selection.start].rfind('\n').map_or(0, |ix| ix + 1);
    let mut end = selection.end;
    if selection.end > selection.start && end > 0 && text.as_bytes()[end - 1] == b'\n' {
        end -= 1;
    }
    let end = text[end..].find('\n').map_or(text.len(), |ix| end + ix + 1);
    start..end.max(start)
}

fn column(text: &str, offset: usize) -> usize {
    offset - text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
}

/// The offset `col` bytes into the line starting at `line_start`, or its end.
fn at_column(text: &str, line_start: usize, col: usize) -> usize {
    let line_end = text[line_start..].find('\n').map_or(text.len(), |ix| line_start + ix);
    let mut at = (line_start + col).min(line_end);
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Comment the lines out, or back in when all of them are comments.
pub fn toggle_comment(text: &str, selection: Range<usize>, prefix: &str) -> Edit {
    let block = line_block(text, &selection);
    let lines: Vec<&str> = text[block.clone()].split_inclusive('\n').collect();
    let code = |line: &str| !line.trim().is_empty();
    let commented = lines.iter().filter(|line| code(line)).all(|line| line.trim_start().starts_with(prefix))
        && lines.iter().any(|line| code(line));
    let indent = lines
        .iter()
        .filter(|line| code(line))
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for line in &lines {
        if !code(line) {
            out.push_str(line);
        } else if commented {
            let at = line.find(prefix).unwrap_or(0);
            let mut rest = &line[at + prefix.len()..];
            rest = rest.strip_prefix(' ').unwrap_or(rest);
            out.push_str(&line[..at]);
            out.push_str(rest);
        } else {
            out.push_str(&line[..indent]);
            out.push_str(prefix);
            out.push(' ');
            out.push_str(&line[indent..]);
        }
    }
    let new_text = format!("{}{}{}", &text[..block.start], out, &text[block.end..]);
    let new_end = block.start + out.len();
    // Like GoLand: a caret moves on to the next line; a selection keeps its lines.
    let selection = if selection.is_empty() {
        let next = new_end.min(new_text.len());
        let col = column(text, selection.start);
        let caret = if next == new_text.len() && !new_text.ends_with('\n') {
            new_text.len()
        } else {
            at_column(&new_text, next, col)
        };
        caret..caret
    } else {
        let end = if out.ends_with('\n') { new_end - 1 } else { new_end };
        block.start..end
    };
    Edit {
        text: new_text,
        selection,
    }
}

/// Duplicate the selection, or the line the caret is on.
pub fn duplicate(text: &str, selection: Range<usize>) -> Edit {
    if !selection.is_empty() {
        let copy = &text[selection.clone()];
        let new_text = format!("{}{}{}", &text[..selection.end], copy, &text[selection.end..]);
        return Edit {
            text: new_text,
            selection: selection.end..selection.end + copy.len(),
        };
    }
    let block = line_block(text, &selection);
    let mut line = text[block.clone()].to_string();
    let mut insert_at = block.end;
    if !line.ends_with('\n') {
        // The last line has no newline: the copy brings one along.
        line.insert(0, '\n');
    }
    let new_text = format!("{}{}{}", &text[..insert_at], line, &text[insert_at..]);
    if !text[block.clone()].ends_with('\n') {
        insert_at += 1;
    }
    let caret = insert_at + column(text, selection.start);
    Edit {
        text: new_text,
        selection: caret..caret,
    }
}

/// Delete the lines the selection touches.
pub fn delete_lines(text: &str, selection: Range<usize>) -> Edit {
    let mut block = line_block(text, &selection);
    // The last line takes the newline before it.
    if block.end == text.len() && !text[block.clone()].ends_with('\n') && block.start > 0 {
        block.start -= 1;
    }
    let col = column(text, selection.start);
    let new_text = format!("{}{}", &text[..block.start], &text[block.end..]);
    let line_start = new_text[..block.start.min(new_text.len())]
        .rfind('\n')
        .map_or(0, |ix| ix + 1);
    let line_start = if block.start < new_text.len() && line_start <= block.start && text[..block.start].ends_with('\n') {
        block.start
    } else {
        line_start
    };
    let caret = at_column(&new_text, line_start.min(new_text.len()), col);
    Edit {
        text: new_text,
        selection: caret..caret,
    }
}

/// Move the lines the selection touches one line up or down.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<Edit> {
    let block = line_block(text, &selection);
    let mut lines = text[block.clone()].to_string();
    let (new_text, shift): (String, isize) = if up {
        if block.start == 0 {
            return None;
        }
        let above = text[..block.start - 1].rfind('\n').map_or(0, |ix| ix + 1);
        let mut previous = text[above..block.start].to_string();
        if !lines.ends_with('\n') {
            // Moving the last line up: the newline stays at the end.
            lines.push('\n');
            previous.pop();
        }
        let moved = format!("{}{}{}{}", &text[..above], lines, previous, &text[block.end..]);
        (moved, -((block.start - above) as isize))
    } else {
        if block.end >= text.len() {
            return None;
        }
        let below_end = text[block.end..].find('\n').map_or(text.len(), |ix| block.end + ix + 1);
        let mut next = text[block.end..below_end].to_string();
        if !next.ends_with('\n') {
            next.push('\n');
            lines.pop();
        }
        let moved = format!("{}{}{}{}", &text[..block.start], next, lines, &text[below_end..]);
        (moved, next.len() as isize)
    };
    let at = |offset: usize| (offset as isize + shift).max(0) as usize;
    Some(Edit {
        selection: at(selection.start).min(new_text.len())..at(selection.end).min(new_text.len()),
        text: new_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` for the caret, or `[` `]` around the selection.
    fn marked(source: &str) -> (String, Range<usize>) {
        if let Some(caret) = source.find('|') {
            return (source.replacen('|', "", 1), caret..caret);
        }
        let start = source.find('[').unwrap();
        let end = source.find(']').unwrap() - 1;
        (source.replacen('[', "", 1).replacen(']', "", 1), start..end)
    }

    fn show(edit: Edit) -> String {
        let mut text = edit.text;
        if edit.selection.is_empty() {
            text.insert(edit.selection.start, '|');
        } else {
            text.insert(edit.selection.end, ']');
            text.insert(edit.selection.start, '[');
        }
        text
    }

    #[test]
    fn comments_go_in_at_the_indentation_and_come_back_out() {
        let (text, sel) = marked("func f() {\n\ta|()\n\tb()\n}\n");
        let once = toggle_comment(&text, sel, "//");
        assert_eq!(show(toggle_comment(&text, 3..3, "//")), "// func f() {\n\ta(|)\n\tb()\n}\n");
        assert_eq!(show(once), "func f() {\n\t// a()\n\tb|()\n}\n");

        let (text, sel) = marked("x\n\t[a()\n\t\tb()]\n");
        let commented = toggle_comment(&text, sel, "//");
        assert_eq!(show(commented), "x\n[\t// a()\n\t// \tb()]\n");
        let (text, sel) = marked("x\n[\t// a()\n\t// \tb()]\n");
        assert_eq!(show(toggle_comment(&text, sel, "//")), "x\n[\ta()\n\t\tb()]\n");
    }

    #[test]
    fn duplicate_copies_the_line_or_the_selection() {
        let (text, sel) = marked("one\nt|wo\nthree");
        assert_eq!(show(duplicate(&text, sel)), "one\ntwo\nt|wo\nthree");
        let (text, sel) = marked("one\nthr|ee");
        assert_eq!(show(duplicate(&text, sel)), "one\nthree\nthr|ee");
        let (text, sel) = marked("a [bc] d");
        assert_eq!(show(duplicate(&text, sel)), "a bc[bc] d");
    }

    #[test]
    fn delete_removes_whole_lines() {
        let (text, sel) = marked("one\nt|wo\nthree\n");
        assert_eq!(show(delete_lines(&text, sel)), "one\nt|hree\n");
        let (text, sel) = marked("one\nthr|ee");
        assert_eq!(show(delete_lines(&text, sel)), "one|");
        let (text, sel) = marked("[one\ntwo\n]three\n");
        assert_eq!(show(delete_lines(&text, sel)), "|three\n");
    }

    #[test]
    fn lines_move_up_and_down_and_stop_at_the_ends() {
        let (text, sel) = marked("a\nb|\nc\n");
        assert_eq!(show(move_lines(&text, sel.clone(), true).unwrap()), "b|\na\nc\n");
        assert_eq!(show(move_lines(&text, sel, false).unwrap()), "a\nc\nb|\n");
        let (text, sel) = marked("a\nb\nc|");
        assert_eq!(show(move_lines(&text, sel, true).unwrap()), "a\nc|\nb");
        let (text, sel) = marked("a|\nb");
        assert_eq!(move_lines(&text, sel.clone(), true), None);
        assert_eq!(show(move_lines(&text, sel, false).unwrap()), "b\na|");
    }
}
