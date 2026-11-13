//! A Fenwick tree (binary indexed tree) over a sequence of `i64` values,
//! supporting O(log n) point-update and O(log n) prefix-sum query. This is
//! the shared structure behind the incremental per-line byte-offset index
//! in `document_cache.rs` (mapping an edit's byte offset to a line index
//! without an O(document) scan) — see that module's own doc comment, and
//! CLAUDE.md's Backlog entry on incremental classification for why this
//! exists at all: resuming a classifier from the *edited* line still needs
//! to find which line that is, and counting newlines from the start of the
//! document to get there would just reintroduce the O(document) cost this
//! whole effort exists to remove. Also the structure a later increment
//! reuses for per-line *height* (CLAUDE.md's own Fenwick-tree mention) —
//! same shape, different per-line value.
//!
//! Deliberately **fixed-size**: this tree has no insert/remove operation.
//! A same-line-count edit (plain typing/deletion within a line) is a pure
//! point update (`add_at`), O(log n). A line-count-changing edit (Enter, a
//! multi-line paste, deleting across a newline) has to rebuild from the
//! edited line onward instead — callers are expected to do that themselves
//! (e.g. via `Fenwick::new` on the affected suffix) rather than this type
//! growing/shrinking in place.

pub(super) struct Fenwick {
    /// 1-indexed internally (`tree[0]` unused) — the standard Fenwick tree
    /// layout, so every index arithmetic below matches textbook
    /// descriptions of the structure.
    tree: Vec<i64>,
}

impl Fenwick {
    pub(super) fn new(values: &[i64]) -> Self {
        let mut fenwick = Self { tree: vec![0; values.len() + 1] };
        for (i, &v) in values.iter().enumerate() {
            fenwick.add_at(i, v);
        }
        fenwick
    }

    pub(super) fn len(&self) -> usize {
        self.tree.len() - 1
    }

    /// Adds `delta` to the value at index `i` — the one point update every
    /// later prefix-sum query downstream of `i` needs to reflect.
    pub(super) fn add_at(&mut self, i: usize, delta: i64) {
        if delta == 0 {
            return;
        }
        let mut i = i + 1;
        while i < self.tree.len() {
            self.tree[i] += delta;
            i += lowest_set_bit(i);
        }
    }

    /// Sum of values at indices `0..i` (exclusive) — e.g. `prefix_sum(k)` is
    /// the byte offset line `k` begins at, when this tree's values are
    /// per-line byte strides.
    pub(super) fn prefix_sum(&self, i: usize) -> i64 {
        let mut i = i;
        let mut sum = 0;
        while i > 0 {
            sum += self.tree[i];
            i -= lowest_set_bit(i);
        }
        sum
    }

    pub(super) fn total(&self) -> i64 {
        self.prefix_sum(self.len())
    }

    /// The index `i` such that `prefix_sum(i) <= target < prefix_sum(i + 1)`
    /// — e.g. "which line contains byte offset `target`," given this tree's
    /// values are per-line byte strides. `None` if `target` is negative or
    /// past the tree's total (including exactly at the total, which is one
    /// past the last index's own range).
    pub(super) fn find(&self, target: i64) -> Option<usize> {
        if target < 0 || target >= self.total() {
            return None;
        }
        let mut pos = 0usize;
        let mut remaining = target;
        let mut step = highest_power_of_two_at_most(self.len());
        while step > 0 {
            let next = pos + step;
            if next <= self.len() && self.tree[next] <= remaining {
                pos = next;
                remaining -= self.tree[next];
            }
            step /= 2;
        }
        Some(pos)
    }
}

fn lowest_set_bit(i: usize) -> usize {
    i.isolate_lowest_one()
}

fn highest_power_of_two_at_most(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    1usize << (usize::BITS - 1 - n.leading_zeros())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Brute-force reference: a plain `Vec<i64>` with the same operations,
    /// recomputed from scratch each time — what every assertion below
    /// cross-checks the Fenwick tree against.
    struct Reference(Vec<i64>);

    impl Reference {
        fn prefix_sum(&self, i: usize) -> i64 {
            self.0[..i].iter().sum()
        }
        fn find(&self, target: i64) -> Option<usize> {
            if target < 0 {
                return None;
            }
            let mut acc = 0i64;
            for (i, &v) in self.0.iter().enumerate() {
                if target < acc + v {
                    return Some(i);
                }
                acc += v;
            }
            None
        }
    }

    #[test]
    fn prefix_sum_matches_a_plain_running_total() {
        let values = [3i64, 1, 4, 1, 5, 9, 2, 6];
        let fenwick = Fenwick::new(&values);
        let reference = Reference(values.to_vec());
        for i in 0..=values.len() {
            assert_eq!(fenwick.prefix_sum(i), reference.prefix_sum(i), "at i={i}");
        }
    }

    #[test]
    fn total_is_the_sum_of_every_value() {
        let values = [10i64, 20, 30];
        assert_eq!(Fenwick::new(&values).total(), 60);
    }

    #[test]
    fn add_at_updates_every_downstream_prefix_sum() {
        let mut fenwick = Fenwick::new(&[1i64, 1, 1, 1, 1]);
        fenwick.add_at(2, 10);
        assert_eq!(fenwick.prefix_sum(2), 2, "unaffected: before the updated index");
        assert_eq!(fenwick.prefix_sum(3), 13, "includes the updated index's new value");
        assert_eq!(fenwick.prefix_sum(5), 15);
    }

    #[test]
    fn find_locates_the_index_whose_range_contains_the_target() {
        // byte strides, as if 4 lines of lengths 3, 0 (empty line), 5, 2.
        let values = [3i64, 0, 5, 2];
        let fenwick = Fenwick::new(&values);
        let reference = Reference(values.to_vec());
        for target in 0..values.iter().sum::<i64>() {
            assert_eq!(fenwick.find(target), reference.find(target), "at target={target}");
        }
    }

    #[test]
    fn find_returns_none_past_the_total_or_for_a_negative_target() {
        let fenwick = Fenwick::new(&[1i64, 2, 3]);
        assert_eq!(fenwick.find(-1), None);
        assert_eq!(fenwick.find(6), None, "6 is the total — one past the last valid index's range");
        assert_eq!(fenwick.find(100), None);
    }

    #[test]
    fn find_on_an_all_zero_tree_is_always_none() {
        let fenwick = Fenwick::new(&[0i64, 0, 0]);
        assert_eq!(fenwick.find(0), None);
    }

    #[test]
    fn single_element_tree_round_trips() {
        let mut fenwick = Fenwick::new(&[5i64]);
        assert_eq!(fenwick.prefix_sum(0), 0);
        assert_eq!(fenwick.prefix_sum(1), 5);
        assert_eq!(fenwick.find(0), Some(0));
        assert_eq!(fenwick.find(4), Some(0));
        fenwick.add_at(0, 3);
        assert_eq!(fenwick.prefix_sum(1), 8);
    }

    #[test]
    fn random_sequence_of_updates_and_queries_matches_the_reference() {
        // Deterministic, hand-rolled LCG — no `rand` dependency (none is
        // approved for this crate) — enough spread to exercise Fenwick's
        // carry/borrow bit-manipulation across many tree shapes/sizes.
        let mut seed: u64 = 0x5EED_5EED_5EED_5EED;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as i64
        };

        for n in [1usize, 2, 3, 7, 16, 37, 100] {
            let initial: Vec<i64> = (0..n).map(|_| next() % 50).collect();
            let mut fenwick = Fenwick::new(&initial);
            let mut reference = Reference(initial);

            for _ in 0..200 {
                let i = (next().unsigned_abs() as usize) % n;
                // Clamped so every value stays non-negative throughout —
                // `find`'s descent (like any Fenwick "find by prefix sum")
                // only works correctly over non-negative values, which is
                // also the only case that matters for this tree's real use
                // (byte lengths are never negative).
                let delta = (next() % 40 - 20).max(-reference.0[i]);
                fenwick.add_at(i, delta);
                reference.0[i] += delta;

                let check_i = (next().unsigned_abs() as usize) % (n + 1);
                assert_eq!(fenwick.prefix_sum(check_i), reference.prefix_sum(check_i), "n={n}");

                let total = reference.prefix_sum(n);
                if total > 0 {
                    let target = (next().unsigned_abs() as i64) % total;
                    assert_eq!(fenwick.find(target), reference.find(target), "n={n}, target={target}");
                }
            }
        }
    }
}
