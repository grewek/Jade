//! Editor colors, taken from the theme's palette. Row heights and layout
//! never depend on color, so a theme change repaints without touching the
//! document or row caches.

use gpui_kit::Hsla;

use super::callout::CalloutType;
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
    /// Blockquote and list-marker text.
    pub subtle: Hsla,
    callouts: [Hsla; CALLOUT_ROLES.len()],
}

impl InlineColors {
    pub fn from_palette(palette: &Palette) -> InlineColors {
        InlineColors {
            text: palette.color(Role::Text),
            code: palette.color(Role::Code),
            wikilink: palette.color(Role::Wikilink),
            link: palette.color(Role::Link),
            subtle: palette.color(Role::TextSubtle),
            callouts: CALLOUT_ROLES.map(|role| palette.color(role)),
        }
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
            subtle: color,
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
