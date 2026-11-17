//! Finds, reads and resolves a theme by id. Never panics on user data: every
//! failure becomes a message and the caller falls back to the built-in default.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use super::builtin;
use super::parse::{Defaults, MAX_THEME_BYTES, ResolvedTheme, ThemeError};
use super::paths::ConfigDirs;

const MAX_ID_BYTES: usize = 64;
/// `roles.toml` in the theme folder is not a theme.
const RESERVED_ID: &str = "roles";

pub struct Loaded {
    pub id: String,
    pub theme: ResolvedTheme,
    /// Everything worth telling the user, for stderr.
    pub notices: Vec<String>,
}

/// Only what the engine reads; `config.toml` will grow other keys.
#[derive(Deserialize)]
struct ConfigFile {
    theme: Option<String>,
}

/// Ids become file names, so anything that could leave the theme folder is out.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id != RESERVED_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Reads at most `MAX_THEME_BYTES`; a bigger file is an error, not a big read.
pub(super) fn read_bounded(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    let limit = MAX_THEME_BYTES as u64 + 1;
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() > MAX_THEME_BYTES {
        return Err(format!(
            "{}: {}",
            path.display(),
            ThemeError::TooLarge { bytes: bytes.len() }
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not valid UTF-8", path.display()))
}

fn messages(id: &str, errors: Vec<ThemeError>) -> Vec<String> {
    errors
        .into_iter()
        .map(|e| format!("theme {id}: {e}"))
        .collect()
}

/// A user file `<theme_dir>/<id>.toml` shadows the built-in of the same id.
pub fn resolve(
    defaults: &Defaults,
    theme_dir: Option<&Path>,
    id: &str,
) -> Result<ResolvedTheme, Vec<String>> {
    if !valid_id(id) {
        return Err(vec![format!(
            "theme id {id:?} is not allowed (use letters, digits, - and _)"
        )]);
    }
    let user_file = theme_dir.map(|dir| dir.join(format!("{id}.toml")));
    let src = match user_file.as_deref().filter(|p| p.is_file()) {
        Some(path) => read_bounded(path).map_err(|e| vec![e])?,
        None => builtin::source(id)
            .ok_or_else(|| vec![format!("theme {id:?} not found")])?
            .to_owned(),
    };
    ResolvedTheme::parse(defaults, &src).map_err(|errors| messages(id, errors))
}

fn wanted_id(dirs: Option<&ConfigDirs>, notices: &mut Vec<String>) -> Option<String> {
    let path = dirs?.config_file();
    if !path.is_file() {
        return None;
    }
    let parsed = read_bounded(&path).and_then(|src| {
        toml::from_str::<ConfigFile>(&src).map_err(|e| format!("config.toml: {e}"))
    });
    match parsed {
        Ok(config) => config.theme,
        Err(message) => {
            notices.push(message);
            None
        }
    }
}

/// The theme `config.toml` names (else the built-in default), with no
/// fallback: a reload keeps the active theme when this fails.
pub fn load_checked(defaults: &Defaults, dirs: Option<&ConfigDirs>) -> Result<Loaded, Vec<String>> {
    let mut notices = Vec::new();
    let theme_dir = dirs.map(ConfigDirs::theme_dir);
    let id = wanted_id(dirs, &mut notices);
    if !notices.is_empty() {
        return Err(notices);
    }
    let id = id.unwrap_or_else(|| builtin::DEFAULT_ID.to_owned());
    let theme = resolve(defaults, theme_dir.as_deref(), &id)?;
    notices.extend(
        theme
            .warnings
            .iter()
            .map(|w| format!("theme {id}: warning: {w}")),
    );
    Ok(Loaded { id, theme, notices })
}

/// Startup: the theme `config.toml` names, else (or on any failure) the
/// built-in default. Always returns a usable theme.
pub fn load_startup(defaults: &Defaults, dirs: Option<&ConfigDirs>) -> Loaded {
    match load_checked(defaults, dirs) {
        Ok(loaded) => loaded,
        Err(mut notices) => {
            notices.push(format!(
                "using the built-in {} theme instead",
                builtin::DEFAULT_ID
            ));
            Loaded {
                id: builtin::DEFAULT_ID.to_owned(),
                theme: builtin::default_theme(defaults),
                notices,
            }
        }
    }
}
