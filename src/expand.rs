//! Extend Selection, as GoLand's Ctrl+W: each step selects the next larger
//! piece of code around the selection. Go is read with tree-sitter, so the
//! steps follow its syntax: a name, the expression, what is inside the
//! brackets, the brackets with it, the statement, the block, the function.
//! Other files go by words, brackets, lines, and then everything.

use std::ops::Range;

use tree_sitter::{Node, Parser};

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The word around `offset`, if there is one.
fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_word(*ch))
        .last()
        .map_or(offset, |(ix, _)| ix);
    let end = text[offset..]
        .char_indices()
        .find(|(_, ch)| !is_word(*ch))
        .map_or(text.len(), |(ix, _)| offset + ix);
    (start < end).then_some(start..end)
}

/// `range` without the whitespace at its ends.
fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let slice = &text[range.clone()];
    let start = range.start + (slice.len() - slice.trim_start().len());
    let end = range.end - (slice.len() - slice.trim_end().len());
    if start <= end { start..end } else { range }
}

/// Whether `outer` holds `inner` and more.
fn grows(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end && outer.len() > inner.len()
}

/// The next larger range to select around `range` in `text`.
pub fn extend(language: &str, text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let range = range.start.min(text.len())..range.end.min(text.len());
    if range.is_empty()
        && let Some(word) = word_at(text, range.start)
    {
        return Some(word);
    }
    let next = if language == "go" {
        extend_go(text, &range)
    } else {
        extend_text(text, &range)
    };
    next.or_else(|| grows(&(0..text.len()), &range).then_some(0..text.len()))
}

const OPENERS: &[(&str, &str)] = &[("(", ")"), ("[", "]"), ("{", "}"), ("\"", "\""), ("`", "`"), ("'", "'")];

/// The ranges a node offers, smaller first: what is inside its brackets or
/// quotes, then the node itself.
fn steps(node: Node, text: &str) -> Vec<Range<usize>> {
    let mut steps = Vec::new();
    let count = node.child_count();
    if count >= 2
        && let (Some(first), Some(last)) = (node.child(0), node.child((count - 1) as u32))
    {
        let pair = (&text[first.byte_range()], &text[last.byte_range()]);
        if OPENERS.iter().any(|(open, close)| pair == (*open, *close)) {
            let inside = first.end_byte()..last.start_byte();
            steps.push(trimmed(text, inside.clone()));
            // Across lines, the whole lines between the brackets.
            let between = &text[inside.clone()];
            if let (Some(first_break), Some(last_break)) = (between.find('\n'), between.rfind('\n'))
                && first_break < last_break
            {
                steps.push(inside.start + first_break + 1..inside.start + last_break + 1);
            }
        }
    }
    // A string literal's quotes are part of its text, not children of their own.
    if matches!(node.kind(), "interpreted_string_literal" | "raw_string_literal" | "rune_literal")
        && node.byte_range().len() >= 2
    {
        steps.push(node.start_byte() + 1..node.end_byte() - 1);
    }
    steps.push(node.byte_range());
    // A statement or declaration goes on to its whole lines, comments above included.
    if node.kind().ends_with("_statement") || node.kind().ends_with("_declaration") {
        let start = text[..node.start_byte()].rfind('\n').map_or(0, |ix| ix + 1);
        let mut end = node.end_byte();
        if text[end..].starts_with('\n') {
            end += 1;
        }
        steps.push(start..end);
    }
    steps
}

fn extend_go(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    let tree = parser.parse(text, None)?;
    let mut node = Some(
        tree.root_node()
            .descendant_for_byte_range(range.start, range.end)?,
    );
    while let Some(current) = node {
        if let Some(step) = steps(current, text).into_iter().find(|step| grows(step, range)) {
            return Some(step);
        }
        node = current.parent();
    }
    None
}

/// The innermost brackets around `range`: where they open and close.
fn brackets_around(text: &str, range: &Range<usize>) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut depth = [0i32; 3];
    let kinds = [(b'(', b')'), (b'[', b']'), (b'{', b'}')];
    let mut ix = range.start;
    while ix > 0 {
        ix -= 1;
        for (kind, (open, close)) in kinds.iter().enumerate() {
            if bytes[ix] == *close {
                depth[kind] += 1;
            } else if bytes[ix] == *open {
                if depth[kind] == 0 {
                    // Its match after the range.
                    let mut level = 0;
                    for (at, byte) in bytes.iter().enumerate().skip(ix + 1) {
                        if *byte == *open {
                            level += 1;
                        } else if *byte == *close {
                            if level == 0 {
                                return (at >= range.end).then_some((ix, at));
                            }
                            level -= 1;
                        }
                    }
                    return None;
                }
                depth[kind] -= 1;
            }
        }
    }
    None
}

fn extend_text(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let line_start = text[..range.start].rfind('\n').map_or(0, |ix| ix + 1);
    let line_end = text[range.end..].find('\n').map_or(text.len(), |ix| range.end + ix);
    let mut steps = Vec::new();
    if let Some((open, close)) = brackets_around(text, range) {
        steps.push(trimmed(text, open + 1..close));
        steps.push(open..close + 1);
    }
    steps.push(trimmed(text, line_start..line_end));
    steps.push(line_start..(line_end + 1).min(text.len()));
    steps.sort_by_key(|step| step.len());
    steps.into_iter().find(|step| grows(step, range))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each selection Ctrl+W goes through from the caret at `|`.
    fn walk(language: &str, marked: &str, steps: usize) -> Vec<String> {
        let at = marked.find('|').unwrap();
        let text = marked.replacen('|', "", 1);
        let mut range = at..at;
        let mut out = Vec::new();
        for _ in 0..steps {
            let Some(next) = extend(language, &text, range.clone()) else {
                break;
            };
            out.push(text[next.clone()].to_string());
            range = next;
        }
        out
    }

    #[test]
    fn go_grows_by_syntax() {
        let source = "package p\n\nfunc f() {\n\tx := add(a, b|ee)\n\t_ = x\n}\n";
        assert_eq!(
            walk("go", source, 7),
            [
                "bee",
                "a, bee",
                "(a, bee)",
                "add(a, bee)",
                "x := add(a, bee)",
                "\tx := add(a, bee)\n",
                "\tx := add(a, bee)\n\t_ = x\n",
            ]
        );
    }

    #[test]
    fn go_strings_select_their_content_first() {
        let source = "package p\n\nvar s = \"hello wor|ld\"\n";
        assert_eq!(walk("go", source, 3), ["world", "hello world", "\"hello world\""]);
    }

    #[test]
    fn other_text_grows_by_brackets_then_lines() {
        let source = "{\n  \"name\": [1, 2|2, 3],\n  \"x\": 1\n}\n";
        assert_eq!(
            walk("json", source, 5),
            [
                "22",
                "1, 22, 3",
                "[1, 22, 3]",
                "\"name\": [1, 22, 3],",
                "  \"name\": [1, 22, 3],\n",
            ]
        );
    }
}
