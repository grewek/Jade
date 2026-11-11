use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::links::extract_wikilinks;
use crate::note::{Note, relative_id, relative_path_id};

pub struct Vault {
    pub root: PathBuf,
    pub notes: HashMap<String, Note>,
    /// Folder ids (relative paths, `/`-separated), including empty folders.
    pub folders: HashSet<String>,
}

impl Vault {
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        let mut notes = HashMap::new();
        let mut folders = HashSet::new();
        collect_entries(&root, &root, &mut notes, &mut folders)?;
        Ok(Self { root, notes, folders })
    }

    pub fn note(&self, id: &str) -> Option<&Note> {
        self.notes.get(id)
    }

    /// Creates a note titled `title` inside `parent` (vault root if `None`),
    /// dedupeing by appending " 1", " 2", ... if that title is already taken there.
    pub fn create_note(&mut self, title: &str, parent: Option<&str>) -> std::io::Result<String> {
        let dir = match parent {
            Some(parent) => self.root.join(parent),
            None => self.root.clone(),
        };
        std::fs::create_dir_all(&dir)?;

        let mut candidate = title.to_string();
        let mut suffix = 1;
        loop {
            let candidate_id = relative_id(&self.root, &dir.join(format!("{candidate}.md")));
            if !self.notes.contains_key(&candidate_id) {
                break;
            }
            candidate = format!("{title} {suffix}");
            suffix += 1;
        }

        let path = dir.join(format!("{candidate}.md"));
        std::fs::write(&path, "")?;
        let note = Note::load(&self.root, path)?;
        let id = note.id.clone();
        self.notes.insert(id.clone(), note);
        Ok(id)
    }

    /// Creates a folder named `name` inside `parent` (vault root if `None`),
    /// dedupeing by appending " 1", " 2", ... if that name is already taken there.
    pub fn create_folder(&mut self, name: &str, parent: Option<&str>) -> std::io::Result<String> {
        let parent_dir = match parent {
            Some(parent) => self.root.join(parent),
            None => self.root.clone(),
        };
        std::fs::create_dir_all(&parent_dir)?;

        let mut candidate = name.to_string();
        let mut suffix = 1;
        loop {
            if !parent_dir.join(&candidate).exists() {
                break;
            }
            candidate = format!("{name} {suffix}");
            suffix += 1;
        }

        let path = parent_dir.join(&candidate);
        std::fs::create_dir_all(&path)?;
        let id = relative_path_id(&self.root, &path);
        self.folders.insert(id.clone());
        Ok(id)
    }

    pub fn update_note_content(&mut self, id: &str, content: String) -> std::io::Result<()> {
        let note = self.notes.get_mut(id).ok_or_else(|| not_found(id))?;
        note.content = content;
        note.save()
    }

    /// Renames a note's underlying file (keeping it in the same folder) and returns its new id.
    /// Dedupes against existing titles the same way `create_note` does.
    pub fn rename_note(&mut self, id: &str, new_title: &str) -> std::io::Result<String> {
        let old_path = self.notes.get(id).ok_or_else(|| not_found(id))?.path.clone();
        let parent = old_path.parent().unwrap_or(&self.root).to_path_buf();

        let mut candidate = new_title.to_string();
        let mut suffix = 1;
        loop {
            let candidate_id = relative_id(&self.root, &parent.join(format!("{candidate}.md")));
            if candidate_id == id || !self.notes.contains_key(&candidate_id) {
                break;
            }
            candidate = format!("{new_title} {suffix}");
            suffix += 1;
        }

        let new_path = parent.join(format!("{candidate}.md"));
        std::fs::rename(&old_path, &new_path)?;

        self.notes.remove(id);
        let note = Note::load(&self.root, new_path)?;
        let new_id = note.id.clone();
        self.notes.insert(new_id.clone(), note);
        Ok(new_id)
    }

    /// Deletes a note's underlying file and removes it from the vault.
    pub fn delete_note(&mut self, id: &str) -> std::io::Result<()> {
        let path = self.notes.get(id).ok_or_else(|| not_found(id))?.path.clone();
        std::fs::remove_file(&path)?;
        self.notes.remove(id);
        Ok(())
    }

    /// Renames a folder's underlying directory (keeping it in the same parent, moving
    /// everything inside it along with it) and returns its new id. Dedupes against
    /// sibling folder names the same way `create_folder` does.
    pub fn rename_folder(&mut self, id: &str, new_name: &str) -> std::io::Result<String> {
        if !self.folders.contains(id) {
            return Err(folder_not_found(id));
        }
        let old_path = self.root.join(id);
        let parent_dir = old_path.parent().unwrap_or(&self.root).to_path_buf();

        let mut candidate = new_name.to_string();
        let mut suffix = 1;
        loop {
            let candidate_path = parent_dir.join(&candidate);
            if candidate_path == old_path || !candidate_path.exists() {
                break;
            }
            candidate = format!("{new_name} {suffix}");
            suffix += 1;
        }

        let new_path = parent_dir.join(&candidate);
        std::fs::rename(&old_path, &new_path)?;

        // The directory (and everything inside it) already moved on disk; drop the
        // stale in-memory entries and re-scan the moved subtree to rebuild them under
        // their new ids, rather than hand-rewriting id prefixes for every nested
        // note/subfolder.
        let old_prefix = format!("{id}/");
        self.folders.retain(|f| f != id && !f.starts_with(&old_prefix));
        self.notes.retain(|note_id, _| !note_id.starts_with(&old_prefix));

        let new_id = relative_path_id(&self.root, &new_path);
        self.folders.insert(new_id.clone());
        collect_entries(&self.root, &new_path, &mut self.notes, &mut self.folders)?;
        Ok(new_id)
    }

    /// Deletes a folder's underlying directory, recursively, along with everything
    /// inside it, and removes it and its contents from the vault.
    pub fn delete_folder(&mut self, id: &str) -> std::io::Result<()> {
        if !self.folders.contains(id) {
            return Err(folder_not_found(id));
        }
        let path = self.root.join(id);
        std::fs::remove_dir_all(&path)?;

        let prefix = format!("{id}/");
        self.folders.retain(|f| f != id && !f.starts_with(&prefix));
        self.notes.retain(|note_id, _| !note_id.starts_with(&prefix));
        Ok(())
    }

    pub fn backlinks(&self) -> HashMap<String, Vec<String>> {
        // Built once per call rather than scanned per wikilink — `resolve`'s
        // fallback used to walk every note for every unresolved link, making this
        // effectively O(notes² × links) on vaults where most links don't match an
        // id exactly (title lookups, case differences). See the benchmark in
        // `benches/vault_operations.rs` for just how far that scaled.
        let index = self.title_index();
        let mut backlinks: HashMap<String, Vec<String>> = HashMap::new();
        for note in self.notes.values() {
            for link in extract_wikilinks(&note.content) {
                if let Some(target_id) = self.resolve(&link.target, &index) {
                    backlinks.entry(target_id).or_default().push(note.id.clone());
                }
            }
        }
        backlinks
    }

    /// Maps each note's lowercased title and id to its id, for `resolve`'s
    /// case-insensitive fallback.
    fn title_index(&self) -> HashMap<String, String> {
        let mut index = HashMap::with_capacity(self.notes.len() * 2);
        for note in self.notes.values() {
            index.entry(note.title.to_lowercase()).or_insert_with(|| note.id.clone());
            index.entry(note.id.to_lowercase()).or_insert_with(|| note.id.clone());
        }
        index
    }

    fn resolve(&self, target: &str, index: &HashMap<String, String>) -> Option<String> {
        if self.notes.contains_key(target) {
            return Some(target.to_string());
        }
        index.get(&target.to_lowercase()).cloned()
    }

    pub fn search(&self, query: &str) -> Vec<&Note> {
        let query = query.to_lowercase();
        self.notes
            .values()
            .filter(|n| {
                n.title.to_lowercase().contains(&query) || n.content.to_lowercase().contains(&query)
            })
            .collect()
    }
}

fn not_found(id: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, format!("note not found: {id}"))
}

fn folder_not_found(id: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, format!("folder not found: {id}"))
}

fn collect_entries(
    root: &Path,
    dir: &Path,
    notes: &mut HashMap<String, Note>,
    folders: &mut HashSet<String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            folders.insert(relative_path_id(root, &path));
            collect_entries(root, &path, notes, folders)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let note = Note::load(root, path)?;
            notes.insert(note.id.clone(), note);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    fn temp_vault() -> TempDir {
        tempdir().unwrap()
    }

    #[test]
    fn loads_notes_and_resolves_backlinks() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "No outgoing links here.").unwrap();

        let vault = Vault::open(dir.path()).unwrap();
        assert_eq!(vault.notes.len(), 2);

        let backlinks = vault.backlinks();
        let b_backlinks = backlinks.get("Note B").expect("Note B should have backlinks");
        assert_eq!(b_backlinks, &vec!["Note A".to_string()]);
    }

    #[test]
    fn create_note_writes_file_and_dedupes_titles() {
        let dir = temp_vault();
        let mut vault = Vault::open(dir.path()).unwrap();

        let id1 = vault.create_note("Untitled", None).unwrap();
        assert_eq!(id1, "Untitled");
        assert!(dir.path().join("Untitled.md").exists());

        let id2 = vault.create_note("Untitled", None).unwrap();
        assert_eq!(id2, "Untitled 1");
        assert!(dir.path().join("Untitled 1.md").exists());

        assert_eq!(vault.notes.len(), 2);
    }

    #[test]
    fn create_folder_creates_dir_and_dedupes_names() {
        let dir = temp_vault();
        let mut vault = Vault::open(dir.path()).unwrap();

        let id1 = vault.create_folder("Math", None).unwrap();
        assert_eq!(id1, "Math");
        assert!(dir.path().join("Math").is_dir());
        assert!(vault.folders.contains("Math"));

        let id2 = vault.create_folder("Math", None).unwrap();
        assert_eq!(id2, "Math 1");
        assert!(dir.path().join("Math 1").is_dir());
    }

    #[test]
    fn create_note_inside_folder_nests_under_it() {
        let dir = temp_vault();
        let mut vault = Vault::open(dir.path()).unwrap();
        vault.create_folder("Math", None).unwrap();

        let id = vault.create_note("Algebra", Some("Math")).unwrap();

        assert_eq!(id, "Math/Algebra");
        assert!(dir.path().join("Math").join("Algebra.md").exists());
        assert_eq!(vault.note("Math/Algebra").unwrap().title, "Algebra");
    }

    #[test]
    fn create_folder_inside_folder_nests_under_it() {
        let dir = temp_vault();
        let mut vault = Vault::open(dir.path()).unwrap();
        vault.create_folder("Math", None).unwrap();

        let id = vault.create_folder("Algebra", Some("Math")).unwrap();

        assert_eq!(id, "Math/Algebra");
        assert!(dir.path().join("Math").join("Algebra").is_dir());
        assert!(vault.folders.contains("Math/Algebra"));
    }

    #[test]
    fn open_discovers_nested_folders_and_notes() {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "x + y").unwrap();
        std::fs::create_dir_all(dir.path().join("Math").join("Empty Subfolder")).unwrap();

        let vault = Vault::open(dir.path()).unwrap();

        assert!(vault.folders.contains("Math"));
        assert!(vault.folders.contains("Math/Empty Subfolder"));
        assert_eq!(vault.note("Math/Algebra").unwrap().content, "x + y");
    }

    #[test]
    fn update_note_content_persists_to_disk() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "old content").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault
            .update_note_content("Note", "new content".to_string())
            .unwrap();

        assert_eq!(vault.note("Note").unwrap().content, "new content");
        let on_disk = std::fs::read_to_string(dir.path().join("Note.md")).unwrap();
        assert_eq!(on_disk, "new content");
    }

    #[test]
    fn rename_note_moves_file_and_updates_id() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Old Name.md"), "content").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        let new_id = vault.rename_note("Old Name", "New Name").unwrap();

        assert_eq!(new_id, "New Name");
        assert!(!dir.path().join("Old Name.md").exists());
        assert!(dir.path().join("New Name.md").exists());
        assert!(vault.note("Old Name").is_none());
        assert_eq!(vault.note("New Name").unwrap().content, "content");
    }

    #[test]
    fn rename_note_dedupes_against_existing_titles() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("A.md"), "a").unwrap();
        std::fs::write(dir.path().join("B.md"), "b").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        let new_id = vault.rename_note("A", "B").unwrap();

        assert_eq!(new_id, "B 1");
        assert!(dir.path().join("B 1.md").exists());
    }

    #[test]
    fn delete_note_removes_file_and_entry() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault.delete_note("Note").unwrap();

        assert!(!dir.path().join("Note.md").exists());
        assert!(vault.note("Note").is_none());
    }

    #[test]
    fn rename_folder_moves_dir_and_updates_nested_ids() {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "x + y").unwrap();
        std::fs::create_dir_all(dir.path().join("Math").join("Geometry")).unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        let new_id = vault.rename_folder("Math", "Mathematics").unwrap();

        assert_eq!(new_id, "Mathematics");
        assert!(!dir.path().join("Math").exists());
        assert!(dir.path().join("Mathematics").join("Algebra.md").exists());

        assert!(!vault.folders.contains("Math"));
        assert!(vault.folders.contains("Mathematics"));
        assert!(vault.folders.contains("Mathematics/Geometry"));
        assert!(vault.note("Math/Algebra").is_none());
        assert_eq!(vault.note("Mathematics/Algebra").unwrap().content, "x + y");
    }

    #[test]
    fn rename_folder_dedupes_against_existing_sibling_names() {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::create_dir_all(dir.path().join("Science")).unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        let new_id = vault.rename_folder("Math", "Science").unwrap();

        assert_eq!(new_id, "Science 1");
        assert!(dir.path().join("Science 1").is_dir());
    }

    #[test]
    fn delete_folder_removes_dir_and_nested_contents() {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();
        std::fs::write(dir.path().join("Other.md"), "untouched").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault.delete_folder("Math").unwrap();

        assert!(!dir.path().join("Math").exists());
        assert!(!vault.folders.contains("Math"));
        assert!(vault.note("Math/Algebra").is_none());
        assert!(vault.note("Other").is_some());
    }

    #[test]
    fn search_matches_title_and_content() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Groceries.md"), "Buy milk and eggs.").unwrap();
        std::fs::write(dir.path().join("Other.md"), "Unrelated content.").unwrap();

        let vault = Vault::open(dir.path()).unwrap();
        let results = vault.search("milk");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Groceries");
    }
}
