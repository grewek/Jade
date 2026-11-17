//! Theme files and the engine that holds the active theme. `parse`, `loader`,
//! `paths` and `builtin` are GPUI-free; `palette` and `engine` are the GPUI
//! side. Format: `assets/themes/FORMAT.md`.

mod apply;
mod builtin;
mod contrast;
mod engine;
mod loader;
mod palette;
mod parse;
mod paths;
mod roles;
#[cfg(test)]
mod tests;

pub use contrast::ContrastWarning;
pub use engine::ThemeEngine;
pub use loader::{Loaded, load_startup};
pub use palette::Palette;
pub use parse::{
    Defaults, FORMAT_VERSION, Kind, MAX_PALETTE_ENTRIES, MAX_THEME_BYTES, ResolvedTheme, Rgb,
    ThemeError,
};
pub use paths::{CONFIG_DIR_ENV, ConfigDirs};
pub use roles::{Alpha, Role};
