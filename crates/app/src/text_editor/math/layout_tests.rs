use super::*;
use crate::text_editor::test_support::Rng;

fn font() -> &'static MathFont {
    MathFont::bundled().unwrap()
}

#[test]
fn the_bundled_font_loads() {
    assert!(MathFont::bundled().is_some());
}

fn lay(src: &str, display: bool) -> Layout {
    let parsed = parse(src);
    assert!(
        !parsed.has_error,
        "{src:?} should parse: {:?}",
        parsed.nodes
    );
    layout(font(), &parsed.nodes, display)
}

fn id(ch: char) -> u16 {
    font()
        .glyph_id(ch)
        .unwrap_or_else(|| panic!("no glyph for {ch:?}"))
        .0
}

/// A glyph's advance and italic correction in em.
fn adv(ch: char) -> (f32, f32) {
    let m = font().metrics(font().glyph_id(ch).unwrap());
    (m.advance, m.italic)
}

/// `(id, x, y, size)` of every glyph, in drawing order.
fn glyphs(l: &Layout) -> Vec<(u16, f32, f32, f32)> {
    l.items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { id, x, y, size, .. } => Some((*id, *x, *y, *size)),
            Item::Rule { .. } | Item::Line { .. } => None,
        })
        .collect()
}

fn rules(l: &Layout) -> Vec<(f32, f32, f32, f32)> {
    l.items
        .iter()
        .filter_map(|i| match i {
            Item::Rule {
                x,
                y,
                width,
                height,
                ..
            } => Some((*x, *y, *width, *height)),
            Item::Glyph { .. } | Item::Line { .. } => None,
        })
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[track_caller]
fn assert_close(a: f32, b: f32) {
    assert!(close(a, b), "{a} != {b}");
}

#[test]
fn the_constants_are_the_ones_tex_defines() {
    let c = font().consts;
    assert_close(c.axis_height, 0.25);
    assert_close(c.script_scale, 0.7);
    assert_close(c.script_script_scale, 0.5);
    assert_close(c.frac_rule, 0.04);
    assert_close(c.sup_shift_up, 0.363);
    assert_close(c.sub_shift_down, 0.247);
}

#[test]
fn a_letter_is_the_math_italic_glyph_and_a_digit_is_upright() {
    let x = lay("x", false);
    assert_eq!(glyphs(&x), [(id('\u{1D465}'), 0.0, 0.0, 1.0)]);
    let (advance, _) = adv('\u{1D465}');
    assert_close(x.width, advance);
    assert_eq!(glyphs(&lay("1", false))[0].0, id('1'));
}

#[test]
fn binary_operators_get_a_medium_space_and_relations_a_thick_one() {
    let (a, ia) = adv('\u{1D44E}');
    let (b, ib) = adv('\u{1D44F}');
    let plus = adv('+').0;
    assert_close(
        lay("a+b", false).width,
        a + ia + plus + b + ib + 2.0 * 4.0 / 18.0,
    );
    let eq = adv('=').0;
    assert_close(
        lay("a=b", false).width,
        a + ia + eq + b + ib + 2.0 * 5.0 / 18.0,
    );
}

#[test]
fn a_unary_minus_takes_no_space() {
    let minus = adv('\u{2212}').0;
    let (a, ia) = adv('\u{1D44E}');
    assert_close(lay("-a", false).width, minus + a + ia);
    // After an opener or a relation, and before a closer, too.
    let (open, close_paren) = (adv('(').0, adv(')').0);
    assert_close(
        lay("(-a)", false).width,
        open + minus + a + ia + close_paren,
    );
}

#[test]
fn scripts_drop_the_conditional_spaces() {
    let l = lay("x^{a+b}", false);
    let g = glyphs(&l);
    let (a, ia) = adv('\u{1D44E}');
    let plus_x = g.iter().find(|g| g.0 == id('+')).unwrap().1;
    let a_x = g.iter().find(|g| g.0 == id('\u{1D44E}')).unwrap().1;
    assert_close(plus_x - a_x, 0.7 * (a + ia));
}

#[test]
fn a_superscript_and_subscript_use_the_font_shifts() {
    let c = font().consts;
    let x = adv('\u{1D465}');
    let l = lay("x^2", false);
    let g = glyphs(&l);
    assert_eq!(g[1].0, id('2'));
    assert_close(g[1].3, 0.7);
    assert_close(g[1].2, -c.sup_shift_up);
    assert_close(g[1].1, x.0 + x.1);
    let l = lay("x_2", false);
    assert_close(glyphs(&l)[1].2, c.sub_shift_down);
}

#[test]
fn both_scripts_keep_their_gap() {
    let c = font().consts;
    let l = lay("x_i^2", false);
    let g = glyphs(&l);
    let (sub, sup) = (&g[1], &g[2]);
    let sup_bottom = sup.2 + font().metrics(font().glyph_id('2').unwrap()).descent * 0.7;
    let sub_top = sub.2 - font().metrics(font().glyph_id('\u{1D456}').unwrap()).ascent * 0.7;
    assert!(sub_top - sup_bottom >= c.sub_sup_gap_min - 1e-3);
}

#[test]
fn scripts_nest_smaller_and_cramp_under_a_root() {
    let c = font().consts;
    let nested = glyphs(&lay("x^{y^z}", false));
    assert_close(nested[2].3, 0.5);
    // Inside a radical the style is cramped: the lower superscript shift.
    let cramped = glyphs(&lay("\\sqrt{x^2}", false));
    let two = cramped.iter().find(|g| g.0 == id('2')).unwrap();
    assert_close(two.2, -c.sup_shift_up_cramped);
}

#[test]
fn a_fraction_has_a_bar_on_the_axis_and_shifts_by_style() {
    let c = font().consts;
    let text = lay("\\frac{a}{b}", false);
    let r = rules(&text);
    assert_eq!(r.len(), 1);
    assert_close(r[0].3, c.frac_rule);
    assert_close(r[0].1, -(c.axis_height + c.frac_rule / 2.0));
    let g = glyphs(&text);
    assert_close(g[0].3, 0.7);
    assert_close(g[0].2, -c.frac_num_shift_up);
    assert_close(g[1].2, c.frac_den_shift_down);
    let display = lay("\\frac{a}{b}", true);
    let g = glyphs(&display);
    assert_close(g[0].3, 1.0);
    assert_close(g[0].2, -c.frac_num_display_shift_up);
    assert_close(g[1].2, c.frac_den_display_shift_down);
    assert!(display.ascent + display.descent > text.ascent + text.descent);
}

#[test]
fn a_fraction_centres_both_parts_and_spans_the_wider() {
    let l = lay("\\frac{a}{bbb}", false);
    let r = rules(&l)[0];
    let g = glyphs(&l);
    let den_width: f32 = (0..3).map(|_| adv('\u{1D44F}').0 * 0.7).sum::<f32>();
    assert_close(r.2, den_width + 3.0 * adv('\u{1D44F}').1 * 0.7);
    // The numerator sits in the middle of the bar.
    let num_centre = g[0].1 + adv('\u{1D44E}').0 * 0.7 / 2.0;
    assert_close(num_centre, r.0 + r.2 / 2.0);
}

#[test]
fn dfrac_and_displaystyle_force_the_display_layout() {
    let forced = lay("\\dfrac{a}{b}", false);
    assert_eq!(forced.items, lay("\\frac{a}{b}", true).items);
    let switched = lay("\\displaystyle\\frac{a}{b}", false);
    assert_eq!(switched.items, lay("\\frac{a}{b}", true).items);
}

#[test]
fn a_binomial_has_no_bar_and_parentheses() {
    let l = lay("\\binom{n}{k}", false);
    assert!(rules(&l).is_empty());
    let g = glyphs(&l);
    assert_ne!(g.first().unwrap().0, id('n'));
    assert!(g.iter().filter(|g| g.3 == 1.0).count() >= 2);
}

#[test]
fn a_root_covers_its_body_with_an_overbar() {
    let l = lay("\\sqrt{x}", false);
    let r = rules(&l);
    assert_eq!(r.len(), 1);
    let g = glyphs(&l);
    let radical = &g[0];
    assert_eq!(radical.0, id('√'));
    let m = font().metrics(font().glyph_id('\u{1D465}').unwrap());
    // Above the body by a gap, and as wide as it.
    assert!(-r[0].1 >= m.ascent + font().consts.radical_gap);
    assert_close(r[0].2, m.advance);
    // The overbar starts where the radical glyph ends.
    assert_close(
        r[0].0,
        font().metrics(font().glyph_id('√').unwrap()).advance,
    );
}

#[test]
fn a_tall_body_picks_a_taller_radical() {
    let small = glyphs(&lay("\\sqrt{x}", false))[0].0;
    let tall = glyphs(&lay("\\sqrt{\\frac{a}{b}}", true))[0].0;
    assert_ne!(small, tall);
}

#[test]
fn a_root_index_sits_in_the_crook() {
    let l = lay("\\sqrt[3]{x}", false);
    let g = glyphs(&l);
    let three = g.iter().find(|g| g.0 == id('3')).unwrap();
    assert_close(three.3, 0.5);
    let radical = g.iter().find(|g| g.0 == id('√')).unwrap();
    // The index is left of the radical and above its bottom.
    assert!(three.1 < radical.1 + 0.5);
    assert!(three.2 < radical.2 + font().metrics(font().glyph_id('√').unwrap()).descent);
}

#[test]
fn left_right_use_the_base_glyph_for_a_small_body_and_grow_for_a_tall_one() {
    let small = glyphs(&lay("\\left( x \\right)", false));
    assert_eq!(small[0].0, id('('));
    assert_eq!(small.last().unwrap().0, id(')'));
    let tall = glyphs(&lay("\\left( \\frac{a}{b} \\right)", true));
    assert_ne!(tall[0].0, id('('));
    // Both delimiters are the same size.
    assert_ne!(tall.first().unwrap().0, id(')'));
}

#[test]
fn delimiters_are_centred_on_the_axis() {
    let l = lay("\\left( \\frac{a}{b} \\right)", true);
    let (open, ..) = (&l.items[0],);
    let Item::Glyph {
        id: gid, y, size, ..
    } = open
    else {
        panic!()
    };
    let m = font().metrics(ttf_parser::GlyphId(*gid));
    let centre_up = -y + (m.ascent - m.descent) / 2.0 * size;
    assert_close(centre_up, font().consts.axis_height);
}

#[test]
fn an_empty_delimiter_draws_nothing() {
    let l = lay("\\left. x \\right|", false);
    let g = glyphs(&l);
    assert_eq!(g.len(), 2);
    assert_eq!(g[1].0, id('|'));
}

#[test]
fn a_big_operator_grows_in_display_style_and_sits_on_the_axis() {
    let text = glyphs(&lay("\\sum", false))[0];
    let display = glyphs(&lay("\\sum", true))[0];
    assert_eq!(text.0, id('∑'));
    assert_ne!(display.0, id('∑'));
    for (op, size) in [(text, 1.0), (display, 1.0)] {
        let m = font().metrics(ttf_parser::GlyphId(op.0));
        assert_close(
            -op.2 + (m.ascent - m.descent) / 2.0,
            font().consts.axis_height * size,
        );
    }
}

#[test]
fn limits_go_above_and_below_in_display_and_beside_in_text() {
    let c = font().consts;
    let display = lay("\\sum_{i}^{n}", true);
    let g = glyphs(&display);
    let (op, sub, sup) = (&g[0], &g[1], &g[2]);
    assert!(sub.2 > 0.0 && sup.2 < 0.0);
    // Centred on the operator.
    let op_w = font().metrics(ttf_parser::GlyphId(op.0)).advance;
    let sub_w = adv('\u{1D456}').0 * 0.7;
    assert_close(sub.1 + sub_w / 2.0, op.1 + op_w / 2.0);
    let _ = c;
    let beside = lay("\\sum_{i}^{n}", false);
    let g = glyphs(&beside);
    let op_w = font().metrics(ttf_parser::GlyphId(g[0].0)).advance;
    assert!(g[1].1 >= op_w - 1e-3 && g[2].1 >= op_w - 1e-3);
}

#[test]
fn integrals_keep_their_scripts_beside_even_in_display() {
    let g = glyphs(&lay("\\int_a^b", true));
    let op_w = font().metrics(ttf_parser::GlyphId(g[0].0)).advance;
    assert!(g[1].1 >= op_w - 1e-3);
    // `\limits` moves them.
    let g = glyphs(&lay("\\int\\limits_a^b", true));
    assert!(g[1].2 > 0.0 && g[2].2 < 0.0);
}

#[test]
fn named_operators_are_upright_words_with_limits_only_when_they_take_them() {
    let g = glyphs(&lay("\\lim_{n}", true));
    assert_eq!(g[0].0, id('l'));
    assert!(g.last().unwrap().2 > 0.0, "below in display");
    let g = glyphs(&lay("\\sin x", true));
    assert_eq!(g[0].0, id('s'));
}

#[test]
fn explicit_space_is_in_eighteenths_of_an_em() {
    let (a, ia) = adv('\u{1D44E}');
    let (b, ib) = adv('\u{1D44F}');
    assert_close(lay("a\\quad b", false).width, a + ia + 1.0 + b + ib);
    assert_close(lay("a\\,b", false).width, a + ia + 3.0 / 18.0 + b + ib);
    assert_close(lay("a\\!b", false).width, a + ia - 3.0 / 18.0 + b + ib);
}

#[test]
fn text_is_upright_and_keeps_its_spaces() {
    let g = glyphs(&lay("\\text{ab}", false));
    assert_eq!(
        g.iter().map(|g| g.0).collect::<Vec<_>>(),
        [id('a'), id('b')]
    );
    let with_space = lay("\\text{a b}", false);
    let (a, b) = (adv('a').0, adv('b').0);
    assert_close(with_space.width, a + 0.25 + b);
}

#[test]
fn fonts_change_the_glyph() {
    assert_eq!(glyphs(&lay("\\mathbb{R}", false))[0].0, id('\u{211D}'));
    assert_eq!(glyphs(&lay("\\mathbf{x}", false))[0].0, id('\u{1D431}'));
    assert_eq!(glyphs(&lay("\\mathrm{x}", false))[0].0, id('x'));
}

#[test]
fn source_that_failed_is_drawn_as_error_glyphs() {
    let parsed = parse("a \\nosuchcommand");
    let l = layout(font(), &parsed.nodes, false);
    let errors = l
        .items
        .iter()
        .filter(|i| matches!(i, Item::Glyph { error: true, .. }))
        .count();
    assert_eq!(errors, "\\nosuchcommand".chars().count());
}

#[test]
fn every_construct_lays_out_finite_and_nonnegative() {
    let mut rng = Rng::new(0x1234_5678_9ABC_DEF1);
    let alphabet = [
        "x",
        "1",
        "+",
        "=",
        "-",
        "^",
        "_",
        "{",
        "}",
        "\\frac",
        "\\bigl",
        "\\Bigr",
        "\\phantom",
        "\\smash",
        "\\\\",
        "\\mod",
        "\\sqrt",
        "[",
        "]",
        "\\left(",
        "\\right)",
        "\\sum",
        "\\int",
        "\\lim",
        "\\text{ab}",
        "\\mathbb",
        "\\alpha",
        "\\,",
        "\\\\",
        "\\begin{matrix}",
        "\\end{matrix}",
        "&",
        "\\hat",
        "\\overset",
        "\\not",
        "\\displaystyle",
        "\\binom",
        "\\over",
        "'",
        "\\boxed",
        "\\cancel",
        "\\xrightarrow",
        "\\middle|",
        "\\unknown",
    ];
    for _ in 0..3_000 {
        let n = 1 + rng.below(24);
        let src: String = (0..n)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let l = layout(font(), &parse(&src).nodes, rng.below(2) == 0);
        for v in [l.width, l.ascent, l.descent] {
            assert!(v.is_finite() && v >= -1e-4, "{src:?}: {v}");
        }
        for item in &l.items {
            let all_finite = match item {
                Item::Glyph { x, y, size, .. } => [*x, *y, *size].iter().all(|v| v.is_finite()),
                Item::Rule {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => [*x, *y, *width, *height].iter().all(|v| v.is_finite()),
                Item::Line { x, y, x2, y2, .. } => [*x, *y, *x2, *y2].iter().all(|v| v.is_finite()),
            };
            assert!(all_finite, "{src:?}: {item:?}");
        }
    }
}

fn extent(l: &Layout) -> (f32, f32, f32) {
    (l.width, l.ascent, l.descent)
}

#[test]
fn an_accent_sits_over_its_letter_without_widening_it() {
    let plain = lay("x", false);
    let hat = lay("\\hat{x}", false);
    assert_close(hat.width, plain.width);
    assert!(hat.ascent > plain.ascent + 0.05, "{hat:?}");
    assert_close(hat.descent, plain.descent);
    assert!(
        !hat.items
            .iter()
            .any(|i| matches!(i, Item::Glyph { error: true, .. }))
    );
}

#[test]
fn a_tall_letter_pushes_its_accent_higher() {
    let low = lay("\\hat{x}", false);
    let high = lay("\\hat{b}", false);
    let gap = |l: &Layout, base: f32| l.ascent - base;
    assert!(gap(&high, lay("b", false).ascent) <= gap(&low, lay("x", false).ascent) + 1e-3);
}

#[test]
fn wide_marks_grow_with_their_body() {
    let short = lay("\\widehat{ab}", false);
    let long = lay("\\widehat{abcdefghij}", false);
    assert!(long.width > short.width);
    let hat = |l: &Layout| glyphs(l).last().copied().unwrap();
    assert!(
        hat(&long).0 != hat(&short).0,
        "a longer body takes a wider hat"
    );
    for src in [
        "\\overrightarrow{abcdefghijkl}",
        "\\overbrace{abcdefghijkl}",
        "\\underbrace{abcdefghijkl}",
    ] {
        let l = lay(src, false);
        let body = lay("abcdefghijkl", false);
        assert!(l.width >= body.width - 1e-3, "{src}: {l:?}");
    }
}

#[test]
fn braces_go_on_the_outside_of_their_body() {
    let body = lay("abc", false);
    let over = lay("\\overbrace{abc}", false);
    let under = lay("\\underbrace{abc}", false);
    assert!(over.ascent > body.ascent && close(over.descent, body.descent));
    assert!(under.descent > body.descent && close(under.ascent, body.ascent));
}

#[test]
fn a_script_on_a_brace_goes_above_it() {
    let plain = lay("\\overbrace{abc}", false);
    let labelled = lay("\\overbrace{abc}^{n}", false);
    assert!(labelled.ascent > plain.ascent);
    assert_close(labelled.width, plain.width.max(labelled.width));
    let under = lay("\\underbrace{abc}_{n}", false);
    assert!(under.descent > lay("\\underbrace{abc}", false).descent);
}

#[test]
fn bars_are_rules_as_wide_as_the_body() {
    let body = lay("xyz", false);
    for (src, above) in [("\\overline{xyz}", true), ("\\underline{xyz}", false)] {
        let l = lay(src, false);
        let r = rules(&l);
        assert_eq!(r.len(), 1, "{src}");
        assert_close(r[0].2, body.width);
        assert_eq!(r[0].1 < 0.0, above, "{src}");
    }
}

#[test]
fn overset_stacks_a_label_over_the_base() {
    let base = lay("=", false);
    let set = lay("\\overset{!}{=}", false);
    assert!(set.ascent > base.ascent + 0.1);
    assert_close(set.descent, base.descent.max(set.descent));
    let under = lay("\\underset{!}{=}", false);
    assert!(under.descent > base.descent + 0.1);
}

#[test]
fn an_arrow_is_longer_than_its_label() {
    let label = lay("\\scriptstyle abcdef", false);
    let arrow = lay("\\xrightarrow{abcdef}", false);
    assert!(arrow.width >= label.width, "{arrow:?}");
    assert!(arrow.ascent > label.ascent);
    let both = lay("\\xrightarrow[xy]{abcdef}", false);
    assert!(both.descent > arrow.descent);
    let left = lay("\\xleftarrow{abcdef}", false);
    assert_close(left.width, arrow.width);
}

#[test]
fn not_draws_a_slash_over_the_relation() {
    let eq = lay("=", false);
    let ne = lay("\\not=", false);
    assert_close(ne.width, eq.width);
    assert_eq!(glyphs(&ne).len(), 2);
}

#[test]
fn boxed_frames_the_body() {
    let body = lay("x", false);
    let boxed = lay("\\boxed{x}", false);
    assert_eq!(rules(&boxed).len(), 4);
    assert!(boxed.width > body.width + 0.2);
    assert!(boxed.ascent > body.ascent && boxed.descent > body.descent);
}

fn lines(l: &Layout) -> Vec<(f32, f32, f32, f32)> {
    l.items
        .iter()
        .filter_map(|i| match i {
            Item::Line { x, y, x2, y2, .. } => Some((*x, *y, *x2, *y2)),
            _ => None,
        })
        .collect()
}

#[test]
fn cancel_strikes_corner_to_corner_without_resizing() {
    let body = lay("x", false);
    for (src, count) in [("\\cancel{x}", 1), ("\\bcancel{x}", 1), ("\\xcancel{x}", 2)] {
        let l = lay(src, false);
        let strokes = lines(&l);
        assert_eq!(strokes.len(), count, "{src}");
        assert_eq!(l.width, body.width, "{src}");
        for (x, _, x2, _) in strokes {
            assert!(x < 0.0 && x2 > l.width, "{src}: reaches past the body");
        }
    }
    let (up, down) = (
        lines(&lay("\\cancel{x}", false)),
        lines(&lay("\\bcancel{x}", false)),
    );
    assert!(up[0].1 > up[0].3, "rises to the right");
    assert!(down[0].1 < down[0].3, "falls to the right");
}

#[test]
fn cancel_takes_the_colour_of_its_surroundings() {
    let l = lay("\\textcolor{red}{\\cancel{x}}", false);
    assert!(colors(&l).iter().all(Option::is_some));
}

#[test]
fn atop_is_a_fraction_without_bar_or_parentheses() {
    let l = lay("a \\atop b", true);
    assert!(rules(&l).is_empty());
    assert_eq!(glyphs(&l).len(), 2);
    let (_, up, down) = extent(&l);
    assert!(up > 0.5 && down > 0.5, "stacked around the axis");
}

#[test]
fn the_added_accents_and_boxes_lay_out_as_formulas() {
    for src in [
        "\\dddot{x}",
        "\\overleftrightarrow{xyz}",
        "\\widecheck{xyz}",
        "\\overparen{xyz}",
        "\\underbar{x}",
        "\\mathinner{x}",
        "\\strut x",
        "\\mathstrut x",
        "\\fbox{a b}",
    ] {
        assert!(!parse(src).has_error, "{src}");
        let l = lay(src, false);
        assert!(!glyphs(&l).is_empty() || !rules(&l).is_empty(), "{src}");
    }
    for mark in ['\u{20DB}', '\u{20E1}', '\u{30C}', '\u{23DC}'] {
        id(mark);
    }
    assert_eq!(rules(&lay("\\fbox{a b}", false)).len(), 4);
    let plain = lay("x", false);
    let wide = lay("\\overparen{xyzxyz}", false);
    assert!(wide.ascent > plain.ascent, "the paren sits above");
    let strut = lay("\\strut", false);
    assert_eq!(strut.width, 0.0);
    assert!(strut.ascent > plain.ascent);
}

#[test]
fn a_size_command_scales_what_follows_it_in_its_group() {
    let plain = lay("xy", false);
    let big = lay("x \\Large y", false);
    let (x, y) = (lay("x", false), lay("y", false));
    let (gx, gy) = (glyphs(&plain), glyphs(&big));
    assert_eq!(gx[0].3, gy[0].3, "before it: unchanged");
    assert!((gy[1].3 / gx[1].3 - 1.44).abs() < 1e-4, "after it: scaled");
    assert!(big.width > plain.width && big.ascent > x.ascent.max(y.ascent));
    let grouped = lay("{\\tiny x} y", false);
    assert!((glyphs(&grouped)[0].3 - 0.5).abs() < 1e-4);
    assert_eq!(glyphs(&grouped)[1].3, 1.0, "the group ends the size");
    assert!(!parse("\\tiny\\scriptsize\\small\\normalsize\\large\\LARGE\\huge\\Huge x").has_error);
}

#[test]
fn above_and_genfrac_are_fractions_with_or_without_a_bar() {
    assert_eq!(rules(&lay("a \\above 1pt b", true)).len(), 1);
    assert!(rules(&lay("a \\above 0pt b", true)).is_empty());
    assert_eq!(rules(&lay("\\genfrac{}{}{}{}{a}{b}", true)).len(), 1);
    assert!(rules(&lay("\\genfrac{}{}{0pt}{}{a}{b}", true)).is_empty());
    let bare = lay("\\genfrac{}{}{0pt}{}{a}{b}", true);
    let wrapped = lay("\\genfrac{(}{)}{0pt}{}{a}{b}", true);
    assert_eq!(glyphs(&wrapped).len(), glyphs(&bare).len() + 2);
    assert!(wrapped.width > bare.width);
    let text = lay("\\genfrac{}{}{}{1}{a}{b}", true);
    assert!(
        text.ascent + text.descent
            < lay("\\genfrac{}{}{}{0}{a}{b}", true).ascent
                + lay("\\genfrac{}{}{}{0}{a}{b}", true).descent
    );
    assert!(parse("\\genfrac{(}").has_error);
}

#[test]
fn a_very_tall_delimiter_is_assembled_from_parts() {
    let l = lay(
        "\\left(\\frac{\\frac{\\frac{a}{b}}{\\frac{c}{d}}}{\\frac{\\frac{e}{f}}{\\frac{g}{h}}}\\right)",
        true,
    );
    let (_, ascent, descent) = extent(&l);
    let paren = id('(');
    let tallest = glyphs(&l).iter().filter(|g| g.0 == paren).count();
    assert_eq!(tallest, 0, "no plain paren glyph at this height");
    assert!(ascent + descent > 3.0);
}

#[test]
fn a_table_is_centred_on_the_math_axis() {
    for src in [
        "\\begin{matrix} a & b \\\\ c & d \\end{matrix}",
        "\\begin{cases} a & b \\\\ c & d \\\\ e & f \\end{cases}",
        "\\begin{array}{cc} a & b \\end{array}",
    ] {
        let l = lay(src, false);
        assert_close(l.ascent - l.descent, 2.0 * font().consts.axis_height);
    }
}

#[test]
fn each_row_is_at_least_a_strut_tall() {
    let two = lay("\\begin{matrix} a \\\\ b \\end{matrix}", false);
    assert_close(two.ascent + two.descent, 2.4);
    let tall = lay("\\begin{matrix} \\dfrac{a}{b} \\\\ c \\end{matrix}", false);
    assert!(tall.ascent + tall.descent > 2.4);
}

#[test]
fn matrices_are_fenced_and_a_plain_matrix_is_not() {
    let plain = lay("\\begin{matrix} a & b \\end{matrix}", false);
    let paren = lay("\\begin{pmatrix} a & b \\end{pmatrix}", false);
    let (open, close) = (id('('), id(')'));
    let count = |l: &Layout, g: u16| glyphs(l).iter().filter(|x| x.0 == g).count();
    assert_eq!((count(&plain, open), count(&plain, close)), (0, 0));
    assert!(paren.width > plain.width);
    assert!(glyphs(&paren).first().unwrap().1 < glyphs(&paren).last().unwrap().1);
}

#[test]
fn cases_has_a_brace_on_the_left_only() {
    let cases = lay("\\begin{cases} a & b \\end{cases}", false);
    let bare = lay("\\begin{matrix} a & b \\end{matrix}", false);
    assert!(cases.width > bare.width);
    let first = glyphs(&cases)[0];
    assert!(first.1 < 0.1, "the brace comes first: {first:?}");
}

#[test]
fn array_rules_come_from_hline_and_the_column_spec() {
    let l = lay(
        "\\begin{array}{c|c} a & b \\\\ \\hline c & d \\end{array}",
        false,
    );
    let r = rules(&l);
    assert_eq!(r.len(), 2, "{r:?}");
    let (wide, tall): (Vec<(f32, f32, f32, f32)>, Vec<(f32, f32, f32, f32)>) =
        r.iter().partition(|x| x.2 > x.3);
    assert_eq!((wide.len(), tall.len()), (1, 1));
    assert_close(wide[0].2, l.width);
}

#[test]
fn columns_align_as_the_spec_says() {
    let x_of = |src: &str, ch: char| {
        let l = lay(src, false);
        glyphs(&l).iter().find(|g| g.0 == id(ch)).unwrap().1
    };
    let (left, right, centre) = (
        x_of("\\begin{array}{l} a \\\\ bbbb \\end{array}", '𝑎'),
        x_of("\\begin{array}{r} a \\\\ bbbb \\end{array}", '𝑎'),
        x_of("\\begin{array}{c} a \\\\ bbbb \\end{array}", '𝑎'),
    );
    assert!(left < centre && centre < right, "{left} {centre} {right}");
}

#[test]
fn aligned_lines_its_relations_up_and_spaces_them() {
    let l = lay("\\begin{aligned} a &= b \\\\ ccc &= d \\end{aligned}", true);
    let eq = id('=');
    let xs: Vec<f32> = glyphs(&l)
        .iter()
        .filter(|g| g.0 == eq)
        .map(|g| g.1)
        .collect();
    assert_eq!(xs.len(), 2);
    assert_close(xs[0], xs[1]);
    let a_end = glyphs(&l)
        .iter()
        .find(|g| g.0 == id('𝑎'))
        .map(|g| g.1 + adv('𝑎').0)
        .unwrap();
    assert!(xs[0] > a_end + 0.2, "a thick space before the relation");
}

#[test]
fn an_empty_environment_is_empty() {
    let l = lay("\\begin{matrix}\\end{matrix}", false);
    assert!(l.items.is_empty());
}

#[test]
fn an_overset_relation_still_spaces_as_a_relation() {
    let rel = lay("a\\underset{n}{\\to}b", false);
    let ord = lay("a{\\underset{n}{\\to}}b", false);
    assert!(
        rel.width > ord.width + 0.2,
        "{} vs {}",
        rel.width,
        ord.width
    );
}

#[test]
fn a_line_break_outside_an_environment_stacks_centred_lines() {
    let one = lay("a", true);
    let two = lay("a \\\\ bbbb", true);
    let gathered = lay("\\begin{gathered} a \\\\ bbbb \\end{gathered}", true);
    assert_eq!(two, gathered);
    assert!(two.ascent + two.descent > 1.5 * (one.ascent + one.descent));
    // A break ending the formula leaves no empty line.
    assert_eq!(lay("a \\\\", true), one);
}

#[test]
fn sized_delimiters_grow_with_their_level_and_space_as_their_side() {
    let height = |src: &str| {
        let l = lay(src, false);
        l.ascent + l.descent
    };
    let heights: Vec<f32> = ["\\big(", "\\Big(", "\\bigg(", "\\Bigg("]
        .into_iter()
        .map(height)
        .collect();
    assert!(heights.windows(2).all(|w| w[0] < w[1]), "{heights:?}");
    assert!(heights[0] >= 1.2 - 0.01, "{heights:?}");
    // `\bigl`/`\bigr` space like an opener and a closer: nothing around them.
    let plain = lay("(x)", false).width;
    let sized = lay("\\bigl( x \\bigr)", false).width;
    assert!(sized > plain && sized < plain + 0.6, "{sized} vs {plain}");
    assert!(lay("\\Biggm| x", false).width > 0.0);
}

#[test]
fn phantoms_keep_the_size_they_say_and_draw_nothing() {
    let body = lay("\\frac{x}{yyy}", false);
    let both = lay("\\phantom{\\frac{x}{yyy}}", false);
    assert!(both.items.is_empty());
    assert_eq!(
        (both.width, both.ascent, both.descent),
        (body.width, body.ascent, body.descent)
    );
    let h = lay("\\hphantom{\\frac{x}{yyy}}", false);
    assert_eq!((h.width, h.ascent, h.descent), (body.width, 0.0, 0.0));
    let v = lay("\\vphantom{\\frac{x}{yyy}}", false);
    assert_eq!(
        (v.width, v.ascent, v.descent),
        (0.0, body.ascent, body.descent)
    );
    let smashed = lay("\\smash{\\frac{x}{yyy}}", false);
    assert_eq!(smashed.items, body.items);
    assert_eq!((smashed.ascent, smashed.descent), (0.0, 0.0));
}

#[test]
fn a_horizontal_space_adds_its_width_in_em() {
    let plain = lay("ab", false).width;
    let spaced = lay("a\\hspace{1.5em}b", false).width;
    assert!((spaced - plain - 1.5).abs() < 0.01, "{spaced} vs {plain}");
    let points = lay("a\\hspace{10pt}b", false).width;
    assert!((points - plain - 1.0).abs() < 0.01);
    assert!(parse("\\hspace{3furlong}").has_error);
}

#[test]
fn mod_and_pod_and_the_marks_that_number_an_equation() {
    for src in [
        "a \\mod n",
        "a \\pod{n}",
        "a \\newline b",
        "x \\nonumber",
        "x \\notag \\label{eq:1}",
    ] {
        assert!(!parse(src).has_error, "{src}");
    }
    assert_eq!(lay("x \\label{a}", true), lay("x", true));
    assert!(lay("a \\mod n", true).width > lay("a n", true).width);
}

/// The colour of each glyph and rule, in drawing order.
fn colors(l: &Layout) -> Vec<Option<u32>> {
    l.items
        .iter()
        .map(|i| match i {
            Item::Glyph { color, .. } | Item::Rule { color, .. } | Item::Line { color, .. } => {
                *color
            }
        })
        .collect()
}

#[test]
fn textcolor_colours_its_body_and_nothing_else() {
    let l = lay("a\\textcolor{red}{b\\frac{c}{d}}e", false);
    let c = colors(&l);
    // a, b, c, d, the bar, e
    assert_eq!(c.len(), 6);
    assert_eq!(c[0], None);
    assert!(c[1..5].iter().all(|&x| x == Some(0xff0000)), "{c:?}");
    assert_eq!(c[5], None);
    // Colour changes no size.
    let plain = lay("a{b\\frac{c}{d}}e", false);
    assert!((plain.width - l.width).abs() < 1e-4);
}

#[test]
fn color_switches_for_the_rest_of_its_group() {
    let l = lay("a{\\color{#0f0} b}c\\color{blue}d e", false);
    assert_eq!(
        colors(&l),
        [None, Some(0x00ff00), None, Some(0x0000ff), Some(0x0000ff)]
    );
}

#[test]
fn an_inner_color_wins_and_an_unknown_one_is_source() {
    let l = lay("\\textcolor{red}{a\\textcolor{#123456}{b}}", false);
    assert_eq!(colors(&l), [Some(0xff0000), Some(0x123456)]);
    assert!(parse("\\textcolor{nosuchcolor}{x}").has_error);
    assert!(parse("\\color{#12}x").has_error);
}

#[test]
fn a_tag_numbers_a_display_formula_and_changes_nothing_else() {
    let plain = lay("x=1", true);
    let tagged = lay("x=1\\tag{2.1}", true);
    assert_eq!(glyphs(&plain), glyphs(&tagged));
    assert_eq!(plain.width, tagged.width);
    let tag = tagged.tag.expect("the number");
    // "(2.1)": five glyphs.
    assert_eq!(glyphs(&tag).len(), 5);
    let starred = lay("x\\tag*{A}", true).tag.unwrap();
    assert_eq!(glyphs(&starred).len(), 1);
    // Inline, a tag shows nothing.
    assert!(lay("x\\tag{1}", false).tag.is_none());
    // The last one counts.
    assert_eq!(
        glyphs(&lay("x\\tag{1}\\tag{22}", true).tag.unwrap()).len(),
        4
    );
}
