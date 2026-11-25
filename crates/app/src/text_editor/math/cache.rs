//! Laid-out formulas, cached by source: a layout is in em, so it does not depend
//! on the font size, and the same formula is looked up once when a line's text is
//! built (for its width) and again on every frame that paints it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{Layout, Macros, MathFont, layout, parse};

/// Advance of the editor's monospace font in em (IBM Plex Mono). A formula
/// reserves that many display columns per advance, so a line's text and the
/// formula drawn over it agree on width.
pub const MONO_ADVANCE_EM: f32 = 0.6;

/// How much larger a formula is drawn than the surrounding text. The math font
/// is lighter and has a smaller x-height than the editor font, so at 1.0 it
/// reads small next to the text. Reserved columns and painting both use it.
pub const MATH_SCALE: f32 = 1.2;

/// The formula size relative to the body font: the built-in 1.2 times the
/// user's `font.math_scale`.
pub fn scale() -> f32 {
    MATH_SCALE * crate::settings::math_scale()
}

/// Most normal rows a row holding a formula may be made.
const MAX_ROW_SCALE: usize = 16;

/// How many normal rows (of `line_height` px) one text row must span so a
/// formula reaching `formula` = (above, below) the baseline stays inside it.
/// `text` = (ascent, descent) of the row's font: GPUI centres that box in the
/// row, so the baseline sits `(asc - desc) / 2` below the row's middle. At
/// least 1, at most [`MAX_ROW_SCALE`].
pub fn rows_to_fit(formula: (f32, f32), text: (f32, f32), line_height: f32) -> usize {
    assert!(line_height > 0.0);
    let lean = (text.0 - text.1) / 2.0;
    let half = (formula.0 - lean).max(formula.1 + lean);
    // The 1e-3 keeps a formula that fits exactly from rounding up to a row more.
    let rows = (2.0 * half / line_height - 1e-3).ceil();
    (rows.max(1.0) as usize).min(MAX_ROW_SCALE)
}

/// How many rows of `line_height` px a display block whose formula reaches
/// `formula` = (above, below) the baseline, in em, takes when drawn at
/// `font_size` px: its height plus half a row of air, at least one row.
pub fn block_rows(formula: (f32, f32), font_size: f32, line_height: f32) -> usize {
    assert!(line_height > 0.0);
    let height = (formula.0 + formula.1) * font_size * scale() + line_height / 2.0;
    ((height / line_height - 1e-3).ceil().max(1.0) as usize).min(MAX_BLOCK_ROWS)
}

/// Most rows a display block may be given.
const MAX_BLOCK_ROWS: usize = 64;

/// Most formulas kept; the cache is dropped whole when it grows past this.
const MAX_ENTRIES: usize = 2048;

/// A formula ready to paint, and the display columns it reserves.
#[derive(Debug)]
pub struct Rendered {
    pub layout: Layout,
    pub columns: usize,
}

/// The macro set (see [`Macros::stamp`]) a formula was made under, and the result.
type Formulas = HashMap<String, (u64, Option<Arc<Rendered>>)>;

/// Index 0 holds inline formulas, 1 display ones.
static CACHE: Mutex<[Option<Formulas>; 2]> = Mutex::new([None, None]);

/// The formula between the delimiters of `span` (`$x$` or `$$x$$`), or `None`
/// if the font is missing or the source does not parse cleanly; the caller then
/// shows the source as typed.
pub fn rendered_span(span: &str, macros: &Macros) -> Option<Arc<Rendered>> {
    let body = span_body(span)?;
    rendered(body, span.starts_with("$$"), macros)
}

/// The source between the delimiters of `span` (`$x$` or `$$x$$`).
pub fn span_body(span: &str) -> Option<&str> {
    let delim = span.bytes().take_while(|&b| b == b'$').count().min(2);
    span.get(delim..span.len().checked_sub(delim)?)
}

pub fn rendered(source: &str, display: bool, macros: &Macros) -> Option<Arc<Rendered>> {
    let slot = usize::from(display);
    // Poisoning only means another thread panicked mid-insert; the map is still
    // a valid cache.
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache[slot].get_or_insert_with(HashMap::new);
    if let Some((stamp, hit)) = map.get(source)
        && *stamp == macros.stamp()
    {
        return hit.clone();
    }
    let made = build(source, display, macros);
    if map.len() >= MAX_ENTRIES {
        map.clear();
    }
    map.insert(source.to_owned(), (macros.stamp(), made.clone()));
    made
}

fn build(source: &str, display: bool, macros: &Macros) -> Option<Arc<Rendered>> {
    let font = MathFont::bundled()?;
    // A macro that expands without end is not a formula: shown as source.
    let expanded = macros.expand(source).ok()?;
    let formula = parse(&expanded);
    if formula.has_error {
        return None;
    }
    let layout = layout(font, &formula.nodes, display);
    let columns = (layout.width * scale() / MONO_ADVANCE_EM).ceil().max(1.0) as usize;
    Some(Arc::new(Rendered { layout, columns }))
}
