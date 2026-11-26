#!/usr/bin/env python3
"""Turns CommonMark's spec.json into N.md / N.expected pairs, where
`expected` is the block elements of the example's HTML as depth+tag. The two
raw-HTML sections are left out: Jade shows HTML as text (K9).
Usage: spec_skeletons.py spec.json OUT_DIR
"""
import json
import pathlib
import sys
from html.parser import HTMLParser

BLOCKS = {"p", "ul", "ol", "li", "blockquote", "pre", "hr", "h1", "h2", "h3", "h4", "h5", "h6"}


class Skeleton(HTMLParser):
    def __init__(self):
        super().__init__()
        self.stack, self.out = [], []

    def handle_starttag(self, tag, attrs):
        if tag not in BLOCKS:
            return
        if not (tag == "p" and "li" in self.stack):
            self.out.append(f"{len(self.stack)}{'h' if tag[0] == 'h' and tag != 'hr' else tag}")
        if tag != "hr":
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if tag in self.stack and tag != "hr":
            del self.stack[len(self.stack) - 1 - self.stack[::-1].index(tag):]


def main():
    out = pathlib.Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    index = []
    for example in json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")):
        if example["section"] in ("HTML blocks", "Raw HTML"):
            continue
        number = example["example"]
        parser = Skeleton()
        parser.feed(example["html"])
        (out / f"{number}.md").write_bytes(example["markdown"].encode())
        (out / f"{number}.expected").write_text("".join(f"{line}\n" for line in parser.out))
        index.append(f"{number}\t{example['section']}\n")
    (out / "index.tsv").write_text("".join(index))
    print(len(index), "examples")


if __name__ == "__main__":
    main()
