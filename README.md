# Jade

Jade is an Obsidian-like note-taking app written in Rust. Notes are plain Markdown files in a vault on disk, with wikilinks and backlinks between them.

- **Layouts:** flexible arrangement of your notes and panels.
- **Performance first:** built natively on [GPUI](https://www.gpui.rs/), with no web technology, and measured against a 60fps frame budget.

Jade is in early development.

## Building

Jade needs a nightly Rust toolchain, which `rust-toolchain.toml` selects automatically.

```sh
cargo run -p jade-app --release
```
