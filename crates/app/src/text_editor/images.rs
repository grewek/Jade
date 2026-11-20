//! What an image block needs to know about its file: the natural size, read
//! from the header (no decoding) so the block's height is right before any
//! pixel is drawn.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::gpui::Pixels;

/// A block is never taller than this many logical pixels, however tall the
/// image: a long screenshot must not make the note unscrollable.
const MAX_HEIGHT_PX: f32 = 1000.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NaturalSize {
    pub width: u32,
    pub height: u32,
}

/// Where an embed's file is and how big it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImageInfo {
    pub size: NaturalSize,
    /// Absolute and inside the vault, so it also keys the decoded-image cache.
    pub path: Arc<Path>,
}

/// What is known about each embed target as written in the note. `None` means
/// the file is missing or its header unreadable (the block keeps its placeholder).
#[derive(Default)]
pub(super) struct ImageSizes {
    known: HashMap<Box<str>, Option<ImageInfo>>,
}

impl ImageSizes {
    pub(super) fn get(&self, target: &str) -> Option<NaturalSize> {
        self.info(target).map(|info| info.size)
    }

    pub(super) fn info(&self, target: &str) -> Option<&ImageInfo> {
        self.known.get(target)?.as_ref()
    }

    pub(super) fn is_known(&self, target: &str) -> bool {
        self.known.contains_key(target)
    }

    pub(super) fn insert(&mut self, target: &str, info: Option<ImageInfo>) {
        self.known.insert(target.into(), info);
    }
}

/// The width an embed asks for: `![[a.png|200]]` (the whole alias, when it is
/// all there is) or `![alt|200](a.png)` / `![[a.png|alt|200]]`.
pub(super) fn requested_width(alt: &str, wiki: bool) -> Option<u32> {
    let tail = match alt.rsplit_once('|') {
        Some((_, tail)) => tail,
        None if wiki => alt,
        None => return None,
    };
    let width = tail.split('x').next()?;
    width.trim().parse().ok().filter(|w| *w > 0)
}

/// The size an image is drawn at: its requested or natural width, shrunk to
/// the editor's width and to [`MAX_HEIGHT_PX`], keeping its proportions.
pub(super) fn display_size(
    natural: NaturalSize,
    requested: Option<u32>,
    available_width: Pixels,
) -> (f32, f32) {
    let (w, h) = (natural.width as f32, natural.height as f32);
    let wanted = requested.map_or(w, |r| r as f32);
    let scale = (wanted.min(f32::from(available_width)) / w).min(MAX_HEIGHT_PX / h);
    (w * scale, h * scale)
}

/// Whole rows covering the displayed image plus its 4px frame margin.
pub(super) fn rows_for(
    natural: NaturalSize,
    requested: Option<u32>,
    available_width: Pixels,
    line_height: Pixels,
) -> usize {
    let (_, height) = display_size(natural, requested, available_width);
    let rows = ((height + 4.) / f32::from(line_height)).ceil() as usize;
    rows.max(1)
}

/// Reads the header of the file `target` names in note `note_id`. Blocks on
/// the filesystem: call it off the main thread.
pub(super) fn read_info(root: &Path, note_id: &str, target: &str) -> Option<ImageInfo> {
    let path = jade_core::image_file(root, note_id, target)?;
    let size = imagesize::size(&path).ok()?;
    let (width, height) = (
        u32::try_from(size.width).ok()?,
        u32::try_from(size.height).ok()?,
    );
    (width > 0 && height > 0).then(|| ImageInfo {
        size: NaturalSize { width, height },
        path: path.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui::px;

    const NATURAL: NaturalSize = NaturalSize {
        width: 400,
        height: 200,
    };

    #[test]
    fn requested_width_reads_the_alias() {
        assert_eq!(requested_width("200", true), Some(200));
        assert_eq!(requested_width("200", false), None);
        assert_eq!(requested_width("a|200", false), Some(200));
        assert_eq!(requested_width("|200x100", true), Some(200));
        assert_eq!(requested_width("a|0", false), None);
        assert_eq!(requested_width("a|big", true), None);
    }

    #[test]
    fn display_size_shrinks_to_the_width_and_keeps_proportions() {
        assert_eq!(display_size(NATURAL, None, px(800.)), (400., 200.));
        assert_eq!(display_size(NATURAL, None, px(100.)), (100., 50.));
        assert_eq!(display_size(NATURAL, Some(200), px(800.)), (200., 100.));
        let tall = NaturalSize {
            width: 100,
            height: 4000,
        };
        assert_eq!(display_size(tall, None, px(800.)), (25., 1000.));
    }

    #[test]
    fn rows_cover_the_image_and_its_margin() {
        assert_eq!(rows_for(NATURAL, None, px(800.), px(20.)), 11);
        assert_eq!(rows_for(NATURAL, Some(10), px(800.), px(20.)), 1);
    }
}
