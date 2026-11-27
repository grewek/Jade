//! The themes a user can pick and remembering the pick in `config.toml`.
//! GPUI-free.

use std::fs;
use std::path::Path;

use toml::Value;

use super::builtin;
use super::loader;
use super::parse::Defaults;
use super::paths::ConfigDirs;

/// More theme files than this in the folder are ignored; the list is shown
/// in a dialog.
const MAX_USER_THEMES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

/// Built-in themes first, then the user's, by id. A user file with a built-in
/// id replaces that entry, as it does when loading. Files that do not load
/// are left out; reloading reports why.
pub fn available(defaults: &Defaults, dirs: Option<&ConfigDirs>) -> Vec<Choice> {
    let theme_dir = dirs.map(ConfigDirs::theme_dir);
    let mut ids: Vec<String> = builtin::THEMES
        .iter()
        .map(|(id, _)| (*id).to_owned())
        .collect();
    for id in user_ids(theme_dir.as_deref()) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.into_iter()
        .filter_map(|id| {
            let theme = loader::resolve(defaults, theme_dir.as_deref(), &id).ok()?;
            Some(Choice {
                name: theme.name,
                id,
            })
        })
        .collect()
}

fn user_ids(theme_dir: Option<&Path>) -> Vec<String> {
    let Some(entries) = theme_dir.and_then(|dir| fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some(name.strip_suffix(".toml")?.to_owned())
        })
        .filter(|id| loader::valid_id(id))
        .collect();
    ids.sort();
    ids.truncate(MAX_USER_THEMES);
    ids
}

/// Writes `theme = "<id>"` into `config.toml`, keeping its other keys (not
/// its comments). A file that does not parse is left alone: overwriting it
/// would destroy what the user typed.
pub fn remember(dirs: &ConfigDirs, id: &str) -> Result<(), String> {
    remember_value(dirs, "theme", Value::String(id.to_owned()))
}

/// Writes `justify = <on>` into `config.toml`, as `remember` does.
pub fn remember_justify(dirs: &ConfigDirs, on: bool) -> Result<(), String> {
    remember_value(dirs, "justify", Value::Boolean(on))
}

/// Writes `vault = "<path>"` into `config.toml`, as `remember` does.
pub fn remember_vault(dirs: &ConfigDirs, vault: &Path) -> Result<(), String> {
    let text = vault
        .to_str()
        .ok_or_else(|| format!("{}: path is not valid UTF-8", vault.display()))?;
    remember_value(dirs, "vault", Value::String(text.to_owned()))
}

fn remember_value(dirs: &ConfigDirs, key: &str, value: Value) -> Result<(), String> {
    remember_setting(dirs, None, key, &value.to_string())
}

/// Writes `key = <value>` (TOML source) into `[section]` of `config.toml`,
/// or the top of the file for `None`. The file is edited in place, so its
/// comments and layout stay; a file that does not parse is left alone.
pub fn remember_setting(
    dirs: &ConfigDirs,
    section: Option<&str>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let path = dirs.config_file();
    let src = if path.is_file() {
        loader::read_bounded(&path)?
    } else {
        String::new()
    };
    let out = crate::settings::write::set_key(&src, section, key, value)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    fs::create_dir_all(dirs.base()).map_err(|e| format!("{}: {e}", dirs.base().display()))?;
    // Rename is atomic, so the watcher never reads a half-written file.
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}
