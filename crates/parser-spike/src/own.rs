//! Candidate 4: an own line-based block parser. It streams: it reads only as
//! far as the block it is on needs, so an update stops exactly where it
//! resynchronises. Not complete CommonMark; the conformance check says how far
//! off it is. HTML blocks are left out on purpose (K9: other HTML is text).

use jade_core::math_scan;

use crate::tree::{self, BlockParser, Ctx, Emit, Kind, Node, Unit};

/// Recursion bound for quotes and lists (input-controlled).
const MAX_DEPTH: u8 = 32;

#[derive(Clone, Copy)]
struct Line<'a> {
    /// Offset in the note of `text`, which may have a prefix stripped.
    at: usize,
    text: &'a str,
}

trait Source<'a> {
    fn line(&mut self, k: usize) -> Option<Line<'a>>;
}

struct Slice<'s, 'a>(&'s [Line<'a>]);

impl<'a> Source<'a> for Slice<'_, 'a> {
    fn line(&mut self, k: usize) -> Option<Line<'a>> {
        self.0.get(k).copied()
    }
}

/// The note's lines from a block's start, read on demand. Ends at the closing
/// tag of the surrounding container: that is the precedence rule (K10).
struct Doc<'s, 'a> {
    text: &'a str,
    next: usize,
    ctx: Ctx,
    lines: &'s mut Vec<Line<'a>>,
}

impl<'a> Source<'a> for Doc<'_, 'a> {
    fn line(&mut self, k: usize) -> Option<Line<'a>> {
        while self.lines.len() <= k {
            if self.next >= self.text.len() {
                return None;
            }
            let (text, next) = tree::line_at(self.text, self.next);
            if tree::is_closer(text, self.ctx) {
                return None;
            }
            self.lines.push(Line {
                at: self.next,
                text,
            });
            self.next = next;
        }
        Some(self.lines[k])
    }
}

#[derive(Default)]
pub struct Own;

impl BlockParser for Own {
    fn parse_from(&mut self, text: &str, mut pos: usize, mut ctx: Ctx, emit: &mut Emit<'_>) {
        let mut lines = Vec::with_capacity(64);
        let mut children = Vec::new();
        while pos < text.len() {
            let (first, next) = tree::line_at(text, pos);
            if tree::is_blank(first) {
                pos = next;
                continue;
            }
            if let Some((kind, after)) = tree::tag_line(first, ctx) {
                let len = first.len() as u32;
                let unit = Unit {
                    start: pos as u32,
                    len,
                    kind,
                    ctx,
                    children: Vec::new(),
                };
                if emit(unit).is_break() {
                    return;
                }
                ctx = after;
                pos = next;
                continue;
            }
            lines.clear();
            children.clear();
            let mut doc = Doc {
                text,
                next: pos,
                ctx,
                lines: &mut lines,
            };
            let (kind, count) = block(&mut doc, ctx, 0, &mut children);
            let (end, last) = block_end(&mut doc, count);
            for node in &mut children {
                node.start -= pos as u32;
            }
            let unit = Unit {
                start: pos as u32,
                len: (end - pos) as u32,
                kind,
                ctx,
                children: children.clone(),
            };
            if emit(unit).is_break() {
                return;
            }
            pos = tree::line_at(text, last).1;
        }
    }
}

/// The end of a block's last non-blank line among its `count` lines, and
/// that line's offset.
fn block_end<'a>(src: &mut impl Source<'a>, count: usize) -> (usize, usize) {
    for k in (0..count).rev() {
        if let Some(line) = src.line(k) {
            if !tree::is_blank(line.text) {
                return (line.at + line.text.len(), line.at);
            }
        }
    }
    (0, 0)
}

/// Parses the block starting at line 0 (not blank): its kind and line count.
fn block<'a>(
    src: &mut impl Source<'a>,
    ctx: Ctx,
    depth: u8,
    nodes: &mut Vec<Node>,
) -> (Kind, usize) {
    let Some(first) = src.line(0) else {
        return (Kind::Other, 1);
    };
    let indent = indent_of(first.text);
    if indent >= 4 {
        return (Kind::Code, indented_code(src));
    }
    let rest = &first.text[indent.min(first.text.len())..];
    if let Some(count) = math_len(src, 0, ctx) {
        return (Kind::Math, count);
    }
    if let Some(fence) = fence_open(rest) {
        return (Kind::Code, fenced_code(src, fence));
    }
    if is_atx(rest) {
        return (Kind::Heading, 1);
    }
    if is_rule(rest) {
        return (Kind::Rule, 1);
    }
    if depth < MAX_DEPTH {
        if rest.starts_with('>') {
            return quote(src, ctx, depth, nodes);
        }
        if marker(first.text).is_some() {
            return (Kind::List, list(src, ctx, depth, nodes));
        }
    }
    // Before reference definitions: a paragraph already ends where a table
    // starts, so the block there has to be one.
    if table_starts(src, 0) {
        return (Kind::Table, table(src, ctx));
    }
    if let Some(count) = ref_def(src, rest) {
        return (Kind::RefDef, count);
    }
    paragraph(src, ctx)
}

fn blocks(lines: &[Line<'_>], ctx: Ctx, depth: u8, nodes: &mut Vec<Node>) {
    let mut i = 0;
    while i < lines.len() {
        if tree::is_blank(lines[i].text) {
            i += 1;
            continue;
        }
        let slot = nodes.len();
        nodes.push(Node {
            kind: Kind::Other,
            depth,
            start: lines[i].at as u32,
            len: 0,
        });
        let mut src = Slice(&lines[i..]);
        let (kind, count) = block(&mut src, ctx, depth, nodes);
        let (end, _) = block_end(&mut src, count);
        nodes[slot].kind = kind;
        nodes[slot].len = end as u32 - nodes[slot].start;
        i += count;
    }
}

fn indent_of(line: &str) -> usize {
    let mut width = 0;
    for byte in line.bytes() {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => break,
        }
    }
    width
}

/// `line` without up to `columns` columns of leading white space.
fn strip_columns(line: &str, columns: usize) -> (usize, &str) {
    let (mut width, mut bytes) = (0, 0);
    for byte in line.bytes() {
        if width >= columns {
            break;
        }
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => break,
        }
        bytes += 1;
    }
    (bytes, &line[bytes..])
}

fn fence_open(rest: &str) -> Option<(u8, usize)> {
    let first = *rest.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let run = rest.bytes().take_while(|&b| b == first).count();
    let info_ok = first == b'~' || !rest[run..].contains('`');
    (run >= 3 && info_ok).then_some((first, run))
}

fn closes_fence(line: &str, (byte, len): (u8, usize)) -> bool {
    if indent_of(line) > 3 {
        return false;
    }
    let rest = line.trim_start().trim_end();
    rest.len() >= len && rest.bytes().all(|b| b == byte)
}

fn fenced_code<'a>(src: &mut impl Source<'a>, fence: (u8, usize)) -> usize {
    let mut k = 1;
    while let Some(line) = src.line(k) {
        k += 1;
        if closes_fence(line.text, fence) {
            break;
        }
    }
    k
}

fn indented_code<'a>(src: &mut impl Source<'a>) -> usize {
    let mut k = 1;
    while let Some(line) = src.line(k) {
        if !tree::is_blank(line.text) && indent_of(line.text) < 4 {
            break;
        }
        k += 1;
    }
    k
}

/// The line count of the `$$` block opening at line `k`, by `math_scan`'s
/// rules; its lines are only joined when the opener could close.
fn math_len<'a>(src: &mut impl Source<'a>, k: usize, ctx: Ctx) -> Option<usize> {
    let first = src.line(k)?;
    if !math_scan::opens_block(first.text) {
        return None;
    }
    let mut held = vec![first.text];
    for j in 1..=math_scan::BLOCK_LOOKAHEAD {
        let line = src.line(k + j)?;
        // Like a blank line, a container tag ends a formula that has not
        // closed: otherwise whether a tag counts would depend on text far
        // below it. (A rule SPEC.md has to state.)
        if tree::is_blank(line.text) || tree::tag_line(line.text, ctx).is_some() {
            return None;
        }
        held.push(line.text);
        if line.text.contains("$$") {
            return math_scan::block_len(&held);
        }
    }
    None
}

/// Where the `$$` block opening at `pos` ends, for the libraries' pre-pass.
pub fn math_block_end(text: &str, pos: usize, ctx: Ctx) -> Option<usize> {
    let mut lines = Vec::new();
    let mut doc = Doc {
        text,
        next: pos,
        ctx,
        lines: &mut lines,
    };
    let count = math_len(&mut doc, 0, ctx)?;
    doc.line(count - 1).map(|l| l.at + l.text.len())
}

fn is_atx(rest: &str) -> bool {
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && matches!(rest.as_bytes().get(hashes), None | Some(b' ' | b'\t'))
}

fn is_rule(rest: &str) -> bool {
    let Some(&first) = rest.as_bytes().first() else {
        return false;
    };
    if !matches!(first, b'-' | b'*' | b'_') {
        return false;
    }
    let mut count = 0;
    for byte in rest.bytes() {
        match byte {
            b if b == first => count += 1,
            b' ' | b'\t' => {}
            _ => return false,
        }
    }
    count >= 3
}

#[derive(Clone, Copy)]
struct Marker {
    /// The bullet, or the ordered delimiter `.`/`)`.
    kind: u8,
    ordered_from_one: Option<bool>,
    /// The column the item's content starts in, and its byte offset.
    column: usize,
    content: usize,
}

fn marker(line: &str) -> Option<Marker> {
    let indent = indent_of(line);
    if indent > 3 {
        return None;
    }
    let bytes = line.as_bytes();
    let start = bytes
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    let first = *bytes.get(start)?;
    let (kind, width, ordered_from_one) = if matches!(first, b'-' | b'+' | b'*') {
        (first, 1, None)
    } else {
        let digits = bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let delimiter = *bytes.get(start + digits)?;
        if !(1..=9).contains(&digits) || !matches!(delimiter, b'.' | b')') {
            return None;
        }
        let one = &line[start..start + digits] == "1";
        (delimiter, digits + 1, Some(one))
    };
    let after = start + width;
    if !matches!(bytes.get(after), None | Some(b' ' | b'\t')) || is_rule(&line[start..]) {
        return None;
    }
    let spaces = bytes[after..]
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    let empty = after + spaces == bytes.len();
    let used = if empty || spaces > 4 { 1 } else { spaces };
    Some(Marker {
        kind,
        ordered_from_one,
        column: indent + width + used,
        content: (after + used).min(bytes.len()),
    })
}

/// Whether `line` (line `k` of `src`) ends a paragraph it would otherwise
/// continue.
fn interrupts<'a>(src: &mut impl Source<'a>, k: usize, line: &str, ctx: Ctx) -> bool {
    if tree::tag_line(line, ctx).is_some() {
        return true;
    }
    let indent = indent_of(line);
    if indent > 3 {
        return false;
    }
    let rest = line.trim_start();
    match rest.as_bytes().first() {
        Some(b'>') => true,
        Some(b'#') => is_atx(rest),
        Some(b'`' | b'~') => fence_open(rest).is_some(),
        Some(b'$') => math_len(src, k, ctx).is_some(),
        Some(_) => {
            is_rule(rest)
                || marker(line)
                    .is_some_and(|m| m.content < line.len() && m.ordered_from_one.unwrap_or(true))
        }
        None => false,
    }
}

/// Whether a following unindented line may lazily continue a container whose
/// last content line is `previous`: only paragraph text takes one.
fn takes_lazy(previous: &str, in_fence: bool) -> bool {
    let rest = previous.trim_start();
    !in_fence
        && !rest.is_empty()
        && indent_of(previous) < 4
        && !is_atx(rest)
        && !is_rule(rest)
        && fence_open(rest).is_none()
        && !rest.starts_with('|')
}

fn track_fence(fence: &mut Option<(u8, usize)>, line: &str) {
    match *fence {
        Some(open) if closes_fence(line, open) => *fence = None,
        Some(_) => {}
        None if indent_of(line) <= 3 => *fence = fence_open(line.trim_start()),
        None => {}
    }
}

fn strip_quote(line: Line<'_>) -> Option<Line<'_>> {
    if indent_of(line.text) > 3 {
        return None;
    }
    let rest = line.text.trim_start().strip_prefix('>')?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let at = line.at + line.text.len() - rest.len();
    Some(Line { at, text: rest })
}

fn quote<'a>(
    src: &mut impl Source<'a>,
    ctx: Ctx,
    depth: u8,
    nodes: &mut Vec<Node>,
) -> (Kind, usize) {
    let mut inner: Vec<Line<'a>> = Vec::new();
    let (mut k, mut fence) = (0, None);
    while let Some(line) = src.line(k) {
        let content = match strip_quote(line) {
            Some(content) => {
                // A nested quote's paragraph takes lazy lines too.
                track_fence(&mut fence, content.text);
                content
            }
            None => {
                let lazy = inner
                    .last()
                    .is_some_and(|p| takes_lazy(p.text, fence.is_some()))
                    && !tree::is_blank(line.text)
                    && !interrupts(src, k, line.text, ctx);
                if !lazy {
                    break;
                }
                line
            }
        };
        inner.push(content);
        k += 1;
    }
    let callout = depth == 0
        && inner
            .first()
            .is_some_and(|l| l.text.starts_with("[!") && l.text.contains(']'));
    blocks(&inner, ctx, depth + 1, nodes);
    (if callout { Kind::Callout } else { Kind::Quote }, k)
}

/// The lines of the list item starting at line `k` with marker `m`, content
/// columns stripped, and the line after them.
fn item_lines<'a>(
    src: &mut impl Source<'a>,
    ctx: Ctx,
    k: usize,
    m: Marker,
    inner: &mut Vec<Line<'a>>,
) -> usize {
    let Some(first) = src.line(k) else {
        return k + 1;
    };
    let head = &first.text[m.content..];
    inner.push(Line {
        at: first.at + m.content,
        text: head,
    });
    let mut fence = None;
    track_fence(&mut fence, head);
    let (mut j, mut after_blank) = (k + 1, false);
    while let Some(line) = src.line(j) {
        let content = if tree::is_blank(line.text) {
            // An item that starts empty ends at the first blank line.
            if j == k + 1 && head.is_empty() {
                break;
            }
            after_blank = true;
            Line {
                at: line.at,
                text: "",
            }
        } else if indent_of(line.text) >= m.column {
            after_blank = false;
            let (bytes, text) = strip_columns(line.text, m.column);
            Line {
                at: line.at + bytes,
                text,
            }
        } else {
            let lazy = !after_blank
                && inner
                    .last()
                    .is_some_and(|p| takes_lazy(p.text, fence.is_some()))
                && marker(line.text).is_none()
                && !interrupts(src, j, line.text, ctx);
            if !lazy {
                break;
            }
            line
        };
        track_fence(&mut fence, content.text);
        inner.push(content);
        j += 1;
    }
    while inner.len() > 1 && tree::is_blank(inner[inner.len() - 1].text) {
        inner.pop();
        j -= 1;
    }
    j
}

fn list<'a>(src: &mut impl Source<'a>, ctx: Ctx, depth: u8, nodes: &mut Vec<Node>) -> usize {
    let mut inner: Vec<Line<'a>> = Vec::new();
    let mut kind = None;
    let (mut k, mut end) = (0, 0);
    loop {
        let Some(line) = src.line(k) else {
            return end;
        };
        if tree::is_blank(line.text) {
            k += 1;
            continue;
        }
        let Some(m) = marker(line.text) else {
            return end;
        };
        if *kind.get_or_insert(m.kind) != m.kind {
            return end;
        }
        inner.clear();
        end = item_lines(src, ctx, k, m, &mut inner);
        let slot = nodes.len();
        let start = line.at as u32;
        nodes.push(Node {
            kind: Kind::Item,
            depth: depth + 1,
            start,
            len: 0,
        });
        blocks(&inner, ctx, depth + 2, nodes);
        let item_end = inner.last().map_or(line.at, |l| l.at + l.text.len()) as u32;
        nodes[slot].len = item_end.max(start + 1) - start;
        k = end;
    }
}

/// A link reference definition: `[label]: destination`, the destination
/// possibly on the next line. Titles on further lines are not handled.
fn ref_def<'a>(src: &mut impl Source<'a>, rest: &str) -> Option<usize> {
    let label_end = rest.strip_prefix('[')?.find("]:")? + 1;
    let label = &rest[1..label_end];
    if label.trim().is_empty() || label.contains('[') || label.starts_with('^') {
        return None;
    }
    let after = rest[label_end + 2..].trim();
    if after.is_empty() {
        let next = src.line(1)?;
        return (!tree::is_blank(next.text)).then_some(2);
    }
    let destination_end = after.find([' ', '\t']).unwrap_or(after.len());
    let title = after[destination_end..].trim();
    let title_ok = title.is_empty() || title.starts_with(['"', '\'', '(']);
    title_ok.then_some(1)
}

fn cells(row: &str) -> usize {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let mut count = 1;
    let mut escaped = false;
    for (i, byte) in row.bytes().enumerate() {
        if byte == b'|' && !escaped && i + 1 < row.len() {
            count += 1;
        }
        escaped = byte == b'\\' && !escaped;
    }
    count
}

fn is_delimiter_row(line: &str) -> bool {
    let row = line.trim();
    indent_of(line) <= 3
        && row.contains('-')
        && row.contains('|')
        && row
            .bytes()
            .all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'))
}

fn table_starts<'a>(src: &mut impl Source<'a>, k: usize) -> bool {
    // The delimiter row is checked first: most lines are not one.
    let Some(delimiter) = src.line(k + 1).filter(|l| is_delimiter_row(l.text)) else {
        return false;
    };
    src.line(k)
        .is_some_and(|h| indent_of(h.text) <= 3 && cells(h.text) == cells(delimiter.text))
}

fn table<'a>(src: &mut impl Source<'a>, ctx: Ctx) -> usize {
    let mut k = 2;
    while let Some(line) = src.line(k) {
        if tree::is_blank(line.text) || interrupts(src, k, line.text, ctx) {
            break;
        }
        k += 1;
    }
    k
}

fn paragraph<'a>(src: &mut impl Source<'a>, ctx: Ctx) -> (Kind, usize) {
    let mut k = 1;
    while let Some(line) = src.line(k) {
        if tree::is_blank(line.text) {
            break;
        }
        if indent_of(line.text) <= 3 && is_underline(line.text.trim()) {
            return (Kind::Heading, k + 1);
        }
        if interrupts(src, k, line.text, ctx) || table_starts(src, k) {
            break;
        }
        k += 1;
    }
    let only = src.line(0).map(|l| l.text).filter(|_| k == 1);
    (only.map_or(Kind::Paragraph, single_line_kind), k)
}

fn is_underline(rest: &str) -> bool {
    !rest.is_empty() && (rest.bytes().all(|b| b == b'=') || rest.bytes().all(|b| b == b'-'))
}

/// A one-line paragraph that is only a formula or only an image is a block of
/// its own in Jade (§5.1). Shared with the libraries' post-pass.
pub fn single_line_kind(line: &str) -> Kind {
    let line = line.trim();
    if line.len() > 4 && line.starts_with("$$") && line[2..].find("$$") == Some(line.len() - 4) {
        return Kind::Math;
    }
    let markdown = line.starts_with("![") && line.ends_with(')') && line.matches("![").count() == 1;
    let embed = line.starts_with("![[") && line.find("]]") == Some(line.len() - 2);
    if markdown || embed {
        Kind::Image
    } else {
        Kind::Paragraph
    }
}
