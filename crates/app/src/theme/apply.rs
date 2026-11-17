//! Turns a resolved theme into gpui-component's `ThemeConfig`. Keys not
//! listed in `MAPPING` derive from these in gpui-component (hover and active
//! states, button variants, light base colors); see FORMAT.md for the table.

use gpui_kit::component::ThemeConfig;
use serde::Deserialize;
use toml::{Table, Value};

use super::parse::{Kind, ResolvedTheme, Rgb};
use super::roles::{Alpha, Role};

/// What a gpui-component color key is set from.
#[derive(Clone, Copy)]
pub(super) enum Source {
    Role(Role),
    /// A role at the theme's opacity for an alpha.
    Faded(Role, Alpha),
}

use Source::{Faded, Role as R};

pub(super) const MAPPING: &[(&str, Source)] = &[
    ("background", R(Role::Background)),
    ("popover.background", R(Role::Background)),
    ("accordion.background", R(Role::Background)),
    ("list.background", R(Role::Background)),
    ("list.even.background", R(Role::Background)),
    ("table.background", R(Role::Background)),
    ("table.even.background", R(Role::Background)),
    ("tab.active.background", R(Role::Background)),
    ("switch.thumb.background", R(Role::Background)),
    ("foreground", R(Role::Text)),
    ("popover.foreground", R(Role::Text)),
    ("accent.foreground", R(Role::Text)),
    ("secondary.foreground", R(Role::Text)),
    ("group_box.foreground", R(Role::Text)),
    ("tab.active.foreground", R(Role::Text)),
    ("sidebar.foreground", R(Role::Text)),
    ("border", R(Role::Border)),
    ("input.border", R(Role::Border)),
    ("sidebar.border", R(Role::Border)),
    ("title_bar.border", R(Role::Border)),
    ("status_bar.border", R(Role::Border)),
    ("window.border", R(Role::Border)),
    ("table.row.border", R(Role::Border)),
    ("muted.background", R(Role::Surface)),
    ("secondary.background", R(Role::Surface)),
    ("skeleton.background", R(Role::Surface)),
    ("tab_bar.segmented.background", R(Role::Surface)),
    ("list.hover.background", R(Role::Surface)),
    ("table.hover.background", R(Role::Surface)),
    ("secondary.hover.background", R(Role::SurfaceHover)),
    ("accent.background", R(Role::SurfaceHover)),
    ("list.active.background", R(Role::SurfaceHover)),
    ("table.active.background", R(Role::SurfaceHover)),
    ("sidebar.accent.background", R(Role::SurfaceHover)),
    ("slider.background", R(Role::SurfaceHover)),
    ("switch.background", R(Role::SurfaceHover)),
    ("secondary.active.background", R(Role::SurfaceActive)),
    ("muted.foreground", R(Role::TextMuted)),
    ("tab.foreground", R(Role::TextMuted)),
    ("group_box.title.foreground", R(Role::TextMuted)),
    ("description_list.label.foreground", R(Role::TextMuted)),
    ("table.head.foreground", R(Role::TextMuted)),
    ("table.foot.foreground", R(Role::TextMuted)),
    ("sidebar.accent.foreground", R(Role::Text)),
    ("sidebar.background", R(Role::BackgroundSecondary)),
    ("tab_bar.background", R(Role::BackgroundSecondary)),
    ("title_bar.background", R(Role::BackgroundSecondary)),
    ("status_bar.background", R(Role::BackgroundSecondary)),
    ("list.head.background", R(Role::BackgroundSecondary)),
    ("table.head.background", R(Role::BackgroundSecondary)),
    ("table.foot.background", R(Role::BackgroundSecondary)),
    ("group_box.background", R(Role::BackgroundSecondary)),
    (
        "description_list.label.background",
        R(Role::BackgroundSecondary),
    ),
    ("primary.background", R(Role::Accent)),
    ("ring", R(Role::Accent)),
    ("drag.border", R(Role::Accent)),
    ("list.active.border", R(Role::Accent)),
    ("table.active.border", R(Role::Accent)),
    ("progress.bar.background", R(Role::Accent)),
    ("slider.thumb.background", R(Role::Accent)),
    ("sidebar.primary.background", R(Role::Accent)),
    ("primary.foreground", R(Role::TextOnAccent)),
    ("sidebar.primary.foreground", R(Role::TextOnAccent)),
    ("danger.foreground", R(Role::TextOnAccent)),
    ("success.foreground", R(Role::TextOnAccent)),
    ("info.foreground", R(Role::TextOnAccent)),
    ("warning.foreground", R(Role::TextOnAccent)),
    ("caret", R(Role::Cursor)),
    (
        "selection.background",
        Faded(Role::Selection, Alpha::Selection),
    ),
    (
        "drop_target.background",
        Faded(Role::Accent, Alpha::AccentHover),
    ),
    ("overlay", Faded(Role::Scrim, Alpha::Overlay)),
    ("link", R(Role::Link)),
    ("scrollbar.thumb.background", R(Role::BorderStrong)),
    ("scrollbar.thumb.hover.background", R(Role::TextSubtle)),
    ("base.red", R(Role::Error)),
    ("danger.background", R(Role::Error)),
    ("base.green", R(Role::Success)),
    ("success.background", R(Role::Success)),
    ("base.blue", R(Role::Info)),
    ("info.background", R(Role::Info)),
    ("base.yellow", R(Role::Warning)),
    ("warning.background", R(Role::Warning)),
    ("base.cyan", R(Role::Tip)),
    ("base.magenta", R(Role::Summary)),
];

fn hex(c: Rgb, alpha: Option<f32>) -> String {
    let a = alpha.map_or(String::new(), |a| {
        format!("{:02x}", (a * 255.0).round() as u8)
    });
    format!("#{:02x}{:02x}{:02x}{a}", c.r, c.g, c.b)
}

/// The gpui-component theme for `theme`. `None` only if gpui-component
/// rejects the generated table, which the mapping test rules out.
pub fn build_config(theme: &ResolvedTheme) -> Option<ThemeConfig> {
    let mut colors = Table::new();
    for &(key, from) in MAPPING {
        let value = match from {
            Source::Role(role) => hex(theme.color(role), None),
            Source::Faded(role, alpha) => hex(theme.color(role), Some(theme.alpha(alpha))),
        };
        colors.insert(key.to_owned(), Value::String(value));
    }
    let mut table = Table::new();
    table.insert("name".into(), Value::String(theme.name.clone()));
    let mode = if theme.kind == Kind::Dark {
        "dark"
    } else {
        "light"
    };
    table.insert("mode".into(), Value::String(mode.into()));
    table.insert("colors".into(), Value::Table(colors));
    ThemeConfig::deserialize(table).ok()
}
