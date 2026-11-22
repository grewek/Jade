//! Image embeds: what an embed line is (`embed`), what is known about its file
//! (`info`: the size read from the header, so a block's height is right before
//! any pixel is drawn), and the decoded pixels shared by every note (`cache`).
//!
//! The rest of the editor sees only the items re-exported here.

mod cache;
mod embed;
mod info;

pub(super) use cache::{Cache, decode};
#[cfg(test)]
pub(super) use embed::PLACEHOLDER_ROWS;
pub(super) use embed::{embed, embed_line, may_be_image, rows, unknown_targets};
pub(super) use info::{Info, Sizes, display_size, read_info, requested_width};
