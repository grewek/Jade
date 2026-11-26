# SPEC.md

What Jade Markdown is and how Jade displays it. It is the reference for the rendering core (#130): a change to how anything is displayed starts here (`CLAUDE.md`, "Spec first").

- **Base:** [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) with GFM tables, task list items, strikethrough and extended autolinks.
- **This document** lists only Jade's extensions, its deviations from the base, and its display rules. Where it says nothing, the base applies.
- **Rule IDs** (`B-…`, `I-…`, `P-…`) are what acceptance tests cite.

## 1. Principles

| ID | Rule |
|---|---|
| P-1 | **The source text is the truth.** Only the Markdown file is stored. Everything shown is derived from it. Jade never writes to the file on its own, except where §7 says so (auto-closing while typing). |
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
| **list item** | ✓ (inline) | ✓ (sublist) | – | – | – | – | – | – |

Headings, thematic breaks and reference definitions are allowed wherever a paragraph is.

| ID | Rule |
|---|---|
| B-NEST-1 | **Disallowed content is text of the enclosing block.** Lines that would start a block marked "–" are read as paragraph text of that container, with inline content parsed as usual. Example: in a list item, an indented `| a | b |` is a line of the item's text, not a table. |
| B-NEST-2 | **List depth is at most 6.** A marker that would open a 7th level is text of the 6th-level item. |
| B-NEST-3 | **Blockquotes do not nest.** A line starting `> >` inside a quote is a quote line whose text starts with `>`. ⚑1 |

### 3.2 Paragraphs, headings, thematic breaks

As in CommonMark, including lazy continuation lines and setext headings (`===`/`---` under a paragraph). ⚑2

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
| B-CALL-3 | **Not nested:** a `> > [!type]` line inside a callout is text (B-NEST-3). |
| B-CALL-4 | **Folding is view state** (§9): `-` means initially folded, `+` or nothing initially open. Folding or unfolding in Jade never changes the file. A folded callout shows only its header. |
| B-CALL-5 | **Blocks inside a callout keep their `>` prefix on every line**, fences and `$$` included. The callout ends at the first line that does not continue it, and an open code or formula block ends there too (CommonMark, P-2). |

### 3.6 Code blocks

Fenced with ```` ``` ```` or `~~~` (at least 3), as in CommonMark: an info string after the opening fence, closed by a fence of the same character that is at least as long. Indented code blocks (4 spaces) as in CommonMark. ⚑3

| ID | Rule |
|---|---|
| B-CODE-1 | **An open fence ends at the end of its container** (CommonMark), and a container's closing tag always ends it (B-TAG-4). |
| B-CODE-2 | An open fence that reaches the end of its container is shown as code up to there, with the open-construct hint (§7). |

### 3.7 Formula blocks

```markdown
$$
\int_0^1 x^2 \, dx
$$
```

| ID | Rule |
|---|---|
| B-MATH-1 | **Opener:** `$$` at the start of a line (after indentation, and after a container's `>` prefix inside a callout). ⚑4 |
| B-MATH-2 | **Closer:** the first unescaped `$$` in the next 256 lines. `$$…$$` on one line is a one-line formula block when it is the whole line, inline display math otherwise (§5). |
| B-MATH-3 | **An opener without a closer is plain text.** A blank line, a code fence, a container tag line (`<jade-…>` or `</jade-…>`) or the 256-line limit, reached before a closer, makes the opener text, so typing `$$` never turns the rest of a note into a formula. **A blank line inside a long formula therefore breaks it.** |

### 3.8 Tables

GFM tables: a header row, a delimiter row (`---`, `:--`, `--:`, `:-:`), body rows.

| ID | Rule |
|---|---|
| B-TAB-1 | The table has as many columns as its widest row. Rows with fewer cells are padded with empty cells. (GFM drops extra cells; Jade shows them, P-4.) ⚑11 |
| B-TAB-2 | Cells hold inline content. `\|` is a literal pipe. **`<br>` is a line break inside a cell, and only there** (I-HTML-2). |
| B-TAB-3 | A table never shows its source as a whole. With the caret in a cell, only that cell's text is raw (§6). |

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
| B-IMG-4 | An embed that is not alone on its line, is remote, or is not an image is inline content (§5): an image link shown as text. ⚑5 |

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
| B-COL-1 | `<jade-columns>` holds **2 to 4** `<jade-column>` children and nothing else. Lines between children other than blank lines are text of the preceding column. ⚑6 |
| B-COL-2 | `ratio="a/b[/c[/d]]"`: whole percentages, one per column, summing to 100. Missing or invalid: equal widths. |
| B-COL-3 | Columns are laid out side by side at equal top. The region is as tall as its tallest column. |
| B-COL-4 | **Columns only at top level** (§3.1). A `<jade-columns>` or `<jade-aside>` line inside a column is text. |
| B-COL-5 | `<jade-column>` is valid only as a direct child of `<jade-columns>`, and `<jade-float>` only as the first child of `<jade-aside>`. Anywhere else these lines are text. |

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
| B-ASIDE-2 | **The floated content is the `<jade-float>` child**, which must come first. Everything after it is the flowing text. ⚑7 |
| B-ASIDE-3 | `side="left"` or `"right"` (default `right`), `width="10%"…"90%"` (default `40%`), the float's share of the line. |
| B-ASIDE-4 | **One float per aside.** Several floats are several asides, one after another. ⚑8 |
| B-ASIDE-5 | Asides only at top level, like columns (B-COL-4). |

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
| B-TAG-5 | **An opening tag line inside a code block is code.** So is a closing tag line when no container of its name is open. ⚑9 |

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
| I-ENT-1 | Numeric entity references decode to any valid scalar value. Named references decode for a fixed common subset; any other `&name;` is text. ⚑10 |

## 6. Live Preview

| ID | Construct | Raw (source shown) when … |
|---|---|---|
| P-LP-1 | inline element (emphasis, link, wikilink, code span, inline formula, escape, entity) | **the caret touches it**: before its first character, inside, or after its last. Arrow keys step through its characters one by one. |
| P-LP-2 | heading, list, task and quote markers | the caret touches the marker |
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
| P-OPEN-1 | **Auto-closing while typing:** typing an opening code fence, `$$` at a block opener position, or a valid opening tag line inserts the matching closer below, **only if the construct would otherwise stay open**. A fence typed while a fence above is open is a closer, and nothing is inserted. |
| P-OPEN-2 | **Every other way** an open construct can arise (paste, delete, undo, external change) is shown as it is. The opening line carries a subtle "not closed" hint, and one click on it inserts a closer at the end of the construct's container. |
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
| E-1 | **Arrow keys move in reading order**, also across columns (column 1, then column 2, then below the region). |
| E-2 | **Ctrl+↓ / Ctrl+↑** leave a columns region or aside spatially, at the same x. **Alt+← / Alt+→** move to the neighbouring column at the same height. Ctrl+← / Ctrl+→ stay word jumps. |
| E-3 | **Selection is in source order.** Copy copies the selected source. |
| E-4 | **A click on a rendered link opens it. Ctrl+click** places the caret inside it. |
| E-5 | **Enter in a table cell** inserts `<br>`. A `<br>` or `\|` is one caret step and is deleted whole. |

## 11. Limits

| What | Limit |
|---|---|
| List depth | 6 (B-NEST-2) |
| Columns per region | 2–4 (B-COL-1) |
| Aside width | 10–90 %, default 40 % (B-ASIDE-3) |
| Formula block lookahead | 256 lines (B-MATH-2) |
| Wikilinks, inline math | one line each |

## 12. Not supported (yet)

Shown as text unless the base spec gives them another meaning: footnotes, `==highlight==`, `%%comments%%`, note embeds `![[note]]`, block references `^id`, CommonMark HTML blocks (I-HTML-1). Each would need an entry here and, where it reaches across blocks, in §8.

## ⚑ To confirm

Points where this spec decides something that was not decided in #130. Each needs the maintainer's yes before stage 2.

1. **B-NEST-3:** nested blockquotes (`> >`) are text. The nesting table did not list a quote inside a quote; CommonMark allows it.
2. **§3.2:** setext headings are supported, as in CommonMark (needed for its examples). They mean a `---` under a paragraph turns it into a heading.
3. **§3.6:** indented code blocks are supported, as in CommonMark. In list items they are text (B-NEST-1).
4. **B-MATH-1:** a formula block may open after a callout's `>` prefix. Today's scanner ignores openers behind `>`; the nesting table allows formula blocks in callouts.
5. **B-IMG-4:** an inline image (not alone on its line) is shown as text, not drawn inline.
6. **B-COL-1:** stray lines between `<jade-column>` children belong to the preceding column.
7. **B-ASIDE-2:** an explicit `<jade-float>` child holds the floated content, instead of "the first block floats". It is unambiguous, lets a float hold several blocks (a table plus a caption), and matches the strict-form tags.
8. **B-ASIDE-4:** one float per aside. Today's `:::` asides allow up to 3 floats and `::: clear`; with self-contained asides, stacking asides replaces both.
9. **B-TAG-5:** an opening tag inside a fenced code block is code. Only closing tags take precedence (B-TAG-4), so a code example of a tag is possible.
10. **I-ENT-1:** named entities stay a fixed common subset, not all of HTML5's.
11. **B-TAB-1:** a row with more cells than the header widens the table instead of losing its extra cells (GFM drops them). Follows from P-4.
