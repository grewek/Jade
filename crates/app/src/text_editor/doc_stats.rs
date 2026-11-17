//! Word and character counts for the status bar. Kept per line so an edit
//! recounts only the lines it touched (words never span a line), and the
//! totals are running sums, so reading them is free.
//!
//! A word is a whitespace-separated token with at least one letter or digit,
//! so list markers, `---` rules and table pipes do not count. Characters are
//! Unicode scalar values, spaces included, line breaks excluded.

use super::document_cache::{DocumentCache, EditPatch};
use super::fenwick::LINE_GROWTH_SLACK;
use super::line_chunks::map_line_chunks;

/// (words, characters) of one line.
type Counts = (u32, u32);

pub(super) struct DocStats {
    per_line: Vec<Counts>,
    words: u64,
    chars: u64,
}

const WHITESPACE: u8 = 1;
const ALNUM: u8 = 2;

/// `char::is_whitespace` / `char::is_alphanumeric` for every byte (non-ASCII
/// bytes never reach this table, see `line_counts`), as bit flags so one
/// lookup per byte decides both.
const ASCII_CLASS: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut b = 0usize;
    while b < 128 {
        let c = b as u8;
        if matches!(c, b'\t'..=b'\r' | b' ') {
            table[b] = WHITESPACE;
        } else if c.is_ascii_alphanumeric() {
            table[b] = ALNUM;
        }
        b += 1;
    }
    table
};

pub(super) fn line_counts(line: &str) -> Counts {
    if line.is_ascii() {
        return ascii_line_counts(line.as_bytes());
    }
    let (mut words, mut chars) = (0u32, 0u32);
    let (mut in_token, mut has_alnum) = (false, false);
    for c in line.chars() {
        chars += 1;
        if c.is_whitespace() {
            words += u32::from(in_token && has_alnum);
            (in_token, has_alnum) = (false, false);
        } else {
            in_token = true;
            has_alnum |= c.is_alphanumeric();
        }
    }
    words += u32::from(in_token && has_alnum);
    (words, chars)
}

/// The same count over ASCII bytes: a word ends at whitespace after a letter
/// or digit. Runs over every byte of a note at open, so it is branch-free.
fn ascii_line_counts(bytes: &[u8]) -> Counts {
    let (mut words, mut has_alnum) = (0u32, 0u8);
    for &b in bytes {
        let class = ASCII_CLASS[usize::from(b)];
        let whitespace = class & WHITESPACE;
        words += u32::from(whitespace & has_alnum);
        has_alnum = (has_alnum | (class >> 1)) & (whitespace ^ 1);
    }
    (words + u32::from(has_alnum), bytes.len() as u32)
}

/// Counts of every line in `chunk`, which starts at a line start and, unless
/// it is the document's last chunk, ends just after a newline.
/// `final_empty_line` is set on the last chunk when the document ends in a
/// newline (or is empty): that last, empty line has no `split_inclusive` piece.
fn count_lines(chunk: &str, final_empty_line: bool) -> Vec<Counts> {
    let mut counts: Vec<Counts> = chunk
        .split_inclusive('\n')
        .map(|line| line_counts(line.strip_suffix('\n').unwrap_or(line)))
        .collect();
    if final_empty_line {
        counts.push((0, 0));
    }
    counts
}

impl DocStats {
    pub(super) fn new(text: &str) -> Self {
        let mut stats = Self {
            per_line: Vec::new(),
            words: 0,
            chars: 0,
        };
        stats.rebuild(text);
        stats
    }

    /// Recounts the whole document: the cold open, and the fallback when an
    /// edit could not be patched. A big note is counted by several threads,
    /// each over a run of whole lines: this is the one O(bytes) pass of a note
    /// open (about 21ms single-threaded on the 11.8MB stress note).
    pub(super) fn rebuild(&mut self, text: &str) {
        let ends_the_last_line = text.is_empty() || text.ends_with('\n');
        let counted: Vec<Vec<Counts>> = map_line_chunks(
            text,
            |_| 0,
            |_, chunk, last| count_lines(chunk, last && ends_the_last_line),
        );

        self.per_line.clear();
        self.per_line
            .reserve_exact(counted.iter().map(Vec::len).sum::<usize>() + LINE_GROWTH_SLACK);
        let (mut words, mut chars) = (0u64, 0u64);
        for counts in counted.iter().flatten() {
            words += u64::from(counts.0);
            chars += u64::from(counts.1);
        }
        for part in counted {
            self.per_line.extend(part);
        }
        (self.words, self.chars) = (words, chars);
    }

    /// Recounts the lines `patch` touched; `text` and `doc` already reflect the edit.
    pub(super) fn patch(&mut self, text: &str, doc: &DocumentCache, patch: &EditPatch) {
        let new_len = doc.line_count();
        let old_len = new_len as isize - patch.line_delta;
        if self.per_line.len() as isize != old_len {
            return self.rebuild(text);
        }
        let (start, old_end) = (patch.touched.start, patch.old_touched().end);
        assert!(old_end <= self.per_line.len());

        let (mut added_words, mut added_chars) = (0u64, 0u64);
        let fresh = (start..patch.touched.end).map(|i| {
            let counts = line_counts(&text[doc.line_start_byte(i)..doc.line_end_byte(i)]);
            added_words += u64::from(counts.0);
            added_chars += u64::from(counts.1);
            counts
        });
        let (mut removed_words, mut removed_chars) = (0u64, 0u64);
        for (words, chars) in self.per_line.splice(start..old_end, fresh) {
            removed_words += u64::from(words);
            removed_chars += u64::from(chars);
        }
        self.words = self.words - removed_words + added_words;
        self.chars = self.chars - removed_chars + added_chars;
    }

    pub(super) fn words(&self) -> usize {
        self.words as usize
    }

    pub(super) fn chars(&self) -> usize {
        self.chars as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_editor::line_chunks::PARALLEL_MIN_BYTES;
    use crate::text_editor::test_support::{Rng, random_edit};

    #[test]
    fn counts_words_with_letters_or_digits_and_every_character() {
        assert_eq!(line_counts(""), (0, 0));
        assert_eq!(line_counts("hello world"), (2, 11));
        assert_eq!(
            line_counts("  - item  "),
            (1, 10),
            "a list marker is no word"
        );
        assert_eq!(line_counts("| a | b |"), (2, 9));
        assert_eq!(line_counts("---"), (0, 3));
        assert_eq!(line_counts("caf\u{e9} na\u{ef}ve"), (2, 10));
        assert_eq!(line_counts("v2.0-beta"), (1, 9));
    }

    #[test]
    fn totals_cover_the_document_without_counting_newlines() {
        let stats = DocStats::new("one two\n\n- three\nfour");
        assert_eq!((stats.words(), stats.chars()), (4, 7 + 0 + 7 + 4));
    }

    /// The threaded cold count must equal a plain line-by-line one, whatever
    /// the chunk boundaries fall on: blank runs, one huge line, either ending.
    #[test]
    fn threaded_count_matches_a_sequential_one() {
        let unit = "word another  - 12 \u{e9}t\u{e9}\n\n\n| a | b |\nlast words here";
        let long_line = "x ".repeat(PARALLEL_MIN_BYTES);
        let texts = [
            unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2),
            format!("{}\n", unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2)),
            "\n".repeat(PARALLEL_MIN_BYTES + 5),
            format!("start\n{long_line}\nend"),
            long_line,
        ];
        for text in &texts {
            assert!(text.len() >= PARALLEL_MIN_BYTES);
            let expected: Vec<Counts> = text.split('\n').map(line_counts).collect();
            let stats = DocStats::new(text);
            assert_eq!(stats.per_line, expected);
            let words: u64 = expected.iter().map(|c| u64::from(c.0)).sum();
            assert_eq!(stats.words() as u64, words);
        }
    }

    /// After any edit the patched totals must equal a fresh count: edits that
    /// change the line count, join lines, and hit the first and last line.
    #[test]
    fn patched_totals_match_a_fresh_count_after_random_edits() {
        let alphabet: Vec<&str> = vec!["a", "bc", " ", "  ", "\n", "- ", "\u{e9}", "|", "1"];
        let mut rng = Rng::new(0x2545_f491_4f6c_dd1d);
        let mut text = String::from("start of the note\n\n- a list item\nlast line");
        let mut cache = DocumentCache::new(&text);
        let mut stats = DocStats::new(&text);
        for step in 0..400 {
            let (new_text, edit) = random_edit(&text, &mut rng, &alphabet);
            match cache.record_edit(&new_text, edit) {
                Some(patch) => stats.patch(&new_text, &cache, &patch),
                None => stats.rebuild(&new_text),
            }
            text = new_text;
            let fresh = DocStats::new(&text);
            assert_eq!(
                (stats.words(), stats.chars()),
                (fresh.words(), fresh.chars()),
                "step {step}, text {text:?}"
            );
            assert_eq!(stats.per_line, fresh.per_line, "step {step}");
        }
    }
}
