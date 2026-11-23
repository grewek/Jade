//! The one definition of where LaTeX math is in a note: `$…$` inline, `$$…$$`
//! display, and `$$` blocks across lines. Like [`crate::link_scan`] it returns
//! byte ranges and never copies text, and the leaf parser is public because the
//! renderer interleaves it with its own code-span and emphasis handling.
//!
//! Inline rules (Obsidian's, which are pandoc's): the opening `$` is followed
//! by a non-space, the closing `$` is preceded by a non-space and not followed
//! by a digit, `\$` is an escape, and the first `$` after the opener decides:
//! if it cannot close, the opener is plain text. So "$5 and $10" and
//! "$5, $x$" read as text and as the math `x`. Inline math stays on one line;
//! a code span wins over a `$` inside it.
//!
//! A display block opens with `$$` at the start of a line (after indentation)
//! and closes at the first unescaped `$$` in the next [`MAX_BLOCK_LINES`] lines.
//! Line breaks inside are whitespace to LaTeX, but a blank line ends the
//! formula: an opener that reaches one, a code fence or the limit without a
//! closer is plain text, so typing `$$` never turns the rest of the note into
//! math. Openers behind a blockquote or list marker are not found.

use std::ops::Range;

use crate::fence::{self, FenceRole, FenceWalk};
use crate::link_scan::find_closing;

/// How far below its opener a display block's closing `$$` may be.
pub const MAX_BLOCK_LINES: usize = 256;

/// Inline math on one line; ranges are byte offsets into that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathSpan {
    /// Delimiters included.
    pub span: Range<usize>,
    /// The LaTeX between the delimiters.
    pub body: Range<usize>,
    /// `$$…$$`: set in its own centred line rather than inline.
    pub display: bool,
}

/// A `$$` block over several lines; ranges are byte offsets into the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathBlock {
    pub first_line: usize,
    /// Inclusive; the line of the closing `$$`.
    pub last_line: usize,
    /// From the opening `$$` to the end of the closing one.
    pub span: Range<usize>,
    pub body: Range<usize>,
}

/// The math span starting at `line[start]`, which must be `$`, within
/// `line[..end]`. `None` when it is not math (a currency sign, an opener with
/// no valid closer).
pub fn parse_inline(line: &str, start: usize, end: usize) -> Option<MathSpan> {
    let bytes = line.as_bytes();
    assert!(start < end && end <= line.len());
    assert_eq!(bytes[start], b'$');
    if bytes.get(start + 1) == Some(&b'$') && start + 1 < end {
        return parse_display(line, start, end);
    }
    let body_start = start + 1;
    let first = line[body_start..end].chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    let close = find_unescaped(line, body_start, end, b'$')?;
    let before = line[..close].chars().next_back()?;
    let digit_after = bytes.get(close + 1).is_some_and(u8::is_ascii_digit) && close + 1 < end;
    if before.is_whitespace() || digit_after {
        return None;
    }
    Some(MathSpan {
        span: start..close + 1,
        body: body_start..close,
        display: false,
    })
}

/// How far to move on from a `$` at `line[start]` that opened no math: past
/// both dollars of a `$$`, so the second does not open a span of its own.
pub fn opener_len(line: &str, start: usize, end: usize) -> usize {
    if start + 1 < end && line.as_bytes()[start + 1] == b'$' {
        2
    } else {
        1
    }
}

/// `$$ … $$` on one line.
fn parse_display(line: &str, start: usize, end: usize) -> Option<MathSpan> {
    let body_start = start + 2;
    let close = find_double(line, body_start, end)?;
    if line[body_start..close].trim().is_empty() {
        return None;
    }
    Some(MathSpan {
        span: start..close + 2,
        body: body_start..close,
        display: true,
    })
}

/// Calls `f` for each inline math span in `line[start..end]`, left to right,
/// leaving code spans and escapes alone.
pub fn scan_inline(line: &str, start: usize, end: usize, mut f: impl FnMut(MathSpan)) {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < end {
        let Some(offset) = bytes[i..end]
            .iter()
            .position(|&b| matches!(b, b'$' | b'`' | b'\\'))
        else {
            return;
        };
        i += offset;
        i = match bytes[i] {
            b'\\' => i + 2,
            b'`' => find_closing(line, i + 1, end, "`").map_or(i + 1, |close| close + 1),
            _ => match parse_inline(line, i, end) {
                Some(math) => {
                    let next = math.span.end;
                    f(math);
                    next
                }
                None => i + opener_len(line, i, end),
            },
        };
    }
}

/// The offset of the first `target` byte in `line[start..end]` that no
/// backslash escapes (a backslash takes the next character with it).
fn find_unescaped(line: &str, start: usize, end: usize, target: u8) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == target => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// The first unescaped `$$` in `line[start..end]`.
fn find_double(line: &str, start: usize, end: usize) -> Option<usize> {
    let mut from = start;
    loop {
        let at = find_unescaped(line, from, end, b'$')?;
        if at + 1 < end && line.as_bytes()[at + 1] == b'$' {
            return Some(at);
        }
        from = at + 1;
    }
}

/// Every `$$` block in `text`, outside fenced code, in order. Whole-text and
/// allocation-light: the definition that incremental passes are tested against.
pub fn scan_blocks(text: &str) -> Vec<MathBlock> {
    let mut blocks = Vec::new();
    let mut fences = FenceWalk::default();
    let (mut pos, mut line_no) = (0, 0);
    loop {
        let end = text[pos..].find('\n').map_or(text.len(), |i| pos + i);
        let line = &text[pos..end];
        let block = (fences.role(line) == FenceRole::Plain)
            .then(|| block_at(text, pos, end, line_no))
            .flatten();
        match block {
            Some(block) => {
                // The lines inside are math, not Markdown: the fence walk must
                // not see them, nor the next scan start in them.
                line_no = block.last_line + 1;
                pos = text[block.span.end..]
                    .find('\n')
                    .map_or(text.len() + 1, |i| block.span.end + i + 1);
                blocks.push(block);
            }
            None => {
                pos = end + 1;
                line_no += 1;
            }
        }
        if pos > text.len() {
            return blocks;
        }
    }
}

/// Which lines of a text lie inside a `$$` block, for a pass that reads prose
/// and must leave math alone (as it leaves fenced code). Ask for lines in
/// increasing order; each is answered in constant time.
pub struct BlockLines {
    blocks: Vec<MathBlock>,
    next: usize,
}

impl BlockLines {
    pub fn of(text: &str) -> Self {
        // Most notes have no math: skip the line walk for them.
        let blocks = if text.contains("$$") {
            scan_blocks(text)
        } else {
            Vec::new()
        };
        BlockLines { blocks, next: 0 }
    }

    /// Whether zero-based `line` is one of a block's lines (its `$$` lines
    /// included).
    pub fn contains(&mut self, line: usize) -> bool {
        while self
            .blocks
            .get(self.next)
            .is_some_and(|b| b.last_line < line)
        {
            self.next += 1;
        }
        self.blocks
            .get(self.next)
            .is_some_and(|b| b.first_line <= line)
    }
}

/// The lines of `text` that are Markdown: outside fenced code and `$$` blocks,
/// with their zero-based numbers.
pub fn prose_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut blocks = BlockLines::of(text);
    fence::plain_lines(text).filter(move |(n, _)| !blocks.contains(*n))
}

/// Whether a plain `line` might open a `$$` block: the only lines a streaming
/// reader needs to look ahead from.
pub fn may_open_block(line: &str) -> bool {
    let bytes = line.as_bytes();
    // One comparison for the common line: this runs on every line of a note.
    match bytes.first() {
        Some(b'$') => bytes.starts_with(b"$$"),
        Some(b' ' | b'\t') => {
            let indent = bytes
                .iter()
                .take_while(|&&b| b == b' ' || b == b'\t')
                .count();
            bytes[indent..].starts_with(b"$$")
        }
        _ => false,
    }
}

/// Whether `line` opens a block as far as its own text says: an opening `$$`
/// at the start (after indent) that nothing on the line closes (a line that
/// closes itself is a display formula, not a block).
pub fn opens_block(line: &str) -> bool {
    if !may_open_block(line) {
        return false;
    }
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    find_double(line, indent + 2, line.len()).is_none()
}

/// How many following lines a reader must hold to decide whether a line that
/// [`may_open_block`] opens one.
pub const BLOCK_LOOKAHEAD: usize = MAX_BLOCK_LINES;

/// The number of lines (the opener included) of the block that opens at
/// `lines[0]`, or `None` if it opens none. `lines` is the opener and up to
/// [`BLOCK_LOOKAHEAD`] lines after it.
pub fn block_len(lines: &[&str]) -> Option<usize> {
    let text = lines.join("\n");
    let block = scan_blocks(&text).into_iter().next()?;
    (block.first_line == 0).then_some(block.last_line + 1)
}

/// The block whose opening `$$` is at the start of the line `text[start..end]`.
fn block_at(text: &str, start: usize, end: usize, line_no: usize) -> Option<MathBlock> {
    let line = &text[start..end];
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    if !line[indent..].starts_with("$$") {
        return None;
    }
    let open = start + indent;
    let body_start = open + 2;
    // Closed on its own line: an inline display span, not a block.
    if find_double(text, body_start, end).is_some() {
        return None;
    }
    let mut next = end + 1;
    for k in 1..=MAX_BLOCK_LINES {
        if next > text.len() {
            return None;
        }
        let next_end = text[next..].find('\n').map_or(text.len(), |i| next + i);
        let line = &text[next..next_end];
        if line.trim().is_empty() || fence::step(None, line).0 == FenceRole::Delimiter {
            return None;
        }
        if let Some(close) = find_double(text, next, next_end) {
            if text[body_start..close].trim().is_empty() {
                return None;
            }
            return Some(MathBlock {
                first_line: line_no,
                last_line: line_no + k,
                span: open..close + 2,
                body: body_start..close,
            });
        }
        next = next_end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline(line: &str) -> Vec<(String, bool)> {
        let mut found = Vec::new();
        scan_inline(line, 0, line.len(), |m| {
            found.push((line[m.body].to_owned(), m.display));
        });
        found
    }

    fn math(body: &str) -> (String, bool) {
        (body.to_owned(), false)
    }

    #[test]
    fn a_dollar_pair_is_inline_math() {
        assert_eq!(inline("let $x^2$ be"), [math("x^2")]);
        assert_eq!(inline("$a$ and $b$"), [math("a"), math("b")]);
    }

    #[test]
    fn the_span_includes_its_delimiters() {
        let line = "a $x$ b";
        let m = parse_inline(line, 2, line.len()).unwrap();
        assert_eq!(&line[m.span], "$x$");
        assert_eq!(&line[m.body], "x");
    }

    #[test]
    fn currency_is_not_math() {
        assert!(inline("it costs $5").is_empty());
        assert!(inline("$5 and $10").is_empty());
        assert!(inline("between $5 and $6.").is_empty());
        // A closer followed by a digit does not close.
        assert!(inline("$x$1").is_empty());
    }

    #[test]
    fn the_first_dollar_decides() {
        // `$5, ` fails at `$x` (a space before it), so `$x$` is its own span.
        assert_eq!(inline("$5, $x$ done"), [math("x")]);
    }

    #[test]
    fn spaces_inside_the_delimiters_are_not_math() {
        assert!(inline("$ x$").is_empty());
        assert!(inline("$x $").is_empty());
        assert!(inline("$ $").is_empty());
    }

    #[test]
    fn an_escaped_dollar_is_neither_opener_nor_closer() {
        assert!(inline("\\$x\\$").is_empty());
        assert_eq!(inline("$a \\$ b$"), [math("a \\$ b")]);
        // A doubled backslash is a backslash, then a real delimiter.
        assert_eq!(inline("\\\\$x$"), [math("x")]);
    }

    #[test]
    fn double_dollars_on_one_line_are_display() {
        assert_eq!(inline("$$a+b$$"), [("a+b".to_owned(), true)]);
        assert_eq!(inline("see $$ x $$ here"), [(" x ".to_owned(), true)]);
        assert!(inline("$$$$").is_empty());
        assert!(inline("$$ unclosed").is_empty());
    }

    #[test]
    fn code_spans_hide_dollars() {
        assert!(inline("`$x$`").is_empty());
        assert_eq!(inline("`a` $x$ `b`"), [math("x")]);
        // An unclosed backtick is plain.
        assert_eq!(inline("` $x$"), [math("x")]);
    }

    #[test]
    fn math_may_hold_braces_backticks_and_brackets() {
        assert_eq!(inline("$\\{a_{1}\\}$"), [math("\\{a_{1}\\}")]);
        assert_eq!(inline("$[[a]]$"), [math("[[a]]")]);
    }

    #[test]
    fn a_range_limits_the_scan() {
        let line = "$a$ $b$";
        let mut found = Vec::new();
        scan_inline(line, 0, 3, |m| found.push(m.span));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], 0..3);
        // The closer is outside the range.
        assert!(parse_inline(line, 4, 6).is_none());
    }

    #[test]
    fn multibyte_text_around_math_is_safe() {
        assert_eq!(inline("äö $α+β$ üß"), [math("α+β")]);
        assert!(inline("$é $").is_empty());
    }

    fn blocks(text: &str) -> Vec<(usize, usize, String)> {
        scan_blocks(text)
            .into_iter()
            .map(|b| (b.first_line, b.last_line, text[b.body].to_owned()))
            .collect()
    }

    #[test]
    fn a_dollar_block_spans_lines() {
        let text = "intro\n$$\na = b\n$$\nafter";
        assert_eq!(blocks(text), [(1, 3, "\na = b\n".to_owned())]);
        let b = &scan_blocks(text)[0];
        assert_eq!(&text[b.span.clone()], "$$\na = b\n$$");
    }

    #[test]
    fn a_block_may_open_and_close_with_content() {
        assert_eq!(blocks("$$ a\nb $$ tail"), [(0, 1, " a\nb ".to_owned())]);
    }

    #[test]
    fn a_one_line_display_is_not_a_block() {
        assert!(blocks("$$a$$\nnext").is_empty());
    }

    #[test]
    fn an_unclosed_opener_is_plain() {
        assert!(blocks("$$\na\nb").is_empty());
        let far = format!("$$\n{}$$", "x\n".repeat(MAX_BLOCK_LINES));
        assert!(blocks(&far).is_empty());
        let near = format!("$$\n{}$$", "x\n".repeat(MAX_BLOCK_LINES - 1));
        assert_eq!(blocks(&near).len(), 1);
    }

    #[test]
    fn a_blank_line_ends_the_formula_and_an_empty_block_is_not_one() {
        assert_eq!(blocks("$$\na\nb\n$$").len(), 1);
        assert!(blocks("$$\na\n\nb\n$$").is_empty());
        assert!(blocks("$$\na\n  \t\nb\n$$").is_empty());
        assert!(blocks("$$\n$$").is_empty());
        // The unclosed opener does not hide a block that follows it.
        assert_eq!(blocks("$$\na\n\n$$\nb\n$$").len(), 1);
    }

    #[test]
    fn indentation_is_allowed_before_the_opener() {
        assert_eq!(blocks("  $$\na\n$$").len(), 1);
        assert!(blocks("x $$\na\n$$").is_empty());
    }

    #[test]
    fn code_fences_and_math_do_not_mix() {
        // An opener inside a fence is code.
        assert!(blocks("```\n$$\na\n$$\n```").is_empty());
        // A fence between opener and closer breaks the block.
        assert!(blocks("$$\n```\n$$").is_empty());
    }

    #[test]
    fn blocks_follow_each_other_and_report_lines() {
        let text = "$$\na\n$$\ntext\n$$\nb\n$$";
        assert_eq!(
            blocks(text),
            [(0, 2, "\na\n".to_owned()), (4, 6, "\nb\n".to_owned())]
        );
    }

    #[test]
    fn a_closing_line_may_start_the_next_block_only_after_it() {
        // The closer's own line is consumed: its `$$` does not reopen.
        assert_eq!(blocks("$$\na\n$$ $$\nb\n$$").len(), 1);
    }

    #[test]
    fn trailing_newlines_and_empty_text_are_fine() {
        assert!(blocks("").is_empty());
        assert_eq!(blocks("$$\na\n$$\n").len(), 1);
        assert_eq!(blocks("$$\na\n$$\n\n").len(), 1);
    }

    #[test]
    fn random_text_never_panics_and_blocks_are_ordered() {
        // xorshift64: deterministic, so a failing seed reproduces.
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n as u64) as usize
            }
        }
        let alphabet = [
            "$", "$$", "\\", "`", "```", "\n", " ", "a", "é", "\\$", "[[", "x", "\n$$\n",
        ];
        let mut rng = Rng(0xDEAD_BEEF_1234_5678);
        for _ in 0..3_000 {
            let n = 1 + rng.below(30);
            let text: String = (0..n)
                .map(|_| alphabet[rng.below(alphabet.len())])
                .collect();
            let found = scan_blocks(&text);
            for pair in found.windows(2) {
                assert!(pair[0].last_line < pair[1].first_line, "{text:?}");
                assert!(pair[0].span.end <= pair[1].span.start, "{text:?}");
            }
            for b in &found {
                assert!(text.is_char_boundary(b.span.start) && text.is_char_boundary(b.span.end));
                assert!(b.first_line < b.last_line, "{text:?}");
            }
            for line in text.split('\n') {
                scan_inline(line, 0, line.len(), |m| {
                    assert!(
                        line.is_char_boundary(m.span.start) && line.is_char_boundary(m.span.end)
                    );
                    assert!(m.body.start <= m.body.end);
                });
            }
        }
    }

    #[test]
    fn prose_lines_leave_out_fences_and_math_blocks() {
        let text = "a\n$$\nb\n$$\nc\n```\nd\n```\ne";
        let got: Vec<_> = prose_lines(text).collect();
        assert_eq!(got, [(0, "a"), (4, "c"), (8, "e")]);
    }

    #[test]
    fn an_unclosed_opener_leaves_its_paragraph_as_prose() {
        let text = "a\n$$\nb\n\nc";
        let got: Vec<_> = prose_lines(text).map(|(n, _)| n).collect();
        assert_eq!(got, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_closing_line_belongs_to_the_block_whole() {
        let text = "$$\nx\n$$ and [[link]]\nafter";
        let got: Vec<_> = prose_lines(text).collect();
        assert_eq!(got, [(3, "after")]);
    }

    #[test]
    fn block_len_counts_the_opener_and_the_closer() {
        assert_eq!(block_len(&["$$", "x", "$$", "tail"]), Some(3));
        assert_eq!(block_len(&["$$", "x", "", "$$"]), None);
        assert_eq!(block_len(&["plain", "$$"]), None);
        assert_eq!(
            block_len(&["$$ x $$", "y"]),
            None,
            "one line is inline math"
        );
    }
}
