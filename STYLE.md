# STYLE.md

Coding rules derived from [Tigerstyle](https://tigerstyle.dev) (TigerBeetle's engineering philosophy), adapted for Jade's stack (Rust, GPUI, a third-party UI dependency Jade does not fork). Not auto-loaded, but mandatory: `CLAUDE.md` requires reading and following it whenever writing or reviewing code, docs, or comments.

Several of these overlap with standing rules already in `CLAUDE.md`; where that's the case it's noted rather than duplicated as a new requirement.

## Safety

1. **Assert liberally.** Check pre-conditions, post-conditions, and invariants with `assert!`/`debug_assert!` instead of only assuming them — especially at module boundaries (`jade-core` ↔ `jade-app`).
2. **Explicit upper bounds on loops and recursion** whenever the bound depends on user-controlled input (markdown nesting depth, backlink traversal) — prevents stack overflow or hangs on malicious or malformed input.
3. **Handle every error explicitly.** No `.unwrap()`/`.expect()` on paths that depend on filesystem or user input; propagate `Result` instead of swallowing it.
4. **Narrow, clearly named function interfaces.** One function does one thing; don't use parameter lists as a dumping ground for loosely related state.
5. **Pre-size and reuse buffers in hot paths** instead of reallocating per call. No `Vec::new()` inside a loop that runs every frame — size it once with `with_capacity`/`reserve` and reuse it (this is already the direction of the recent `RowHeightCache` / undo-stack-cap work).

## Performance

6. **Napkin math before optimizing.** Estimate the relevant order of magnitude (data size, call frequency, known hardware/library constants) before tuning anything. This is a pre-filter for deciding whether an optimization is worth attempting at all — it never substitutes for the real numbers `CLAUDE.md`'s "Always measure" rule requires.
7. **Never optimize without measuring.** Already required by `CLAUDE.md`'s Performance section — restated here only because it's where Tigerstyle's version of the rule comes from.
8. **Count allocations in hot-path benchmarks, not just elapsed time.** Use an allocation-counting `GlobalAlloc` wrapper in Criterion benches and assert an upper bound per frame/call, rather than assuming allocation behavior is bounded.
9. **Keep one-time costs (startup, note-open, vault scan) explicitly separate from steady-state per-frame/per-call costs.** Already established in `CLAUDE.md`; restated here for completeness.
14. **Clone sparingly in hot paths.** `.clone()`, `.to_string()`, `.to_owned()` and `.to_vec()` on document-sized data (note text, per-line cache vectors) never go in per-keystroke or per-frame code; borrow (`&str`, `&[T]`), or share with `Rc`/`Arc` where ownership is unavoidable. `Rc::clone`/handle clones (`Entity`, `FocusHandle`) are fine. A copy that exists only to satisfy the borrow checker is a smell: restructure the borrows instead. Measure with `test_alloc`'s `total` (cumulative bytes) per call, not just live bytes. Precedent: a per-frame snapshot plus per-key text copies cost 37MB/keystroke on the stress fixture (now 1.4MB).

## Developer experience / style

10. **Length limits as a guideline**: functions roughly ≤ 70 lines, lines ≤ ~100 columns. Can be enforced with `clippy::too_many_lines` where it's worth the friction.
11. **Comments explain why, not what.** Keep code comments short; never explain behavior that's obvious from reading the code, only the non-obvious *why*.
11a. **Don't over-document.** Keep API docs (doc comments, `ARCHITECTURE.md` entries) small and concise.
12. **Small, atomic commits with the rationale in the message**, not just a description of the diff.
13. **Consistent naming across parallel structures** (e.g. `*_cache`, `*_stack` named the same way across modules). Lowest priority of this list — cosmetic, not load-bearing.

## Explicitly not adopted

- **TigerBeetle-style static allocation (zero allocations after init).** Not achievable in Jade: GPUI's own render pipeline allocates per frame internally (element tree construction, Taffy layout nodes, text shaping buffers), and Jade does not fork GPUI (see `CLAUDE.md`'s Vision section). Rules 5 and 8 above are the realistic translation of this idea — pre-size and reuse what Jade's own code controls, and measure allocation counts in benchmarks instead of assuming they're bounded.
- **Zig-specific mechanisms** (e.g. an explicit allocator parameter threaded through every function signature). No idiomatic Rust equivalent that's worth the ergonomic cost at Jade's current size.
