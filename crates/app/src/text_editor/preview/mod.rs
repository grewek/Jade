//! Per-line Live Preview rendering model: how a note's *raw* Markdown line
//! maps to what should actually be painted — hidden markup (heading `#`
//! prefixes, `**`/`*` emphasis delimiters, `` ` `` code fences, `[[`/`]]`/
//! `|target` wikilink syntax) collapsed to zero display width, the remaining
//! text tagged with the style it should be painted in.
//!
//! Deliberately GPUI-free (same pure-logic-first approach `buffer.rs` and
//! every other Live Preview module use) — `element.rs`/`input_handler.rs`
//! consume [`DisplaySegment::style`] to pick an actual `Font`/color, and its
//! raw↔display byte mapping ([`DisplayLine::display_offset_for_raw`]/
//! [`DisplayLine::raw_offset_for_display`]) to convert between a raw buffer
//! offset and an x position in the *display* text they actually shape and
//! paint — the two are no longer the same string once markup is hidden.
//!
//! Scope deliberately narrow for this first pass: headings, bold, italic,
//! inline code, strikethrough, wikilinks, blockquote `>` prefixes, list
//! markers, and tables (`table/model.rs`'s detection, this module's
//! `table::geometry` rendering) — not the full
//! CommonMark/GFM grammar (no autolinks). Emphasis matching is also
//! simplified: `**x**`/`__x__` is bold, a lone `*x*`/`_x_` is italic,
//! without CommonMark's full left/right-flanking delimiter-run algorithm —
//! a deliberate v1 scope cut. Strikethrough (`~~x~~`) only recognizes the
//! doubled-tilde GFM form, mirroring Strong's `**`/`__` pairing — unlike GitHub's own
//! renderer, a lone `~x~` isn't treated as anything special and is left as
//! plain text, since GFM itself doesn't define single-tilde strikethrough
//! either (that's a cmark-gfm extension option GitHub doesn't enable).
//!
//! Most detection in this module (headings, blockquote `>` prefixes, list
//! markers, one level of compounding) is fully determined by a single
//! line's own content, needing no whole-document state — unlike fenced code
//! (`preview/fence.rs`'s own concern; whether a line sits inside an open
//! fence depends on earlier lines). Two things *do* need cross-line state
//! though: an indented list marker's nesting *depth* (relative to
//! whichever ancestor items are still open), and whether a line with no
//! marker of its own still continues an open blockquote/list (CommonMark
//! "lazy continuation"). [`classify_block_context`] is this module's own
//! single sequential pass resolving both — the same kind of whole-document
//! state machine `preview::fence::classify_lines` already uses for fences,
//! just tracking list-indentation depth and open-block state instead of
//! fence delimiters. `element.rs`'s `render_editor` computes it once per
//! paint (like `classify_lines`) and feeds each line's resulting
//! [`LineContext`] into [`build_display_line_in_context`] instead of
//! calling [`build_display_line`] directly — the latter, being pure
//! per-line, has no way to know a marker is nested or a line is a lazy
//! continuation.
//!
//! List markers are hidden like any other markup, but — unlike a `**`/`#`,
//! which simply vanishes — are *replaced* with a bullet (`•`) or a
//! canonical `N. `, not left blank: see [`DisplaySegment::is_replaced`].
//!
//! The line containing the cursor is never hidden — `build_display_line`'s
//! `active` flag turns rendering off entirely for it, showing the literal
//! raw text unstyled. (A "real" Live Preview editor keeps *formatting*
//! visible while editing and hides only the delimiter characters; doing that
//! too is a reasonable follow-up, not required for this first pass given the
//! narrow construct list above.)

use std::ops::Range;
use std::rc::Rc;

use super::math::rendered_span;
use callout::{CalloutType, callout_header_prefix};
use jade_core::link_scan::{
    find_closing, parse_autolink, parse_bare_url, parse_inline_link, parse_wikilink,
};
use jade_core::math_scan::{opener_len, parse_inline as parse_math};

pub(in crate::text_editor) mod callout;
pub(in crate::text_editor) mod fence;

mod build;
mod context;
mod inline;
mod line;

// What the rest of the editor uses. Everything else in the sibling files is
// `pub(super)`: visible to `preview` and its tests, not beyond.
pub(in crate::text_editor) use build::{
    build_callout_header_line, build_cell_line, build_display_line, build_display_line_in_context,
    hidden_line, is_thematic_break, raw_cell_line,
};
pub(in crate::text_editor) use context::{LineContext, OpenBlock, block_context_step, line_indent};
pub(in crate::text_editor) use fence::LineRole;
pub(in crate::text_editor) use line::{
    DisplayLine, DisplaySegment, InlineStyle, LineIndent, Widget,
};

// Siblings and the tests reach each other's items through this module.
use {build::*, context::*, inline::*, line::*};

#[cfg(test)]
pub(in crate::text_editor) use context::{classify_block_context, classify_block_context_from};

#[cfg(test)]
mod tests;
