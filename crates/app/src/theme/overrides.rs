//! The user's `[colors]` table in `config.toml` (#58): a color for one role, on
//! top of the active theme and kept when the theme is switched. GPUI-free.
//!
//! A value is `#rrggbb`, `#rrggbbaa`, or the name of another role (that role's
//! *theme* color). A bad entry is skipped with a notice; the rest still apply.
//! A role that only follows another (`heading_3` follows `heading`, see
//! [`Role::fallback`]) follows the user's color too, unless the theme gave it a
//! color of its own.

use toml::{Table, Value};

use super::parse::{ResolvedTheme, Rgb};
use super::paths::ConfigDirs;
use super::roles::Role;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Override {
    Color { rgb: Rgb, alpha: u8 },
    Alias(Role),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorOverrides {
    set: [Option<Override>; Role::COUNT],
}

/// A theme with the overrides applied, and the opacity of each role (255 =
/// opaque) for the palette; widgets ignore opacity.
pub struct Applied {
    pub theme: ResolvedTheme,
    pub alpha: [u8; Role::COUNT],
}

impl Default for ColorOverrides {
    fn default() -> Self {
        ColorOverrides {
            set: [None; Role::COUNT],
        }
    }
}

impl ColorOverrides {
    /// The `[colors]` of a `config.toml` text, and what to tell the user.
    pub fn parse(src: &str) -> (ColorOverrides, Vec<String>) {
        let mut overrides = ColorOverrides::default();
        let mut notices = Vec::new();
        let Ok(root) = src.parse::<Table>() else {
            // The theme load reports a file that does not parse.
            return (overrides, notices);
        };
        let Some(value) = root.get("colors") else {
            return (overrides, notices);
        };
        let Some(table) = value.as_table() else {
            notices.push("config.toml: [colors] is not a table, ignoring it".to_owned());
            return (overrides, notices);
        };
        for (name, value) in table {
            let Some(role) = Role::from_name(name) else {
                notices.push(format!("config.toml: colors.{name} is not a role"));
                continue;
            };
            match parse_value(value) {
                Some(o) => overrides.set[role.index()] = Some(o),
                None => notices.push(format!(
                    "config.toml: colors.{name} must be \"#rrggbb\", \"#rrggbbaa\" or a role name"
                )),
            }
        }
        (overrides, notices)
    }

    /// Reads `config.toml` under `dirs`; none when there is no file.
    pub fn load(dirs: Option<&ConfigDirs>) -> (ColorOverrides, Vec<String>) {
        match super::loader::config_text(dirs) {
            Some(Ok(src)) => ColorOverrides::parse(&src),
            // The theme load reports an unreadable file.
            _ => (ColorOverrides::default(), Vec::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.set.iter().all(Option::is_none)
    }

    /// `theme` with the overrides on top. Contrast warnings are for the merged
    /// colors; callers report only the new ones.
    pub fn apply_to(&self, theme: &ResolvedTheme) -> Applied {
        let mut colors = *theme.colors();
        let mut alpha = [255u8; Role::COUNT];
        for &role in Role::ALL {
            let i = role.index();
            match self.set[i] {
                Some(Override::Color { rgb, alpha: a }) => (colors[i], alpha[i]) = (rgb, a),
                Some(Override::Alias(source)) => colors[i] = theme.color(source),
                None => {
                    // Fallbacks come earlier in `Role::ALL`, so they are final.
                    let follows = role
                        .fallback()
                        .filter(|f| theme.color(*f) == theme.color(role));
                    if let Some(f) = follows {
                        (colors[i], alpha[i]) = (colors[f.index()], alpha[f.index()]);
                    }
                }
            }
        }
        Applied {
            theme: theme.with_colors(colors),
            alpha,
        }
    }
}

/// The warnings `after` has that `before` did not: a theme's own low
/// contrast is reported when it loads.
pub fn new_warnings(before: &ResolvedTheme, after: &ResolvedTheme) -> Vec<String> {
    after
        .warnings
        .iter()
        .filter(|w| {
            !before
                .warnings
                .iter()
                .any(|b| (b.foreground, b.background) == (w.foreground, w.background))
        })
        .map(|w| format!("colors: warning: {w}"))
        .collect()
}

fn parse_value(value: &Value) -> Option<Override> {
    let s = value.as_str()?;
    if s.starts_with('#') {
        let (rgb, alpha) = Rgb::parse_hex_alpha(s)?;
        return Some(Override::Color { rgb, alpha });
    }
    Role::from_name(s).map(Override::Alias)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> (ColorOverrides, Vec<String>) {
        ColorOverrides::parse(src)
    }

    #[test]
    fn nothing_to_override_gives_nothing_and_no_notice() {
        for src in [
            "",
            "vault = \"/v\"\n[font]\nsize = 3\n",
            "[colors]\n",
            "[font\n",
        ] {
            let (o, notices) = parse(src);
            assert!(o.is_empty(), "{src:?}");
            assert!(notices.is_empty(), "{src:?}: {notices:?}");
        }
    }

    #[test]
    fn colors_aliases_and_alpha_are_read() {
        let (o, notices) = parse(
            "[colors]\nheading_5 = \"#c0392b\"\ncode_background = \"#10203040\"\nquote_bar = \"accent\"\n",
        );
        assert!(notices.is_empty(), "{notices:?}");
        let c = |hex: u32| Rgb {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        };
        assert_eq!(
            o.set[Role::Heading5.index()],
            Some(Override::Color {
                rgb: c(0xc0392b),
                alpha: 255
            })
        );
        assert_eq!(
            o.set[Role::CodeBackground.index()],
            Some(Override::Color {
                rgb: c(0x102030),
                alpha: 0x40
            })
        );
        assert_eq!(
            o.set[Role::QuoteBar.index()],
            Some(Override::Alias(Role::Accent))
        );
    }

    #[test]
    fn a_bad_entry_is_skipped_and_the_rest_applies() {
        let (o, notices) = parse(
            "[colors]\nheading = \"red\"\nheading_1 = \"#12345\"\nnot_a_role = \"#ffffff\"\n\
             link = 5\ntext = \"#000000\"\nheading_2 = \"nope\"\ncode = \"#abcdefg0\"\n",
        );
        assert_eq!(notices.len(), 6, "{notices:#?}");
        assert!(o.set[Role::Text.index()].is_some());
        assert_eq!(o.set.iter().flatten().count(), 1);
    }

    #[test]
    fn colors_that_is_not_a_table_is_noticed() {
        let (o, notices) = parse("colors = \"x\"\n");
        assert!(o.is_empty());
        assert_eq!(notices.len(), 1);
        let (_, notices) = parse("[colors.\"Jade Dark\"]\ntext = \"#fff000\"\n");
        assert_eq!(notices.len(), 1, "per-theme tables are a later phase");
    }
}
