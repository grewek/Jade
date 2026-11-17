//! WCAG contrast warnings. A low-contrast theme is the user's choice, so
//! these are reported, never rejected.

use std::fmt;

use super::parse::Rgb;
use super::roles::Role;

/// (text, background, minimum ratio). 4.5 is WCAG AA for text, 3.0 for UI.
const PAIRS: &[(Role, Role, f32)] = &[
    (Role::Text, Role::Background, 4.5),
    (Role::TextMuted, Role::Background, 4.5),
    (Role::TextMuted, Role::BackgroundSecondary, 4.5),
    (Role::TextSubtle, Role::Background, 4.5),
    (Role::Link, Role::Background, 4.5),
    (Role::Wikilink, Role::Background, 4.5),
    (Role::Code, Role::Background, 4.5),
    (Role::Info, Role::Background, 4.5),
    (Role::Summary, Role::Background, 4.5),
    (Role::Tip, Role::Background, 4.5),
    (Role::Success, Role::Background, 4.5),
    (Role::Warning, Role::Background, 4.5),
    (Role::Error, Role::Background, 4.5),
    (Role::Neutral, Role::Background, 4.5),
    (Role::TextOnAccent, Role::Accent, 4.5),
    (Role::Accent, Role::Background, 3.0),
];

#[derive(Clone, Debug, PartialEq)]
pub struct ContrastWarning {
    pub foreground: Role,
    pub background: Role,
    pub ratio: f32,
    pub minimum: f32,
}

impl fmt::Display for ContrastWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} on {} has contrast {:.1}:1, below {:.1}:1",
            self.foreground.name(),
            self.background.name(),
            self.ratio,
            self.minimum
        )
    }
}

fn luminance(c: Rgb) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

pub fn ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub fn check(colors: &[Rgb; Role::COUNT]) -> Vec<ContrastWarning> {
    let mut warnings = Vec::new();
    for &(fg, bg, minimum) in PAIRS {
        let r = ratio(colors[fg.index()], colors[bg.index()]);
        if r < minimum {
            warnings.push(ContrastWarning {
                foreground: fg,
                background: bg,
                ratio: r,
                minimum,
            });
        }
    }
    warnings
}
