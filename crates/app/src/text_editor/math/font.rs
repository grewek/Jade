//! The math font: glyph lookup, metrics, size variants and the OpenType MATH
//! constants, read once from the font file. Everything is in em (design units
//! over units-per-em), so a layout does not depend on the font size.

use std::sync::OnceLock;

use ttf_parser::{Face, GlyphId};

/// Latin Modern Math, bundled unmodified (GUST Font License).
static LATIN_MODERN_MATH: &[u8] =
    include_bytes!("../../../assets/fonts/latin-modern-math/latinmodern-math.otf");

/// A glyph's size in em. `ascent` and `descent` are the ink's, as TeX uses a
/// character's height and depth, not the font's line metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphBox {
    pub id: GlyphId,
    pub advance: f32,
    pub ascent: f32,
    pub descent: f32,
    /// Added after the glyph before a superscript or the next atom.
    pub italic: f32,
}

/// The MATH table's constants, in em; `*_scale` are fractions (0.7 for 70%).
#[derive(Debug, Clone, Copy)]
pub struct Consts {
    pub script_scale: f32,
    pub script_script_scale: f32,
    pub display_operator_min_height: f32,
    pub axis_height: f32,
    pub sub_shift_down: f32,
    pub sub_top_max: f32,
    pub sub_baseline_drop_min: f32,
    pub sup_shift_up: f32,
    pub sup_shift_up_cramped: f32,
    pub sup_bottom_min: f32,
    pub sup_baseline_drop_max: f32,
    pub sub_sup_gap_min: f32,
    pub sup_bottom_max_with_sub: f32,
    pub space_after_script: f32,
    pub upper_limit_gap_min: f32,
    pub upper_limit_rise_min: f32,
    pub lower_limit_gap_min: f32,
    pub lower_limit_drop_min: f32,
    pub stack_top_shift_up: f32,
    pub stack_top_display_shift_up: f32,
    pub stack_bottom_shift_down: f32,
    pub stack_bottom_display_shift_down: f32,
    pub stack_gap_min: f32,
    pub stack_display_gap_min: f32,
    pub frac_num_shift_up: f32,
    pub frac_num_display_shift_up: f32,
    pub frac_den_shift_down: f32,
    pub frac_den_display_shift_down: f32,
    pub frac_num_gap_min: f32,
    pub frac_num_display_gap_min: f32,
    pub frac_rule: f32,
    pub frac_den_gap_min: f32,
    pub frac_den_display_gap_min: f32,
    pub overbar_gap: f32,
    pub overbar_rule: f32,
    pub overbar_extra: f32,
    pub underbar_gap: f32,
    pub underbar_rule: f32,
    pub underbar_extra: f32,
    pub radical_gap: f32,
    pub radical_display_gap: f32,
    pub radical_rule: f32,
    pub radical_extra: f32,
    pub radical_kern_before: f32,
    pub radical_kern_after: f32,
    pub radical_raise: f32,
}

pub struct MathFont {
    face: Face<'static>,
    upem: f32,
    pub consts: Consts,
}

impl MathFont {
    /// Parses a font that has a MATH table; `None` if it has none.
    pub fn new(data: &'static [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let table = face.tables().math?;
        let c = table.constants?;
        let upem = f32::from(face.units_per_em());
        let em = |v: ttf_parser::math::MathValue| f32::from(v.value) / upem;
        let consts = Consts {
            script_scale: f32::from(c.script_percent_scale_down()) / 100.0,
            script_script_scale: f32::from(c.script_script_percent_scale_down()) / 100.0,
            display_operator_min_height: f32::from(c.display_operator_min_height()) / upem,
            axis_height: em(c.axis_height()),
            sub_shift_down: em(c.subscript_shift_down()),
            sub_top_max: em(c.subscript_top_max()),
            sub_baseline_drop_min: em(c.subscript_baseline_drop_min()),
            sup_shift_up: em(c.superscript_shift_up()),
            sup_shift_up_cramped: em(c.superscript_shift_up_cramped()),
            sup_bottom_min: em(c.superscript_bottom_min()),
            sup_baseline_drop_max: em(c.superscript_baseline_drop_max()),
            sub_sup_gap_min: em(c.sub_superscript_gap_min()),
            sup_bottom_max_with_sub: em(c.superscript_bottom_max_with_subscript()),
            space_after_script: em(c.space_after_script()),
            upper_limit_gap_min: em(c.upper_limit_gap_min()),
            upper_limit_rise_min: em(c.upper_limit_baseline_rise_min()),
            lower_limit_gap_min: em(c.lower_limit_gap_min()),
            lower_limit_drop_min: em(c.lower_limit_baseline_drop_min()),
            stack_top_shift_up: em(c.stack_top_shift_up()),
            stack_top_display_shift_up: em(c.stack_top_display_style_shift_up()),
            stack_bottom_shift_down: em(c.stack_bottom_shift_down()),
            stack_bottom_display_shift_down: em(c.stack_bottom_display_style_shift_down()),
            stack_gap_min: em(c.stack_gap_min()),
            stack_display_gap_min: em(c.stack_display_style_gap_min()),
            frac_num_shift_up: em(c.fraction_numerator_shift_up()),
            frac_num_display_shift_up: em(c.fraction_numerator_display_style_shift_up()),
            frac_den_shift_down: em(c.fraction_denominator_shift_down()),
            frac_den_display_shift_down: em(c.fraction_denominator_display_style_shift_down()),
            frac_num_gap_min: em(c.fraction_numerator_gap_min()),
            frac_num_display_gap_min: em(c.fraction_num_display_style_gap_min()),
            frac_rule: em(c.fraction_rule_thickness()),
            frac_den_gap_min: em(c.fraction_denominator_gap_min()),
            frac_den_display_gap_min: em(c.fraction_denom_display_style_gap_min()),
            overbar_gap: em(c.overbar_vertical_gap()),
            overbar_rule: em(c.overbar_rule_thickness()),
            overbar_extra: em(c.overbar_extra_ascender()),
            underbar_gap: em(c.underbar_vertical_gap()),
            underbar_rule: em(c.underbar_rule_thickness()),
            underbar_extra: em(c.underbar_extra_descender()),
            radical_gap: em(c.radical_vertical_gap()),
            radical_display_gap: em(c.radical_display_style_vertical_gap()),
            radical_rule: em(c.radical_rule_thickness()),
            radical_extra: em(c.radical_extra_ascender()),
            radical_kern_before: em(c.radical_kern_before_degree()),
            radical_kern_after: em(c.radical_kern_after_degree()),
            radical_raise: f32::from(c.radical_degree_bottom_raise_percent()) / 100.0,
        };
        Some(MathFont { face, upem, consts })
    }

    /// The bundled font, parsed on first use; `None` if it cannot be read, so
    /// a caller can show the source instead.
    pub fn bundled() -> Option<&'static MathFont> {
        static FONT: OnceLock<Option<MathFont>> = OnceLock::new();
        FONT.get_or_init(|| MathFont::new(LATIN_MODERN_MATH))
            .as_ref()
    }

    pub fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        self.face.glyph_index(ch)
    }

    /// The glyph for `ch`, or the font's `.notdef` box if it has none.
    pub fn glyph_or_notdef(&self, ch: char) -> GlyphId {
        self.glyph_id(ch).unwrap_or(GlyphId(0))
    }

    pub fn metrics(&self, id: GlyphId) -> GlyphBox {
        let advance = self.face.glyph_hor_advance(id).map_or(0.0, f32::from) / self.upem;
        let (ascent, descent) = self.face.glyph_bounding_box(id).map_or((0.0, 0.0), |b| {
            (
                f32::from(b.y_max) / self.upem,
                -f32::from(b.y_min) / self.upem,
            )
        });
        let italic = self
            .face
            .tables()
            .math
            .and_then(|m| m.glyph_info)
            .and_then(|g| g.italic_corrections)
            .and_then(|table| table.get(id))
            .map_or(0.0, |v| f32::from(v.value) / self.upem);
        GlyphBox {
            id,
            advance,
            ascent,
            descent,
            italic,
        }
    }

    /// The glyph's forms, smallest first, with their height plus depth in em;
    /// the list starts with the glyph itself.
    pub fn vertical_variants(&self, id: GlyphId) -> Vec<(GlyphId, f32)> {
        let construction = self
            .face
            .tables()
            .math
            .and_then(|m| m.variants)
            .and_then(|v| v.vertical_constructions.get(id));
        construction.map_or_else(Vec::new, |c| {
            c.variants
                .into_iter()
                .map(|v| {
                    (
                        v.variant_glyph,
                        f32::from(v.advance_measurement) / self.upem,
                    )
                })
                .collect()
        })
    }
}
