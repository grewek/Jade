//! The one definition of Jade's link grammar. The editor's Live Preview, the
//! vault's backlink index and (later) click-through, rename and backlink
//! context all read links through this module, so they cannot disagree about
//! what a link is.
//!
//! Everything works on one line (a link never spans a line break) and returns
//! byte ranges into it, so nothing here allocates or copies text. The leaf
//! parsers (`parse_*`) are public because the renderer interleaves them with
//! its own emphasis and code handling; [`scan_line`] is the standalone walk.
//!
//! Recognized: `[[target#heading|alias]]`, `[text](dest "title")`, `<url>` and
//! `<email>`, and a bare `http(s)://` URL. Not links (as in rendering): an
//! image `![t](u)`, anything inside an inline code span, an escaped opener.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// `[[target#heading|alias]]`
    Wiki,
    /// `[text](dest)`
    Inline,
    /// `<https://x>`, `<mailto:a@b.c>` or `<a@b.c>`
    Autolink,
    /// `https://x` with no delimiters
    BareUrl,
}

/// One link in a line. All ranges are byte offsets into that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// The whole construct, delimiters included.
    pub span: Range<usize>,
    /// What Live Preview shows: the alias, else the target (without its
    /// `#heading`), the link text, or the URL.
    pub text: Range<usize>,
    /// Where it points: the wikilink target (trimmed), the inline link's
    /// destination (title and `<>` removed), or the URL.
    pub dest: Range<usize>,
    /// A wikilink's `#heading` (trimmed, never empty).
    pub heading: Option<Range<usize>>,
    /// A wikilink's `|alias` (not trimmed); equal to `text` when present.
    pub alias: Option<Range<usize>>,
}

/// Emphasis nests at most this deep when scanning; deeper content is
/// scanned as if the extra delimiters were plain text.
const MAX_DEPTH: usize = 16;

/// The URL scheme of a link destination (`https` in `https://x`, `mailto`),
/// if it has one. A single letter is a Windows drive (`C:\\x`), not a scheme.
pub fn scheme(dest: &str) -> Option<&str> {
    let scheme = &dest[..dest.find(':')?];
    let valid = scheme.len() >= 2
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'));
    valid.then_some(scheme)
}

/// The bytes that can start something `scan_range` reacts to.
static OPENS: [bool; 256] = {
    let mut table = [false; 256];
    let opens = *b"\\`*_~[<h";
    let mut i = 0;
    while i < opens.len() {
        table[opens[i] as usize] = true;
        i += 1;
    }
    table
};

/// Calls `f` for each link in `line`, left to right. Ranges are relative to
/// `line`, which is the whole raw line: the block prefix (blockquote, list
/// marker, heading hashes) is skipped here, as rendering does, so a bullet
/// `*` is not taken for emphasis.
pub fn scan_line(line: &str, mut f: impl FnMut(Link)) {
    scan_range(line, inline_start(line), line.len(), 0, &mut f);
}

/// Where the inline content of `line` begins: after any blockquote markers
/// (`>`, nestable), then a list marker (`-`, `*`, `+`, `1.` or `1)`) with an
/// optional task checkbox, or a heading's `#`s. This is the part of block
/// parsing that link scanning needs; Live Preview's own block pass remains
/// the authority for rendering, and a test compares the two.
pub fn inline_start(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0;
    loop {
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'>') {
            break;
        }
        i += 1;
        if bytes.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    let rest = &bytes[i..];
    let at_boundary = |n: usize| matches!(rest.get(n), Some(b' ' | b'\t'));
    if matches!(rest.first(), Some(b'-' | b'*' | b'+')) && at_boundary(1) {
        i += 2;
        for checkbox in [b"[ ] ", b"[x] ", b"[X] "] {
            if bytes[i..].starts_with(checkbox) {
                i += 4;
                break;
            }
        }
        return i;
    }
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&digits)
        && matches!(rest.get(digits), Some(b'.' | b')'))
        && at_boundary(digits + 1)
    {
        return i + digits + 2;
    }
    let hashes = rest.iter().take_while(|&&b| b == b'#').count();
    if (1..=6).contains(&hashes) && at_boundary(hashes) {
        return i + hashes + 1;
    }
    i
}

/// Scans `line[start..end]`. Bold, italic and strikethrough bound their
/// content like rendering does (a link cannot reach past its closer) but the
/// content is still scanned, so `**[[a]]**` is a link.
fn scan_range(line: &str, start: usize, end: usize, depth: usize, f: &mut impl FnMut(Link)) {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < end {
        // Most bytes open nothing; skip them without the match below.
        match bytes[i..end].iter().position(|&b| OPENS[b as usize]) {
            Some(offset) => i += offset,
            None => return,
        }
        let mut matched = None;
        let skip = match bytes[i] {
            b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => 2,
            // Like rendering: the next unescaped backtick closes the span.
            b'`' => find_closing(line, i + 1, end, "`").map_or(1, |close| close + 1 - i),
            b'*' | b'_' | b'~' => {
                let delim_len = match bytes.get(i + 1) {
                    Some(&next) if next == bytes[i] => 2,
                    _ if bytes[i] == b'~' => 0, // a lone `~` is plain text
                    _ => 1,
                };
                let closed = (delim_len > 0 && depth < MAX_DEPTH)
                    .then(|| find_closing(line, i + delim_len, end, &line[i..i + delim_len]))
                    .flatten();
                match closed {
                    Some(close) => {
                        scan_range(line, i + delim_len, close, depth + 1, f);
                        close + delim_len - i
                    }
                    None => delim_len.max(1),
                }
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                matched = parse_wikilink(line, i, end);
                2
            }
            b'[' if i > 0 && bytes[i - 1] == b'!' => 1,
            b'[' => {
                matched = parse_inline_link(line, i, end);
                1
            }
            b'<' => {
                matched = parse_autolink(line, i, end);
                1
            }
            b'h' if line[i..end].starts_with("http://") || line[i..end].starts_with("https://") => {
                matched = Some(parse_bare_url(line, i, end));
                1
            }
            _ => 1,
        };
        i = match matched {
            Some(link) => {
                let next = link.span.end;
                f(link);
                next
            }
            None => i + skip,
        };
    }
}

/// The absolute byte offset of the first `delim` in `line[start..end]` that
/// is not preceded by a backslash. A run of backslashes is not counted: any
/// backslash escapes, a deliberate simplification shared with rendering.
pub fn find_closing(line: &str, start: usize, end: usize, delim: &str) -> Option<usize> {
    let mut search_start = start;
    loop {
        let abs = search_start + line[search_start..end].find(delim)?;
        if abs > 0 && line.as_bytes()[abs - 1] == b'\\' {
            search_start = abs + 1;
            continue;
        }
        return Some(abs);
    }
}

fn trimmed(line: &str, range: Range<usize>) -> Range<usize> {
    let s = &line[range.clone()];
    let lead = s.len() - s.trim_start().len();
    let start = range.start + lead;
    start..start + s[lead..].trim_end().len()
}

/// `[[...]]` at `start` (the first `[`), within `line[..end]`. Not a link:
/// empty brackets, an empty target (`[[#h]]`), or an empty alias (`[[a|]]`).
pub fn parse_wikilink(line: &str, start: usize, end: usize) -> Option<Link> {
    let inner_start = start + 2;
    let inner_end = inner_start + line[inner_start..end].find("]]")?;
    let inner = &line[inner_start..inner_end];
    let (target_part_end, alias) = match inner.find('|') {
        Some(pipe) => {
            let alias_start = inner_start + pipe + 1;
            if alias_start >= inner_end {
                return None;
            }
            (inner_start + pipe, Some(alias_start..inner_end))
        }
        None => (inner_end, None),
    };
    let (target_end, heading) = match line[inner_start..target_part_end].find('#') {
        Some(hash) => (
            inner_start + hash,
            Some(trimmed(line, inner_start + hash + 1..target_part_end)),
        ),
        None => (target_part_end, None),
    };
    let dest = trimmed(line, inner_start..target_end);
    if dest.is_empty() {
        return None;
    }
    Some(Link {
        kind: LinkKind::Wiki,
        span: start..inner_end + 2,
        text: alias.clone().unwrap_or(inner_start..target_end),
        dest,
        heading: heading.filter(|h| !h.is_empty()),
        alias,
    })
}

/// `[text](dest)` at `start` (the `[`), within `line[..end]`. The text may be
/// empty. The destination ends at the first `)` that is not escaped (no
/// nested-paren balancing), minus an optional `"title"` or surrounding `<>`.
pub fn parse_inline_link(line: &str, start: usize, end: usize) -> Option<Link> {
    let text_start = start + 1;
    let text_close = find_closing(line, text_start, end, "]")?;
    if line.as_bytes().get(text_close + 1) != Some(&b'(') {
        return None;
    }
    let url_start = text_close + 2;
    let url_close = find_closing(line, url_start, end, ")")?;
    let body = trimmed(line, url_start..url_close);
    let rest = &line[body.clone()];
    let dest = match rest.strip_prefix('<') {
        Some(inner) => match inner.find('>') {
            Some(gt) => body.start + 1..body.start + 1 + gt,
            None => body,
        },
        None => {
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            body.start..body.start + len
        }
    };
    Some(Link {
        kind: LinkKind::Inline,
        span: start..url_close + 1,
        text: text_start..text_close,
        dest,
        heading: None,
        alias: None,
    })
}

/// `<...>` at `start`: a URI (contains `://` or starts with `mailto:`) or an
/// email (contains `@`), with no whitespace. Other angle brackets are raw
/// HTML, which Jade does not render.
pub fn parse_autolink(line: &str, start: usize, end: usize) -> Option<Link> {
    let inner_start = start + 1;
    let close = find_closing(line, inner_start, end, ">")?;
    let inner = &line[inner_start..close];
    if inner.is_empty() || inner.chars().any(char::is_whitespace) {
        return None;
    }
    let is_uri = inner.contains("://") || inner.starts_with("mailto:");
    if !is_uri && !inner.contains('@') {
        return None;
    }
    Some(Link {
        kind: LinkKind::Autolink,
        span: start..close + 1,
        text: inner_start..close,
        dest: inner_start..close,
        heading: None,
        alias: None,
    })
}

/// A byte plausible inside a bare URL.
fn is_url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b':'
                | b'/'
                | b'?'
                | b'#'
                | b'['
                | b']'
                | b'@'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b'%'
        )
}

/// A bare `http(s)://` URL at `start`: every following URL byte, minus one
/// trailing `.`, `,`, `;`, `:`, `!` or `?` (a simplified GFM rule).
pub fn parse_bare_url(line: &str, start: usize, end: usize) -> Link {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < end && is_url_byte(bytes[i]) {
        i += 1;
    }
    if i > start && matches!(bytes[i - 1], b'.' | b',' | b';' | b':' | b'!' | b'?') {
        i -= 1;
    }
    Link {
        kind: LinkKind::BareUrl,
        span: start..i,
        text: start..i,
        dest: start..i,
        heading: None,
        alias: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(line: &str) -> Vec<Link> {
        let mut out = Vec::new();
        scan_line(line, |l| out.push(l));
        out
    }

    fn text<'a>(line: &'a str, r: &Range<usize>) -> &'a str {
        &line[r.clone()]
    }

    #[test]
    fn wikilink_forms() {
        let line = "a [[Note]] b [[Note#Head|shown]] c [[ Spaced # Head ]]";
        let found = links(line);
        assert_eq!(found.len(), 3);
        assert_eq!(text(line, &found[0].dest), "Note");
        assert_eq!(text(line, &found[0].text), "Note");
        assert_eq!(found[0].heading, None);
        assert_eq!(text(line, &found[1].dest), "Note");
        assert_eq!(text(line, &found[1].text), "shown");
        assert_eq!(text(line, found[1].heading.as_ref().unwrap()), "Head");
        assert_eq!(text(line, found[1].alias.as_ref().unwrap()), "shown");
        assert_eq!(text(line, &found[2].dest), "Spaced");
        assert_eq!(text(line, found[2].heading.as_ref().unwrap()), "Head");
        assert_eq!(text(line, &found[0].span), "[[Note]]");
    }

    #[test]
    fn heading_before_alias_and_both_missing_parts() {
        let line = "[[A#h|x]] [[A|x]] [[A#]]";
        let found = links(line);
        assert_eq!(found.len(), 3);
        assert_eq!(text(line, &found[0].dest), "A");
        assert_eq!(found[1].heading, None);
        assert_eq!(found[2].heading, None, "an empty heading is no heading");
    }

    #[test]
    fn invalid_wikilinks_are_not_links() {
        for line in ["[[]]", "[[#h]]", "[[a|]]", "[[ ]]", "[[open", "[[a]"] {
            assert!(links(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn inline_link_destination_drops_title_and_angle_brackets() {
        let line = "[t](https://x.y/z) [u](<my file.md> \"T\") [v](a.md \"T\") [](q)";
        let found = links(line);
        assert_eq!(found.len(), 4);
        assert_eq!(text(line, &found[0].dest), "https://x.y/z");
        assert_eq!(text(line, &found[0].text), "t");
        assert_eq!(text(line, &found[1].dest), "my file.md");
        assert_eq!(text(line, &found[2].dest), "a.md");
        assert_eq!(text(line, &found[3].text), "");
        assert_eq!(text(line, &found[3].dest), "q");
    }

    #[test]
    fn autolinks_and_bare_urls() {
        let line = "<https://a.b> <me@x.y> <div> see https://c.d/e. and <a b://c>";
        let found = links(line);
        let kinds: Vec<_> = found.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [LinkKind::Autolink, LinkKind::Autolink, LinkKind::BareUrl]
        );
        assert_eq!(text(line, &found[0].dest), "https://a.b");
        assert_eq!(text(line, &found[2].dest), "https://c.d/e", "trailing dot");
    }

    #[test]
    fn code_spans_escapes_and_images_hide_links() {
        for line in [
            "`[[a]]`",
            "`[t](u)`",
            "\\[[a]]",
            "\\[t](u)",
            "![t](u)",
            "`https://x.y`",
        ] {
            assert!(links(line).is_empty(), "{line}");
        }
        // An unclosed backtick is a literal one.
        assert_eq!(links("` [[a]]").len(), 1);
        // A link right after a code span is still found.
        assert_eq!(links("`c` [[a]]").len(), 1);
    }

    #[test]
    fn links_inside_emphasis_and_next_to_text_are_found() {
        let line = "**[[a]]** and *[t](u)* and ~~<https://x.y>~~";
        assert_eq!(links(line).len(), 3);
    }

    #[test]
    fn scheme_detection() {
        assert_eq!(scheme("https://x.y"), Some("https"));
        assert_eq!(scheme("mailto:a@b.c"), Some("mailto"));
        assert_eq!(scheme("git+ssh://h/r"), Some("git+ssh"));
        assert_eq!(scheme("C:\\dir\\f.md"), None, "a drive letter");
        assert_eq!(scheme("notes/a.md"), None);
        assert_eq!(scheme("a:b"), None, "one letter");
        assert_eq!(scheme("1a:b"), None);
        assert_eq!(scheme(""), None);
    }

    #[test]
    fn block_prefixes_are_skipped_so_markers_are_not_emphasis() {
        for (line, start) in [
            ("* item", 2),
            ("  - [ ] task", 8),
            ("1. one", 3),
            ("12) two", 4),
            ("> > quoted", 4),
            ("> - item", 4),
            ("## Heading", 3),
            ("#nospace", 0),
            ("*not a list*", 0),
            ("-", 0),
            ("plain", 0),
        ] {
            assert_eq!(inline_start(line), start, "{line:?}");
        }
        let line = "* a [[x]] * b [[y]]";
        assert_eq!(links(line).len(), 2);
    }

    #[test]
    fn multibyte_text_keeps_ranges_on_char_boundaries() {
        let line = "é [[Ünï|ä]] ✓ [日本](ü.md) <https://ö.de>";
        for l in links(line) {
            for r in [&l.span, &l.text, &l.dest] {
                assert!(line.is_char_boundary(r.start) && line.is_char_boundary(r.end));
            }
        }
        assert_eq!(links(line).len(), 3);
    }

    #[test]
    fn arbitrary_garbage_never_panics_and_stays_in_bounds() {
        let alphabet = [
            '[', ']', '(', ')', '<', '>', '`', '\\', '|', '#', 'h', 't', 'p', ':', '/', ' ', 'é',
            '!', '"',
        ];
        let mut state = 0x2545_f491_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 24) as usize;
            let line: String = (0..len)
                .map(|_| alphabet[next() as usize % alphabet.len()])
                .collect();
            let mut last_end = 0;
            scan_line(&line, |l| {
                assert!(l.span.start >= last_end, "overlap in {line:?}");
                last_end = l.span.end;
                for r in [&l.span, &l.text, &l.dest] {
                    assert!(r.start <= r.end && r.end <= line.len(), "{line:?}");
                    assert!(line.is_char_boundary(r.start) && line.is_char_boundary(r.end));
                }
                assert!(l.span.start <= l.text.start && l.text.end <= l.span.end);
            });
        }
    }
}
