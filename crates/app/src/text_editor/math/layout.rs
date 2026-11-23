//! Formula tree to positioned glyphs and rules, after TeX's math typesetting
//! (TeXbook appendix G) with the constants the OpenType MATH table provides.
//! Pure: no GPUI, and in em, so one layout serves every font size and is
//! cached per formula. A glyph is its font id, its baseline position and a
//! size (1.0, or smaller in scripts); a painter draws it with the same font.

use ttf_parser::GlyphId;

use super::alphabet;
use super::ast::{Accent, Class, Font, FracKind, Limits, MathStyle, Node, OpName};
use super::font::{Consts, MathFont};

mod marks;
mod stretch;

/// What a painter draws. `y` is down from the formula's baseline.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Glyph {
        id: u16,
        x: f32,
        y: f32,
        /// 1.0 is the formula's size.
        size: f32,
        /// Source that did not parse or lay out, drawn in the error colour.
        error: bool,
    },
    /// A filled rectangle: fraction bars, root overbars. `y` is its top edge.
    Rule {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

impl Item {
    fn shifted(self, dx: f32, dy: f32) -> Item {
        match self {
            Item::Glyph {
                id,
                x,
                y,
                size,
                error,
            } => Item::Glyph {
                id,
                x: x + dx,
                y: y + dy,
                size,
                error,
            },
            Item::Rule {
                x,
                y,
                width,
                height,
            } => Item::Rule {
                x: x + dx,
                y: y + dy,
                width,
                height,
            },
        }
    }
}

/// A laid-out formula or part of one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    /// Above the baseline.
    pub ascent: f32,
    /// Below the baseline.
    pub descent: f32,
    pub items: Vec<Item>,
    /// The italic correction of a lone glyph: room a superscript or the next
    /// atom wants after it, not in `width`.
    italic: f32,
}

impl Layout {
    /// Puts `other` in, its origin `dx` right and `dy` below ours.
    fn place(&mut self, other: Layout, dx: f32, dy: f32) {
        self.ascent = self.ascent.max(other.ascent - dy);
        self.descent = self.descent.max(other.descent + dy);
        self.width = self.width.max(dx + other.width);
        self.items
            .extend(other.items.into_iter().map(|item| item.shifted(dx, dy)));
    }

    /// Appends `other` after the current width, on the same baseline.
    fn push(&mut self, other: Layout) {
        let at = self.width;
        self.place(other, at, 0.0);
    }

    fn space(&mut self, width: f32) {
        self.width += width;
    }

    fn rule(&mut self, x: f32, top: f32, width: f32, height: f32) {
        self.ascent = self.ascent.max(-top);
        self.descent = self.descent.max(top + height);
        self.width = self.width.max(x + width);
        self.items.push(Item::Rule {
            x,
            y: top,
            width,
            height,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// TeX's style: a size level and whether the formula is cramped (superscripts
/// sit lower under a bar or in a denominator).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    level: Level,
    cramped: bool,
}

impl Style {
    fn new(level: Level) -> Self {
        Style {
            level,
            cramped: false,
        }
    }

    fn scale(self, c: &Consts) -> f32 {
        match self.level {
            Level::Display | Level::Text => 1.0,
            Level::Script => c.script_scale,
            Level::ScriptScript => c.script_script_scale,
        }
    }

    fn is_display(self) -> bool {
        self.level == Level::Display
    }

    fn is_script(self) -> bool {
        matches!(self.level, Level::Script | Level::ScriptScript)
    }

    fn sup(self) -> Style {
        Style {
            level: if self.is_script() {
                Level::ScriptScript
            } else {
                Level::Script
            },
            cramped: self.cramped,
        }
    }

    fn sub(self) -> Style {
        Style {
            cramped: true,
            ..self.sup()
        }
    }

    fn numerator(self) -> Style {
        Style {
            level: match self.level {
                Level::Display => Level::Text,
                Level::Text => Level::Script,
                _ => Level::ScriptScript,
            },
            cramped: self.cramped,
        }
    }

    fn denominator(self) -> Style {
        Style {
            cramped: true,
            ..self.numerator()
        }
    }

    fn cramp(self) -> Style {
        Style {
            cramped: true,
            ..self
        }
    }
}

/// The spacing between two atoms, in mu (1/18 em); `Cond` only outside scripts.
#[derive(Clone, Copy)]
enum Gap {
    None,
    Thin,
    CondThin,
    CondMedium,
    CondThick,
}

/// TeXbook p. 170, rows the left atom, columns the right one in the order of
/// [`class_index`]. Bin next to Bin/Rel/Close/Punct (`*` there) never occurs
/// after the Bin-to-Ord conversion.
const SPACING: [[Gap; 8]; 8] = {
    use Gap::{CondMedium as M, CondThick as K, CondThin as T, None as N, Thin as S};
    [
        [N, S, M, K, N, N, N, T], // Ord
        [S, S, N, K, N, N, N, T], // Op
        [M, M, N, N, M, N, N, M], // Bin
        [K, K, N, N, K, N, N, K], // Rel
        [N, N, N, N, N, N, N, N], // Open
        [N, S, M, K, N, N, N, T], // Close
        [T, T, N, T, T, T, T, T], // Punct
        [T, S, M, K, T, N, T, T], // Inner
    ]
};

fn class_index(class: Class) -> usize {
    match class {
        Class::Ord => 0,
        Class::Op => 1,
        Class::Bin => 2,
        Class::Rel => 3,
        Class::Open => 4,
        Class::Close => 5,
        Class::Punct => 6,
        Class::Inner => 7,
    }
}

/// Lays out `nodes` as a formula; `display` is the style of `$$…$$`.
pub fn layout(font: &MathFont, nodes: &[Node], display: bool) -> Layout {
    let level = if display { Level::Display } else { Level::Text };
    Typesetter { font }.list(nodes, Style::new(level), None)
}

struct Typesetter<'f> {
    font: &'f MathFont,
}

/// One thing in a list: an atom with its class, or explicit space.
enum Entry {
    Atom(Class, Layout),
    Space(f32),
}

impl Typesetter<'_> {
    fn c(&self) -> &Consts {
        &self.font.consts
    }

    /// Atoms side by side with TeX's inter-atom spacing.
    fn list(&self, nodes: &[Node], mut style: Style, alpha: Option<Font>) -> Layout {
        let mut entries: Vec<Entry> = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                Node::Style(m) => style = Style::new(level_of(*m)),
                Node::Space(mu) => {
                    entries.push(Entry::Space(f32::from(*mu) / 18.0 * style.scale(self.c())))
                }
                other => {
                    let (class, boxed) = self.atom(other, style, alpha);
                    entries.push(Entry::Atom(class, boxed));
                }
            }
        }
        let classes = effective_classes(&entries);
        let mut out = Layout::default();
        let mut prev: Option<Class> = None;
        let mut next_class = classes.iter().copied();
        for entry in entries {
            match entry {
                Entry::Space(width) => out.space(width),
                Entry::Atom(_, boxed) => {
                    let class = next_class.next().unwrap_or(Class::Ord);
                    if let Some(left) = prev {
                        out.space(self.gap(left, class, style));
                    }
                    let italic = boxed.italic;
                    let mut boxed = boxed;
                    boxed.italic = 0.0;
                    out.push(boxed);
                    out.space(italic);
                    prev = Some(class);
                }
            }
        }
        out
    }

    fn gap(&self, left: Class, right: Class, style: Style) -> f32 {
        let mu = match SPACING[class_index(left)][class_index(right)] {
            Gap::None => 0.0,
            Gap::Thin => 3.0,
            Gap::CondThin if !style.is_script() => 3.0,
            Gap::CondMedium if !style.is_script() => 4.0,
            Gap::CondThick if !style.is_script() => 5.0,
            _ => 0.0,
        };
        mu / 18.0 * style.scale(self.c())
    }

    /// A node as one atom: its class and box.
    fn atom(&self, node: &Node, style: Style, alpha: Option<Font>) -> (Class, Layout) {
        match node {
            Node::Symbol { ch, class } => (*class, self.glyph_atom(*ch, style, alpha)),
            Node::Op { name, .. } => (Class::Op, self.operator(name, style)),
            Node::Text { text, font } => (Class::Ord, self.text(text, *font, style)),
            Node::Group(inner) => (Class::Ord, self.list(inner, style, alpha)),
            Node::Font { font, body } => {
                let (class, boxed) = self.atom(body, style, Some(*font));
                (class, boxed)
            }
            Node::Class { class, body } => (*class, self.atom(body, style, alpha).1),
            Node::Script { base, sub, sup } => {
                self.scripted(base, sub.as_deref(), sup.as_deref(), style, alpha)
            }
            Node::Frac {
                num,
                den,
                kind,
                style: forced,
            } => (
                Class::Inner,
                self.fraction(num, den, *kind, *forced, style, alpha),
            ),
            Node::Sqrt { index, body } => (
                Class::Ord,
                self.radical(index.as_deref(), body, style, alpha),
            ),
            Node::Delimited { open, close, body } => (
                Class::Inner,
                self.delimited(*open, *close, body, style, alpha),
            ),
            Node::Accent { kind, body } => (Class::Ord, self.accent(*kind, body, style, alpha)),
            Node::Stack { base, over, under } => {
                self.stack(base, over.as_deref(), under.as_deref(), style, alpha)
            }
            Node::Arrow { right, over, under } => (
                Class::Rel,
                self.arrow(*right, over, under.as_deref(), style, alpha),
            ),
            Node::Negated(body) => self.negated(body, style, alpha),
            Node::Boxed(body) => (Class::Ord, self.boxed(body, style, alpha)),
            Node::Error(source) => (Class::Ord, self.error_run(source, style)),
            // Not laid out yet: shown as source so it is not silently lost.
            other => (Class::Ord, self.error_run(&placeholder(other), style)),
        }
    }

    /// One character as a glyph box at the style's size.
    fn glyph_atom(&self, ch: char, style: Style, alpha: Option<Font>) -> Layout {
        let id = self
            .font
            .glyph_id(alphabet::styled(ch, alpha))
            .or_else(|| self.font.glyph_id(ch))
            .unwrap_or(GlyphId(0));
        self.glyph(id, style.scale(self.c()), false)
    }

    /// A glyph at `size`, its origin on the baseline.
    fn glyph(&self, id: GlyphId, size: f32, error: bool) -> Layout {
        let m = self.font.metrics(id);
        Layout {
            width: m.advance * size,
            ascent: m.ascent * size,
            descent: m.descent * size,
            items: vec![Item::Glyph {
                id: id.0,
                x: 0.0,
                y: 0.0,
                size,
                error,
            }],
            italic: m.italic * size,
        }
    }

    /// Characters in a row, upright unless `font` says otherwise.
    fn run(&self, text: &str, font: Option<Font>, size: f32, error: bool) -> Layout {
        let mut out = Layout::default();
        for ch in text.chars() {
            if ch == ' ' {
                out.space(0.25 * size);
                continue;
            }
            let id = self
                .font
                .glyph_id(alphabet::styled(ch, font))
                .or_else(|| self.font.glyph_id(ch))
                .unwrap_or(GlyphId(0));
            let mut g = self.glyph(id, size, error);
            g.italic = 0.0;
            out.push(g);
        }
        out
    }

    fn text(&self, text: &str, font: Font, style: Style) -> Layout {
        self.run(text, Some(font), style.scale(self.c()), false)
    }

    fn error_run(&self, source: &str, style: Style) -> Layout {
        self.run(source, Some(Font::Roman), style.scale(self.c()), true)
    }

    /// A big operator or a named one (`lim`), scripts not yet attached.
    fn operator(&self, name: &OpName, style: Style) -> Layout {
        let s = style.scale(self.c());
        match name {
            OpName::Text(word) => self.run(word, Some(Font::Roman), s, false),
            OpName::Glyph(ch) => {
                let base = self.font.glyph_or_notdef(*ch);
                let id = if style.is_display() {
                    self.at_least(base, self.c().display_operator_min_height)
                } else {
                    base
                };
                let g = self.glyph(id, s, false);
                // Centred on the math axis, as TeX sets an operator.
                let shift = (g.ascent - g.descent) / 2.0 - self.c().axis_height * s;
                shifted_box(g, shift)
            }
        }
    }

    /// The smallest form of `base` that is `height` em tall, else the largest.
    fn at_least(&self, base: GlyphId, height: f32) -> GlyphId {
        let m = self.font.metrics(base);
        if m.ascent + m.descent >= height {
            return base;
        }
        let variants = self.font.vertical_variants(base);
        variants
            .iter()
            .find(|(_, size)| *size >= height)
            .or(variants.last())
            .map_or(base, |(id, _)| *id)
    }

    /// A nucleus with its scripts: beside it, or above and below for an
    /// operator that takes limits.
    fn scripted(
        &self,
        base: &Node,
        sub: Option<&Node>,
        sup: Option<&Node>,
        style: Style,
        alpha: Option<Font>,
    ) -> (Class, Layout) {
        let (class, nucleus) = self.atom(base, style, alpha);
        let sub = sub.map(|n| self.atom(n, style.sub(), alpha).1);
        let sup = sup.map(|n| self.atom(n, style.sup(), alpha).1);
        let limits = match base {
            Node::Op { limits, .. } => match limits {
                Limits::Always => true,
                Limits::Display => style.is_display(),
                Limits::Never => false,
            },
            Node::Accent { kind, .. } => matches!(kind, Accent::OverBrace | Accent::UnderBrace),
            _ => false,
        };
        let out = if limits {
            self.limits(nucleus, sub, sup, style)
        } else {
            let is_char = matches!(base, Node::Symbol { .. } | Node::Text { .. });
            self.beside(nucleus, sub, sup, style, is_char)
        };
        (class, out)
    }

    /// TeXbook rule 18: scripts raised and lowered by the MATH constants.
    fn beside(
        &self,
        nucleus: Layout,
        sub: Option<Layout>,
        sup: Option<Layout>,
        style: Style,
        is_char: bool,
    ) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let (sup_scale, sub_scale) = (style.sup().scale(c), style.sub().scale(c));
        let mut up = if is_char || sup.is_none() {
            0.0
        } else {
            nucleus.ascent - c.sup_baseline_drop_max * sup_scale
        };
        let mut down = if is_char || sub.is_none() {
            0.0
        } else {
            nucleus.descent + c.sub_baseline_drop_min * sub_scale
        };
        let shift_up = if style.cramped {
            c.sup_shift_up_cramped
        } else {
            c.sup_shift_up
        } * s;
        if let Some(sup) = &sup {
            up = up.max(shift_up).max(sup.descent + c.sup_bottom_min * s);
        }
        if let Some(sub) = &sub {
            down = down.max(c.sub_shift_down * s);
            if sup.is_none() {
                down = down.max(sub.ascent - c.sub_top_max * s);
            }
        }
        if let (Some(sup), Some(sub)) = (&sup, &sub) {
            let gap = (up - sup.descent) - (sub.ascent - down);
            let wanted = c.sub_sup_gap_min * s;
            if gap < wanted {
                down += wanted - gap;
                let low = c.sup_bottom_max_with_sub * s - (up - sup.descent);
                if low > 0.0 {
                    up += low;
                    down -= low;
                }
            }
        }
        let (width, italic) = (nucleus.width, nucleus.italic);
        let mut out = Layout {
            italic: 0.0,
            ..nucleus
        };
        if let Some(sub) = sub {
            out.place(sub, width, down);
        }
        if let Some(sup) = sup {
            out.place(sup, width + italic, -up);
        }
        out.width += c.space_after_script * s;
        out
    }

    /// Scripts above and below a big operator, centred on it.
    fn limits(
        &self,
        nucleus: Layout,
        sub: Option<Layout>,
        sup: Option<Layout>,
        style: Style,
    ) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let delta = nucleus.italic;
        let width = [
            Some(nucleus.width),
            sup.as_ref().map(|b| b.width),
            sub.as_ref().map(|b| b.width),
        ]
        .into_iter()
        .flatten()
        .fold(0.0, f32::max);
        let centre = |w: f32| (width - w) / 2.0;
        let mut out = Layout::default();
        let (nucleus_ascent, nucleus_descent) = (nucleus.ascent, nucleus.descent);
        let nucleus_width = nucleus.width;
        out.place(
            Layout {
                italic: 0.0,
                ..nucleus
            },
            centre(nucleus_width),
            0.0,
        );
        if let Some(sub) = sub {
            let drop = nucleus_descent
                + (c.lower_limit_gap_min * s + sub.ascent).max(c.lower_limit_drop_min * s);
            let x = centre(sub.width) - delta / 2.0;
            out.place(sub, x, drop);
        }
        if let Some(sup) = sup {
            let rise = nucleus_ascent
                + (c.upper_limit_gap_min * s + sup.descent).max(c.upper_limit_rise_min * s);
            let x = centre(sup.width) + delta / 2.0;
            out.place(sup, x, -rise);
        }
        out.width = width;
        out
    }

    /// TeXbook rule 15: numerator over denominator, with a bar unless a
    /// binomial.
    fn fraction(
        &self,
        num: &Node,
        den: &Node,
        kind: FracKind,
        forced: Option<MathStyle>,
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let c = self.c();
        let style = forced.map_or(style, |m| Style {
            cramped: style.cramped,
            ..Style::new(level_of(m))
        });
        let (s, display) = (style.scale(c), style.is_display());
        let n = self.atom(num, style.numerator(), alpha).1;
        let d = self.atom(den, style.denominator(), alpha).1;
        let axis = c.axis_height * s;
        let rule = match kind {
            FracKind::Fraction => c.frac_rule * s,
            FracKind::Binom => 0.0,
        };
        let (up, down) = self.fraction_shifts(&n, &d, kind, style, rule);
        // TeX's null delimiter space on both sides.
        let null = 0.12 * s;
        let width = n.width.max(d.width);
        let mut out = Layout::default();
        out.place(n.centred(width), null, -up);
        out.place(d.centred(width), null, down);
        if kind == FracKind::Fraction {
            out.rule(null, -(axis + rule / 2.0), width, rule);
        }
        out.width = width + 2.0 * null;
        if kind == FracKind::Binom {
            let target = if display { 2.4 } else { 1.0 } * s;
            let mut wrapped = self.delimiter('(', target, s);
            wrapped.push(out);
            let close = self.delimiter(')', target, s);
            wrapped.push(close);
            return wrapped;
        }
        out
    }

    /// How far the numerator's baseline is above the main baseline and the
    /// denominator's below it: the font's shifts, or more to keep the gaps
    /// to the bar (or to each other, with no bar).
    fn fraction_shifts(
        &self,
        n: &Layout,
        d: &Layout,
        kind: FracKind,
        style: Style,
        rule: f32,
    ) -> (f32, f32) {
        let c = self.c();
        let (s, display) = (style.scale(c), style.is_display());
        let axis = c.axis_height * s;
        if kind == FracKind::Fraction {
            let (shift_up, shift_down, num_gap, den_gap) = if display {
                (
                    c.frac_num_display_shift_up,
                    c.frac_den_display_shift_down,
                    c.frac_num_display_gap_min,
                    c.frac_den_display_gap_min,
                )
            } else {
                (
                    c.frac_num_shift_up,
                    c.frac_den_shift_down,
                    c.frac_num_gap_min,
                    c.frac_den_gap_min,
                )
            };
            return (
                (shift_up * s).max(axis + rule / 2.0 + num_gap * s + n.descent),
                (shift_down * s).max(d.ascent - axis + rule / 2.0 + den_gap * s),
            );
        }
        let (shift_up, shift_down, gap) = if display {
            (
                c.stack_top_display_shift_up,
                c.stack_bottom_display_shift_down,
                c.stack_display_gap_min,
            )
        } else {
            (
                c.stack_top_shift_up,
                c.stack_bottom_shift_down,
                c.stack_gap_min,
            )
        };
        let (mut up, mut down) = (shift_up * s, shift_down * s);
        let room = (up - n.descent) + (down - d.ascent);
        if room < gap * s {
            let extra = (gap * s - room) / 2.0;
            up += extra;
            down += extra;
        }
        (up, down)
    }

    /// TeXbook rule 11: a radical glyph tall enough, an overbar, and an
    /// optional degree raised into its crook.
    fn radical(
        &self,
        index: Option<&Node>,
        body: &Node,
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let inner = self.atom(body, style.cramp(), alpha).1;
        let rule = c.radical_rule * s;
        let gap = if style.is_display() {
            c.radical_display_gap
        } else {
            c.radical_gap
        } * s;
        let need = inner.ascent + inner.descent + gap + rule;
        let base = self.font.glyph_or_notdef('√');
        let id = self.at_least(base, need / s);
        let glyph = self.glyph(id, s, false);
        let total = glyph.ascent + glyph.descent;
        // Spare height in the glyph is split above and below the body.
        let extra = (total - need).max(0.0);
        let rule_top = inner.ascent + gap + extra / 2.0 + rule;
        let glyph_dy = glyph.ascent - rule_top;
        let (degree, degree_x, glyph_x) = match index {
            Some(index) => {
                let degree = self.atom(index, Style::new(Level::ScriptScript), alpha).1;
                let before = c.radical_kern_before * s;
                let after = c.radical_kern_after * s;
                let glyph_x = (before + degree.width + after).max(0.0);
                (Some(degree), before.max(0.0), glyph_x)
            }
            None => (None, 0.0, 0.0),
        };
        let (glyph_width, body_width) = (glyph.width, inner.width);
        let mut out = Layout::default();
        out.place(glyph, glyph_x, glyph_dy);
        out.place(inner, glyph_x + glyph_width, 0.0);
        out.rule(glyph_x + glyph_width, -rule_top, body_width, rule);
        if let Some(degree) = degree {
            let bottom = rule_top - total;
            let raise = bottom + c.radical_raise * total;
            out.place(degree, degree_x, -raise);
        }
        out
    }

    /// `\left … \right` with delimiters grown to the body (`\middle` too).
    fn delimited(
        &self,
        open: Option<char>,
        close: Option<char>,
        body: &[Node],
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let s = style.scale(self.c());
        let axis = self.c().axis_height * s;
        let mut segments: Vec<Layout> = Vec::new();
        let mut middles: Vec<Option<char>> = Vec::new();
        let mut start = 0;
        for (i, node) in body.iter().enumerate() {
            if let Node::Middle(delim) = node {
                segments.push(self.list(&body[start..i], style, alpha));
                middles.push(*delim);
                start = i + 1;
            }
        }
        segments.push(self.list(&body[start..], style, alpha));
        let reach = segments
            .iter()
            .map(|b| (b.ascent - axis).max(b.descent + axis))
            .fold(0.0, f32::max);
        // TeX's delimiter factor and shortfall (0.901, and 5pt of 10pt).
        let target = (2.0 * reach * 0.901).max(2.0 * reach - 0.5 * s);
        // One delimiter before each segment, one more after the last.
        let mut delims = [open].into_iter().chain(middles).chain([close]);
        let mut out = Layout::default();
        for seg in segments {
            if let Some(Some(ch)) = delims.next() {
                out.push(self.delimiter(ch, target, s));
            }
            out.push(seg);
        }
        if let Some(Some(ch)) = delims.next() {
            out.push(self.delimiter(ch, target, s));
        }
        out
    }

    /// A delimiter glyph at least `target` em tall, centred on the axis.
    fn delimiter(&self, ch: char, target: f32, size: f32) -> Layout {
        let base = self.font.glyph_or_notdef(ch);
        let id = self.at_least(base, target / size);
        let g = self.glyph(id, size, false);
        // No variant is tall enough: build one from the font's parts.
        let g = match self.assemble_tall(base, target, size) {
            Some(built) if g.ascent + g.descent < target => built,
            _ => g,
        };
        let shift = (g.ascent - g.descent) / 2.0 - self.c().axis_height * size;
        shifted_box(g, shift)
    }
}

impl Layout {
    /// The box widened to `width` with its content centred.
    fn centred(self, width: f32) -> Layout {
        if width <= self.width {
            return self;
        }
        let dx = (width - self.width) / 2.0;
        let mut out = Layout::default();
        out.place(self, dx, 0.0);
        out.width = width;
        out
    }
}

/// `g` moved `down` (negative: up), its extent following.
fn shifted_box(g: Layout, down: f32) -> Layout {
    let mut out = Layout::default();
    let width = g.width;
    let italic = g.italic;
    out.place(Layout { italic: 0.0, ..g }, 0.0, down);
    out.width = width;
    out.italic = italic;
    out
}

fn level_of(style: MathStyle) -> Level {
    match style {
        MathStyle::Display => Level::Display,
        MathStyle::Text => Level::Text,
        MathStyle::Script => Level::Script,
        MathStyle::ScriptScript => Level::ScriptScript,
    }
}

/// The class each atom spaces as: a Bin with nothing, or an operator, relation,
/// opener or punctuation before it, or a relation, closer or punctuation (or
/// nothing) after it, is an ordinary symbol (a unary minus).
fn effective_classes(entries: &[Entry]) -> Vec<Class> {
    let atoms: Vec<Class> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Atom(class, _) => Some(*class),
            Entry::Space(_) => None,
        })
        .collect();
    let mut out = atoms.clone();
    for i in 0..atoms.len() {
        if atoms[i] != Class::Bin {
            continue;
        }
        let before_ok = i > 0
            && !matches!(
                out[i - 1],
                Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct
            );
        let after_ok = atoms
            .get(i + 1)
            .is_some_and(|next| !matches!(next, Class::Rel | Class::Close | Class::Punct));
        if !(before_ok && after_ok) {
            out[i] = Class::Ord;
        }
    }
    out
}

/// What shows for a construct the layout does not handle yet.
fn placeholder(node: &Node) -> String {
    match node {
        Node::Env(_) => "matrix",
        Node::Break => "\\\\",
        Node::Middle(_) => "\\middle",
        _ => "?",
    }
    .to_owned()
}
