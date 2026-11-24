//! What the debug socket can say about how an element looks: compact facts
//! measured on its rendered pixels, checks against expectations written as
//! geometry, and the PNG itself. Everything here works on an `RgbaImage` and
//! the frame's scale factor, with no GPUI and no GPU, so it is tested on
//! synthetic images; `capture.rs` supplies the real frame.
//!
//! Coordinates in and out are logical pixels relative to the region (the
//! element, or the part of it that was asked for). "Ink" is a pixel that
//! differs from the region's background (its most common colour) by more than
//! the tolerance in any channel, so anti-aliasing noise does not count.

use std::collections::HashMap;

use image::{Rgba, RgbaImage};
use jade_debug_protocol::{
    Check, CheckResult, Fact, FactValue, FactsReply, MAX_GRID_CELLS, MAX_PALETTE, MAX_PROFILE_LEN,
    PaletteEntry, Rect,
};

/// The pixels of `window` inside `bounds` (window coordinates, logical),
/// narrowed to `within` (relative to `bounds`) when given. Returns the region
/// and where it lies in window coordinates.
pub(super) fn crop(
    window: &RgbaImage,
    scale: f32,
    bounds: Rect,
    within: Option<Rect>,
) -> Result<(RgbaImage, Rect), String> {
    let wanted = within.map_or(bounds, |r| Rect {
        x: bounds.x + r.x,
        y: bounds.y + r.y,
        w: r.w,
        h: r.h,
    });
    let left = (wanted.x * scale).round().max(0.0) as u32;
    let top = (wanted.y * scale).round().max(0.0) as u32;
    let right = (((wanted.x + wanted.w) * scale).round().max(0.0) as u32).min(window.width());
    let bottom = (((wanted.y + wanted.h) * scale).round().max(0.0) as u32).min(window.height());
    if right <= left || bottom <= top {
        return Err("the requested region is empty or outside the window".into());
    }
    let region = image::imageops::crop_imm(window, left, top, right - left, bottom - top);
    let at = Rect {
        x: left as f32 / scale,
        y: top as f32 / scale,
        w: (right - left) as f32 / scale,
        h: (bottom - top) as f32 / scale,
    };
    Ok((region.to_image(), at))
}

pub(super) fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn parse_hex(text: &str) -> Result<[u8; 3], String> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    let bad = || format!("{text:?} is not a #rrggbb colour");
    if digits.len() != 6 || !digits.is_ascii() {
        return Err(bad());
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| bad());
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

/// The most common colour of `region` (the lowest one wins a tie, so the
/// answer does not depend on hash order).
fn background(region: &RgbaImage) -> [u8; 3] {
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for Rgba([r, g, b, _]) in region.pixels() {
        *counts.entry([*r, *g, *b]).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map_or([0; 3], |(color, _)| color)
}

fn differs(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).any(|(x, y)| x.abs_diff(y) > tolerance)
}

/// Which pixels of a region are ink.
struct Ink {
    width: u32,
    height: u32,
    scale: f32,
    background: [u8; 3],
    bits: Vec<bool>,
}

impl Ink {
    fn of(region: &RgbaImage, scale: f32, tolerance: u8) -> Self {
        let background = background(region);
        let bits = region
            .pixels()
            .map(|Rgba([r, g, b, _])| differs([*r, *g, *b], background, tolerance))
            .collect();
        Self {
            width: region.width(),
            height: region.height(),
            scale,
            background,
            bits,
        }
    }

    fn at(&self, x: u32, y: u32) -> bool {
        self.bits[(y * self.width + x) as usize]
    }

    /// Ink pixels and total pixels in the device rectangle `x0..x1` by `y0..y1`.
    fn count(&self, x0: u32, x1: u32, y0: u32, y1: u32) -> (u32, u32) {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        let mut ink = 0;
        for y in y0..y1 {
            ink += (x0..x1).filter(|&x| self.at(x, y)).count() as u32;
        }
        (ink, x1.saturating_sub(x0) * y1.saturating_sub(y0))
    }

    /// The device rectangle of a logical one (relative to the region).
    fn device(&self, r: Rect) -> (u32, u32, u32, u32) {
        let edge = |v: f32| (v * self.scale).round().max(0.0) as u32;
        (edge(r.x), edge(r.x + r.w), edge(r.y), edge(r.y + r.h))
    }

    fn coverage_percent(&self, r: Rect) -> f32 {
        let (x0, x1, y0, y1) = self.device(r);
        match self.count(x0, x1, y0, y1) {
            (_, 0) => 0.0,
            (ink, total) => ink as f32 * 100.0 / total as f32,
        }
    }

    /// Bounds of all the ink in logical pixels, `None` without any.
    fn bbox(&self) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..self.height {
            for x in (0..self.width).filter(|&x| self.at(x, y)) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        (x1 > x0).then(|| Rect {
            x: x0 as f32 / self.scale,
            y: y0 as f32 / self.scale,
            w: (x1 - x0) as f32 / self.scale,
            h: (y1 - y0) as f32 / self.scale,
        })
    }

    fn logical_len(&self, device: u32) -> u32 {
        (device as f32 / self.scale).ceil() as u32
    }

    /// Ink per logical-pixel row (`rows`) or column, in logical pixels.
    fn profile(&self, rows: bool) -> Result<Vec<u32>, String> {
        let len = self.logical_len(if rows { self.height } else { self.width });
        if len > MAX_PROFILE_LEN {
            return Err(format!(
                "the region is {len} px along the profile, more than {MAX_PROFILE_LEN}; \
                 ask for a smaller rect"
            ));
        }
        let edge = |i: u32| (i as f32 * self.scale).round() as u32;
        Ok((0..len)
            .map(|i| {
                let (a, b) = (edge(i), edge(i + 1));
                let (ink, _) = if rows {
                    self.count(0, self.width, a, b)
                } else {
                    self.count(a, b, 0, self.height)
                };
                // A logical row spans `scale` device rows, each `scale` times as
                // wide as a logical pixel.
                (ink as f32 / (self.scale * self.scale)).round() as u32
            })
            .collect())
    }

    fn grid(&self, cols: u16, rows: u16) -> Result<Vec<u8>, String> {
        let (cols, rows) = (u32::from(cols), u32::from(rows));
        if cols == 0 || rows == 0 || cols * rows > MAX_GRID_CELLS {
            return Err(format!(
                "an ink grid has 1 to {MAX_GRID_CELLS} cells, not {cols}x{rows}"
            ));
        }
        let mut cells = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            for c in 0..cols {
                let (x0, x1) = (c * self.width / cols, (c + 1) * self.width / cols);
                let (y0, y1) = (r * self.height / rows, (r + 1) * self.height / rows);
                cells.push(match self.count(x0, x1, y0, y1) {
                    (_, 0) => 0,
                    (ink, total) => (ink as f32 * 255.0 / total as f32).round() as u8,
                });
            }
        }
        Ok(cells)
    }
}

fn palette(region: &RgbaImage, max: u8) -> Vec<PaletteEntry> {
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for Rgba([r, g, b, _]) in region.pixels() {
        let quantize = |v: u8| (v & 0xF0) | 0x08;
        *counts
            .entry([quantize(*r), quantize(*g), quantize(*b)])
            .or_default() += 1;
    }
    let total = (region.width() * region.height()) as f32;
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted
        .into_iter()
        .take(usize::from(max.min(MAX_PALETTE)))
        .map(|(color, n)| PaletteEntry {
            color: hex(color),
            percent: n as f32 * 100.0 / total,
        })
        .collect()
}

fn pixel(region: &RgbaImage, scale: f32, x: f32, y: f32) -> Result<[u8; 3], String> {
    let (px, py) = ((x * scale) as i64, (y * scale) as i64);
    if x < 0.0 || y < 0.0 || px >= i64::from(region.width()) || py >= i64::from(region.height()) {
        return Err(format!("({x}, {y}) is outside the region"));
    }
    let Rgba([r, g, b, _]) = *region.get_pixel(px as u32, py as u32);
    Ok([r, g, b])
}

/// Measures `facts` on `region`, whose frame had `scale` device pixels per
/// logical pixel.
pub(super) fn measure(
    region: &RgbaImage,
    scale: f32,
    tolerance: u8,
    facts: &[Fact],
) -> Result<FactsReply, String> {
    let ink = Ink::of(region, scale, tolerance);
    let mut values = Vec::with_capacity(facts.len());
    for fact in facts {
        values.push(match fact {
            Fact::InkGrid { cols, rows } => FactValue::InkGrid {
                cols: *cols,
                rows: *rows,
                cells: ink.grid(*cols, *rows)?,
            },
            Fact::RowProfile => FactValue::RowProfile {
                counts: ink.profile(true)?,
            },
            Fact::ColProfile => FactValue::ColProfile {
                counts: ink.profile(false)?,
            },
            Fact::Bbox => FactValue::Bbox { rect: ink.bbox() },
            Fact::Palette { max } => FactValue::Palette {
                colors: palette(region, *max),
            },
            Fact::Pixel { x, y } => FactValue::Pixel {
                color: hex(pixel(region, scale, *x, *y)?),
            },
        });
    }
    Ok(FactsReply {
        width: region.width() as f32 / scale,
        height: region.height() as f32 / scale,
        scale,
        background: hex(ink.background),
        facts: values,
    })
}

/// `x,y WxH`, the way the CLI takes a rect.
fn show(r: &Rect) -> String {
    format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
}

fn verdict(passed: bool, detail: String) -> CheckResult {
    CheckResult { passed, detail }
}

fn check_one(ink: &Ink, region: &RgbaImage, check: &Check) -> Result<CheckResult, String> {
    Ok(match check {
        Check::InkIn { rect, min_percent } => {
            let got = ink.coverage_percent(*rect);
            verdict(
                got >= *min_percent,
                format!(
                    "{got:.2}% ink in {}, wanted at least {min_percent}%",
                    show(rect)
                ),
            )
        }
        Check::Empty { rect, max_percent } => {
            let got = ink.coverage_percent(*rect);
            verdict(
                got <= *max_percent,
                format!(
                    "{got:.2}% ink in {}, wanted at most {max_percent}%",
                    show(rect)
                ),
            )
        }
        Check::Color {
            x,
            y,
            color,
            tolerance,
        } => {
            let want = parse_hex(color)?;
            let got = pixel(region, ink.scale, *x, *y)?;
            verdict(
                !differs(got, want, *tolerance),
                format!(
                    "pixel ({x}, {y}) is {}, wanted {color} ±{tolerance}",
                    hex(got)
                ),
            )
        }
        Check::BboxWithin { rect } => match ink.bbox() {
            None => verdict(true, "there is no ink".into()),
            Some(b) => {
                // Half a device pixel of slack: edges round.
                let slack = 0.5 / ink.scale;
                let inside = b.x >= rect.x - slack
                    && b.y >= rect.y - slack
                    && b.x + b.w <= rect.x + rect.w + slack
                    && b.y + b.h <= rect.y + rect.h + slack;
                verdict(
                    inside,
                    format!("ink spans {}, wanted inside {}", show(&b), show(rect)),
                )
            }
        },
    })
}

/// Evaluates `checks` on `region`; the bool is whether every one passed.
pub(super) fn evaluate(
    region: &RgbaImage,
    scale: f32,
    tolerance: u8,
    checks: &[Check],
) -> Result<(bool, Vec<CheckResult>), String> {
    let ink = Ink::of(region, scale, tolerance);
    let results = checks
        .iter()
        .map(|check| check_one(&ink, region, check))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((results.iter().all(|r| r.passed), results))
}

/// `region` shrunk by `scale` (0 < scale <= 1) and encoded as a PNG.
pub(super) fn png(region: &RgbaImage, scale: Option<f32>) -> Result<(Vec<u8>, u32, u32), String> {
    let scaled;
    let shown = match scale {
        Some(s) if !(s > 0.0 && s <= 1.0) => {
            return Err(format!("scale must be above 0 and at most 1, not {s}"));
        }
        Some(s) if s < 1.0 => {
            let w = ((region.width() as f32 * s).round() as u32).max(1);
            let h = ((region.height() as f32 * s).round() as u32).max(1);
            scaled = image::imageops::resize(region, w, h, image::imageops::FilterType::Triangle);
            &scaled
        }
        _ => region,
    };
    let mut bytes = Vec::new();
    shown
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|err| format!("cannot encode the PNG: {err}"))?;
    Ok((bytes, shown.width(), shown.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    /// A white `w` x `h` image with black rectangles `(x, y, w, h)`.
    fn image(w: u32, h: u32, ink: &[(u32, u32, u32, u32)]) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(w, h, Rgba(WHITE));
        for &(x0, y0, rw, rh) in ink {
            for y in y0..y0 + rh {
                for x in x0..x0 + rw {
                    img.put_pixel(x, y, Rgba(BLACK));
                }
            }
        }
        img
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn one(img: &RgbaImage, scale: f32, fact: Fact) -> FactValue {
        measure(img, scale, 8, &[fact]).unwrap().facts.remove(0)
    }

    #[test]
    fn the_background_is_the_most_common_colour_and_ink_is_the_rest() {
        let img = image(10, 10, &[(2, 2, 3, 3)]);
        let reply = measure(&img, 1.0, 8, &[Fact::Bbox]).unwrap();
        assert_eq!(reply.background, "#ffffff");
        assert_eq!(
            reply.facts,
            vec![FactValue::Bbox {
                rect: Some(rect(2.0, 2.0, 3.0, 3.0))
            }]
        );
    }

    #[test]
    fn noise_within_the_tolerance_is_not_ink() {
        let mut img = image(8, 8, &[]);
        img.put_pixel(3, 3, Rgba([250, 250, 250, 255]));
        assert_eq!(one(&img, 1.0, Fact::Bbox), FactValue::Bbox { rect: None });
        img.put_pixel(4, 4, Rgba([200, 255, 255, 255]));
        assert!(matches!(
            one(&img, 1.0, Fact::Bbox),
            FactValue::Bbox { rect: Some(_) }
        ));
    }

    #[test]
    fn the_ink_grid_is_a_coverage_thumbnail() {
        // Left half black, right half white.
        let img = image(8, 4, &[(0, 0, 4, 4)]);
        let FactValue::InkGrid { cells, .. } = one(&img, 1.0, Fact::InkGrid { cols: 2, rows: 2 })
        else {
            panic!("not a grid");
        };
        // The most common colour ties; the lower one (black) is the
        // background, so the white half is the ink.
        assert_eq!(cells, vec![0, 255, 0, 255]);
    }

    #[test]
    fn profiles_show_text_lines_and_gaps() {
        // Two 2px lines at y = 1 and y = 5 in a 10 px tall region.
        let img = image(10, 10, &[(1, 1, 8, 2), (1, 5, 8, 2)]);
        let FactValue::RowProfile { counts } = one(&img, 1.0, Fact::RowProfile) else {
            panic!("not a profile");
        };
        assert_eq!(counts, vec![0, 8, 8, 0, 0, 8, 8, 0, 0, 0]);
        let FactValue::ColProfile { counts } = one(&img, 1.0, Fact::ColProfile) else {
            panic!("not a profile");
        };
        assert_eq!(counts, vec![0, 4, 4, 4, 4, 4, 4, 4, 4, 0]);
    }

    #[test]
    fn a_scale_of_two_reports_logical_pixels() {
        // 20x20 device pixels = 10x10 logical; a 4x4 device block at (4, 4).
        let img = image(20, 20, &[(4, 4, 4, 4)]);
        assert_eq!(
            one(&img, 2.0, Fact::Bbox),
            FactValue::Bbox {
                rect: Some(rect(2.0, 2.0, 2.0, 2.0))
            }
        );
        let FactValue::RowProfile { counts } = one(&img, 2.0, Fact::RowProfile) else {
            panic!("not a profile");
        };
        assert_eq!(counts.len(), 10);
        assert_eq!(counts[2], 2); // 4 device px wide = 2 logical px
        assert_eq!(counts[0], 0);
        let reply = measure(&img, 2.0, 8, &[]).unwrap();
        assert_eq!((reply.width, reply.height), (10.0, 10.0));
    }

    #[test]
    fn the_palette_lists_the_dominant_colours() {
        let img = image(10, 10, &[(0, 0, 10, 3)]);
        let FactValue::Palette { colors } = one(&img, 1.0, Fact::Palette { max: 2 }) else {
            panic!("not a palette");
        };
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0].color, "#f8f8f8");
        assert!((colors[0].percent - 70.0).abs() < 0.01);
        assert!((colors[1].percent - 30.0).abs() < 0.01);
    }

    #[test]
    fn a_pixel_fact_reads_one_colour_and_rejects_outside_points() {
        let img = image(4, 4, &[(1, 1, 1, 1)]);
        assert_eq!(
            one(&img, 1.0, Fact::Pixel { x: 1.0, y: 1.0 }),
            FactValue::Pixel {
                color: "#000000".into()
            }
        );
        assert!(measure(&img, 1.0, 8, &[Fact::Pixel { x: 9.0, y: 0.0 }]).is_err());
    }

    #[test]
    fn replies_stay_bounded() {
        let img = image(10, 10, &[]);
        assert!(measure(&img, 1.0, 8, &[Fact::InkGrid { cols: 0, rows: 3 }]).is_err());
        assert!(
            measure(
                &img,
                1.0,
                8,
                &[Fact::InkGrid {
                    cols: 100,
                    rows: 100
                }]
            )
            .is_err(),
            "10000 cells is over the cap"
        );
        let tall = image(2, MAX_PROFILE_LEN + 1, &[]);
        assert!(measure(&tall, 1.0, 8, &[Fact::RowProfile]).is_err());
    }

    #[test]
    fn checks_pass_and_fail_with_the_measured_value() {
        let img = image(20, 10, &[(2, 2, 6, 6)]);
        let checks = [
            Check::InkIn {
                rect: rect(2.0, 2.0, 6.0, 6.0),
                min_percent: 50.0,
            },
            Check::Empty {
                rect: rect(10.0, 0.0, 10.0, 10.0),
                max_percent: 0.0,
            },
            Check::Color {
                x: 0.0,
                y: 0.0,
                color: "#ffffff".into(),
                tolerance: 4,
            },
            Check::BboxWithin {
                rect: rect(0.0, 0.0, 10.0, 10.0),
            },
        ];
        let (passed, results) = evaluate(&img, 1.0, 8, &checks).unwrap();
        assert!(passed, "{results:?}");

        let failing = [
            Check::Empty {
                rect: rect(2.0, 2.0, 6.0, 6.0),
                max_percent: 0.0,
            },
            Check::BboxWithin {
                rect: rect(0.0, 0.0, 5.0, 5.0),
            },
            Check::Color {
                x: 3.0,
                y: 3.0,
                color: "#ffffff".into(),
                tolerance: 4,
            },
        ];
        let (passed, results) = evaluate(&img, 1.0, 8, &failing).unwrap();
        assert!(!passed);
        assert!(results.iter().all(|r| !r.passed), "{results:?}");
        assert!(
            results[0].detail.contains("100.00% ink in 2,2 6x6"),
            "{results:?}"
        );
        assert!(
            evaluate(
                &img,
                1.0,
                8,
                &[Check::Color {
                    x: 0.0,
                    y: 0.0,
                    color: "red".into(),
                    tolerance: 4,
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn cropping_follows_the_element_and_the_scale() {
        let window = image(40, 40, &[(10, 10, 4, 4)]);
        // Element at (4, 4) logical, 8x8, on a 2x frame: device 8..24.
        let (region, at) = crop(&window, 2.0, rect(4.0, 4.0, 8.0, 8.0), None).unwrap();
        assert_eq!((region.width(), region.height()), (16, 16));
        assert_eq!(at, rect(4.0, 4.0, 8.0, 8.0));
        let (inner, at) = crop(
            &window,
            2.0,
            rect(4.0, 4.0, 8.0, 8.0),
            Some(rect(1.0, 1.0, 2.0, 2.0)),
        )
        .unwrap();
        assert_eq!((inner.width(), inner.height()), (4, 4));
        assert_eq!(at, rect(5.0, 5.0, 2.0, 2.0));
        assert!(crop(&window, 2.0, rect(100.0, 100.0, 5.0, 5.0), None).is_err());
    }

    #[test]
    fn a_png_decodes_back_to_the_same_pixels_and_scales_down() {
        let img = image(16, 8, &[(2, 2, 4, 4)]);
        let (bytes, w, h) = png(&img, None).unwrap();
        assert_eq!((w, h), (16, 8));
        let back = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(back, img);
        let (_, w, h) = png(&img, Some(0.5)).unwrap();
        assert_eq!((w, h), (8, 4));
        assert!(png(&img, Some(2.0)).is_err());
        assert!(png(&img, Some(0.0)).is_err());
    }
}
