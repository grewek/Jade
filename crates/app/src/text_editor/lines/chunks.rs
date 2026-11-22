//! Running one pass over a big note on several threads, each over a run of
//! whole lines. The cold open builds several per-line tables (word counts,
//! the link table); they share this so the cut points, the threading and the
//! thresholds are decided once.

/// Below this a thread per core costs more than it saves.
pub(in crate::text_editor) const PARALLEL_MIN_BYTES: usize = 1 << 20;
const MAX_THREADS: usize = 8;

/// The byte offsets that cut `text` into `threads` runs of whole lines:
/// `0`, then each cut just after a newline, then `text.len()`. Runs may be
/// empty when a single line spans several cut guesses.
fn cuts(text: &str, threads: usize) -> Vec<usize> {
    let mut cuts = Vec::with_capacity(threads + 1);
    cuts.push(0);
    for k in 1..threads {
        let guess = text.len() / threads * k;
        let cut = text[guess..]
            .find('\n')
            .map_or(text.len(), |i| guess + i + 1);
        cuts.push(cut.max(cuts[k - 1]));
    }
    cuts.push(text.len());
    cuts
}

/// Calls `scan(first_line, chunk, is_last)` on each run of whole lines of
/// `text` (`is_last` marks the one that ends the document) and returns the
/// results in document order. `line_at` maps a run's starting byte offset to
/// its line number; it runs on the calling thread, so it may borrow what the
/// workers cannot share (pass `|_| 0` when the line number is not needed).
/// Big notes run the runs on one thread each; small ones on the calling
/// thread.
pub(in crate::text_editor) fn map_line_chunks<T: Send>(
    text: &str,
    line_at: impl Fn(usize) -> usize,
    scan: impl Fn(usize, &str, bool) -> T + Sync,
) -> Vec<T> {
    let threads = if text.len() < PARALLEL_MIN_BYTES {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS))
    };
    if threads == 1 {
        return vec![scan(0, text, true)];
    }
    let cuts = cuts(text, threads);
    let scan = &scan;
    std::thread::scope(|scope| {
        let handles: Vec<_> = cuts
            .windows(2)
            .enumerate()
            .map(|(k, w)| {
                let chunk = &text[w[0]..w[1]];
                let last = k + 2 == cuts.len();
                let first = if w[0] == 0 { 0 } else { line_at(w[0]) };
                scope.spawn(move || scan(first, chunk, last))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the cut points hit (a blank run, one huge line, either
    /// ending), the chunks concatenate to the text, each starts a line, and
    /// only the last is marked last.
    #[test]
    fn chunks_tile_the_text_on_line_starts() {
        let unit = "word\n\n\nanother line here\nlast";
        let long_line = "x ".repeat(PARALLEL_MIN_BYTES);
        let texts = [
            unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2),
            format!("{}\n", unit.repeat(PARALLEL_MIN_BYTES / unit.len() + 2)),
            "\n".repeat(PARALLEL_MIN_BYTES + 5),
            format!("start\n{long_line}\nend"),
            long_line,
        ];
        for text in &texts {
            assert!(text.len() >= PARALLEL_MIN_BYTES);
            let chunks = map_line_chunks(
                text,
                |start| text[..start].matches('\n').count(),
                |first, chunk, last| (first, chunk.to_string(), last),
            );
            let mut at = 0;
            for (i, (first, chunk, last)) in chunks.iter().enumerate() {
                assert_eq!(*first, text[..at].matches('\n').count());
                assert!(at == 0 || at == text.len() || text.as_bytes()[at - 1] == b'\n');
                assert_eq!(*last, i + 1 == chunks.len());
                at += chunk.len();
            }
            assert_eq!(at, text.len());
        }
    }

    #[test]
    fn a_small_text_is_one_chunk_on_the_calling_thread() {
        let chunks = map_line_chunks(
            "a\nb",
            |_| 0,
            |first, chunk, last| (first, chunk.len(), last),
        );
        assert_eq!(chunks, [(0, 3, true)]);
    }
}
