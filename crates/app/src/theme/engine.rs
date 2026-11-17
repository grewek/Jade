//! The theme engine: owns the active theme's colors as a GPUI global.

use std::rc::Rc;

use gpui_kit::component::Theme;
use gpui_kit::{App, Global, Hsla};

use super::apply;
use super::builtin;
use super::loader;
use super::palette::Palette;
use super::parse::{Kind, ResolvedTheme};
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
        let loaded = loader::load_startup(&builtin::defaults(), dirs.as_ref());
        let mut notices = loaded.notices;
        let palette = Palette::from_resolved(&loaded.theme);
        if !apply_to_widgets(&loaded.theme, cx) {
            notices.push("could not apply the theme to the widgets".to_owned());
        }
        cx.set_global(ThemeEngine {
            palette,
            kind: loaded.theme.kind,
            active_id: loaded.id,
        });
        notices
    }

    /// Re-reads the configured theme and switches to it. On any error the
    /// active theme stays and the messages say why. Repaints every window.
    pub fn reload(dirs: Option<&ConfigDirs>, cx: &mut App) -> Vec<String> {
        let loaded = match loader::load_checked(&builtin::defaults(), dirs) {
            Ok(loaded) => loaded,
            Err(mut notices) => {
                notices.push(format!("keeping the {} theme", Self::global(cx).active_id));
                return notices;
            }
        };
        let mut notices = loaded.notices;
        if !apply_to_widgets(&loaded.theme, cx) {
            notices.push("could not apply the theme to the widgets".to_owned());
            return notices;
        }
        let engine = cx.global_mut::<ThemeEngine>();
        engine.palette = Palette::from_resolved(&loaded.theme);
        engine.kind = loaded.theme.kind;
        engine.active_id = loaded.id;
        cx.refresh_windows();
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

fn apply_to_widgets(theme: &ResolvedTheme, cx: &mut App) -> bool {
    let Some(config) = apply::build_config(theme) else {
        return false;
    };
    Theme::update(cx, |t| t.apply_config(&Rc::new(config)));
    true
}
