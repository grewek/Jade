# Changelog

Notable changes per release, newest first. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/). Updated only when a release is prepared.

## [0.2.0] - 2026-11-22

### Added
- First start: Jade asks where to store your notes and remembers the folder in `config.toml` (#4). The config directory follows the OS convention (`$XDG_CONFIG_HOME/jade` on Linux).
- GitHub Actions CI: format, clippy, tests and docs for the crates that build without GPUI's system libraries.
- README with a demo video; GitFlow and issue workflow rules in `CLAUDE.md`.

### Changed
- Starting without a vault argument no longer falls back to `./vault`; the first-start dialog opens instead until a folder is saved.

## [0.1.0]

First tagged version: Markdown vault, wikilinks and backlinks, live-preview editor, themes, images, tables and floats.
