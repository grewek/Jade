#!/usr/bin/env python3
"""Generates a large (>10MB), deterministic Markdown stress-test fixture
exercising every construct Jade's Live Preview renders, for future
criterion benchmarks of crates/app/src/text_editor's internals.
"""
import random

random.seed(1337)

TARGET_BYTES = 11 * 1024 * 1024  # 11 MiB, comfortably over the 10MB floor

WORDS = (
    "vault note link graph editor render cursor buffer markdown syntax "
    "heading paragraph quote list table cell column row width height "
    "style color theme focus select scroll wrap glyph font parse scan "
    "token offset byte line block inline span segment display raw hidden "
    "callout warning info tip success danger bug example quote summary "
    "note todo question failure fold marker bullet checkbox task item "
    "depth nest indent lazy continuation compound construct active plain "
    "search backlink folder rename delete create open close save flush "
    "async window frame paint canvas mouse click drag select caret"
).split()

WIKITARGETS = [
    "Note A", "Note B", "Math-Stuff", "Project Plan", "Daily Log",
    "Architecture Overview", "Meeting Notes", "Research", "Ideas", "Todo List",
]

CALLOUT_TYPES = [
    "note", "abstract", "summary", "tldr", "info", "todo", "tip", "hint",
    "important", "success", "check", "done", "question", "help", "faq",
    "warning", "caution", "attention", "failure", "fail", "missing",
    "danger", "error", "bug", "example", "quote", "cite",
]

rng = random.Random(1337)


def word(n=1):
    return " ".join(rng.choice(WORDS) for _ in range(n))


def sentence(min_words=6, max_words=16):
    return word(rng.randint(min_words, max_words)).capitalize() + "."


def inline_salad(n_sentences=3):
    """A paragraph of plain sentences with inline constructs woven in:
    bold, italic, inline code, strikethrough, wikilinks (plain/alias/
    heading-anchor), inline links, bracketed/bare autolinks, HTML
    entities, and backslash escapes."""
    parts = []
    for i in range(n_sentences):
        parts.append(sentence())
        pick = i % 10
        if pick == 0:
            parts.append(f"This is **{word(2)}** in bold.")
        elif pick == 1:
            parts.append(f"This is *{word(2)}* in italic, and __{word(2)}__ also bold.")
        elif pick == 2:
            parts.append(f"Run `{word(3)}` inline.")
        elif pick == 3:
            parts.append(f"This got ~~{word(2)}~~ struck through.")
        elif pick == 4:
            target = rng.choice(WIKITARGETS)
            parts.append(f"See [[{target}]] for more.")
        elif pick == 5:
            target = rng.choice(WIKITARGETS)
            parts.append(f"See [[{target}|{word(2)}]] (aliased).")
        elif pick == 6:
            target = rng.choice(WIKITARGETS)
            parts.append(f"See [[{target}#Section]] (heading anchor).")
        elif pick == 7:
            parts.append(f"An inline [link to GitHub]({'https://github.com/example/' + word(1)}) here.")
        elif pick == 8:
            parts.append(f"A bracketed autolink <https://example.com/{word(1)}> and bare https://example.com/{word(1)} too.")
        else:
            parts.append(f"Entities: Bob &amp; Alice, a dash &mdash; here, a numeric &#39; ref, and an escape \\*not italic\\*.")
    return " ".join(parts)


def heading(level, text=None):
    text = text or sentence(3, 8).rstrip(".")
    return f"{'#' * level} {text}\n\n"


def blockquote(depth=1, lines=2):
    prefix = "> " * depth
    out = []
    for i in range(lines):
        out.append(f"{prefix}{sentence()}")
    return "\n".join(out) + "\n\n"


def unordered_list(n=4, nested=False):
    markers = ["-", "*", "+"]
    out = []
    for i in range(n):
        m = markers[i % len(markers)]
        out.append(f"{m} {sentence(3, 8)}")
        if nested and i == n // 2:
            out.append(f"  - {sentence(3, 6)}")
            out.append(f"    - {sentence(3, 6)}")
    return "\n".join(out) + "\n\n"


def ordered_list(n=4):
    out = [f"{i + 1}. {sentence(3, 8)}" for i in range(n)]
    return "\n".join(out) + "\n\n"


def task_list():
    out = [
        "- [ ] An unchecked task.",
        "- [x] A checked task.",
        "- [X] Also checked (uppercase).",
        f"1. [ ] {sentence(2, 4)}",
        f"2. [x] {sentence(2, 4)}",
    ]
    return "\n".join(out) + "\n\n"


def table(rows=4, cols=3, wide_cell=False):
    aligns = [":---", ":---:", "---:"]
    header = "| " + " | ".join(f"Column {c}" for c in range(cols)) + " |"
    delim = "| " + " | ".join(aligns[c % len(aligns)] for c in range(cols)) + " |"
    out = [header, delim]
    for r in range(rows):
        cells = []
        for c in range(cols):
            if wide_cell and r == 0 and c == 0:
                cells.append(word(8))
            else:
                cells.append(word(rng.randint(1, 3)))
        out.append("| " + " | ".join(cells) + " |")
    return "\n".join(out) + "\n\n"


def fenced_code(lines=6):
    body = "\n".join(f"    let {word(1)}_{i} = {rng.randint(0, 999)}; // **not bold**, [[not a link]]" for i in range(lines))
    return f"```rust\nfn example() {{\n{body}\n}}\n```\n\n"


def thematic_break():
    return rng.choice(["---", "***", "___", "- - -"]) + "\n\n"


def callout(ctype=None, title=None, fold=None, body_lines=2):
    ctype = ctype or rng.choice(CALLOUT_TYPES)
    marker = f"[!{ctype}]"
    if fold:
        marker += fold
    if title:
        marker += f" {title}"
    out = [f"> {marker}"]
    for _ in range(body_lines):
        out.append(f"> {sentence()}")
    return "\n".join(out) + "\n\n"


def compounding_block():
    out = [
        f"> # {sentence(2, 4).rstrip('.')}",
        "",
        f"> - {sentence(3, 6)}",
        "",
        f"- # {sentence(2, 4).rstrip('.')}",
        "",
        f"- > {sentence(3, 6)}",
        "",
    ]
    return "\n".join(out) + "\n"


def lazy_continuation_block():
    out = [
        f"- {sentence(4, 8)}",
        sentence(4, 8),
        "",
        f"> {sentence(4, 8)}",
        sentence(4, 8),
        "",
    ]
    return "\n".join(out) + "\n"


def chapter(i):
    buf = []
    buf.append(heading(1, f"Chapter {i}: {word(2).title()}"))
    buf.append(inline_salad(4))
    buf.append("\n\n")
    buf.append(heading(2, "Subsection: Lists"))
    buf.append(unordered_list(5, nested=True))
    buf.append(ordered_list(4))
    buf.append(task_list())
    buf.append(heading(2, "Subsection: Quotes and Compounding"))
    buf.append(blockquote(1, 2))
    buf.append(blockquote(2, 2))
    buf.append(compounding_block())
    buf.append(lazy_continuation_block())
    buf.append(heading(2, "Subsection: Table"))
    buf.append(table(rows=rng.randint(3, 8), cols=rng.randint(2, 5), wide_cell=(i % 7 == 0)))
    buf.append(heading(2, "Subsection: Callouts"))
    buf.append(callout())
    buf.append(callout(title=f"{word(2).title()}"))
    buf.append(callout(fold=rng.choice(["+", "-"])))
    buf.append(heading(2, "Subsection: Code"))
    buf.append(fenced_code(lines=rng.randint(4, 10)))
    buf.append(heading(2, "Subsection: Break"))
    buf.append(thematic_break())
    buf.append(inline_salad(2))
    buf.append("\n\n")
    return "".join(buf)


def extreme_stress_sections():
    buf = []

    buf.append(heading(1, "Stress: One Very Wide Table"))
    buf.append(table(rows=40, cols=30))

    buf.append(heading(1, "Stress: One Very Tall Table"))
    buf.append(table(rows=400, cols=4))

    buf.append(heading(1, "Stress: Deeply Nested List"))
    lines = []
    for depth in range(16):
        indent = "  " * depth
        lines.append(f"{indent}- {sentence(2, 4)}")
    buf.append("\n".join(lines) + "\n\n")

    buf.append(heading(1, "Stress: One Extremely Long Line"))
    buf.append(word(4000) + "\n\n")

    buf.append(heading(1, "Stress: Huge Fenced Code Block"))
    buf.append(fenced_code(lines=3000))

    buf.append(heading(1, "Stress: Many Consecutive Callouts"))
    for ctype in CALLOUT_TYPES * 4:
        buf.append(callout(ctype=ctype, body_lines=1))

    buf.append(heading(1, "Stress: Many Consecutive Thematic Breaks"))
    for _ in range(30):
        buf.append(thematic_break())

    buf.append(heading(1, "Stress: Link- and Autolink-Dense Paragraph"))
    dense = " ".join(
        f"[link{i}](https://example.com/{i}) <https://example.com/bare{i}> https://example.com/plain{i}"
        for i in range(200)
    )
    buf.append(dense + "\n\n")

    buf.append(heading(1, "Stress: Entity- and Escape-Dense Paragraph"))
    dense2 = " ".join(
        "Bob &amp; Alice &mdash; &ldquo;quote&rdquo; &#39; &#x27; \\*esc\\* \\[esc\\]"
        for _ in range(300)
    )
    buf.append(dense2 + "\n\n")

    buf.append(heading(1, "Stress: Deeply Nested Blockquote"))
    buf.append(">" * 12 + " " + sentence(4, 8) + "\n\n")

    return "".join(buf)


def math_section():
    """Formulas, inline and `$$` blocks (single- and multi-line), prose around
    them, and a few `$` that are not math. Deterministic and independent of
    the random stream above, so it is appended without changing anything else."""
    formulas = [
        r"x^2 + y^2 = z^2",
        r"\frac{-b \pm \sqrt{b^2-4ac}}{2a}",
        r"\sum_{i=1}^{n} \frac{1}{i^2}",
        r"\int_0^\infty e^{-x^2} \, dx",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        r"\begin{cases} x & x \ge 0 \\ -x & x < 0 \end{cases}",
        r"\vec{v} \cdot \vec{w} = \lVert v \rVert \lVert w \rVert \cos\theta",
        r"\lim_{n \to \infty} \left(1 + \frac{1}{n}\right)^n = e",
    ]
    out = ["\n# Stress: Math\n\n"]
    for i in range(600):
        f = formulas[i % len(formulas)]
        g = formulas[(i * 3 + 1) % len(formulas)]
        out.append(f"Chapter {i} states $a_{{{i % 7}}} + {i}$ and also [[Note {i % 50}]], costing $5 or $6 at most.\n\n")
        out.append(f"$${f}$$\n\n")
        out.append(f"$$\n{f}\n{g}\n$$\n\n")
        out.append(f"Between them [[Note {(i + 1) % 50}]] and $${g}$$ inline.\n\n")
    return "".join(out)


def main():
    out_path = "/home/apollo/Projects/Rust/Jade/crates/app/benches/fixtures/large_markdown_stress_test.md"

    header = (
        "# Large Markdown Stress Test\n\n"
        "Generated fixture (`crates/app/benches/fixtures/generate_stress_markdown.py`, "
        "seed 1337, deterministic) for future criterion benchmarks of "
        "`text_editor`'s rendering internals — see CLAUDE.md's Backlog entry on "
        "why `jade-app` can't link a real criterion bench against them yet. "
        "Exercises every construct Live Preview renders — headings, bold/italic, "
        "inline code, strikethrough, wikilinks (plain/aliased/heading-anchored), "
        "inline links, bracketed/bare autolinks, HTML entities, backslash escapes, "
        "blockquotes (including nesting and compounding), unordered/ordered/task "
        "lists (including nesting, lazy continuation, and compounding), tables "
        "(including very wide/tall ones), fenced code, thematic breaks, and "
        "Obsidian-style callouts (every built-in type/alias, custom titles, fold "
        "markers, an unrecognized type, and nested callouts) — at both a repeated "
        "\"typical note\" scale and a handful of deliberately extreme single "
        "elements (one huge table, one very long line, one huge fenced code "
        "block, deep list/blockquote nesting, dense link/entity paragraphs) meant "
        "to stress specific code paths beyond what repetition alone would reach.\n\n"
        "## Nested Callout Example\n\n"
        "> [!info]\n"
        "> Outer callout body.\n"
        ">> [!note]\n"
        ">> Plain nested-blockquote text, not its own callout (top-level only).\n\n"
        "## Unrecognized Callout Type\n\n"
        "> [!this-is-not-a-real-type]\n"
        "> Still a callout, defaulting to Note styling.\n\n"
        "## Deferred Construct (left untouched on purpose)\n\n"
        "![a diagram](https://example.com/diagram.png)\n\n"
    )

    chunks = [header]
    total = len(header.encode("utf-8"))
    i = 0
    while total < TARGET_BYTES:
        i += 1
        c = chapter(i)
        chunks.append(c)
        total += len(c.encode("utf-8"))

    extreme = extreme_stress_sections()
    chunks.append(extreme)
    chunks.append(math_section())

    text = "".join(chunks)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(text)

    size = len(text.encode("utf-8"))
    print(f"Wrote {out_path}: {size:,} bytes ({size / 1024 / 1024:.2f} MiB), {i} chapters")


if __name__ == "__main__":
    main()
