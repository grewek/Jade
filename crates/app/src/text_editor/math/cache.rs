//! Laid-out formulas, cached by source: a layout is in em, so it does not depend
//! on the font size, and the same formula is looked up once when a line's text is
//! built (for its width) and again on every frame that paints it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{Layout, MathFont, layout, parse};

/// Advance of the editor's monospace font in em (IBM Plex Mono). A formula
/// reserves that many display columns per advance, so a line's text and the
/// formula drawn over it agree on width.
pub const MONO_ADVANCE_EM: f32 = 0.6;

/// How much larger a formula is drawn than the surrounding text. The math font
/// is lighter and has a smaller x-height than the editor font, so at 1.0 it
/// reads small next to the text. Reserved columns and painting both use it.
pub const MATH_SCALE: f32 = 1.2;

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

/// Most formulas kept; the cache is dropped whole when it grows past this.
const MAX_ENTRIES: usize = 2048;

/// A formula ready to paint, and the display columns it reserves.
#[derive(Debug)]
pub struct Rendered {
    pub layout: Layout,
    pub columns: usize,
}

type Formulas = HashMap<String, Option<Arc<Rendered>>>;

/// Index 0 holds inline formulas, 1 display ones.
static CACHE: Mutex<[Option<Formulas>; 2]> = Mutex::new([None, None]);

/// The formula between the delimiters of `span` (`$x$` or `$$x$$`), or `None`
/// if the font is missing or the source does not parse cleanly; the caller then
/// shows the source as typed.
pub fn rendered_span(span: &str) -> Option<Arc<Rendered>> {
    let delim = span.bytes().take_while(|&b| b == b'$').count().min(2);
    let body = span.get(delim..span.len().checked_sub(delim)?)?;
    rendered(body, delim == 2)
}

pub fn rendered(source: &str, display: bool) -> Option<Arc<Rendered>> {
    let slot = usize::from(display);
    // Poisoning only means another thread panicked mid-insert; the map is still
    // a valid cache.
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache[slot].get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(source) {
        return hit.clone();
    }
    let made = build(source, display);
    if map.len() >= MAX_ENTRIES {
        map.clear();
    }
    map.insert(source.to_owned(), made.clone());
    made
}

fn build(source: &str, display: bool) -> Option<Arc<Rendered>> {
    let font = MathFont::bundled()?;
    let formula = parse(source);
    if formula.has_error {
        return None;
    }
    let layout = layout(font, &formula.nodes, display);
    let columns = (layout.width * MATH_SCALE / MONO_ADVANCE_EM)
        .ceil()
        .max(1.0) as usize;
    Some(Arc::new(Rendered { layout, columns }))
}
