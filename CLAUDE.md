# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository. It's deliberately minimal — standing rules and commands only, since this file is reloaded in full on every turn.

- **`ARCHITECTURE.md`** — rules and invariants, GPUI/test gotchas. Not auto-loaded; read before touching the editor, dialogs, or UI tests.
- **`SPEC.md`** — what Jade Markdown is and how it is displayed. Not auto-loaded; read before touching parsing or rendering.
- **`STYLE.md`** — Tigerstyle-derived coding rules plus documentation/comment rules. Not auto-loaded, but **mandatory: read it before writing or reviewing any code, docs, or comments, and always follow it.**

## Vision

Jade is an Obsidian-like note-taking app, written in Rust, with these firm decisions:

- **UI framework: GPUI** (the framework built for the Zed editor) — chosen specifically for performance. No web technology (no Tauri/Electron/webview) anywhere in the app, including for the drawing canvas described below.
- **UI components: prefer `gpui-kit`** (community component library built on GPUI) over hand-rolling widgets; only build a custom component when `gpui-kit` has no equivalent.
- **Native Excalidraw-style canvas**: a planned drawing/diagramming feature equivalent to Obsidian's Excalidraw plugin, implemented natively on GPUI's own rendering primitives — not a port or embedding of Excalidraw's web (JS/SVG) implementation.
- **Storage format**: plain Markdown files on disk (Obsidian-vault-like), not a database.
- **Phasing**: core foundation (vault handling, markdown parsing, note linking/backlinks, search) must be stable before the native Excalidraw-style canvas is built. A project-management feature is planned even later, after the canvas.

**Do not add dependencies to any `Cargo.toml` without first asking the user for explicit approval** — this applies even to crates already agreed upon in principle (e.g. `gpui`, `gpui-kit`), ask again at the point of actually adding them.

## Git workflow (GitFlow) and issues

- **`main`** holds tagged releases only; **`develop`** is the integration branch. Never commit or push directly to either, and never force-push. Merge with merge commits (`--no-ff`), never squash or rebase.
- **Work** happens on `feature/<issue#>-<slug>` cut from `develop` and ends in a PR into `develop`. Don't start a `release/*` or `hotfix/*` branch or create a tag unless asked.
- **Release / hotfix** (when asked): `release/x.y.z` from `develop`, `hotfix/x.y.z` from `main`; each goes by PR into `main` (then tag `vX.Y.Z`) and by a second PR back into `develop`.
- **Issues:** read the whole issue and its comments first, then do only what it asks. A new dependency needs a comment asking the user, not a guess. Reference the issue in commits (`refs #n`); the PR targets `develop`, and its body says `Closes #n`.
- **`CHANGELOG.md`** is edited only when preparing a release, on the `release/*` branch (newest version first, one entry per user-visible change). Never touch it in feature or hotfix work.
- **Before opening the PR**, run what CI runs (`.github/workflows/ci.yml`, all four crates): `cargo fmt --all --check`, `cargo clippy --lib --bins -- -D warnings`, `cargo test -- --test-threads=1`, and `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`. Opening the PR ends the task; merging is the user's call.
- **Reviewable steps:** one topic per PR, at most ~500 changed lines of production code (tests count separately), commits that each carry one reason. Cut larger work into PRs first and show the cut to the user.

## Rendering (rebuild in progress: #130)

The editor's layout core moves from lines to blocks (#130, milestone "Rendering core"). Until that milestone is done:

- **Feature freeze:** no new rendering features, and no fixes to the old line-based path except crashes or data loss. #87 and #128 wait. Old rendering bugs stay open as acceptance tests for the new core.
- **Spec first:** a change to how anything is displayed starts as a change to `SPEC.md` (written in stage 1) that the user approves, then code.
- **A bug is a test first:** record a rendering bug as a failing acceptance test, then fix it in the block type that causes it.
- **No special cases in shared code:** document, height map, frame and scrolling never branch on block type; what a block type needs goes into its interface.
- **Cache rule:** every derived value has one owner, its block, and is invalidated only by that block's text, the width or a layout setting. A new cache or global state outside that chain needs the user's approval, like a dependency.
- **Paint is read-only:** layout state changes only in prepaint.
- **Geometry only inside the block type:** caret, hit test, movement and selection inside a block are computed by that block alone.
- **Files stay under ~800 lines.** `ARCHITECTURE.md` holds rules and invariants only: no fix history, no measurement logs.
- **Stop rule:** if a fix would have to break one of these rules, stop and ask the user. Never build a workaround.

Stage 1 adds automatic checks for these rules where possible (#130 §10).

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
- Acceptance catalog of the rendering core: `cargo test -p jade-app acceptance -- --test-threads=1 --nocapture` runs every scenario under `crates/app/tests/acceptance/` (needs a Vulkan adapter: a GPU, or Mesa's software driver lavapipe, which CI uses; format in `debug_server/acceptance/`). A scenario that cannot open fails at every stage. `JADE_ACCEPTANCE=<id part>` picks scenarios; `JADE_UPDATE_INDEX=1` rewrites `INDEX.md`. Each stage's PR raises `CURRENT_STAGE`.
- **`jade-app`'s headless UI tests require `--test-threads=1`** — the parallel harness is non-deterministic under load (a confirmed concurrency bug in the vendored gpui/gpui-component test-support stack, not Jade's code).
- Lint: `cargo clippy --lib --bins -- -D warnings`. The `#[expect(clippy::too_many_lines)]` exemptions are for functions that were too long when CI began linting `jade-app`; don't add new ones.
- Format: `cargo fmt`
- **Debug harness** (drive/observe the live app: click, type, frame times, paint trace): `cargo build --release -p jade-app --features debug-socket && cargo build --release -p jade-debug-cli` (the socket is compiled out of default builds; cargo feature unification would otherwise leak `debug-socket` into a combined build), then `target/release/jade-debug help`; `launch --headless` renders without a window so `facts`/`expect`/`capture` can check how something looks. See `ARCHITECTURE.md` "Debug harness" and `.claude/skills/jade-debug`. Name every new interactive widget with `.debug_name(..)`.
- Benchmark: `cargo bench -p jade-core` (criterion; watch for a `change:`/`Performance has regressed` line, not just the absolute time). `cargo bench -p jade-app --bench editor` (~3 min) times the window-free editor internals on the stress fixture through `text_editor::bench_api`; `cargo bench -p jade-app --features bench-support --bench layout` times whole headless frames, and `--bench allocations` asserts a per-operation heap bound. Add new hot-path logic to these. Don't pass `--all-features` (benches should run without `debug-socket`).
