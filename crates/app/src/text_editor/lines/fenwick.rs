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
//! A same-line-count edit (plain typing/deletion within a line) is a pure
//! point update (`add_at`), O(log n). A line-count-changing edit (Enter, a
//! multi-line paste, deleting across a newline) uses [`Fenwick::resize`]:
//! an in-place O(n) splice (a few memmoves, no allocation), still far
//! cheaper than re-deriving every line's value.

/// Spare element capacity kept past a per-line `Vec`'s length, so Enter
/// doesn't reallocate it (a doubling realloc of a multi-MB array leaves a
/// freed hole the allocator keeps).
pub(in crate::text_editor) const LINE_GROWTH_SLACK: usize = 4096;

pub(in crate::text_editor) struct Fenwick {
    /// 1-indexed internally (`tree[0]` unused) — the standard Fenwick tree
    /// layout, so every index arithmetic below matches textbook
    /// descriptions of the structure.
    tree: Vec<i64>,
}

impl Fenwick {
    pub(in crate::text_editor) fn new(values: &[i64]) -> Self {
        let mut tree = Vec::with_capacity(values.len() + 1 + LINE_GROWTH_SLACK);
        tree.push(0);
        tree.extend_from_slice(values);
        let mut fenwick = Self { tree };
        fenwick.build_in_place();
        fenwick
    }

    /// Like [`Self::new`], but takes ownership of `values` and reuses its
    /// buffer instead of copying it (a second multi-MB allocation).
    pub(in crate::text_editor) fn from_vec(mut values: Vec<i64>) -> Self {
        values.reserve_exact(1 + LINE_GROWTH_SLACK);
        values.insert(0, 0);
        let mut fenwick = Self { tree: values };
        fenwick.build_in_place();
        fenwick
    }

    /// Raw values -> tree, O(n): each node adds itself into its parent.
    fn build_in_place(&mut self) {
        let n = self.len();
        for i in 1..=n {
            let parent = i + lowest_set_bit(i);
            if parent <= n {
                let v = self.tree[i];
                self.tree[parent] += v;
            }
        }
    }

    /// Exact inverse of [`Self::build_in_place`]: undo its steps in
    /// reverse order, leaving the raw values.
    fn unbuild_in_place(&mut self) {
        let n = self.len();
        for i in (1..=n).rev() {
            let parent = i + lowest_set_bit(i);
            if parent <= n {
                let v = self.tree[i];
                self.tree[parent] -= v;
            }
        }
    }

    /// Inserts `delta` zero values at index `at` (`delta > 0`), or removes
    /// `-delta` values starting at `at` (`delta < 0`). O(n), in place.
    pub(in crate::text_editor) fn resize(&mut self, at: usize, delta: isize) {
        let n = self.len();
        assert!(at <= n, "resize position {at} past length {n}");
        if delta == 0 {
            return;
        }
        self.unbuild_in_place();
        if delta > 0 {
            let k = delta as usize;
            if self.tree.capacity() - self.tree.len() < k {
                self.tree.reserve_exact(k.max(LINE_GROWTH_SLACK));
            }
            self.tree.splice(at + 1..at + 1, std::iter::repeat_n(0, k));
        } else {
            let k = delta.unsigned_abs();
            assert!(at + k <= n, "cannot remove {k} values at {at} of {n}");
            self.tree.drain(at + 1..at + 1 + k);
        }
        self.build_in_place();
    }

    pub(in crate::text_editor) fn len(&self) -> usize {
        self.tree.len() - 1
    }

    /// Adds `delta` to the value at index `i` — the one point update every
    /// later prefix-sum query downstream of `i` needs to reflect.
    pub(in crate::text_editor) fn add_at(&mut self, i: usize, delta: i64) {
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
    pub(in crate::text_editor) fn prefix_sum(&self, i: usize) -> i64 {
        let mut i = i;
        let mut sum = 0;
        while i > 0 {
            sum += self.tree[i];
            i -= lowest_set_bit(i);
        }
        sum
    }

    /// `prefix_sum(i)` for every `i` in `0..=len`, in one O(n) sweep (each
    /// node's prefix is its parent-in-the-prefix-chain's prefix plus its own
    /// partial sum) instead of `len` separate O(log n) queries.
    pub(in crate::text_editor) fn prefix_sums(&self) -> Vec<i64> {
        let n = self.len();
        let mut sums = Vec::with_capacity(n + 1);
        sums.push(0);
        for i in 1..=n {
            sums.push(sums[i - lowest_set_bit(i)] + self.tree[i]);
        }
        sums
    }

    pub(in crate::text_editor) fn total(&self) -> i64 {
        self.prefix_sum(self.len())
    }

    /// The index `i` such that `prefix_sum(i) <= target < prefix_sum(i + 1)`
    /// — e.g. "which line contains byte offset `target`," given this tree's
    /// values are per-line byte strides. `None` if `target` is negative or
    /// past the tree's total (including exactly at the total, which is one
    /// past the last index's own range).
    pub(in crate::text_editor) fn find(&self, target: i64) -> Option<usize> {
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
        assert_eq!(
            fenwick.prefix_sum(2),
            2,
            "unaffected: before the updated index"
        );
        assert_eq!(
            fenwick.prefix_sum(3),
            13,
            "includes the updated index's new value"
        );
        assert_eq!(fenwick.prefix_sum(5), 15);
    }

    #[test]
    fn find_locates_the_index_whose_range_contains_the_target() {
        // byte strides, as if 4 lines of lengths 3, 0 (empty line), 5, 2.
        let values = [3i64, 0, 5, 2];
        let fenwick = Fenwick::new(&values);
        let reference = Reference(values.to_vec());
        for target in 0..values.iter().sum::<i64>() {
            assert_eq!(
                fenwick.find(target),
                reference.find(target),
                "at target={target}"
            );
        }
    }

    #[test]
    fn find_returns_none_past_the_total_or_for_a_negative_target() {
        let fenwick = Fenwick::new(&[1i64, 2, 3]);
        assert_eq!(fenwick.find(-1), None);
        assert_eq!(
            fenwick.find(6),
            None,
            "6 is the total — one past the last valid index's range"
        );
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
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
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
                assert_eq!(
                    fenwick.prefix_sum(check_i),
                    reference.prefix_sum(check_i),
                    "n={n}"
                );

                let total = reference.prefix_sum(n);
                if total > 0 {
                    let target = (next().unsigned_abs() as i64) % total;
                    assert_eq!(
                        fenwick.find(target),
                        reference.find(target),
                        "n={n}, target={target}"
                    );
                }
            }
        }
    }

    #[test]
    fn resize_matches_rebuilding_from_the_spliced_values() {
        let mut seed: u64 = 0x5EED_F00D_1234_5678;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        let mut values: Vec<i64> = (0..40).map(|_| (next() % 50) as i64).collect();
        let mut tree = Fenwick::new(&values);
        for step in 0..500 {
            let at = (next() as usize) % (values.len() + 1);
            let delta: isize = if values.len() - at > 0 && next() % 2 == 0 {
                -((1 + (next() as usize) % (values.len() - at).min(5)) as isize)
            } else {
                1 + (next() % 5) as isize
            };
            if delta > 0 {
                for _ in 0..delta {
                    values.insert(at, 0);
                }
            } else {
                values.drain(at..at + delta.unsigned_abs());
            }
            tree.resize(at, delta);
            // Interleave point updates so later steps resize a tree that
            // has seen real edits, not only zeros.
            if !values.is_empty() {
                let i = (next() as usize) % values.len();
                let d = (next() % 9) as i64 - 4;
                values[i] += d;
                tree.add_at(i, d);
            }
            let fresh = Fenwick::new(&values);
            assert_eq!(tree.len(), fresh.len(), "step {step}");
            for i in 0..=values.len() {
                assert_eq!(tree.prefix_sum(i), fresh.prefix_sum(i), "step {step} i {i}");
            }
        }
    }

    #[test]
    fn prefix_sums_matches_prefix_sum_at_every_index() {
        for n in [0usize, 1, 2, 7, 8, 9, 100, 1000] {
            let values: Vec<i64> = (0..n as i64).map(|v| (v * 37) % 11 - 3).collect();
            let fenwick = Fenwick::new(&values);
            let sums = fenwick.prefix_sums();
            assert_eq!(sums.len(), n + 1);
            for (i, &sum) in sums.iter().enumerate() {
                assert_eq!(sum, fenwick.prefix_sum(i), "n={n} i={i}");
            }
        }
    }
}
