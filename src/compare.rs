//! Comparing two versions of a file line by line, as a side-by-side view
//! needs it: which lines changed, which words in them, and which line of one
//! side stands beside a given line of the other.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkKind {
    /// Lines only the new side has.
    Added,
    /// Lines only the old side has.
    Removed,
    /// Lines changed from the old side's to the new side's.
    Modified,
}

/// A run of lines, zero-based, on each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub kind: ChunkKind,
}

#[derive(Clone, Debug, Default)]
pub struct Comparison {
    /// Every run of lines in order, equal ones included.
    runs: Vec<(Range<usize>, Range<usize>, bool)>,
    /// The runs that changed.
    pub chunks: Vec<Chunk>,
    /// Changed words: byte ranges in the old text and in the new text.
    pub old_words: Vec<Range<usize>>,
    pub new_words: Vec<Range<usize>>,
    pub added: usize,
    pub removed: usize,
}

/// `new` with the lines of `chunk` replaced by the old side's lines.
pub fn take_lines(old: &str, new: &str, chunk: &Chunk) -> String {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = String::with_capacity(new.len());
    let clamp = |range: &Range<usize>, len: usize| range.start.min(len)..range.end.min(len);
    let replaced = clamp(&chunk.new, new_lines.len());
    for line in &new_lines[..replaced.start] {
        out.push_str(line);
    }
    let taken = clamp(&chunk.old, old_lines.len());
    for line in &old_lines[taken] {
        out.push_str(line);
    }
    for line in &new_lines[replaced.end..] {
        out.push_str(line);
    }
    out
}

/// Byte offset where each line starts, plus the end of the text.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(ix, _)| ix + 1));
    if *starts.last().unwrap() != text.len() {
        starts.push(text.len());
    }
    starts
}

/// A modified pair of lines longer than this is marked whole, not word by word.
const MAX_WORD_DIFF_LINE: usize = 2000;

pub fn compare(old: &str, new: &str) -> Comparison {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut comparison = Comparison::default();
    let old_starts = line_starts(old);
    let new_starts = line_starts(new);
    for op in diff.ops() {
        let (old_range, new_range) = (op.old_range(), op.new_range());
        let kind = match op {
            similar::DiffOp::Equal { .. } => {
                comparison.runs.push((old_range, new_range, true));
                continue;
            }
            similar::DiffOp::Insert { .. } => ChunkKind::Added,
            similar::DiffOp::Delete { .. } => ChunkKind::Removed,
            similar::DiffOp::Replace { .. } => ChunkKind::Modified,
        };
        comparison.added += new_range.len();
        comparison.removed += old_range.len();
        if kind == ChunkKind::Modified {
            // Lines paired in order; the words that differ in each pair.
            for (old_line, new_line) in old_range.clone().zip(new_range.clone()) {
                let old_at = old_starts[old_line];
                let new_at = new_starts[new_line];
                let old_text = &old[old_at..old_starts.get(old_line + 1).copied().unwrap_or(old.len())];
                let new_text = &new[new_at..new_starts.get(new_line + 1).copied().unwrap_or(new.len())];
                if old_text.len().max(new_text.len()) > MAX_WORD_DIFF_LINE {
                    continue;
                }
                let (old_words, new_words) = changed_words(old_text, new_text);
                comparison
                    .old_words
                    .extend(old_words.into_iter().map(|r| r.start + old_at..r.end + old_at));
                comparison
                    .new_words
                    .extend(new_words.into_iter().map(|r| r.start + new_at..r.end + new_at));
            }
        }
        comparison.runs.push((old_range.clone(), new_range.clone(), false));
        comparison.chunks.push(Chunk {
            old: old_range,
            new: new_range,
            kind,
        });
    }
    comparison
}

/// Byte ranges of the words that differ between two versions of one line,
/// leaving out the line break and changes that are only spacing.
fn changed_words(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = similar::TextDiff::configure().diff_words(old, new);
    let offsets = |slices: &[&str]| {
        let mut offsets = Vec::with_capacity(slices.len() + 1);
        let mut at = 0;
        offsets.push(0);
        for slice in slices {
            at += slice.len();
            offsets.push(at);
        }
        offsets
    };
    let old_offsets = offsets(diff.old_slices());
    let new_offsets = offsets(diff.new_slices());
    let (mut old_words, mut new_words) = (Vec::new(), Vec::new());
    let push = |words: &mut Vec<Range<usize>>, text: &str, range: Range<usize>| {
        let slice = &text[range.clone()];
        // Trim spacing and the line break off the ends of the change.
        let start = range.start + (slice.len() - slice.trim_start().len());
        let end = range.end - (slice.len() - slice.trim_end().len());
        if start < end {
            words.push(start..end);
        }
    };
    for op in diff.ops() {
        if matches!(op, similar::DiffOp::Equal { .. }) {
            continue;
        }
        let old_range = op.old_range();
        let new_range = op.new_range();
        if !old_range.is_empty() {
            push(&mut old_words, old, old_offsets[old_range.start]..old_offsets[old_range.end]);
        }
        if !new_range.is_empty() {
            push(&mut new_words, new, new_offsets[new_range.start]..new_offsets[new_range.end]);
        }
    }
    (merge_spaced(old, old_words), merge_spaced(new, new_words))
}

/// Join changed words that only spacing separates, so a changed phrase reads as one.
fn merge_spaced(text: &str, words: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(words.len());
    for word in words {
        match merged.last_mut() {
            Some(last) if text[last.end..word.start].trim().is_empty() => last.end = word.end,
            _ => merged.push(word),
        }
    }
    merged
}

impl Comparison {
    /// The line of the other side that stands beside `line` of one side, as a
    /// fraction for smooth scrolling: equal runs line up exactly, changed runs
    /// stretch to fit each other.
    pub fn map_line(&self, line: f32, from_old: bool) -> f32 {
        let line = line.max(0.);
        for (old, new, _) in &self.runs {
            let (from, to) = if from_old { (old, new) } else { (new, old) };
            if from.is_empty() {
                continue;
            }
            if line < from.end as f32 {
                let into = (line - from.start as f32).max(0.);
                let fraction = into / from.len() as f32;
                return to.start as f32 + fraction * to.len() as f32;
            }
        }
        // Past the last run: keep the same distance beyond its end.
        let (old_end, new_end) = self
            .runs
            .last()
            .map_or((0, 0), |(old, new, _)| (old.end, new.end));
        let (from_end, to_end) = if from_old {
            (old_end, new_end)
        } else {
            (new_end, old_end)
        };
        to_end as f32 + (line - from_end as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "a\nb\nc\nd\ne\nf\n";
    const NEW: &str = "a\nB\nc\ne\nf\ng\nh\n";

    #[test]
    fn taking_a_chunk_puts_the_old_lines_in_its_place() {
        let old = "a\nB\nc\nd\n";
        let new = "a\nb\nc\nx\nd\n";
        let comparison = compare(old, new);
        assert_eq!(comparison.chunks.len(), 2);
        assert_eq!(take_lines(old, new, &comparison.chunks[0]), "a\nB\nc\nx\nd\n");
        assert_eq!(take_lines(old, new, &comparison.chunks[1]), "a\nb\nc\nd\n");
    }

    #[test]
    fn chunks_tell_added_removed_and_modified_lines_apart() {
        let comparison = compare(OLD, NEW);
        assert_eq!(
            comparison.chunks,
            [
                Chunk { old: 1..2, new: 1..2, kind: ChunkKind::Modified },
                Chunk { old: 3..4, new: 3..3, kind: ChunkKind::Removed },
                Chunk { old: 6..6, new: 5..7, kind: ChunkKind::Added },
            ]
        );
        assert_eq!((comparison.added, comparison.removed), (3, 2));
    }

    #[test]
    fn lines_beside_each_other_follow_the_changes() {
        let comparison = compare(OLD, NEW);
        // Before any change, and after one that changed nothing in length.
        assert_eq!(comparison.map_line(0., true), 0.);
        assert_eq!(comparison.map_line(2., true), 2.);
        // A removed line squeezes to where it was.
        assert_eq!(comparison.map_line(3.5, true), 3.);
        assert_eq!(comparison.map_line(4., true), 3.);
        // And the way back.
        assert_eq!(comparison.map_line(3., false), 4.);
        assert_eq!(comparison.map_line(6., false), 6.);
    }

    #[test]
    fn modified_lines_mark_the_words_that_changed() {
        let old = "x := compute(a, b)\n";
        let new = "x := computeAll(a, b, c)\n";
        let comparison = compare(old, new);
        let old_words: Vec<&str> = comparison.old_words.iter().map(|r| &old[r.clone()]).collect();
        let new_words: Vec<&str> = comparison.new_words.iter().map(|r| &new[r.clone()]).collect();
        assert_eq!(old_words, ["compute(a, b)"]);
        assert_eq!(new_words, ["computeAll(a, b, c)"]);
        // Spacing-only changes and unchanged text are not marked.
        let spacing = compare("a  b\n", "a b\n");
        assert!(spacing.new_words.is_empty());
    }
}
