use super::*;
use crate::text_editor::test_support::Rng;

fn nodes(src: &str) -> Vec<Node> {
    let f = parse(src);
    assert!(!f.has_error, "{src:?} should parse clean: {:?}", f.nodes);
    f.nodes
}

fn sym(ch: char, class: Class) -> Node {
    Node::Symbol { ch, class }
}

fn ord(ch: char) -> Node {
    sym(ch, Class::Ord)
}

fn group(nodes: Vec<Node>) -> Node {
    Node::Group(nodes)
}

fn script(base: Node, sub: Option<Node>, sup: Option<Node>) -> Node {
    Node::Script {
        base: Box::new(base),
        sub: sub.map(Box::new),
        sup: sup.map(Box::new),
    }
}

fn env(src: &str) -> Environment {
    match nodes(src).remove(0) {
        Node::Env(e) => *e,
        other => panic!("not an environment: {other:?}"),
    }
}

#[test]
fn letters_digits_and_operators_are_atoms_with_classes() {
    assert_eq!(
        nodes("x+1=y"),
        [
            ord('x'),
            sym('+', Class::Bin),
            ord('1'),
            sym('=', Class::Rel),
            ord('y')
        ]
    );
    // ASCII hyphen and star become the real minus and asterisk glyphs.
    assert_eq!(
        nodes("a-b*c"),
        [
            ord('a'),
            sym('\u{2212}', Class::Bin),
            ord('b'),
            sym('\u{2217}', Class::Bin),
            ord('c')
        ]
    );
}

#[test]
fn whitespace_and_comments_are_ignored() {
    assert_eq!(nodes(" x % a comment\n  y "), [ord('x'), ord('y')]);
}

#[test]
fn scripts_attach_in_either_order() {
    let both = script(ord('x'), Some(ord('i')), Some(ord('2')));
    assert_eq!(nodes("x_i^2")[0], both);
    assert_eq!(nodes("x^2_i")[0], both);
    // One token, or a group; the next character is not part of it.
    assert_eq!(
        nodes("x^23"),
        [script(ord('x'), None, Some(ord('2'))), ord('3')]
    );
    assert_eq!(
        nodes("x^{23}"),
        [script(
            ord('x'),
            None,
            Some(group(vec![ord('2'), ord('3')]))
        )]
    );
}

#[test]
fn primes_are_a_superscript() {
    let prime = ord('′');
    assert_eq!(nodes("f'"), [script(ord('f'), None, Some(prime.clone()))]);
    assert_eq!(
        nodes("f''"),
        [script(
            ord('f'),
            None,
            Some(group(vec![prime.clone(), prime.clone()]))
        )]
    );
    // A prime and an explicit power share one superscript.
    assert_eq!(
        nodes("f'^2"),
        [script(ord('f'), None, Some(group(vec![prime, ord('2')])))]
    );
}

#[test]
fn a_script_with_nothing_before_it_scripts_an_empty_group() {
    assert_eq!(nodes("^2"), [script(group(vec![]), None, Some(ord('2')))]);
}

#[test]
fn a_second_superscript_is_an_error() {
    assert!(parse("x^a^b").has_error);
    assert!(parse("x_a_b").has_error);
}

#[test]
fn fractions_take_a_group_or_one_token_each() {
    let frac = |num, den| Node::Frac {
        num: Box::new(num),
        den: Box::new(den),
        kind: FracKind::Fraction,
        style: None,
    };
    assert_eq!(nodes("\\frac12"), [frac(ord('1'), ord('2'))]);
    assert_eq!(
        nodes("\\frac{a+b}{c}"),
        [frac(
            group(vec![ord('a'), sym('+', Class::Bin), ord('b')]),
            group(vec![ord('c')])
        )]
    );
    let Node::Frac { style, .. } = &nodes("\\dfrac12")[0] else {
        panic!()
    };
    assert_eq!(*style, Some(MathStyle::Display));
    let Node::Frac { kind, .. } = &nodes("\\binom{n}{k}")[0] else {
        panic!()
    };
    assert_eq!(*kind, FracKind::Binom);
}

#[test]
fn infix_over_splits_its_group() {
    assert_eq!(
        nodes("{a \\over b}"),
        [group(vec![Node::Frac {
            num: Box::new(group(vec![ord('a')])),
            den: Box::new(group(vec![ord('b')])),
            kind: FracKind::Fraction,
            style: None,
        }])]
    );
}

#[test]
fn roots_have_an_optional_index() {
    assert_eq!(
        nodes("\\sqrt{x}"),
        [Node::Sqrt {
            index: None,
            body: Box::new(group(vec![ord('x')]))
        }]
    );
    let Node::Sqrt { index, .. } = &nodes("\\sqrt[3]{x}")[0] else {
        panic!()
    };
    assert_eq!(index.as_deref(), Some(&group(vec![ord('3')])));
}

#[test]
fn left_right_and_middle_make_a_delimited_run() {
    assert_eq!(
        nodes("\\left( x \\middle| y \\right)"),
        [Node::Delimited {
            open: Some('('),
            close: Some(')'),
            body: vec![ord('x'), Node::Middle(Some('|')), ord('y')],
        }]
    );
    let Node::Delimited { open, close, .. } = &nodes("\\left. x \\right\\rangle")[0] else {
        panic!()
    };
    assert_eq!((*open, *close), (None, Some('⟩')));
    let Node::Delimited { open, .. } = &nodes("\\left\\{ x \\right.")[0] else {
        panic!()
    };
    assert_eq!(*open, Some('{'));
}

#[test]
fn big_operators_know_where_their_scripts_go() {
    let Node::Script { base, .. } = &nodes("\\sum_{i=0}^n")[0] else {
        panic!()
    };
    assert_eq!(
        **base,
        Node::Op {
            name: OpName::Glyph('∑'),
            limits: Limits::Display
        }
    );
    let Node::Script { base, .. } = &nodes("\\int_0^1")[0] else {
        panic!()
    };
    assert!(matches!(
        **base,
        Node::Op {
            limits: Limits::Never,
            ..
        }
    ));
    // `\limits` and `\nolimits` override, and are not left behind as atoms.
    let parsed = nodes("\\int\\limits_0^1 \\sum\\nolimits_i");
    assert_eq!(parsed.len(), 2);
    assert!(matches!(
        &parsed[0],
        Node::Script { base, .. } if matches!(**base, Node::Op { limits: Limits::Always, .. })
    ));
    assert!(matches!(
        &parsed[1],
        Node::Script { base, .. } if matches!(**base, Node::Op { limits: Limits::Never, .. })
    ));
}

#[test]
fn named_operators_are_upright_words() {
    let Node::Script { base, .. } = &nodes("\\lim_{x \\to 0}")[0] else {
        panic!()
    };
    assert_eq!(
        **base,
        Node::Op {
            name: OpName::Text("lim".into()),
            limits: Limits::Display
        }
    );
    assert_eq!(
        nodes("\\operatorname{argmax}"),
        [Node::Op {
            name: OpName::Text("argmax".into()),
            limits: Limits::Never
        }]
    );
}

#[test]
fn text_keeps_its_spaces_and_resolves_escapes() {
    assert_eq!(
        nodes("\\text{if } x \\text{50\\% off}"),
        [
            Node::Text {
                text: "if ".into(),
                font: Font::Roman
            },
            ord('x'),
            Node::Text {
                text: "50% off".into(),
                font: Font::Roman
            },
        ]
    );
    assert!(matches!(
        &nodes("\\textbf{hi}")[0],
        Node::Text {
            font: Font::Bold,
            ..
        }
    ));
}

#[test]
fn fonts_and_accents_wrap_their_argument() {
    assert_eq!(
        nodes("\\mathbb{R}"),
        [Node::Font {
            font: Font::Blackboard,
            body: Box::new(group(vec![ord('R')]))
        }]
    );
    assert_eq!(
        nodes("\\hat x"),
        [Node::Accent {
            kind: Accent::Hat,
            body: Box::new(ord('x'))
        }]
    );
    assert!(Accent::UnderBrace.is_under());
    assert!(!Accent::OverBrace.is_under());
}

#[test]
fn spaces_are_in_eighteenths_of_an_em() {
    assert_eq!(
        nodes("a\\,b\\;c\\!d\\quad e\\ f~g"),
        [
            ord('a'),
            Node::Space(3),
            ord('b'),
            Node::Space(5),
            ord('c'),
            Node::Space(-3),
            ord('d'),
            Node::Space(18),
            ord('e'),
            Node::Space(6),
            ord('f'),
            Node::Space(6),
            ord('g'),
        ]
    );
}

#[test]
fn escaped_characters_are_ordinary_symbols() {
    assert_eq!(
        nodes("\\{ \\} \\$ \\% \\_ \\&"),
        [
            sym('{', Class::Open),
            sym('}', Class::Close),
            ord('$'),
            ord('%'),
            ord('_'),
            ord('&')
        ]
    );
}

#[test]
fn a_matrix_is_rows_of_cells_and_a_trailing_break_is_not_a_row() {
    let m = env("\\begin{pmatrix} a & b \\\\ c & d \\\\ \\end{pmatrix}");
    assert_eq!(m.kind, Env::Matrix(Fence::Paren));
    assert_eq!(
        m.rows,
        vec![
            vec![vec![ord('a')], vec![ord('b')]],
            vec![vec![ord('c')], vec![ord('d')]],
        ]
    );
    assert!(m.hlines.is_empty());
}

#[test]
fn every_matrix_fence_is_recognised() {
    for (name, fence) in [
        ("matrix", Fence::None),
        ("pmatrix", Fence::Paren),
        ("bmatrix", Fence::Bracket),
        ("Bmatrix", Fence::Brace),
        ("vmatrix", Fence::Bar),
        ("Vmatrix", Fence::DoubleBar),
    ] {
        let m = env(&format!("\\begin{{{name}}} a \\end{{{name}}}"));
        assert_eq!(m.kind, Env::Matrix(fence), "{name}");
    }
}

#[test]
fn arrays_carry_columns_and_rules() {
    let a = env(
        "\\begin{array}{c|lr} \\hline a & b & c \\\\ \\hline d & e & f \\\\ \\hline \\end{array}",
    );
    assert_eq!(a.cols, [Col::Center, Col::Rule, Col::Left, Col::Right]);
    assert_eq!(a.hlines, [0, 1, 2]);
    assert_eq!(a.rows.len(), 2);
    assert_eq!(a.rows[0].len(), 3);
}

#[test]
fn cases_aligned_and_gathered_are_environments() {
    let c = env("\\begin{cases} x & x > 0 \\\\ 0 & \\text{else} \\end{cases}");
    assert_eq!(c.kind, Env::Cases);
    assert_eq!(c.rows.len(), 2);
    assert_eq!(
        env("\\begin{align*} a &= b \\\\ c &= d \\end{align*}").kind,
        Env::Aligned
    );
    assert_eq!(env("\\begin{aligned} a \\end{aligned}").kind, Env::Aligned);
    assert_eq!(
        env("\\begin{gather} a \\\\ b \\end{gather}").kind,
        Env::Gathered
    );
}

#[test]
fn a_matrix_cell_may_hold_anything_including_a_group_first() {
    let m = env("\\begin{matrix}{x} & \\frac12 \\end{matrix}");
    assert_eq!(m.rows[0][0], vec![group(vec![ord('x')])]);
}

#[test]
fn substack_stacks_rows() {
    let Node::Script { sub, .. } = &nodes("\\sum_{\\substack{i<n \\\\ j>m}}")[0] else {
        panic!()
    };
    let Some(Node::Group(inner)) = sub.as_deref() else {
        panic!()
    };
    let Node::Env(e) = &inner[0] else { panic!() };
    assert_eq!(e.kind, Env::Substack);
    assert_eq!(e.rows.len(), 2);
}

#[test]
fn stacking_and_extensible_arrows() {
    assert!(matches!(
        &nodes("\\overset{!}{=}")[0],
        Node::Stack {
            over: Some(_),
            under: None,
            ..
        }
    ));
    assert!(matches!(
        &nodes("\\underset{n}{\\max}")[0],
        Node::Stack {
            over: None,
            under: Some(_),
            ..
        }
    ));
    let Node::Arrow { right, under, .. } = &nodes("\\xrightarrow[b]{a}")[0] else {
        panic!()
    };
    assert!(*right && under.is_some());
}

#[test]
fn mod_and_negation_and_style_switches() {
    assert!(matches!(
        &nodes("a \\bmod b")[1],
        Node::Class {
            class: Class::Bin,
            ..
        }
    ));
    assert!(matches!(&nodes("\\pmod{n}")[0], Node::Group(g) if g.len() == 6));
    assert!(matches!(&nodes("\\not\\in")[0], Node::Negated(_)));
    assert_eq!(
        nodes("\\displaystyle x"),
        [Node::Style(MathStyle::Display), ord('x')]
    );
}

#[test]
fn a_line_break_outside_an_environment_is_a_break() {
    assert_eq!(nodes("a \\\\[2pt] b"), [ord('a'), Node::Break, ord('b')]);
}

#[test]
fn unknown_commands_are_errors_that_keep_their_source() {
    let f = parse("x + \\nosuchcommand y");
    assert!(f.has_error);
    assert_eq!(f.nodes[2], Node::Error("\\nosuchcommand".into()));
    // The rest still parsed.
    assert_eq!(f.nodes[3], ord('y'));
}

#[test]
fn unbalanced_input_is_an_error_not_a_panic() {
    for src in [
        "{",
        "}",
        "x^{",
        "\\frac{a}",
        "\\frac",
        "\\left( x",
        "\\right)",
        "\\left",
        "\\left x",
        "\\begin{matrix} a",
        "\\begin{matrix} a \\end{pmatrix}",
        "\\begin{nonsense} a \\end{nonsense}",
        "\\end{matrix}",
        "&",
        "\\sqrt[3",
        "\\text{abc",
        "\\text",
        "\\",
        "\\begin",
        "\\begin{",
        "\\substack",
        "\\substack{a",
        "^",
        "_",
        "x^",
        "\\frac{}{}",
        "\\xrightarrow",
    ] {
        let f = parse(src);
        // `^` alone and `\frac{}{}` are legal-enough; everything else flags.
        if !matches!(src, "^" | "_" | "\\frac{}{}") {
            assert!(f.has_error, "{src:?} should be flagged");
        }
    }
}

#[test]
fn an_unknown_environment_keeps_its_whole_source_as_the_error() {
    let f = parse("\\begin{nonsense} a & b \\end{nonsense} z");
    assert_eq!(
        f.nodes[0],
        Node::Error("\\begin{nonsense} a & b \\end{nonsense}".into())
    );
    assert_eq!(f.nodes[1], ord('z'));
}

#[test]
fn deep_nesting_is_bounded() {
    // None of these may overflow the stack; each is flagged past the cap.
    let braces = format!("{}x{}", "{".repeat(5_000), "}".repeat(5_000));
    let fracs = "\\frac".repeat(3_000);
    let lefts = "\\left(".repeat(2_000);
    let scripts = "x".to_owned() + &"^{a".repeat(4_000) + &"}".repeat(4_000);
    let begins = "\\begin{matrix}".repeat(1_000);
    for src in [braces, fracs, lefts, scripts, begins] {
        assert!(src.len() <= MAX_SOURCE, "{}", src.len());
        assert!(parse(&src).has_error);
    }
}

#[test]
fn nesting_under_the_cap_is_fine() {
    let src = format!(
        "{}x{}",
        "{".repeat(MAX_DEPTH - 1),
        "}".repeat(MAX_DEPTH - 1)
    );
    assert!(!parse(&src).has_error);
}

#[test]
fn an_oversized_source_is_not_parsed() {
    let f = parse(&"x".repeat(MAX_SOURCE + 1));
    assert!(f.has_error);
    assert_eq!(f.nodes, [Node::Error(String::new())]);
}

#[test]
fn random_token_soup_never_panics() {
    let alphabet = [
        "{",
        "}",
        "^",
        "_",
        "&",
        "\\\\",
        "\\frac",
        "\\sqrt",
        "[",
        "]",
        "\\left",
        "\\right",
        "\\middle",
        "(",
        ")",
        "|",
        ".",
        "\\begin{matrix}",
        "\\begin{array}{cc}",
        "\\end{matrix}",
        "\\end{array}",
        "\\text{",
        "\\sum",
        "\\limits",
        "x",
        "1",
        "'",
        "\\hline",
        "\\substack",
        "\\over",
        " ",
        "%",
        "\\",
        "é",
        "\\xrightarrow",
        "\\not",
        "\\mathbb",
        "\\hat",
    ];
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4_000 {
        let n = 1 + rng.below(40);
        let src: String = (0..n)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let _ = parse(&src);
    }
}

#[test]
fn a_formula_that_fits_the_row_needs_no_more() {
    // 24px rows, text 16px with 4px/2px of ascent/descent: the baseline is
    // 1px below the middle.
    assert_eq!(rows_to_fit((10.0, 6.0), (4.0, 2.0), 24.0), 1);
    assert_eq!(rows_to_fit((0.0, 0.0), (4.0, 2.0), 24.0), 1);
}

#[test]
fn a_taller_formula_takes_whole_extra_rows() {
    let rows = |above: f32, below: f32| rows_to_fit((above, below), (14.0, 6.0), 24.0);
    assert_eq!(rows(20.0, 10.0), 2);
    assert!(rows(60.0, 20.0) >= 4);
    // Deep below the baseline is as much a problem as high above it.
    assert!(rows(8.0, 40.0) >= 3);
}

#[test]
fn row_growth_is_bounded() {
    assert!(rows_to_fit((1e6, 1e6), (14.0, 6.0), 24.0) <= 16);
}

#[test]
fn the_rows_chosen_really_contain_the_formula() {
    let (asc, desc, lh) = (14.0f32, 6.0f32, 24.0f32);
    for above in [5.0f32, 17.0, 33.0, 50.0, 90.0] {
        for below in [2.0f32, 12.0, 30.0, 70.0] {
            let rows = rows_to_fit((above, below), (asc, desc), lh) as f32 * lh;
            let baseline = (rows - (asc + desc)) / 2.0 + asc;
            assert!(baseline >= above - 1e-2, "{above}/{below}: top clipped");
            assert!(
                rows - baseline >= below - 1e-2,
                "{above}/{below}: bottom clipped"
            );
        }
    }
}
