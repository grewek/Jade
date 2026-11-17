//! Hot reload: watches the config directory and reloads the theme after the
//! files have been quiet for `QUIET`. Editors save in bursts (write, rename,
//! chmod), so one save must cause one reload.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use gpui_kit::App;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::engine::ThemeEngine;
use super::paths::ConfigDirs;

const QUIET: Duration = Duration::from_millis(200);

/// Counts quiet ticks after the last file event. Pure so the timing rule is
/// testable without a file system.
#[derive(Default)]
pub(super) struct Debounce {
    pending: bool,
}

impl Debounce {
    /// Feed one tick: `events` is how many file events arrived since the
    /// previous tick. True when a reload is due now.
    pub(super) fn tick(&mut self, events: usize) -> bool {
        if events > 0 {
            self.pending = true;
            return false;
        }
        std::mem::take(&mut self.pending)
    }
}

/// Starts watching and reloading. Returns the messages to show the user
/// (watching can fail, e.g. no inotify watches left); the app still runs.
pub fn start(dirs: ConfigDirs, cx: &mut App) -> Vec<String> {
    let (tx, rx) = channel();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Reading a theme raises access events; counting them would make
        // every reload trigger the next one.
        if event.is_ok_and(|e| e.kind.is_access()) {
            return;
        }
        // The receiver only goes away at shutdown.
        let _ = tx.send(());
    });
    let mut watcher: RecommendedWatcher = match watcher {
        Ok(watcher) => watcher,
        Err(e) => return vec![format!("theme hot reload is off: {e}")],
    };
    // Created so a user can drop a theme in without restarting.
    if let Err(e) = std::fs::create_dir_all(dirs.theme_dir()) {
        return vec![format!("theme hot reload is off: {e}")];
    }
    if let Err(e) = watcher.watch(dirs.base(), RecursiveMode::Recursive) {
        return vec![format!("theme hot reload is off: {e}")];
    }
    cx.spawn(async move |cx| {
        // The watcher stops when dropped, so this task owns it.
        let _watcher = watcher;
        let mut debounce = Debounce::default();
        loop {
            cx.background_executor().timer(QUIET).await;
            if debounce.tick(drain(&rx)) {
                cx.update(|cx| {
                    for notice in ThemeEngine::reload(Some(&dirs), cx) {
                        eprintln!("jade: {notice}");
                    }
                });
            }
        }
    })
    .detach();
    Vec::new()
}

/// Counts the events waiting, without blocking.
fn drain(rx: &Receiver<()>) -> usize {
    rx.try_iter().count()
}
