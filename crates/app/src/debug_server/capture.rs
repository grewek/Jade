//! `facts`, `expect` and `capture`: what an element looks like, from the
//! pixels of the last painted frame. The frame comes from
//! `Window::render_to_image`, which only a headless window can do (the real
//! Linux window has no pixel readback), so on a live window these commands
//! answer with an error that says to launch with `--headless`.

use gpui_kit::App;
use image::RgbaImage;
use jade_debug_protocol::{
    Answer, CHUNK_BYTES, CaptureInfo, Command, MAX_CAPTURE_BYTES, Message, Rect, base64,
};

use super::{pixels, targets};

fn error(message: impl Into<String>) -> (Answer, Vec<Message>) {
    (
        Answer::Error {
            message: message.into(),
        },
        Vec::new(),
    )
}

/// Whether `command` is answered here.
pub(super) fn is_pixel_command(command: &Command) -> bool {
    matches!(
        command,
        Command::Facts { .. } | Command::Expect { .. } | Command::Capture { .. }
    )
}

/// The last painted frame as an image, and its device pixels per logical pixel.
fn frame(cx: &mut App) -> Result<(RgbaImage, f32), String> {
    let handle = cx.windows().into_iter().next().ok_or("no window is open")?;
    let (image, scale) = handle
        .update(cx, |_, window, _| {
            (window.render_to_image(), window.scale_factor())
        })
        .map_err(|err| format!("window unavailable: {err}"))?;
    let image = image.map_err(|err| {
        format!(
            "cannot read the frame back ({err}); pixel commands need the headless mode: \
             `jade-debug launch --headless <vault>`"
        )
    })?;
    Ok((image, scale))
}

/// The pixels of `target` (or `within` it) as painted, with where they lie.
fn region(
    target: &str,
    within: Option<Rect>,
    cx: &mut App,
) -> Result<(RgbaImage, f32, Rect), String> {
    let bounds = targets::visible_bounds(target)?;
    let (window, scale) = frame(cx)?;
    let (region, at) = pixels::crop(&window, scale, bounds, within)?;
    Ok((region, scale, at))
}

/// The PNG of `region` as the reply and its chunks.
fn capture(
    region: &RgbaImage,
    scale: f32,
    at: Rect,
    shrink: Option<f32>,
) -> Result<(Answer, Vec<Message>), String> {
    let (bytes, width, height) = pixels::png(region, shrink)?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(format!(
            "the PNG is {} bytes, over the {MAX_CAPTURE_BYTES} limit; ask for a rect or a scale",
            bytes.len()
        ));
    }
    Ok(chunked(&bytes, width, height, scale, at))
}

/// The reply for `png` and the chunks that carry it (`req` is filled in when
/// they are sent).
fn chunked(png: &[u8], width: u32, height: u32, scale: f32, at: Rect) -> (Answer, Vec<Message>) {
    let total = png.chunks(CHUNK_BYTES).count() as u32;
    let chunks = png
        .chunks(CHUNK_BYTES)
        .enumerate()
        .map(|(index, part)| Message::Chunk {
            req: 0,
            index: index as u32,
            total,
            data: base64::encode(part),
        })
        .collect();
    let info = CaptureInfo {
        width,
        height,
        scale,
        bounds: at,
        bytes: png.len(),
        chunks: total,
    };
    (Answer::Capture(info), chunks)
}

/// Runs one pixel command: the reply, and for a capture the chunks to send
/// right after it.
pub(super) fn execute(command: &Command, cx: &mut App) -> (Answer, Vec<Message>) {
    let outcome = match command {
        Command::Facts {
            target,
            facts,
            rect,
            tolerance,
        } => region(target, *rect, cx)
            .and_then(|(img, scale, _)| pixels::measure(&img, scale, *tolerance, facts))
            .map(|reply| (Answer::Facts(reply), Vec::new())),
        Command::Expect {
            target,
            checks,
            tolerance,
        } => region(target, None, cx)
            .and_then(|(img, scale, _)| pixels::evaluate(&img, scale, *tolerance, checks))
            .map(|(passed, results)| (Answer::Expect { passed, results }, Vec::new())),
        Command::Capture {
            target,
            rect,
            scale: shrink,
        } => {
            region(target, *rect, cx).and_then(|(img, scale, at)| capture(&img, scale, at, *shrink))
        }
        _ => Err("not a pixel command".into()),
    };
    match outcome {
        Ok(done) => done,
        Err(message) => error(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn a_png_travels_as_ordered_chunks_that_join_back() {
        let mut img = RgbaImage::from_pixel(300, 200, Rgba([255, 255, 255, 255]));
        // Noise makes the PNG big enough for several chunks.
        let mut seed = 7u32;
        for p in img.pixels_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *p = Rgba([
                (seed >> 8) as u8,
                (seed >> 16) as u8,
                (seed >> 24) as u8,
                255,
            ]);
        }
        let (bytes, w, h) = pixels::png(&img, None).unwrap();
        let at = Rect {
            x: 0.0,
            y: 0.0,
            w: 150.0,
            h: 100.0,
        };
        let (answer, chunks) = chunked(&bytes, w, h, 2.0, at);
        let Answer::Capture(info) = answer else {
            panic!("not a capture reply");
        };
        assert!(info.chunks > 1, "{} chunks", info.chunks);
        assert_eq!(chunks.len() as u32, info.chunks);
        assert_eq!(info.bytes, bytes.len());
        let mut joined = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let Message::Chunk {
                index, total, data, ..
            } = chunk
            else {
                panic!("not a chunk");
            };
            assert_eq!((*index as usize, *total), (i, info.chunks));
            assert!(data.len() < jade_debug_protocol::MAX_LINE_BYTES - 1024);
            joined.extend(base64::decode(data).unwrap());
        }
        assert_eq!(joined, bytes);
        assert_eq!(image::load_from_memory(&joined).unwrap().to_rgba8(), img);
    }
}
