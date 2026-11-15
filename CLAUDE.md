# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository. It's deliberately minimal — standing rules and commands only, since this file is reloaded in full on every turn.

- **`ARCHITECTURE.md`** — non-obvious invariants, GPUI/test gotchas, and the Backlog. Not auto-loaded; read before touching the editor, dialogs, or UI tests.
- **`STYLE.md`** — Tigerstyle-derived coding rules plus documentation/comment rules. Not auto-loaded, but **mandatory: read it before writing or reviewing any code, docs, or comments, and always follow it.**

## Vision

Jade is an Obsidian-like note-taking app, written in Rust, with these firm decisions:

- **UI framework: GPUI** (the framework built for the Zed editor) — chosen specifically for performance. No web technology (no Tauri/Electron/webview) anywhere in the app, including for the drawing canvas described below.
- **UI components: prefer `gpui-kit`** (community component library built on GPUI) over hand-rolling widgets; only build a custom component when `gpui-kit` has no equivalent.
- **Native Excalidraw-style canvas**: a planned drawing/diagramming feature equivalent to Obsidian's Excalidraw plugin, implemented natively on GPUI's own rendering primitives — not a port or embedding of Excalidraw's web (JS/SVG) implementation.
- **Storage format**: plain Markdown files on disk (Obsidian-vault-like), not a database.
- **Phasing**: core foundation (vault handling, markdown parsing, note linking/backlinks, search) must be stable before the native Excalidraw-style canvas is built. A project-management feature is planned even later, after the canvas.

**Do not add dependencies to any `Cargo.toml` without first asking the user for explicit approval** — this applies even to crates already agreed upon in principle (e.g. `gpui`, `gpui-kit`), ask again at the point of actually adding them.

**`ARCHITECTURE.md`'s Backlog section lists deliberately deferred follow-ups — don't pick one up without checking in with the user first.**

## Performance

Performance is a standing, firm requirement, not an afterthought to clean up once something feels slow:

- **Always measure** methods/functions/structs for CPU and memory inefficiency — profile/time directly (e.g. against `crates/app/benches/fixtures/large_markdown_stress_test.md`) rather than reasoning about Big-O alone. A plan that "sounds cheap" still gets measured before being called done.
- **The bar for anything rendered or updated on every frame is 16.66ms (60fps)**, full stop — not "better than before," not "much faster than the broken baseline." If a change can't hit that, say so plainly and either keep closing the gap or explicitly queue the remaining work as the *next* thing (not indefinite backlog). Lower than 16.66ms is still better than exactly 16.66ms — treat the budget as a ceiling to clear, not a target to land on.
- **If an abstraction costs real performance, don't just accept the cost for the abstraction's sake.** Work out *why* it's expensive, then either find a cheaper abstraction that keeps the same clarity, or drop the abstraction entirely if nothing cheaper preserves it.
- This applies everywhere in the codebase (`jade-core` included, not just `jade-app`'s render loop) — a one-time cost at startup/file-open is a separate, more negotiable question from steady-state per-frame/per-call cost; ask if that distinction matters for a specific case rather than assuming either way.

## Commands

Requires nightly Rust (`gpui-pre` uses an unstable feature) — pinned via `rust-toolchain.toml`, so plain `cargo` commands pick it up automatically; no `+nightly` needed. Edition 2024.

Workspace-wide:
- Build: `cargo build`
- Run the app: `cargo run -p jade-app`
- Release build: `cargo build --release` (~37MB stripped binary)
- **A debug build (`cargo run -p jade-app` with no `--release`) can feel noticeably slow to open/render a note — this is expected GPU/compositor overhead from unoptimized codegen, not a code bug.** Don't try to "fix" a debug-build-only slowdown in the rendering code itself; verify against `--release` first.
- Check (fast compile check without producing a binary): `cargo check`
- Test all crates: `cargo test -- --test-threads=1`
- Test a single crate: `cargo test -p jade-core` / `cargo test -p jade-app -- --test-threads=1`
- Run a single test: `cargo test <test_name>`
- **`jade-app`'s headless UI tests require `--test-threads=1`** — the parallel harness is non-deterministic under load (a confirmed concurrency bug in the vendored gpui/gpui-component test-support stack, not Jade's code).
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Benchmark: `cargo bench -p jade-core` (criterion; watch for a `change:`/`Performance has regressed` line, not just the absolute time). `jade-app` has no benchmarks currently (binary-only, no `[lib]` target — see `ARCHITECTURE.md`'s Backlog).
