//! Jade's app as a library, so `main.rs` stays a one-line launcher and the
//! editor internals can be benchmarked and tested from outside the binary.

mod app;
mod assets;
// Without the `debug-socket` feature (and outside tests) a no-op stub with the
// same names replaces the socket server, so the shipped binary has none.
#[cfg(any(test, feature = "debug-socket"))]
mod debug_server;
#[cfg(not(any(test, feature = "debug-socket")))]
#[path = "debug_server_stub.rs"]
mod debug_server;
#[cfg(feature = "bench-support")]
#[doc(hidden)]
pub mod headless;
#[cfg(any(test, feature = "debug-socket"))]
mod memory_probe;
#[cfg(any(test, feature = "bench-support"))]
mod test_alloc;
mod text_editor;
pub mod theme;
mod vault_path;

/// Test-build-only — see `test_alloc`'s own module doc for why. No effect
/// on release/`cargo run` (this whole item is `#[cfg(test)]`-gated).
#[cfg(test)]
#[global_allocator]
static GLOBAL_ALLOC: test_alloc::CountingAllocator = test_alloc::CountingAllocator;

pub(crate) use app::JadeApp;
#[cfg(any(test, feature = "debug-socket"))]
pub(crate) use app::SidebarTab;
pub use app::run;

/// Internals the criterion benches need; not a stable API.
#[doc(hidden)]
pub use text_editor::bench_api;
