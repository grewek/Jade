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

/// A note's identity and location. Deliberately holds no text: note text
/// lives on disk (read on demand) and, while a note is open, in the editor,
/// so there is only ever one in-memory copy.
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
}

impl Note {
    /// The note at `path`, without touching the filesystem.
    pub fn new(vault_root: &Path, path: PathBuf) -> Self {
        let id = relative_id(vault_root, &path);
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        Self { id, path, title }
    }

    /// Reads the note's current text from disk.
    pub fn read_content(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.path)
    }

    /// Like [`Self::read_content`], into a caller-owned buffer (cleared
    /// first) so scanning many notes reuses one allocation.
    pub fn read_content_into(&self, buf: &mut String) -> std::io::Result<()> {
        use std::io::Read;
        buf.clear();
        std::fs::File::open(&self.path)?.read_to_string(buf)?;
        Ok(())
    }

    /// Writes `text` as the note's content.
    pub fn save(&self, text: &str) -> std::io::Result<()> {
        std::fs::write(&self.path, text)
    }
}
