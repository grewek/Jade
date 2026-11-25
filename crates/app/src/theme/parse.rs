//! Parse, validate and resolve theme files. Every problem found is
//! collected, so a theme author sees all of them in one pass.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::contrast::{self, ContrastWarning};
use super::roles::{Alpha, Role};

pub const FORMAT_VERSION: u32 = 1;
/// Theme files are tiny; the cap bounds work on a hostile or wrong file.
pub const MAX_THEME_BYTES: usize = 64 * 1024;
pub const MAX_PALETTE_ENTRIES: usize = 256;
const MAX_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// `#rrggbb` or `#rrggbbaa` (alpha 255 when absent).
    pub(super) fn parse_hex_alpha(s: &str) -> Option<(Rgb, u8)> {
        match s.len() {
            7 => Some((Rgb::parse_hex(s)?, 255)),
            9 => {
                let rgb = Rgb::parse_hex(&s[..7])?;
                let alpha = s.get(7..)?;
                let ok = alpha.bytes().all(|b| b.is_ascii_hexdigit());
                ok.then(|| u8::from_str_radix(alpha, 16).ok())
                    .flatten()
                    .map(|a| (rgb, a))
            }
            _ => None,
        }
    }

    /// Exactly `#rrggbb`.
    fn parse_hex(s: &str) -> Option<Rgb> {
        let digits = s.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
        Some(Rgb {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ThemeError {
    TooLarge {
        bytes: usize,
    },
    /// TOML syntax, wrong types, unknown top-level keys; carries line context.
    Syntax(String),
    FormatVersion {
        found: u32,
    },
    EmptyName,
    NameTooLong,
    TooManyPaletteEntries {
        count: usize,
    },
    BadHex {
        place: String,
        value: String,
    },
    UnknownRole {
        section: &'static str,
        name: String,
    },
    UnknownAlpha {
        name: String,
    },
    BadAlpha {
        name: String,
        value: f32,
    },
    /// Roles resolve to a name the palette does not define; one error per
    /// name, so a missing color is not reported once per role using it.
    UnknownPaletteName {
        name: String,
        roles: Vec<Role>,
    },
    /// `roles.toml` itself lacks a role or alpha.
    MissingDefault {
        name: &'static str,
    },
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => {
                write!(f, "file is {bytes} bytes, the limit is {MAX_THEME_BYTES}")
            }
            Self::Syntax(msg) => write!(f, "{}", msg.trim_end()),
            Self::FormatVersion { found } => {
                write!(
                    f,
                    "format_version {found} is not supported (expected {FORMAT_VERSION})"
                )
            }
            Self::EmptyName => write!(f, "name is empty"),
            Self::NameTooLong => write!(f, "name is longer than {MAX_NAME_BYTES} bytes"),
            Self::TooManyPaletteEntries { count } => {
                write!(
                    f,
                    "palette has {count} entries, the limit is {MAX_PALETTE_ENTRIES}"
                )
            }
            Self::BadHex { place, value } => {
                write!(f, "{place}: {value:?} is not a #rrggbb color")
            }
            Self::UnknownRole { section, name } => {
                write!(f, "[{section}] has unknown role {name:?}")
            }
            Self::UnknownAlpha { name } => write!(f, "[alpha] has unknown key {name:?}"),
            Self::BadAlpha { name, value } => {
                write!(f, "alpha {name} = {value} is outside 0.0..=1.0")
            }
            Self::UnknownPaletteName { name, roles } => {
                let used_by: Vec<&str> = roles.iter().map(|r| r.name()).collect();
                write!(
                    f,
                    "palette does not define {name:?} (used by {}); define it or override those roles",
                    used_by.join(", ")
                )
            }
            Self::MissingDefault { name } => write!(f, "roles.toml does not define {name:?}"),
        }
    }
}

impl std::error::Error for ThemeError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsFile {
    format_version: u32,
    roles: BTreeMap<String, String>,
    alpha: BTreeMap<String, f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    format_version: u32,
    name: String,
    kind: Kind,
    palette: BTreeMap<String, String>,
    #[serde(default)]
    overrides: BTreeMap<String, String>,
    #[serde(default)]
    alpha: BTreeMap<String, f32>,
}

fn parse_toml<T: DeserializeOwned>(src: &str, errors: &mut Vec<ThemeError>) -> Option<T> {
    if src.len() > MAX_THEME_BYTES {
        errors.push(ThemeError::TooLarge { bytes: src.len() });
        return None;
    }
    match toml::from_str(src) {
        Ok(value) => Some(value),
        Err(e) => {
            errors.push(ThemeError::Syntax(e.to_string()));
            None
        }
    }
}

fn check_version(found: u32, errors: &mut Vec<ThemeError>) {
    if found != FORMAT_VERSION {
        errors.push(ThemeError::FormatVersion { found });
    }
}

fn check_alpha(name: &str, value: f32, errors: &mut Vec<ThemeError>) -> Option<Alpha> {
    let Some(alpha) = Alpha::from_name(name) else {
        errors.push(ThemeError::UnknownAlpha {
            name: name.to_owned(),
        });
        return None;
    };
    if !(0.0..=1.0).contains(&value) {
        errors.push(ThemeError::BadAlpha {
            name: name.to_owned(),
            value,
        });
        return None;
    }
    Some(alpha)
}

/// The shared role assignments from `roles.toml`.
#[derive(Clone, Debug)]
pub struct Defaults {
    /// Per role: a palette name or `#rrggbb`; `None` for a role that takes its
    /// [`Role::fallback`].
    roles: Vec<Option<String>>,
    alphas: [f32; Alpha::COUNT],
}

impl Defaults {
    /// Whether roles.toml names a value for `role` itself.
    #[cfg(test)]
    pub(super) fn assigns(&self, role: Role) -> bool {
        self.roles[role.index()].is_some()
    }

    pub fn parse(src: &str) -> Result<Defaults, Vec<ThemeError>> {
        let mut errors = Vec::new();
        let Some(file) = parse_toml::<DefaultsFile>(src, &mut errors) else {
            return Err(errors);
        };
        check_version(file.format_version, &mut errors);
        let mut roles: Vec<Option<String>> = vec![None; Role::COUNT];
        for (name, value) in file.roles {
            match Role::from_name(&name) {
                Some(role) => roles[role.index()] = Some(value),
                None => errors.push(ThemeError::UnknownRole {
                    section: "roles",
                    name,
                }),
            }
        }
        let mut alphas = [None; Alpha::COUNT];
        for (name, value) in file.alpha {
            if let Some(alpha) = check_alpha(&name, value, &mut errors) {
                alphas[alpha.index()] = Some(value);
            }
        }
        for role in Role::ALL {
            if roles[role.index()].is_none() && role.fallback().is_none() {
                errors.push(ThemeError::MissingDefault { name: role.name() });
            }
        }
        for alpha in Alpha::ALL {
            if alphas[alpha.index()].is_none() {
                errors.push(ThemeError::MissingDefault { name: alpha.name() });
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        assert!(alphas.iter().all(Option::is_some));
        Ok(Defaults {
            roles,
            alphas: alphas.map(|a| a.unwrap_or(0.0)),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedTheme {
    pub name: String,
    pub kind: Kind,
    colors: [Rgb; Role::COUNT],
    alphas: [f32; Alpha::COUNT],
    pub warnings: Vec<ContrastWarning>,
}

impl ResolvedTheme {
    pub fn color(&self, role: Role) -> Rgb {
        self.colors[role.index()]
    }

    pub(super) fn colors(&self) -> &[Rgb; Role::COUNT] {
        &self.colors
    }

    /// The same theme with other colors; the warnings follow the new ones.
    pub(super) fn with_colors(&self, colors: [Rgb; Role::COUNT]) -> ResolvedTheme {
        ResolvedTheme {
            warnings: contrast::check(&colors),
            colors,
            ..self.clone()
        }
    }

    pub fn alpha(&self, alpha: Alpha) -> f32 {
        self.alphas[alpha.index()]
    }

    /// Merge a theme file over `defaults` and resolve every role to a color.
    pub fn parse(defaults: &Defaults, src: &str) -> Result<ResolvedTheme, Vec<ThemeError>> {
        let mut errors = Vec::new();
        let Some(file) = parse_toml::<ThemeFile>(src, &mut errors) else {
            return Err(errors);
        };
        check_version(file.format_version, &mut errors);
        if file.name.trim().is_empty() {
            errors.push(ThemeError::EmptyName);
        } else if file.name.len() > MAX_NAME_BYTES {
            errors.push(ThemeError::NameTooLong);
        }
        let palette = parse_palette(&file.palette, &mut errors);
        let alphas = merge_alphas(defaults, &file.alpha, &mut errors);
        let colors = resolve_roles(defaults, &file.overrides, &palette, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        let warnings = contrast::check(&colors);
        Ok(ResolvedTheme {
            name: file.name,
            kind: file.kind,
            colors,
            alphas,
            warnings,
        })
    }
}

fn parse_palette(
    raw: &BTreeMap<String, String>,
    errors: &mut Vec<ThemeError>,
) -> BTreeMap<String, Rgb> {
    if raw.len() > MAX_PALETTE_ENTRIES {
        errors.push(ThemeError::TooManyPaletteEntries { count: raw.len() });
        return BTreeMap::new();
    }
    let mut palette = BTreeMap::new();
    for (name, value) in raw {
        match Rgb::parse_hex(value) {
            Some(rgb) => {
                palette.insert(name.clone(), rgb);
            }
            None => errors.push(ThemeError::BadHex {
                place: format!("palette.{name}"),
                value: value.clone(),
            }),
        }
    }
    palette
}

fn merge_alphas(
    defaults: &Defaults,
    overrides: &BTreeMap<String, f32>,
    errors: &mut Vec<ThemeError>,
) -> [f32; Alpha::COUNT] {
    let mut alphas = defaults.alphas;
    for (name, &value) in overrides {
        if let Some(alpha) = check_alpha(name, value, errors) {
            alphas[alpha.index()] = value;
        }
    }
    alphas
}

/// What `role` is assigned: its own value, else its fallback's (chains are
/// short; the bound makes a cycle in `Role::fallback` fail loudly).
fn chosen_value<'a>(role: Role, set: &[Option<&'a str>]) -> &'a str {
    let mut current = role;
    for _ in 0..Role::COUNT {
        if let Some(value) = set[current.index()] {
            return value;
        }
        current = current
            .fallback()
            .unwrap_or_else(|| panic!("role {} has no value and no fallback", current.name()));
    }
    panic!("the fallbacks of role {} form a cycle", role.name());
}

fn resolve_roles(
    defaults: &Defaults,
    overrides: &BTreeMap<String, String>,
    palette: &BTreeMap<String, Rgb>,
    errors: &mut Vec<ThemeError>,
) -> [Rgb; Role::COUNT] {
    let mut set: Vec<Option<&str>> = defaults.roles.iter().map(|r| r.as_deref()).collect();
    for (name, value) in overrides {
        match Role::from_name(name) {
            Some(role) => set[role.index()] = Some(value),
            None => errors.push(ThemeError::UnknownRole {
                section: "overrides",
                name: name.clone(),
            }),
        }
    }
    let chosen: Vec<&str> = Role::ALL
        .iter()
        .map(|&role| chosen_value(role, &set))
        .collect();
    let mut colors = [Rgb { r: 0, g: 0, b: 0 }; Role::COUNT];
    let mut missing: BTreeMap<&str, Vec<Role>> = BTreeMap::new();
    for &role in Role::ALL {
        let value = chosen[role.index()];
        if value.starts_with('#') {
            match Rgb::parse_hex(value) {
                Some(rgb) => colors[role.index()] = rgb,
                None => errors.push(ThemeError::BadHex {
                    place: role.name().to_owned(),
                    value: value.to_owned(),
                }),
            }
        } else if let Some(&rgb) = palette.get(value) {
            colors[role.index()] = rgb;
        } else {
            missing.entry(value).or_default().push(role);
        }
    }
    for (name, roles) in missing {
        errors.push(ThemeError::UnknownPaletteName {
            name: name.to_owned(),
            roles,
        });
    }
    colors
}
