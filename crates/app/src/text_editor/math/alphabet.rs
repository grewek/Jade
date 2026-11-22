//! Math alphabets: `\mathbb{R}`, `\mathbf{x}` and the italic of plain letters
//! are different Unicode characters (the Mathematical Alphanumeric Symbols), so
//! a letter in a font is one char lookup away from its glyph.

use super::ast::Font;

/// `ch` as it is drawn in `font`; `None` is the default of a bare letter:
/// italic Latin and lowercase Greek, everything else as it is.
pub fn styled(ch: char, font: Option<Font>) -> char {
    match font {
        None => match ch {
            'a'..='z' | 'A'..='Z' | 'α'..='ω' | 'ϑ' | 'ϰ' | 'ϕ' | 'ϱ' | 'ϖ' | 'ϵ' => {
                in_font(ch, Font::Italic)
            }
            _ => ch,
        },
        Some(font) => in_font(ch, font),
    }
}

fn in_font(ch: char, font: Font) -> char {
    match font {
        Font::Roman => ch,
        Font::Italic => match ch {
            'h' => '\u{210E}',
            _ => latin(ch, 0x1D434, 0x1D44E, &[]).unwrap_or_else(|| greek(ch, 0x1D6E2, 0x1D6FC)),
        },
        Font::Bold => latin(ch, 0x1D400, 0x1D41A, &[])
            .or_else(|| digit(ch, 0x1D7CE))
            .unwrap_or_else(|| greek(ch, 0x1D6A8, 0x1D6C2)),
        Font::BoldItalic => latin(ch, 0x1D468, 0x1D482, &[])
            .or_else(|| digit(ch, 0x1D7CE))
            .unwrap_or_else(|| greek(ch, 0x1D71C, 0x1D736)),
        Font::SansSerif => latin(ch, 0x1D5A0, 0x1D5BA, &[])
            .or_else(|| digit(ch, 0x1D7E2))
            .unwrap_or(ch),
        Font::Monospace => latin(ch, 0x1D670, 0x1D68A, &[])
            .or_else(|| digit(ch, 0x1D7F6))
            .unwrap_or(ch),
        Font::Blackboard => latin(
            ch,
            0x1D538,
            0x1D552,
            &[
                ('C', '\u{2102}'),
                ('H', '\u{210D}'),
                ('N', '\u{2115}'),
                ('P', '\u{2119}'),
                ('Q', '\u{211A}'),
                ('R', '\u{211D}'),
                ('Z', '\u{2124}'),
            ],
        )
        .or_else(|| digit(ch, 0x1D7D8))
        .unwrap_or(ch),
        Font::Calligraphic | Font::Script => latin(
            ch,
            0x1D49C,
            0x1D4B6,
            &[
                ('B', '\u{212C}'),
                ('E', '\u{2130}'),
                ('F', '\u{2131}'),
                ('H', '\u{210B}'),
                ('I', '\u{2110}'),
                ('L', '\u{2112}'),
                ('M', '\u{2133}'),
                ('R', '\u{211B}'),
                ('e', '\u{212F}'),
                ('g', '\u{210A}'),
                ('o', '\u{2134}'),
            ],
        )
        .unwrap_or(ch),
        Font::Fraktur => latin(
            ch,
            0x1D504,
            0x1D51E,
            &[
                ('C', '\u{212D}'),
                ('H', '\u{210C}'),
                ('I', '\u{2111}'),
                ('R', '\u{211C}'),
                ('Z', '\u{2128}'),
            ],
        )
        .unwrap_or(ch),
    }
}

/// A Latin letter in an alphabet that starts at `caps` and `small`; `holes` are
/// the letters the block leaves to Letterlike Symbols.
fn latin(ch: char, caps: u32, small: u32, holes: &[(char, char)]) -> Option<char> {
    if let Some(&(_, to)) = holes.iter().find(|(from, _)| *from == ch) {
        return Some(to);
    }
    let base = match ch {
        'A'..='Z' => caps + (ch as u32 - 'A' as u32),
        'a'..='z' => small + (ch as u32 - 'a' as u32),
        _ => return None,
    };
    char::from_u32(base)
}

fn digit(ch: char, zero: u32) -> Option<char> {
    ch.is_ascii_digit()
        .then(|| char::from_u32(zero + (ch as u32 - '0' as u32)))
        .flatten()
}

/// Greek in an alphabet that starts at `caps` (for Α) and `small` (for α);
/// the lowercase variant forms sit after the letters. Anything else is as is.
fn greek(ch: char, caps: u32, small: u32) -> char {
    let at = |base: u32, offset: u32| char::from_u32(base + offset).unwrap_or(ch);
    match ch {
        // Capitals stay upright in TeX's math italic, so only the other
        // alphabets move them.
        'Α'..='Ω' if caps != 0x1D6E2 => at(caps, ch as u32 - 'Α' as u32),
        'α'..='ω' => at(small, ch as u32 - 'α' as u32),
        'ϑ' => at(small, 0x1D717 - 0x1D6FC),
        'ϰ' => at(small, 0x1D718 - 0x1D6FC),
        'ϕ' => at(small, 0x1D719 - 0x1D6FC),
        'ϱ' => at(small, 0x1D71A - 0x1D6FC),
        'ϖ' => at(small, 0x1D71B - 0x1D6FC),
        'ϵ' => at(small, 0x1D716 - 0x1D6FC),
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_letters_are_italic_and_digits_upright() {
        assert_eq!(styled('x', None), '\u{1D465}');
        assert_eq!(styled('A', None), '\u{1D434}');
        assert_eq!(styled('1', None), '1');
        assert_eq!(styled('+', None), '+');
        // The italic h is a Letterlike Symbol; the block has a hole there.
        assert_eq!(styled('h', None), '\u{210E}');
    }

    #[test]
    fn lowercase_greek_is_italic_and_capitals_stay_upright() {
        assert_eq!(styled('α', None), '\u{1D6FC}');
        assert_eq!(styled('ω', None), '\u{1D714}');
        assert_eq!(styled('ϕ', None), '\u{1D719}');
        assert_eq!(styled('Γ', None), 'Γ');
    }

    #[test]
    fn alphabets_use_the_letterlike_exceptions() {
        assert_eq!(styled('R', Some(Font::Blackboard)), '\u{211D}');
        assert_eq!(styled('A', Some(Font::Blackboard)), '\u{1D538}');
        assert_eq!(styled('1', Some(Font::Blackboard)), '\u{1D7D9}');
        assert_eq!(styled('L', Some(Font::Calligraphic)), '\u{2112}');
        assert_eq!(styled('C', Some(Font::Fraktur)), '\u{212D}');
        assert_eq!(styled('x', Some(Font::Bold)), '\u{1D431}');
        assert_eq!(styled('x', Some(Font::Roman)), 'x');
        assert_eq!(styled('+', Some(Font::Bold)), '+');
    }

    #[test]
    fn bold_greek_and_bold_italic() {
        assert_eq!(styled('α', Some(Font::Bold)), '\u{1D6C2}');
        assert_eq!(styled('Α', Some(Font::Bold)), '\u{1D6A8}');
        assert_eq!(styled('x', Some(Font::BoldItalic)), '\u{1D499}');
    }
}
