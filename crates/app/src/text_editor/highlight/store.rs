//! The loaded definitions and block cache that every editor shares, set up
//! the first time a highlighted fence needs them.

use std::path::PathBuf;

use super::cache::{BlockCache, Limits};
use super::syntax::Highlighter;

pub(in crate::text_editor) struct Syntaxes {
    folder: Option<PathBuf>,
    cache: Option<BlockCache>,
}

impl Syntaxes {
    /// Nothing is read until [`Syntaxes::cache`]; no folder means no colours.
    pub fn new(folder: Option<PathBuf>) -> Self {
        Self {
            folder,
            cache: None,
        }
    }

    pub fn folder(&self) -> Option<&std::path::Path> {
        self.folder.as_deref()
    }

    /// The cache over the folder's definitions, loading them on first use (one
    /// stderr line per bad file). `None` without a folder.
    pub fn cache(&mut self) -> Option<&mut BlockCache> {
        let folder = self.folder.as_deref()?;
        Some(self.cache.get_or_insert_with(|| {
            let loaded = Highlighter::load_dir(folder);
            for notice in &loaded.notices {
                eprintln!("syntax: {notice}");
            }
            BlockCache::new(loaded.highlighter, Limits::default())
        }))
    }
}
