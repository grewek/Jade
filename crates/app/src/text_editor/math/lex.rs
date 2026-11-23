//! Tokens of a formula: a command, a character, or one of the five characters
//! TeX gives a meaning. Whitespace and `%` comments are skipped here, so the
//! parser never sees them; text in `\text{…}` is read raw instead.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind<'a> {
    Char(char),
    /// A command's name without the backslash: `frac`, or one non-letter
    /// character (`,`, `\`, `{`). Empty for a backslash at the end.
    Cmd(&'a str),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
}

#[derive(Debug, Clone, Copy)]
pub struct Token<'a> {
    pub kind: Kind<'a>,
    pub start: usize,
}

#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src, pos: 0 }
    }

    /// The source from `start` to the end of the last token read.
    pub fn slice_from(&self, start: usize) -> &'a str {
        assert!(start <= self.pos);
        &self.src[start..self.pos]
    }

    pub fn peek(&self) -> Option<Token<'a>> {
        self.clone().next()
    }

    pub fn next(&mut self) -> Option<Token<'a>> {
        self.skip_blank();
        let rest = &self.src[self.pos..];
        let c = rest.chars().next()?;
        let start = self.pos;
        let kind = match c {
            '{' => self.take(1, Kind::Open),
            '}' => self.take(1, Kind::Close),
            '^' => self.take(1, Kind::Sup),
            '_' => self.take(1, Kind::Sub),
            '&' => self.take(1, Kind::Amp),
            '\\' => self.command(&rest[1..]),
            _ => self.take(c.len_utf8(), Kind::Char(c)),
        };
        Some(Token { kind, start })
    }

    fn take(&mut self, len: usize, kind: Kind<'a>) -> Kind<'a> {
        self.pos += len;
        kind
    }

    /// `after` is the text past the backslash.
    fn command(&mut self, after: &'a str) -> Kind<'a> {
        let Some(first) = after.chars().next() else {
            return self.take(1, Kind::Cmd(""));
        };
        let len = if first.is_ascii_alphabetic() {
            after
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(after.len())
        } else {
            first.len_utf8()
        };
        self.take(1 + len, Kind::Cmd(&after[..len]))
    }

    fn skip_blank(&mut self) {
        loop {
            let rest = &self.src[self.pos..];
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if !trimmed.starts_with('%') {
                return;
            }
            self.pos += trimmed.find('\n').unwrap_or(trimmed.len());
        }
    }

    /// The text of a `{…}` group, braces balanced and `\x` skipped as a
    /// pair, with whether its closing brace was there. `None` (nothing
    /// consumed) when no `{` follows.
    pub fn raw_group(&mut self) -> Option<(&'a str, bool)> {
        let mut probe = self.clone();
        let open = probe.next()?;
        if open.kind != Kind::Open {
            return None;
        }
        let body = &self.src[probe.pos..];
        let mut depth = 1usize;
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos = probe.pos + i + 1;
                        return Some((&body[..i], true));
                    }
                }
                _ => {}
            }
        }
        self.pos = self.src.len();
        Some((body, false))
    }
}
