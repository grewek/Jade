//! Renaming a note and keeping the links that point at it working.
//!
//! Links are rewritten with the same scanner that finds them for rendering
//! and clicking ([`crate::link_scan`]), and with the same rules for what a
//! link points at ([`Vault::resolve_wikilink`], [`Vault::resolve_path_link`]),
//! so exactly the links that opened the note before now open it under its new
//! name. Fenced code is left alone. Only the part of a link that names the
//! note changes: a wikilink keeps its `#heading` and `|alias`, a path link its
//! folders, extension and `#fragment`.

use std::collections::HashMap;
use std::ops::Range;

use crate::fence::{FenceRole, step};
use crate::link_scan::{LinkKind, scan_line, scheme};
use crate::resolve::{LinkTarget, split_fragment};
use crate::vault::Vault;

/// What a rename did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    /// The renamed note's id.
    pub new_id: String,
    /// Notes whose files were rewritten (by their ids after the rename,
    /// which includes the renamed note when it linked to itself). The caller
    /// holds no stale text for these only if it reloads them.
    pub rewritten: Vec<String>,
    /// Notes that needed a rewrite but could not be written.
    pub failed: Vec<String>,
}

impl Vault {
    /// Renames a note like [`Vault::rename_note`], then rewrites every link to
    /// it in the vault's notes (wikilinks and `[text](path)` links) to use the
    /// new name. Reads every note's file once to find the links, so call it on
    /// a user's rename, not per frame. Callers must save open editors first.
    pub fn rename_note_updating_links(
        &mut self,
        id: &str,
        new_title: &str,
    ) -> std::io::Result<Renamed> {
        let title = self.free_title(id, new_title)?;
        // Decided against the vault as it is now, before the file moves.
        let plan = self.plan_link_rewrites(id, &title);
        let new_id = self.rename_note_to(id, &title)?;

        let mut rewritten = Vec::new();
        let mut failed = Vec::new();
        for (old_note_id, text) in plan {
            let now = if old_note_id == id {
                new_id.clone()
            } else {
                old_note_id
            };
            match self.save_note(&now, &text) {
                Ok(()) => rewritten.push(now),
                Err(_) => failed.push(now),
            }
        }
        rewritten.sort_unstable();
        failed.sort_unstable();
        Ok(Renamed {
            new_id,
            rewritten,
            failed,
        })
    }

    /// Each note that links to `id`, with its text rewritten for the file
    /// being renamed to `title`.
    fn plan_link_rewrites(&self, id: &str, title: &str) -> Vec<(String, String)> {
        let dir = id.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut plan = Vec::new();
        let mut text = String::new();
        // Built once: resolving each wikilink by scanning the notes would be
        // quadratic in the vault.
        let index = self.title_index();
        for note in self.notes.values() {
            if note.read_content_into(&mut text).is_err() {
                continue;
            }
            if !text.contains("[[") && !text.contains("](") {
                continue;
            }
            if let Some(new_text) = rewrite_text(self, &index, &note.id, &text, id, dir, title) {
                plan.push((note.id.clone(), new_text));
            }
        }
        plan
    }
}

/// `text` with every link in note `from` that points at note `target` changed
/// to name `title` instead (`dir` is the target's folder, unchanged by a
/// rename). `None` when no link changed.
fn rewrite_text(
    vault: &Vault,
    index: &HashMap<String, String>,
    from: &str,
    text: &str,
    target: &str,
    dir: &str,
    title: &str,
) -> Option<String> {
    let mut out = String::new();
    let mut changed = false;
    let mut fence = None;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let (role, next) = step(fence, line);
        fence = next;
        let edits = if role == FenceRole::Plain {
            line_edits(vault, index, from, line, target, dir, title)
        } else {
            Vec::new()
        };
        if edits.is_empty() {
            out.push_str(line);
            continue;
        }
        changed = true;
        let mut at = 0;
        for (range, replacement) in edits {
            out.push_str(&line[at..range.start]);
            out.push_str(&replacement);
            at = range.end;
        }
        out.push_str(&line[at..]);
    }
    changed.then_some(out)
}

/// The replacements, in order of position, that retarget `line`'s links.
fn line_edits(
    vault: &Vault,
    index: &HashMap<String, String>,
    from: &str,
    line: &str,
    target: &str,
    dir: &str,
    title: &str,
) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    scan_line(line, |link| match link.kind {
        LinkKind::Wiki => {
            let written = &line[link.dest.clone()];
            if vault.resolve(written, index).as_deref() == Some(target) {
                let new = wikilink_target(written, dir, title);
                if new != written {
                    edits.push((link.dest, new));
                }
            }
        }
        LinkKind::Inline => {
            let written = &line[link.dest.clone()];
            if scheme(written).is_some() {
                return;
            }
            if vault.resolve_path_link(from, written) == Ok(LinkTarget::Note(target.to_string())) {
                let (path, _) = split_fragment(written);
                let name_at = path.rfind('/').map_or(0, |i| i + 1);
                let (name, ext) = split_extension(&path[name_at..]);
                let start = link.dest.start + name_at;
                let end = start + name.len() + ext.len();
                let new = format!("{}{ext}", encode_segment(title));
                if new != line[start..end] {
                    edits.push((start..end, new));
                }
            }
        }
        LinkKind::Autolink | LinkKind::BareUrl => {}
    });
    edits
}

/// A wikilink's new target: a path through folders stays a path to the new
/// file, a plain title becomes the new title; a `.md` the author wrote stays.
fn wikilink_target(written: &str, dir: &str, title: &str) -> String {
    let (stem, ext) = split_extension(written);
    let name = if stem.contains('/') && !dir.is_empty() {
        format!("{dir}/{title}")
    } else {
        title.to_string()
    };
    format!("{name}{ext}")
}

/// `name` split before a trailing `.md` (any case), which may be absent.
fn split_extension(name: &str) -> (&str, &str) {
    match name.len().checked_sub(3) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".md") => {
            name.split_at(at)
        }
        _ => (name, ""),
    }
}

/// `name` escaped for use as one segment of a `[text](path)` destination.
fn encode_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(
            ch,
            ' ' | '%' | '#' | '?' | '(' | ')' | '<' | '>' | '[' | ']' | '\\' | '"'
        ) {
            let mut buf = [0; 4];
            for byte in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    fn read(dir: &tempfile::TempDir, path: &str) -> String {
        fs::read_to_string(dir.path().join(path)).unwrap()
    }

    #[test]
    fn wikilinks_keep_their_heading_alias_and_extension() {
        let (dir, mut v) = vault(&[
            ("Old.md", "# Old"),
            (
                "A.md",
                "see [[Old]] and [[old#Sec|alias]] [[Old.md]] [[Other]]\nsecond [[Old#Only]]",
            ),
            ("Other.md", "x"),
        ]);
        let done = v.rename_note_updating_links("Old", "New Name").unwrap();
        assert_eq!(done.new_id, "New Name");
        assert_eq!(done.rewritten, ["A"]);
        assert_eq!(
            read(&dir, "A.md"),
            "see [[New Name]] and [[New Name#Sec|alias]] [[New Name.md]] [[Other]]\nsecond [[New Name#Only]]"
        );
        assert_eq!(v.resolve_wikilink("New Name"), Some("New Name"));
    }

    #[test]
    fn code_is_left_alone() {
        let text = "[[Old]]\n```\n[[Old]] [x](Old.md)\n```\n`[[Old]]` \\[[Old]]";
        let (dir, mut v) = vault(&[("Old.md", ""), ("A.md", text)]);
        v.rename_note_updating_links("Old", "New").unwrap();
        assert_eq!(
            read(&dir, "A.md"),
            "[[New]]\n```\n[[Old]] [x](Old.md)\n```\n`[[Old]]` \\[[Old]]"
        );
    }

    #[test]
    fn path_links_keep_folders_extension_and_fragment() {
        let (dir, mut v) = vault(&[
            ("f/Old.md", ""),
            (
                "f/X.md",
                "[a](Old.md#h) [b](./Old.md) [c](/f/Old.md) [d](Old) [e](other.md) [f](https://x/Old.md)",
            ),
            (
                "Root.md",
                "[g](f/Old.md) [h](<f/Old.md?x=1>) [i](f/Old%20.md)",
            ),
        ]);
        let done = v.rename_note_updating_links("f/Old", "A (b) c").unwrap();
        assert_eq!(done.new_id, "f/A (b) c");
        assert_eq!(done.rewritten, ["Root", "f/X"]);
        assert_eq!(
            read(&dir, "f/X.md"),
            "[a](A%20%28b%29%20c.md#h) [b](./A%20%28b%29%20c.md) [c](/f/A%20%28b%29%20c.md) [d](A%20%28b%29%20c) [e](other.md) [f](https://x/Old.md)"
        );
        assert_eq!(
            read(&dir, "Root.md"),
            "[g](f/A%20%28b%29%20c.md) [h](<f/A%20%28b%29%20c.md?x=1>) [i](f/Old%20.md)"
        );
        // Every rewritten link opens the renamed note.
        assert_eq!(
            v.resolve_path_link("f/X", "A%20%28b%29%20c.md#h"),
            Ok(LinkTarget::Note("f/A (b) c".into()))
        );
    }

    #[test]
    fn a_path_wikilink_stays_a_path() {
        let (dir, mut v) = vault(&[
            ("f/Old.md", ""),
            ("A.md", "[[f/Old]] [[f/Old.md|x]] [[Old]]"),
        ]);
        v.rename_note_updating_links("f/Old", "New").unwrap();
        assert_eq!(read(&dir, "A.md"), "[[f/New]] [[f/New.md|x]] [[New]]");
    }

    #[test]
    fn only_links_that_resolved_to_the_renamed_note_change() {
        // `[[Old]]` means the shorter id, the root note, so it is not ours.
        let (dir, mut v) = vault(&[
            ("Old.md", ""),
            ("f/Old.md", ""),
            ("A.md", "[[Old]] [[f/Old]] [x](f/Old.md) [y](Old.md)"),
        ]);
        v.rename_note_updating_links("f/Old", "New").unwrap();
        assert_eq!(
            read(&dir, "A.md"),
            "[[Old]] [[f/New]] [x](f/New.md) [y](Old.md)"
        );
    }

    #[test]
    fn the_renamed_note_may_link_to_itself_and_a_taken_name_gets_a_suffix() {
        let (dir, mut v) = vault(&[("Old.md", "me [[Old]]"), ("Taken.md", "")]);
        let done = v.rename_note_updating_links("Old", "Taken").unwrap();
        assert_eq!(done.new_id, "Taken 1");
        assert_eq!(done.rewritten, ["Taken 1"]);
        assert_eq!(read(&dir, "Taken 1.md"), "me [[Taken 1]]");
    }

    #[test]
    fn nothing_changes_when_nothing_links_or_the_name_is_the_same() {
        let (dir, mut v) = vault(&[("Old.md", ""), ("A.md", "[[Else]]\r\ntext")]);
        let done = v.rename_note_updating_links("Old", "New").unwrap();
        assert!(done.rewritten.is_empty() && done.failed.is_empty());
        assert_eq!(read(&dir, "A.md"), "[[Else]]\r\ntext");
        fs::write(dir.path().join("B.md"), "[[New]] [x](New.md)").unwrap();
        let mut v = Vault::open(dir.path()).unwrap();
        let same = v.rename_note_updating_links("New", "New").unwrap();
        assert_eq!((same.new_id.as_str(), same.rewritten.len()), ("New", 0));
    }

    #[test]
    fn line_endings_and_other_bytes_survive() {
        let (dir, mut v) = vault(&[
            ("Old.md", ""),
            ("A.md", "a [[Old]]\r\n\r\n\u{e9} [[Old]]\n"),
        ]);
        v.rename_note_updating_links("Old", "N").unwrap();
        assert_eq!(read(&dir, "A.md"), "a [[N]]\r\n\r\n\u{e9} [[N]]\n");
    }

    #[test]
    fn backlinks_follow_the_rename() {
        let (_dir, mut v) = vault(&[("Old.md", ""), ("A.md", "[[Old]]")]);
        v.rename_note_updating_links("Old", "New").unwrap();
        assert_eq!(
            v.backlinks().get("New").map(Vec::as_slice),
            Some(&["A".to_string()][..])
        );
    }
}
