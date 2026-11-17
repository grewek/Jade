//! The theme engine: owns the active theme's colors as a GPUI global.

use std::rc::Rc;

use gpui_kit::component::Theme;
use gpui_kit::{App, Global, Hsla};

use super::apply;
use super::builtin;
use super::loader;
use super::palette::Palette;
use super::parse::Kind;
use super::paths::ConfigDirs;
use super::roles::{Alpha, Role};

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
        let mut notices = loaded.notices;
        match apply::build_config(&loaded.theme) {
            Some(config) => Theme::update(cx, |theme| theme.apply_config(&Rc::new(config))),
            None => notices.push("could not apply the theme to the widgets".to_owned()),
        }
        cx.set_global(ThemeEngine {
            palette: Palette::from_resolved(&loaded.theme),
            kind: loaded.theme.kind,
            active_id: loaded.id,
        });
        notices
    }

    pub fn global(cx: &App) -> &ThemeEngine {
        cx.global::<ThemeEngine>()
    }

    /// The color for `role`; shorthand for `global(cx).palette().color(role)`.
    pub fn color(cx: &App, role: Role) -> Hsla {
        Self::global(cx).palette.color(role)
    }

    pub fn faded(cx: &App, role: Role, alpha: Alpha) -> Hsla {
        Self::global(cx).palette.with_alpha(role, alpha)
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
