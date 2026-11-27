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
| [arrow-up-below-aside-goes-to-the-line-above](caret/arrow-up-below-aside-goes-to-the-line-above.toml) | E-1, B-TAG-9 | 9 | #16 |
| [a-pending-fence-in-a-float-changes-nothing-below](code/a-pending-fence-in-a-float-changes-nothing-below.toml) | P-OPEN-1, P-2 | 9 | #126 |
| [code-in-a-column-is-a-code-block](code/code-in-a-column-is-a-code-block.toml) | B-COL-3 | 9 | #126 |
| [enter-on-a-fence-with-an-info-string-keeps-it](code/enter-on-a-fence-with-an-info-string-keeps-it.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [enter-on-four-backticks-inserts-four](code/enter-on-four-backticks-inserts-four.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [open-fence-in-aside-stays-in-aside](code/open-fence-in-aside-stays-in-aside.toml) | B-TAG-4, B-CODE-3, P-2 | 9 | #126 |
| [typing-a-fence-in-a-float-closes-it-inside](code/typing-a-fence-in-a-float-closes-it-inside.toml) | P-OPEN-1a, B-CODE-3, P-2 | 9 | #126 |
| [columns-without-a-line-take-a-click-on-either-side](regions/columns-without-a-line-take-a-click-on-either-side.toml) | B-COL-6, V-1, V-4 | 9 | #116 |
| [ctrl-down-at-the-end-of-a-note-leaves-the-region](regions/ctrl-down-at-the-end-of-a-note-leaves-the-region.toml) | E-2, V-3, V-4, P-1 | 9 | #119 |
| [down-at-the-end-of-a-note-leaves-the-region](regions/down-at-the-end-of-a-note-leaves-the-region.toml) | V-1, V-3, V-4, P-1, E-1 | 9 | #119 |
| [empty-columns-take-a-click-on-either-side](regions/empty-columns-take-a-click-on-either-side.toml) | B-COL-1, B-COL-3 | 9 | #116 |
| [typing-above-a-region-at-the-start-creates-the-line](regions/typing-above-a-region-at-the-start-creates-the-line.toml) | V-4 | 9 | #119 |
| [typing-below-a-region-at-the-end-creates-the-line](regions/typing-below-a-region-at-the-end-creates-the-line.toml) | V-4 | 9 | #119 |
| [typing-in-a-right-float-keeps-the-caret-in-it](regions/typing-in-a-right-float-keeps-the-caret-in-it.toml) | B-ASIDE-1, B-ASIDE-2 | 9 | #117 |
| [up-at-the-start-of-a-note-leaves-the-region](regions/up-at-the-start-of-a-note-leaves-the-region.toml) | V-1, V-3, V-4, P-1, E-1 | 9 | #119 |
