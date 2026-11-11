use std::path::{Path, PathBuf};

/// `path` relative to `vault_root`, `/`-separated, extension untouched.
pub fn relative_path_id(vault_root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(vault_root).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

/// The id a note at `path` would have, relative to `vault_root` (no extension, `/`-separated).
pub fn relative_id(vault_root: &Path, path: &Path) -> String {
    relative_path_id(vault_root, &path.with_extension(""))
}

#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub content: String,
}

impl Note {
    pub fn load(vault_root: &Path, path: PathBuf) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(&path)?;
        let id = relative_id(vault_root, &path);
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        Ok(Self {
            id,
            path,
            title,
            content,
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::write(&self.path, &self.content)
    }
}
