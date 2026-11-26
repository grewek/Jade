#!/usr/bin/env python3
"""Builds the 1 MB and 5 MB notes of #133 from the stress fixture, cut on
block boundaries (chapter, section and math-entry starts), plus a variant of
each with <jade-aside>/<jade-columns> containers. Usage: make_fixtures.py OUT_DIR
"""
import pathlib
import re
import sys

SOURCE = pathlib.Path(__file__).resolve().parents[1] / "app/benches/fixtures/large_markdown_stress_test.md"


def split(text):
    first = text.index("# Chapter 1:")
    tail = text.index("# Stress: One Very Wide Table")
    math = text.index("# Stress: Math")
    chapters = re.split(r"(?m)^(?=# Chapter \d+:)", text[first:tail])
    return text[:first], [c for c in chapters if c], text[tail:math], text[math:]


def math_prefix(math, limit):
    # Every math entry starts with a "Chapter N states" paragraph.
    parts = re.split(r"(?m)^(?=Chapter \d+ states)", math)
    out = parts[0]
    for part in parts[1:]:
        if len(out) + len(part) > limit:
            break
        out += part
    return out


def build(target, header, chapters, extreme, math):
    # The 5 MB note keeps every extreme section; in 1 MB they would be half
    # of the note, so it keeps only a slice of the math section.
    tail = extreme + math if target >= 4_000_000 else math_prefix(math, target // 10)
    body, size = [], len(header) + len(tail)
    for chapter in chapters:
        if size >= target:
            break
        body.append(chapter)
        size += len(chapter)
    return header, body, tail


def section(chapter, name):
    start = chapter.index(f"## Subsection: {name}\n")
    end = chapter.find("## Subsection:", start + 5)
    return start, len(chapter) if end < 0 else end


def tag(chapter, number):
    """Every 3rd chapter floats its table section, every 4th puts its lists
    and quotes side by side."""
    if number % 3 == 0:
        start, end = section(chapter, "Table")
        chapter = (chapter[:start] + '<jade-aside side="left">\n\n' + chapter[start:end]
                   + "</jade-aside>\n\n" + chapter[end:])
    if number % 4 == 0:
        a, b = section(chapter, "Lists")
        c, d = section(chapter, "Quotes and Compounding")
        chapter = (chapter[:a] + "<jade-columns>\n\n<jade-column>\n\n" + chapter[a:b]
                   + "</jade-column>\n<jade-column>\n\n" + chapter[c:d]
                   + "</jade-column>\n\n</jade-columns>\n\n" + chapter[d:])
    return chapter


def main():
    out = pathlib.Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    header, chapters, extreme, math = split(SOURCE.read_text(encoding="utf-8"))
    for name, target in (("1mb", 1 << 20), ("5mb", 5 << 20)):
        head, body, tail = build(target, header, chapters, extreme, math)
        plain = head + "".join(body) + tail
        tagged = head + "".join(tag(c, i + 1) for i, c in enumerate(body)) + tail
        for suffix, text in (("", plain), ("-tagged", tagged)):
            path = out / f"{name}{suffix}.md"
            path.write_bytes(text.encode())
            print(path.name, len(text.encode()), "bytes", text.count("\n"), "lines")


if __name__ == "__main__":
    main()
