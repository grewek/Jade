//! The themes compiled into the binary, so Jade always has one to show.

use super::parse::{Defaults, ResolvedTheme};

pub const DEFAULT_ID: &str = "jade-light";
const ROLES: &str = include_str!("../../assets/themes/roles.toml");

pub const THEMES: &[(&str, &str)] = &[
    (
        "jade-light",
        include_str!("../../assets/themes/jade-light.toml"),
    ),
    (
        "jade-neon",
        include_str!("../../assets/themes/jade-neon.toml"),
    ),
    (
        "catppuccin-latte",
        include_str!("../../assets/themes/catppuccin-latte.toml"),
    ),
    (
        "catppuccin-frappe",
        include_str!("../../assets/themes/catppuccin-frappe.toml"),
    ),
    (
        "catppuccin-macchiato",
        include_str!("../../assets/themes/catppuccin-macchiato.toml"),
    ),
    (
        "catppuccin-mocha",
        include_str!("../../assets/themes/catppuccin-mocha.toml"),
    ),
];

pub fn source(id: &str) -> Option<&'static str> {
    THEMES
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, src)| *src)
}

/// Embedded files are validated by tests, so a failure here is a build bug,
/// not bad user input.
pub fn defaults() -> Defaults {
    match Defaults::parse(ROLES) {
        Ok(defaults) => defaults,
        Err(errors) => panic!("embedded roles.toml is invalid: {errors:?}"),
    }
}

pub fn default_theme(defaults: &Defaults) -> ResolvedTheme {
    let src = source(DEFAULT_ID).unwrap_or_default();
    match ResolvedTheme::parse(defaults, src) {
        Ok(theme) => theme,
        Err(errors) => panic!("embedded {DEFAULT_ID} is invalid: {errors:?}"),
    }
}
