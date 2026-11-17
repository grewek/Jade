//! The resolved theme as GPUI colors, computed once per theme so rendering
//! only copies a color.

use gpui_kit::{Hsla, rgb};

use super::parse::{ResolvedTheme, Rgb};
use super::roles::{Alpha, Role};

#[derive(Clone, Debug)]
pub struct Palette {
    colors: [Hsla; Role::COUNT],
    alphas: [f32; Alpha::COUNT],
}

fn hsla(c: Rgb) -> Hsla {
    rgb((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)).into()
}

impl Palette {
    pub fn from_resolved(theme: &ResolvedTheme) -> Palette {
        Palette {
            colors: std::array::from_fn(|i| hsla(theme.color(Role::ALL[i]))),
            alphas: std::array::from_fn(|i| theme.alpha(Alpha::ALL[i])),
        }
    }

    pub fn color(&self, role: Role) -> Hsla {
        self.colors[role.index()]
    }

    /// `role` at the theme's opacity for `alpha`.
    pub fn with_alpha(&self, role: Role, alpha: Alpha) -> Hsla {
        self.color(role).opacity(self.alphas[alpha.index()])
    }
}
