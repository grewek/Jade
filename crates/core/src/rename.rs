//! Renaming a note and keeping the links that point at it working.
//!
//! Links are rewritten with the same scanner that finds them for rendering
//! and clicking ([`crate::link_scan`]), and with the same rules for what a
//! link points at ([`Vault::link_destination`]),
//! so exactly the links that opened the note before now open it under its new
//! name. Fenced code is left alone. Only the part of a link that names the
//! note changes: a wikilink keeps its `#heading` and `|alias`, a path link its
//! folders, extension and `#fragment`.

use std::ops::Range;
use std::path::Path;

use crate::fence::{FenceRole, lines_with_roles};
use crate::link_scan::{LinkKind, scan_line};
use crate::math_scan::BlockLines;
use crate::note_name::split_md;
use crate::resolve::{Destination, percent_decode, split_fragment};
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
        let dir = id.rsplit_once('/').map_or("", |(dir, _)| dir);
        let plan =
            self.plan_rewrites(|from, line| note_line_edits(self, from, line, id, dir, &title));
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

    /// Renames a folder like [`Vault::rename_folder`], then rewrites the links
    /// that pointed into it: path links to its notes and other files, and
    /// path-style wikilinks (`[[folder/Note]]`) to its notes. A link that names
    /// a note by title alone is unaffected by a folder's name and is left as
    /// written. Same cost and caveats as [`Vault::rename_note_updating_links`];
    /// `new_id` is the folder's new id and `rewritten` holds note ids after the
    /// move.
    pub fn rename_folder_updating_links(
        &mut self,
        id: &str,
        new_name: &str,
    ) -> std::io::Result<Renamed> {
        let name = self.free_folder_name(id, new_name)?;
        let old_name = id.rsplit('/').next().unwrap_or(id).to_string();
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let plan = self.plan_rewrites(|from, line| {
            folder_line_edits(self, &root, from, line, id, &old_name, &name)
        });
        let new_id = self.rename_folder_to(id, &name)?;

        let old_prefix = format!("{id}/");
        let mut rewritten = Vec::new();
        let mut failed = Vec::new();
        for (old_note_id, text) in plan {
            let now = match old_note_id.strip_prefix(&old_prefix) {
                Some(rest) => format!("{new_id}/{rest}"),
                None => old_note_id,
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

    /// Each note with at least one link `edits_for(note_id, line)` wants
    /// changed, with its rewritten text.
    fn plan_rewrites(
        &self,
        mut edits_for: impl FnMut(&str, &str) -> Vec<(Range<usize>, String)>,
    ) -> Vec<(String, String)> {
        let mut plan = Vec::new();
        let mut text = String::new();
        for note in self.notes.values() {
            if note.read_content_into(&mut text).is_err() {
                continue;
            }
            if !text.contains("[[") && !text.contains("](") {
                continue;
            }
            if let Some(new_text) = rewrite_text(&text, |line| edits_for(&note.id, line)) {
                plan.push((note.id.clone(), new_text));
            }
        }
        plan
    }
}

/// `text` with the edits `edits_for` returns for each line outside fenced code
/// applied (ranges within the line, in order). `None` when nothing changed.
fn rewrite_text(
    text: &str,
    mut edits_for: impl FnMut(&str) -> Vec<(Range<usize>, String)>,
) -> Option<String> {
    let mut out = String::new();
    let mut changed = false;
    let mut math = BlockLines::of(text);
    for (n, line, role) in lines_with_roles(text) {
        if n > 0 {
            out.push('\n');
        }
        let edits = if role == FenceRole::Plain && !math.contains(n) {
            edits_for(line)
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

/// The replacements, in order of position, that `change` asks for among the
/// links of `line` (written in note `from`). `change` sees how a link is
/// written, its text and where it points, and answers with a range within
/// that text and what to put there.
fn link_edits(
    vault: &Vault,
    from: &str,
    line: &str,
    mut change: impl FnMut(LinkKind, &str, &Destination) -> Option<(Range<usize>, String)>,
) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    scan_line(line, |link| {
        let written = &line[link.dest.clone()];
        let destination = vault.link_destination(from, link.kind, written);
        if let Some((range, new)) = change(link.kind, written, &destination)
            && new != written[range.clone()]
        {
            let at = link.dest.start;
            edits.push((at + range.start..at + range.end, new));
        }
    });
    edits
}

/// Retargets `line`'s links at note `target`, renamed to `title` (`dir` is its
/// folder, unchanged by a rename).
fn note_line_edits(
    vault: &Vault,
    from: &str,
    line: &str,
    target: &str,
    dir: &str,
    title: &str,
) -> Vec<(Range<usize>, String)> {
    link_edits(vault, from, line, |kind, written, destination| {
        if !matches!(destination, Destination::Note(id) if id == target) {
            return None;
        }
        match kind {
            LinkKind::Wiki => Some((0..written.len(), wikilink_target(written, dir, title))),
            _ => {
                let (path, _) = split_fragment(written);
                let name_at = path.rfind('/').map_or(0, |i| i + 1);
                let (name, ext) = split_md(&path[name_at..]);
                let end = name_at + name.len() + ext.len();
                Some((name_at..end, format!("{}{ext}", encode_segment(title))))
            }
        }
    })
}

/// Retargets `line`'s links that point into folder `folder` (whose last
/// component is `old_name`) at it renamed to `new_name`. The folder's
/// component is found by counting back from the end of the written path: the
/// rest of the target's path inside the folder has a known number of
/// components.
fn folder_line_edits(
    vault: &Vault,
    root: &Path,
    from: &str,
    line: &str,
    folder: &str,
    old_name: &str,
    new_name: &str,
) -> Vec<(Range<usize>, String)> {
    let prefix = format!("{folder}/");
    link_edits(vault, from, line, |kind, written, destination| {
        let inside = match destination {
            Destination::Note(id) => id.strip_prefix(&prefix),
            Destination::File(path) => path
                .strip_prefix(root)
                .ok()
                .and_then(Path::to_str)
                .and_then(|p| p.strip_prefix(&prefix)),
            _ => None,
        }?;
        let after = inside.split('/').count();
        if kind == LinkKind::Wiki {
            // A bare title does not mention the folder.
            if !written.contains('/') {
                return None;
            }
            let (stem, _) = split_md(written);
            let seg = folder_segment(stem, after, |s| s.eq_ignore_ascii_case(old_name))?;
            Some((seg, new_name.to_string()))
        } else {
            let (path, _) = split_fragment(written);
            let seg = folder_segment(path, after, |s| {
                percent_decode(s).is_some_and(|decoded| decoded == old_name)
            })?;
            Some((seg, encode_segment(new_name)))
        }
    })
}

/// The byte range of the `/`-separated component of `path` that sits `after`
/// components before its end, if `matches` accepts it.
fn folder_segment(
    path: &str,
    after: usize,
    matches: impl Fn(&str) -> bool,
) -> Option<Range<usize>> {
    let count = path.split('/').count();
    let wanted = count.checked_sub(after + 1)?;
    let mut start = 0;
    for (i, seg) in path.split('/').enumerate() {
        if i == wanted {
            return matches(seg).then_some(start..start + seg.len());
        }
        start += seg.len() + 1;
    }
    None
}

/// A wikilink's new target: a path through folders stays a path to the new
/// file, a plain title becomes the new title; a `.md` the author wrote stays.
fn wikilink_target(written: &str, dir: &str, title: &str) -> String {
    let (stem, ext) = split_md(written);
    let name = if stem.contains('/') && !dir.is_empty() {
        format!("{dir}/{title}")
    } else {
        title.to_string()
    };
    format!("{name}{ext}")
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

    use crate::test_support::vault_with as vault;

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
            v.link_destination("f/X", LinkKind::Inline, "A%20%28b%29%20c.md#h"),
            Destination::Note("f/A (b) c".into())
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
    fn renaming_a_folder_rewrites_path_links_into_it() {
        let (dir, mut v) = vault(&[
            (
                "docs/Note.md",
                "in [[docs/Other]] [a](Other.md) [b](docs/Other.md) [c](../Top.md)",
            ),
            ("docs/Other.md", ""),
            ("docs/sub/Deep.md", "[up](../Note.md) [abs](/docs/Other.md)"),
            ("docs/file.pdf", "pdf"),
            (
                "Top.md",
                "[[docs/Note#h|n]] [[Note]] [x](docs/Note.md#h) [y](docs/sub/Deep) [z](docs/file.pdf) [w](docs/missing.md)",
            ),
            (
                "other/Away.md",
                "[l](../docs/Note.md) [m](../docs/file.pdf)",
            ),
        ]);
        let done = v.rename_folder_updating_links("docs", "My Docs").unwrap();
        assert_eq!(done.new_id, "My Docs");
        assert_eq!(
            done.rewritten,
            ["My Docs/Note", "My Docs/sub/Deep", "Top", "other/Away"]
        );
        assert_eq!(
            read(&dir, "My Docs/Note.md"),
            "in [[My Docs/Other]] [a](Other.md) [b](My%20Docs/Other.md) [c](../Top.md)"
        );
        assert_eq!(
            read(&dir, "My Docs/sub/Deep.md"),
            "[up](../Note.md) [abs](/My%20Docs/Other.md)"
        );
        assert_eq!(
            read(&dir, "Top.md"),
            "[[My Docs/Note#h|n]] [[Note]] [x](My%20Docs/Note.md#h) [y](My%20Docs/sub/Deep) [z](My%20Docs/file.pdf) [w](docs/missing.md)"
        );
        assert_eq!(
            read(&dir, "other/Away.md"),
            "[l](../My%20Docs/Note.md) [m](../My%20Docs/file.pdf)"
        );
        // Everything rewritten still opens what it did.
        assert_eq!(
            v.link_destination("Top", LinkKind::Inline, "My%20Docs/Note.md#h"),
            Destination::Note("My Docs/Note".into())
        );
        assert_eq!(v.resolve_wikilink("My Docs/Other"), Some("My Docs/Other"));
    }

    #[test]
    fn a_nested_folder_and_a_repeated_name_are_counted_from_the_end() {
        let (dir, mut v) = vault(&[
            ("a/a/Note.md", ""),
            ("a/Note.md", ""),
            ("Top.md", "[x](a/a/Note.md) [y](a/Note.md) [[a/a/Note]]"),
        ]);
        // Renaming the inner `a/a` must not touch the outer `a`.
        v.rename_folder_updating_links("a/a", "b").unwrap();
        assert_eq!(
            read(&dir, "Top.md"),
            "[x](a/b/Note.md) [y](a/Note.md) [[a/b/Note]]"
        );
    }

    #[test]
    fn a_folder_rename_leaves_code_and_title_links_alone() {
        let text = "```\n[x](docs/N.md)\n```\n[[N]] `[[docs/N]]`";
        let (dir, mut v) = vault(&[("docs/N.md", ""), ("T.md", text)]);
        let done = v.rename_folder_updating_links("docs", "d2").unwrap();
        assert!(done.rewritten.is_empty());
        assert_eq!(read(&dir, "T.md"), text);
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

    #[test]
    fn a_link_inside_a_math_block_is_not_rewritten() {
        let (dir, mut v) = vault(&[
            ("Old.md", "# Old"),
            ("A.md", "[[Old]]\n$$\n[[Old]]\nx\n$$\n[[Old]]"),
        ]);
        v.rename_note_updating_links("Old", "New").unwrap();
        assert_eq!(read(&dir, "A.md"), "[[New]]\n$$\n[[Old]]\nx\n$$\n[[New]]");
    }
}
