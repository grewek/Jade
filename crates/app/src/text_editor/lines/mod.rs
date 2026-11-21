//! Cutting a text into lines, and the arrays that follow its line count.
//! Nothing here knows about Markdown: `doc` builds its classification caches
//! on these.

mod scan;
mod splice;

pub(super) mod chunks;
pub(super) mod fenwick;

use scan::for_each_newline;

pub(in crate::text_editor) use fenwick::{Fenwick, LINE_GROWTH_SLACK};
pub(in crate::text_editor) use splice::{resize_vec, shift_line};

/// The number of lines `text.split('\n')` yields; the oracle for
/// [`line_strides`].
#[cfg(test)]
pub(in crate::text_editor) fn count_lines(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .map(|&b| usize::from(b == b'\n'))
        .sum::<usize>()
        + 1
}

/// Each line's byte stride (`len + 1`, except the last line, which has no
/// trailing `\n`), with room for [`LINE_GROWTH_SLACK`] more lines. The one
/// scan for newlines in a cold build.
pub(in crate::text_editor) fn line_strides(text: &str) -> Vec<i64> {
    // A guess at the line count (the stress fixture averages 37 bytes per
    // line): counting first costs as much as the scan, and growing the
    // vector once or twice is cheaper than that.
    let mut strides = Vec::with_capacity(text.len() / 32 + LINE_GROWTH_SLACK);
    let mut previous = 0;
    for_each_newline(text.as_bytes(), |newline| {
        strides.push((newline + 1 - previous) as i64);
        previous = newline + 1;
    });
    strides.push((text.len() - previous) as i64);
    strides
}

/// The lines `strides` (from [`line_strides`] of `text`) describe, without
/// scanning for newlines again.
pub(in crate::text_editor) fn lines_of<'a>(
    text: &'a str,
    strides: &'a [i64],
) -> impl Iterator<Item = &'a str> + 'a {
    let last = strides.len().saturating_sub(1);
    let mut start = 0;
    strides.iter().enumerate().map(move |(i, &stride)| {
        let stride = stride as usize;
        let end = start + if i < last { stride - 1 } else { stride };
        let line = &text[start..end];
        start += stride;
        line
    })
}

/// Yields `(line_text, line_len, has_next_line)` starting at `start_byte`
/// (which must be a real line-start byte offset in `text`), walking
/// forward via a running cursor and `text[cursor..].find('\n')` rather
/// than ever materializing the whole document's lines — the technique
/// every patch function above relies on to stay bounded by how far it
/// actually walks, not by the document's total length. The same bounded-
/// scan idea `buffer.rs`'s own `line_start`/`line_end` already use, just
/// walking forward instead of from a single point.
pub(in crate::text_editor) fn lines_from(
    text: &str,
    start_byte: usize,
) -> impl Iterator<Item = (&str, usize, bool)> {
    let mut cursor = start_byte;
    std::iter::from_fn(move || {
        if cursor > text.len() {
            return None;
        }
        let rest = &text[cursor..];
        let line_len = rest.find('\n').unwrap_or(rest.len());
        let line = &rest[..line_len];
        let has_next_line = cursor + line_len < text.len();
        cursor += line_len + 1;
        Some((line, line_len, has_next_line))
    })
}
