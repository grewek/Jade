//! Candidates 1-3 as [`SegmentParser`]s: Markdown in, units out. Containers
//! (and, where a library has no block math, `$$` blocks) come from the shared
//! pre-pass in [`crate::tree`].

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::own::single_line_kind;
use crate::tree::{self, Ctx, Kind, Node, SegmentParser, Unit};

/// Source positions that were not a valid range of the text.
pub static BAD_POSITIONS: AtomicUsize = AtomicUsize::new(0);

/// Builds units from blocks in document order: depth 0 starts a unit.
struct Builder<'a, 'o> {
    segment: &'a str,
    base: usize,
    ctx: Ctx,
    out: &'o mut Vec<Unit>,
}

impl Builder<'_, '_> {
    fn push(&mut self, mut kind: Kind, depth: usize, start: usize, end: usize) {
        let valid = start <= end
            && end <= self.segment.len()
            && self.segment.is_char_boundary(start)
            && self.segment.is_char_boundary(end);
        if !valid {
            BAD_POSITIONS.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if depth == 0 {
            let (start, end) = tree::whole_lines(self.segment, start, end);
            let text = &self.segment[start..end];
            if kind == Kind::Paragraph && !text.contains('\n') {
                kind = single_line_kind(text);
            }
            if kind == Kind::Quote && is_callout(text) {
                kind = Kind::Callout;
            }
            self.out.push(Unit {
                start: (self.base + start) as u32,
                len: (end - start) as u32,
                kind,
                ctx: self.ctx,
                children: Vec::new(),
            });
            return;
        }
        // tree-sitter includes a nested block's indentation only when it is
        // the first thing parsed.
        let indented = &self.segment[start..end];
        let start = end - indented.trim_start_matches([' ', '\t']).len();
        let text = self.segment[start..end].trim_end();
        if kind == Kind::Paragraph && !text.contains('\n') {
            kind = single_line_kind(text);
        }
        if let Some(unit) = self.out.last_mut() {
            let relative = (self.base + start).saturating_sub(unit.start as usize) as u32;
            let len = (text.len() as u32).min(unit.len.saturating_sub(relative));
            unit.children.push(Node {
                kind,
                depth: depth.min(255) as u8,
                start: relative,
                len,
            });
        }
    }
}

fn is_callout(quote: &str) -> bool {
    let first = quote.lines().next().unwrap_or("").trim_start();
    first
        .strip_prefix('>')
        .map(str::trim_start)
        .is_some_and(|rest| rest.starts_with("[!") && rest.contains(']'))
}

pub struct Pulldown;

impl Pulldown {
    pub fn options() -> pulldown_cmark::Options {
        use pulldown_cmark::Options;
        Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_MATH
            | Options::ENABLE_WIKILINKS
    }
}

impl SegmentParser for Pulldown {
    fn math_pre_pass(&self) -> bool {
        true
    }

    fn parse_segment(&mut self, segment: &str, base: usize, ctx: Ctx, out: &mut Vec<Unit>) {
        use pulldown_cmark::{Event, Tag, TagEnd};
        let first = out.len();
        let mut builder = Builder {
            segment,
            base,
            ctx,
            out,
        };
        let mut depth = 0;
        let parser = pulldown_cmark::Parser::new_ext(segment, Self::options());
        let mut events = parser.into_offset_iter();
        for (event, range) in events.by_ref() {
            match event {
                Event::Start(tag) => {
                    let kind = match tag {
                        Tag::Paragraph => Kind::Paragraph,
                        Tag::Heading { .. } => Kind::Heading,
                        Tag::BlockQuote(_) => Kind::Quote,
                        Tag::CodeBlock(_) => Kind::Code,
                        Tag::HtmlBlock => Kind::Html,
                        Tag::List(_) => Kind::List,
                        Tag::Item => Kind::Item,
                        Tag::Table(_) => Kind::Table,
                        _ => continue,
                    };
                    builder.push(kind, depth, range.start, range.end);
                    depth += 1;
                }
                Event::End(
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_)
                    | TagEnd::CodeBlock
                    | TagEnd::HtmlBlock
                    | TagEnd::List(_)
                    | TagEnd::Item
                    | TagEnd::Table,
                ) => depth -= 1,
                Event::Rule => builder.push(Kind::Rule, depth, range.start, range.end),
                _ => {}
            }
        }
        // Definitions produce no events; the parser lists them apart (and
        // only the first of each label).
        let definitions: Vec<_> = events
            .reference_definitions()
            .iter()
            .map(|(_, def)| def.span.clone())
            .collect();
        for span in definitions {
            let (start, end) = tree::whole_lines(segment, span.start, span.end);
            let at = out[first..].partition_point(|u| u.end() <= base + start) + first;
            if out
                .get(at)
                .is_some_and(|u| u.start as usize <= base + start)
            {
                continue;
            }
            let unit = Unit {
                start: (base + start) as u32,
                len: (end - start) as u32,
                kind: Kind::RefDef,
                ctx,
                children: Vec::new(),
            };
            out.insert(at, unit);
        }
    }
}

pub struct Comrak {
    options: comrak::Options<'static>,
    lines: Vec<usize>,
}

impl Comrak {
    pub fn new() -> Self {
        let mut options = comrak::Options::default();
        options.extension.table = true;
        options.extension.tasklist = true;
        options.extension.strikethrough = true;
        options.extension.wikilinks_title_after_pipe = true;
        options.extension.math_dollars = true;
        options.extension.math_dollars_block = true;
        Comrak {
            options,
            lines: Vec::new(),
        }
    }

    fn walk<'a>(&self, node: comrak::Node<'a>, depth: usize, builder: &mut Builder<'_, '_>) {
        use comrak::nodes::NodeValue;
        for child in node.children() {
            let data = child.data.borrow();
            let (kind, container) = match &data.value {
                NodeValue::Paragraph => (Kind::Paragraph, false),
                NodeValue::Heading(_) => (Kind::Heading, false),
                NodeValue::BlockQuote => (Kind::Quote, true),
                NodeValue::List(_) => (Kind::List, true),
                NodeValue::Item(_) | NodeValue::TaskItem(_) => (Kind::Item, true),
                NodeValue::CodeBlock(_) => (Kind::Code, false),
                NodeValue::HtmlBlock(_) => (Kind::Html, false),
                NodeValue::Table(_) => (Kind::Table, false),
                NodeValue::ThematicBreak(_) => (Kind::Rule, false),
                NodeValue::MathBlock(_) => (Kind::Math, false),
                _ => (Kind::Other, false),
            };
            // Lines and byte columns, both from 1; the end is inclusive.
            let position = data.sourcepos;
            let line = |n: usize| self.lines.get(n.wrapping_sub(1)).copied();
            let start = line(position.start.line).map(|l| l + position.start.column - 1);
            let end = line(position.end.line).map(|l| l + position.end.column);
            match (start, end) {
                (Some(start), Some(end)) => {
                    let end = end.min(builder.segment.len());
                    builder.push(kind, depth, start, end);
                }
                _ => {
                    BAD_POSITIONS.fetch_add(1, Ordering::Relaxed);
                }
            }
            if container {
                self.walk(child, depth + 1, builder);
            }
        }
    }
}

impl SegmentParser for Comrak {
    fn parse_segment(&mut self, segment: &str, base: usize, ctx: Ctx, out: &mut Vec<Unit>) {
        self.lines.clear();
        self.lines.push(0);
        self.lines
            .extend(segment.match_indices('\n').map(|(i, _)| i + 1));
        let arena = comrak::Arena::new();
        let root = comrak::parse_document(&arena, segment, &self.options);
        let mut builder = Builder {
            segment,
            base,
            ctx,
            out,
        };
        self.walk(root, 0, &mut builder);
    }
}

pub struct TreeSitter(pub tree_sitter::Parser);

impl TreeSitter {
    pub fn new() -> Self {
        let mut parser = tree_sitter::Parser::new();
        // The grammar and runtime versions are pinned by Cargo; a mismatch
        // would be a broken build of this spike.
        let loaded = parser.set_language(&tree_sitter_md::LANGUAGE.into());
        assert!(loaded.is_ok(), "grammar does not match the runtime");
        TreeSitter(parser)
    }
}

fn walk_tree(cursor: &mut tree_sitter::TreeCursor<'_>, depth: usize, out: &mut Builder<'_, '_>) {
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let node = cursor.node();
        let (kind, container) = match node.kind() {
            // Sections only group blocks under headings.
            "section" => {
                walk_tree(cursor, depth, out);
                (None, false)
            }
            "paragraph" => (Some(Kind::Paragraph), false),
            "atx_heading" | "setext_heading" => (Some(Kind::Heading), false),
            "block_quote" => (Some(Kind::Quote), true),
            "list" => (Some(Kind::List), true),
            "list_item" => (Some(Kind::Item), true),
            "fenced_code_block" | "indented_code_block" => (Some(Kind::Code), false),
            "html_block" => (Some(Kind::Html), false),
            "pipe_table" => (Some(Kind::Table), false),
            "thematic_break" => (Some(Kind::Rule), false),
            "link_reference_definition" => (Some(Kind::RefDef), false),
            _ => (None, false),
        };
        if let Some(kind) = kind {
            out.push(kind, depth, node.start_byte(), node.end_byte());
        }
        if container {
            walk_tree(cursor, depth + 1, out);
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    cursor.goto_parent();
}

impl SegmentParser for TreeSitter {
    fn math_pre_pass(&self) -> bool {
        true
    }

    fn parse_segment(&mut self, segment: &str, base: usize, ctx: Ctx, out: &mut Vec<Unit>) {
        // The grammar wants a final line break.
        let owned;
        let source = match segment.ends_with('\n') {
            true => segment,
            false => {
                owned = format!("{segment}\n");
                &owned
            }
        };
        let Some(parsed) = self.0.parse(source, None) else {
            return;
        };
        let mut builder = Builder {
            segment,
            base,
            ctx,
            out,
        };
        let mut cursor = parsed.walk();
        // The last nodes may reach into the added line break.
        walk_tree_clamped(&mut cursor, &mut builder);
    }
}

fn walk_tree_clamped(cursor: &mut tree_sitter::TreeCursor<'_>, builder: &mut Builder<'_, '_>) {
    let padded;
    let original = builder.segment;
    if original.ends_with('\n') {
        walk_tree(cursor, 0, builder);
        return;
    }
    padded = format!("{original}\n");
    let mut wide = Builder {
        segment: &padded,
        base: builder.base,
        ctx: builder.ctx,
        out: builder.out,
    };
    walk_tree(cursor, 0, &mut wide);
}
