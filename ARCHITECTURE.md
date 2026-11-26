# ARCHITECTURE.md

Rules and invariants: what must stay true, and the non-obvious traps. Not auto-loaded. The code is the source of truth for module structure, `SPEC.md` for what is displayed, git for history.

- **No fix history, no measurement logs here** (`CLAUDE.md`). A rule may name the test that guards it.
- The previous, long version of this file (including every detail of the old rendering path) is `git show 838dc96:ARCHITECTURE.md`. Deferred work is in `BACKLOG.md`.

## Crates

| Crate | What it is |
|---|---|
| `jade-core` | GPUI-free: vault, notes, link grammar (`link_scan`), math delimiters (`math_scan`), fence rules (`fence`), link resolution, rename, backlink context |
| `jade-app` | the GPUI app: `app/` (window, tabs, sidebar, dialogs, search), `text_editor/`, `theme/`, `settings/`, `debug_server/` |
| `jade-debug-protocol`, `jade-debug-cli` | the debug harness wire types and `jade-debug` |
| `jade-layout` (planned, #141) | GPUI-free block tree, height map and geometry of the new rendering core |

## Notes and the vault

- **Note text exists once.** `jade_core::Note` holds only id, path and title. The vault keeps the wikilink index and reads text from disk on demand. While a note is open, its only copy is the editor buffer. No `.clone()`/`.to_string()` of note text anywhere (`STYLE.md` rule 14). The vault-open scan keeps each note's distinct link targets only, never a whole file (guard: `opening_a_vault_does_not_buffer_a_whole_note`).
- **One grammar per concept.** Links: `jade_core::link_scan`. Math delimiters: `jade_core::math_scan`. Code fences: `jade_core::fence`. Every pass over note text (rendering, backlink index, rename, backlink context) uses these, so a `[[x]]` in fenced code is never a backlink. Differential tests in `preview` guard the agreement between rendering and `link_scan`.
- **One place decides what a link means:** `Vault::link_destination(from, kind, written) -> Destination` (`Note`, `File`, `External`, `SameNote`, `Missing`). Clicks, broken-link marking, backlink context and rename all ask it. A wikilink target is an exact id, else a case-insensitive id or title match (trailing `.md` ignored); several matches pick the shortest id, then the alphabetically first. A path link tries the note's folder, then the vault root. **Nothing outside the vault is ever returned.**
- **Rename rewrites links** (`Vault::rename_note_updating_links`, `rename_folder_updating_links`): every rewrite is planned against the vault as it is before the file moves, by the same resolution rules as clicks. A wikilink keeps `#heading`, `|alias` and a written `.md`; a path link keeps folders, extension, `#fragment` and `?query`. Open tabs are saved first and rewritten tabs reloaded.
- **Backlinks** come from the cached wikilink targets; backlink context reads the source files on demand and is cached per `content_generation`, so a frame does no file I/O.
- **Search runs on a background thread** (`refresh_search` snapshots, spawns, drops results of superseded generations). **Never call `Vault::search` from `render`.** Search sees disk, so unsaved edits appear after the autosave.

## Tabs, saving, undo

- **At most one tab per note**; each tab owns its `Editor` (cursor, scroll, undo survive switching).
- **Autosave** is debounced (`AUTOSAVE_DEBOUNCE`, 500 ms) and writes only `dirty` tabs. `open_note`, vault renames and quit save **all** tabs, because `dirty` can lag a keystroke. `flush_editor` persists before `open_note` and renames.
- **Undo entries are deltas** (replaced range plus removed text); bursts of same-kind edits coalesce; the stack is capped at `UNDO_STACK_CAP`. Undo and redo report the `TextEdit` they applied, so derived state is patched like for any edit.
- **`EditorBuffer` grows in chunks** (`TEXT_GROWTH_CHUNK`), not by doubling.

## Settings and themes

- **A bad setting never panics.** Each `config.toml` key is judged alone: missing, mistyped or out of range gives that key its default and one notice. `CONFIG.md` documents every key and `config.sample.toml` lists the built-in values; a test keeps the sample equal to the defaults, so a new key goes into both files.
- **A layout setting that changes** bumps `settings::layout_generation`, and every layout cache compares it with its own. A paint-only change just repaints. A new key counts as layout until listed as paint-only.
- **Themes colour; they never change layout.** A bad theme logs and falls back to the built-in Jade Light. All UI colours come from the palette (`ThemeEngine::color`/`faded`, `text_editor/colors.rs`): a test bans colour literals in the UI files it lists, **so add every new UI file to that list** (`theme/tests.rs`). A new role gets an entry in `MAPPING`, `assets/themes/FORMAT.md` and the contrast check; element roles fall back to the role they follow.
- **Config dir:** `JADE_CONFIG_DIR`, else `target/jade-dev` in debug builds, else the OS config dir plus `jade/`. Only `run()` reads the environment; tests pass directories.
- **Fonts** (IBM Plex Sans and Mono under OFL 1.1, Latin Modern Math) are embedded via `include_bytes!` in `install_fonts`.

## Rendering: the new core (#130)

What is displayed is `SPEC.md`. How it is built:

- **Pipeline (one chain):** text change → block tree (reparse the affected window until it matches the old tree) → inline content of the affected blocks → their heights → height map → anchor and view state carried along. Nothing else is invalidated, except the cross-block dependencies listed in `SPEC.md` §8.
- **A block type owns everything about itself:** `estimate(width)`, `measure(width)`, `paint`, and the four geometry questions (position → caret rectangle, point → position, movement, selection rectangles). **No other code computes positions inside a block**, and shared code (document, height map, frame, scrolling) **never branches on block type**.
- **Cache rule:** every derived value has one owner, its block, and is invalidated only by the block's text, the width or a layout setting. A new cache outside the chain needs the user's approval.
- **Frame phases:** the editor element fills its pane, so `prepaint` gets this frame's width from its bounds; the layout step stays empty. **`prepaint` is the only place layout state changes**: it lays out the visible blocks plus about two screen heights of look-ahead exactly, applies the scroll anchor and builds an immutable frame snapshot. **`paint` only reads the snapshot**; GPUI's per-frame registrations that must happen there (`handle_input`, `set_key_context`, `on_action`, `on_key_event`, `on_mouse_event`, `set_cursor_style`) are not editor state. Input handlers read the snapshot of the frame on screen.
- **Background work** (exact measurement after open or a width change) reports back through `defer`/`on_next_frame`: a `notify` made during `prepaint` schedules no frame.
- **Scrolling:** the position is an **anchor**, (block, offset), never a pixel total, so heights changing above the reader cannot move the view. The block type decides what its offset is attached to and translates it after a remeasure. Totals are read only for the scrollbar, which is always visible and has its own gutter. Positions in the height map are `f64`: at document heights in the millions of pixels, `f32` cannot hold fractions of a pixel.
- **Height map:** a balanced sum tree over blocks (log-time insert and remove).
- **Frame budget:** every frame ≤ 16.66 ms. Exceptions: the first frame after opening a note and the first after a width change, at most 33 ms. A visible block is never drawn from an estimate.

## Rendering: the old line-based path (deleted in stage 10, #153)

Until the switch, the old path receives fixes for crashes and data loss only (`CLAUDE.md`). For such a fix these rules still hold; details are in `git show 838dc96:ARCHITECTURE.md`.

- **`LineState::of`** is the one place a line's display state is derived; painting, hit-testing, row counts and IME bounds all build it there.
- **Edits patch, never rebuild:** every edit reports a `TextEdit` to `Editor::record_edit`, which patches `DocumentCache`, `RowCache`, `LinkTable` and `DocStats` from one `EditPatch`. Randomized patch-versus-fresh tests guard each.
- **Row heights** change only through `RowCache::update`, `invalidate`, `verify` and `correct_late`.
- **Mouse closures** share one `Rc<PaintSnapshot>`, so hit-testing matches what was last painted.
- **Cold per-line passes** above 1 MB go through `lines/chunks.rs` (`map_line_chunks`), threaded over whole-line runs.

## GPUI and UI gotchas

- Reused `ElementId`s across panels break click dispatch: namespace them per site (`"folder-{id}"`, `"search-result-{id}"`, `"backlink-{id}"`).
- Raw `window.on_mouse_event` listeners are not hitbox-aware: Down and Move must bail on `window.has_active_dialog(cx)`.
- Dialog Enter dispatches to `.on_ok(...)`; give it the same logic as the footer button. `AlertDialog` wires it already.
- Command palette: a global `KeyBinding` plus `cx.on_action` (an element's `.on_action` only sees the focused path); `CommandState` must be focused explicitly.
- Tab uses the `InsertTab` action in the `"TextEditor"` key context; test windows must register the binding.
- Icons need `.with_assets(gpui_kit::assets::Assets)`. Indent tree rows with `.ml(...)`, not padding.
- Updating another entity from a dialog callback: `entity.update(cx, |s, cx| s.method(window, cx))`, not `update_in`.
- Wayland window icon: GPUI's `WindowOptions.icon` is X11-only, so on Wayland the icon comes from the `app_id` (`APP_ID` = `jade`) matching `packaging/jade.desktop`.

## Headless UI tests

`cargo test -p jade-app -- --test-threads=1`.

- Clickable or findable elements need `.test_support()` plus `.id(...)`.
- Overlays open asynchronously: `cx.run_until_parked()` between separate `cx.update_window(...)` calls.
- Import gpui-kit items explicitly (a glob shadows `#[test]`).
- A `None`-context `KeyBinding` needs a focused element; clicking a div without `.track_focus()` does not focus.
- `window.dispatch_keystroke` adds simulated text input for Enter, Tab and Space, so an Enter through it inserts twice; the real Linux platform sends a bare key-down. Send Enter via `window.dispatch_event(PlatformInput::KeyDown(..))` when measuring it.
- `test_alloc.rs`: `reset()` rebases `peak` but must never zero `current`.
- Shared helpers: `text_editor/test_support.rs` (seeded `Rng`, `apply_edit`, `random_edit`); `bench_api::Toggle` is the insert-then-undo edit every bench loop alternates. A fuzz test that finds a panic in core code is a real bug.

## Verifying performance and memory

- **A fix is done only when a test drives the real input path** (`window.dispatch_keystroke`, `scroll_handle().set_offset` plus `render_frame`), not just the function underneath.
- Paint and mouse closures read text and derived state through the editor entity, never through per-frame copies.
- Guards to keep green: `opening_a_huge_note_keeps_live_memory_bounded`, `search_results_are_cached_until_something_changes`, `line_count_changing_edits_do_not_rebuild_the_document`, `first_keystroke_in_a_huge_file_does_not_reallocate_the_document`, `real_keystrokes_in_a_huge_file_do_not_churn_the_whole_document`, `real_keystrokes_in_a_huge_file_keep_undo_memory_bounded`, `scrolling_through_a_huge_file_does_not_leak`.
- Real app: `JADE_MEMORY_LOG=1 cargo run -p jade-app --release --features debug-socket` logs RSS every 2 s.

## Debug harness

Lets a tool drive and observe the live app. Code: `crates/debug-protocol`, `crates/debug-cli` (`jade-debug`), `crates/app/src/debug_server/`. Skill: `.claude/skills/jade-debug`. Build: `cargo build --release -p jade-app --features debug-socket && cargo build --release -p jade-debug-cli`, then `target/release/jade-debug help`.

- **Off by default:** without the `debug-socket` feature, same-named no-op stubs replace the server and the paint trace (`.debug_name(..)` returns the element unchanged), so the shipped binary has no input socket. Tests always use the real module. Name the build when reporting numbers.
- **`launch <vault>`** runs the app on a **copy** of the vault (autosave would edit real notes) and waits until it is ready; `quit` flushes and closes. The window must be visible: an occluded window gets no frames.
- **Input is real:** it goes through `Window::dispatch_event`/`dispatch_keystroke` (hit-testing, focus, bindings), and its reply comes only after the window painted the effect, so a script never acts on a stale screen. `type` errors when nothing consumed the text.
- **Targets:** `.debug_name("x")` (last builder call) records an element's bounds and visibility each frame; a click aims at the centre of the visible part; unknown, duplicate and clipped names are errors. **Name every new interactive widget.** Not nameable: gpui-kit alert-dialog buttons (use `enter`/`escape`) and menu items (use the keyboard).
- **Pixels need `--headless`:** `facts`, `expect` and `capture` read how an element looks, which only GPUI's headless renderer can provide (scale factor 2; needs a GPU). Coordinates are logical pixels relative to the element's visible bounds.
- **Frame times** (`frames.rs`) are Jade's CPU work per frame in a fixed ring, not GPU submit.
- **Tests** use `debug_server::test_client::TestClient` with `open_app_with_debug_client`, `call_on_main` and `step`; a test that talks to server threads needs `cx.executor().allow_parking()`.

## Fixtures

- `crates/app/benches/fixtures/large_markdown_stress_test.md` is the performance fixture.
- `vault/Markdown Showcase.md` (local, gitignored) is updated in the same change that adds or changes a displayed construct.
