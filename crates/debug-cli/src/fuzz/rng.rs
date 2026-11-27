//! A small seeded random number generator (SplitMix64), so a run is repeated
//! by its seed. Not for anything but test input.

pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`; `n` must not be 0. The slight bias towards small
    /// numbers does not matter here.
    pub(super) fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A number in `low..=high`.
    pub(super) fn between(&mut self, low: u32, high: u32) -> u32 {
        assert!(low <= high);
        low + self.below(u64::from(high - low) + 1) as u32
    }

    pub(super) fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// One of `items`, which must not be empty.
    pub(super) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}
