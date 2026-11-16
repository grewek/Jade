# ARCHITECTURE.md

Non-obvious invariants, gotchas, and the Backlog. Not auto-loaded. The code is the source of truth for module structure — grep it.

## Invariants

- **Live Preview**: the cursor's line renders raw. A table renders entirely raw if the cursor is anywhere in it (`is_table_active`). Callouts are render-only (always expanded). List markers are *replaced* (`•`, `N. `), not just hidden. Lazy continuation is deliberately more lenient than CommonMark.
- **Text ownership**: note text exists once. `jade_core::Note` holds only id/path/title; the vault keeps the wikilink index and reads text from disk on demand (`read_note`, `save_note(&str)`, `search`). While a note is open its only copy is the editor buffer (`open_note` reads straight into it; autosave/flush write from the buffer's `&str`). No `.clone()`/`.to_string()` of the text anywhere (STYLE.md rule 14). Search reads files, so `JadeApp` caches results and recomputes only when `search_stale` is set by a mutation; searching runs on a background thread (`refresh_search` snapshots with `Vault::search_targets`, spawns, and drops results from superseded generations); never call `Vault::search` from `render`. Search sees disk, so unsaved edits show up after the 500ms autosave. The vault-open scan extracts wikilinks line by line (`link_targets_from_file`; a link never spans a line), and stores each note's distinct link targets only (`Box<[Box<str>]>`; the stress note's 27K links have 1 target), so scan peak is tiny, not a whole-file buffer (was a 14MiB peak; guard: `opening_a_vault_does_not_buffer_a_whole_note`).
- **Row heights**: `RowHeightCache` seeds non-table lines with an allocation-free over-estimate (`estimate_row_count`) and corrects to exact when painted, so total height starts slightly tall and settles while scrolling. Table rows always use the exact path. Validated against `exact_row_counts`. The first frame has no width yet (`render_params` falls back to `Pixels::MAX`, so every estimate is 1 row), and callout backgrounds are drawn before the lines from these heights; `LineLayout::verify_rows` therefore makes the visible window exact before the callout pass (guard: `first_paint_of_a_huge_note_needs_no_row_height_corrections`).
- **`DocumentCache` patching**: edits report a `TextEdit` to `Editor::record_edit`. A line-count change (Enter, joining lines, multi-line paste) resizes every per-line array and both Fenwick trees at one splice point (`resize_lines`, `Fenwick::resize`, `RowHeightCache::resize_for`), shifts stored line numbers of later tables/callouts, then runs the usual resumable walks. Undo/redo patch too: `EditorBuffer::undo`/`redo` return the `TextEdit` they applied (an undo entry is a delta). `classify_tables` (whole-document, with a per-line lookup) is now test-only: the oracle for the streaming `classify_table_list` production uses. A table's delimiter row needn't contain `|`, so `patch_tables` widens its window to fixpoint over adjacent tables and the header above. Validated by a randomized test (25 seeds) comparing every field, entering states included, against a freshly built cache.
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
- Guards: `opening_a_huge_note_keeps_live_memory_bounded` (vault scan + open note <3.5x the note size; the vault keeps no text), `search_results_are_cached_until_something_changes`, `line_count_changing_edits_do_not_rebuild_the_document` (Enter/Backspace-join: bytes allocated), `first_keystroke_in_a_huge_file_does_not_reallocate_the_document`, `real_keystrokes_in_a_huge_file_do_not_churn_the_whole_document` (bytes allocated per key), `real_keystrokes_in_a_huge_file_keep_undo_memory_bounded`, `scrolling_through_a_huge_file_does_not_leak` (`main.rs`; heap flat across 200 scroll steps, RSS ~160MB flat in the headless harness).
- Real app: `JADE_MEMORY_LOG=1 cargo run -p jade-app --release` logs RSS every 2s (`memory_probe.rs`).

## Backlog

Deliberately deferred — don't pick up without checking in (see `CLAUDE.md`).

- **Search on huge vaults**: search runs on a background thread, but its snapshot (`Vault::search_targets`, ~80ns/note on the UI thread) is O(notes): ~8ms at 100K notes, over budget past ~200K. Next step there: a cached `Arc` snapshot invalidated by note-set mutations, or an on-disk index.
- **Per-keystroke cost on the 320K-line stress fixture** (release, headless, so no GPU submit): a typed character is ~1.2ms edit + ~1.1ms render, Enter ~4ms + ~0.8ms, all under 16.66ms. ~1ms of the edit is `String::insert` memmoving the 11.8MB buffer; only a gap buffer/rope would remove it, at the cost of the contiguous `&str` that `text()` hands every consumer. The cache patch itself is ~0.1ms and allocates nothing (`callout_tail_scratch`, in-place `tables.splice`, `Table` stores no byte range: `DocumentCache::table_byte_range` derives it). Guard: `typing_a_character_in_a_huge_file_allocates_almost_nothing`.
- **Memory on the stress fixture**: opening the note adds ~30MB live in the editor (`DocumentCache` 16.4MB, buffer 11.3MB, `RowHeightCache` 2.5MB), and nothing more: the vault holds no note text (see Text ownership below). Construction streams into the arrays, so peak equals live. Per-line `table_for_line`/`callout_for_line` arrays are gone (`table_index_at`/`callout_index_at` binary-search `tables`/`callouts`); fence/callout entering states are packed to 8 bytes (`pack_fence`, `CalloutState::pack`). Remaining per-line cost: `open_block_states` (16B + list nodes), `line_index` and row counts (8B each as `i64`). Allocations come mostly from table rows (`scan_row`).
- **`Vault::backlinks()`** rebuilds its whole map per call (worst measured ~0.44ms on 200 notes × 20k words, now that the index holds distinct targets only).
- **Live Preview**: full CommonMark emphasis; formatting visible on the active line; heading sizes (bold only); theme-driven colors (currently hardcoded `Hsla`).
- **Callouts**: fold interactivity (with task-checkbox toggling and WYSIWYG tables, a later "interactions" phase); nested callouts.
- **WYSIWYG table editing**: not designed.
- **Settings dialog**: non-functional placeholder; themes, text size, editor preferences to come.
- **`jade-app` benchmarks**: binary-only (no `[lib]`), so `text_editor` internals can't be criterion-benchmarked; restructuring declined for now. Fixture: `crates/app/benches/fixtures/large_markdown_stress_test.md` (~11.3MB, seed `1337`, regenerate with `python3 crates/app/benches/fixtures/generate_stress_markdown.py`).
- **Permanently declined**: images, setext headings, indented code blocks, link reference definitions, raw HTML, hard/soft line-break distinction.
- Debug server: `JADE_DEBUG_SOCKET=/path/to.sock cargo run -p jade-app --release` speaks `jade-debug-protocol` (newline-delimited JSON, `crates/debug-protocol`; code in `debug_server/`). Nothing is pushed until a client sends `{"req":1,"cmd":"subscribe","topic":"paint"}` (topics: paint, memory, frames, input); every request gets a `response` with its `req`, events arrive as `{"kind":"event",...}`. `await` replies with the next event on a topic, or errors on timeout. Commands that need windows run on the GPUI main thread via a waker-based pump (`pump.rs`, no polling). `paint` events carry what the editor canvas painted (`text_editor/paint_trace.rs`: bands, visible lines, wrap width, verified window), only when a frame differs from the last, so a bug that fixes itself on the next frame is still seen; `consistent: false` means a callout band disagrees with its lines or painting corrected heights after the bands were placed. The socket is mode 0600. Cost when off: one cached bool per paint. Tests: `debug_server::test_client::TestClient`; a test that wakes the pump from a server thread needs `cx.executor().allow_parking()`. Implemented so far: subscriptions, `await`, `query memory|window_size`; input injection, `list_targets`, frame times and editor state are next.
