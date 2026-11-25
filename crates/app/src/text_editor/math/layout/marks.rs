//! Things set on or around another box: accents, stacked and arrow labels,
//! the `\not` slash and `\boxed`.

use ttf_parser::GlyphId;

use super::super::alphabet;
use super::super::ast::{Accent, Class, Font, Node, Strike};
use super::{Item, Layout, Style, Typesetter};

/// The character an accent draws; the combining form sits over a base letter.
fn accent_char(kind: Accent) -> Option<char> {
    Some(match kind {
        Accent::Hat | Accent::WideHat => '\u{302}',
        Accent::Check => '\u{30C}',
        Accent::Tilde | Accent::WideTilde => '\u{303}',
        Accent::Acute => '\u{301}',
        Accent::Grave => '\u{300}',
        Accent::Dot => '\u{307}',
        Accent::DoubleDot => '\u{308}',
        Accent::Breve => '\u{306}',
        Accent::Bar => '\u{304}',
        Accent::Vec | Accent::OverRightArrow => '\u{20D7}',
        Accent::OverLeftArrow => '\u{20D6}',
        Accent::OverLeftRightArrow => '\u{20E1}',
        Accent::DotDotDot => '\u{20DB}',
        Accent::WideCheck => '\u{30C}',
        Accent::OverParen => '\u{23DC}',
        Accent::Ring => '\u{30A}',
        Accent::OverBrace => '\u{23DE}',
        Accent::UnderBrace => '\u{23DF}',
        Accent::OverLine | Accent::UnderLine => return None,
    })
}

/// Whether the mark stretches to the body instead of keeping its own width.
fn stretches(kind: Accent) -> bool {
    matches!(
        kind,
        Accent::WideHat
            | Accent::WideCheck
            | Accent::WideTilde
            | Accent::OverLeftRightArrow
            | Accent::OverParen
            | Accent::OverRightArrow
            | Accent::OverLeftArrow
            | Accent::OverBrace
            | Accent::UnderBrace
    )
}

/// The class of the one symbol `node` is, looking through braces: `\overset`
/// over `{\to}` is still a relation.
fn lone_symbol(node: &Node) -> Option<Class> {
    match node {
        Node::Symbol { class, .. } => Some(*class),
        Node::Group(inner) if inner.len() == 1 => lone_symbol(&inner[0]),
        _ => None,
    }
}

impl Typesetter<'_> {
    pub(super) fn accent(
        &self,
        kind: Accent,
        body: &Node,
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let inner = self.atom(body, style.cramp(), alpha).1;
        let Some(ch) = accent_char(kind) else {
            return self.bar(kind, inner, style);
        };
        let s = style.scale(self.c());
        let id = self.font.glyph_or_notdef(ch);
        if stretches(kind) {
            return self.wide_accent(kind, id, inner, s);
        }
        let mark = self.glyph(id, s, false);
        let lift = (inner.ascent - self.c().accent_base_height * s).max(0.0);
        let attach = self.attach_point(body, &inner, style, alpha);
        let (left, right) = self.font.ink_x(id);
        let own_middle = (left + right) / 2.0;
        let mark_at = self.font.top_accent(id).unwrap_or(own_middle) * s;
        let width = inner.width;
        let mut out = Layout::default();
        out.place(inner, 0.0, 0.0);
        out.place(mark, attach - mark_at, -lift);
        out.width = width;
        out
    }

    /// Where an accent over `body` points: the font's attachment for a lone
    /// letter (which leans), else the middle.
    fn attach_point(&self, body: &Node, inner: &Layout, style: Style, alpha: Option<Font>) -> f32 {
        if let Node::Symbol { ch, .. } = body {
            let id = self.font.glyph_id(alphabet::styled(*ch, alpha));
            if let Some(x) = id.and_then(|id| self.font.top_accent(id)) {
                return x * style.scale(self.c());
            }
        }
        inner.width / 2.0
    }

    /// `\widehat`, `\overrightarrow`, braces: a mark as wide as the body, set
    /// above it (below for `\underbrace`), the gap kept from the ink.
    fn wide_accent(&self, kind: Accent, id: GlyphId, inner: Layout, s: f32) -> Layout {
        let c = self.c();
        let mark = self.stretch_wide(id, inner.width, s);
        let (width, mark_width) = (inner.width, mark.layout.width);
        let (ascent, descent) = (inner.ascent, inner.descent);
        let dy = match kind {
            Accent::UnderBrace => descent + c.stretch_gap_below_min * s + mark.top,
            Accent::OverBrace | Accent::OverParen => {
                -(ascent + c.stretch_gap_above_min * s - mark.bottom)
            }
            _ => -(ascent - c.accent_base_height * s).max(0.0),
        };
        let mut out = Layout::default();
        out.place(inner, ((mark_width - width) / 2.0).max(0.0), 0.0);
        out.place(mark.layout, ((width - mark_width) / 2.0).max(0.0), dy);
        out.width = width.max(mark_width);
        out
    }

    /// `\overline` and `\underline`: a rule with the font's gap and thickness.
    fn bar(&self, kind: Accent, inner: Layout, style: Style) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let width = inner.width;
        let (ascent, descent) = (inner.ascent, inner.descent);
        let mut out = Layout::default();
        out.place(inner, 0.0, 0.0);
        if kind.is_under() {
            let top = descent + c.underbar_gap * s;
            out.rule(0.0, top, width, c.underbar_rule * s);
            out.descent += c.underbar_extra * s;
        } else {
            let rule = c.overbar_rule * s;
            out.rule(0.0, -(ascent + c.overbar_gap * s + rule), width, rule);
            out.ascent += c.overbar_extra * s;
        }
        out.width = width;
        out
    }

    /// `\overset`, `\underset`, `\stackrel`: labels in script size, centred
    /// above and below the base; the class is the base's.
    pub(super) fn stack(
        &self,
        base: &Node,
        over: Option<&Node>,
        under: Option<&Node>,
        style: Style,
        alpha: Option<Font>,
    ) -> (Class, Layout) {
        let (class, nucleus) = self.atom(base, style, alpha);
        let class = lone_symbol(base).unwrap_or(class);
        let up = over.map(|n| self.atom(n, style.sup(), alpha).1);
        let down = under.map(|n| self.atom(n, style.sub(), alpha).1);
        (class, self.limits(nucleus, down, up, style))
    }

    /// `\xrightarrow[under]{over}`: an arrow as long as its labels need.
    pub(super) fn arrow(
        &self,
        right: bool,
        over: &Node,
        under: Option<&Node>,
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let up = self.atom(over, style.sup(), alpha).1;
        let down = under.map(|n| self.atom(n, style.sub(), alpha).1);
        let labels = down.as_ref().map_or(up.width, |d| d.width.max(up.width));
        let id = self
            .font
            .glyph_or_notdef(if right { '\u{2192}' } else { '\u{2190}' });
        let shaft = self.stretch_wide(id, labels + 0.6 * s, s);
        let width = shaft.layout.width;
        let gap = c.stretch_gap_above_min * s;
        let raise = shaft.top + gap + up.descent;
        let mut out = Layout::default();
        out.place(up.centred(width), 0.0, -raise);
        if let Some(down) = down {
            let drop = -shaft.bottom + c.stretch_gap_below_min * s + down.ascent;
            out.place(down.centred(width), 0.0, drop);
        }
        out.place(shaft.layout, 0.0, 0.0);
        out.width = width;
        out
    }

    /// `\not`: a slash through the body; spaces as the body does.
    pub(super) fn negated(
        &self,
        body: &Node,
        style: Style,
        alpha: Option<Font>,
    ) -> (Class, Layout) {
        let (class, inner) = self.atom(body, style, alpha);
        let s = style.scale(self.c());
        let id = self.font.glyph_or_notdef('\u{338}');
        let (left, right) = self.font.ink_x(id);
        let slash = self.glyph(id, s, false);
        let width = inner.width;
        let mut out = Layout::default();
        out.place(inner, 0.0, 0.0);
        out.place(slash, (width - (left + right) * s) / 2.0, 0.0);
        out.width = width;
        (class, out)
    }

    /// `\cancel` and friends: diagonals corner to corner over `inner`, a
    /// little past it as TeX's `cancel` package does.
    pub(super) fn cancelled(&self, mut inner: Layout, kind: Strike, style: Style) -> Layout {
        let s = style.scale(self.c());
        let (pad, thickness) = (0.1 * s, self.c().frac_rule * s);
        let (left, right) = (-pad, inner.width + pad);
        let (top, bottom) = (-inner.ascent - pad, inner.descent + pad);
        if matches!(kind, Strike::Up | Strike::Cross) {
            inner.items.push(Item::Line {
                x: left,
                y: bottom,
                x2: right,
                y2: top,
                thickness,
                color: None,
            });
        }
        if matches!(kind, Strike::Down | Strike::Cross) {
            inner.items.push(Item::Line {
                x: left,
                y: top,
                x2: right,
                y2: bottom,
                thickness,
                color: None,
            });
        }
        inner
    }

    /// `\boxed`: the body in a frame of rules, a little air inside.
    pub(super) fn boxed(&self, body: &Node, style: Style, alpha: Option<Font>) -> Layout {
        let c = self.c();
        let s = style.scale(c);
        let (pad, line) = (0.2 * s, c.frac_rule * s);
        let inner = self.atom(body, style, alpha).1;
        let (width, ascent, descent) = (inner.width, inner.ascent, inner.descent);
        let (top, bottom) = (-(ascent + pad + line), descent + pad);
        let full = width + 2.0 * (pad + line);
        let mut out = Layout::default();
        out.place(inner, pad + line, 0.0);
        out.rule(0.0, top, full, line);
        out.rule(0.0, bottom, full, line);
        let side = bottom + line - top;
        out.rule(0.0, top, line, side);
        out.rule(full - line, top, line, side);
        out.width = full;
        out
    }
}
