# Changelog

Notable changes per release, newest first. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/). Updated only when a release is prepared.

## [0.201.0] - 2026-11-25

Versions follow `0.<number>.<patch>` from here on.

### Added
- LaTeX math: `$…$` and `$$…$$` formulas are drawn from the Latin Modern Math font, inline and as centred blocks, in table cells, in columns and asides (#13, #39). Commands include matrices, `cases`, `aligned`, accents, stacks and arrows, sized delimiters, `\cancel`, `\color`, `\tag`, `\genfrac`, `\above` and the size commands; note-local macros (`\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\DeclareMathOperator`) work (#14, #18). Rows grow to fit tall formulas (#15).
- Style for Markdown elements, set in `config.toml` and applied live (#58): font size and line height, heading size, weight, colour and spacing per level, a box behind fenced code with a language label, a chip behind inline code, callout and table radii, image radius, link underline, blockquote bar, list and task styles, done-task and strikethrough colours, and `[colors]` overrides for any theme role. Text size, heading size and corner radius are also in Settings. Every key is documented in `CONFIG.md`, and `config.sample.toml` lists them at their defaults.
- Syntax highlighting for fenced code, off by default (`code_block.highlight`, #99). Jade reads `.sublime-syntax` definitions from a `syntaxes/` folder next to `config.toml` (none are bundled; see `CONFIG.md`), loads them on demand and colours blocks in the background with ten theme roles, so typing and scrolling stay smooth. Edits keep the old colours until the new ones arrive, and over-long blocks and lines stay plain.
- The vault tree, search results and backlinks scroll when they are taller than their sidebar, with a scrollbar; the tree follows the open note (#98).
- Debug harness: `capture`, `facts` and `expect` read an element's pixels over the debug socket (#42).

### Changed
- Headings can be sized, weighted and ruled per level in `[heading]`; the defaults are unchanged (body size, bold, no rule) (#58).
- Row heights are measured in one place and corrected as they scroll into view, so the scrollbar and content stop jumping in huge notes (#20, #24, #26).

### Fixed
- A column or float that was empty could not be clicked into (0.2.2).
- Ticking a task in a note that was not focused scrolled the editor to the top (#89).
- Tables: a header as tall as its cells, words kept whole in narrow columns, and columns sized by the drawn width of formulas (#46, #47, #48); the grid stays aligned below paragraphs with tall formulas (#21).
- Math: a formula wider than its line or column shows its source instead of a blank (#43, #49), nested fractions no longer overlap their bars (#50), hidden macro definitions leave no gap (#51), and only rows holding a tall formula get the tall height (#32). Tall formulas in asides and columns no longer overlap the next row (#23).
- The caret's line in a long wrapped heading was a row off until the next check (#74).

## [0.2.2] - 2026-11-22

### Fixed
- A new empty column or float gets a blank line so a click reaches it.

## [0.2.1] - 2026-11-22

### Added
- GitHub Actions builds and publishes the release binary.

## [0.2.0] - 2026-11-22

### Added
- First start: Jade asks where to store your notes and remembers the folder in `config.toml` (#4). The config directory follows the OS convention (`$XDG_CONFIG_HOME/jade` on Linux).
- GitHub Actions CI: format, clippy, tests and docs for the crates that build without GPUI's system libraries.
- README with a demo video; GitFlow and issue workflow rules in `CLAUDE.md`.

### Changed
- Starting without a vault argument no longer falls back to `./vault`; the first-start dialog opens instead until a folder is saved.

## [0.1.0]

First tagged version: Markdown vault, wikilinks and backlinks, live-preview editor, themes, images, tables and floats.
