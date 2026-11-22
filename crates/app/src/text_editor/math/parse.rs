//! LaTeX math source to [`Node`]s. Total: any input gives a tree, with the
//! pieces that do not parse kept as [`Node::Error`], never a panic. Every loop
//! consumes a token and every recursion passes `parse_atom`, which is capped by
//! [`MAX_DEPTH`], so hostile nesting costs time linear in the source.

use super::ast::{Class, Env, Environment, FracKind, Limits, MathStyle, Node, OpName};
use super::lex::{Kind, Lexer, Token};
use super::symbols;

/// Nesting (groups, arguments, environments) past which the rest is an error.
/// Real formulas nest under ten deep; each level costs a few hundred bytes of
/// stack.
pub const MAX_DEPTH: usize = 48;

/// Sources longer than this are not parsed (a formula is a few hundred bytes).
pub const MAX_SOURCE: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub nodes: Vec<Node>,
    /// Whether any [`Node::Error`] is in the tree.
    pub has_error: bool,
}

/// Parses the source between the `$` delimiters.
pub fn parse(src: &str) -> Formula {
    if src.len() > MAX_SOURCE {
        return Formula {
            nodes: vec![Node::Error(String::new())],
            has_error: true,
        };
    }
    let mut parser = Parser {
        lex: Lexer::new(src),
        depth: 0,
        has_error: false,
    };
    let nodes = parser.parse_list(Ctx::Top);
    Formula {
        nodes,
        has_error: parser.has_error,
    }
}

/// What ends the list being parsed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Top,
    /// `{ … }`.
    Brace,
    /// `\left … \right` (and `\middle`).
    Delim,
    /// An environment cell: ends at `&`, `\\` or `\end`.
    Cell,
    /// A `\substack{ … }` cell: ends at `&`, `\\` or `}`.
    BraceCell,
    /// An optional `[ … ]` argument.
    Bracket,
}

impl Ctx {
    fn stops_at(self, kind: Kind) -> bool {
        match self {
            Ctx::Top => false,
            Ctx::Brace => kind == Kind::Close,
            Ctx::Delim => matches!(kind, Kind::Cmd("right" | "middle")),
            Ctx::Cell => matches!(kind, Kind::Amp | Kind::Cmd("\\" | "end")),
            Ctx::BraceCell => matches!(kind, Kind::Amp | Kind::Close | Kind::Cmd("\\")),
            Ctx::Bracket => kind == Kind::Char(']'),
        }
    }
}

struct Parser<'a> {
    lex: Lexer<'a>,
    depth: usize,
    has_error: bool,
}

impl<'a> Parser<'a> {
    fn error(&mut self, source: &str) -> Node {
        self.has_error = true;
        Node::Error(source.to_owned())
    }

    fn error_token(&mut self, tok: Token<'a>) -> Node {
        let text = self.lex.slice_from(tok.start);
        self.error(text)
    }

    /// A sequence of atoms with their scripts, up to what `ctx` ends at (left
    /// unread). An infix `\over` or `\choose` makes everything before it the
    /// numerator and everything after the denominator.
    fn parse_list(&mut self, ctx: Ctx) -> Vec<Node> {
        let mut out = Vec::new();
        let mut infix: Option<(FracKind, usize)> = None;
        while let Some(tok) = self.lex.peek() {
            if ctx.stops_at(tok.kind) {
                break;
            }
            let atom = match tok.kind {
                // A script with nothing before it scripts an empty group.
                Kind::Sup | Kind::Sub => Node::Group(Vec::new()),
                Kind::Cmd(name @ ("over" | "choose")) if infix.is_none() => {
                    self.lex.next();
                    let kind = if name == "over" {
                        FracKind::Fraction
                    } else {
                        FracKind::Binom
                    };
                    infix = Some((kind, out.len()));
                    continue;
                }
                _ => {
                    self.lex.next();
                    self.parse_atom(tok)
                }
            };
            out.push(self.attach_scripts(atom));
        }
        match infix {
            Some((kind, at)) => {
                let den = Node::Group(out.split_off(at));
                let num = Node::Group(out);
                vec![Node::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    kind,
                    style: None,
                }]
            }
            None => out,
        }
    }

    /// `^`, `_` and `'` after `base`, in any order and number.
    fn attach_scripts(&mut self, mut base: Node) -> Node {
        if let Node::Op { limits, .. } = &mut base {
            let set = match self.lex.peek().map(|t| t.kind) {
                Some(Kind::Cmd("limits")) => Some(Limits::Always),
                Some(Kind::Cmd("nolimits")) => Some(Limits::Never),
                _ => None,
            };
            if let Some(set) = set {
                self.lex.next();
                *limits = set;
            }
        }
        let mut sub: Option<Node> = None;
        let mut sup: Vec<Node> = Vec::new();
        let (mut has_sub, mut has_sup) = (false, false);
        while let Some(tok) = self.lex.peek() {
            match tok.kind {
                Kind::Sub => {
                    self.lex.next();
                    let arg = self.parse_arg();
                    if has_sub {
                        sup.push(Node::Error("_".to_owned()));
                        self.has_error = true;
                    } else {
                        sub = Some(arg);
                        has_sub = true;
                    }
                }
                Kind::Sup => {
                    self.lex.next();
                    let arg = self.parse_arg();
                    if has_sup {
                        sup.push(Node::Error("^".to_owned()));
                        self.has_error = true;
                    }
                    sup.push(arg);
                    has_sup = true;
                }
                Kind::Char('\'') => {
                    self.lex.next();
                    sup.push(Node::Symbol {
                        ch: '′',
                        class: Class::Ord,
                    });
                }
                _ => break,
            }
        }
        if sub.is_none() && sup.is_empty() {
            return base;
        }
        let sup = match sup.len() {
            0 => None,
            1 => sup.pop().map(Box::new),
            _ => Some(Box::new(Node::Group(sup))),
        };
        Node::Script {
            base: Box::new(base),
            sub: sub.map(Box::new),
            sup,
        }
    }

    /// One command argument: a brace group or a single token. A missing one
    /// (end of input, a closer) is an empty error and the closer stays.
    fn parse_arg(&mut self) -> Node {
        match self.lex.peek() {
            Some(tok)
                if !matches!(
                    tok.kind,
                    Kind::Close
                        | Kind::Amp
                        | Kind::Sup
                        | Kind::Sub
                        | Kind::Cmd("right" | "middle" | "end" | "\\")
                ) =>
            {
                self.lex.next();
                self.parse_atom(tok)
            }
            _ => self.error(""),
        }
    }

    /// An optional `[ … ]` argument.
    fn parse_optional(&mut self) -> Option<Node> {
        if self.lex.peek()?.kind != Kind::Char('[') {
            return None;
        }
        self.lex.next();
        let mut body = self.parse_list(Ctx::Bracket);
        match self.lex.peek() {
            Some(t) if t.kind == Kind::Char(']') => {
                self.lex.next();
            }
            _ => body.push(self.error("]")),
        }
        Some(Node::Group(body))
    }

    /// The atom `tok` (already read) starts, without its scripts.
    fn parse_atom(&mut self, tok: Token<'a>) -> Node {
        if self.depth >= MAX_DEPTH {
            return self.error_token(tok);
        }
        self.depth += 1;
        let node = match tok.kind {
            Kind::Open => self.parse_group(),
            Kind::Char(c) => char_atom(c),
            Kind::Cmd(name) => self.parse_command(name, tok),
            Kind::Close | Kind::Amp | Kind::Sup | Kind::Sub => self.error_token(tok),
        };
        self.depth -= 1;
        node
    }

    fn parse_group(&mut self) -> Node {
        let mut body = self.parse_list(Ctx::Brace);
        match self.lex.peek() {
            Some(t) if t.kind == Kind::Close => {
                self.lex.next();
            }
            _ => body.push(self.error("}")),
        }
        Node::Group(body)
    }

    fn parse_command(&mut self, name: &'a str, tok: Token<'a>) -> Node {
        if let Some((ch, class)) = symbols::symbol(name) {
            return Node::Symbol { ch, class };
        }
        if let Some((ch, limits)) = symbols::big_op(name) {
            return Node::Op {
                name: OpName::Glyph(ch),
                limits,
            };
        }
        if let Some((word, limits)) = symbols::named_op(name) {
            return Node::Op {
                name: OpName::Text(word.to_owned()),
                limits,
            };
        }
        if let Some(kind) = symbols::accent(name) {
            let body = Box::new(self.parse_arg());
            return Node::Accent { kind, body };
        }
        if let Some(font) = symbols::font(name) {
            let body = Box::new(self.parse_arg());
            return Node::Font { font, body };
        }
        if let Some(mu) = symbols::space(name) {
            return Node::Space(mu);
        }
        if let Some(font) = symbols::text_font(name) {
            return match self.lex.raw_group() {
                Some((raw, closed)) => {
                    self.has_error |= !closed;
                    Node::Text {
                        text: unescape(raw),
                        font,
                    }
                }
                None => self.error_token(tok),
            };
        }
        self.parse_structure(name, tok)
    }

    /// The commands that take arguments or open a construct.
    fn parse_structure(&mut self, name: &'a str, tok: Token<'a>) -> Node {
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" => {
                self.parse_frac(name)
            }
            "sqrt" => {
                let index = self.parse_optional().map(Box::new);
                let body = Box::new(self.parse_arg());
                Node::Sqrt { index, body }
            }
            "left" => self.parse_delimited(),
            "begin" => self.parse_env(tok),
            "substack" => self.parse_substack(tok),
            "operatorname" => self.parse_operatorname(tok),
            "mathbin" | "mathrel" | "mathop" | "mathord" | "mathopen" | "mathclose"
            | "mathpunct" => self.parse_class(name),
            "overset" | "stackrel" | "underset" => self.parse_stack(name),
            "xrightarrow" | "xleftarrow" => {
                let under = self.parse_optional().map(Box::new);
                let over = Box::new(self.parse_arg());
                Node::Arrow {
                    right: name == "xrightarrow",
                    over,
                    under,
                }
            }
            "bmod" | "pmod" => self.parse_mod(name),
            "not" => Node::Negated(Box::new(self.parse_arg())),
            "boxed" => Node::Boxed(Box::new(self.parse_arg())),
            "displaystyle" => Node::Style(MathStyle::Display),
            "textstyle" => Node::Style(MathStyle::Text),
            "scriptstyle" => Node::Style(MathStyle::Script),
            "scriptscriptstyle" => Node::Style(MathStyle::ScriptScript),
            "\\" => {
                self.skip_row_spacing();
                Node::Break
            }
            // Meaningless alone; harmless to drop.
            "limits" | "nolimits" => Node::Group(Vec::new()),
            _ => self.error_token(tok),
        }
    }

    fn parse_frac(&mut self, name: &str) -> Node {
        let kind = if name.ends_with("binom") {
            FracKind::Binom
        } else {
            FracKind::Fraction
        };
        // `d…` and `\cfrac` force display style, `t…` text style.
        let style = match name.as_bytes()[0] {
            b'd' | b'c' => Some(MathStyle::Display),
            b't' => Some(MathStyle::Text),
            _ => None,
        };
        let num = Box::new(self.parse_arg());
        let den = Box::new(self.parse_arg());
        Node::Frac {
            num,
            den,
            kind,
            style,
        }
    }

    /// `\operatorname{word}`, and `\operatorname*` for limits above and below.
    fn parse_operatorname(&mut self, tok: Token<'a>) -> Node {
        let limits = if self.lex.peek().map(|t| t.kind) == Some(Kind::Char('*')) {
            self.lex.next();
            Limits::Display
        } else {
            Limits::Never
        };
        match self.lex.raw_group() {
            Some((raw, _)) => Node::Op {
                name: OpName::Text(unescape(raw)),
                limits,
            },
            None => self.error_token(tok),
        }
    }

    /// `\mathbin{…}` and the other spacing-class overrides.
    fn parse_class(&mut self, name: &str) -> Node {
        let class = match name {
            "mathbin" => Class::Bin,
            "mathrel" => Class::Rel,
            "mathop" => Class::Op,
            "mathopen" => Class::Open,
            "mathclose" => Class::Close,
            "mathpunct" => Class::Punct,
            _ => Class::Ord,
        };
        let body = Box::new(self.parse_arg());
        Node::Class { class, body }
    }

    /// `\overset{mark}{base}`, `\stackrel` (the same) and `\underset`.
    fn parse_stack(&mut self, name: &str) -> Node {
        let mark = Box::new(self.parse_arg());
        let base = Box::new(self.parse_arg());
        let (over, under) = if name == "underset" {
            (None, Some(mark))
        } else {
            (Some(mark), None)
        };
        Node::Stack { base, over, under }
    }

    /// `a \bmod b` and `\pmod{n}`.
    fn parse_mod(&mut self, name: &str) -> Node {
        let word = || Node::Op {
            name: OpName::Text("mod".to_owned()),
            limits: Limits::Never,
        };
        if name == "bmod" {
            return Node::Class {
                class: Class::Bin,
                body: Box::new(word()),
            };
        }
        let body = self.parse_arg();
        let paren = |ch, class| Node::Symbol { ch, class };
        Node::Group(vec![
            Node::Space(18),
            paren('(', Class::Open),
            word(),
            Node::Space(6),
            body,
            paren(')', Class::Close),
        ])
    }

    /// The `[2pt]` some write after a `\\`.
    fn skip_row_spacing(&mut self) {
        if self.lex.peek().map(|t| t.kind) != Some(Kind::Char('[')) {
            return;
        }
        while let Some(tok) = self.lex.next() {
            if tok.kind == Kind::Char(']') {
                return;
            }
        }
    }

    /// After `\left`: the delimiters and everything between, `\middle`s
    /// included.
    fn parse_delimited(&mut self) -> Node {
        let open = self.read_delimiter();
        let mut body = Vec::new();
        let close = loop {
            body.extend(self.parse_list(Ctx::Delim));
            match self.lex.next().map(|t| t.kind) {
                Some(Kind::Cmd("middle")) => {
                    let delim = self.read_delimiter();
                    body.push(Node::Middle(delim));
                }
                Some(Kind::Cmd("right")) => break self.read_delimiter(),
                _ => {
                    body.push(self.error("\\right"));
                    break None;
                }
            }
        };
        Node::Delimited { open, close, body }
    }

    /// The token after `\left`, `\middle` or `\right`; `.` and a bad one are
    /// no delimiter (a bad one is also an error).
    fn read_delimiter(&mut self) -> Option<char> {
        let Some(tok) = self.lex.next() else {
            self.has_error = true;
            return None;
        };
        let found = match tok.kind {
            Kind::Char('.') => return None,
            Kind::Char('<') => Some('⟨'),
            Kind::Char('>') => Some('⟩'),
            Kind::Char(c @ ('(' | ')' | '[' | ']' | '|' | '/')) => Some(c),
            Kind::Cmd(name) => symbols::delimiter(name),
            _ => None,
        };
        self.has_error |= found.is_none();
        found
    }

    /// After `\begin`: the environment's rows up to its `\end`.
    fn parse_env(&mut self, begin: Token<'a>) -> Node {
        let Some((raw, closed)) = self.lex.raw_group() else {
            return self.error_token(begin);
        };
        let name = raw.trim();
        let base = name.strip_suffix('*').unwrap_or(name);
        let kind = symbols::environment(base);
        // Only an array has a column argument; a matrix's `{` is cell content.
        let cols = match kind {
            Some(Env::Array) => self
                .lex
                .raw_group()
                .map_or_else(Vec::new, |(spec, _)| symbols::columns(spec)),
            _ => Vec::new(),
        };
        let (rows, hlines) = self.parse_rows(Some(name));
        match kind {
            Some(kind) if closed => Node::Env(Box::new(Environment {
                kind,
                cols,
                rows,
                hlines,
            })),
            _ => {
                let source = self.lex.slice_from(begin.start);
                self.error(source)
            }
        }
    }

    /// After `\substack`: `{ a \\ b }`.
    fn parse_substack(&mut self, tok: Token<'a>) -> Node {
        match self.lex.peek() {
            Some(open) if open.kind == Kind::Open => {
                self.lex.next();
                let (rows, hlines) = self.parse_rows(None);
                Node::Env(Box::new(Environment {
                    kind: Env::Substack,
                    cols: Vec::new(),
                    rows,
                    hlines,
                }))
            }
            _ => self.error_token(tok),
        }
    }

    /// Rows of cells up to `\end{name}` (`Some`) or the closing `}` of a
    /// substack (`None`), with the rows a `\hline` sits above.
    fn parse_rows(&mut self, name: Option<&str>) -> (Vec<Vec<Vec<Node>>>, Vec<usize>) {
        let ctx = if name.is_some() {
            Ctx::Cell
        } else {
            Ctx::BraceCell
        };
        let mut rows: Vec<Vec<Vec<Node>>> = Vec::new();
        let mut hlines: Vec<usize> = Vec::new();
        let mut row: Vec<Vec<Node>> = Vec::new();
        loop {
            while self.lex.peek().map(|t| t.kind) == Some(Kind::Cmd("hline")) {
                self.lex.next();
                if hlines.last() != Some(&rows.len()) {
                    hlines.push(rows.len());
                }
            }
            row.push(self.parse_list(ctx));
            match self.lex.next().map(|t| t.kind) {
                Some(Kind::Amp) => continue,
                Some(Kind::Cmd("\\")) => {
                    self.skip_row_spacing();
                    rows.push(std::mem::take(&mut row));
                }
                Some(Kind::Cmd("end")) if name.is_some() => {
                    self.finish_end(name);
                    break;
                }
                Some(Kind::Close) if name.is_none() => break,
                _ => {
                    let missing = self.error("\\end");
                    if let Some(cell) = row.last_mut() {
                        cell.push(missing);
                    }
                    break;
                }
            }
        }
        // A `\\` before `\end` leaves one empty row that is not a row.
        let trailing_empty = row.len() == 1 && row[0].is_empty();
        if !(trailing_empty && !rows.is_empty()) {
            rows.push(row);
        }
        (rows, hlines)
    }

    /// The `{name}` after `\end`; a different name is an error.
    fn finish_end(&mut self, expected: Option<&str>) {
        match self.lex.raw_group() {
            Some((raw, closed)) if closed && Some(raw.trim()) == expected => {}
            _ => self.has_error = true,
        }
    }
}

/// An ordinary character as an atom.
fn char_atom(c: char) -> Node {
    let (ch, class) = match c {
        '+' => ('+', Class::Bin),
        '-' => ('\u{2212}', Class::Bin),
        '*' => ('\u{2217}', Class::Bin),
        '=' | '<' | '>' | ':' => (c, Class::Rel),
        '(' | '[' => (c, Class::Open),
        ')' | ']' => (c, Class::Close),
        ',' | ';' => (c, Class::Punct),
        '\'' => ('′', Class::Ord),
        '~' => return Node::Space(6),
        _ => (c, Class::Ord),
    };
    Node::Symbol { ch, class }
}

/// `\text` content with its escapes (`\%`, `\$`, `\{`, …) resolved.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(e @ ('$' | '%' | '&' | '#' | '_' | '{' | '}' | '\\')) => out.push(e),
            Some(' ') | Some(',') => out.push(' '),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
