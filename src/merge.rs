//! Merge conflicts as git writes them into a file:
//!
//! ```text
//! <<<<<<< ours
//! current side
//! ||||||| base          (with merge.conflictStyle = diff3)
//! common ancestor
//! =======
//! incoming side
//! >>>>>>> theirs
//! ```

use std::ops::Range;

/// One conflict, by lines (zero-based).
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: usize,
    /// The lines of the current side.
    pub ours: Range<usize>,
    /// The lines of the incoming side.
    pub theirs: Range<usize>,
    /// The `>>>>>>>` line.
    pub end: usize,
    /// What the markers call the two sides, as `HEAD` and `origin/main`.
    pub ours_label: String,
    pub theirs_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Ours,
    Theirs,
    Both,
    /// Neither side: the lines go.
    Neither,
}

/// The conflicts in `text`, in order. Unfinished markers are left out.
pub fn conflicts(text: &str) -> Vec<Conflict> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut ix = 0;
    while ix < lines.len() {
        if !lines[ix].starts_with("<<<<<<<") {
            ix += 1;
            continue;
        }
        let start = ix;
        let (mut base, mut middle, mut end) = (None, None, None);
        let mut at = start + 1;
        while at < lines.len() {
            let line = lines[at];
            if line.starts_with("|||||||") && middle.is_none() {
                base = Some(at);
            } else if line.starts_with("=======") && middle.is_none() {
                middle = Some(at);
            } else if line.starts_with(">>>>>>>") && middle.is_some() {
                end = Some(at);
                break;
            } else if line.starts_with("<<<<<<<") {
                break;
            }
            at += 1;
        }
        match (middle, end) {
            (Some(middle), Some(end)) => {
                let label = |line: &str| line.trim_start_matches(['<', '>']).trim().to_string();
                found.push(Conflict {
                    start,
                    ours: start + 1..base.unwrap_or(middle),
                    theirs: middle + 1..end,
                    end,
                    ours_label: label(lines[start]),
                    theirs_label: label(lines[end]),
                });
                ix = end + 1;
            }
            _ => ix = at.max(start + 1),
        }
    }
    found
}

/// `text` with `conflict` replaced by the side or sides chosen.
pub fn resolve(text: &str, conflict: &Conflict, choice: Choice) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    for line in &lines[..conflict.start] {
        out.push_str(line);
    }
    let mut keep = |range: &Range<usize>| {
        for line in &lines[range.clone()] {
            out.push_str(line);
        }
    };
    match choice {
        Choice::Ours => keep(&conflict.ours),
        Choice::Theirs => keep(&conflict.theirs),
        Choice::Both => {
            keep(&conflict.ours);
            keep(&conflict.theirs);
        }
        Choice::Neither => {}
    }
    for line in &lines[conflict.end + 1..] {
        out.push_str(line);
    }
    out
}

/// A line of one side in the merge window, numbered within that side.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub number: usize,
    pub text: String,
}

/// A row of the merge window. Outside conflicts both sides hold the same
/// line, changes from either side already merged; inside a conflict each
/// side holds its own line, or nothing where it is shorter.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Same { left: usize, right: usize, text: String },
    Conflict {
        index: usize,
        left: Option<Line>,
        right: Option<Line>,
        first: bool,
    },
}

/// The rows of a merged file: the incoming side on the left, the current
/// side on the right.
pub fn rows(text: &str) -> Vec<Row> {
    let lines: Vec<&str> = text.lines().collect();
    let (mut left, mut right) = (0, 0);
    let mut rows = Vec::with_capacity(lines.len());
    let mut at = 0;
    for (index, conflict) in conflicts(text).iter().enumerate() {
        for line in &lines[at..conflict.start] {
            left += 1;
            right += 1;
            rows.push(Row::Same { left, right, text: line.to_string() });
        }
        let theirs = &lines[conflict.theirs.clone()];
        let ours = &lines[conflict.ours.clone()];
        let height = theirs.len().max(ours.len()).max(1);
        for k in 0..height {
            let side = |lines: &[&str], counter: &mut usize| {
                lines.get(k).map(|text| {
                    *counter += 1;
                    Line { number: *counter, text: text.to_string() }
                })
            };
            let l = side(theirs, &mut left);
            let r = side(ours, &mut right);
            rows.push(Row::Conflict { index, left: l, right: r, first: k == 0 });
        }
        at = conflict.end + 1;
    }
    for line in &lines[at.min(lines.len())..] {
        left += 1;
        right += 1;
        rows.push(Row::Same { left, right, text: line.to_string() });
    }
    rows
}

/// What a pair of decisions on a conflict means: keep the incoming side
/// (left), the current side (right), both, or neither.
pub fn choice(keep_left: bool, keep_right: bool) -> Choice {
    match (keep_left, keep_right) {
        (true, false) => Choice::Theirs,
        (false, true) => Choice::Ours,
        (true, true) => Choice::Both,
        (false, false) => Choice::Neither,
    }
}

/// `text` with every conflict settled by `choices`, in order.
pub fn resolve_all(text: &str, choices: &[Choice]) -> String {
    let mut text = text.to_string();
    // From the last, so the earlier ones keep their lines.
    for (index, choice) in choices.iter().enumerate().rev() {
        if let Some(conflict) = conflicts(&text).get(index).cloned() {
            text = resolve(&text, &conflict, *choice);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n<<<<<<< HEAD\nmine\n=======\ntheirs\nmore\n>>>>>>> feature\nb\n<<<<<<< HEAD\nx\n||||||| base\nold\n=======\ny\n>>>>>>> feature\n";

    #[test]
    fn conflicts_are_found_with_and_without_a_base() {
        assert_eq!(
            conflicts(TEXT),
            [
                Conflict {
                    start: 1,
                    ours: 2..3,
                    theirs: 4..6,
                    end: 6,
                    ours_label: "HEAD".into(),
                    theirs_label: "feature".into(),
                },
                Conflict {
                    start: 8,
                    ours: 9..10,
                    theirs: 13..14,
                    end: 14,
                    ours_label: "HEAD".into(),
                    theirs_label: "feature".into(),
                },
            ]
        );
        assert!(conflicts("<<<<<<< HEAD\nunfinished\n").is_empty());
    }

    #[test]
    fn a_conflict_resolves_to_the_side_chosen() {
        let first = &conflicts(TEXT)[0];
        let ours = resolve(TEXT, first, Choice::Ours);
        assert!(ours.starts_with("a\nmine\nb\n<<<<<<<"));
        assert!(resolve(TEXT, first, Choice::Theirs).starts_with("a\ntheirs\nmore\nb\n"));
        assert!(resolve(TEXT, first, Choice::Both).starts_with("a\nmine\ntheirs\nmore\nb\n"));
        // The other conflict is still there, at its new place.
        assert_eq!(conflicts(&ours).len(), 1);
    }

    #[test]
    fn rows_line_the_two_sides_up() {
        let text = "a\n<<<<<<< HEAD\nmine\n=======\ntheirs\nmore\n>>>>>>> feature\nb\n";
        let rows = rows(text);
        let line = |number: usize, text: &str| Some(Line { number, text: text.to_string() });
        assert_eq!(
            rows,
            [
                Row::Same { left: 1, right: 1, text: "a".into() },
                Row::Conflict { index: 0, left: line(2, "theirs"), right: line(2, "mine"), first: true },
                Row::Conflict { index: 0, left: line(3, "more"), right: None, first: false },
                Row::Same { left: 4, right: 3, text: "b".into() },
            ]
        );
    }

    #[test]
    fn every_conflict_is_settled_by_its_choice() {
        let choices = [choice(true, false), choice(true, true)];
        assert_eq!(resolve_all(TEXT, &choices), "a\ntheirs\nmore\nb\nx\ny\n");
    }
}
