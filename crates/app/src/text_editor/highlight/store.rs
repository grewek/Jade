//! What every editor shares about highlighting: the definitions (read on a
//! background task when first needed), the block cache, and the parses in
//! flight. GPUI-free; the editor spawns the tasks and reports back here.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::cache::{BlockCache, Key, Limits, Lookup};
use super::syntax::{Highlighted, Highlighter};

/// Parses running at once. Each is a whole core for up to a second on a huge
/// block, and the UI thread must keep a core.
pub(in crate::text_editor) const MAX_JOBS: usize = 4;

enum State {
    Unloaded,
    Loading,
    Ready(Arc<Highlighter>),
}

/// A parse that was started: set the flag to make it give up.
pub(in crate::text_editor) type Cancel = Arc<AtomicBool>;

pub(in crate::text_editor) struct Syntaxes {
    folder: Option<PathBuf>,
    state: State,
    cache: BlockCache,
    jobs: HashMap<Key, Cancel>,
    /// Languages already reported as broken, lower case.
    reported: HashSet<String>,
}

impl Syntaxes {
    /// Nothing is read until [`Syntaxes::start_load`]; no folder means no colours.
    pub fn new(folder: Option<PathBuf>) -> Self {
        Self::with_limits(folder, Limits::default())
    }

    pub fn with_limits(folder: Option<PathBuf>, limits: Limits) -> Self {
        Self {
            folder,
            state: State::Unloaded,
            cache: BlockCache::new(limits),
            jobs: HashMap::new(),
            reported: HashSet::new(),
        }
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The folder to read, once: moves from unloaded to loading. `None` when
    /// there is no folder or the read has started.
    pub fn start_load(&mut self) -> Option<PathBuf> {
        if !matches!(self.state, State::Unloaded) {
            return None;
        }
        let folder = self.folder.clone()?;
        self.state = State::Loading;
        Some(folder)
    }

    pub fn finish_load(&mut self, highlighter: Highlighter) {
        self.state = State::Ready(Arc::new(highlighter));
    }

    /// The definitions, once they are read.
    pub fn highlighter(&self) -> Option<Arc<Highlighter>> {
        match &self.state {
            State::Ready(highlighter) => Some(Arc::clone(highlighter)),
            _ => None,
        }
    }

    /// What is kept for a block; plain until the definitions are read.
    pub fn lookup(&mut self, language: &str, text: &str) -> Lookup {
        match &self.state {
            State::Ready(highlighter) => self.cache.lookup(highlighter, language, text),
            _ => Lookup::Plain,
        }
    }

    /// Registers a parse of `key` unless one runs or [`MAX_JOBS`] do; the
    /// token to cancel it with when it may start.
    pub fn begin(&mut self, key: Key) -> Option<Cancel> {
        if self.jobs.len() >= MAX_JOBS || self.jobs.contains_key(&key) {
            return None;
        }
        let token = Cancel::default();
        self.jobs.insert(key, Arc::clone(&token));
        Some(token)
    }

    /// Cancels the parses of blocks no longer wanted (edited, or scrolled
    /// away), so they stop and make room.
    pub fn cancel_unwanted(&mut self, wanted: &HashSet<Key>) {
        self.jobs.retain(|key, token| {
            let keep = wanted.contains(key);
            if !keep {
                token.store(true, Ordering::Relaxed);
            }
            keep
        });
    }

    /// A parse ended: its spans are kept when it finished. A job that was
    /// cancelled and replaced by a newer one for the same block does not
    /// remove the newer one.
    pub fn finish(&mut self, key: Key, token: &Cancel, spans: Option<Highlighted>) {
        if self.jobs.get(&key).is_some_and(|t| Arc::ptr_eq(t, token)) {
            self.jobs.remove(&key);
        }
        if let Some(spans) = spans {
            self.cache.insert(key, &Arc::new(spans));
        }
    }

    /// `true` the first time `language` is asked about, so a broken
    /// definition is reported once and not on every parse of every block.
    pub fn first_report(&mut self, language: &str) -> bool {
        self.reported.insert(language.to_ascii_lowercase())
    }

    pub fn jobs(&self) -> usize {
        self.jobs.len()
    }

    /// How many blocks have their spans kept.
    pub fn cached(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;

    use super::{MAX_JOBS, Syntaxes};
    use crate::text_editor::highlight::cache::Lookup;
    use crate::text_editor::highlight::{Highlighted, Highlighter};

    const TINY: &str = "%YAML 1.2\n---\nname: Tiny\nfile_extensions: [tiny]\nscope: source.tiny\n\
        contexts:\n  main:\n    - match: '\\b(?:let|fn)\\b'\n      scope: keyword.control.tiny\n";

    fn ready() -> (tempfile::TempDir, Syntaxes) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tiny.sublime-syntax"), TINY).unwrap();
        let mut store = Syntaxes::new(Some(dir.path().to_path_buf()));
        assert_eq!(store.start_load().as_deref(), Some(dir.path()));
        store.finish_load(Highlighter::load_dir(dir.path()).highlighter);
        (dir, store)
    }

    fn miss(store: &mut Syntaxes, text: &str) -> super::Key {
        match store.lookup("tiny", text) {
            Lookup::Miss(key) => key,
            _ => panic!("expected a miss for {text:?}"),
        }
    }

    #[test]
    fn the_load_starts_once_and_blocks_are_plain_until_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Tiny.sublime-syntax"), TINY).unwrap();
        let mut store = Syntaxes::new(Some(dir.path().to_path_buf()));
        assert!(matches!(store.lookup("tiny", "let\n"), Lookup::Plain));
        assert!(store.start_load().is_some());
        assert!(store.start_load().is_none(), "already loading");
        assert!(matches!(store.lookup("tiny", "let\n"), Lookup::Plain));
        assert!(store.highlighter().is_none());
        store.finish_load(Highlighter::load_dir(dir.path()).highlighter);
        assert!(matches!(store.lookup("tiny", "let\n"), Lookup::Miss(_)));
        assert!(Syntaxes::new(None).start_load().is_none(), "no folder");
    }

    #[test]
    fn a_finished_parse_turns_a_miss_into_a_hit() {
        let (_dir, mut store) = ready();
        let key = miss(&mut store, "let a\n");
        let token = store.begin(key).unwrap();
        assert!(store.begin(key).is_none(), "one parse per block");
        let spans = store.highlighter().unwrap().highlight("tiny", "let a\n");
        store.finish(key, &token, spans);
        assert_eq!(store.jobs(), 0);
        assert!(matches!(store.lookup("tiny", "let a\n"), Lookup::Hit(_)));
    }

    #[test]
    fn at_most_max_jobs_run_and_a_finished_one_makes_room() {
        let (_dir, mut store) = ready();
        let texts: Vec<String> = (0..=MAX_JOBS).map(|i| format!("let a{i}\n")).collect();
        let keys: Vec<_> = texts.iter().map(|t| miss(&mut store, t)).collect();
        let tokens: Vec<_> = keys[..MAX_JOBS]
            .iter()
            .map(|&k| store.begin(k).unwrap())
            .collect();
        assert!(store.begin(keys[MAX_JOBS]).is_none());
        store.finish(keys[0], &tokens[0], None);
        assert!(store.begin(keys[MAX_JOBS]).is_some());
    }

    #[test]
    fn a_job_no_longer_wanted_is_cancelled_and_the_others_are_not() {
        let (_dir, mut store) = ready();
        let (a, b) = (miss(&mut store, "let a\n"), miss(&mut store, "let b\n"));
        let (ta, tb) = (store.begin(a).unwrap(), store.begin(b).unwrap());
        store.cancel_unwanted(&HashSet::from([b]));
        assert!(ta.load(Ordering::Relaxed) && !tb.load(Ordering::Relaxed));
        assert_eq!(store.jobs(), 1);
    }

    #[test]
    fn a_language_is_reported_once_whatever_its_case() {
        let mut store = Syntaxes::new(None);
        assert!(store.first_report("Rust"));
        assert!(!store.first_report("rust"));
        assert!(!store.first_report("RUST"));
        assert!(store.first_report("python"));
    }

    #[test]
    fn a_cancelled_job_ending_late_does_not_remove_its_replacement() {
        let (_dir, mut store) = ready();
        let key = miss(&mut store, "let a\n");
        let old = store.begin(key).unwrap();
        store.cancel_unwanted(&HashSet::new());
        let new = store.begin(key).unwrap();
        store.finish(key, &old, None);
        assert_eq!(store.jobs(), 1, "the replacement still runs");
        store.finish(key, &new, Some(Highlighted::default()));
        assert_eq!(store.jobs(), 0);
    }
}
