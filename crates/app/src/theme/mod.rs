//! Theme files: parsing, validation and resolution to plain colors. No GPUI
//! here, so it is testable and benchable on its own; the engine that applies
//! a resolved theme builds on top. Format: `assets/themes/FORMAT.md`.

mod contrast;
mod parse;
mod roles;
#[cfg(test)]
mod tests;

pub use contrast::ContrastWarning;
pub use parse::{
    Defaults, FORMAT_VERSION, Kind, MAX_PALETTE_ENTRIES, MAX_THEME_BYTES, ResolvedTheme, Rgb,
    ThemeError,
};
pub use roles::{Alpha, Role};
