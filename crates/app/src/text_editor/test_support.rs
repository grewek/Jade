//! Helpers the editor's randomized tests share: a seeded generator and edits
//! built the way the real mutation sites build them.

use super::document_cache::TextEdit;

/// xorshift64: deterministic, so a failing seed reproduces.
pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        assert_ne!(seed, 0, "xorshift is stuck at zero");
        Rng(seed)
    }

    /// A value in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// Replaces `start..old_end` of `text` with `insert`: the new text and the
/// `TextEdit` describing it, as `editor.rs` and `input_handler.rs` build one.
pub(super) fn apply_edit(
    text: &str,
    start: usize,
    old_end: usize,
    insert: &str,
) -> (String, TextEdit) {
    let mut new_text = String::with_capacity(text.len() - (old_end - start) + insert.len());
    new_text.push_str(&text[..start]);
    new_text.push_str(insert);
    new_text.push_str(&text[old_end..]);
    let edit = TextEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: start + insert.len(),
    };
    (new_text, edit)
}

/// A random edit of `text`: up to 12 bytes at a random character boundary
/// replaced by up to three tokens of `alphabet`.
pub(super) fn random_edit(text: &str, rng: &mut Rng, alphabet: &[&str]) -> (String, TextEdit) {
    let starts: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let a = starts[rng.below(starts.len())];
    let b = starts[rng.below(starts.len())];
    let (start, far) = (a.min(b), a.max(b).min(a.min(b) + 12));
    let old_end = starts.iter().copied().filter(|&c| c <= far).max().unwrap();
    let insert: String = (0..rng.below(4))
        .map(|_| alphabet[rng.below(alphabet.len())])
        .collect();
    apply_edit(text, start, old_end, &insert)
}
