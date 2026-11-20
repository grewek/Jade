//! The themes a user can pick and remembering the pick in `config.toml`.
//! GPUI-free.

use std::fs;
use std::path::Path;

use toml::{Table, Value};

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

/// Writes `justify = <on>` into `config.toml`, as [`remember`] does.
pub fn remember_justify(dirs: &ConfigDirs, on: bool) -> Result<(), String> {
    remember_value(dirs, "justify", Value::Boolean(on))
}

fn remember_value(dirs: &ConfigDirs, key: &str, value: Value) -> Result<(), String> {
    let path = dirs.config_file();
    let mut table = if path.is_file() {
        let src = loader::read_bounded(&path)?;
        src.parse::<Table>()
            .map_err(|e| format!("{}: {e}; not changed", path.display()))?
    } else {
        Table::new()
    };
    table.insert(key.to_owned(), value);
    let out = toml::to_string(&table).map_err(|e| e.to_string())?;
    fs::create_dir_all(dirs.base()).map_err(|e| format!("{}: {e}", dirs.base().display()))?;
    // Rename is atomic, so the watcher never reads a half-written file.
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}
