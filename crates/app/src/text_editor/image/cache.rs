//! Decoded images, shared by every open note. A decode runs off the main
//! thread at no more than the width an image is drawn at (the full-size RGBA
//! of a photo would be tens of MB), and the cache holds decoded bytes to a
//! budget, dropping the least recently drawn images first.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::gpui::{Global, RenderImage};
use image::imageops::FilterType;
use image::{Frame, ImageReader, Limits};

/// Decoded bytes kept at most (an image drawn this frame is never dropped).
const BUDGET_BYTES: usize = 64 << 20;

/// Files larger than this are not decoded.
const MAX_FILE_BYTES: u64 = 64 << 20;

/// A decode may allocate at most this much.
const MAX_DECODE_BYTES: u64 = 256 << 20;

enum Slot {
    /// A decode at this width is running.
    Pending,
    Ready {
        image: Arc<RenderImage>,
        /// Pixel width it was decoded at.
        width: u32,
        bytes: usize,
        /// The frame it was last asked for in.
        used: u64,
        upgrading: bool,
    },
    /// Unreadable or undecodable (AVIF, SVG, corrupt): the placeholder stays.
    Failed,
}

#[derive(Default)]
pub(in crate::text_editor) struct Cache {
    slots: HashMap<Arc<Path>, Slot>,
    frame: u64,
    bytes: usize,
    /// Dropped from the cache but still in the GPU atlas until a window frees them.
    evicted: Vec<Arc<RenderImage>>,
}

impl Global for Cache {}

/// What [`Cache::lookup`] found.
pub(in crate::text_editor) struct Lookup {
    /// Something to draw: the decode, or an earlier smaller one while a
    /// larger is made.
    pub image: Option<Arc<RenderImage>>,
    /// The width to start a decode at, if one is needed and none is running.
    pub decode_at: Option<u32>,
}

impl Cache {
    /// Starts a frame. The returned images left the cache since the last one;
    /// the caller frees them from the window.
    pub(in crate::text_editor) fn begin_frame(&mut self) -> Vec<Arc<RenderImage>> {
        self.frame += 1;
        std::mem::take(&mut self.evicted)
    }

    /// The image at `path` for drawing `want` pixels wide, `natural` being the
    /// file's own width (more than that is never decoded).
    pub(in crate::text_editor) fn lookup(
        &mut self,
        path: &Arc<Path>,
        want: u32,
        natural: u32,
    ) -> Lookup {
        let want = want.min(natural).max(1);
        match self.slots.get_mut(path) {
            None => {
                self.slots.insert(path.clone(), Slot::Pending);
                Lookup {
                    image: None,
                    decode_at: Some(want),
                }
            }
            Some(Slot::Pending | Slot::Failed) => Lookup {
                image: None,
                decode_at: None,
            },
            Some(Slot::Ready {
                image,
                width,
                used,
                upgrading,
                ..
            }) => {
                *used = self.frame;
                let decode_at = (*width < want && !*upgrading).then(|| {
                    *upgrading = true;
                    want
                });
                Lookup {
                    image: Some(image.clone()),
                    decode_at,
                }
            }
        }
    }

    /// Stores a finished decode (`None`: it failed) and drops what no longer fits.
    pub(in crate::text_editor) fn finish(
        &mut self,
        path: &Arc<Path>,
        decoded: Option<(Arc<RenderImage>, u32)>,
    ) {
        if let Some(Slot::Ready { image, bytes, .. }) = self.slots.remove(path) {
            self.bytes -= bytes;
            self.evicted.push(image);
        }
        let Some((image, width)) = decoded else {
            self.slots.insert(path.clone(), Slot::Failed);
            return;
        };
        let bytes = image.as_bytes(0).map_or(0, <[u8]>::len);
        self.bytes += bytes;
        let slot = Slot::Ready {
            image,
            width,
            bytes,
            used: self.frame,
            upgrading: false,
        };
        self.slots.insert(path.clone(), slot);
        self.evict_to_budget();
    }

    fn evict_to_budget(&mut self) {
        while self.bytes > BUDGET_BYTES {
            let oldest = self
                .slots
                .iter()
                .filter_map(|(path, slot)| match slot {
                    Slot::Ready { used, .. } if *used < self.frame => Some((*used, path.clone())),
                    _ => None,
                })
                .min_by_key(|(used, _)| *used);
            let Some((_, path)) = oldest else {
                return;
            };
            if let Some(Slot::Ready { image, bytes, .. }) = self.slots.remove(&path) {
                self.bytes -= bytes;
                self.evicted.push(image);
            }
        }
    }

    /// Decoded bytes held.
    #[cfg(test)]
    pub(in crate::text_editor) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(in crate::text_editor) fn is_ready(&self, path: &Arc<Path>) -> bool {
        matches!(self.slots.get(path), Some(Slot::Ready { .. }))
    }
}

/// Decodes the file at `path`, scaled down to `max_width` pixels wide if it is
/// wider. Blocks: call it off the main thread. `None` when it cannot be read.
pub(in crate::text_editor) fn decode(
    path: &Path,
    max_width: u32,
) -> Option<(Arc<RenderImage>, u32)> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoded = reader.decode().ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width > max_width {
        let scaled = (u64::from(height) * u64::from(max_width) / u64::from(width)).max(1);
        decoded = decoded.resize_exact(max_width, scaled as u32, FilterType::Triangle);
    }
    let mut pixels = decoded.into_rgba8();
    let width = pixels.width();
    // GPUI draws BGRA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let image = RenderImage::new(vec![Frame::new(pixels)]);
    Some((Arc::new(image), width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn render_image(width: u32, height: u32) -> Arc<RenderImage> {
        let frame = Frame::new(RgbaImage::new(width, height));
        Arc::new(RenderImage::new(vec![frame]))
    }

    fn path(name: &str) -> Arc<Path> {
        Path::new(name).into()
    }

    #[test]
    fn the_first_lookup_asks_for_a_decode_and_the_next_waits() {
        let mut cache = Cache::default();
        let p = path("a.png");
        let first = cache.lookup(&p, 300, 1000);
        assert_eq!((first.image.is_none(), first.decode_at), (true, Some(300)));
        let second = cache.lookup(&p, 300, 1000);
        assert_eq!((second.image.is_none(), second.decode_at), (true, None));
    }

    #[test]
    fn never_asks_for_more_than_the_files_own_width() {
        let mut cache = Cache::default();
        assert_eq!(cache.lookup(&path("a.png"), 900, 400).decode_at, Some(400));
    }

    #[test]
    fn a_wider_draw_upgrades_once_and_keeps_showing_the_old() {
        let mut cache = Cache::default();
        let p = path("a.png");
        cache.lookup(&p, 100, 1000);
        cache.finish(&p, Some((render_image(100, 50), 100)));
        let wider = cache.lookup(&p, 200, 1000);
        assert!(wider.image.is_some());
        assert_eq!(wider.decode_at, Some(200));
        assert_eq!(
            cache.lookup(&p, 200, 1000).decode_at,
            None,
            "already running"
        );
        assert_eq!(
            cache.lookup(&p, 50, 1000).decode_at,
            None,
            "smaller needs nothing"
        );
    }

    #[test]
    fn a_failed_decode_is_not_retried() {
        let mut cache = Cache::default();
        let p = path("a.avif");
        cache.lookup(&p, 100, 100);
        cache.finish(&p, None);
        let again = cache.lookup(&p, 100, 100);
        assert_eq!((again.image.is_none(), again.decode_at), (true, None));
    }

    #[test]
    fn over_budget_drops_the_least_recently_drawn_but_never_this_frames() {
        let mut cache = Cache::default();
        // Each is 4 bytes a pixel: three of these exceed the budget.
        let side = (BUDGET_BYTES / 4 / 2) as f64;
        let side = side.sqrt() as u32;
        let (a, b, c) = (path("a.png"), path("b.png"), path("c.png"));
        for p in [&a, &b] {
            cache.begin_frame();
            cache.lookup(p, side, side);
            cache.finish(p, Some((render_image(side, side), side)));
        }
        assert!(cache.is_ready(&a) && cache.is_ready(&b));
        cache.begin_frame();
        cache.lookup(&b, side, side);
        cache.lookup(&c, side, side);
        cache.finish(&c, Some((render_image(side, side), side)));
        assert!(!cache.is_ready(&a), "least recently drawn goes");
        assert!(cache.is_ready(&b) && cache.is_ready(&c));
        assert!(cache.bytes() <= BUDGET_BYTES);
        assert_eq!(
            cache.begin_frame().len(),
            1,
            "freed from the window next frame"
        );
    }
}
