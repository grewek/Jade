//! Headless run mode (`JADE_DEBUG_HEADLESS=1`, or `jade-debug launch
//! --headless`): the app runs on GPUI's headless context with the real text
//! system and the wgpu headless renderer instead of a window on screen. No
//! window appears and an occluded desktop does not matter, and because that
//! window can render itself to an image the `facts`, `expect` and `capture`
//! commands work. The same socket and the same command handlers serve it.
//! `JADE_DEBUG_SIZE=WxH` sets the window size in logical pixels (1200x800).
//!
//! GPUI's headless context has no frame loop, so [`run`] is one: it lets the
//! tasks run, and after every command it draws frames (the effect of an input
//! is only answered once painted, which takes two frames).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, HeadlessAppContext, Size, WindowBounds, WindowOptions, point, px, size,
};

use super::pump;
use crate::JadeApp;
use crate::assets::JadeAssets;

const HEADLESS_VAR: &str = "JADE_DEBUG_HEADLESS";
const SIZE_VAR: &str = "JADE_DEBUG_SIZE";
const DEFAULT_SIZE: (f32, f32) = (1200.0, 800.0);
/// How long the loop sleeps when nothing happened.
const IDLE: Duration = Duration::from_millis(2);
/// Frames drawn after a command: enough for an input's two-frame reply.
const FRAMES_PER_COMMAND: usize = 3;

/// Whether the headless mode was asked for.
pub(crate) fn requested() -> bool {
    std::env::var_os(HEADLESS_VAR).is_some()
}

/// `WxH` as a window size, or the default.
fn window_size() -> Size<gpui_kit::Pixels> {
    let parsed = std::env::var(SIZE_VAR).ok().and_then(|text| {
        let (w, h) = text.split_once('x')?;
        Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
    });
    let (w, h) = parsed
        .filter(|(w, h)| *w >= 100.0 && *h >= 100.0)
        .unwrap_or(DEFAULT_SIZE);
    size(px(w), px(h))
}

/// Runs the app headlessly until a client sends `quit`.
pub(crate) fn run(cli_vault: Option<PathBuf>) {
    super::start_if_enabled();
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(JadeAssets), || {
        gpui_kit::platform::current_headless_renderer()
    });
    // Socket threads wake the main-thread task; the scheduler would otherwise
    // reject activity from threads it did not start.
    cx.allow_parking();
    let config_dirs = cx.update(crate::app::setup);
    let Some(vault) = crate::vault_path::resolve(
        cli_vault,
        crate::theme::configured_vault(config_dirs.as_ref()),
    ) else {
        return eprintln!("[jade-debug] the headless mode needs a vault: pass its path");
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: window_size(),
        })),
        ..Default::default()
    };
    let opened = cx.update(|cx| {
        gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| JadeApp::new(vault.clone(), window, cx))
        })
    });
    let Ok((window, app)) = opened else {
        return eprintln!("[jade-debug] cannot open the headless window");
    };
    cx.update(|cx| super::install(&app, cx));
    let frame = |cx: &mut HeadlessAppContext| {
        let _ = cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        });
    };
    frame(&mut cx);
    eprintln!("[jade-debug] headless window ready");
    let mut seen = pump::COMPLETED.load(Ordering::Acquire);
    while !pump::QUIT.load(Ordering::Acquire) {
        cx.run_until_parked();
        let done = pump::COMPLETED.load(Ordering::Acquire);
        if done == seen {
            std::thread::sleep(IDLE);
            continue;
        }
        seen = done;
        for _ in 0..FRAMES_PER_COMMAND {
            frame(&mut cx);
            cx.run_until_parked();
        }
    }
}
