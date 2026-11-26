//! A Fenwick tree over block heights: prefix sum, point update and "which
//! block is at y" in O(log n). `f64`, because `f32` steps are a whole pixel
//! from about 8.4 million px on.

pub(crate) struct HeightMap {
    values: Vec<f64>,
    /// One-based; `tree[i]` sums the `i & -i` values ending at `i`.
    tree: Vec<f64>,
}

impl HeightMap {
    pub(crate) fn new(values: Vec<f64>) -> Self {
        assert!(!values.is_empty(), "a document has a block");
        let mut tree = vec![0.; values.len() + 1];
        for i in 1..=values.len() {
            tree[i] += values[i - 1];
            let parent = i + (i & i.wrapping_neg());
            if parent <= values.len() {
                let sum = tree[i];
                tree[parent] += sum;
            }
        }
        Self { values, tree }
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn height(&self, block: usize) -> f64 {
        self.values[block]
    }

    pub(crate) fn set(&mut self, block: usize, height: f64) {
        assert!(height > 0., "a block has a height");
        let delta = height - self.values[block];
        self.values[block] = height;
        let mut i = block + 1;
        while i < self.tree.len() {
            self.tree[i] += delta;
            i += i & i.wrapping_neg();
        }
    }

    /// The sum of the heights before `block`.
    pub(crate) fn before(&self, block: usize) -> f64 {
        let mut i = block;
        let mut sum = 0.;
        while i > 0 {
            sum += self.tree[i];
            i &= i - 1;
        }
        sum
    }

    pub(crate) fn total(&self) -> f64 {
        self.before(self.len())
    }

    /// The block covering `y` and the offset into it, clamped to the document.
    pub(crate) fn find(&self, y: f64) -> (usize, f64) {
        let mut remaining = y.max(0.);
        let mut index = 0;
        let mut step = (self.len() + 1).next_power_of_two() / 2;
        while step > 0 {
            let next = index + step;
            if next <= self.len() && self.tree[next] <= remaining {
                remaining -= self.tree[next];
                index = next;
            }
            step /= 2;
        }
        if index >= self.len() {
            let last = self.len() - 1;
            return (last, self.values[last]);
        }
        (index, remaining)
    }

    /// What the tree has drifted from a plain sum.
    pub(crate) fn drift(&self) -> f64 {
        (self.total() - self.values.iter().sum::<f64>()).abs()
    }
}
