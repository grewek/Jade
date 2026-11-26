# Acceptance catalog

Generated from the scenario files; do not edit. After changing one, run
`JADE_UPDATE_INDEX=1 cargo test -p jade-app acceptance_index -- --test-threads=1`.

| Scenario | SPEC rules | Stage | Source |
|---|---|---|---|
| [list-gap-keeps-lines-below-in-place](lists/list-gap-keeps-lines-below-in-place.toml) | P-3, P-LP-2 | 3 | #120 |
| [arrow-up-below-aside-goes-to-the-line-above](caret/arrow-up-below-aside-goes-to-the-line-above.toml) | E-1, B-TAG-9 | 9 | #16 |
| [code-in-a-column-is-a-code-block](code/code-in-a-column-is-a-code-block.toml) | B-COL-3 | 9 | #126 |
| [open-fence-in-aside-stays-in-aside](code/open-fence-in-aside-stays-in-aside.toml) | B-TAG-4, B-CODE-3, P-2 | 9 | #126 |
| [typing-a-fence-in-a-float-closes-it-inside](code/typing-a-fence-in-a-float-closes-it-inside.toml) | P-OPEN-1, B-CODE-3, P-2 | 9 | #126 |
| [down-at-the-end-of-a-note-leaves-the-region](regions/down-at-the-end-of-a-note-leaves-the-region.toml) | E-1 | 9 | #119 |
| [empty-columns-take-a-click-on-either-side](regions/empty-columns-take-a-click-on-either-side.toml) | B-COL-1, B-COL-3 | 9 | #116 |
| [typing-in-a-right-float-keeps-the-caret-in-it](regions/typing-in-a-right-float-keeps-the-caret-in-it.toml) | B-ASIDE-1, B-ASIDE-2 | 9 | #117 |
