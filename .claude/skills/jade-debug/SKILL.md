---
name: jade-debug
description: Drive and observe the live Jade app (click, type, scroll, read editor text, frame times, paint trace) through its debug socket with the `jade-debug` CLI. Use to reproduce or verify UI behavior, rendering bugs, performance and memory in the real app instead of guessing from unit tests.
---

# Debugging the live Jade app

Full reference: `ARCHITECTURE.md` "Debug harness". The loop:

```sh
cargo build --release -p jade-app -p jade-debug-cli
J=target/release/jade-debug
$J launch /abs/path/to/vault        # runs the app on a COPY of the vault; prints pid, socket, copy path
export JADE_DEBUG_SOCKET=/tmp/jade-debug.sock   # the default socket launch uses
$J targets                          # what is on screen, by name, with bounds
$J script <<'EOF2'
click note-Big
click editor
type hello\nworld
text 0 40
state
EOF2
$J quit
```

`jade-debug help` lists every command. Targets are names like `note-Big`, `new-note`, `editor`; mouse commands also take `x,y`.

## Rules that save time

- **Never debug on the real vault.** Typing autosaves. `launch` copies by default; keep it that way.
- **The window must be visible.** An occluded window gets no frames: input is unconfirmed (error after 3s), `paint`/`frames` stay silent. Ask the user to leave it in front; do not move their windows or take screenshots of their desktop.
- **Click `editor` to focus text**, not `editor-pane` (the scroll container, tall for short notes). `type` errors if nothing consumed the text.
- **Replies mean painted.** Input commands answer after the window painted their effect, so consecutive commands are race-free; `list_targets` always shows the last painted frame.
- **Verify, don't assume**: `text` reads the buffer back, `state` gives cursor/selection/scroll/focus, `app` gives note/tab/search/dialog open.
- **Dialogs**: inputs and footer buttons are named (`rename-input`, `confirm-rename`, ...); alert dialogs (delete) take `key enter` / `key escape`; popup menus take the keyboard.
- **Performance**: `frames` summarises editor render+paint against the 16.66ms budget (CPU only, not GPU). Opening a huge note costs a one-time slow first frame; judge steady state on typing/scrolling. `watch paint 10` streams the paint trace; `consistent:false` flags a mismatched callout band. `memory` reports RSS and heap.
- **Add `.debug_name(..)` to every new interactive widget** (last builder call) so it can be driven by name; add a test through `open_app_with_debug_client`/`step` for new flows.
- Clean up: `quit` the app (it removes its socket); the vault copy is in `/tmp/jade-debug-vault-*`.
