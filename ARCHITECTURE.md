# ARCHITECTURE.md

Module map: current types, functions, and known limitations. Not auto-loaded — read the relevant section before working in an area. Standing rules and commands live in `CLAUDE.md`. Keep additions terse.

## `crates/core` (`jade-core`)

UI-independent, std-only (`tempfile` is a dev-dependency for test vaults).

- `note.rs`: `Note` (`id` = `/`-separated relative path without extension, `title` = file stem, raw `content`). `relative_id` strips the extension (notes); `relative_path_id` keeps it (folders).
- `links.rs`: `extract_wikilinks` — hand-written parser for `[[Target]]`, `[[Target|Alias]]`, `[[Target#Heading|Alias]]`.
- `vault.rs`: `Vault` scans a root for `.md` files (`notes: HashMap<id, Note>`) and directories (`folders: HashSet<id>`).
  - `backlinks()` rebuilds a target→sources map per call, resolving by id, then case-insensitive title (`title_index()`), using a private incrementally-maintained `wikilinks` cache. `search()` is a naive substring match over title/content.
  - Mutations: `create_note`/`create_folder` (dedupe " 1", " 2"…), `update_note_content` (memory + disk), `rename_note`, `delete_note`, `rename_folder` (single `fs::rename`, then rebuild subtree entries), `delete_folder` (recursive).
  - `Vault::empty(root)` is the fallback when `Vault::open` fails.
- No Markdown rendering in core (would need a parser crate, requires approval).
- Tests use `tempfile::tempdir()` per test (same in app UI tests).

## `crates/app` (`jade-app`)

Binary crate; depends on `jade-core` and `gpui-kit = "0.7"` (re-exports `gpui-component` as `gpui_kit::component`).

- `test_alloc.rs` (`#[cfg(test)]` only): counting `GlobalAlloc` wrapper tracking live/peak bytes and allocation count; `reset()` rebases `peak` but must never zero `current`.

### `text_editor/`

Own editor on raw GPUI primitives (not `gpui-component`'s `Textarea`). Phase 1 (plain editing) done; Phase 2 (Live Preview) covers inline styles, lists, code fences, tables, callouts.

- `buffer.rs`: `EditorBuffer` — GPUI-free `String` + `cursor`/`anchor`; char-boundary (not grapheme) movement; sticky `preferred_column`. `insert`/`delete_range` push to a linear undo/redo stack, coalescing contiguous same-kind bursts (`last_edit: Option<LastEdit>`), capped at `UNDO_STACK_CAP` (100). Snapshots still clone the full text (a rope would be the next step; not planned).
- `live_preview.rs`: inline model. `build_display_line(line, active) -> DisplayLine` hides markup and tags text with `InlineStyle`; `DisplayLine` has `display_text` + `segments` for raw↔display mapping (`display_offset_for_raw`/`raw_offset_for_display`). List markers are *replaced* (`•`, canonical `N. `; `is_replaced()`). Active (cursor) line renders raw.
  - Supports: headings, bold/italic/strike, inline code, wikilinks, inline links and autolinks, blockquote prefixes, list markers, task checkboxes (render-only), backslash escapes, HTML entities, thematic breaks. One level of compounding (`> # H`, `- > q`).
  - Emphasis matching is simplified, not CommonMark flanking rules. Images are not rendered.
  - `classify_block_context(source) -> Vec<LineContext>` resolves list depth and lazy continuation (lenient: a continuation stays in the open list/quote regardless of indentation). `OpenBlock::List` uses a persistent `Rc`-linked `ListNode` stack.
  - `build_callout_header_line` renders callout headers; `hidden_line` handles fence delimiters/rules.
- `block_preview.rs`: `classify_lines(source) -> Vec<LineRole>` (`Plain`/`FenceDelimiter`/`FenceContent`) for ``` and `~~~` fences.
- `table_preview.rs`: `scan_row`, `classify_tables(source) -> (per-line table index, Vec<Table>)`. `Table` has line/byte ranges and `Column { width, align }`.
  - Rendered as an aligned grid (monospace font): cells padded, grid lines painted as vector quads in `paint_lines`. Cell text is not inline-scanned.
  - Cursor anywhere in a table renders it entirely raw (`is_table_active`).
  - Too-wide tables: `cap_column_widths` (min `MIN_COLUMN_WIDTH`) then `wrap_cell_text`; wrapped rows get coarser click targets. No tables inside quotes/lists or fences.
- `callout.rs`: `classify_callouts` finds Obsidian callouts (`> [!type]+/-`, top-level only). `resolve_callout_type` maps aliases to 13 `CalloutType`s (default `Note`); `callout_icon` gives glyphs. Render-only: always expanded, fold marker ignored. Tint + accent bar are painted as quads before the text loop.
- `element.rs`: `render_editor` composes a `div()` with a `canvas()` for painting and mouse hit-testing; mouse listeners are re-registered each paint and share one `Rc<PaintSnapshot>`. `shape_display_line` shapes styled runs (colors are hardcoded `Hsla` constants, not theme-driven). `indent_px(level)` applies list/quote indentation; blockquotes get a quote bar per level.
  - **Soft wrap**: `shape_text` with `wrap_width_for` (`None` for table rows/rules); painting and hit-testing accumulate wrapped-row heights.
  - **Viewport-bounded**: `LineLayout` (from `analyze_line_layout`) queries `DocumentCache`/`RowHeightCache` in O(log n); `visible_line_range` binary-searches the viewport (+ `OVERSCAN_LINES`). Only visible lines are shaped.
  - **`RowHeightCache`**: `Fenwick` of per-line wrapped-row counts. Invalidated by edits (`Editor::record_edit`/`invalidate_all`), cursor moves (`sync_active_line`), and resize/font changes (`needs_full_rebuild` → `rebuild`). `rebuild` uses `estimate_row_count` (allocation-free over-estimate from raw length + indent) for non-table lines; table rows use the exact path. Estimates are corrected to exact when painted (`correct_row_count`), so total height starts slightly tall and settles while scrolling. Verified against an `exact_row_counts` oracle.
  - Autoscroll: `autoscroll_to_cursor`, gated by `last_autoscrolled_cursor` so manual scrolling isn't undone.
- `document_cache.rs` + `fenwick.rs`: `DocumentCache` (on `Editor`, `Rc<RefCell<_>>`) caches per-line classification. Edits report a `TextEdit {start_byte, old_end_byte, new_end_byte}` to `Editor::record_edit`, which patches via resumable state machines (fences, callouts, block context) and a bounded window rescan (tables); undo/redo and any edit changing the line count fully recompute. `record_edit` returns the touched line range for `RowHeightCache`. Patches are validated against whole-document oracles by property tests.
- `input_handler.rs`: `EntityInputHandler for Editor` (UTF-16 offsets at the IME boundary; `character_index_for_point` returns `None`). `resolve_replace_range` borrows the buffer only when a range is given.
- `editor.rs`: `Editor` owns one `EditorBuffer`, `FocusHandle`, and `scroll_handle`. `handle_key_down` covers navigation, editing, and clipboard; typing goes through `replace_text_in_range`. `EditorEvent::Changed` fires only on text changes. Tab uses an `InsertTab` action in the `"TextEditor"` key context (test windows must register the binding).

### `main.rs`

Takes an optional vault path (default `./vault`) and renders `JadeApp`.

- `vault/Markdown Showcase.md` is a local, gitignored fixture showing every supported Live Preview construct. **Update it in the same change that adds or changes a construct.**
- Sidebar tabs (`sidebar_tab`: `Vault`/`Search`) are plain styled divs. The `"NoteList"` key context (F2/Backspace) wraps only the Vault tree and is active while `sidebar_focus` is focused.
  - **Vault**: New note/folder buttons and a recursive collapsible tree (`render_tree_level`/`render_folder_row`/`render_note_row`; folders first, alphabetical). Folder row ids are `"folder-{id}"`; collapse state in `collapsed_folders`. Right-click creates inside a folder. Indent with `.ml(...)`, not padding.
  - **Search**: `search_input` filters via `Vault::search()` on `InputEvent::Change`; results keyed `"search-result-{id}"`.
- **Right pane**: one `text_editor::Editor` per open note plus a "Backlinks (n)" panel (`"backlink-{note_id}"`).
- **Autosave**: `open_note` subscribes to `EditorEvent::Changed`; `autosave_editor` calls `Vault::update_note_content`. `flush_editor` persists and drops the editor before `open_note`/renames.
- **Rename/delete**: context menu or hotkey (folders: menu only). Dialogs take target + `Entity<JadeApp>` + `window`/`cx` explicitly. Rename/New-folder use `Dialog` + `Input` with `.on_ok(...)` matching the footer button (Enter dispatches to `on_ok`); delete uses `AlertDialog`. Folder rename/delete flush the editor and reconcile `collapsed_folders` and the open editor.
- **Command palette** (`Ctrl+P`): global `KeyBinding` + `cx.on_action::<OpenCommandPalette>` (element `.on_action` only sees the focused path). `CommandState` must be focused explicitly. One item: "Open Settings".
- **Settings dialog** (`open_settings_dialog`): non-functional placeholder.
- Raw `window.on_mouse_event` listeners aren't hitbox-aware: Down/Move must bail on `window.has_active_dialog(cx)`.
- Icons need `.with_assets(gpui_kit::assets::Assets)` in `main()`.
- To update another entity from a dialog callback: `entity.update(cx, |s, cx| s.method(window, cx))`, not `update_in`.
- **Fonts**: IBM Plex Sans/Mono (OFL 1.1) in `crates/app/assets/fonts/`, embedded via `include_bytes!` in `install_fonts(cx)`.
- **Headless UI tests** (`cargo test -p jade-app -- --test-threads=1`): `gpui-kit` `test-support` renders `JadeApp` off-screen with real hit-testing (`click`/`press`/`input`/`right_click` by id).
  - Clickable/findable elements need `.test_support()` plus `.id(...)`.
  - Overlays open asynchronously: `cx.run_until_parked()` between separate `cx.update_window(...)` calls.
  - Import gpui-kit items explicitly (a glob shadows `#[test]`).
  - A `None`-context `KeyBinding` needs a focused element; clicking a non-`.track_focus()` div doesn't focus.

## Backlog

Deliberately deferred — don't pick up without checking in (see `CLAUDE.md`).

- **Per-keystroke cost on the 320K-line stress fixture**: render ~13–16ms; edit side adds ~15–20ms, traced to GPUI's `dispatch_keystroke` (vendored), not Jade code. A full edit+render cycle is not yet under 16.66ms.
- **Memory on the stress fixture** (opening it retains ~77MB over ~1.9M allocations): `EditorBuffer.text` ~11.3MB, `DocumentCache` ~38.8MB, `RowHeightCache` ~2.6MB. Remaining allocations come from table rows on the exact path.
- **`Vault::backlinks()`** rebuilds its whole map per call (worst measured ~17ms on 200 notes × 20k words). Incremental maintenance not done.
- **Live Preview**: full CommonMark emphasis; formatting visible on the active line; heading sizes (bold only); theme-driven colors.
- **Callouts**: fold interactivity (with task-checkbox toggling and WYSIWYG tables, a later "interactions" phase); nested callouts.
- **WYSIWYG table editing**: not designed.
- **Settings dialog**: themes, text size, editor preferences.
- **`jade-app` benchmarks**: binary-only (no `[lib]`), so `text_editor` internals can't be criterion-benchmarked; restructuring declined for now. Fixture: `crates/app/benches/fixtures/large_markdown_stress_test.md` (~11.3MB, seed `1337`, regenerate with `python3 crates/app/benches/fixtures/generate_stress_markdown.py`).
- **Permanently declined**: images, setext headings, indented code blocks, link reference definitions, raw HTML, hard/soft line-break distinction.
