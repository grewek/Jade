//! Resolves a link's destination to something the app can open. Wikilinks
//! resolve by name (`Vault::resolve_wikilink`); this module handles the
//! path-style destinations of `[text](dest)` and `<url>`.
//!
//! A note can contain any destination, so the rules are strict: nothing that
//! leaves the vault root is ever returned, symlinks included.

use std::path::PathBuf;

use crate::link_scan::{LinkKind, scheme};
use crate::note_name::split_md;
use crate::vault::Vault;

/// What a written link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination<'a> {
    /// A note in the vault, by id.
    Note(String),
    /// Another file inside the vault (a PDF, an image), by absolute path.
    File(PathBuf),
    /// Has a URL scheme; opening it is the caller's decision.
    External(&'a str),
    /// Only a `#fragment` or `?query`: it points into the note it is written in.
    SameNote,
    /// Nothing the vault has, or outside it.
    Missing,
}

impl Vault {
    /// What the link written as `written` in note `from` points at. The one
    /// place a link's meaning is decided: clicks, broken-link marking,
    /// backlink context and rename rewriting all ask here, so they cannot
    /// disagree. `kind` says how it is written: a wikilink names a note, the
    /// rest are paths or URLs.
    pub fn link_destination<'a>(
        &self,
        from: &str,
        kind: LinkKind,
        written: &'a str,
    ) -> Destination<'a> {
        if kind == LinkKind::Wiki {
            return match self.resolve_wikilink(written) {
                Some(id) => Destination::Note(id.to_string()),
                None => Destination::Missing,
            };
        }
        if let Some(scheme) = scheme(written) {
            return Destination::External(scheme);
        }
        match self.resolve_path_link(from, written) {
            Ok(LinkTarget::Note(id)) => Destination::Note(id),
            Ok(LinkTarget::File(path)) => Destination::File(path),
            Err(PathLinkError::SameNote) => Destination::SameNote,
            Err(PathLinkError::External) => Destination::External(""),
            Err(PathLinkError::OutsideVault | PathLinkError::NotFound) => Destination::Missing,
        }
    }
}

/// A destination longer than this is not a path anyone wrote by hand.
const MAX_DEST_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A note in the vault, by id.
    Note(String),
    /// Another file inside the vault (a PDF, an image), by absolute path.
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathLinkError {
    /// Only a `#fragment` or `?query`: it points into the current note.
    SameNote,
    /// Has a URL scheme (`https:`, `mailto:`); the caller decides what to do.
    External,
    /// Resolves to somewhere outside the vault.
    OutsideVault,
    /// Nothing there, or a directory, or not valid text.
    NotFound,
}

/// What follows the path in a destination: `#heading` or `?query`.
pub fn split_fragment(dest: &str) -> (&str, Option<&str>) {
    match dest.find(['#', '?']) {
        Some(at) => (&dest[..at], Some(&dest[at + 1..]).filter(|f| !f.is_empty())),
        None => (dest, None),
    }
}

/// Decodes `%20`-style escapes. `None` when an escape is malformed or the
/// result is not UTF-8.
pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `base` (a vault-relative directory, `/`-separated) joined with `rel`,
/// with `.` and `..` resolved lexically. `None` if `..` climbs above the root.
fn normalize(base: &str, rel: &str) -> Option<Vec<String>> {
    let mut parts: Vec<String> = base
        .split('/')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other.to_owned()),
        }
    }
    Some(parts)
}

impl Vault {
    /// Resolves the destination of a `[text](dest)` link written in note
    /// `from_note_id`. Relative paths are tried against the note's folder,
    /// then against the vault root (a leading `/` means the root only). A
    /// `.md` path or an extension-less one that names a note gives
    /// [`LinkTarget::Note`]; any other existing file inside the vault gives
    /// [`LinkTarget::File`].
    pub(crate) fn resolve_path_link(
        &self,
        from_note_id: &str,
        dest: &str,
    ) -> Result<LinkTarget, PathLinkError> {
        if dest.len() > MAX_DEST_BYTES {
            return Err(PathLinkError::NotFound);
        }
        if scheme(dest).is_some() {
            return Err(PathLinkError::External);
        }
        let (path, _) = split_fragment(dest);
        if path.is_empty() {
            return Err(PathLinkError::SameNote);
        }
        let decoded = percent_decode(path).ok_or(PathLinkError::NotFound)?;
        if decoded.contains('\0') || decoded.contains('\\') {
            return Err(PathLinkError::NotFound);
        }
        let note_dir = from_note_id.rsplit_once('/').map_or("", |(dir, _)| dir);
        let bases: &[&str] = if decoded.starts_with('/') {
            &[""]
        } else {
            &[note_dir, ""]
        };
        let mut escaped = false;
        for base in bases {
            match normalize(base, &decoded) {
                Some(parts) if !parts.is_empty() => {
                    if let Some(found) = self.lookup(&parts) {
                        return found;
                    }
                }
                Some(_) => {}
                None => escaped = true,
            }
        }
        Err(if escaped {
            PathLinkError::OutsideVault
        } else {
            PathLinkError::NotFound
        })
    }

    /// `None` when `parts` names nothing here, so the caller tries its next base.
    fn lookup(&self, parts: &[String]) -> Option<Result<LinkTarget, PathLinkError>> {
        let joined = parts.join("/");
        let (id, ext) = split_md(&joined);
        if !ext.is_empty() && !id.is_empty() {
            return self
                .notes
                .contains_key(id)
                .then(|| Ok(LinkTarget::Note(id.to_owned())));
        }
        let path = parts.iter().fold(self.root.clone(), |p, part| p.join(part));
        if !path.is_file() {
            // No such file: an extension-less link may still name a note.
            return self
                .notes
                .contains_key(&joined)
                .then_some(Ok(LinkTarget::Note(joined)));
        }
        // A symlink inside the vault must not lead out of it.
        let (Ok(real), Ok(root)) = (path.canonicalize(), self.root.canonicalize()) else {
            return Some(Err(PathLinkError::NotFound));
        };
        Some(if real.starts_with(&root) {
            Ok(LinkTarget::File(real))
        } else {
            Err(PathLinkError::OutsideVault)
        })
    }
}

/// The file an image embed written as `dest` in note `from_note_id` points at:
/// tried against the note's folder, then the vault root, and never outside
/// the root (symlinks included). Needs only the root, so a worker thread can
/// run it without the [`Vault`].
pub fn image_file(root: &std::path::Path, from_note_id: &str, dest: &str) -> Option<PathBuf> {
    if dest.len() > MAX_DEST_BYTES || scheme(dest).is_some() {
        return None;
    }
    let decoded = percent_decode(split_fragment(dest).0)?;
    if decoded.is_empty() || decoded.contains(['\0', '\\']) {
        return None;
    }
    let note_dir = from_note_id.rsplit_once('/').map_or("", |(dir, _)| dir);
    let bases: &[&str] = if decoded.starts_with('/') {
        &[""]
    } else {
        &[note_dir, ""]
    };
    let root = root.canonicalize().ok()?;
    let by_path = bases.iter().find_map(|base| {
        let parts = normalize(base, &decoded).filter(|p| !p.is_empty())?;
        let real = parts
            .iter()
            .fold(root.clone(), |p, part| p.join(part))
            .canonicalize()
            .ok()?;
        (real.is_file() && real.starts_with(&root)).then_some(real)
    });
    // Obsidian finds a bare file name anywhere in the vault.
    by_path.or_else(|| {
        (!decoded.contains('/'))
            .then(|| find_by_name(&root, &decoded))
            .flatten()
    })
}

/// Most folders and files [`find_by_name`] looks at before giving up.
const MAX_SCANNED_ENTRIES: usize = 200_000;

/// The file called `name` nearest the top of the vault under `root` (ties go
/// to the alphabetically first path), skipping hidden folders and symlinks.
fn find_by_name(root: &std::path::Path, name: &str) -> Option<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    let mut scanned = 0;
    while !level.is_empty() {
        let mut found: Vec<PathBuf> = Vec::new();
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                scanned += 1;
                let (file_name, Ok(kind)) = (entry.file_name(), entry.file_type()) else {
                    continue;
                };
                if kind.is_dir() && !file_name.to_string_lossy().starts_with('.') {
                    next.push(entry.path());
                } else if kind.is_file() && file_name.to_string_lossy() == name {
                    found.push(entry.path());
                }
            }
            if scanned > MAX_SCANNED_ENTRIES {
                return None;
            }
        }
        if let Some(best) = found.into_iter().min() {
            return Some(best);
        }
        level = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault() -> (tempfile::TempDir, Vault) {
        crate::test_support::vault_with(&[
            ("Top.md", "top"),
            ("sub/Child.md", "child"),
            ("sub/deep/Leaf.md", "leaf"),
            ("sub/doc.pdf", "pdf"),
            ("sub/my file.png", "png"),
        ])
    }

    fn note(id: &str) -> Result<LinkTarget, PathLinkError> {
        Ok(LinkTarget::Note(id.into()))
    }

    #[test]
    fn relative_paths_resolve_from_the_notes_folder_then_the_root() {
        let (_dir, v) = vault();
        assert_eq!(
            v.resolve_path_link("sub/Child", "deep/Leaf.md"),
            note("sub/deep/Leaf")
        );
        assert_eq!(
            v.resolve_path_link("sub/deep/Leaf", "../Child.md"),
            note("sub/Child")
        );
        assert_eq!(v.resolve_path_link("sub/Child", "../Top.md"), note("Top"));
        assert_eq!(v.resolve_path_link("sub/Child", "/Top.md"), note("Top"));
        // Not next to the note, but a vault-relative path.
        assert_eq!(
            v.resolve_path_link("sub/Child", "sub/deep/Leaf.md"),
            note("sub/deep/Leaf")
        );
        assert_eq!(
            v.resolve_path_link("Top", "./sub/Child.md"),
            note("sub/Child")
        );
    }

    #[test]
    fn extension_is_optional_for_notes_and_case_of_md_is_ignored() {
        let (_dir, v) = vault();
        assert_eq!(v.resolve_path_link("Top", "sub/Child"), note("sub/Child"));
        assert_eq!(
            v.resolve_path_link("Top", "sub/Child.MD"),
            note("sub/Child")
        );
    }

    #[test]
    fn fragments_queries_and_percent_escapes_are_handled() {
        let (_dir, v) = vault();
        assert_eq!(
            v.resolve_path_link("Top", "sub/Child.md#Heading"),
            note("sub/Child")
        );
        assert_eq!(
            v.resolve_path_link("Top", "sub/Child.md?x=1"),
            note("sub/Child")
        );
        assert_eq!(
            v.resolve_path_link("Top", "#Heading"),
            Err(PathLinkError::SameNote)
        );
        assert_eq!(
            v.resolve_path_link("Top", "?q"),
            Err(PathLinkError::SameNote)
        );
        let file = v.resolve_path_link("Top", "sub/my%20file.png").unwrap();
        assert!(matches!(file, LinkTarget::File(p) if p.ends_with("sub/my file.png")));
        assert_eq!(
            v.resolve_path_link("Top", "sub/%zz.md"),
            Err(PathLinkError::NotFound)
        );
        assert_eq!(
            v.resolve_path_link("Top", "sub/%ff.md"),
            Err(PathLinkError::NotFound)
        );
    }

    #[test]
    fn other_files_inside_the_vault_resolve_to_a_path() {
        let (dir, v) = vault();
        let want = dir.path().canonicalize().unwrap().join("sub/doc.pdf");
        assert_eq!(
            v.resolve_path_link("sub/Child", "doc.pdf"),
            Ok(LinkTarget::File(want))
        );
    }

    #[test]
    fn schemes_are_external_and_missing_things_are_not_found() {
        let (_dir, v) = vault();
        for dest in [
            "https://x.y",
            "mailto:a@b.c",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            assert_eq!(
                v.resolve_path_link("Top", dest),
                Err(PathLinkError::External),
                "{dest}"
            );
        }
        for dest in [
            "nope.md", "sub", "sub/", "sub/deep", "", "a\0b.md", "a\\b.md",
        ] {
            assert_ne!(v.resolve_path_link("Top", dest), note("Top"), "{dest}");
            assert!(v.resolve_path_link("Top", dest).is_err(), "{dest}");
        }
    }

    #[test]
    fn nothing_outside_the_vault_is_ever_returned() {
        let (dir, v) = vault();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "s").unwrap();
        for dest in ["../secret.txt", "../../etc/passwd", "sub/../../x", "/../x"] {
            assert!(
                matches!(
                    v.resolve_path_link("Top", dest),
                    Err(PathLinkError::OutsideVault | PathLinkError::NotFound)
                ),
                "{dest}"
            );
        }
        assert_eq!(
            v.resolve_path_link("Top", "../x"),
            Err(PathLinkError::OutsideVault)
        );
        // An absolute path is read as vault-relative, so it cannot name /etc.
        assert!(v.resolve_path_link("Top", "/etc/passwd").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("secret.txt"),
                dir.path().join("link.txt"),
            )
            .unwrap();
            assert_eq!(
                v.resolve_path_link("Top", "link.txt"),
                Err(PathLinkError::OutsideVault),
                "a symlink must not lead out of the vault"
            );
        }
    }

    #[test]
    fn a_note_with_a_dot_in_its_name_resolves_without_an_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Meeting 2024.11.md"), "m").unwrap();
        let v = Vault::open(dir.path()).unwrap();
        assert_eq!(
            v.resolve_path_link("x", "Meeting%202024.11"),
            note("Meeting 2024.11")
        );
    }

    #[test]
    fn wikilinks_resolve_by_id_then_case_insensitively_by_title() {
        let (_dir, v) = vault();
        assert_eq!(v.resolve_wikilink("Top"), Some("Top"));
        assert_eq!(v.resolve_wikilink("top"), Some("Top"));
        assert_eq!(v.resolve_wikilink("Top.md"), Some("Top"));
        assert_eq!(v.resolve_wikilink("Child"), Some("sub/Child"), "by title");
        assert_eq!(v.resolve_wikilink("sub/child"), Some("sub/Child"));
        assert_eq!(v.resolve_wikilink("Leaf"), Some("sub/deep/Leaf"));
        assert_eq!(v.resolve_wikilink("Nope"), None);
        assert_eq!(v.resolve_wikilink(""), None);
    }

    #[test]
    fn an_ambiguous_wikilink_prefers_the_shortest_id_and_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["a/b/Same.md", "z/Same.md", "a/Same.md", "Same.md"] {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        let v = Vault::open(dir.path()).unwrap();
        assert_eq!(v.resolve_wikilink("same"), Some("Same"));
        fs::remove_file(dir.path().join("Same.md")).unwrap();
        let v = Vault::open(dir.path()).unwrap();
        assert_eq!(
            v.resolve_wikilink("same"),
            Some("a/Same"),
            "shorter, then alphabetical"
        );
    }

    #[test]
    fn backlinks_and_clicks_agree_on_an_ambiguous_title() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["z/Same.md", "a/Same.md"] {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        fs::write(dir.path().join("Linker.md"), "[[Same]]").unwrap();
        let v = Vault::open(dir.path()).unwrap();
        let clicked = v.resolve_wikilink("Same").unwrap().to_owned();
        let backlinks = v.backlinks();
        assert_eq!(backlinks.get(&clicked), Some(&vec!["Linker".to_owned()]));
        assert_eq!(backlinks.len(), 1);
    }

    #[test]
    fn oversized_destinations_are_rejected() {
        let (_dir, v) = vault();
        let long = "a/".repeat(MAX_DEST_BYTES);
        assert_eq!(
            v.resolve_path_link("Top", &long),
            Err(PathLinkError::NotFound)
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(split_fragment("a.md#h"), ("a.md", Some("h")));
        assert_eq!(split_fragment("a.md?q#h"), ("a.md", Some("q#h")));
        assert_eq!(split_fragment("a.md"), ("a.md", None));
        assert_eq!(percent_decode("a%20b%C3%A9"), Some("a bé".into()));
        assert_eq!(percent_decode("100%"), None);
        assert_eq!(normalize("a/b", "../c"), Some(vec!["a".into(), "c".into()]));
        assert_eq!(normalize("", "../c"), None);
    }

    #[test]
    fn image_file_finds_files_in_the_vault_and_nothing_outside() {
        let (dir, _v) = vault();
        let root = dir.path();
        let found = image_file(root, "sub/Child", "my%20file.png").unwrap();
        assert!(found.ends_with("sub/my file.png"));
        assert_eq!(image_file(root, "Top", "sub/my file.png"), Some(found));
        assert_eq!(image_file(root, "sub/Child", "../../etc/passwd"), None);
        assert_eq!(image_file(root, "Top", "https://x/y.png"), None);
        assert_eq!(image_file(root, "Top", "missing.png"), None);
        assert_eq!(image_file(root, "Top", "sub"), None);
    }

    #[test]
    fn image_file_finds_a_bare_name_anywhere_nearest_first() {
        let (dir, _v) = crate::test_support::vault_with(&[
            ("Top.md", "t"),
            ("a/deep/x.png", "1"),
            ("b/x.png", "2"),
            ("c/.hidden/y.png", "3"),
        ]);
        let root = dir.path();
        let found = image_file(root, "Top", "x.png").unwrap();
        assert!(found.ends_with("b/x.png"), "the shallower one: {found:?}");
        assert_eq!(
            image_file(root, "Top", "y.png"),
            None,
            "hidden folders are skipped"
        );
        assert_eq!(
            image_file(root, "Top", "a/x.png"),
            None,
            "a path is not a name"
        );
    }
}
