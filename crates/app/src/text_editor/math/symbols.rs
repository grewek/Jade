//! The command tables: which glyph, class, operator or style a name means.
//! Plain `match`es on the name, which the compiler turns into a length-and-byte
//! dispatch, so a lookup costs no hash and no allocation.

use super::ast::{Accent, Class, Col, Env, Fence, Font, Limits};

/// A command that is one glyph: Greek, relations, arrows, binary operators.
// A lookup table, not logic: splitting it would only scatter the rows.
#[allow(clippy::too_many_lines)]
pub fn symbol(name: &str) -> Option<(char, Class)> {
    use Class::{Bin, Close, Inner, Open, Ord, Punct, Rel};
    Some(match name {
        // Greek, lowercase.
        "alpha" => ('α', Ord),
        "beta" => ('β', Ord),
        "gamma" => ('γ', Ord),
        "delta" => ('δ', Ord),
        "epsilon" => ('ϵ', Ord),
        "varepsilon" => ('ε', Ord),
        "zeta" => ('ζ', Ord),
        "eta" => ('η', Ord),
        "theta" => ('θ', Ord),
        "vartheta" => ('ϑ', Ord),
        "iota" => ('ι', Ord),
        "kappa" => ('κ', Ord),
        "lambda" => ('λ', Ord),
        "mu" => ('μ', Ord),
        "nu" => ('ν', Ord),
        "xi" => ('ξ', Ord),
        "pi" => ('π', Ord),
        "varpi" => ('ϖ', Ord),
        "rho" => ('ρ', Ord),
        "varrho" => ('ϱ', Ord),
        "sigma" => ('σ', Ord),
        "varsigma" => ('ς', Ord),
        "tau" => ('τ', Ord),
        "upsilon" => ('υ', Ord),
        "phi" => ('ϕ', Ord),
        "varphi" => ('φ', Ord),
        "chi" => ('χ', Ord),
        "psi" => ('ψ', Ord),
        "omega" => ('ω', Ord),
        // Greek, uppercase (the others are the Latin capitals).
        "Gamma" => ('Γ', Ord),
        "Delta" => ('Δ', Ord),
        "Theta" => ('Θ', Ord),
        "Lambda" => ('Λ', Ord),
        "Xi" => ('Ξ', Ord),
        "Pi" => ('Π', Ord),
        "Sigma" => ('Σ', Ord),
        "Upsilon" => ('Υ', Ord),
        "Phi" => ('Φ', Ord),
        "Psi" => ('Ψ', Ord),
        "Omega" => ('Ω', Ord),
        // Ordinary symbols.
        "infty" => ('∞', Ord),
        "partial" => ('∂', Ord),
        "nabla" => ('∇', Ord),
        "forall" => ('∀', Ord),
        "exists" => ('∃', Ord),
        "nexists" => ('∄', Ord),
        "emptyset" | "varnothing" => ('∅', Ord),
        "neg" | "lnot" => ('¬', Ord),
        "aleph" => ('ℵ', Ord),
        "hbar" => ('ℏ', Ord),
        "ell" => ('ℓ', Ord),
        "Re" => ('ℜ', Ord),
        "Im" => ('ℑ', Ord),
        "wp" => ('℘', Ord),
        "imath" => ('ı', Ord),
        "jmath" => ('ȷ', Ord),
        "angle" => ('∠', Ord),
        "triangle" => ('△', Ord),
        "top" => ('⊤', Ord),
        "bot" => ('⊥', Ord),
        "prime" => ('′', Ord),
        "backslash" => ('\\', Ord),
        "Box" | "square" => ('□', Ord),
        "blacksquare" => ('■', Ord),
        "checkmark" => ('✓', Ord),
        "vert" => ('|', Ord),
        "Vert" | "|" => ('‖', Ord),
        "&" => ('&', Ord),
        "%" => ('%', Ord),
        "$" => ('$', Ord),
        "#" => ('#', Ord),
        "_" => ('_', Ord),
        "dots" | "ldots" | "dotsc" | "mathellipsis" => ('…', Inner),
        "cdots" | "dotsb" | "dotsm" => ('⋯', Inner),
        "vdots" => ('⋮', Inner),
        "ddots" => ('⋱', Inner),
        "colon" => (':', Punct),
        // Binary operators.
        "pm" => ('±', Bin),
        "mp" => ('∓', Bin),
        "times" => ('×', Bin),
        "div" => ('÷', Bin),
        "cdot" => ('⋅', Bin),
        "ast" => ('∗', Bin),
        "star" => ('⋆', Bin),
        "circ" => ('∘', Bin),
        "bullet" => ('∙', Bin),
        "cap" => ('∩', Bin),
        "cup" => ('∪', Bin),
        "uplus" => ('⊎', Bin),
        "sqcap" => ('⊓', Bin),
        "sqcup" => ('⊔', Bin),
        "vee" | "lor" => ('∨', Bin),
        "wedge" | "land" => ('∧', Bin),
        "setminus" => ('∖', Bin),
        "oplus" => ('⊕', Bin),
        "ominus" => ('⊖', Bin),
        "otimes" => ('⊗', Bin),
        "oslash" => ('⊘', Bin),
        "odot" => ('⊙', Bin),
        "dagger" => ('†', Bin),
        "ddagger" => ('‡', Bin),
        "diamond" => ('⋄', Bin),
        "triangleleft" => ('◁', Bin),
        "triangleright" => ('▷', Bin),
        // Relations.
        "leq" | "le" => ('≤', Rel),
        "geq" | "ge" => ('≥', Rel),
        "neq" | "ne" => ('≠', Rel),
        "approx" => ('≈', Rel),
        "equiv" => ('≡', Rel),
        "sim" => ('∼', Rel),
        "simeq" => ('≃', Rel),
        "cong" => ('≅', Rel),
        "propto" => ('∝', Rel),
        "ll" => ('≪', Rel),
        "gg" => ('≫', Rel),
        "subset" => ('⊂', Rel),
        "supset" => ('⊃', Rel),
        "subseteq" => ('⊆', Rel),
        "supseteq" => ('⊇', Rel),
        "nsubseteq" => ('⊈', Rel),
        "in" => ('∈', Rel),
        "notin" => ('∉', Rel),
        "ni" => ('∋', Rel),
        "perp" => ('⊥', Rel),
        "prec" => ('≺', Rel),
        "succ" => ('≻', Rel),
        "preceq" => ('⪯', Rel),
        "succeq" => ('⪰', Rel),
        "sqsubseteq" => ('⊑', Rel),
        "sqsupseteq" => ('⊒', Rel),
        "vdash" => ('⊢', Rel),
        "dashv" => ('⊣', Rel),
        "models" | "vDash" => ('⊨', Rel),
        "asymp" => ('≍', Rel),
        "doteq" => ('≐', Rel),
        "bowtie" => ('⋈', Rel),
        "mid" => ('∣', Rel),
        "nmid" => ('∤', Rel),
        "parallel" => ('∥', Rel),
        "nparallel" => ('∦', Rel),
        "therefore" => ('∴', Rel),
        "because" => ('∵', Rel),
        // Arrows (relations too).
        "to" | "rightarrow" => ('→', Rel),
        "leftarrow" | "gets" => ('←', Rel),
        "leftrightarrow" => ('↔', Rel),
        "Rightarrow" => ('⇒', Rel),
        "Leftarrow" => ('⇐', Rel),
        "Leftrightarrow" => ('⇔', Rel),
        "mapsto" => ('↦', Rel),
        "longmapsto" => ('⟼', Rel),
        "longrightarrow" => ('⟶', Rel),
        "longleftarrow" => ('⟵', Rel),
        "longleftrightarrow" => ('⟷', Rel),
        "Longrightarrow" | "implies" => ('⟹', Rel),
        "Longleftarrow" | "impliedby" => ('⟸', Rel),
        "Longleftrightarrow" | "iff" => ('⟺', Rel),
        "uparrow" => ('↑', Rel),
        "downarrow" => ('↓', Rel),
        "updownarrow" => ('↕', Rel),
        "Uparrow" => ('⇑', Rel),
        "Downarrow" => ('⇓', Rel),
        "nearrow" => ('↗', Rel),
        "searrow" => ('↘', Rel),
        "swarrow" => ('↙', Rel),
        "nwarrow" => ('↖', Rel),
        "hookrightarrow" => ('↪', Rel),
        "hookleftarrow" => ('↩', Rel),
        "rightharpoonup" => ('⇀', Rel),
        "leftharpoonup" => ('↼', Rel),
        "rightleftharpoons" => ('⇌', Rel),
        "leadsto" => ('⇝', Rel),
        // Brackets used without `\left`/`\right`.
        "{" | "lbrace" => ('{', Open),
        "}" | "rbrace" => ('}', Close),
        "lbrack" => ('[', Open),
        "rbrack" => (']', Close),
        "langle" => ('⟨', Open),
        "rangle" => ('⟩', Close),
        "lfloor" => ('⌊', Open),
        "rfloor" => ('⌋', Close),
        "lceil" => ('⌈', Open),
        "rceil" => ('⌉', Close),
        "lvert" => ('|', Open),
        "rvert" => ('|', Close),
        "lVert" => ('‖', Open),
        "rVert" => ('‖', Close),
        _ => return None,
    })
}

/// A big operator glyph and where its scripts go.
pub fn big_op(name: &str) -> Option<(char, Limits)> {
    Some(match name {
        "sum" => ('∑', Limits::Display),
        "prod" => ('∏', Limits::Display),
        "coprod" => ('∐', Limits::Display),
        "bigcup" => ('⋃', Limits::Display),
        "bigcap" => ('⋂', Limits::Display),
        "bigvee" => ('⋁', Limits::Display),
        "bigwedge" => ('⋀', Limits::Display),
        "bigoplus" => ('⨁', Limits::Display),
        "bigotimes" => ('⨂', Limits::Display),
        "bigodot" => ('⨀', Limits::Display),
        "biguplus" => ('⨄', Limits::Display),
        "bigsqcup" => ('⨆', Limits::Display),
        "int" => ('∫', Limits::Never),
        "iint" => ('∬', Limits::Never),
        "iiint" => ('∭', Limits::Never),
        "oint" => ('∮', Limits::Never),
        "oiint" => ('∯', Limits::Never),
        _ => return None,
    })
}

/// A named operator, set upright (`\sin`, `\lim`).
pub fn named_op(name: &str) -> Option<(&'static str, Limits)> {
    use Limits::{Display, Never};
    Some(match name {
        "sin" => ("sin", Never),
        "cos" => ("cos", Never),
        "tan" => ("tan", Never),
        "cot" => ("cot", Never),
        "sec" => ("sec", Never),
        "csc" => ("csc", Never),
        "arcsin" => ("arcsin", Never),
        "arccos" => ("arccos", Never),
        "arctan" => ("arctan", Never),
        "sinh" => ("sinh", Never),
        "cosh" => ("cosh", Never),
        "tanh" => ("tanh", Never),
        "coth" => ("coth", Never),
        "log" => ("log", Never),
        "ln" => ("ln", Never),
        "lg" => ("lg", Never),
        "exp" => ("exp", Never),
        "dim" => ("dim", Never),
        "ker" => ("ker", Never),
        "deg" => ("deg", Never),
        "hom" => ("hom", Never),
        "arg" => ("arg", Never),
        "lim" => ("lim", Display),
        "limsup" => ("lim sup", Display),
        "liminf" => ("lim inf", Display),
        "max" => ("max", Display),
        "min" => ("min", Display),
        "sup" => ("sup", Display),
        "inf" => ("inf", Display),
        "det" => ("det", Display),
        "gcd" => ("gcd", Display),
        "Pr" => ("Pr", Display),
        _ => return None,
    })
}

pub fn accent(name: &str) -> Option<Accent> {
    Some(match name {
        "hat" => Accent::Hat,
        "check" => Accent::Check,
        "tilde" => Accent::Tilde,
        "acute" => Accent::Acute,
        "grave" => Accent::Grave,
        "dot" => Accent::Dot,
        "ddot" => Accent::DoubleDot,
        "breve" => Accent::Breve,
        "bar" => Accent::Bar,
        "vec" => Accent::Vec,
        "mathring" => Accent::Ring,
        "widehat" => Accent::WideHat,
        "widetilde" => Accent::WideTilde,
        "overline" => Accent::OverLine,
        "overrightarrow" => Accent::OverRightArrow,
        "overleftarrow" => Accent::OverLeftArrow,
        "overleftrightarrow" => Accent::OverLeftRightArrow,
        "dddot" => Accent::DotDotDot,
        "widecheck" => Accent::WideCheck,
        "overparen" => Accent::OverParen,
        "overbrace" => Accent::OverBrace,
        "underline" | "underbar" => Accent::UnderLine,
        "underbrace" => Accent::UnderBrace,
        _ => return None,
    })
}

/// A colour as `\color` takes it: a name or `#rgb` / `#rrggbb`, as `0xRRGGBB`.
pub fn color(spec: &str) -> Option<u32> {
    let spec = spec.trim();
    if let Some(hex) = spec.strip_prefix('#') {
        let digits = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => Some(digits),
            3 => {
                let (r, g, b) = ((digits >> 8) & 15, (digits >> 4) & 15, digits & 15);
                Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
            }
            _ => None,
        };
    }
    Some(match spec.to_ascii_lowercase().as_str() {
        "black" => 0x000000,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "green" => 0x00ff00,
        "blue" => 0x0000ff,
        "yellow" => 0xffff00,
        "cyan" => 0x00ffff,
        "magenta" => 0xff00ff,
        "gray" | "grey" => 0x808080,
        "orange" => 0xff8000,
        "purple" => 0xbf0040,
        "violet" => 0x800080,
        "brown" => 0xbf8040,
        "pink" => 0xffbfbf,
        "teal" => 0x008080,
        "olive" => 0x808000,
        "lime" => 0xbfff00,
        "darkgray" => 0x404040,
        "lightgray" => 0xbfbfbf,
        _ => return None,
    })
}

/// The red, green and blue of a `0xRRGGBB` colour, each 0 to 1.
pub fn channels(rgb: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((rgb >> shift) & 255) as f32 / 255.0)
}

/// A command that sets its argument in another alphabet.
pub fn font(name: &str) -> Option<Font> {
    Some(match name {
        "mathbf" => Font::Bold,
        "mathit" | "mathnormal" => Font::Italic,
        "mathrm" => Font::Roman,
        "mathbb" => Font::Blackboard,
        "mathcal" => Font::Calligraphic,
        "mathscr" => Font::Script,
        "mathfrak" => Font::Fraktur,
        "mathsf" => Font::SansSerif,
        "mathtt" => Font::Monospace,
        "boldsymbol" | "bm" => Font::BoldItalic,
        _ => return None,
    })
}

/// The font a text command (`\text`, `\textbf`, …) sets its words in.
pub fn text_font(name: &str) -> Option<Font> {
    Some(match name {
        "text" | "textrm" | "mbox" | "hbox" => Font::Roman,
        "textit" => Font::Italic,
        "textbf" => Font::Bold,
        "textsf" => Font::SansSerif,
        "texttt" => Font::Monospace,
        _ => return None,
    })
}

/// A horizontal space in 1/18 em.
pub fn space(name: &str) -> Option<i16> {
    Some(match name {
        "," => 3,
        ":" | ">" => 4,
        ";" => 5,
        "!" => -3,
        " " => 6,
        "quad" => 18,
        "qquad" => 36,
        "enspace" => 9,
        _ => return None,
    })
}

/// The glyph a `\left`/`\right`/`\middle` delimiter command names.
pub fn delimiter(name: &str) -> Option<char> {
    Some(match name {
        "{" | "lbrace" => '{',
        "}" | "rbrace" => '}',
        "lbrack" => '[',
        "rbrack" => ']',
        "langle" => '⟨',
        "rangle" => '⟩',
        "lfloor" => '⌊',
        "rfloor" => '⌋',
        "lceil" => '⌈',
        "rceil" => '⌉',
        "vert" | "lvert" | "rvert" => '|',
        "Vert" | "|" | "lVert" | "rVert" => '‖',
        "backslash" => '\\',
        "uparrow" => '↑',
        "downarrow" => '↓',
        "updownarrow" => '↕',
        "Uparrow" => '⇑',
        "Downarrow" => '⇓',
        "Updownarrow" => '⇕',
        _ => return None,
    })
}

/// The environment `\begin{name}` opens, the star of `align*` already cut.
pub fn environment(name: &str) -> Option<Env> {
    Some(match name {
        "matrix" => Env::Matrix(Fence::None),
        "pmatrix" => Env::Matrix(Fence::Paren),
        "bmatrix" => Env::Matrix(Fence::Bracket),
        "Bmatrix" => Env::Matrix(Fence::Brace),
        "vmatrix" => Env::Matrix(Fence::Bar),
        "Vmatrix" => Env::Matrix(Fence::DoubleBar),
        "smallmatrix" => Env::SmallMatrix,
        "cases" => Env::Cases,
        "array" => Env::Array,
        "align" | "aligned" | "split" | "alignat" | "alignedat" | "eqnarray" => Env::Aligned,
        "gather" | "gathered" | "equation" => Env::Gathered,
        _ => return None,
    })
}

/// The columns an `array` argument such as `c|lr` or `p{3cm}c` names; widths
/// in a `p{…}` column are ignored.
pub fn columns(spec: &str) -> Vec<Col> {
    let mut out = Vec::with_capacity(spec.len());
    let mut depth = 0usize;
    for c in spec.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            'l' | 'p' | 'm' | 'b' => out.push(Col::Left),
            'c' => out.push(Col::Center),
            'r' => out.push(Col::Right),
            '|' => out.push(Col::Rule),
            _ => {}
        }
    }
    out
}
