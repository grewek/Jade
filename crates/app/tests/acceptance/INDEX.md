# Acceptance catalog

Generated from the scenario files; do not edit. After changing one, run
`JADE_UPDATE_INDEX=1 cargo test -p jade-app acceptance_index -- --test-threads=1`.

| Scenario | SPEC rules | Stage | Source |
|---|---|---|---|
| [a-click-below-the-text-moves-the-caret-to-the-end](caret/a-click-below-the-text-moves-the-caret-to-the-end.toml) | – | 2 | 2f9218a |
| [a-heading-keeps-its-height-with-the-caret-in-it](caret/a-heading-keeps-its-height-with-the-caret-in-it.toml) | P-3, P-LP-2 | 2 | 719e1e4, 225621f |
| [the-view-follows-the-caret-downwards](caret/the-view-follows-the-caret-downwards.toml) | – | 2 | e1ce888 |
| [a-composed-character-is-typed-once](editing/a-composed-character-is-typed-once.toml) | – | 2 | 54988fd |
| [tab-types-a-tab-character](editing/tab-types-a-tab-character.toml) | – | 2 | e1ce888 |
| [undo-takes-back-a-typed-burst](editing/undo-takes-back-a-typed-burst.toml) | – | 2 | 9359b29 |
| [scrolling-back-over-unmeasured-paragraphs-does-not-jump](scroll/scrolling-back-over-unmeasured-paragraphs-does-not-jump.toml) | – | 2 | 6f26122, 18233f8, 59f87bc |
| [a-list-stays-whole-with-the-caret-in-an-item](lists/a-list-stays-whole-with-the-caret-in-an-item.toml) | P-3, B-LIST-1 | 3 | c398e57 |
| [list-gap-keeps-lines-below-in-place](lists/list-gap-keeps-lines-below-in-place.toml) | P-3, P-LP-2 | 3 | #120 |
| [ticking-a-task-keeps-the-view](lists/ticking-a-task-keeps-the-view.toml) | – | 3 | c9d98b9 |
| [a-closing-fence-may-end-in-a-space](code/a-closing-fence-may-end-in-a-space.toml) | B-CODE-3 | 4 | e3b27de |
| [a-break-tag-still-breaks-the-line-in-the-edited-cell](tables/a-break-tag-still-breaks-the-line-in-the-edited-cell.toml) | B-TAB-2, P-LP-4 | 5 | 506c8ed |
| [a-column-is-as-wide-as-its-drawn-formula](tables/a-column-is-as-wide-as-its-drawn-formula.toml) | B-TAB-4 | 5 | e77120c |
| [a-delimiter-typed-below-a-header-makes-a-table](tables/a-delimiter-typed-below-a-header-makes-a-table.toml) | B-TAB-3 | 5 | ed47bf8 |
| [a-tables-delimiter-line-takes-no-height](tables/a-tables-delimiter-line-takes-no-height.toml) | B-TAB-3 | 5 | 534f7ed |
| [shrunk-columns-keep-words-whole](tables/shrunk-columns-keep-words-whole.toml) | B-TAB-5 | 5 | 089ceb9 |
| [spaces-typed-at-a-cells-end-keep-the-row-height](tables/spaces-typed-at-a-cells-end-keep-the-row-height.toml) | B-TAB-3 | 5 | 63d90f7 |
| [the-edited-cell-is-left-aligned](tables/the-edited-cell-is-left-aligned.toml) | B-TAB-7 | 5 | 5d63880 |
| [a-rewritten-fold-marker-opens-the-callout](callouts/a-rewritten-fold-marker-opens-the-callout.toml) | B-CALL-4, P-VIEW-2 | 6 | f165f92 |
| [callout-backgrounds-are-right-from-the-first-frame](callouts/callout-backgrounds-are-right-from-the-first-frame.toml) | – | 6 | 8df19a7 |
| [callout-backgrounds-follow-a-new-width](callouts/callout-backgrounds-follow-a-new-width.toml) | – | 6 | 870f6e8 |
| [folding-a-callout-keeps-the-view](callouts/folding-a-callout-keeps-the-view.toml) | B-CALL-4, P-VIEW-1 | 6 | 49e8aed |
| [a-blank-line-ends-a-formula-block](math/a-blank-line-ends-a-formula-block.toml) | B-MATH-3 | 7 | 0e65772 |
| [a-drawn-formula-stays-whole-in-a-wrapping-cell](math/a-drawn-formula-stays-whole-in-a-wrapping-cell.toml) | B-TAB-5 | 7 | 31217ac |
| [a-formula-wider-than-its-column-is-shown-as-source](math/a-formula-wider-than-its-column-is-shown-as-source.toml) | B-TAB-6 | 7 | fa19b0b |
| [a-wide-formula-block-is-shown-as-source](math/a-wide-formula-block-is-shown-as-source.toml) | B-MATH-4, P-4 | 7 | #165 |
| [a-wide-inline-formula-is-shown-as-source-in-every-block](math/a-wide-inline-formula-is-shown-as-source-in-every-block.toml) | I-MATH-2 | 7 | fa19b0b |
| [only-the-row-with-a-tall-formula-is-tall](math/only-the-row-with-a-tall-formula-is-tall.toml) | – | 7 | 4002239 |
| [scrolling-back-over-unmeasured-tall-formulas-does-not-jump](math/scrolling-back-over-unmeasured-tall-formulas-does-not-jump.toml) | – | 7 | 6f26122, 83537ae |
| [arrow-up-below-aside-goes-to-the-line-above](caret/arrow-up-below-aside-goes-to-the-line-above.toml) | E-1, B-TAG-9 | 9 | #16 |
| [a-pending-fence-in-a-float-changes-nothing-below](code/a-pending-fence-in-a-float-changes-nothing-below.toml) | P-OPEN-1, P-2 | 9 | #126 |
| [code-in-a-column-is-a-code-block](code/code-in-a-column-is-a-code-block.toml) | B-COL-3 | 9 | #126 |
| [enter-on-a-fence-with-an-info-string-keeps-it](code/enter-on-a-fence-with-an-info-string-keeps-it.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [enter-on-four-backticks-inserts-four](code/enter-on-four-backticks-inserts-four.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [open-fence-in-aside-stays-in-aside](code/open-fence-in-aside-stays-in-aside.toml) | B-TAG-4, B-CODE-3, P-2 | 9 | #126 |
| [typing-a-fence-in-a-float-closes-it-inside](code/typing-a-fence-in-a-float-closes-it-inside.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [a-table-does-not-paint-into-the-next-column](regions/a-table-does-not-paint-into-the-next-column.toml) | B-COL-3, B-TAB-4 | 9 | c63c311 |
| [a-table-stays-inside-a-narrow-float](regions/a-table-stays-inside-a-narrow-float.toml) | B-ASIDE-3, B-TAB-4 | 9 | 571eab7 |
| [a-tall-formula-grows-its-line-in-columns-and-asides](regions/a-tall-formula-grows-its-line-in-columns-and-asides.toml) | B-COL-3, B-ASIDE-2 | 9 | d641572 |
| [columns-without-a-line-take-a-click-on-either-side](regions/columns-without-a-line-take-a-click-on-either-side.toml) | B-COL-6, V-1, V-4 | 9 | #116 |
| [ctrl-down-at-the-end-of-a-note-leaves-the-region](regions/ctrl-down-at-the-end-of-a-note-leaves-the-region.toml) | E-2, V-3, V-4, P-1 | 9 | #119 |
| [deleting-at-a-tag-line-never-joins-text-to-it](regions/deleting-at-a-tag-line-never-joins-text-to-it.toml) | B-TAG-12, P-1 | 9 | 21991dc |
| [deleting-at-a-tag-line-removes-an-empty-line](regions/deleting-at-a-tag-line-removes-an-empty-line.toml) | B-TAG-12 | 9 | 21991dc |
| [down-at-the-end-of-a-note-leaves-the-region](regions/down-at-the-end-of-a-note-leaves-the-region.toml) | V-1, V-3, V-4, P-1, E-1 | 9 | #119 |
| [dragging-a-divider-keeps-the-view](regions/dragging-a-divider-keeps-the-view.toml) | L-10, P-3 | 9 | 3a3e6db |
| [empty-columns-take-a-click-on-either-side](regions/empty-columns-take-a-click-on-either-side.toml) | B-COL-1, B-COL-3 | 9 | #116 |
| [moving-the-caret-in-a-float-region-keeps-the-view](regions/moving-the-caret-in-a-float-region-keeps-the-view.toml) | P-3, B-ASIDE-1 | 9 | 23a04aa |
| [the-caret-stays-with-its-text-when-a-column-is-added](regions/the-caret-stays-with-its-text-when-a-column-is-added.toml) | L-5, L-13 | 9 | a0e7edb |
| [typing-above-a-region-at-the-start-creates-the-line](regions/typing-above-a-region-at-the-start-creates-the-line.toml) | V-4 | 9 | #119 |
| [typing-below-a-region-at-the-end-creates-the-line](regions/typing-below-a-region-at-the-end-creates-the-line.toml) | V-4 | 9 | #119 |
| [typing-in-a-right-float-keeps-the-caret-in-it](regions/typing-in-a-right-float-keeps-the-caret-in-it.toml) | B-ASIDE-1, B-ASIDE-2 | 9 | #117 |
| [undoing-a-layout-command-restores-the-caret](regions/undoing-a-layout-command-restores-the-caret.toml) | L-6, L-12, L-14 | 9 | 6cb7e50 |
| [up-at-the-start-of-a-note-leaves-the-region](regions/up-at-the-start-of-a-note-leaves-the-region.toml) | V-1, V-3, V-4, P-1, E-1 | 9 | #119 |
| [what-follows-a-tall-float-starts-below-it](regions/what-follows-a-tall-float-starts-below-it.toml) | B-ASIDE-1 | 9 | e37eb13 |

## Still owed

| Source | Stage | Scenario |
|---|---|---|
| 62bc2b0 | 5 | A table's grid lines have no gaps (pixels) |
| 384fe02 | 6 | A rounded callout's accent bar is its left edge (pixels) |
| 87d5c2d | 7 | A formula's thin rules are one device pixel thick (pixels) |
| #126 | 9 | Code in a column or float has its box and language label (pixels) |

## Fix commits without a scenario

| Reason | Commits |
|---|---|
| Covered by the empty-column scenarios | 5b52ccc, 29450f6 |
| Covered by open-fence-in-aside-stays-in-aside (discarded #126 branch) | ddd0627 |
| Documentation or the debug harness | be8c245, 2fedb09, c4f459e, e0626ce |
| Speed or memory only; the benchmarks guard these | c046850, 2ebbab3, 8ad94fe, 9f28011 |
| The widget that was fixed no longer exists | 914e293 |
| The app around the note: dialogs, tab bar, sidebars, the right-click menu | 9c1ba19, db554f3, 562d5cd, 5ea91da |
| `f32` precision in a 105 MB note, too large to run as a scenario | 362241e, 32ccc5e |
