//! Choosing the vault folder: which one the app opens and how the path a user
//! types on first start is read and checked. GPUI-free.

use std::fs;
use std::path::{Path, PathBuf};

/// More than any real path; stops a pasted blob from reaching the filesystem.
const MAX_INPUT_BYTES: usize = 4096;

/// The vault to open: the command line wins over `config.toml`. `None` means
/// the vault is not known yet, so the user is asked.
pub fn resolve(cli: Option<PathBuf>, configured: Option<PathBuf>) -> Option<PathBuf> {
    cli.or(configured)
}

/// What the welcome dialog suggests: a `Jade` folder in the user's documents
/// (else home) folder.
pub fn suggestion(documents: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    documents.or(home).map(|dir| dir.join("Jade"))
}

/// The absolute path the user typed; a leading `~` means `home`. Relative
/// paths are refused because they would depend on where the app was started.
pub fn parse_input(text: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a folder for your notes.".to_owned());
    }
    if text.len() > MAX_INPUT_BYTES {
        return Err("That path is too long.".to_owned());
    }
    let path = match text.strip_prefix('~') {
        Some("") => home_or_err(home)?.to_path_buf(),
        Some(rest) if rest.starts_with(['/', '\\']) => home_or_err(home)?.join(&rest[1..]),
        _ => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err("Use an absolute path, for example ~/Notes.".to_owned());
    }
    Ok(path)
}

fn home_or_err(home: Option<&Path>) -> Result<&Path, String> {
    home.ok_or_else(|| "Cannot find your home folder; type the full path.".to_owned())
}

/// Creates the folder when it is missing; a file at that path is refused.
pub fn prepare(path: &Path) -> Result<(), String> {
    if path.is_file() {
        return Err(format!("{} is a file, not a folder.", path.display()));
    }
    fs::create_dir_all(path).map_err(|e| format!("Cannot create {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn the_command_line_wins_over_the_config() {
        assert_eq!(resolve(Some(abs("a")), Some(abs("b"))), Some(abs("a")));
        assert_eq!(resolve(None, Some(abs("b"))), Some(abs("b")));
        assert_eq!(resolve(None, None), None);
    }

    #[test]
    fn the_suggestion_prefers_documents_over_home() {
        let docs = abs("docs");
        let home = abs("home");
        assert_eq!(
            suggestion(Some(docs.clone()), Some(home.clone())),
            Some(docs.join("Jade"))
        );
        assert_eq!(
            suggestion(None, Some(home.clone())),
            Some(home.join("Jade"))
        );
        assert_eq!(suggestion(None, None), None);
    }

    #[test]
    fn typed_paths_are_trimmed_and_tilde_expands_to_home() {
        let home = abs("home");
        let typed = format!("  {}  ", abs("notes").display());
        assert_eq!(parse_input(&typed, Some(&home)), Ok(abs("notes")));
        assert_eq!(parse_input("~", Some(&home)), Ok(home.clone()));
        assert_eq!(parse_input("~/Notes", Some(&home)), Ok(home.join("Notes")));
    }

    #[test]
    fn bad_input_says_why() {
        let home = abs("home");
        assert!(parse_input("   ", Some(&home)).is_err());
        assert!(parse_input("notes/here", Some(&home)).is_err());
        assert!(parse_input("~/Notes", None).is_err());
        assert!(parse_input(&"x".repeat(MAX_INPUT_BYTES + 1), Some(&home)).is_err());
    }

    #[test]
    fn a_tilde_in_a_name_is_not_a_home_folder() {
        let home = abs("home");
        // `~backup` is a relative name, not `~` + path.
        assert!(parse_input("~backup", Some(&home)).is_err());
    }

    #[test]
    fn prepare_creates_missing_folders_and_refuses_files() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        assert_eq!(prepare(&nested), Ok(()));
        assert!(nested.is_dir());
        assert_eq!(prepare(&nested), Ok(()), "an existing folder is fine");

        let file = dir.path().join("note.md");
        fs::write(&file, "x").unwrap();
        assert!(prepare(&file).is_err());
    }
}
