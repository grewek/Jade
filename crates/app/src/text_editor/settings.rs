//! Editor preferences that change how text is drawn, not laid out.

use gpui_kit::{App, Global};

use crate::theme::{self, ConfigDirs, ThemeEngine};

/// Held as a global; a missing one (tests) means every preference is off.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct TextSettings {
    /// Prose rows are stretched to the wrap width (Blocksatz).
    pub justify: bool,
}

impl Global for TextSettings {}

impl TextSettings {
    /// Reads `config.toml` once at startup.
    pub fn install(dirs: Option<&ConfigDirs>, cx: &mut App) {
        cx.set_global(TextSettings {
            justify: theme::configured_justify(dirs),
        });
    }

    pub fn justify(cx: &App) -> bool {
        cx.try_global::<TextSettings>().is_some_and(|s| s.justify)
    }

    /// A user's pick: applies now and remembers it in `config.toml`; the
    /// switch holds even when remembering fails.
    pub fn choose_justify(on: bool, cx: &mut App) -> Result<(), String> {
        cx.set_global(TextSettings { justify: on });
        cx.refresh_windows();
        match ThemeEngine::config_dirs(cx) {
            Some(dirs) => theme::remember_justify(dirs, on),
            None => Ok(()),
        }
    }
}
