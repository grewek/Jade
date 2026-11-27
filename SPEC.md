# SPEC.md

What Jade Markdown is and how Jade displays it. It is the reference for the rendering core (#130): a change to how anything is displayed starts here (`CLAUDE.md`, "Spec first").

- **Base:** [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) with GFM tables, task list items, strikethrough and extended autolinks.
- **This document** lists only Jade's extensions, its deviations from the base, and its display rules. Where it says nothing, the base applies.
- **Rule IDs** (`B-…`, `I-…`, `P-…`) are what acceptance tests cite.

## 1. Principles

| ID | Rule |
|---|---|
| P-1 | **The source text is the truth.** Only the Markdown file is stored. Everything shown is derived from it. Jade never writes to the file on its own, except where §7 says so (inserting a closer on Enter). Moving the caret never writes (§10.1). |
| P-2 | **Errors stay local.** An incomplete construct never reaches past the end of its container (§4.3). At top level, the container is the document. |
| P-3 | **Stability.** Switching a construct between raw and rendered changes only the height of the block that holds the caret. Nothing above it moves. |
| P-4 | **Nothing is lost.** Content Jade does not support at a place is shown as text (§3.1), never hidden or dropped. |

## 2. Structure

A document is a sequence of **blocks**. **Container blocks** hold other blocks; **leaf blocks** hold inline content or none.

| Container | Leaf |
|---|---|
| list, list item, blockquote, callout, columns, column, aside, float | paragraph, heading, thematic break, code block, formula block, table, image block, reference definition |

## 3. Blocks

### 3.1 Nesting

| Container ↓ may contain → | paragraph | list | table | code | formula block | callout | image | columns / aside |
|---|---|---|---|---|---|---|---|---|
| **document** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| **column, float, aside text** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |
| **callout** | ✓ | ✓ | ✓ | ✓ | ✓ | – | ✓ | – |
| **blockquote** | ✓ | ✓ | – | – | – | – | – | – |

Blockquotes may also contain blockquotes, everywhere a blockquote is allowed, and callouts may contain blockquotes (B-NEST-3).
| **list item** | ✓ (inline) | ✓ (sublist) | – | – | – | – | – | – |

Headings, thematic breaks and reference definitions are allowed wherever a paragraph is, **except in list items**, where their lines are item text.

| ID | Rule |
|---|---|
| B-NEST-1 | **Disallowed content is text of the enclosing block.** Lines that would start a block marked "–" are read as paragraph text of that container, with inline content parsed as usual. Example: in a list item, an indented `| a | b |` is a line of the item's text, not a table. |
| B-NEST-2 | **List depth is at most 6.** A marker that would open a 7th level is text of the 6th-level item. |
| B-NEST-3 | **Blockquotes nest up to depth 6** (`> > …`), as in CommonMark, including lazy continuation (`> > a` followed by `> b` continues the inner paragraph). A 7th `>` is text. A nested quote holds paragraphs, lists and quotes only: a `[!type]` line inside it is text, so a callout is never inside a quote (B-CALL-3). |

### 3.2 Paragraphs, headings, thematic breaks

As in CommonMark, including lazy continuation lines and setext headings (`===` or `---` under a paragraph).

| ID | Rule |
|---|---|
| B-HEAD-1 | **A setext underline holding the caret shows the block raw**: the paragraph keeps its normal style and the underline its characters. Otherwise typing `-` under a paragraph (the first character of a list item) would flash the paragraph into a heading until the space is typed. |
| B-HEAD-2 | A `---` line directly under a paragraph is a setext underline, not a thematic break (CommonMark). A blank line between them makes it a thematic break. |

### 3.3 Lists

As in CommonMark (`-`, `*`, `+`; `1.`, `1)`), with GFM task items `- [ ]` / `- [x]`.

| ID | Rule |
|---|---|
| B-LIST-1 | A list item contains a paragraph and sublists only (B-NEST-1). |
| B-LIST-2 | **Ordered lists number from their first item's number**, and each next item counts on, whatever number its source shows (CommonMark). The numbering never depends on anything outside the list. |

### 3.4 Blockquotes

As in CommonMark, restricted by §3.1. A quote whose first line is a callout header (§3.5) is a callout instead.

### 3.5 Callouts

```markdown
> [!warning]- Optional title
> Body text, a list, a table, code, a formula or an image.
```

| ID | Rule |
|---|---|
| B-CALL-1 | **Header:** a blockquote's first line is `[!type]`, optionally followed by `+` or `-` and a title. Type names are case-insensitive. Obsidian's types and aliases are known; an unknown type is shown as `note`. Without a title, the type's name is shown. |
| B-CALL-2 | **Body:** every following line that continues the quote (`>` prefix, or a lazy continuation line of a paragraph). Contents per §3.1. |
| B-CALL-3 | **Not nested:** a callout header is only recognized at quote depth 1 of a place that allows callouts (§3.1). A `> > [!type]` line inside a callout is a nested quote whose text starts with `[!type]`. |
| B-CALL-4 | **Folding is view state** (§9): `-` means initially folded, `+` or nothing initially open. Folding or unfolding in Jade never changes the file. A folded callout shows only its header. |
| B-CALL-5 | **Blocks inside a callout keep their `>` prefix on every line**, fences and `$$` included. The callout ends at the first line that does not continue it, and an open code or formula block ends there too (CommonMark, P-2). |

### 3.6 Code blocks

Fenced with ```` ``` ```` or `~~~` (at least 3), as in CommonMark: an info string after the opening fence, closed by a fence of the same character that is at least as long. Indented code blocks (4 spaces) as in CommonMark.

| ID | Rule |
|---|---|
| B-CODE-1 | **Indentation of 4 or more spaces decides first:** after a blank line, such a line at top level or in a column, float, aside text or callout is indented code. That includes lines that would otherwise start a table, a formula block, a list or a tag. Container tags are only recognized in column 0 (B-TAG-1), so content inside a container never needs indenting and never turns into code by accident. |
| B-CODE-2 | An indented code line cannot interrupt a paragraph (CommonMark): without a blank line before it, it continues the paragraph. In a list item, deeper indentation is item text (B-NEST-1). |

| ID | Rule |
|---|---|
| B-CODE-3 | **An open fence ends at the end of its container** (CommonMark), and a container's closing tag always ends it (B-TAG-4). |
| B-CODE-4 | An open fence that reaches the end of its container is shown as code up to there, with the open-construct hint (§7). |

### 3.7 Formula blocks

```markdown
$$
\int_0^1 x^2 \, dx
$$
```

| ID | Rule |
|---|---|
| B-MATH-1 | **Opener:** `$$` at the start of a line, after at most 3 spaces of indentation and after a callout's `>` prefix. 4 or more spaces make it indented code (B-CODE-1). |
| B-MATH-2 | **Closer:** the first unescaped `$$` in the next 256 lines. `$$…$$` on one line is a one-line formula block when it is the whole line, inline display math otherwise (§5). |
| B-MATH-3 | **An opener without a closer is plain text.** A blank line, a code fence, a container tag line (`<jade-…>` or `</jade-…>`) or the 256-line limit, reached before a closer, makes the opener text, so typing `$$` never turns the rest of a note into a formula. **A blank line inside a long formula therefore breaks it.** |

### 3.8 Tables

GFM tables: a header row, a delimiter row (`---`, `:--`, `--:`, `:-:`), body rows.

| ID | Rule |
|---|---|
| B-TAB-1 | The table has as many columns as its widest row. Rows with fewer cells are padded with empty cells. (GFM drops extra cells; Jade shows them, P-4.) |
| B-TAB-2 | Cells hold inline content. `\|` is a literal pipe. **`<br>` is a line break inside a cell, and only there** (I-HTML-2). |
| B-TAB-3 | A table never shows its source as a whole. With the caret in a cell, only that cell's text is raw (§6). |
| B-TAB-4 | **Column widths follow the drawn content**: rendered text, drawn formulas and `<br>` breaks, never character counts. A column is as wide as its widest drawn cell, then shrunk to fit the available width. |
| B-TAB-5 | **A shrunk column wraps at word boundaries.** A word is cut only when it alone is wider than the column. A drawn formula is never split. |
| B-TAB-6 | **A formula wider than its column is shown as its source**, wrapped like text, so nothing is clipped (P-4). |
| B-TAB-7 | **The cell being edited is left-aligned**, whatever the column's alignment, so the caret does not shift while typing. |

### 3.9 Image blocks

```markdown
![[diagram.png|300]]
![Alt text|300](images/diagram.png)
```

| ID | Rule |
|---|---|
| B-IMG-1 | **An image block is a line that holds nothing but one embed** of a vault-local file with an image extension (`avif`, `bmp`, `gif`, `jpeg`, `jpg`, `png`, `svg`, `webp`). |
| B-IMG-2 | **Width:** `|300` or `|300x200` after the target or alt text requests 300 px; the height is always derived from the image's proportions. The image is shrunk to the available width. |
| B-IMG-3 | **Size before pixels:** the block's height comes from the file header, read when the note opens. Decoding waits until the image becomes visible. |
| B-IMG-4 | An embed that is not alone on its line, is remote, or is not an image is inline content (§5): an image link shown as text. |

### 3.10 Reference definitions

`[label]: destination "title"` as in CommonMark: labels are case-insensitive and whitespace-collapsed, **the first definition of a label wins**, and a definition may stand anywhere in the note. In Live Preview a definition line is shown dimmed; it is never hidden (P-4).

### 3.11 Columns

```markdown
<jade-columns ratio="60/40">

<jade-column>

Left column.

</jade-column>
<jade-column>

Right column.

</jade-column>

</jade-columns>
```

| ID | Rule |
|---|---|
| B-COL-1 | `<jade-columns>` holds **2 to 4** `<jade-column>` children and nothing else. Lines between children other than blank lines are text of the preceding column. |
| B-COL-2 | `ratio="a/b[/c[/d]]"`: whole percentages, one per column, summing to 100. Missing or invalid: equal widths. |
| B-COL-3 | Columns are laid out side by side at equal top. The region is as tall as its tallest column. |
| B-COL-4 | **Columns only at top level** (§3.1). A `<jade-columns>` or `<jade-aside>` line inside a column is text. |
| B-COL-5 | `<jade-column>` is valid only as a direct child of `<jade-columns>`, and `<jade-float>` only as the first child of `<jade-aside>`. Anywhere else these lines are text. |
| B-COL-6 | **Every column has at least one caret position**, also when empty: its first blank line, else a virtual empty line (V-1). |

### 3.12 Asides (floats)

```markdown
<jade-aside side="left" width="40%">

<jade-float>

| Table | floated left |
|---|---|
| a | b |

</jade-float>

Text that flows beside the table and continues below it at full width.

</jade-aside>
```

| ID | Rule |
|---|---|
| B-ASIDE-1 | **An aside is self-contained** (K16): its float and the text flowing around it are inside it. Content after `</jade-aside>` starts below the whole aside at full width. |
| B-ASIDE-2 | **The floated content is the `<jade-float>` child**, which must come first. Everything after it is the flowing text. |
| B-ASIDE-3 | `side="left"` or `"right"` (default `right`), `width="10%"…"90%"` (default `40%`), the float's share of the line. |
| B-ASIDE-4 | **One float per aside.** Several floats are several asides, one after another. |
| B-ASIDE-5 | Asides only at top level, like columns (B-COL-4). |
| B-ASIDE-6 | **The float and the flowing text each have at least one caret position**, also when empty (B-COL-6). |

## 4. Container tags

### 4.1 Strict form

| ID | Rule |
|---|---|
| B-TAG-1 | **A tag line is the whole line**, starting in column 0: `<jade-NAME ATTRS>` or `</jade-NAME>`, nothing else on the line. Names: `columns`, `column`, `aside`, `float`. |
| B-TAG-2 | **Attributes:** `name="value"`, separated by single spaces, double quotes only, no escapes. |
| B-TAG-3 | **A line that breaks B-TAG-1 or B-TAG-2 is text** (shown as written, with the open-construct hint). An unknown attribute is kept in the file and ignored. |

### 4.2 Precedence

| ID | Rule |
|---|---|
| B-TAG-4 | **While a container is open, a closing tag line of its name always closes it**, even inside an open code block or formula, which end there (P-2). |
| B-TAG-5 | **An opening tag line inside a code block is code.** So is a closing tag line when no container of its name is open. |

### 4.3 Open and stray tags

| ID | Rule |
|---|---|
| B-TAG-6 | **A container without its closing tag is not a container.** Its opening tag line is text with the open-construct hint, and the following lines are read as if it were absent. |
| B-TAG-7 | A closing tag without an open container of that name is text. |
| B-TAG-8 | A blank line after an opening tag and before a closing tag is recommended so other readers show the content as Markdown. Jade does not require it. |

### 4.4 Display

| ID | Rule |
|---|---|
| B-TAG-9 | **Tag lines are hidden** and take no space. The caret skips them. |
| B-TAG-10 | **"Show structure"** (hotkey and right-click menu) shows every tag line as an editable line. It is a view setting, not stored in the file. |
| B-TAG-11 | While a tag line is being typed it is visible, because the caret is on it. When the caret leaves a valid tag line, the line is hidden and the container forms. |
| B-TAG-12 | **Deleting next to a hidden tag line:** Backspace at the start of the line after a closing tag, or Delete at the end of the line before an opening tag, deletes that line if it is empty and does nothing otherwise, so text never joins a tag line. With "Show structure" on, tag lines are ordinary lines. |

## 5. Inline content

As in CommonMark and GFM: emphasis and strong emphasis (`*`, `_`), strikethrough (`~~`), code spans, links (inline, reference, autolinks, extended autolinks), backslash escapes, entity references, hard line breaks (`\` or two spaces at the end of a line).

| ID | Rule |
|---|---|
| I-WIKI-1 | **Wikilinks:** `[[target]]`, `[[target#heading]]`, `[[target|alias]]`, `[[target#heading|alias]]`, on one line. A wikilink inside a code span or inline formula is text. |
| I-WIKI-2 | A wikilink whose target does not exist is shown in the error colour. That depends only on the vault and never changes a height (§8). |
| I-MATH-1 | **Inline math** `$…$`: the opening `$` is followed by a non-space, the closing `$` is preceded by a non-space and not followed by a digit, `\$` escapes, and the first `$` after the opener decides (if it cannot close, the opener is text). One line only. A code span wins over a `$` inside it. `$$…$$` within text is inline display math. |
| I-REF-1 | **Reference links** `[text][label]`, `[label][]` and `[label]` resolve against the note's definitions (§3.10). An unresolved one is text. |
| I-HTML-1 | **Raw HTML is text**, inline and as whole lines, and never swallows the Markdown after it (no CommonMark HTML blocks). HTML comments `<!-- … -->` are visible text. |
| I-HTML-2 | **`<br>` is a line break in a table cell only.** Anywhere else it is text. |
| I-HTML-3 | Autolinks `<https://…>`, `<mailto:…>` and `<user@host>` are links, not HTML. |
| I-ENT-1 | Numeric entity references decode to any valid scalar value. Named references decode for a fixed common subset; any other `&name;` is text. |

## 6. Live Preview

| ID | Construct | Raw (source shown) when … |
|---|---|---|
| P-LP-1 | inline element (emphasis, link, wikilink, code span, inline formula, escape, entity) | **the caret touches it**: before its first character, inside, or after its last. Arrow keys step through its characters one by one. |
| P-LP-2 | heading, list, task and quote markers | the caret touches the marker; a setext heading while its underline holds the caret (B-HEAD-1) |
| P-LP-3 | callout header | the caret is on the header line |
| P-LP-4 | table | never as a whole. Only the **cell** holding the caret shows its text raw |
| P-LP-5 | formula block | the caret is in the block. Then the **whole block** is raw |
| P-LP-6 | code block | content is always shown as code. Fence lines are visible while the caret is in the block |
| P-LP-7 | image block | the caret is on the embed line |
| P-LP-8 | container tags | only with "Show structure" (B-TAG-10) |
| P-LP-9 | reference definition | always shown as dimmed source (§3.10) |

**P-LP-10:** Raw and rendered differ in width and so can wrap differently. Only the block holding the caret may change height (P-3).

## 7. Open constructs

| ID | Rule |
|---|---|
| P-OPEN-1 | **A pending opener opens nothing yet.** An opening code fence line, a `$$` line at a block opener position, or a valid opening tag line that holds the caret and has no closer below it in its container is shown raw, and nothing below it changes. A fence typed while a fence above is open is a closer, not an opener. |
| P-OPEN-1a | **Enter on a pending opener inserts its closer:** a fence of the same character and length, `$$`, or the matching closing tag, on a new line below, and puts the caret on an empty line between them. This is the only time Jade inserts text the user did not type (P-1). |
| P-OPEN-1b | **Leaving a pending opener without Enter** (arrows, click) makes it an ordinary open construct (P-OPEN-2). |
| P-OPEN-2 | **Every other way** an open construct can arise (leaving a pending opener, paste, delete, undo, external change) is shown as it is. The opening line carries a subtle "not closed" hint, and one click on it inserts a closer at the end of the construct's container. |
| P-OPEN-3 | The hint also marks malformed tag lines (B-TAG-3) and open tags (B-TAG-6). |
| P-OPEN-4 | **Accepted cost (K33):** a fence that nothing closes changes the block tree to the end of its container. On a 5 MB note this tree update may take ≈ 7 ms. The whole frame must still meet the frame budget. |

## 8. Cross-block dependencies

**This list is exhaustive.** A block's display depends only on its own source, the width, the layout settings, and the entries below. A new dependency needs a change to this table.

| ID | Dependency | When it changes, … |
|---|---|---|
| D-1 | **Formula macros** (`\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\DeclareMathOperator`), defined in any formula of the note, **note-wide, the last definition wins** | … every formula block and every block with inline math is remeasured. |
| D-2 | **Reference definitions** (§3.10) | … every block with a reference link is reparsed inline. |
| D-3 | **Wikilink target exists** (vault) | … only the colour changes: repaint, no remeasure. |
| D-4 | **Ordered list numbering** | … nothing outside the list changes (the list is one block). |

## 9. View state

| ID | Rule |
|---|---|
| P-VIEW-1 | View state (callout folding, the table column being dragged, "Show structure") is **not stored in the note**. Per-block view state is tied to the block's source position and carried along by edits. |
| P-VIEW-2 | When a note is opened, a callout's fold marker (B-CALL-4) gives its initial state. |

## 10. Editing behaviour fixed by this spec

| ID | Rule |
|---|---|
| E-1 | **Arrow keys move in reading order**, also across columns (column 1, then column 2, then below the region) and onto virtual empty lines (§10.1). |
| E-2 | **Ctrl+↓ / Ctrl+↑** leave a columns region or aside spatially, at the same x. **Alt+← / Alt+→** move to the neighbouring column at the same height. Ctrl+← / Ctrl+→ stay word jumps. |
| E-3 | **Selection is in source order.** Copy copies the selected source. |
| E-4 | **A click on a rendered link opens it. Ctrl+click** places the caret inside it. |
| E-5 | **Enter in a table cell** inserts `<br>`. A `<br>` or `\|` is one caret step and is deleted whole. |

### 10.1 Virtual empty lines

| ID | Rule |
|---|---|
| V-1 | **Where the caret must be able to go but no source line exists, the editor shows a virtual empty line**, one body line tall, that is not in the file: after the last block of a note and before its first, when that block is not a paragraph or heading and no blank line is there; and inside a column, float or aside text with no line between its tags. |
| V-2 | **It is always shown**, not only while the caret is on it, so nothing moves when the caret arrives (P-3). |
| V-3 | Arrows, Ctrl+↓ / Ctrl+↑ (E-2) and clicks reach it like any line. |
| V-4 | **Typing or pasting on it creates the line in the file** (with the line break it needs). **Moving onto it or away never writes.** |
| V-5 | Virtual lines belong to the transitions between blocks, not to a block type (`ARCHITECTURE.md`, geometry). |

### 10.2 Layout commands

Offered in the right-click menu, by where the caret is. They are the user's edits: they write the tags below in strict form, with blank lines as B-TAG-8 recommends.

| ID | Caret is … | Command | Writes |
|---|---|---|---|
| L-1 | on a blank line at top level | **Insert columns** | `<jade-columns>` with two empty `<jade-column>`s |
| L-2 | same | **Insert float left / right** | an `<jade-aside side="…">` with an empty `<jade-float>` and empty flowing text |
| L-3 | in a top-level block of plain text (not a heading, code block, formula block, table or callout) | **Make columns** | the block becomes the first of two columns; the second is empty |
| L-4 | same | **Float left / right** | the block becomes the `<jade-float>` of a new aside; up to three following blocks of plain text become its flowing text |
| L-5 | in a columns region | **Add column** (2–3 columns only) | an empty `<jade-column>` at the end; `ratio` is removed, so widths become equal |
| L-6 | same | **Unwrap columns** | the tags are removed; the columns' content follows one after another, in order |
| L-7 | in an aside (float or flowing text) | **Flip side** | `side` changes to the other side |
| L-8 | same | **Unwrap float** | the tags are removed; the float's content comes first, then the flowing text |
| L-9 | in an aside's float | **Move float up / down** | the `<jade-float>` moves past the paragraph before (up) or after (down) it, so it sits beside different text; not offered when there is no plain paragraph that way |
| L-10 | dragging a divider between columns | **Resize** | `ratio`, written **once on release**; while dragging, the columns reflow as a preview. Each column keeps at least 10 %. |
| L-11 | dragging the inner edge of a float | **Resize** | `width`, written once on release, within 10–90 % |

| ID | Rule for every layout command |
|---|---|
| L-12 | **One undo step.** |
| L-13 | **The caret stays with its text**: it ends at the same character of the same content, now inside or outside the container. |
| L-14 | **Undo restores the caret** to where it was before the command. |
| L-15 | Not offered where the nesting table (§3.1) forbids the result. |

## 11. Limits

| What | Limit |
|---|---|
| List depth | 6 (B-NEST-2) |
| Blockquote depth | 6 (B-NEST-3) |
| Columns per region | 2–4 (B-COL-1) |
| Aside width | 10–90 %, default 40 % (B-ASIDE-3) |
| Formula block lookahead | 256 lines (B-MATH-2) |
| Wikilinks, inline math | one line each |

## 12. Not supported (yet)

Shown as text unless the base spec gives them another meaning: footnotes, `==highlight==`, `%%comments%%`, note embeds `![[note]]`, block references `^id`, CommonMark HTML blocks (I-HTML-1). Each would need an entry here and, where it reaches across blocks, in §8.
