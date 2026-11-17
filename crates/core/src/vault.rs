use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::links::{link_targets, link_targets_from_file};
use crate::note::{Note, relative_id, relative_path_id};

pub struct Vault {
    pub root: PathBuf,
    pub notes: HashMap<String, Note>,
    /// Folder ids (relative paths, `/`-separated), including empty folders.
    pub folders: HashSet<String>,
    /// Each note's distinct wikilink targets (sorted), cached and kept in sync
    /// with `notes` on every mutation. Note text is not kept in memory (see
    /// [`Note`]), so this is the only way `backlinks()` can know what a note
    /// links to. Targets only: headings and aliases are never read here.
    wikilinks: HashMap<String, Box<[Box<str>]>>,
}

impl Vault {
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        let mut notes = HashMap::new();
        let mut folders = HashSet::new();
        let mut wikilinks = HashMap::new();
        collect_entries(&root, &root, &mut notes, &mut folders, &mut wikilinks)?;
        Ok(Self {
            root,
            notes,
            folders,
            wikilinks,
        })
    }

    /// A vault with no notes or folders yet, e.g. as a fallback when `open` fails.
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            notes: HashMap::new(),
            folders: HashSet::new(),
            wikilinks: HashMap::new(),
        }
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
        let note = Note::new(&self.root, path);
        let id = note.id.clone();
        self.wikilinks.insert(id.clone(), Box::default());
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

    /// The note's current text, read from disk. The caller owns the only
    /// in-memory copy.
    pub fn read_note(&self, id: &str) -> std::io::Result<String> {
        self.notes
            .get(id)
            .ok_or_else(|| not_found(id))?
            .read_content()
    }

    /// Writes `text` to the note's file and refreshes its cached wikilinks
    /// from it. Takes a borrow so the editor's text is never copied to save.
    pub fn save_note(&mut self, id: &str, text: &str) -> std::io::Result<()> {
        let note = self.notes.get(id).ok_or_else(|| not_found(id))?;
        note.save(text)?;
        self.wikilinks.insert(id.to_string(), link_targets(text));
        Ok(())
    }

    /// Renames a note's underlying file (keeping it in the same folder) and returns its new id.
    /// Dedupes against existing titles the same way `create_note` does.
    pub fn rename_note(&mut self, id: &str, new_title: &str) -> std::io::Result<String> {
        let title = self.free_title(id, new_title)?;
        self.rename_note_to(id, &title)
    }

    /// `new_title`, or the first of `new_title 1`, `new_title 2`, ... not
    /// already taken in `id`'s folder by another note.
    pub(crate) fn free_title(&self, id: &str, new_title: &str) -> std::io::Result<String> {
        let old_path = self
            .notes
            .get(id)
            .ok_or_else(|| not_found(id))?
            .path
            .clone();
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
        Ok(candidate)
    }

    /// Renames the note's file to exactly `title` (which [`Self::free_title`]
    /// has checked) and returns its new id.
    pub(crate) fn rename_note_to(&mut self, id: &str, title: &str) -> std::io::Result<String> {
        let old_path = self
            .notes
            .get(id)
            .ok_or_else(|| not_found(id))?
            .path
            .clone();
        let parent = old_path.parent().unwrap_or(&self.root).to_path_buf();
        let new_path = parent.join(format!("{title}.md"));
        std::fs::rename(&old_path, &new_path)?;

        self.notes.remove(id);
        let note = Note::new(&self.root, new_path);
        let new_id = note.id.clone();
        self.notes.insert(new_id.clone(), note);
        // Content is unchanged by a rename, so move the cached links to the new
        // key instead of re-parsing.
        if let Some(links) = self.wikilinks.remove(id) {
            self.wikilinks.insert(new_id.clone(), links);
        }
        Ok(new_id)
    }

    /// Deletes a note's underlying file and removes it from the vault.
    pub fn delete_note(&mut self, id: &str) -> std::io::Result<()> {
        let path = self
            .notes
            .get(id)
            .ok_or_else(|| not_found(id))?
            .path
            .clone();
        std::fs::remove_file(&path)?;
        self.notes.remove(id);
        self.wikilinks.remove(id);
        Ok(())
    }

    /// Renames a folder's underlying directory (keeping it in the same parent, moving
    /// everything inside it along with it) and returns its new id. Dedupes against
    /// sibling folder names the same way `create_folder` does.
    pub fn rename_folder(&mut self, id: &str, new_name: &str) -> std::io::Result<String> {
        let name = self.free_folder_name(id, new_name)?;
        self.rename_folder_to(id, &name)
    }

    /// `new_name`, or the first of `new_name 1`, `new_name 2`, ... not already
    /// a sibling of folder `id`.
    pub(crate) fn free_folder_name(&self, id: &str, new_name: &str) -> std::io::Result<String> {
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
        Ok(candidate)
    }

    /// Renames folder `id` to exactly `name` (checked by
    /// [`Self::free_folder_name`]) and returns its new id.
    pub(crate) fn rename_folder_to(&mut self, id: &str, name: &str) -> std::io::Result<String> {
        if !self.folders.contains(id) {
            return Err(folder_not_found(id));
        }
        let old_path = self.root.join(id);
        let parent_dir = old_path.parent().unwrap_or(&self.root).to_path_buf();
        let new_path = parent_dir.join(name);
        std::fs::rename(&old_path, &new_path)?;

        // The directory (and everything inside it) already moved on disk; drop the
        // stale in-memory entries and re-scan the moved subtree to rebuild them under
        // their new ids, rather than hand-rewriting id prefixes for every nested
        // note/subfolder.
        let old_prefix = format!("{id}/");
        self.folders
            .retain(|f| f != id && !f.starts_with(&old_prefix));
        self.notes
            .retain(|note_id, _| !note_id.starts_with(&old_prefix));
        self.wikilinks
            .retain(|note_id, _| !note_id.starts_with(&old_prefix));

        let new_id = relative_path_id(&self.root, &new_path);
        self.folders.insert(new_id.clone());
        // Re-scanning (which also re-extracts the links) rather than remapping the
        // old prefix to the new one key by key is simpler for arbitrarily nested
        // children — this only runs on a user-initiated folder rename.
        collect_entries(
            &self.root,
            &new_path,
            &mut self.notes,
            &mut self.folders,
            &mut self.wikilinks,
        )?;
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
        self.notes
            .retain(|note_id, _| !note_id.starts_with(&prefix));
        self.wikilinks
            .retain(|note_id, _| !note_id.starts_with(&prefix));
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
        // Reads the cached `wikilinks` map instead of re-running `extract_wikilinks`
        // on every note's content here — that cache is what turns this from a full
        // re-parse of the whole vault into a cheap read on every keystroke.
        for (note_id, targets) in &self.wikilinks {
            for target in targets.iter() {
                if let Some(target_id) = self.resolve(target, &index) {
                    backlinks
                        .entry(target_id)
                        .or_default()
                        .push(note_id.clone());
                }
            }
        }
        // Two targets can resolve to one note (`[[a]]`, `[[A]]`): list each
        // source once.
        for sources in backlinks.values_mut() {
            sources.sort_unstable();
            sources.dedup();
        }
        backlinks
    }

    /// Maps each note's lowercased title and id to its id, for `resolve`'s
    /// case-insensitive fallback. When several notes share a key, the
    /// preferred one wins (see [`preferred`]), so the answer does not depend
    /// on `HashMap` order.
    pub(crate) fn title_index(&self) -> HashMap<String, String> {
        let mut index: HashMap<String, String> = HashMap::with_capacity(self.notes.len() * 2);
        for note in self.notes.values() {
            for key in [note.title.to_lowercase(), note.id.to_lowercase()] {
                match index.entry(key) {
                    Entry::Occupied(mut slot) => {
                        if preferred(&note.id, slot.get()) {
                            slot.insert(note.id.clone());
                        }
                    }
                    Entry::Vacant(slot) => {
                        slot.insert(note.id.clone());
                    }
                }
            }
        }
        index
    }

    pub(crate) fn resolve(&self, target: &str, index: &HashMap<String, String>) -> Option<String> {
        let target = without_md(target);
        if self.notes.contains_key(target) {
            return Some(target.to_string());
        }
        index.get(&target.to_lowercase()).cloned()
    }

    /// The id of the note a wikilink `target` names: an exact id, else a
    /// case-insensitive match on id or title (`[[note]]` finds `Note`,
    /// `[[Note.md]]` finds `Note`). Among several matches the shortest id
    /// wins, then the alphabetically first. Scans the notes, so call it on a
    /// click, not per frame; `backlinks` uses the same rule.
    pub fn resolve_wikilink(&self, target: &str) -> Option<&str> {
        let target = without_md(target);
        if let Some(note) = self.notes.get(target) {
            return Some(&note.id);
        }
        let want = target.to_lowercase();
        self.notes
            .values()
            .filter(|n| n.title.to_lowercase() == want || n.id.to_lowercase() == want)
            .map(|n| n.id.as_str())
            .reduce(|best, id| if preferred(id, best) { id } else { best })
    }

    /// Notes whose title or on-disk text contains `query` (case-insensitive).
    /// Reads files, so call it when the query or notes change, not per frame.
    /// A note that can't be read matches on its title only.
    pub fn search(&self, query: &str) -> Vec<&Note> {
        let query = query.to_lowercase();
        let mut scratch = SearchScratch::default();
        self.notes
            .values()
            .filter(|n| note_matches(&n.title, &n.path, &query, &mut scratch))
            .collect()
    }

    /// An owned copy of what [`search_targets`] needs from every note, so a
    /// search can run on another thread while the vault keeps changing.
    /// Costs three small allocations per note.
    pub fn search_targets(&self) -> Vec<SearchTarget> {
        self.notes
            .values()
            .map(|n| SearchTarget {
                id: n.id.clone(),
                title: n.title.clone(),
                path: n.path.clone(),
            })
            .collect()
    }
}

/// A note as a search sees it. Owned (`Send`) so [`search_targets`] can run
/// off the UI thread.
#[derive(Debug, Clone)]
pub struct SearchTarget {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
}

/// [`Vault::search`] over a snapshot: the `(id, title)` of each matching
/// target, in snapshot order.
pub fn search_targets(targets: &[SearchTarget], query: &str) -> Vec<(String, String)> {
    let query = query.to_lowercase();
    let mut scratch = SearchScratch::default();
    targets
        .iter()
        .filter(|t| note_matches(&t.title, &t.path, &query, &mut scratch))
        .map(|t| (t.id.clone(), t.title.clone()))
        .collect()
}

/// One reused pair of buffers for a whole search, not a fresh text and
/// lowercase copy per note.
#[derive(Default)]
struct SearchScratch {
    text: String,
    lowered: String,
}

fn note_matches(title: &str, path: &Path, query_lower: &str, scratch: &mut SearchScratch) -> bool {
    use std::io::Read;
    title.to_lowercase().contains(query_lower) || {
        scratch.text.clear();
        std::fs::File::open(path)
            .and_then(|mut f| f.read_to_string(&mut scratch.text))
            .is_ok()
            && text_contains(&scratch.text, query_lower, &mut scratch.lowered)
    }
}

/// Whether `text` contains `query_lower` (already lowercased), ignoring case.
/// An ASCII query is matched on the raw bytes with ASCII case folding: no
/// lowercase copy of the text, and it stops at the first hit. (That folds
/// only ASCII letters in the text, so e.g. the Kelvin sign U+212A doesn't
/// match "k".) Other queries lowercase the text into `scratch` first, for
/// full Unicode folding.
fn text_contains(text: &str, query_lower: &str, scratch: &mut String) -> bool {
    if query_lower.is_ascii() {
        return contains_ascii_ignore_case(text.as_bytes(), query_lower.as_bytes());
    }
    scratch.clear();
    scratch.extend(text.chars().flat_map(char::to_lowercase));
    scratch.contains(query_lower)
}

/// `needle` must be lowercase ASCII. Bytes of a multi-byte UTF-8 character
/// are all >= 0x80, so they can never match an ASCII needle byte.
fn contains_ascii_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else {
        return true;
    };
    let first_upper = first.to_ascii_uppercase();
    let last_start = match haystack.len().checked_sub(needle.len()) {
        Some(last) => last,
        None => return false,
    };
    let mut i = 0;
    while i <= last_start {
        let Some(offset) = haystack[i..=last_start]
            .iter()
            .position(|&b| b == first || b == first_upper)
        else {
            return false;
        };
        i += offset;
        if haystack[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether note id `a` beats `b` when a link matches both: shorter first, then
/// alphabetical.
fn preferred(a: &str, b: &str) -> bool {
    (a.len(), a) < (b.len(), b)
}

/// `target` without a trailing `.md`, which a link may carry.
fn without_md(target: &str) -> &str {
    match target.len().checked_sub(3) {
        Some(at) if target.is_char_boundary(at) && target[at..].eq_ignore_ascii_case(".md") => {
            &target[..at]
        }
        _ => target,
    }
}

fn not_found(id: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("note not found: {id}"),
    )
}

fn folder_not_found(id: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("folder not found: {id}"),
    )
}

/// Scans `dir` recursively into `notes`/`folders`, streaming each note
/// line by line to extract its wikilinks into `wikilinks` (no whole-file buffer).
fn collect_entries(
    root: &Path,
    dir: &Path,
    notes: &mut HashMap<String, Note>,
    folders: &mut HashSet<String>,
    wikilinks: &mut HashMap<String, Box<[Box<str>]>>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            folders.insert(relative_path_id(root, &path));
            collect_entries(root, &path, notes, folders, wikilinks)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let note = Note::new(root, path);
            wikilinks.insert(note.id.clone(), link_targets_from_file(&note.path)?);
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
        let b_backlinks = backlinks
            .get("Note B")
            .expect("Note B should have backlinks");
        assert_eq!(b_backlinks, &vec!["Note A".to_string()]);
    }

    /// Regression test for the wikilink cache `backlinks()` reads from: a note
    /// that gains a link *after* the vault was opened must still show up, not
    /// just links present at `Vault::open` time.
    #[test]
    fn backlinks_reflect_edits_made_after_the_vault_was_opened() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "no links yet").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "unrelated").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        assert!(!vault.backlinks().contains_key("Note B"));

        vault
            .save_note("Note A", "Links to [[Note B]] now.")
            .unwrap();

        let backlinks = vault.backlinks();
        assert_eq!(backlinks.get("Note B"), Some(&vec!["Note A".to_string()]));
    }

    #[test]
    fn backlinks_still_resolve_after_the_linking_note_is_renamed() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Target.md"), "content").unwrap();
        std::fs::write(dir.path().join("Source.md"), "Links to [[Target]].").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        let new_id = vault.rename_note("Source", "Renamed Source").unwrap();

        let backlinks = vault.backlinks();
        assert_eq!(backlinks.get("Target"), Some(&vec![new_id]));
    }

    #[test]
    fn deleted_notes_no_longer_appear_in_backlinks() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Target.md"), "content").unwrap();
        std::fs::write(dir.path().join("Source.md"), "Links to [[Target]].").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault.delete_note("Source").unwrap();

        assert!(!vault.backlinks().contains_key("Target"));
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
        assert_eq!(vault.read_note("Math/Algebra").unwrap(), "x + y");
    }

    #[test]
    fn save_note_persists_to_disk() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "old content").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault.save_note("Note", "new content").unwrap();

        assert_eq!(vault.read_note("Note").unwrap(), "new content");
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
        assert_eq!(vault.read_note("New Name").unwrap(), "content");
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
        assert_eq!(vault.read_note("Mathematics/Algebra").unwrap(), "x + y");
    }

    #[test]
    fn backlinks_still_resolve_after_the_linking_notes_folder_is_renamed() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Target.md"), "content").unwrap();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(
            dir.path().join("Math").join("Source.md"),
            "Links to [[Target]].",
        )
        .unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();

        vault.rename_folder("Math", "Mathematics").unwrap();

        let backlinks = vault.backlinks();
        assert_eq!(
            backlinks.get("Target"),
            Some(&vec!["Mathematics/Source".to_string()])
        );
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

    /// The ASCII fast path must agree with plain `to_lowercase().contains`
    /// everywhere except the documented exotic-fold characters, which the
    /// random alphabet below deliberately leaves out.
    #[test]
    fn ascii_search_matches_the_lowercase_contains_reference() {
        let mut seed: u64 = 0xBADC_0FFE_E0DD_F00D;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let alphabet: Vec<char> = "abAB cdCD\n|[]é中🙂".chars().collect();
        let mut scratch = String::new();
        for _ in 0..20_000 {
            let text: String = (0..next() % 40)
                .map(|_| alphabet[next() % alphabet.len()])
                .collect();
            let query: String = (0..1 + next() % 4)
                .map(|_| alphabet[next() % 9])
                .collect::<String>()
                .to_lowercase();
            assert_eq!(
                text_contains(&text, &query, &mut scratch),
                text.to_lowercase().contains(&query),
                "text {text:?} query {query:?}"
            );
        }
    }

    #[test]
    fn non_ascii_search_uses_full_unicode_case_folding() {
        let mut scratch = String::new();
        assert!(text_contains("Große Straße", "straße", &mut scratch));
        assert!(text_contains("ÉCOLE", "école", &mut scratch));
        assert!(!text_contains("ecole", "école", &mut scratch));
    }

    #[test]
    fn search_reads_the_current_text_from_disk() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "nothing special").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        assert!(vault.search("zebra").is_empty());

        // The vault keeps no copy of the text, so a change on disk is seen.
        std::fs::write(dir.path().join("Note.md"), "A Zebra appears").unwrap();
        let found = vault.search("zebra");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "Note");
    }

    #[test]
    fn search_still_matches_the_title_of_an_unreadable_note() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Gone.md"), "text").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join("Gone.md")).unwrap();

        assert_eq!(vault.search("gone").len(), 1, "title match needs no read");
        assert!(vault.search("text").is_empty(), "content can't be read");
    }

    #[test]
    fn read_note_reports_not_found_for_an_unknown_id() {
        let vault = Vault::empty(temp_vault().path());
        let err = vault.read_note("Nope").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn save_note_refreshes_the_cached_links_from_the_given_text() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("A.md"), "no links").unwrap();
        std::fs::write(dir.path().join("B.md"), "b").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        assert!(!vault.backlinks().contains_key("B"));

        vault.save_note("A", "now links to [[B]]").unwrap();
        assert_eq!(vault.backlinks().get("B"), Some(&vec!["A".to_string()]));
        vault.save_note("A", "link removed").unwrap();
        assert!(!vault.backlinks().contains_key("B"));
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

    #[test]
    fn a_source_is_listed_once_however_often_it_links() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("A.md"), "[[B]] [[B]] [[b]]\n[[B#x]]").unwrap();
        std::fs::write(dir.path().join("B.md"), "").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        assert_eq!(vault.backlinks().get("B"), Some(&vec!["A".to_string()]));
    }

    #[test]
    fn snapshot_search_matches_vault_search() {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Alpha.md"), "nothing").unwrap();
        std::fs::write(dir.path().join("B.md"), "has ALPHA inside").unwrap();
        std::fs::write(dir.path().join("C.md"), "no match").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        for query in ["alpha", "ALPHA", "zzz", "é"] {
            let mut live: Vec<String> = vault.search(query).iter().map(|n| n.id.clone()).collect();
            let mut snap: Vec<String> = search_targets(&vault.search_targets(), query)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            live.sort();
            snap.sort();
            assert_eq!(live, snap, "{query}");
        }
    }
}
