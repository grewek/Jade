//! The inline scanner: emphasis, code spans, links, entities.

use super::*;

pub(super) enum ConstructKind {
    Code,
    Strong,
    Emphasis,
    Strikethrough,
    WikiLink,
    /// `\` followed by an ASCII punctuation character — a one-shot match
    /// (no closer to search for), handled directly in `scan_inline`.
    Escape,
    /// `&name;` / `&#NNN;` / `&#xHHH;` — a one-shot match like `Escape`,
    /// resolved by [`parse_entity`].
    Entity,
    /// `[text](url)` — resolved by [`parse_inline_link`]. Not matched at all when
    /// immediately preceded by `!` (an image, deliberately out of scope —
    /// see `scan_inline`'s own handling).
    Link,
    /// `<https://x>` / `<mailto:a@b.com>` / `<a@b.com>` — resolved by
    /// [`parse_autolink`].
    AutolinkBracket,
    /// A bare `http://`/`https://` URL with no surrounding delimiters (GFM
    /// extended autolinks) — a one-shot match, resolved directly in
    /// `scan_inline` since there's no closer to search for.
    BareUrl,
    /// `$x$` / `$$x$$`: opaque, its LaTeX is not Markdown. Drawn as the plain
    /// source until math layout exists (#13).
    Math,
}

pub(super) struct Construct {
    /// Byte offset of the opener, relative to the slice `find_construct` was
    /// called on (the caller adds its own base offset back in).
    offset: usize,
    opener_len: usize,
    kind: ConstructKind,
}

/// The next inline construct's opener in `rest`, if any — a backtick, a
/// doubled or single `*`/`_` run, a doubled `~` run, `[[`, a backslash escape,
/// an entity's `&`, a lone `[`, `<`, or a bare `http(s)://`. A lone `~` (not
/// doubled) isn't a construct at all — unlike `*`/`_`, which have both a
/// doubled (strong) and single (emphasis) form, strikethrough only
/// recognizes the doubled form, so a single `~` is simply skipped over, same
/// as any other plain character. Scans byte-by-byte; safe over multi-byte
/// UTF-8 since none of its continuation bytes can match any of these ASCII
/// patterns, so stepping through one at a time just finds nothing until the
/// next real ASCII delimiter.
pub(super) fn find_construct(rest: &str) -> Option<Construct> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::Escape,
                });
            }
            b'`' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Code,
                });
            }
            b'*' | b'_' => {
                let c = bytes[i];
                return Some(if bytes.get(i + 1) == Some(&c) {
                    Construct {
                        offset: i,
                        opener_len: 2,
                        kind: ConstructKind::Strong,
                    }
                } else {
                    Construct {
                        offset: i,
                        opener_len: 1,
                        kind: ConstructKind::Emphasis,
                    }
                });
            }
            b'~' if bytes.get(i + 1) == Some(&b'~') => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::Strikethrough,
                });
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::WikiLink,
                });
            }
            b'[' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Link,
                });
            }
            b'<' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::AutolinkBracket,
                });
            }
            b'&' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Entity,
                });
            }
            b'h' if rest[i..].starts_with("http://") || rest[i..].starts_with("https://") => {
                return Some(Construct {
                    offset: i,
                    opener_len: 0,
                    kind: ConstructKind::BareUrl,
                });
            }
            b'$' => {
                return Some(Construct {
                    offset: i,
                    opener_len: opener_len(rest, i, rest.len()),
                    kind: ConstructKind::Math,
                });
            }
            _ => i += 1,
        }
    }
    None
}

/// Common named HTML entities this module decodes — not the full ~2000-entry
/// HTML5 named character reference table (a deliberate v1 scope cut, same
/// category as strikethrough's GFM-only doubled-tilde): just a subset
/// plausible in a note's prose.
pub(super) const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{00A0}'),
    ("mdash", '\u{2014}'),
    ("ndash", '\u{2013}'),
    ("hellip", '\u{2026}'),
    ("copy", '\u{00A9}'),
    ("reg", '\u{00AE}'),
    ("trade", '\u{2122}'),
    ("laquo", '\u{00AB}'),
    ("raquo", '\u{00BB}'),
    ("ldquo", '\u{201C}'),
    ("rdquo", '\u{201D}'),
    ("lsquo", '\u{2018}'),
    ("rsquo", '\u{2019}'),
    ("deg", '\u{00B0}'),
    ("plusmn", '\u{00B1}'),
    ("times", '\u{00D7}'),
    ("divide", '\u{00F7}'),
    ("sect", '\u{00A7}'),
    ("para", '\u{00B6}'),
    ("bull", '\u{2022}'),
    ("euro", '\u{20AC}'),
    ("pound", '\u{00A3}'),
    ("yen", '\u{00A5}'),
    ("cent", '\u{00A2}'),
];

/// Parses an HTML entity reference (`&amp;`, `&#39;`, `&#x27;`) starting at
/// `start` (pointing at `&`), if `line[start..end]` contains a recognized
/// one — returns its raw span (including `&`/`;`) and the decoded
/// character. Numeric references decode any valid Unicode scalar value;
/// named references are looked up in [`NAMED_ENTITIES`]. An unrecognized
/// name or invalid numeric value is `None` — the usual "leave it as plain
/// text" fallback every other unmatched construct in this module gets, so
/// e.g. a bare `&` in "Bob & Alice" is untouched.
pub(super) fn parse_entity(line: &str, start: usize, end: usize) -> Option<(Range<usize>, char)> {
    let rest = &line[start..end];
    let semi_rel = rest.find(';')?;
    let body = &rest[1..semi_rel];
    let raw_end = start + semi_rel + 1;

    let ch = if let Some(hex) = body
        .strip_prefix('#')
        .and_then(|b| b.strip_prefix('x').or_else(|| b.strip_prefix('X')))
    {
        if hex.is_empty() || hex.len() > 6 {
            return None;
        }
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if let Some(dec) = body.strip_prefix('#') {
        if dec.is_empty() || dec.len() > 7 {
            return None;
        }
        char::from_u32(dec.parse().ok()?)?
    } else {
        NAMED_ENTITIES
            .iter()
            .find(|(name, _)| *name == body)
            .map(|(_, c)| *c)?
    };
    Some((start..raw_end, ch))
}

/// Scans `line[pos..end]` for inline constructs, appending hidden/visible
/// [`DisplaySegment`]s (and the visible text they show) in raw order. Plain
/// text outside any construct is tagged `default_style` — `Plain` normally,
/// or the enclosing heading's level so e.g. `# **bold** title` still renders
/// its non-bold text at heading size/color.
pub(super) fn scan_inline(
    line: &str,
    pos: usize,
    end: usize,
    default_style: InlineStyle,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
) {
    scan_inline_in(line, pos, end, default_style, true, segments, display_text);
}

/// [`scan_inline`], where `math` says whether a formula may become a drawn
/// [`InlineStyle::Math`] segment: a table cell paints its own text and cannot.
pub(super) fn scan_inline_in(
    line: &str,
    mut pos: usize,
    end: usize,
    default_style: InlineStyle,
    math: bool,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
) {
    let mut plain_start = pos;
    while pos < end {
        let Some(construct) = find_construct(&line[pos..end]) else {
            break;
        };
        let construct_start = pos + construct.offset;

        push_visible(
            line,
            segments,
            display_text,
            plain_start..construct_start,
            default_style,
        );
        plain_start = construct_start;

        let matched = match construct.kind {
            ConstructKind::Code => find_closing(line, construct_start + 1, end, "`").map(|close| {
                push_hidden(segments, display_text, construct_start..construct_start + 1);
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start + 1..close,
                    InlineStyle::Code,
                );
                push_hidden(segments, display_text, close..close + 1);
                close + 1
            }),
            ConstructKind::Strong | ConstructKind::Emphasis => {
                let delim_len = construct.opener_len;
                let delim = &line[construct_start..construct_start + delim_len];
                let style = if matches!(construct.kind, ConstructKind::Strong) {
                    InlineStyle::Bold
                } else {
                    InlineStyle::Italic
                };
                find_closing(line, construct_start + delim_len, end, delim).map(|close| {
                    push_hidden(
                        segments,
                        display_text,
                        construct_start..construct_start + delim_len,
                    );
                    push_visible(
                        line,
                        segments,
                        display_text,
                        construct_start + delim_len..close,
                        style,
                    );
                    push_hidden(segments, display_text, close..close + delim_len);
                    close + delim_len
                })
            }
            ConstructKind::Strikethrough => {
                find_closing(line, construct_start + 2, end, "~~").map(|close| {
                    push_hidden(segments, display_text, construct_start..construct_start + 2);
                    push_visible(
                        line,
                        segments,
                        display_text,
                        construct_start + 2..close,
                        InlineStyle::Strikethrough,
                    );
                    push_hidden(segments, display_text, close..close + 2);
                    close + 2
                })
            }
            ConstructKind::WikiLink => parse_wikilink(line, construct_start, end).map(|link| {
                push_hidden(segments, display_text, construct_start..link.text.start);
                push_visible(
                    line,
                    segments,
                    display_text,
                    link.text.clone(),
                    InlineStyle::WikiLink,
                );
                push_hidden(segments, display_text, link.text.end..link.span.end);
                link.span.end
            }),
            ConstructKind::Escape => {
                push_hidden(segments, display_text, construct_start..construct_start + 1);
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start + 1..construct_start + 2,
                    default_style,
                );
                Some(construct_start + 2)
            }
            ConstructKind::Entity => parse_entity(line, construct_start, end).map(|(raw, ch)| {
                let mut buf = [0u8; 4];
                let decoded = ch.encode_utf8(&mut buf);
                push_replaced(segments, display_text, raw.clone(), decoded, default_style);
                raw.end
            }),
            // `![...]` is an image, explicitly deferred (see this module's
            // doc comment) — left as plain text, same as any other
            // unmatched opener.
            ConstructKind::Link
                if construct_start > 0 && line.as_bytes()[construct_start - 1] == b'!' =>
            {
                None
            }
            ConstructKind::Link => parse_inline_link(line, construct_start, end).map(|link| {
                push_hidden(segments, display_text, construct_start..link.text.start);
                push_visible(
                    line,
                    segments,
                    display_text,
                    link.text.clone(),
                    InlineStyle::Link,
                );
                push_hidden(segments, display_text, link.text.end..link.span.end);
                link.span.end
            }),
            ConstructKind::AutolinkBracket => {
                parse_autolink(line, construct_start, end).map(|link| {
                    push_hidden(segments, display_text, construct_start..link.text.start);
                    push_visible(
                        line,
                        segments,
                        display_text,
                        link.text.clone(),
                        InlineStyle::Link,
                    );
                    push_hidden(segments, display_text, link.text.end..link.span.end);
                    link.span.end
                })
            }
            ConstructKind::Math => parse_math(line, construct_start, end).map(|found| {
                let drawn = math
                    .then(|| rendered_span(&line[found.span.clone()]))
                    .flatten();
                if let Some(drawn) = drawn {
                    let blank = " ".repeat(drawn.columns);
                    push_replaced(
                        segments,
                        display_text,
                        found.span.clone(),
                        &blank,
                        InlineStyle::Math,
                    );
                } else {
                    push_visible(
                        line,
                        segments,
                        display_text,
                        found.span.clone(),
                        default_style,
                    );
                }
                found.span.end
            }),
            ConstructKind::BareUrl => {
                let url_end = parse_bare_url(line, construct_start, end).span.end;
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start..url_end,
                    InlineStyle::Link,
                );
                Some(url_end)
            }
        };

        match matched {
            Some(new_pos) => {
                pos = new_pos;
                plain_start = pos;
            }
            // No closer on this line: leave the opener as plain text (still
            // unflushed, in `plain_start`) and keep scanning right after it.
            None => pos = construct_start + construct.opener_len,
        }
    }
    push_visible(
        line,
        segments,
        display_text,
        plain_start..end,
        default_style,
    );
}

pub(super) fn push_hidden(
    segments: &mut Vec<DisplaySegment>,
    display_text: &str,
    raw: Range<usize>,
) {
    if raw.is_empty() {
        return;
    }
    let at = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: at..at,
        style: InlineStyle::Plain,
        replaced: false,
    });
}

pub(super) fn push_visible(
    line: &str,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
    raw: Range<usize>,
    style: InlineStyle,
) {
    if raw.is_empty() {
        return;
    }
    let display_start = display_text.len();
    display_text.push_str(&line[raw.clone()]);
    let display_end = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: display_start..display_end,
        style,
        replaced: false,
    });
}

/// Like [`push_visible`], but `replacement` is painted in place of `raw`
/// instead of `raw`'s own text — used only for a list marker's bullet/
/// number glyph. `raw` must be non-empty (callers only ever pass an actual
/// marker's byte range).
pub(super) fn push_replaced(
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
    raw: Range<usize>,
    replacement: &str,
    style: InlineStyle,
) {
    let display_start = display_text.len();
    display_text.push_str(replacement);
    let display_end = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: display_start..display_end,
        style,
        replaced: true,
    });
}
