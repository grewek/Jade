//! Runs `.sublime-syntax` definitions over a block of code and yields the
//! coloured spans of each line. Definitions are data read from a folder, none
//! is built in.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use syntect::parsing::{
    ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxSet, SyntaxSetBuilder,
};

use super::scope::role_for_scope;
use crate::theme::Role;

/// More files than this in the folder are ignored (one notice), so a wrong
/// folder (a home directory) cannot stall the load.
const MAX_DEFINITIONS: usize = 1_000;

/// A larger definition file is skipped; real ones are under 200 KB.
const MAX_DEFINITION_BYTES: u64 = 4 * 1024 * 1024;

const EXTENSION: &str = "sublime-syntax";

/// A coloured run inside one line; `range` is in bytes of that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::text_editor) struct Span {
    pub range: Range<usize>,
    pub role: Role,
}

/// A line longer than this is not parsed and stays plain: parsing costs about
/// 6 µs per byte, so one minified or data line would hold a block for seconds,
/// and it would add thousands of runs to shape.
const MAX_LINE_BYTES: usize = 2_000;

/// The spans of a block, one list per line; text with no role has no span.
#[derive(Debug, Default)]
pub(in crate::text_editor) struct Highlighted {
    pub lines: Vec<Vec<Span>>,
    /// A hash of each line's text, to tell which lines an edit left alone.
    hashes: Vec<u64>,
    /// The line where the parser gave up (a definition that refers to a file
    /// the folder lacks). Lines before it are coloured, it and the rest plain.
    pub stopped_at: Option<usize>,
}

impl Highlighted {
    /// Stand-in spans for `text`, a new version of the block `self` was made
    /// for, shown until the real ones arrive so an edit does not flash the
    /// block plain: lines before and after the edit keep their spans, the
    /// edited lines keep theirs as they were (a token may be a character off
    /// for a moment), and extra new lines are plain.
    pub fn carried_over(&self, text: &str) -> Highlighted {
        let new: Vec<(u64, usize)> = text
            .split_inclusive('\n')
            .map(|l| {
                let body = l.strip_suffix('\n').unwrap_or(l);
                (hash_line(body), body.len())
            })
            .collect();
        let (old_n, new_n) = (self.lines.len(), new.len());
        let same_len = self.hashes.len() == old_n;
        let common = old_n.min(new_n);
        let at = |i: usize| self.hashes[i];
        let prefix = if same_len {
            (0..common).take_while(|&i| at(i) == new[i].0).count()
        } else {
            0
        };
        let suffix = if same_len {
            (0..common - prefix)
                .take_while(|&k| at(old_n - 1 - k) == new[new_n - 1 - k].0)
                .count()
        } else {
            0
        };
        let middle_old = old_n - prefix - suffix;
        let lines = (0..new_n)
            .map(|i| {
                let from = if i < prefix {
                    Some(i)
                } else if i >= new_n - suffix {
                    Some(old_n - (new_n - i))
                } else {
                    (i - prefix < middle_old).then_some(i)
                };
                from.map_or_else(Vec::new, |f| clamp_spans(&self.lines[f], new[i].1))
            })
            .collect();
        Highlighted {
            lines,
            stopped_at: None,
            hashes: new.iter().map(|&(h, _)| h).collect(),
        }
    }
}

fn hash_line(line: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// `spans` cut to a line of `len` bytes.
fn clamp_spans(spans: &[Span], len: usize) -> Vec<Span> {
    spans
        .iter()
        .filter(|s| s.range.start < len)
        .map(|s| Span {
            range: s.range.start..s.range.end.min(len),
            role: s.role,
        })
        .collect()
}

/// The definitions of one folder, linked into a single set (a definition may
/// include another file, so they cannot be used one by one).
pub(in crate::text_editor) struct Highlighter {
    set: SyntaxSet,
}

/// A loaded folder and one notice per file or folder problem.
pub(in crate::text_editor) struct Loaded {
    pub highlighter: Highlighter,
    pub notices: Vec<String>,
}

impl Highlighter {
    /// Reads every `*.sublime-syntax` in `dir` (not below it) file by file: a
    /// bad file is a notice and is left out, the others still load.
    pub fn load_dir(dir: &Path) -> Loaded {
        let mut notices = Vec::new();
        let mut builder = SyntaxSetBuilder::new();
        let files = definition_files(dir, &mut notices);
        for path in files {
            match read_definition(&path) {
                Ok(definition) => builder.add(definition),
                Err(why) => notices.push(format!("{}: {why}", path.display())),
            }
        }
        Loaded {
            highlighter: Self {
                set: builder.build(),
            },
            notices,
        }
    }

    /// Whether some definition knows the fence word `language`.
    pub fn knows(&self, language: &str) -> bool {
        self.set.find_syntax_by_token(language).is_some()
    }

    /// The spans of `text` as `language` (a fence word: a name or extension a
    /// definition lists, any case); `None` when no definition knows it.
    pub fn highlight(&self, language: &str, text: &str) -> Option<Highlighted> {
        self.highlight_until(language, text, &|| false)
    }

    /// [`Highlighter::highlight`] that gives up (`None`) as soon as `cancelled`
    /// says so, checked before every line: a block that was edited or scrolled
    /// away while it was being parsed need not be finished.
    pub fn highlight_until(
        &self,
        language: &str,
        text: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Highlighted> {
        let syntax = self.set.find_syntax_by_token(language)?;
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut roles = HashMap::new();
        let mut out = Highlighted::default();
        let mut line = String::new();
        for (index, text_line) in text.split_inclusive('\n').enumerate() {
            if cancelled() {
                return None;
            }
            let body = text_line.strip_suffix('\n').unwrap_or(text_line);
            out.hashes.push(hash_line(body));
            if out.stopped_at.is_some() || body.len() > MAX_LINE_BYTES {
                out.lines.push(Vec::new());
                continue;
            }
            // The parser wants every line to end in a newline.
            line.clear();
            line.push_str(body);
            line.push('\n');
            let Ok(ops) = state.parse_line(&line, &self.set) else {
                out.stopped_at = Some(index);
                out.lines.push(Vec::new());
                continue;
            };
            out.lines
                .push(line_spans(&ops, body.len(), &mut stack, &mut roles));
        }
        Some(out)
    }
}

/// Definition files of `dir`, sorted so a load is the same every time.
fn definition_files(dir: &Path, notices: &mut Vec<String>) -> Vec<std::path::PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(why) => {
            notices.push(format!("{}: {why}", dir.display()));
            return Vec::new();
        }
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .collect();
    files.sort();
    if files.len() > MAX_DEFINITIONS {
        notices.push(format!(
            "{}: {} definitions, only the first {MAX_DEFINITIONS} are read",
            dir.display(),
            files.len()
        ));
        files.truncate(MAX_DEFINITIONS);
    }
    files
}

fn read_definition(path: &Path) -> Result<SyntaxDefinition, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_DEFINITION_BYTES {
        return Err(format!(
            "{size} bytes is over the {MAX_DEFINITION_BYTES} limit"
        ));
    }
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fallback_name = path.file_stem().and_then(|s| s.to_str());
    SyntaxDefinition::load_from_str(&source, true, fallback_name).map_err(|e| e.to_string())
}

/// Turns the scope operations of one line into spans: runs with the same role
/// are joined and runs with none are left out. `stack` carries over lines.
fn line_spans(
    ops: &[(usize, ScopeStackOp)],
    len: usize,
    stack: &mut ScopeStack,
    roles: &mut HashMap<Scope, Option<Role>>,
) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut at = 0;
    let mut role = current_role(stack, roles);
    for (end, op) in ops {
        push_run(&mut spans, at, (*end).min(len), role);
        at = (*end).min(len).max(at);
        // An operation the stack rejects leaves it as it was; the colours of
        // the rest of the block may be off, but nothing fails.
        let _ = stack.apply(op);
        role = current_role(stack, roles);
    }
    push_run(&mut spans, at, len, role);
    spans
}

fn push_run(spans: &mut Vec<Span>, start: usize, end: usize, role: Option<Role>) {
    let Some(role) = role else { return };
    if start >= end {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.role == role && last.range.end == start => last.range.end = end,
        _ => spans.push(Span {
            range: start..end,
            role,
        }),
    }
}

/// The role of the innermost scope on the stack that has one; each distinct
/// scope is looked up in the table once per block.
fn current_role(stack: &ScopeStack, roles: &mut HashMap<Scope, Option<Role>>) -> Option<Role> {
    stack.as_slice().iter().rev().find_map(|scope| {
        *roles
            .entry(*scope)
            .or_insert_with(|| role_for_scope(&scope.build_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::{Highlighter, Span};
    use crate::theme::Role;

    const TINY: &str = r#"%YAML 1.2
---
name: Tiny
file_extensions: [tiny]
scope: source.tiny
contexts:
  main:
    - match: '#.*$'
      scope: comment.line.tiny
    - match: '\b(?:let|fn)\b'
      scope: keyword.control.tiny
    - match: '\b[0-9]+\b'
      scope: constant.numeric.tiny
    - match: '"'
      scope: punctuation.definition.string.begin.tiny
      push: string
    - match: '@'
      push: scope:source.missing
  string:
    - meta_scope: string.quoted.double.tiny
    - match: '"'
      scope: punctuation.definition.string.end.tiny
      pop: true
"#;

    fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    fn tiny() -> (tempfile::TempDir, Highlighter) {
        let dir = folder(&[("Tiny.sublime-syntax", TINY)]);
        let loaded = Highlighter::load_dir(dir.path());
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        (dir, loaded.highlighter)
    }

    fn span(range: std::ops::Range<usize>, role: Role) -> Span {
        Span { range, role }
    }

    #[test]
    fn a_line_gets_a_span_per_coloured_run() {
        let (_dir, h) = tiny();
        let got = h.highlight("tiny", "let x = 42 # note\n").unwrap();
        assert_eq!(got.stopped_at, None);
        assert_eq!(
            got.lines,
            [vec![
                span(0..3, Role::SyntaxKeyword),
                span(8..10, Role::SyntaxNumber),
                span(11..17, Role::SyntaxComment),
            ]]
        );
    }

    #[test]
    fn a_string_spans_lines_and_its_quotes_join_it() {
        let (_dir, h) = tiny();
        let got = h.highlight("tiny", "a \"one\ntwo\" b\n").unwrap();
        assert_eq!(
            got.lines,
            [
                vec![span(2..6, Role::SyntaxString)],
                vec![span(0..4, Role::SyntaxString)],
            ]
        );
    }

    #[test]
    fn the_last_line_needs_no_newline() {
        let (_dir, h) = tiny();
        let got = h.highlight("TINY", "let\nfn").unwrap();
        assert_eq!(
            got.lines,
            [
                vec![span(0..3, Role::SyntaxKeyword)],
                vec![span(0..2, Role::SyntaxKeyword)],
            ]
        );
    }

    #[test]
    fn an_unknown_language_is_plain() {
        let (_dir, h) = tiny();
        assert!(h.highlight("nope", "let x\n").is_none());
        assert!(h.highlight("", "let x\n").is_none());
    }

    #[test]
    fn empty_text_has_no_lines() {
        let (_dir, h) = tiny();
        assert!(h.highlight("tiny", "").unwrap().lines.is_empty());
    }

    #[test]
    fn a_missing_reference_keeps_the_lines_before_it() {
        let (_dir, h) = tiny();
        let got = h.highlight("tiny", "let\na @ b\nlet\n").unwrap();
        assert_eq!(got.stopped_at, Some(1));
        assert_eq!(got.lines.len(), 3);
        assert_eq!(got.lines[0], [span(0..3, Role::SyntaxKeyword)]);
        assert!(got.lines[1].is_empty() && got.lines[2].is_empty());
    }

    #[test]
    fn a_cancelled_parse_stops_and_a_known_word_is_known() {
        let (_dir, h) = tiny();
        assert!(h.knows("tiny") && h.knows("TINY") && !h.knows("nope"));
        let polled = std::cell::Cell::new(0);
        // Cancelled on the third line: nothing is returned, and it stopped there.
        let got = h.highlight_until("tiny", "let\nlet\nlet\nlet\n", &|| {
            polled.set(polled.get() + 1);
            polled.get() > 2
        });
        assert!(got.is_none());
        assert_eq!(polled.get(), 3);
        assert!(h.highlight_until("tiny", "let\n", &|| false).is_some());
    }

    #[test]
    fn a_very_long_line_is_not_parsed_and_its_neighbours_are() {
        let (_dir, h) = tiny();
        let long = format!("let {}\n", "x ".repeat(2_000));
        let text = format!("let\n{long}let\n");
        let started = std::time::Instant::now();
        let got = h.highlight("tiny", &text).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(got.lines.len(), 3);
        assert_eq!(got.lines[0], [span(0..3, Role::SyntaxKeyword)]);
        assert!(got.lines[1].is_empty(), "over the line limit");
        assert_eq!(got.lines[2], [span(0..3, Role::SyntaxKeyword)]);
        assert_eq!(got.stopped_at, None);
    }

    fn lines_of(h: &Highlighter, text: &str) -> super::Highlighted {
        h.highlight("tiny", text).unwrap()
    }

    #[test]
    fn carried_over_keeps_lines_an_edit_left_alone_and_shifts_with_them() {
        let (_dir, h) = tiny();
        let old = lines_of(&h, "let a\nplain\nfn b\n");
        // Unchanged text: the same spans.
        assert_eq!(old.carried_over("let a\nplain\nfn b\n").lines, old.lines);
        // A line inserted in the middle: the ones around it keep theirs.
        let inserted = old.carried_over("let a\nnew\nplain\nfn b\n");
        assert_eq!(inserted.lines.len(), 4);
        assert_eq!(inserted.lines[0], old.lines[0]);
        assert_eq!(inserted.lines[2], old.lines[1]);
        assert_eq!(inserted.lines[3], old.lines[2]);
        // The first line is gone: the rest moves up.
        let removed = old.carried_over("plain\nfn b\n");
        assert_eq!(removed.lines, old.lines[1..]);
    }

    #[test]
    fn carried_over_keeps_an_edited_lines_spans_cut_to_its_new_length() {
        let (_dir, h) = tiny();
        let old = lines_of(&h, "let a\nfn b\n");
        let edited = old.carried_over("let a\nfn\n");
        assert_eq!(edited.lines[0], old.lines[0]);
        assert_eq!(edited.lines[1], [span(0..2, Role::SyntaxKeyword)]);
        let longer = old.carried_over("let a\nfn bcd\n");
        assert_eq!(longer.lines[1], old.lines[1], "kept as they were");
        // More new lines than old ones in the changed middle: the extra is plain.
        let split = old.carried_over("let a\nfn b\nmore\n");
        assert_eq!(split.lines.len(), 3);
        assert!(split.lines[2].is_empty());
        // Nothing to carry from.
        let none = super::Highlighted::default().carried_over("let a\n");
        assert_eq!(none.lines, [Vec::<Span>::new()]);
    }

    #[test]
    fn a_bad_file_is_one_notice_and_the_rest_loads() {
        let dir = folder(&[
            ("Tiny.sublime-syntax", TINY),
            ("Broken.sublime-syntax", "name: [unclosed"),
            ("notes.txt", "not a definition"),
        ]);
        let loaded = Highlighter::load_dir(dir.path());
        assert_eq!(loaded.notices.len(), 1, "{:?}", loaded.notices);
        assert!(loaded.notices[0].contains("Broken.sublime-syntax"));
        assert!(loaded.highlighter.highlight("tiny", "let\n").is_some());
    }

    #[test]
    fn a_missing_folder_is_a_notice_and_nothing_highlights() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Highlighter::load_dir(&dir.path().join("absent"));
        assert_eq!(loaded.notices.len(), 1);
        assert!(loaded.highlighter.highlight("tiny", "let\n").is_none());
    }
}
