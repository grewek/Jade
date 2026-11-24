//! Theme files and the engine that holds the active theme. `parse`, `loader`,
//! `paths` and `builtin` are GPUI-free; `palette` and `engine` are the GPUI
//! side. Format: `assets/themes/FORMAT.md`.

mod apply;
mod builtin;
mod choices;
mod contrast;
mod engine;
mod loader;
mod overrides;
mod palette;
mod parse;
mod paths;
mod roles;
#[cfg(test)]
mod tests;
mod watch;

pub use choices::{Choice, remember_justify, remember_vault};
pub use contrast::ContrastWarning;
pub use engine::ThemeEngine;
pub use loader::{Loaded, config_text, configured_justify, configured_vault, load_startup};
pub use overrides::ColorOverrides;
pub use palette::Palette;
pub use parse::{
    Defaults, FORMAT_VERSION, Kind, MAX_PALETTE_ENTRIES, MAX_THEME_BYTES, ResolvedTheme, Rgb,
    ThemeError,
};
pub use paths::{CONFIG_DIR_ENV, ConfigDirs};
pub use roles::{Alpha, Role};
pub use watch::start as watch;
