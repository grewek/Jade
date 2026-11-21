//! Keeping per-line arrays the same length as the document's line count.

use super::fenwick::LINE_GROWTH_SLACK;

/// `line` moved by `delta` lines.
pub(in crate::text_editor) fn shift_line(line: usize, delta: isize) -> usize {
    let shifted = line as isize + delta;
    debug_assert!(shifted >= 0, "line {line} shifted by {delta}");
    shifted.max(0) as usize
}

/// Inserts `delta` copies of the entry before `at` (`delta > 0`) or removes
/// `-delta` entries starting at `at` (`delta < 0`). Grows in fixed chunks:
/// see [`LINE_GROWTH_SLACK`].
pub(in crate::text_editor) fn resize_vec<T: Clone>(v: &mut Vec<T>, at: usize, delta: isize) {
    if delta > 0 {
        let k = delta as usize;
        let fill = v[at - 1].clone();
        if v.capacity() - v.len() < k {
            v.reserve_exact(k.max(LINE_GROWTH_SLACK));
        }
        v.splice(at..at, std::iter::repeat_n(fill, k));
    } else {
        v.drain(at..at + delta.unsigned_abs());
    }
}
