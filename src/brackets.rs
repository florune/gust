//! Brackets of a file: how deeply each is nested and which one closes it.
//! Brackets inside strings and comments are not counted.

use std::collections::HashMap;

/// How a language writes comments and strings, which brackets inside do not count in.
struct Syntax {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quotes whose strings end at the line's end, and whether `\` escapes in them.
    quotes: &'static [(u8, bool)],
    /// Quotes whose strings may span lines, with no escapes: Go's raw strings.
    raw_quotes: &'static [u8],
}

fn syntax(language: &str) -> Option<Syntax> {
    let c_like = |quotes, raw_quotes| Syntax {
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes,
        raw_quotes,
    };
    let hash = |quotes| Syntax {
        line_comments: &["#"],
        block_comment: None,
        quotes,
        raw_quotes: &[],
    };
    Some(match language {
        "go" => c_like(&[(b'"', true), (b'\'', true)], b"`"),
        "json" | "proto" => c_like(&[(b'"', true), (b'\'', true)], &[]),
        "yaml" | "toml" | "bash" | "make" => hash(&[(b'"', true), (b'\'', false)]),
        "sql" => Syntax {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: &[(b'\'', false), (b'"', false)],
            raw_quotes: &[],
        },
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bracket {
    /// Byte offset in the text.
    pub offset: usize,
    /// How many brackets enclose it; `None` when nothing closes it, or it closes nothing.
    pub depth: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Brackets {
    pub all: Vec<Bracket>,
    /// The other half of each matched bracket, both ways.
    partner: HashMap<usize, usize>,
}

fn closing(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        _ => b'}',
    }
}

impl Brackets {
    /// The brackets of `text` in a file of `language`; none for languages
    /// whose strings and comments are not known.
    pub fn scan(text: &str, language: &str) -> Self {
        let Some(syntax) = syntax(language) else {
            return Self::default();
        };
        let bytes = text.as_bytes();
        let starts = |at: usize, token: &str| bytes[at..].starts_with(token.as_bytes());
        let mut out = Self::default();
        let mut open: Vec<(u8, usize)> = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let byte = bytes[at];
            if syntax.line_comments.iter().any(|token| starts(at, token)) {
                at = text[at..].find('\n').map_or(bytes.len(), |end| at + end);
                continue;
            }
            if let Some((start, end)) = syntax.block_comment
                && starts(at, start)
            {
                at = text[at + start.len()..]
                    .find(end)
                    .map_or(bytes.len(), |found| at + start.len() + found + end.len());
                continue;
            }
            if syntax.raw_quotes.contains(&byte) {
                at = bytes[at + 1..]
                    .iter()
                    .position(|&b| b == byte)
                    .map_or(bytes.len(), |found| at + 1 + found + 1);
                continue;
            }
            if let Some(&(quote, escapes)) = syntax.quotes.iter().find(|(quote, _)| *quote == byte) {
                at += 1;
                while at < bytes.len() && bytes[at] != b'\n' {
                    if escapes && bytes[at] == b'\\' {
                        at += 2;
                        continue;
                    }
                    at += 1;
                    if bytes[at - 1] == quote {
                        break;
                    }
                }
                continue;
            }
            match byte {
                b'(' | b'[' | b'{' => {
                    open.push((byte, out.all.len()));
                    out.all.push(Bracket {
                        offset: at,
                        depth: None,
                    });
                }
                b')' | b']' | b'}' => {
                    let mut depth = None;
                    if let Some(&(kind, opener)) = open.last()
                        && closing(kind) == byte
                    {
                        open.pop();
                        depth = Some(open.len());
                        out.all[opener].depth = depth;
                        let start = out.all[opener].offset;
                        out.partner.insert(start, at);
                        out.partner.insert(at, start);
                    }
                    out.all.push(Bracket { offset: at, depth });
                }
                _ => {}
            }
            at += 1;
        }
        out
    }

    /// The matched pair at the caret: the bracket right after it, else the one right before.
    pub fn pair_at(&self, caret: usize) -> Option<(usize, usize)> {
        [Some(caret), caret.checked_sub(1)]
            .into_iter()
            .flatten()
            .find_map(|at| self.partner.get(&at).map(|other| (at.min(*other), at.max(*other))))
    }
}

#[cfg(test)]
mod tests {
    use super::Brackets;

    fn depths(text: &str, language: &str) -> Vec<(char, Option<usize>)> {
        Brackets::scan(text, language)
            .all
            .iter()
            .map(|bracket| (text.as_bytes()[bracket.offset] as char, bracket.depth))
            .collect()
    }

    #[test]
    fn nesting_gives_each_pair_one_depth() {
        assert_eq!(
            depths("f(a[1], {b})", "go"),
            [
                ('(', Some(0)),
                ('[', Some(1)),
                (']', Some(1)),
                ('{', Some(1)),
                ('}', Some(1)),
                (')', Some(0)),
            ]
        );
    }

    #[test]
    fn strings_and_comments_do_not_count() {
        let text = "x := \"(\" + `\n{` + ')' // )\n/* ] */ y()";
        assert_eq!(depths(text, "go"), [('(', Some(0)), (')', Some(0))]);
        assert_eq!(depths("a: \"{\" # ]\nb: [1]", "yaml"), [('[', Some(0)), (']', Some(0))]);
        assert!(depths("(text)", "text").is_empty());
    }

    #[test]
    fn brackets_without_a_partner_have_no_depth() {
        assert_eq!(depths("(]", "go"), [('(', None), (']', None)]);
        assert_eq!(depths("{(}", "go"), [('{', None), ('(', None), ('}', None)]);
    }

    #[test]
    fn the_pair_at_the_caret_is_found_on_either_side() {
        let text = "f(x)";
        let brackets = Brackets::scan(text, "go");
        assert_eq!(brackets.pair_at(1), Some((1, 3)));
        assert_eq!(brackets.pair_at(4), Some((1, 3)));
        assert_eq!(brackets.pair_at(0), None);
    }
}
