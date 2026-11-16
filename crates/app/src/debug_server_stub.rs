//! No-op stand-in for `debug_server/` in builds without the `debug-socket`
//! feature, so the shipped binary carries no input-injecting socket while the
//! call sites in `app.rs` and `text_editor/` stay free of `#[cfg]`. Same
//! names, no behavior.

use gpui_kit::{App, Entity};

use crate::JadeApp;

pub(crate) fn start_if_enabled() {}

pub(crate) fn install(_app: &Entity<JadeApp>, _cx: &mut App) {}

pub(crate) mod targets {
    use gpui_kit::SharedString;

    pub(crate) fn begin_frame() {}

    /// `.debug_name(..)` that names nothing: returns the element unchanged.
    pub(crate) trait DebugName: Sized {
        fn debug_name(self, _name: &'static str) -> Self {
            self
        }

        fn debug_name_with(self, _name: impl FnOnce() -> SharedString) -> Self {
            self
        }
    }

    impl<E> DebugName for E {}
}

pub(crate) mod frames {
    /// Zero-sized stand-in for a clock read.
    pub(crate) struct Stamp;

    pub(crate) fn now() -> Stamp {
        Stamp
    }

    pub(crate) fn micros_since(_start: Stamp) -> u32 {
        0
    }

    pub(crate) fn start() -> Stamp {
        Stamp
    }

    pub(crate) fn finish(_clock: Stamp, _render_us: u32) {}
}
