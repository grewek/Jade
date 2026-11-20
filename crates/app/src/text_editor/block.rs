//! Lines that render as a box with a height of their own instead of text. An
//! image embed is the first; real tables and callouts follow. A block is still
//! one line to the document: its height is just that many rows, so the row
//! cache, scrolling and hit-testing treat it like any wrapped line.

use gpui_kit::gpui::Pixels;
use jade_core::link_scan::{ImageEmbed, parse_image_embed_line};

use super::block_preview::LineRole;
use super::document_cache::DocumentCache;
use super::element::{LineState, TableLineKind};
use super::images::{self, ImageSizes};
use super::live_preview::LineContext;

/// Rows an image block takes until the image's real size is known.
pub(super) const PLACEHOLDER_ROWS: usize = 6;

/// `line` as an image block, when it is a whole-line vault-local embed on an
/// ordinary line: not code, a table, a callout or a list, and not the line the
/// cursor is on (that shows its source).
pub(super) fn image_block(line: &str, state: &LineState) -> Option<ImageEmbed> {
    let plain = matches!(state.role, LineRole::Plain)
        && matches!(state.block_ctx, LineContext::Plain)
        && matches!(state.table_kind, TableLineKind::None)
        && state.callout.is_none();
    (plain && !state.active)
        .then(|| image_embed_line(line))
        .flatten()
}

/// `line` as an image embed, whatever its surroundings (see [`image_block`]).
pub(super) fn image_embed_line(line: &str) -> Option<ImageEmbed> {
    may_be_image(line)
        .then(|| parse_image_embed_line(line))
        .flatten()
}

/// A byte check that rules out nearly every line before anything is parsed.
pub(super) fn may_be_image(line: &str) -> bool {
    match line.as_bytes().first() {
        Some(b'!') => true,
        Some(b' ' | b'\t') => line.trim_start().starts_with("!["),
        _ => false,
    }
}

/// The distinct targets of image embeds on `lines` whose size is not known yet.
pub(super) fn unknown_image_targets(
    text: &str,
    doc: &DocumentCache,
    lines: std::ops::Range<usize>,
) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for i in lines.start..lines.end.min(doc.line_count()) {
        let line = &text[doc.line_start_byte(i)..doc.line_end_byte(i)];
        if !may_be_image(line)
            || !matches!(doc.line_roles()[i], LineRole::Plain)
            || !matches!(doc.block_contexts()[i], LineContext::Plain)
        {
            continue;
        }
        let Some(embed) = image_embed_line(line) else {
            continue;
        };
        let target = &line[embed.target];
        if !doc.images().is_known(target) && !targets.iter().any(|t| t == target) {
            targets.push(target.to_string());
        }
    }
    targets
}

/// How many rows `embed` (written in `line`) takes: its real height once the
/// header has been read, the placeholder's until then or when it cannot be.
pub(super) fn block_rows(
    line: &str,
    embed: &ImageEmbed,
    sizes: Option<&ImageSizes>,
    available_width: Pixels,
    line_height: Pixels,
) -> usize {
    let natural = sizes.and_then(|s| s.get(&line[embed.target.clone()]));
    match natural {
        Some(natural) => {
            let wiki = line.trim_start().starts_with("![[");
            let requested = images::requested_width(&line[embed.alt.clone()], wiki);
            images::rows_for(natural, requested, available_width, line_height)
        }
        None => PLACEHOLDER_ROWS,
    }
}
