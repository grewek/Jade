//! User macros of a note: `\newcommand`, `\renewcommand`, `\providecommand`,
//! `\def` and `\DeclareMathOperator`, with `#1`..`#9` arguments (#14).
//!
//! A formula's definitions are cut out of its source and its macro uses are
//! replaced by their bodies as text before it is parsed, so the parser, the
//! layout and the painter never know about macros. Expansion is bounded in
//! depth and in size, so a macro that calls itself is an error, not a hang.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use super::lex::{Kind, Lexer};
use super::parse::MAX_SOURCE;
use super::symbols;

/// Macro calls nested deeper than this (a macro whose body uses a macro…) are
/// an error. Real macro sets nest a handful deep.
pub const MAX_EXPANSION_DEPTH: usize = 32;

/// Most macros a note may define; later ones are ignored.
pub const MAX_MACROS: usize = 256;

/// Expansion went too deep or grew past [`MAX_SOURCE`].
#[derive(Debug, PartialEq, Eq)]
pub struct TooDeep;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Macro {
    params: u8,
    /// `\newcommand{\f}[2][x]{…}`: what `#1` is when no `[…]` follows.
    default: Option<String>,
    body: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum How {
    /// `\newcommand`, `\renewcommand`, `\def`, `\DeclareMathOperator`.
    Define,
    /// `\providecommand`: only if the name is free.
    Provide,
}

struct Definition {
    name: String,
    mac: Macro,
    how: How,
}

/// The macros a note defines, in document order (a later definition of a name
/// replaces an earlier one, for the whole note).
#[derive(Debug, Clone, Default)]
pub struct Macros {
    defs: BTreeMap<String, Macro>,
    /// Identifies the set in the formula cache; 0 for none.
    stamp: u64,
}

/// No macros, for code that has no note.
pub static NO_MACROS: Macros = Macros::new();

impl Macros {
    pub const fn new() -> Macros {
        Macros {
            defs: BTreeMap::new(),
            stamp: 0,
        }
    }

    /// The macros defined by the formula sources `formulas`, in order.
    pub fn collect<'a>(formulas: impl IntoIterator<Item = &'a str>) -> Macros {
        let mut defs = BTreeMap::new();
        for src in formulas {
            if !may_define(src) {
                continue;
            }
            for d in definitions(src).1 {
                let free = !defs.contains_key(&d.name);
                if (d.how == How::Define || free) && (defs.len() < MAX_MACROS || !free) {
                    defs.insert(d.name, d.mac);
                }
            }
        }
        let mut hasher = DefaultHasher::new();
        defs.hash(&mut hasher);
        let stamp = if defs.is_empty() {
            0
        } else {
            hasher.finish() | 1
        };
        Macros { defs, stamp }
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// A number that differs between different sets (equal sets may differ
    /// too: it only has to keep a cache honest).
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// `src` without its definitions and with every macro use replaced by its
    /// body. An unknown command is left for the parser to report.
    pub fn expand<'s>(&self, src: &'s str) -> Result<Cow<'s, str>, TooDeep> {
        let stripped = if may_define(src) {
            match definitions(src) {
                (Cow::Borrowed(_), _) => Cow::Borrowed(src),
                (Cow::Owned(rest), _) => Cow::Owned(rest),
            }
        } else {
            Cow::Borrowed(src)
        };
        if self.defs.is_empty() {
            return Ok(stripped);
        }
        match self.expand_text(&stripped, 0)? {
            Cow::Borrowed(_) => Ok(stripped),
            Cow::Owned(text) => Ok(Cow::Owned(text)),
        }
    }

    fn expand_text<'s>(&self, src: &'s str, depth: usize) -> Result<Cow<'s, str>, TooDeep> {
        if depth > MAX_EXPANSION_DEPTH || !src.contains('\\') {
            return if depth > MAX_EXPANSION_DEPTH {
                Err(TooDeep)
            } else {
                Ok(Cow::Borrowed(src))
            };
        }
        let mut lex = Lexer::new(src);
        let mut out = String::new();
        let mut copied = 0;
        while let Some(tok) = lex.next() {
            let Kind::Cmd(name) = tok.kind else {
                continue;
            };
            if symbols::text_font(name).is_some() {
                lex.raw_group();
                continue;
            }
            let Some(mac) = self.defs.get(name) else {
                continue;
            };
            let Some(args) = read_args(&mut lex, mac) else {
                continue;
            };
            out.push_str(&src[copied..tok.start]);
            let body = substitute(&mac.body, &args);
            out.push(' ');
            out.push_str(&self.expand_text(&body, depth + 1)?);
            out.push(' ');
            copied = lex.pos();
            if out.len() > MAX_SOURCE {
                return Err(TooDeep);
            }
        }
        if copied == 0 {
            return Ok(Cow::Borrowed(src));
        }
        out.push_str(&src[copied..]);
        Ok(Cow::Owned(out))
    }
}

/// Where in `text` a definition command begins (a backslash). One pass that
/// stops only at backslashes, so a note without any costs a search, not five.
pub fn definition_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    const COMMANDS: [&str; 5] = [
        "newcommand",
        "renewcommand",
        "providecommand",
        "DeclareMathOperator",
        "def",
    ];
    text.match_indices('\\')
        .filter(|(i, _)| COMMANDS.iter().any(|c| text[i + 1..].starts_with(c)))
        .map(|(i, _)| i)
}

/// Whether `src` could hold a definition; most formulas cannot.
pub fn may_define(src: &str) -> bool {
    definition_starts(src).next().is_some()
}

/// A macro's arguments, `None` when too few follow it.
fn read_args<'a>(lex: &mut Lexer<'a>, mac: &Macro) -> Option<Vec<Cow<'a, str>>> {
    let mut args = Vec::with_capacity(usize::from(mac.params));
    for i in 0..mac.params {
        if i == 0
            && let Some(default) = &mac.default
        {
            let given = lex.raw_bracket();
            args.push(match given {
                Some(text) => Cow::Borrowed(text),
                None => Cow::Owned(default.clone()),
            });
            continue;
        }
        args.push(Cow::Borrowed(read_argument(lex)?));
    }
    Some(args)
}

/// One argument as TeX reads it: a `{…}` group's contents or one token.
fn read_argument<'a>(lex: &mut Lexer<'a>) -> Option<&'a str> {
    if lex.peek()?.kind == Kind::Open {
        return match lex.raw_group()? {
            (text, true) => Some(text),
            _ => None,
        };
    }
    let tok = lex.next()?;
    if tok.kind == Kind::Close {
        return None;
    }
    Some(lex.slice_from(tok.start))
}

/// `body` with `#k` replaced by argument `k` and `##` by `#`.
fn substitute(body: &str, args: &[Cow<'_, str>]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '#' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('#') => {
                chars.next();
                out.push('#');
            }
            Some(d @ '1'..='9') => {
                chars.next();
                if let Some(arg) = args.get(d as usize - '1' as usize) {
                    out.push_str(arg);
                }
            }
            _ => out.push('#'),
        }
    }
    out
}

/// The definitions in `src` and `src` without them (borrowed when there are
/// none). Only top-level ones count: inside a group they are left as written.
fn definitions(src: &str) -> (Cow<'_, str>, Vec<Definition>) {
    let mut lex = Lexer::new(src);
    let mut found = Vec::new();
    let mut out = String::new();
    let mut copied = 0;
    let mut depth = 0usize;
    while let Some(tok) = lex.next() {
        match tok.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth = depth.saturating_sub(1),
            Kind::Cmd(
                name @ ("newcommand"
                | "renewcommand"
                | "providecommand"
                | "DeclareMathOperator"
                | "def"),
            ) if depth == 0 => {
                let at = lex.clone();
                if let Some(def) = read_definition(&mut lex, name) {
                    out.push_str(&src[copied..tok.start]);
                    copied = lex.pos();
                    found.push(def);
                } else {
                    lex = at;
                }
            }
            _ => {}
        }
    }
    if found.is_empty() {
        return (Cow::Borrowed(src), found);
    }
    out.push_str(&src[copied..]);
    (Cow::Owned(out), found)
}

/// The definition after `\name`, or `None` if it is malformed (it is then
/// left in the source, where the parser reports it).
fn read_definition(lex: &mut Lexer<'_>, name: &str) -> Option<Definition> {
    if name == "def" {
        return read_def(lex);
    }
    let starred = lex.peek()?.kind == Kind::Char('*');
    if starred {
        lex.next();
    }
    let target = read_name(lex)?;
    if name == "DeclareMathOperator" {
        let (text, true) = lex.raw_group()? else {
            return None;
        };
        let star = if starred { "*" } else { "" };
        return Some(Definition {
            name: target,
            mac: Macro {
                params: 0,
                default: None,
                body: format!("\\operatorname{star}{{{text}}}"),
            },
            how: How::Define,
        });
    }
    let params = match lex.raw_bracket() {
        Some(n) => n.trim().parse::<u8>().ok().filter(|n| *n <= 9)?,
        None => 0,
    };
    let default = if params > 0 {
        lex.raw_bracket().map(str::to_owned)
    } else {
        None
    };
    let (body, true) = lex.raw_group()? else {
        return None;
    };
    Some(Definition {
        name: target,
        mac: Macro {
            params,
            default,
            body: body.to_owned(),
        },
        how: if name == "providecommand" {
            How::Provide
        } else {
            How::Define
        },
    })
}

/// `\def\name#1#2{body}`.
fn read_def(lex: &mut Lexer<'_>) -> Option<Definition> {
    let Kind::Cmd(name) = lex.next()?.kind else {
        return None;
    };
    let mut params = 0u8;
    while lex.peek()?.kind == Kind::Char('#') {
        lex.next();
        let Kind::Char(d) = lex.next()?.kind else {
            return None;
        };
        if d != char::from(b'1' + params) {
            return None;
        }
        params += 1;
    }
    let (body, true) = lex.raw_group()? else {
        return None;
    };
    Some(Definition {
        name: name.to_owned(),
        mac: Macro {
            params,
            default: None,
            body: body.to_owned(),
        },
        how: How::Define,
    })
}

/// The name in `{\R}` or `\R`, without the backslash.
fn read_name(lex: &mut Lexer<'_>) -> Option<String> {
    if lex.peek()?.kind == Kind::Open {
        let (raw, true) = lex.raw_group()? else {
            return None;
        };
        let name = raw.trim().strip_prefix('\\')?;
        return valid_name(name).then(|| name.to_owned());
    }
    match lex.next()?.kind {
        Kind::Cmd(name) if valid_name(name) => Some(name.to_owned()),
        _ => None,
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphabetic()),
        Some(_) => chars.next().is_none(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(sources: &[&str]) -> Macros {
        Macros::collect(sources.iter().copied())
    }

    fn expanded(sources: &[&str], src: &str) -> String {
        table(sources).expand(src).unwrap().into_owned()
    }

    #[test]
    fn a_formula_without_macros_is_borrowed() {
        let m = table(&[]);
        assert!(matches!(
            m.expand("x^2 + \\frac{a}{b}"),
            Ok(Cow::Borrowed(_))
        ));
        assert_eq!(m.stamp(), 0);
    }

    #[test]
    fn newcommand_without_arguments() {
        let out = expanded(&["\\newcommand{\\R}{\\mathbb{R}}"], "f: \\R \\to \\R");
        assert_eq!(
            out.split_whitespace().collect::<String>(),
            "f:\\mathbb{R}\\to\\mathbb{R}"
        );
    }

    #[test]
    fn arguments_are_substituted() {
        let defs = ["\\newcommand{\\norm}[1]{\\left\\lVert #1 \\right\\rVert}"];
        let out = expanded(&defs, "\\norm{x}");
        assert!(out.contains("\\left\\lVert x \\right\\rVert"), "{out}");
        let two = ["\\newcommand\\pair[2]{(#1, #2)}"];
        let out = expanded(&two, "\\pair{a}{b} \\pair xy");
        assert!(out.contains("(a, b)") && out.contains("(x, y)"), "{out}");
    }

    #[test]
    fn an_optional_first_argument_has_a_default() {
        let defs = ["\\newcommand{\\f}[2][n]{#1 + #2}"];
        assert!(expanded(&defs, "\\f{a}").contains("n + a"));
        assert!(expanded(&defs, "\\f[m]{a}").contains("m + a"));
    }

    #[test]
    fn def_and_declare_math_operator() {
        assert!(expanded(&["\\def\\pr#1#2{#1 #2}"], "\\pr ab").contains("a b"));
        let out = expanded(&["\\DeclareMathOperator{\\Tr}{Tr}"], "\\Tr A");
        assert!(out.contains("\\operatorname{Tr}"), "{out}");
        let out = expanded(
            &["\\DeclareMathOperator*{\\argmax}{arg\\,max}"],
            "\\argmax_x",
        );
        assert!(out.contains("\\operatorname*{arg\\,max}"), "{out}");
    }

    #[test]
    fn definitions_are_cut_out_of_their_formula() {
        let src = "\\newcommand{\\R}{\\mathbb{R}}";
        assert_eq!(expanded(&[src], src).trim(), "");
        let mixed = "\\newcommand{\\a}{x} \\a + 1";
        let out = expanded(&[mixed], mixed);
        assert_eq!(out.split_whitespace().collect::<String>(), "x+1");
    }

    #[test]
    fn a_later_definition_replaces_an_earlier_one_and_provide_does_not() {
        let m = table(&[
            "\\newcommand{\\a}{1}",
            "\\providecommand{\\a}{2}",
            "\\providecommand{\\b}{3}",
            "\\renewcommand{\\a}{4}",
        ]);
        assert_eq!(
            m.expand("\\a\\b")
                .unwrap()
                .split_whitespace()
                .collect::<String>(),
            "43"
        );
    }

    #[test]
    fn macros_use_macros() {
        let m = table(&["\\newcommand{\\a}{\\b\\b}", "\\newcommand{\\b}{x}"]);
        let out = m.expand("\\a").unwrap();
        assert_eq!(out.split_whitespace().collect::<String>(), "xx");
    }

    #[test]
    fn a_macro_that_calls_itself_is_an_error() {
        let m = table(&["\\newcommand{\\a}{\\a}"]);
        assert_eq!(m.expand("\\a"), Err(TooDeep));
        let blow = table(&["\\newcommand{\\a}{\\b\\b}", "\\newcommand{\\b}{\\a\\a}"]);
        assert_eq!(blow.expand("\\a"), Err(TooDeep));
    }

    #[test]
    fn a_macro_with_missing_arguments_is_left_for_the_parser() {
        let m = table(&["\\newcommand{\\a}[2]{#1#2}"]);
        assert!(m.expand("\\a{x}").unwrap().contains("\\a{x}"));
    }

    #[test]
    fn text_and_unknown_commands_are_left_alone() {
        let m = table(&["\\newcommand{\\R}{X}"]);
        assert_eq!(
            m.expand("\\text{\\R} \\alpha").unwrap(),
            "\\text{\\R} \\alpha"
        );
    }

    #[test]
    fn a_malformed_definition_stays_in_the_source() {
        let src = "\\newcommand{x}{y}";
        let m = table(&[src]);
        assert!(m.is_empty());
        assert!(m.expand(src).unwrap().contains("\\newcommand"));
    }

    #[test]
    fn the_stamp_follows_the_set() {
        let a = table(&["\\newcommand{\\a}{1}"]);
        let b = table(&["\\newcommand{\\a}{2}"]);
        assert_ne!(a.stamp(), 0);
        assert_ne!(a.stamp(), b.stamp());
        assert_eq!(a.stamp(), table(&["\\newcommand{\\a}{1}"]).stamp());
    }

    #[test]
    fn the_number_of_macros_is_bounded() {
        let many: Vec<String> = (0..MAX_MACROS + 50)
            .map(|i| {
                format!(
                    "\\newcommand{{\\m{}}}{{x}}",
                    ["a", "b", "c"][i % 3].repeat(i / 3 + 1)
                )
            })
            .collect();
        let m = Macros::collect(many.iter().map(String::as_str));
        assert!(m.defs.len() <= MAX_MACROS);
    }
}
