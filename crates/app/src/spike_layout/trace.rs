//! What each phase of one frame saw. Diagnostics only, never read by layout.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;

#[derive(Default)]
pub(crate) struct FrameTrace {
    pub(crate) frame: u64,
    /// Every call of the measure callback: known width, available width, µs.
    pub(crate) measures: Vec<(Option<Pixels>, AvailableSpace, u128)>,
    /// The width each measure call computed its height from.
    pub(crate) measured_widths: Vec<Pixels>,
    pub(crate) measured_height: Option<Pixels>,
    pub(crate) previous_width: Pixels,
    pub(crate) prepaint_width: Pixels,
    pub(crate) paint_width: Pixels,
    pub(crate) shaped_width: Pixels,
    /// Total height after prepaint replaced estimates with exact heights.
    pub(crate) height_after_prepaint: Pixels,
    pub(crate) shaped_lines: usize,
    pub(crate) prepaint_time: Duration,
    pub(crate) paint_time: Duration,
}

pub(crate) type SharedTrace = Rc<RefCell<FrameTrace>>;

impl FrameTrace {
    pub(crate) fn line(&self) -> String {
        let measures: Vec<String> = self
            .measures
            .iter()
            .map(|(known, available, us)| {
                let known = known.map_or("-".into(), |k| format!("{:.1}", f32::from(k)));
                let available = match available {
                    AvailableSpace::Definite(w) => format!("{:.1}", f32::from(*w)),
                    AvailableSpace::MinContent => "min".into(),
                    AvailableSpace::MaxContent => "max".into(),
                };
                format!("{known}/{available}:{us}us")
            })
            .collect();
        format!(
            "frame={} prev_w={:.1} measure=[{}] used=[{}] prepaint_w={:.1} shaped_w={:.1} paint_w={:.1} \
             h_measure={:.0} h_after_prepaint={:.0} shaped_lines={} prepaint_us={} paint_us={}",
            self.frame,
            f32::from(self.previous_width),
            measures.join(" "),
            self.measured_widths
                .iter()
                .map(|w| format!("{:.1}", f32::from(*w)))
                .collect::<Vec<_>>()
                .join(" "),
            f32::from(self.prepaint_width),
            f32::from(self.shaped_width),
            f32::from(self.paint_width),
            self.measured_height.map_or(-1., f32::from),
            f32::from(self.height_after_prepaint),
            self.shaped_lines,
            self.prepaint_time.as_micros(),
            self.paint_time.as_micros(),
        )
    }

    /// Appends to `JADE_SPIKE_LOG`, if set.
    pub(crate) fn log(&self) {
        let Some(path) = std::env::var_os("JADE_SPIKE_LOG") else {
            return;
        };
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| writeln!(file, "{}", self.line()));
        if let Err(err) = written {
            eprintln!("[spike] cannot write the log: {err}");
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static FRAMES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn keep_last(trace: &FrameTrace) {
    FRAMES.with(|frames| frames.borrow_mut().push(trace.line()));
}
