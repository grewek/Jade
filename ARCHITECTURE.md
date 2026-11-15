# ARCHITECTURE.md

Non-obvious invariants, gotchas, and the Backlog. Not auto-loaded. The code is the source of truth for module structure — grep it.

## Invariants

- **Live Preview**: the cursor's line renders raw. A table renders entirely raw if the cursor is anywhere in it (`is_table_active`). Callouts are render-only (always expanded). List markers are *replaced* (`•`, `N. `), not just hidden. Lazy continuation is deliberately more lenient than CommonMark.
- **Row heights**: `RowHeightCache` seeds non-table lines with an allocation-free over-estimate (`estimate_row_count`) and corrects to exact when painted, so total height starts slightly tall and settles while scrolling. Table rows always use the exact path. Validated against `exact_row_counts`.
- **`DocumentCache` patching**: edits report a `TextEdit` to `Editor::record_edit`. A line-count change (Enter, joining lines, multi-line paste) resizes every per-line array and both Fenwick trees at one splice point (`resize_lines`, `Fenwick::resize`, `RowHeightCache::resize_for`), shifts stored line numbers of later tables/callouts, then runs the usual resumable walks. Only undo/redo fully recomputes. A table's delimiter row needn't contain `|`, so `patch_tables` widens its window to fixpoint over adjacent tables and the header above. Validated by a randomized test (25 seeds) comparing every field, entering states included, against a freshly built cache.
- **Undo**: bursts of same-kind edits coalesce into one delta (`UndoEntry`: replaced range + removed text), so memory scales with edited text, not document size; stack capped at `UNDO_STACK_CAP` (100).
- **Paint snapshot**: mouse closures share one `Rc<PaintSnapshot>` so hit-testing matches what was last painted, not the live document.
- **Autosave**: `open_note` subscribes to `EditorEvent::Changed`; `flush_editor` persists and drops the editor before `open_note`/renames.
- **Showcase fixture**: `vault/Markdown Showcase.md` (local, gitignored) must be updated in the same change that adds or changes a Live Preview construct.

## GPUI/UI gotchas

- Reused `ElementId`s across panels break click dispatch — namespace per site (`"folder-{id}"`, `"search-result-{id}"`, `"backlink-{id}"`).
- Raw `window.on_mouse_event` listeners aren't hitbox-aware: Down/Move must bail on `window.has_active_dialog(cx)`.
- Dialog Enter dispatches to `.on_ok(...)`; set it to the same logic as the footer button. `AlertDialog` wires it already.
- Command palette: global `KeyBinding` + `cx.on_action` (element `.on_action` only sees the focused path); `CommandState` must be focused explicitly.
- Tab uses the `InsertTab` action in the `"TextEditor"` key context; test windows must register the binding.
- Icons need `.with_assets(gpui_kit::assets::Assets)`. Indent tree rows with `.ml(...)`, not padding.
- Updating another entity from a dialog callback: `entity.update(cx, |s, cx| s.method(window, cx))`, not `update_in`.
- Fonts (IBM Plex, OFL 1.1) are embedded via `include_bytes!` in `install_fonts`.

## Headless UI tests

`cargo test -p jade-app -- --test-threads=1`.
- Clickable/findable elements need `.test_support()` plus `.id(...)`.
- Overlays open asynchronously: `cx.run_until_parked()` between separate `cx.update_window(...)` calls.
- Import gpui-kit items explicitly (a glob shadows `#[test]`).
- A `None`-context `KeyBinding` needs a focused element; clicking a non-`.track_focus()` div doesn't focus.
- `window.dispatch_keystroke` adds simulated text input for Enter/Tab/Space (`with_simulated_ime`), so an Enter through it inserts twice; the real Linux platform sends a bare key-down. Send Enter via `window.dispatch_event(PlatformInput::KeyDown(..))` when measuring it.
- `test_alloc.rs` (`#[cfg(test)]`): `reset()` rebases `peak` but must never zero `current`.

## Verifying perf/memory fixes

A fix isn't done until a test drives the real input path (`window.dispatch_keystroke`, `scroll_handle().set_offset` + `render_frame`), not just the underlying function. Calling `EditorBuffer::insert` directly hid a bug where `replace_text_in_range`'s `set_cursor` calls defeated undo coalescing (1.19GB retained over 150 keystrokes on the stress fixture, vs ~24MB fixed).
- Autosave is debounced (`AUTOSAVE_DEBOUNCE`, 500ms; flushed on note switch and app quit): a save copies the note, rescans wikilinks and rewrites the file. Paint/mouse closures read text and `DocumentCache` through the editor entity, never per-frame copies.
- `EditorBuffer` keeps `TEXT_GROWTH_CHUNK` spare capacity and grows in chunks, not by doubling (a cloned note has none; the first key used to retain ~11MB).
- Guards: `line_count_changing_edits_do_not_rebuild_the_document` (Enter/Backspace-join: bytes allocated), `first_keystroke_in_a_huge_file_does_not_reallocate_the_document`, `real_keystrokes_in_a_huge_file_do_not_churn_the_whole_document` (bytes allocated per key), `real_keystrokes_in_a_huge_file_keep_undo_memory_bounded`, `scrolling_through_a_huge_file_does_not_leak` (`main.rs`; heap flat across 200 scroll steps, RSS ~160MB flat in the headless harness).
- Real app: `JADE_MEMORY_LOG=1 cargo run -p jade-app --release` logs RSS every 2s (`memory_probe.rs`).

## Backlog

Deliberately deferred — don't pick up without checking in (see `CLAUDE.md`).

- **Undo/redo of a line-count-changing edit** still fully recomputes both caches (`invalidate_all`); `UndoEntry` knows the replaced range, so it could report a `TextEdit` instead.
- **Per-keystroke cost on the 320K-line stress fixture**: render ~13–16ms; edit side adds ~15–20ms, traced to GPUI's `dispatch_keystroke` (vendored), not Jade code. A full edit+render cycle is not yet under 16.66ms.
- **Memory on the stress fixture**: opening it retains ~77MB over ~1.9M allocations (`DocumentCache` ~38.8MB, buffer ~11.3MB, `RowHeightCache` ~2.6MB). Remaining allocations come from table rows on the exact path.
- **`Vault::backlinks()`** rebuilds its whole map per call (worst measured ~17ms on 200 notes × 20k words).
- **Live Preview**: full CommonMark emphasis; formatting visible on the active line; heading sizes (bold only); theme-driven colors (currently hardcoded `Hsla`).
- **Callouts**: fold interactivity (with task-checkbox toggling and WYSIWYG tables, a later "interactions" phase); nested callouts.
- **WYSIWYG table editing**: not designed.
- **Settings dialog**: non-functional placeholder; themes, text size, editor preferences to come.
- **`jade-app` benchmarks**: binary-only (no `[lib]`), so `text_editor` internals can't be criterion-benchmarked; restructuring declined for now. Fixture: `crates/app/benches/fixtures/large_markdown_stress_test.md` (~11.3MB, seed `1337`, regenerate with `python3 crates/app/benches/fixtures/generate_stress_markdown.py`).
- **Permanently declined**: images, setext headings, indented code blocks, link reference definitions, raw HTML, hard/soft line-break distinction.
- Debug socket: `JADE_DEBUG_SOCKET=/path/to.sock cargo run -p jade-app --release` broadcasts the memory-probe lines to clients (`socat - UNIX-CONNECT:/path/to.sock`); read-only, std-only (`debug_socket.rs`).
