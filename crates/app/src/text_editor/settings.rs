//! Editor preferences from `config.toml`: whether text is justified and the
//! typography (#58). Held as one global; a missing one (tests) means the
//! defaults. Colours are the theme engine's.

use gpui_kit::{App, Global};

use crate::settings::{self, Typography};
use crate::theme::{self, ConfigDirs, ThemeEngine};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct TextSettings {
    /// Prose rows are stretched to the wrap width (Blocksatz).
    pub justify: bool,
    pub typography: Typography,
}

impl Global for TextSettings {}

impl TextSettings {
    /// Reads `config.toml` once at startup; returns what to tell the user.
    pub fn install(dirs: Option<&ConfigDirs>, cx: &mut App) -> Vec<String> {
        let read = settings::load(dirs);
        cx.set_global(TextSettings {
            justify: theme::configured_justify(dirs),
            typography: read.typography,
        });
        read.notices
    }

    /// Reads `config.toml` again (the file changed). A layout key that moved
    /// makes every row cache rebuild at the next paint; anything else only
    /// repaints. Returns what to tell the user.
    pub fn reload(dirs: Option<&ConfigDirs>, cx: &mut App) -> Vec<String> {
        let read = settings::load(dirs);
        let next = TextSettings {
            justify: theme::configured_justify(dirs),
            typography: read.typography,
        };
        let before = cx.try_global::<TextSettings>().copied().unwrap_or_default();
        if next != before {
            if before.typography.layout_differs(&next.typography) {
                settings::layout_changed();
            }
            cx.set_global(next);
            cx.refresh_windows();
        }
        read.notices
    }

    pub fn justify(cx: &App) -> bool {
        cx.try_global::<TextSettings>().is_some_and(|s| s.justify)
    }

    pub fn typography(cx: &App) -> Typography {
        cx.try_global::<TextSettings>()
            .map_or_else(Typography::default, |s| s.typography)
    }

    /// A user's pick: applies now and remembers it in `config.toml`; the
    /// switch holds even when remembering fails.
    pub fn choose_justify(on: bool, cx: &mut App) -> Result<(), String> {
        let typography = Self::typography(cx);
        cx.set_global(TextSettings {
            justify: on,
            typography,
        });
        cx.refresh_windows();
        match ThemeEngine::config_dirs(cx) {
            Some(dirs) => theme::remember_justify(dirs, on),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn write(dirs: &ConfigDirs, src: &str) {
        std::fs::create_dir_all(dirs.base()).unwrap();
        std::fs::write(dirs.config_file(), src).unwrap();
    }

    #[gpui_kit::test]
    fn install_reads_the_file_and_reload_follows_it(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = ConfigDirs::new(tmp.path().to_path_buf());
        write(&dirs, "justify = true\n[font]\nsize = 20\nsise = 1\n");
        let notices = cx.update(|cx| TextSettings::install(Some(&dirs), cx));
        assert_eq!(notices.len(), 1, "{notices:?}");
        cx.update(|cx| {
            assert!(TextSettings::justify(cx));
            assert_eq!(TextSettings::typography(cx).font.size, 20.0);
        });
        // A paint-only key repaints; the layout generation stays.
        let before = settings::layout_generation();
        write(
            &dirs,
            "justify = true\n[font]\nsize = 20\n[shape]\nradius = 9\n",
        );
        cx.update(|cx| TextSettings::reload(Some(&dirs), cx));
        assert_eq!(settings::layout_generation(), before);
        // A layout key bumps it, once.
        write(&dirs, "[font]\nsize = 22\n");
        cx.update(|cx| TextSettings::reload(Some(&dirs), cx));
        assert_eq!(settings::layout_generation(), before + 1);
        cx.update(|cx| {
            assert!(!TextSettings::justify(cx), "justify follows the file too");
            assert_eq!(TextSettings::typography(cx).font.size, 22.0);
            TextSettings::reload(Some(&dirs), cx);
        });
        assert_eq!(settings::layout_generation(), before + 1, "unchanged file");
    }

    #[gpui_kit::test]
    fn a_missing_global_means_the_defaults(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!TextSettings::justify(cx));
            assert_eq!(TextSettings::typography(cx), Typography::default());
        });
    }
}
