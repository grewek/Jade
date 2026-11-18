//! Lines that render as a box with a height of their own instead of text. An
//! image embed is the first; real tables and callouts follow. A block is still
//! one line to the document: its height is just that many rows, so the row
//! cache, scrolling and hit-testing treat it like any wrapped line.

use jade_core::link_scan::{ImageEmbed, parse_image_embed_line};

use super::block_preview::LineRole;
use super::element::{LineState, TableLineKind};
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

/// How many rows `embed` takes.
pub(super) fn block_rows(_embed: &ImageEmbed) -> usize {
    PLACEHOLDER_ROWS
}
