//! Editor colors, taken from the theme's palette. Row heights and layout
//! never depend on color, so a theme change repaints without touching the
//! document or row caches.

use gpui_kit::Hsla;

use super::preview::callout::CalloutType;
use crate::settings::HEADING_LEVELS;
use crate::theme::{Palette, Role};

/// One entry per callout color group, in `callouts` order.
const CALLOUT_ROLES: [Role; 7] = [
    Role::Info,
    Role::Summary,
    Role::Tip,
    Role::Success,
    Role::Warning,
    Role::Error,
    Role::Neutral,
];

pub(super) fn callout_role(kind: CalloutType) -> Role {
    match kind {
        CalloutType::Note | CalloutType::Info => Role::Info,
        CalloutType::Summary => Role::Summary,
        CalloutType::Tip => Role::Tip,
        CalloutType::Todo | CalloutType::Success => Role::Success,
        CalloutType::Question | CalloutType::Warning => Role::Warning,
        CalloutType::Failure | CalloutType::Danger | CalloutType::Bug => Role::Error,
        CalloutType::Example | CalloutType::Quote => Role::Neutral,
    }
}

/// What shaping needs to color a line's text runs.
#[derive(Clone, Copy)]
pub(super) struct InlineColors {
    pub text: Hsla,
    pub code: Hsla,
    pub wikilink: Hsla,
    pub link: Hsla,
    /// A link whose target does not exist.
    pub broken: Hsla,
    /// Blockquote and list-marker text.
    pub subtle: Hsla,
    /// Heading text, per level 1..=6.
    pub(in crate::text_editor) heading: [Hsla; HEADING_LEVELS],
    callouts: [Hsla; CALLOUT_ROLES.len()],
}

impl InlineColors {
    pub fn from_palette(palette: &Palette) -> InlineColors {
        InlineColors {
            text: palette.color(Role::Text),
            code: palette.color(Role::Code),
            wikilink: palette.color(Role::Wikilink),
            link: palette.color(Role::Link),
            broken: palette.color(Role::Error),
            subtle: palette.color(Role::TextSubtle),
            heading: std::array::from_fn(|i| palette.color(Role::heading_level(i as u8 + 1))),
            callouts: CALLOUT_ROLES.map(|role| palette.color(role)),
        }
    }

    /// The color of a level's heading text (1..=6, clamped).
    pub fn heading(&self, level: u8) -> Hsla {
        self.heading[usize::from(level.clamp(1, HEADING_LEVELS as u8)) - 1]
    }

    pub fn callout(&self, kind: CalloutType) -> Hsla {
        let role = callout_role(kind);
        match CALLOUT_ROLES.iter().position(|r| *r == role) {
            Some(i) => self.callouts[i],
            None => self.text,
        }
    }
}

/// Every run in one color. For measuring (row counts, hit-testing), where
/// color cannot change a line's size.
impl From<Hsla> for InlineColors {
    fn from(color: Hsla) -> InlineColors {
        InlineColors {
            text: color,
            code: color,
            wikilink: color,
            link: color,
            broken: color,
            subtle: color,
            heading: [color; HEADING_LEVELS],
            callouts: [color; CALLOUT_ROLES.len()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Alpha, Defaults, ResolvedTheme};

    const ROLES: &str = include_str!("../../assets/themes/roles.toml");
    const MOCHA: &str = include_str!("../../assets/themes/catppuccin-mocha.toml");

    fn mocha() -> Palette {
        let defaults = Defaults::parse(ROLES).expect("roles");
        Palette::from_resolved(&ResolvedTheme::parse(&defaults, MOCHA).expect("mocha"))
    }

    #[test]
    fn inline_colors_follow_the_palette() {
        let palette = mocha();
        let colors = InlineColors::from_palette(&palette);
        assert_eq!(colors.text, palette.color(Role::Text));
        assert_eq!(colors.link, palette.color(Role::Link));
        assert_eq!(colors.wikilink, palette.color(Role::Wikilink));
        assert_eq!(colors.code, palette.color(Role::Code));
        assert_eq!(colors.subtle, palette.color(Role::TextSubtle));
        assert_ne!(colors.link, colors.wikilink, "links stay distinguishable");
    }

    #[test]
    fn every_callout_kind_resolves_to_its_role_color() {
        let palette = mocha();
        let colors = InlineColors::from_palette(&palette);
        for kind in [
            CalloutType::Note,
            CalloutType::Info,
            CalloutType::Summary,
            CalloutType::Tip,
            CalloutType::Todo,
            CalloutType::Success,
            CalloutType::Question,
            CalloutType::Warning,
            CalloutType::Failure,
            CalloutType::Danger,
            CalloutType::Bug,
            CalloutType::Example,
            CalloutType::Quote,
        ] {
            assert_eq!(
                colors.callout(kind),
                palette.color(callout_role(kind)),
                "{kind:?}"
            );
        }
        let tint = palette.with_alpha(Role::Info, Alpha::CalloutBackground);
        assert!((tint.a - 0.10).abs() < 1e-6);
    }

    #[test]
    fn monochrome_colors_every_run_alike() {
        let color = mocha().color(Role::Text);
        let colors = InlineColors::from(color);
        assert!(colors.link == color && colors.callout(CalloutType::Bug) == color);
    }
}

#[cfg(test)]
mod heading_tests {
    use super::*;
    use crate::theme::{Defaults, ResolvedTheme};

    #[test]
    fn heading_colors_follow_each_levels_role() {
        let defaults = Defaults::parse(include_str!("../../assets/themes/roles.toml")).unwrap();
        let mocha = include_str!("../../assets/themes/catppuccin-mocha.toml");
        let theme = mocha.replacen(
            "[overrides]\n",
            "[overrides]\nheading = \"#112233\"\nheading_5 = \"#445566\"\n",
            1,
        );
        let palette = Palette::from_resolved(&ResolvedTheme::parse(&defaults, &theme).unwrap());
        let colors = InlineColors::from_palette(&palette);
        for level in 1..=6 {
            assert_eq!(
                colors.heading(level),
                palette.color(Role::heading_level(level))
            );
        }
        assert_ne!(colors.heading(5), colors.heading(4), "a level can differ");
        assert_eq!(colors.heading(0), colors.heading(1), "clamped");
        assert_eq!(colors.heading(9), colors.heading(6), "clamped");
    }
}
