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

/// The body font size, as the editor lays out and paints with it.
pub fn body_font_size() -> gpui_kit::Pixels {
    gpui_kit::px(settings::body_font_size())
}

/// The height of one body row (`font.size * font.line_height`).
pub fn body_row_height() -> gpui_kit::Pixels {
    gpui_kit::px(settings::body_row_height())
}

impl TextSettings {
    /// Reads `config.toml` once at startup; returns what to tell the user.
    pub fn install(dirs: Option<&ConfigDirs>, cx: &mut App) -> Vec<String> {
        let read = settings::load(dirs);
        settings::set_body(&read.typography);
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
            settings::set_body(&next.typography);
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

    /// Applies a change to the typography now, as a reload of an edited file
    /// does: a layout key that moved rebuilds the row caches, anything else
    /// repaints.
    pub fn apply_typography(edit: impl FnOnce(&mut Typography), cx: &mut App) {
        let before = cx.try_global::<TextSettings>().copied().unwrap_or_default();
        let mut next = before;
        edit(&mut next.typography);
        if next == before {
            return;
        }
        if before.typography.layout_differs(&next.typography) {
            settings::layout_changed();
        }
        settings::set_body(&next.typography);
        cx.set_global(next);
        cx.refresh_windows();
    }

    /// Remembers `key = value` of `[section]` in `config.toml` (a pick made
    /// in Settings, already applied); the setting holds even when this fails.
    pub fn remember(section: &str, key: &str, value: &str, cx: &App) -> Result<(), String> {
        match ThemeEngine::config_dirs(cx) {
            Some(dirs) => theme::remember_setting(dirs, Some(section), key, value),
            None => Ok(()),
        }
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
    fn a_dialog_pick_applies_now_and_is_written_in_place(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = ConfigDirs::new(tmp.path().to_path_buf());
        write(&dirs, "# mine\n[font]\nsize = 16 # body\n");
        cx.update(|cx| {
            TextSettings::install(Some(&dirs), cx);
            let before = settings::layout_generation();
            TextSettings::apply_typography(|t| t.font.size = 24.0, cx);
            assert_eq!(TextSettings::typography(cx).font.size, 24.0);
            assert!(
                settings::layout_generation() > before,
                "a size is a layout key"
            );
            let before = settings::layout_generation();
            TextSettings::apply_typography(|t| t.shape.radius = settings::Radius::Px(9.0), cx);
            assert_eq!(
                settings::layout_generation(),
                before,
                "a radius only repaints"
            );
            theme::remember_setting(&dirs, Some("font"), "size", "24").unwrap();
            settings::set_body(&Typography::default());
        });
        let text = std::fs::read_to_string(dirs.config_file()).unwrap();
        assert_eq!(text, "# mine\n[font]\nsize = 24 # body\n");
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
        // The editor's body metrics follow the file: size 22 at the default 1.625.
        assert_eq!(settings::body_font_size(), 22.0);
        assert_eq!(settings::body_row_height(), 22.0 * 1.625);
        write(&dirs, "[font]\nsize = 20\nline_height = 2\n");
        cx.update(|cx| TextSettings::reload(Some(&dirs), cx));
        assert_eq!(
            (settings::body_font_size(), settings::body_row_height()),
            (20.0, 40.0)
        );
        // Formulas: the built-in 1.2 times font.math_scale; a formula block's rows follow.
        assert_eq!(crate::text_editor::math::scale(), 1.2);
        let rows = |s| crate::text_editor::math::block_rows((1.0, 0.5), 20.0, s);
        let before_rows = rows(40.0);
        write(&dirs, "[font]\nmath_scale = 2\n");
        cx.update(|cx| TextSettings::reload(Some(&dirs), cx));
        assert_eq!(settings::math_scale(), 2.0);
        assert_eq!(crate::text_editor::math::scale(), 2.4);
        assert!(rows(40.0) > before_rows);
        // Back to the defaults so no other test sees this file's numbers.
        write(&dirs, "");
        cx.update(|cx| TextSettings::reload(Some(&dirs), cx));
        assert_eq!(
            (settings::body_font_size(), settings::body_row_height()),
            (16.0, 26.0)
        );
    }

    #[gpui_kit::test]
    fn a_missing_global_means_the_defaults(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert!(!TextSettings::justify(cx));
            assert_eq!(TextSettings::typography(cx), Typography::default());
        });
    }
}
