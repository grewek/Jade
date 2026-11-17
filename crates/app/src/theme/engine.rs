//! The theme engine: owns the active theme's colors as a GPUI global.

use gpui_kit::{App, Global};

use super::builtin;
use super::loader;
use super::palette::Palette;
use super::parse::Kind;
use super::paths::ConfigDirs;

pub struct ThemeEngine {
    active_id: String,
    kind: Kind,
    palette: Palette,
}

impl Global for ThemeEngine {}

impl ThemeEngine {
    /// Loads the startup theme and installs the engine as a global. Returns
    /// the messages to show the user; `run()` prints them to stderr.
    pub fn install(dirs: Option<ConfigDirs>, cx: &mut App) -> Vec<String> {
        let defaults = builtin::defaults();
        let loaded = loader::load_startup(&defaults, dirs.as_ref());
        cx.set_global(ThemeEngine {
            palette: Palette::from_resolved(&loaded.theme),
            kind: loaded.theme.kind,
            active_id: loaded.id,
        });
        loaded.notices
    }

    pub fn global(cx: &App) -> &ThemeEngine {
        cx.global::<ThemeEngine>()
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn active_id(&self) -> &str {
        &self.active_id
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }
}
