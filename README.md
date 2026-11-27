# Jade

Jade is an Obsidian-like note-taking app written in Rust. Notes are plain Markdown files in a vault on disk, with wikilinks and backlinks between them.

- **Layouts:** flexible arrangement of your notes and panels.
- **Performance first:** built natively on [GPUI](https://www.gpui.rs/), with no web technology, and measured against a 60fps frame budget.

https://github.com/user-attachments/assets/307b841b-6840-4fad-a0ac-c7da5e733fd8

Jade is in early development.

## Disclaimer

Jade is also a learning project. It is developed entirely with AI: I take the active role of the software engineer (designing, deciding, reviewing and verifying), while the AI writes the code.

## Building

Jade builds with stable Rust. `rust-toolchain.toml` pins the version, and rustup selects it automatically. Raising the version is its own pull request, with CI green.

```sh
cargo run -p jade-app --release
```
