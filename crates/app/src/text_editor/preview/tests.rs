use super::*;

/// The raw ranges Live Preview paints as a link on an inactive line.
fn rendered_link_texts(line: &str) -> Vec<Range<usize>> {
    build_display_line(line, false)
        .segments
        .iter()
        .filter(|s| matches!(s.style, InlineStyle::Link | InlineStyle::WikiLink))
        .filter(|s| !s.is_hidden())
        .map(|s| s.raw.clone())
        .collect()
}

fn scanned_link_texts(line: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    jade_core::link_scan::scan_line(line, |l| {
        if !l.text.is_empty() {
            out.push(l.text);
        }
    });
    out
}

/// Rendering and `link_scan` must agree on what is a link, with one known
/// gap: Live Preview does not scan inside bold, italic or strikethrough
/// (`**[[a]]**` paints `[[a]]` as plain bold text), while the scanner,
/// like the backlink index, still counts it. They share the leaf
/// parsers; this guards the loops around them.
fn assert_scanner_matches_rendering(line: &str) {
    let rendered = rendered_link_texts(line);
    let display = build_display_line(line, false);
    let scanned = scanned_link_texts(line);
    for r in &rendered {
        assert!(scanned.contains(r), "rendered link not scanned: {line:?}");
    }
    for r in scanned.iter().filter(|r| !rendered.contains(r)) {
        let inside_emphasis = display.segments.iter().any(|s| {
            matches!(
                s.style,
                InlineStyle::Bold | InlineStyle::Italic | InlineStyle::Strikethrough
            ) && s.raw.start <= r.start
                && r.end <= s.raw.end
        });
        assert!(inside_emphasis, "scanned link not rendered: {line:?} {r:?}");
    }
}

#[test]
fn link_scan_agrees_with_rendering_on_hand_written_lines() {
    for line in [
        "plain [[a]] and [[b|c]] and [[d#e]] end",
        "**[[bold link]]** and *[t](u)* and ~~<https://x.y>~~",
        "`[[code]]` then [[real]]",
        "\\[[escaped]] [[after]]",
        "![img](u.png) [text](u.md)",
        "see https://a.b/c. and <me@x.y> and <div>",
        "[a](b) [c](d \"title\") [e](<f g>)",
        "a [[unclosed and [t](unclosed",
        "[[a]] [[b]] [[c]]",
        "# [[heading link]]",
        "> quoted [[link]]",
        "- item [t](u)",
        "[[ ]] [[]] [[a|]] [[#h]]",
        "[x](y)(z) [[a]](b)",
        "é [[Ünï|ä]] ✓ [日本](ü.md)",
        "$[[a]]$ [[b]] $x$ $5 [[c]] $10",
        "$a [b](c) d$ [e](f)",
        "$$[[a]]$$ and `$` [[b]] $",
    ] {
        assert_scanner_matches_rendering(line);
    }
}

#[test]
fn link_scan_agrees_with_rendering_on_random_inline_soup() {
    let tokens = [
        "[[",
        "]]",
        "[",
        "]",
        "(",
        ")",
        "<",
        ">",
        "`",
        "\\",
        "|",
        "#",
        "**",
        "*",
        "_",
        "~~",
        "https://x.y",
        "a",
        "b c",
        " ",
        "!",
        "é",
        "&amp;",
        "[[a]]",
        "[t](u)",
        "<https://z>",
        "$",
        "$$",
        "$x$",
        "\\$",
    ];
    let mut state = 0x9e37_79b9_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as usize
    };
    for _ in 0..20_000 {
        let line: String = (0..next() % 12)
            .map(|_| tokens[next() % tokens.len()])
            .collect();
        assert_scanner_matches_rendering(&line);
    }
}

#[test]
fn link_scan_agrees_with_rendering_on_the_stress_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benches/fixtures/large_markdown_stress_test.md"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines().take(20_000) {
        assert_scanner_matches_rendering(line);
    }
}

#[test]
fn math_is_opaque_to_emphasis_and_links() {
    // The LaTeX is one drawn segment: no italic from `*`, no link from `[[`.
    let line = "a $x*y*z$ b $[[n]]$ c";
    let dl = build_display_line(line, false);
    let math: Vec<_> = dl
        .segments
        .iter()
        .filter(|s| s.style == InlineStyle::Math)
        .collect();
    assert_eq!(math.len(), 2);
    assert!(math.iter().all(|s| s.is_replaced() && !s.is_hidden()));
    assert!(
        dl.segments
            .iter()
            .all(|s| !matches!(s.style, InlineStyle::Italic | InlineStyle::WikiLink))
    );
    // Currency is not math: the emphasis after it still renders.
    let dl = build_display_line("$5 and *x*", false);
    assert_eq!(dl.display_text, "$5 and x");
}

#[test]
fn a_formula_reserves_blank_columns_and_the_active_line_shows_source() {
    let line = "see $x^2$ now";
    let dl = build_display_line(line, false);
    let seg = dl
        .segments
        .iter()
        .find(|s| s.style == InlineStyle::Math)
        .expect("a math segment");
    assert_eq!(&line[seg.raw.clone()], "$x^2$");
    assert!(
        dl.display_text[seg.display.clone()]
            .chars()
            .all(|c| c == ' ')
    );
    assert!(!seg.display.is_empty());
    // The caret line renders raw, like every construct.
    let active = build_display_line(line, true);
    assert_eq!(active.display_text, line);
}

#[test]
fn a_formula_that_does_not_parse_stays_source() {
    let line = "a $\\frac{a$ b";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
    assert!(dl.segments.iter().all(|s| s.style != InlineStyle::Math));
}

#[test]
fn a_formula_is_drawn_once_the_notes_macro_defines_it() {
    let line = "let $f: \\R \\to \\R$ here";
    let unknown = build_display_line(line, false);
    assert!(
        unknown
            .segments
            .iter()
            .all(|s| s.style != InlineStyle::Math)
    );
    let macros = Macros::collect(["\\newcommand{\\R}{\\mathbb{R}}"]);
    let dl = build_display_line_with(line, false, &macros);
    assert_eq!(
        dl.segments
            .iter()
            .filter(|s| s.style == InlineStyle::Math)
            .count(),
        1
    );
    // A macro that never ends is source, not a hang.
    let looping = Macros::collect(["\\newcommand{\\R}{\\R}"]);
    let dl = build_display_line_with(line, false, &looping);
    assert_eq!(dl.display_text, line);
}

#[test]
fn a_formula_wider_than_a_row_is_shown_as_its_source() {
    let macros = Macros::default();
    let line = "a $x^2$ b $\\frac{a}{b} + \\frac{c}{d} + \\frac{e}{f}$ c";
    let drawn = build_display_line_with(line, false, &macros);
    let wide = drawn
        .segments
        .iter()
        .filter(|s| s.style == InlineStyle::Math)
        .map(|s| s.display.len())
        .max()
        .unwrap();
    let narrow = drawn
        .segments
        .iter()
        .filter(|s| s.style == InlineStyle::Math)
        .map(|s| s.display.len())
        .min()
        .unwrap();
    assert!(narrow < wide);
    // A row that holds the wide one changes nothing.
    let mut same = drawn.clone();
    show_wide_formulas(&mut same, line, wide);
    assert_eq!(same, drawn);
    // One that holds only the narrow one shows the wide one's source.
    let mut shown = drawn.clone();
    show_wide_formulas(&mut shown, line, narrow);
    let source = "$\\frac{a}{b} + \\frac{c}{d} + \\frac{e}{f}$";
    assert!(
        shown.display_text.contains(source),
        "{}",
        shown.display_text
    );
    assert_eq!(
        shown.display_text.len(),
        drawn.display_text.len() - wide + source.len()
    );
    let math: Vec<_> = shown
        .segments
        .iter()
        .filter(|s| s.style == InlineStyle::Math)
        .collect();
    assert_eq!(math.len(), 1, "only the narrow formula is still drawn");
    // Every segment still points at its own text, in order.
    let mut end = 0;
    for seg in &shown.segments {
        assert_eq!(seg.display.start, end);
        end = seg.display.end;
    }
    assert_eq!(end, shown.display_text.len());
    let at = shown
        .segments
        .iter()
        .find(|s| shown.display_text[s.display.clone()] == *source)
        .unwrap();
    assert_eq!(at.style, InlineStyle::Plain);
    assert!(!at.is_replaced());
    // Unknown width leaves it alone.
    let mut unknown = drawn.clone();
    show_wide_formulas(&mut unknown, line, 0);
    assert_eq!(unknown, drawn);
}

#[test]
fn a_table_cell_draws_a_formula_that_fits_its_column() {
    let none = Macros::default();
    let dl = build_cell_line_with("a $x^2$ b", &none, 12);
    assert_eq!(
        dl.segments
            .iter()
            .filter(|s| s.style == InlineStyle::Math)
            .count(),
        1
    );
    // Wider than the column once drawn: shown as typed.
    let dl = build_cell_line_with("$WWWWWWWW$", &none, 10);
    assert_eq!(dl.display_text, "$WWWWWWWW$");
    let macros = Macros::collect(["\\newcommand{\\R}{\\mathbb{R}}"]);
    let dl = build_cell_line_with("$\\R$", &macros, 5);
    assert!(dl.segments.iter().any(|s| s.style == InlineStyle::Math));
}

#[test]
fn a_table_cell_without_the_notes_macros_never_gets_a_drawn_formula() {
    let dl = build_cell_line("$x^2$");
    assert_eq!(dl.display_text, "$x^2$");
}

#[test]
fn a_failed_double_dollar_does_not_open_a_span_with_its_second_dollar() {
    // `$$ x$` would be inline math `x` if the second `$` could open one; it
    // must read as text so rendering and the link scan agree.
    let dl = build_display_line("$$ *x*$ [[a]]", false);
    assert!(
        visible_styles(&dl)
            .iter()
            .any(|(_, style)| *style == InlineStyle::WikiLink)
    );
}

fn visible_styles(dl: &DisplayLine) -> Vec<(String, InlineStyle)> {
    dl.segments
        .iter()
        .filter(|s| !s.is_hidden())
        .map(|s| (dl.display_text[s.display.clone()].to_string(), s.style))
        .collect()
}

fn assert_contiguous(dl: &DisplayLine, raw_len: usize) {
    let mut expected_raw = 0;
    let mut expected_display = 0;
    for seg in &dl.segments {
        assert_eq!(
            seg.raw.start, expected_raw,
            "raw gap/overlap in {:?}",
            dl.segments
        );
        assert_eq!(
            seg.display.start, expected_display,
            "display gap/overlap in {:?}",
            dl.segments
        );
        expected_raw = seg.raw.end;
        expected_display = seg.display.end;
    }
    assert_eq!(expected_raw, raw_len);
    assert_eq!(expected_display, dl.display_text.len());
}

#[test]
fn active_line_is_shown_verbatim_and_unstyled() {
    for line in ["plain **bold** [[Link]]", "#nospace", "a # b"] {
        let dl = build_display_line(line, true);
        assert_eq!(dl.display_text, line);
        assert_eq!(
            dl.segments,
            vec![DisplaySegment {
                raw: 0..line.len(),
                display: 0..line.len(),
                style: InlineStyle::Plain,
                replaced: false
            }]
        );
    }
}

/// The raw line of a heading keeps its level, so it is not resized when the
/// caret enters it (#58 C7); the text itself is still verbatim.
#[test]
fn the_active_line_of_a_heading_keeps_its_level() {
    for (line, level) in [
        ("# **bold** [[Link]]", 1),
        ("### Title", 3),
        ("> ## quoted", 2),
        ("- # item", 1),
    ] {
        let dl = build_display_line(line, true);
        assert_eq!(dl.display_text, line);
        assert_eq!(dl.heading_level(), Some(level), "{line}");
        assert_eq!(dl.segments.len(), 1);
        assert_contiguous(&dl, line.len());
    }
}

#[test]
fn empty_line_is_a_single_empty_segment() {
    let dl = build_display_line("", false);
    assert_eq!(dl.display_text, "");
    assert_eq!(
        dl.segments,
        vec![DisplaySegment {
            raw: 0..0,
            display: 0..0,
            style: InlineStyle::Plain,
            replaced: false
        }]
    );
}

#[test]
fn plain_line_with_no_constructs_is_unchanged() {
    let dl = build_display_line("just plain text", false);
    assert_eq!(dl.display_text, "just plain text");
    assert_contiguous(&dl, "just plain text".len());
    assert_eq!(dl.indent, LineIndent::None);
}

#[test]
fn blockquote_indent_matches_its_nesting_depth() {
    assert_eq!(
        build_display_line("> one level", false).indent,
        LineIndent::Blockquote(1)
    );
    assert_eq!(
        build_display_line(">> two levels", false).indent,
        LineIndent::Blockquote(2)
    );
    assert_eq!(
        build_display_line("> > also two levels", false).indent,
        LineIndent::Blockquote(2)
    );
}

#[test]
fn list_item_indent_is_always_one_level() {
    assert_eq!(
        build_display_line("- a dash item", false).indent,
        LineIndent::List(1)
    );
    assert_eq!(
        build_display_line("12. an ordered item", false).indent,
        LineIndent::List(1)
    );
}

#[test]
fn indent_applies_even_on_the_active_line() {
    // The active line renders raw/unstyled (see
    // `active_line_is_shown_verbatim_and_unstyled`), but indent is a
    // structural property, not a Live Preview hiding choice — it must
    // still be set so `element.rs` keeps shifting the line right while
    // it's being edited.
    assert_eq!(
        build_display_line("> quoted", true).indent,
        LineIndent::Blockquote(1)
    );
    assert_eq!(
        build_display_line("- item", true).indent,
        LineIndent::List(1)
    );
}

#[test]
fn line_indent_level_matches_blockquote_depth_or_one_for_a_list() {
    assert_eq!(LineIndent::None.level(), 0);
    assert_eq!(LineIndent::Blockquote(3).level(), 3);
    assert_eq!(LineIndent::List(1).level(), 1);
}

#[test]
fn heading_hides_its_marker_and_styles_the_rest() {
    let line = "## Section Title";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "Section Title");
    assert_eq!(
        visible_styles(&dl),
        vec![("Section Title".to_string(), InlineStyle::Heading(2))]
    );
    assert_contiguous(&dl, line.len());
}

/// A heading whose whole text is another construct has no heading-styled
/// visible run, but is still a heading (it takes its size, #58).
#[test]
fn a_heading_made_only_of_another_construct_keeps_its_level() {
    for (line, level) in [
        ("## **bold**", 2),
        ("# [[note]]", 1),
        ("### `code`", 3),
        ("# $x^2$", 1),
        ("> # **quoted**", 1),
        ("- ## **item**", 2),
    ] {
        let dl = build_display_line(line, false);
        assert_eq!(dl.heading_level(), Some(level), "{line}");
        assert_contiguous(&dl, line.len());
    }
    assert_eq!(
        build_display_line("plain **bold**", false).heading_level(),
        None
    );
}

#[test]
fn heading_without_a_following_space_is_not_a_heading() {
    let line = "##nospace";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "##nospace");
}

#[test]
fn bold_hides_its_delimiters_and_styles_the_content() {
    let line = "a **bold** word";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a bold word");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("a ".to_string(), InlineStyle::Plain),
            ("bold".to_string(), InlineStyle::Bold),
            (" word".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn underscore_bold_is_also_recognized() {
    let dl = build_display_line("__bold__", false);
    assert_eq!(dl.display_text, "bold");
    assert_eq!(
        visible_styles(&dl),
        vec![("bold".to_string(), InlineStyle::Bold)]
    );
}

#[test]
fn single_asterisk_is_italic() {
    let line = "a *italic* word";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a italic word");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("a ".to_string(), InlineStyle::Plain),
            ("italic".to_string(), InlineStyle::Italic),
            (" word".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn strikethrough_hides_its_delimiters_and_styles_the_content() {
    let line = "a ~~gone~~ word";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a gone word");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("a ".to_string(), InlineStyle::Plain),
            ("gone".to_string(), InlineStyle::Strikethrough),
            (" word".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn a_single_tilde_is_not_strikethrough() {
    let line = "a ~lone~ tilde";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
}

#[test]
fn unterminated_strikethrough_falls_back_to_plain_text() {
    let line = "this ~~never closes";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
    assert_contiguous(&dl, line.len());
}

#[test]
fn inline_code_hides_its_backticks() {
    let line = "run `cargo test` now";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "run cargo test now");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("run ".to_string(), InlineStyle::Plain),
            ("cargo test".to_string(), InlineStyle::Code),
            (" now".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn wikilink_without_alias_shows_the_target_and_hides_the_brackets() {
    let line = "see [[Other Note]] please";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "see Other Note please");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("see ".to_string(), InlineStyle::Plain),
            ("Other Note".to_string(), InlineStyle::WikiLink),
            (" please".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn wikilink_with_alias_shows_only_the_alias() {
    let line = "[[Other Note|here]]";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "here");
    assert_eq!(
        visible_styles(&dl),
        vec![("here".to_string(), InlineStyle::WikiLink)]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn wikilink_with_heading_hides_the_heading_suffix() {
    let line = "[[Note#Section]]";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "Note");
    assert_eq!(
        visible_styles(&dl),
        vec![("Note".to_string(), InlineStyle::WikiLink)]
    );
}

#[test]
fn unterminated_bold_falls_back_to_plain_text() {
    let line = "this **never closes";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
    assert_contiguous(&dl, line.len());
}

#[test]
fn unterminated_wikilink_falls_back_to_plain_text() {
    let line = "this [[never closes";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
}

#[test]
fn heading_content_can_itself_contain_inline_constructs() {
    let line = "# a **bold** title";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a bold title");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("a ".to_string(), InlineStyle::Heading(1)),
            ("bold".to_string(), InlineStyle::Bold),
            (" title".to_string(), InlineStyle::Heading(1)),
        ]
    );
}

#[test]
fn display_and_raw_offsets_round_trip_around_a_bold_span() {
    let line = "a **bold** word"; // raw: a(1) **(2) bold(4) **(2) ' word'(5)
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a bold word");

    // Start of "bold" in raw (byte 4, right after "a **") maps to
    // display byte 2 (right after "a ").
    assert_eq!(dl.display_offset_for_raw(4), 2);
    assert_eq!(dl.raw_offset_for_display(2), 4);

    // A raw offset inside the hidden leading "**" collapses to the same
    // display point as the text immediately following it.
    assert_eq!(dl.display_offset_for_raw(2), 2);
    assert_eq!(dl.display_offset_for_raw(3), 2);

    // End of line round-trips too.
    assert_eq!(dl.display_offset_for_raw(line.len()), dl.display_text.len());
    assert_eq!(dl.raw_offset_for_display(dl.display_text.len()), line.len());
}

#[test]
fn raw_and_display_offsets_round_trip_on_an_entirely_hidden_line() {
    let dl = build_display_line("# ", false);
    assert_eq!(dl.display_text, "");
    assert_eq!(dl.raw_offset_for_display(0), 2);
    assert_eq!(dl.display_offset_for_raw(0), 0);
    assert_eq!(dl.display_offset_for_raw(1), 0);
    assert_eq!(dl.display_offset_for_raw(2), 0);
}

#[test]
fn blockquote_hides_its_prefix_and_styles_the_rest() {
    let line = "> a quoted line";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a quoted line");
    assert_eq!(
        visible_styles(&dl),
        vec![("a quoted line".to_string(), InlineStyle::Blockquote(1))]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn nested_blockquote_prefixes_increase_depth() {
    let dl = build_display_line(">> deeper", false);
    assert_eq!(dl.display_text, "deeper");
    assert_eq!(
        visible_styles(&dl),
        vec![("deeper".to_string(), InlineStyle::Blockquote(2))]
    );

    let dl = build_display_line("> > also deeper", false);
    assert_eq!(dl.display_text, "also deeper");
    assert_eq!(
        visible_styles(&dl),
        vec![("also deeper".to_string(), InlineStyle::Blockquote(2))]
    );
}

#[test]
fn blockquote_content_can_itself_contain_inline_constructs() {
    let line = "> a **bold** quote";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a bold quote");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("a ".to_string(), InlineStyle::Blockquote(1)),
            ("bold".to_string(), InlineStyle::Bold),
            (" quote".to_string(), InlineStyle::Blockquote(1)),
        ]
    );
}

#[test]
fn a_heading_inside_a_blockquote_hides_both_prefixes() {
    let line = "> # Quoted heading";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "Quoted heading");
    assert_eq!(
        visible_styles(&dl),
        vec![("Quoted heading".to_string(), InlineStyle::Heading(1))]
    );
    assert_contiguous(&dl, line.len());
    assert_eq!(
        dl.indent,
        LineIndent::Blockquote(1),
        "the line should still indent as a blockquote"
    );
}

#[test]
fn a_list_item_inside_a_blockquote_hides_both_markers() {
    let line = "> - a quoted item";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  a quoted item");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
            ("a quoted item".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
    assert_eq!(dl.indent, LineIndent::Blockquote(1));
}

#[test]
fn a_nested_blockquote_does_not_also_trigger_list_compounding() {
    // ">>" is already one `blockquote_prefix` match at depth 2 (see
    // `line_indent`'s own doc comment) — `compound_inner` must not also
    // go looking for a list marker right after it.
    let line = ">> deeper text";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "deeper text");
    assert_eq!(
        visible_styles(&dl),
        vec![("deeper text".to_string(), InlineStyle::Blockquote(2))]
    );
}

#[test]
fn a_bare_greater_than_without_following_text_is_still_a_blockquote() {
    let dl = build_display_line(">", false);
    assert_eq!(dl.display_text, "");
    assert_contiguous(&dl, 1);
}

#[test]
fn unordered_list_marker_is_replaced_with_a_bullet() {
    for marker in ["-", "*", "+"] {
        let line = format!("{marker} first item");
        let dl = build_display_line(&line, false);
        assert_eq!(dl.display_text, "  first item", "marker {marker:?}");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
                ("first item".to_string(), InlineStyle::Plain)
            ],
            "marker {marker:?}"
        );
    }
}

#[test]
fn ordered_list_marker_is_replaced_with_its_canonical_form() {
    let line = "12) ninth item";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "12. ninth item");
    assert_contiguous(&dl, line.len());
}

#[test]
fn list_item_content_can_itself_contain_inline_constructs() {
    let line = "- a [[Link]] here";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  a Link here");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
            ("a ".to_string(), InlineStyle::Plain),
            ("Link".to_string(), InlineStyle::WikiLink),
            (" here".to_string(), InlineStyle::Plain),
        ]
    );
}

#[test]
fn a_heading_inside_a_list_item_hides_both_markers() {
    let line = "- # Heading item";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  Heading item");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
            ("Heading item".to_string(), InlineStyle::Heading(1))
        ]
    );
    assert_contiguous(&dl, line.len());
    assert_eq!(dl.indent, LineIndent::List(1));
}

#[test]
fn a_blockquote_inside_a_list_item_hides_both_markers() {
    let line = "- > quoted item";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  quoted item");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
            ("quoted item".to_string(), InlineStyle::Blockquote(1))
        ]
    );
    assert_contiguous(&dl, line.len());
    assert_eq!(
        dl.indent,
        LineIndent::List(1),
        "list indent wins over the inner (compounded, same-line) blockquote's"
    );
}

#[test]
fn a_second_dash_inside_a_list_item_is_not_treated_as_a_nested_list() {
    // Nested-list recognition is indentation-based (see
    // `classify_block_context`), not triggered by a second marker
    // appearing right after the first on the very same line — per
    // CommonMark itself, an unindented "- " immediately following
    // another is just plain text starting with "-".
    let line = "- - not nested";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  - not nested");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("  ".to_string(), InlineStyle::Widget(Widget::Bullet)),
            ("- not nested".to_string(), InlineStyle::Plain)
        ]
    );
}

#[test]
fn a_digit_run_without_a_following_space_is_not_a_list_marker() {
    let dl = build_display_line("12.5 is not a list", false);
    assert_eq!(dl.display_text, "12.5 is not a list");
}

#[test]
fn indented_list_marker_reports_its_leading_whitespace_separately() {
    let (marker, indent, marker_len) = indented_list_marker("   - nested item").unwrap();
    assert!(matches!(marker, ListMarker::Unordered));
    assert_eq!(indent, 3);
    assert_eq!(marker_len, 2);
}

#[test]
fn an_indented_marker_still_hides_its_whitespace_and_marker_in_isolation() {
    // `build_display_line` alone (no document context) can't know this
    // is actually nested — it resolves to depth 1 regardless, same as
    // an unindented marker (see `build_display_line_in_context` for the
    // version that *does* know).
    let line = "  - indented item";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  indented item");
    assert_eq!(dl.indent, LineIndent::List(1));
    assert_contiguous(&dl, line.len());
}

#[test]
fn classify_block_context_resolves_nested_list_depth_by_indentation() {
    let source = "1. outer\n   - inner\n   - inner sibling\n2. outer sibling";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(1),
        ]
    );
}

#[test]
fn classify_block_context_supports_three_levels_of_nesting() {
    let source = "- one\n  - two\n    - three";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(3)
        ]
    );
}

#[test]
fn classify_block_context_a_marker_at_the_same_column_is_a_sibling_not_a_dedent() {
    // The third line's indentation (2 spaces) matches the *second*
    // level's own column exactly — it's a sibling item at that same
    // depth, not a dedent back to the first level.
    let source = "- one\n  - two\n  - two's sibling";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(2)
        ]
    );
}

#[test]
fn list_node_chains_with_the_same_indents_compare_equal_even_when_separately_allocated() {
    // Two independently-built chains representing the same logical
    // nesting (0, then 2) must compare equal regardless of being
    // different `Rc` allocations — `doc/`'s reconvergence
    // check relies on exactly this (see `ListNode`'s own doc comment):
    // a freshly-walked chain reconverging with a cached one needs
    // value equality, not pointer identity, the same way two
    // `Vec<usize>`s with identical contents already compared equal.
    let a = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2);
    let b = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2);
    assert!(
        !Rc::ptr_eq(&a, &b),
        "sanity check: these really are two separate allocations"
    );
    assert_eq!(a, b);
    assert_eq!(OpenBlock::List(a), OpenBlock::List(b));

    let c = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 3); // different inner indent
    assert_ne!(
        ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2),
        c
    );
}

#[test]
fn classify_block_context_pops_a_level_whose_indentation_is_no_longer_matched() {
    let source = "- one\n  - two\n- one sibling (back to depth 1)";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(1),
        ]
    );
}

#[test]
fn classify_block_context_resolves_lazy_continuation_of_a_blockquote() {
    let source = "> first line\nsecond line, no \">\" of its own";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::Plain,
            LineContext::LazyContinuation(LineIndent::Blockquote(1))
        ]
    );
}

#[test]
fn classify_block_context_resolves_lazy_continuation_of_a_list_item() {
    let source = "- first line\nsecond line, no marker, not even indented";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::LazyContinuation(LineIndent::List(1))
        ]
    );
}

#[test]
fn classify_block_context_multiple_consecutive_lazy_lines_all_continue() {
    let source = "> quote\nsecond\nthird";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::Plain,
            LineContext::LazyContinuation(LineIndent::Blockquote(1)),
            LineContext::LazyContinuation(LineIndent::Blockquote(1)),
        ]
    );
}

#[test]
fn classify_block_context_a_blank_line_ends_lazy_continuation() {
    let source = "> quote\n\nnot part of the quote";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![LineContext::Plain, LineContext::Plain, LineContext::Plain]
    );
}

#[test]
fn classify_block_context_a_heading_ends_an_open_list_or_quote() {
    let source = "- item\n# Heading\nnot part of the list";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::Plain,
            LineContext::Plain
        ]
    );
}

#[test]
fn classify_block_context_a_fresh_blockquote_resets_an_open_list() {
    let source = "- item\n> quote\ncontinues the quote, not the list";
    let ctx = classify_block_context(source);
    assert_eq!(
        ctx,
        vec![
            LineContext::ListMarkerLine(1),
            LineContext::Plain,
            LineContext::LazyContinuation(LineIndent::Blockquote(1))
        ]
    );
}

#[test]
fn build_display_line_in_context_overrides_depth_for_a_nested_marker() {
    let line = "  - nested item";
    let dl = build_display_line_in_context(line, false, LineContext::ListMarkerLine(2));
    assert_eq!(dl.display_text, "  nested item");
    assert_eq!(dl.indent, LineIndent::List(2));
}

#[test]
fn build_display_line_in_context_styles_a_lazy_blockquote_continuation() {
    let line = "no marker here, but **bold** still works";
    let dl = build_display_line_in_context(
        line,
        false,
        LineContext::LazyContinuation(LineIndent::Blockquote(2)),
    );
    assert_eq!(dl.display_text, "no marker here, but bold still works");
    assert_eq!(dl.indent, LineIndent::Blockquote(2));
    assert_eq!(
        visible_styles(&dl),
        vec![
            (
                "no marker here, but ".to_string(),
                InlineStyle::Blockquote(2)
            ),
            ("bold".to_string(), InlineStyle::Bold),
            (" still works".to_string(), InlineStyle::Blockquote(2)),
        ]
    );
}

#[test]
fn build_display_line_in_context_styles_a_lazy_list_continuation_as_plain() {
    let line = "continues the list item";
    let dl = build_display_line_in_context(
        line,
        false,
        LineContext::LazyContinuation(LineIndent::List(1)),
    );
    assert_eq!(dl.display_text, line);
    assert_eq!(dl.indent, LineIndent::List(1));
    assert_eq!(
        visible_styles(&dl),
        vec![(line.to_string(), InlineStyle::Plain)]
    );
}

#[test]
fn build_display_line_in_context_shows_a_lazy_continuation_raw_when_active() {
    let line = "being edited right now";
    let dl = build_display_line_in_context(
        line,
        true,
        LineContext::LazyContinuation(LineIndent::Blockquote(1)),
    );
    assert_eq!(dl.display_text, line);
    assert_eq!(
        dl.indent,
        LineIndent::Blockquote(1),
        "indent still applies on the active line, same as everywhere else"
    );
}

#[test]
fn display_and_raw_offsets_round_trip_around_a_replaced_list_marker() {
    let line = "- item"; // raw: "-"(1) " "(1) "item"(4), marker = raw 0..2
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "  item");

    // Raw offsets strictly inside the 2-byte marker collapse to display
    // 0, where the replacement bullet begins.
    assert_eq!(dl.display_offset_for_raw(0), 0);
    assert_eq!(dl.display_offset_for_raw(1), 0);
    // Raw offset 2 (right after the marker, start of "item") lands right
    // after the widget's two blank columns, not inside them.
    assert_eq!(dl.display_offset_for_raw(2), 2);

    // Clicking anywhere on the bullet's display range (0..2) lands right
    // at the start of the actual item text, raw byte 2.
    assert_eq!(dl.raw_offset_for_display(0), 2);
    assert_eq!(dl.raw_offset_for_display(1), 2);
}

#[test]
fn build_table_rule_is_entirely_hidden() {
    // The delimiter row has no cell content of its own to show at all
    // — `element.rs` paints the header/body divider as a vector quad
    // over this (otherwise blank) line instead of a dashed-rule glyph.
    let line = "| --- | --- |";
    let dl = hidden_line(line);
    assert_eq!(dl.display_text, "");
    assert_contiguous(&dl, line.len());
    assert_eq!(dl.indent, LineIndent::None);
}

#[test]
fn a_cell_line_styles_inline_markup_and_never_reads_a_block() {
    let dl = build_cell_line("**bold** and `code`");
    assert_eq!(dl.display_text, "bold and code");
    assert_eq!(dl.indent, LineIndent::None);
    assert_contiguous(&dl, "**bold** and `code`".len());
    // A leading `-`, `>` or `#` is text inside a cell, not a list, quote or heading.
    for text in ["- item", "> quote", "# not a heading"] {
        let dl = build_cell_line(text);
        assert_eq!(dl.display_text, text);
        assert_eq!(dl.indent, LineIndent::None);
    }
    assert_eq!(build_cell_line(r"a \| b").display_text, "a | b");
}

#[test]
fn backslash_escapes_a_delimiter_and_hides_the_backslash() {
    let line = r"a \*literal\* asterisk";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "a *literal* asterisk");
    assert_contiguous(&dl, line.len());
}

#[test]
fn an_escaped_delimiter_does_not_falsely_close_emphasis() {
    // The real closer is the final `*`, not the escaped `\*` in the
    // middle — `find_closing` must skip past it. The emphasis content
    // itself isn't recursively re-scanned for escapes (no construct's
    // content is, in this module — the same simplification that also
    // keeps bold/italic from compounding with each other), so the
    // backslash stays visible verbatim inside it; that's expected, not
    // a bug.
    let line = r"*italic \* still italic*";
    let dl = build_display_line(line, false);
    assert_eq!(
        visible_styles(&dl),
        vec![(r"italic \* still italic".to_string(), InlineStyle::Italic)]
    );
}

#[test]
fn named_entity_decodes_to_its_character() {
    let line = "Bob &amp; Alice";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "Bob & Alice");
    assert_contiguous(&dl, line.len());
}

#[test]
fn numeric_and_hex_entities_decode() {
    assert_eq!(build_display_line("&#39;", false).display_text, "'");
    assert_eq!(build_display_line("&#x27;", false).display_text, "'");
}

#[test]
fn unknown_entity_falls_back_to_plain_text() {
    let line = "a & b &notreal; c";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
}

#[test]
fn thematic_break_is_recognized_in_its_three_forms() {
    assert!(is_thematic_break("---"));
    assert!(is_thematic_break("***"));
    assert!(is_thematic_break("___"));
    assert!(is_thematic_break("- - -"));
}

#[test]
fn thematic_break_does_not_match_a_list_item_or_short_run() {
    assert!(!is_thematic_break("- item"));
    assert!(!is_thematic_break("--"));
    assert!(!is_thematic_break("- -* _"));
}

#[test]
fn inline_link_hides_the_url_and_shows_the_text() {
    let line = "see [github](https://github.com) now";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "see github now");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("see ".to_string(), InlineStyle::Plain),
            ("github".to_string(), InlineStyle::Link),
            (" now".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn unterminated_link_falls_back_to_plain_text() {
    let line = "[no closing paren](https://x";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
    assert_contiguous(&dl, line.len());
}

#[test]
fn image_syntax_is_left_untouched() {
    // Images are explicitly deferred (no bitmap-paint pipeline) — the
    // leading `!` must suppress the link match entirely, not render
    // "alt" as a styled link.
    let line = "![alt](https://example.com/x.png)";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
}

#[test]
fn bracketed_autolink_shows_the_url_and_hides_the_angle_brackets() {
    let line = "see <https://example.com> now";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, "see https://example.com now");
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("see ".to_string(), InlineStyle::Plain),
            ("https://example.com".to_string(), InlineStyle::Link),
            (" now".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn bracketed_email_autolink_is_recognized() {
    let dl = build_display_line("<a@b.com>", false);
    assert_eq!(dl.display_text, "a@b.com");
    assert_eq!(
        visible_styles(&dl),
        vec![("a@b.com".to_string(), InlineStyle::Link)]
    );
}

#[test]
fn angle_brackets_around_ordinary_text_are_left_untouched() {
    // Raw HTML (`<div>`) is explicitly declined — this must not be
    // mistaken for an autolink.
    let line = "a <div> tag";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
}

#[test]
fn bare_url_autolinks_without_any_delimiter() {
    let line = "visit https://example.com/page today";
    let dl = build_display_line(line, false);
    assert_eq!(dl.display_text, line);
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("visit ".to_string(), InlineStyle::Plain),
            ("https://example.com/page".to_string(), InlineStyle::Link),
            (" today".to_string(), InlineStyle::Plain),
        ]
    );
    assert_contiguous(&dl, line.len());
}

#[test]
fn bare_url_trims_one_trailing_punctuation_character() {
    let line = "see https://example.com.";
    let dl = build_display_line(line, false);
    assert_eq!(
        visible_styles(&dl),
        vec![
            ("see ".to_string(), InlineStyle::Plain),
            ("https://example.com".to_string(), InlineStyle::Link),
            (".".to_string(), InlineStyle::Plain),
        ]
    );
}

#[test]
fn task_checkbox_renders_unchecked_and_checked_widgets() {
    let checkbox = |line| {
        let dl = build_display_line(line, false);
        let first = visible_styles(&dl).remove(0);
        (dl.display_text, first)
    };
    let blank = |checked| {
        (
            "  ".to_string(),
            InlineStyle::Widget(Widget::Checkbox { checked }),
        )
    };
    assert_eq!(checkbox("- [ ] todo"), ("  todo".to_string(), blank(false)));
    assert_eq!(checkbox("- [x] done"), ("  done".to_string(), blank(true)));
    assert_eq!(checkbox("- [X] done"), ("  done".to_string(), blank(true)));
}

#[test]
fn task_checkbox_works_on_an_ordered_marker_too() {
    let dl = build_display_line("1. [ ] todo", false);
    assert_eq!(dl.display_text, "1.   todo");
}

#[test]
fn task_checkbox_works_inside_a_compounding_blockquote() {
    let dl = build_display_line("> - [ ] todo", false);
    assert_eq!(dl.display_text, "  todo");
}

#[test]
fn a_list_item_without_a_checkbox_is_unaffected() {
    let dl = build_display_line("- [not a checkbox] text", false);
    assert_eq!(dl.display_text, "  [not a checkbox] text");
}

#[test]
fn callout_header_active_line_is_raw() {
    let line = "> [!info] Custom Title";
    let dl = build_callout_header_line(line, true);
    assert_eq!(dl.display_text, line);
}

#[test]
fn callout_header_without_a_title_shows_the_capitalized_type_name() {
    let dl = build_callout_header_line("> [!warning]", false);
    assert_eq!(dl.display_text, "  Warning");
    assert_eq!(
        visible_styles(&dl),
        vec![
            (
                "  ".to_string(),
                InlineStyle::Widget(Widget::CalloutIcon(CalloutType::Warning))
            ),
            ("Warning".to_string(), InlineStyle::CalloutTitle),
        ]
    );
}

#[test]
fn callout_header_with_a_custom_title_shows_it_instead() {
    let dl = build_callout_header_line("> [!info] Custom Title", false);
    assert_eq!(dl.display_text, "  Custom Title");
}

#[test]
fn callout_header_hides_the_fold_marker() {
    let dl = build_callout_header_line("> [!info]+ Title", false);
    assert_eq!(dl.display_text, "  Title");
}

#[test]
fn callout_header_title_can_itself_contain_bold() {
    let dl = build_callout_header_line("> [!info] a **bold** title", false);
    assert_eq!(dl.display_text, "  a bold title");
    assert_eq!(
        visible_styles(&dl),
        vec![
            (
                "  ".to_string(),
                InlineStyle::Widget(Widget::CalloutIcon(CalloutType::Info))
            ),
            ("a ".to_string(), InlineStyle::CalloutTitle),
            ("bold".to_string(), InlineStyle::Bold),
            (" title".to_string(), InlineStyle::CalloutTitle),
        ]
    );
}

#[test]
fn callout_header_falls_back_to_plain_blockquote_rendering_when_not_actually_a_header() {
    let dl = build_callout_header_line("> just a regular quote", false);
    assert_eq!(dl.display_text, "just a regular quote");
    assert_eq!(
        visible_styles(&dl),
        vec![(
            "just a regular quote".to_string(),
            InlineStyle::Blockquote(1)
        )]
    );
}

#[test]
fn a_display_line_splits_into_two_that_read_as_lines_of_their_own() {
    let line = "plain **bold** and a [[Note]] link";
    let dl = build_display_line(line, false);
    for at in 0..=dl.display_text.len() {
        if !dl.display_text.is_char_boundary(at) {
            continue;
        }
        let (a, b, raw_cut) = dl.split_at(at);
        assert_eq!(
            format!("{}{}", a.display_text, b.display_text),
            dl.display_text
        );
        // Every raw offset on one side maps to the same display byte as in
        // the whole line (shifted for the second part).
        let cut = a.display_text.len();
        for raw in 0..=line.len() {
            let whole = dl.display_offset_for_raw(raw);
            if raw < raw_cut {
                assert!(a.display_offset_for_raw(raw) <= cut, "{at}/{raw}");
            } else if whole >= cut {
                assert_eq!(
                    b.display_offset_for_raw(raw - raw_cut) + cut,
                    whole,
                    "{at}/{raw}"
                );
            }
        }
        // The parts tile the raw line.
        let raw_end = |d: &DisplayLine| d.segments.last().map_or(0, |s| s.raw.end);
        assert_eq!(raw_end(&b) + raw_cut, line.len(), "at {at}");
    }
}

#[test]
fn a_replaced_segment_is_never_cut() {
    let dl = build_display_line_in_context("- item text", false, LineContext::Plain);
    let glyph_end = dl
        .segments
        .iter()
        .find(|s| s.is_replaced())
        .map(|s| s.display.end)
        .unwrap();
    let (a, _, _) = dl.split_at(glyph_end - 1);
    assert!(a.display_text.is_empty(), "the cut moved before the glyph");
}

#[cfg(test)]
mod is_blank_tests {
    use super::is_blank;

    #[test]
    fn matches_trim_is_empty_for_ascii_and_unicode_whitespace() {
        let lines = [
            "",
            " ",
            "  \t ",
            "\t",
            "\r",
            "x",
            " x",
            "# h",
            ">",
            "-",
            "\u{7f}",
            "\u{a0}",
            "\u{a0}x",
            "\u{3000}",
            "\u{3000} x",
            "\u{85}",
            "\u{2003}\u{2003}",
            "é",
            "日本",
        ];
        for line in lines {
            assert_eq!(is_blank(line), line.trim().is_empty(), "{line:?}");
        }
    }
}

#[test]
fn a_list_item_leaves_the_item_gap_below_and_other_lines_none() {
    let mut t = crate::settings::Typography::default();
    t.list.item_gap = 2.0;
    crate::settings::set_body(&t);
    let rows = |line: &str| build_display_line(line, false).space_rows();
    assert_eq!(rows("- item"), (0, 2));
    assert_eq!(rows("3. item"), (0, 2));
    assert_eq!(rows("- [x] task"), (0, 2));
    assert_eq!(rows("plain text"), (0, 0));
    assert_eq!(rows("> - quoted"), (0, 0), "a quote is not a list level");
    crate::settings::set_body(&crate::settings::Typography::default());
    assert_eq!(rows("- item"), (0, 0), "today's look");
}
