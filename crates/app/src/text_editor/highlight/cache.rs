//! The spans of each code block, kept so highlighting runs on an edit and
//! never per frame. Keyed by the block's language and text, not by where it
//! sits: an edit changes only the key of the block it touches, and lines
//! moving above a block cost nothing.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::size_of;
use std::sync::Arc;

use super::syntax::{Highlighted, Highlighter, Span};

/// What the cache holds and what it will highlight at all.
#[derive(Clone, Copy, Debug)]
pub(in crate::text_editor) struct Limits {
    /// A longer block stays plain. Parsing costs about 0.2 ms per line.
    pub block_lines: usize,
    /// A larger block stays plain. Parsing is linear, about 6 µs per byte
    /// (also for one very long line), so this bounds the worst case at
    /// roughly 0.75 s.
    pub block_bytes: usize,
    /// Blocks kept; the least recently used goes first.
    pub entries: usize,
    /// Estimated memory of the kept spans.
    pub bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            block_lines: 2_000,
            block_bytes: 128 * 1024,
            entries: 256,
            bytes: 8 * 1024 * 1024,
        }
    }
}

/// A 64-bit hash plus the length: a collision would need both to agree, and
/// would only colour a block wrongly.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    hash: u64,
    len: usize,
}

struct Entry {
    spans: Arc<Highlighted>,
    size: usize,
    used: u64,
}

pub(in crate::text_editor) struct BlockCache {
    highlighter: Highlighter,
    limits: Limits,
    entries: HashMap<Key, Entry>,
    bytes: usize,
    tick: u64,
}

impl BlockCache {
    pub fn new(highlighter: Highlighter, limits: Limits) -> Self {
        assert!(limits.entries > 0 && limits.block_bytes > 0);
        Self {
            highlighter,
            limits,
            entries: HashMap::with_capacity(limits.entries),
            bytes: 0,
            tick: 0,
        }
    }

    /// The spans of the block `text` as `language`, highlighted now if they
    /// are not kept. `None` is plain: no definition knows the language, or the
    /// block is over the limits.
    pub fn spans(&mut self, language: &str, text: &str) -> Option<Arc<Highlighted>> {
        if text.len() > self.limits.block_bytes || line_count(text) > self.limits.block_lines {
            return None;
        }
        let key = key_of(language, text);
        self.tick += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.tick;
            return Some(Arc::clone(&entry.spans));
        }
        let spans = Arc::new(self.highlighter.highlight(language, text)?);
        self.insert(key, &spans);
        Some(spans)
    }

    /// How many blocks are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn insert(&mut self, key: Key, spans: &Arc<Highlighted>) {
        let size = estimate(spans);
        // One block larger than the whole budget is not kept.
        if size > self.limits.bytes {
            return;
        }
        while self.entries.len() >= self.limits.entries || self.bytes + size > self.limits.bytes {
            self.evict_oldest();
        }
        self.bytes += size;
        let used = self.tick;
        self.entries.insert(
            key,
            Entry {
                spans: Arc::clone(spans),
                size,
                used,
            },
        );
    }

    fn evict_oldest(&mut self) {
        let oldest = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| *k);
        let Some(key) = oldest else {
            // Nothing to drop but the budget says over: the books are wrong.
            debug_assert!(false, "cache over budget with no entries");
            self.bytes = 0;
            return;
        };
        if let Some(entry) = self.entries.remove(&key) {
            self.bytes -= entry.size;
        }
    }
}

fn line_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + usize::from(!text.ends_with('\n'))
}

fn key_of(language: &str, text: &str) -> Key {
    let mut hasher = DefaultHasher::new();
    language.hash(&mut hasher);
    text.hash(&mut hasher);
    Key {
        hash: hasher.finish(),
        len: text.len(),
    }
}

fn estimate(spans: &Highlighted) -> usize {
    let lines = spans.lines.len() * size_of::<Vec<Span>>();
    let spans: usize = spans
        .lines
        .iter()
        .map(|l| l.len() * size_of::<Span>())
        .sum();
    size_of::<Highlighted>() + lines + spans
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{BlockCache, Limits};
    use crate::text_editor::highlight::Highlighter;

    const TINY: &str = "%YAML 1.2\n---\nname: Tiny\nfile_extensions: [tiny]\nscope: source.tiny\n\
        contexts:\n  main:\n    - match: '\\b(?:let|fn)\\b'\n      scope: keyword.control.tiny\n";

    fn cache(limits: Limits) -> (tempfile::TempDir, BlockCache) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tiny.sublime-syntax"), TINY).unwrap();
        let loaded = Highlighter::load_dir(dir.path());
        assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
        (dir, BlockCache::new(loaded.highlighter, limits))
    }

    #[test]
    fn the_same_block_is_highlighted_once() {
        let (_dir, mut c) = cache(Limits::default());
        let a = c.spans("tiny", "let x\n").unwrap();
        let b = c.spans("tiny", "let x\n").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn an_edit_changes_only_its_own_block() {
        let (_dir, mut c) = cache(Limits::default());
        let first = c.spans("tiny", "let a\n").unwrap();
        let second = c.spans("tiny", "fn b\n").unwrap();
        let edited = c.spans("tiny", "let a2\n").unwrap();
        assert!(!Arc::ptr_eq(&first, &edited));
        assert!(Arc::ptr_eq(&second, &c.spans("tiny", "fn b\n").unwrap()));
    }

    #[test]
    fn the_language_is_part_of_the_key() {
        let (_dir, mut c) = cache(Limits::default());
        let a = c.spans("tiny", "let a\n").unwrap();
        let b = c.spans("TINY", "let a\n").unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn an_unknown_language_is_plain_and_not_kept() {
        let (_dir, mut c) = cache(Limits::default());
        assert!(c.spans("nope", "let a\n").is_none());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn a_block_over_the_limits_is_plain_and_one_at_them_is_not() {
        let limits = Limits {
            block_lines: 3,
            block_bytes: 12,
            ..Limits::default()
        };
        let (_dir, mut c) = cache(limits);
        assert!(c.spans("tiny", "let\nlet\nlet\n").is_some());
        assert!(c.spans("tiny", "let\nlet\nlet\nlet\n").is_none(), "4 lines");
        assert!(
            c.spans("tiny", "let\nlet\nlet").is_some(),
            "no trailing newline"
        );
        assert!(c.spans("tiny", "let aaaaaaaaaa\n").is_none(), "15 bytes");
    }

    #[test]
    fn the_least_recently_used_block_goes_first() {
        let limits = Limits {
            entries: 2,
            ..Limits::default()
        };
        let (_dir, mut c) = cache(limits);
        let a = c.spans("tiny", "let a\n").unwrap();
        c.spans("tiny", "let b\n").unwrap();
        c.spans("tiny", "let a\n").unwrap(); // a is now newer than b
        c.spans("tiny", "let c\n").unwrap(); // evicts b
        assert_eq!(c.len(), 2);
        assert!(Arc::ptr_eq(&a, &c.spans("tiny", "let a\n").unwrap()));
    }

    #[test]
    fn the_memory_budget_is_kept() {
        let limits = Limits {
            bytes: 600,
            ..Limits::default()
        };
        let (_dir, mut c) = cache(limits);
        for i in 0..20 {
            c.spans("tiny", &format!("let a{i}\n")).unwrap();
        }
        assert!(c.bytes <= 600 && c.len() < 20, "{} kept", c.len());
        let total: usize = c.entries.values().map(|e| e.size).sum();
        assert_eq!(total, c.bytes);
    }
}
