//! The expression tree a formula parses into. Pure data: the layout step
//! decides every size and position, so nothing here knows about fonts.

/// How an atom spaces against its neighbours (TeX's atom classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// Whether scripts on an operator sit above/below it or beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limits {
    /// Beside, always (integrals, `sin`).
    Never,
    /// Above/below in display style, beside otherwise (`\sum`, `\lim`).
    Display,
    /// Above/below, always (`\limits`).
    Always,
}

/// What an operator draws: one big glyph or a word set upright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpName {
    Glyph(char),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Roman,
    Italic,
    Bold,
    BoldItalic,
    SansSerif,
    Monospace,
    Blackboard,
    Calligraphic,
    Script,
    Fraktur,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FracKind {
    /// `\frac`: a bar.
    Fraction,
    /// `\binom`: no bar, in parentheses.
    Binom,
    /// `\atop`: no bar, no parentheses.
    Atop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Hat,
    Check,
    Tilde,
    Acute,
    Grave,
    Dot,
    DoubleDot,
    Breve,
    Bar,
    Vec,
    Ring,
    WideHat,
    WideTilde,
    OverLine,
    DotDotDot,
    WideCheck,
    OverRightArrow,
    OverLeftArrow,
    OverLeftRightArrow,
    OverBrace,
    OverParen,
    UnderLine,
    UnderBrace,
}

impl Accent {
    /// Whether the mark goes below its body.
    pub fn is_under(self) -> bool {
        matches!(self, Accent::UnderLine | Accent::UnderBrace)
    }
}

/// The brackets around a matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fence {
    None,
    Paren,
    Bracket,
    Brace,
    Bar,
    DoubleBar,
}

/// What of its body a `\phantom`-like command keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phantom {
    /// Width and height, nothing drawn.
    Both,
    /// Width only.
    Width,
    /// Height only.
    Height,
    /// Drawn, but takes no height (`\smash`).
    Smash,
}

/// Which diagonals a `\cancel` command draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    /// `\cancel`: bottom left to top right.
    Up,
    /// `\bcancel`: top left to bottom right.
    Down,
    /// `\xcancel`: both.
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Matrix(Fence),
    SmallMatrix,
    /// A brace on the left, left-aligned cells.
    Cases,
    /// A table with the column alignments of [`Environment::cols`].
    Array,
    /// `align`, `aligned`, `split`: cells alternate right/left at the `&`s.
    Aligned,
    /// `gather`, `gathered`, `equation`: one centred cell per row.
    Gathered,
    /// `\substack`: centred rows, small.
    Substack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Left,
    Center,
    Right,
    /// A `|` between columns.
    Rule,
}

/// A matrix-like block: rows of cells of nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    pub kind: Env,
    /// Only for [`Env::Array`].
    pub cols: Vec<Col>,
    pub rows: Vec<Vec<Vec<Node>>>,
    /// Row indices (`0..=rows.len()`) a horizontal rule is drawn above.
    pub hlines: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Symbol {
        ch: char,
        class: Class,
    },
    Op {
        name: OpName,
        limits: Limits,
    },
    Text {
        text: String,
        font: Font,
    },
    Group(Vec<Node>),
    Font {
        font: Font,
        body: Box<Node>,
    },
    Script {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Frac {
        num: Box<Node>,
        den: Box<Node>,
        kind: FracKind,
        /// `\dfrac`/`\tfrac` force a style; `None` follows the context.
        style: Option<MathStyle>,
    },
    Sqrt {
        index: Option<Box<Node>>,
        body: Box<Node>,
    },
    /// `\left … \right`; `None` is the empty delimiter `.`.
    Delimited {
        open: Option<char>,
        close: Option<char>,
        body: Vec<Node>,
    },
    /// `\big`, `\Bigl`, `\biggr`, `\Biggm`…: one delimiter of a fixed size,
    /// `height` times the font size, spaced as `class`.
    Sized {
        delim: Option<char>,
        height: f32,
        class: Class,
    },
    /// `\middle` inside a [`Node::Delimited`] body.
    Middle(Option<char>),
    Accent {
        kind: Accent,
        body: Box<Node>,
    },
    /// `\overset`, `\underset`, `\stackrel`.
    Stack {
        base: Box<Node>,
        over: Option<Box<Node>>,
        under: Option<Box<Node>>,
    },
    /// `\xrightarrow[under]{over}`.
    Arrow {
        right: bool,
        over: Box<Node>,
        under: Option<Box<Node>>,
    },
    /// `\not` before a relation: a slash through it.
    Negated(Box<Node>),
    /// `\mathbin`, `\mathrel`, …: spacing of another class.
    Class {
        class: Class,
        body: Box<Node>,
    },
    /// `\displaystyle` and friends, for the siblings after it.
    Style(MathStyle),
    /// `\tag{n}` (`starred`: `\tag*{n}`, with no parentheses): the equation's
    /// number, shown at the right edge of a display formula.
    Tag {
        text: String,
        starred: bool,
    },
    /// `\color{c}`: `0xRRGGBB` for the siblings after it.
    ColorSwitch(u32),
    /// `\textcolor{c}{body}`.
    Color {
        color: u32,
        body: Box<Node>,
    },
    /// A horizontal space in 1/18 em (TeX's mu); negative pulls back.
    Space(i16),
    /// `\\` outside an environment.
    Break,
    Env(Box<Environment>),
    Boxed(Box<Node>),
    /// `\cancel`, `\bcancel`, `\xcancel`: strike lines through the body.
    Cancel {
        kind: Strike,
        body: Box<Node>,
    },
    /// `\phantom`, `\hphantom`, `\vphantom`, `\smash`: the body's box without
    /// all or part of it.
    Phantom {
        kind: Phantom,
        body: Box<Node>,
    },
    /// Source that did not parse, kept verbatim to show in the error colour
    /// (empty: a missing argument).
    Error(String),
}
