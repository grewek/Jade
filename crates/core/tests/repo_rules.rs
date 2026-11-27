//! Checks of the repository itself: the process rules of `CLAUDE.md` that a
//! test can decide (#130 §10). They live in `jade-core` because it builds
//! without GPUI's system libraries.

use std::fs;
use std::path::{Path, PathBuf};

const FILE_LINE_LIMIT: usize = 800;

/// Files that were over the limit when the check arrived, with their length
/// then. None may grow past it; raising a number needs a reason in the PR.
/// Never add a file: split it. An entry goes once its file is within the limit.
const OVERSIZED: &[(&str, usize)] = &[
    ("crates/app/src/app/tests.rs", 4010),
    ("crates/app/src/text_editor/buffer.rs", 1298),
    ("crates/app/src/text_editor/columns/mod.rs", 1237),
    ("crates/app/src/text_editor/editor.rs", 905),
    ("crates/app/src/text_editor/editor/column_edit.rs", 808),
    ("crates/app/src/text_editor/element.rs", 4620),
    ("crates/app/src/text_editor/element/input.rs", 863),
    ("crates/app/src/text_editor/element/paint.rs", 841),
    ("crates/app/src/text_editor/element/region.rs", 1280),
    ("crates/app/src/text_editor/layout/shape.rs", 1096),
    ("crates/app/src/text_editor/math/layout.rs", 1091),
    ("crates/app/src/text_editor/math/layout_tests.rs", 954),
    ("crates/app/src/text_editor/math/parse.rs", 889),
    ("crates/app/src/text_editor/preview/tests.rs", 1593),
    ("crates/app/src/text_editor/rows/count.rs", 827),
    ("crates/app/src/text_editor/table/model.rs", 1334),
    ("crates/app/src/theme/tests.rs", 1051),
    ("crates/core/src/vault.rs", 884),
    ("crates/debug-cli/src/args.rs", 881),
    ("crates/debug-protocol/src/lib.rs", 1021),
];

/// `ARCHITECTURE.md` holds rules and invariants only. Bytes, not lines: the
/// version that grew into a repair log did so in very long paragraphs.
const ARCHITECTURE_BYTE_LIMIT: u64 = 16 * 1024;

/// The crates the block-type check knows about. A new one must be added here,
/// and to `NEW_CORE` if it is part of the rendering core.
const CRATES: &[&str] = &["app", "core", "debug-cli", "debug-protocol"];
const PLANNED_CRATES: &[&str] = &["layout"];

/// The source trees of the new rendering core (#130 §6.1). Everything in them
/// is shared code except `blocks/`, so a new module is checked by default.
/// They do not exist before stage 2 (#141, #142).
const NEW_CORE: &[&str] = &["crates/layout/src", "crates/app/src/text_editor/v2"];
const BLOCKS_DIR: &str = "blocks";

/// The block types of `SPEC.md` §2, as words of an identifier. `code`, `block`
/// and `quote` alone are ordinary words, so only their pairs count.
const BLOCK_WORDS: &[&str] = &[
    "aside",
    "blockquote",
    "callout",
    "column",
    "columns",
    "float",
    "formula",
    "heading",
    "image",
    "list",
    "paragraph",
    "table",
];
const BLOCK_WORD_PAIRS: &[(&str, &str)] = &[
    ("block", "quote"),
    ("code", "block"),
    ("reference", "definition"),
    ("thematic", "break"),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// All `.rs` files below `dir`; none if it does not exist.
fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// What is wrong with a file of `lines` lines whose exemption is `ceiling`.
fn length_problem(path: &str, lines: usize, ceiling: Option<usize>) -> Option<String> {
    match ceiling {
        None if lines > FILE_LINE_LIMIT => Some(format!(
            "{path}: {lines} lines, over the limit of {FILE_LINE_LIMIT}. Split it."
        )),
        Some(ceiling) if lines > ceiling => Some(format!(
            "{path}: {lines} lines, over its ceiling of {ceiling}. Move code out."
        )),
        Some(_) if lines <= FILE_LINE_LIMIT => Some(format!(
            "{path}: now within the limit. Remove its `OVERSIZED` entry."
        )),
        _ => None,
    }
}

#[test]
fn files_stay_within_the_length_limit() {
    let root = repo_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    assert!(!files.is_empty(), "no sources found");

    let mut problems = Vec::new();
    for file in files {
        let path = file.strip_prefix(&root).unwrap().to_str().unwrap();
        let lines = fs::read_to_string(&file).unwrap().lines().count();
        let ceiling = OVERSIZED.iter().find(|(p, _)| *p == path).map(|e| e.1);
        problems.extend(length_problem(path, lines, ceiling));
    }
    for (path, _) in OVERSIZED {
        if !root.join(path).is_file() {
            problems.push(format!("{path}: gone. Remove its `OVERSIZED` entry."));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn architecture_md_stays_short() {
    let bytes = fs::metadata(repo_root().join("ARCHITECTURE.md"))
        .unwrap()
        .len();
    assert!(
        bytes <= ARCHITECTURE_BYTE_LIMIT,
        "ARCHITECTURE.md is {bytes} bytes, limit {ARCHITECTURE_BYTE_LIMIT}. \
         It holds rules and invariants only."
    );
}

/// `source` with comments and the contents of string and character literals
/// blanked out. Line breaks stay, so line numbers still match.
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let end = match (at(i), at(i + 1)) {
            ('/', '/') => (i..chars.len())
                .find(|&j| at(j) == '\n')
                .unwrap_or(chars.len()),
            ('/', '*') => {
                let (mut j, mut depth) = (i + 2, 1);
                while j < chars.len() && depth > 0 {
                    match (at(j), at(j + 1)) {
                        ('/', '*') => (j, depth) = (j + 2, depth + 1),
                        ('*', '/') => (j, depth) = (j + 2, depth - 1),
                        _ => j += 1,
                    }
                }
                j
            }
            // A raw string: `r"…"`, `r#"…"#`, also after `b` or `c`.
            ('r', '"' | '#') if i == 0 || !is_ident(at(i - 1)) || "bc".contains(at(i - 1)) => {
                let hashes = (i + 1..).take_while(|&j| at(j) == '#').count();
                let open = i + 1 + hashes;
                if at(open) != '"' {
                    // A raw identifier such as `r#type`.
                    out.push('r');
                    i += 1;
                    continue;
                }
                (open + 1..chars.len())
                    .find(|&j| at(j) == '"' && (1..=hashes).all(|k| at(j + k) == '#'))
                    .map_or(chars.len(), |j| j + 1 + hashes)
            }
            // A character literal; any other `'` starts a lifetime.
            ('\'', next) if next == '\\' || at(i + 2) == '\'' => (i + 3..chars.len())
                .find(|&j| at(j - 1) == '\'' || at(j) == '\'')
                .map_or(chars.len(), |j| j + 1),
            ('"', _) => {
                let mut j = i + 1;
                while j < chars.len() && at(j) != '"' {
                    j += if at(j) == '\\' { 2 } else { 1 };
                }
                j + 1
            }
            (c, _) => {
                out.push(c);
                i += 1;
                continue;
            }
        };
        let end = end.min(chars.len());
        out.extend(chars[i..end].iter().filter(|&&c| c == '\n'));
        i = end;
    }
    out
}

/// The lowercase words of an identifier in `snake_case` or `CamelCase`.
fn words(identifier: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut previous = '_';
    let mut chars = identifier.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '_' {
            // A capital starts a word, also the last one of an acronym.
            let ends_acronym = chars.peek().is_some_and(|next| next.is_lowercase());
            if previous == '_' || (c.is_uppercase() && (!previous.is_uppercase() || ends_acronym)) {
                words.push(String::new());
            }
            if let Some(word) = words.last_mut() {
                word.extend(c.to_lowercase());
            }
        }
        previous = c;
    }
    words
}

fn names_a_block_type(identifier: &str) -> bool {
    let words = words(identifier);
    let singular = |word: &str| word.strip_suffix('s').unwrap_or(word).to_owned();
    let matches = |word: &str, name: &str| word == name || singular(word) == name;
    words
        .iter()
        .any(|w| BLOCK_WORDS.iter().any(|name| matches(w, name)))
        || words.windows(2).any(|pair| {
            BLOCK_WORD_PAIRS
                .iter()
                .any(|(a, b)| pair[0] == *a && matches(&pair[1], b))
        })
}

/// Line number and identifier of each block type named in production code.
/// Top-level `#[cfg(test)]` modules are skipped.
fn block_type_names(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let (mut after_cfg_test, mut in_test_module) = (false, false);
    for (index, line) in code_only(source).lines().enumerate() {
        if in_test_module {
            // rustfmt closes a top-level module in column 0.
            in_test_module = !line.starts_with('}');
            continue;
        }
        if after_cfg_test && line.starts_with("mod ") && line.trim_end().ends_with('{') {
            in_test_module = true;
            continue;
        }
        after_cfg_test = line.starts_with("#[cfg(test)]");
        found.extend(
            line.split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|identifier| names_a_block_type(identifier))
                .map(|identifier| (index + 1, identifier.to_owned())),
        );
    }
    found
}

fn is_test_file(path: &Path) -> bool {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    name == "tests"
        || name.ends_with("_tests")
        || name.starts_with("test_")
        || path.components().any(|c| c.as_os_str() == "tests")
}

#[test]
fn shared_code_does_not_name_block_types() {
    let root = repo_root();
    let mut problems = Vec::new();
    for tree in NEW_CORE {
        let tree = root.join(tree);
        let mut files = Vec::new();
        rust_files(&tree, &mut files);
        for file in files {
            let inside = file.strip_prefix(&tree).unwrap();
            if inside.starts_with(BLOCKS_DIR) || is_test_file(inside) {
                continue;
            }
            let path = file.strip_prefix(&root).unwrap().display();
            let source = fs::read_to_string(&file).unwrap();
            for (line, identifier) in block_type_names(&source) {
                problems.push(format!("{path}:{line}: `{identifier}`"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "shared code names a block type; move it into `{BLOCKS_DIR}/` or the \
         block's interface:\n{}",
        problems.join("\n")
    );
}

/// The check above finds nothing in a tree it does not know, so a new crate
/// has to be classified first.
#[test]
fn every_crate_is_known_to_the_rule_checks() {
    for entry in fs::read_dir(repo_root().join("crates")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            CRATES.contains(&name.as_str()) || PLANNED_CRATES.contains(&name.as_str()),
            "unknown crate `crates/{name}`: add it to `CRATES`, and to `NEW_CORE` \
             if it is part of the rendering core"
        );
    }
}

#[test]
fn the_length_check_catches_growth_and_stale_exemptions() {
    assert_eq!(length_problem("a.rs", 800, None), None);
    assert!(length_problem("a.rs", 801, None).is_some());
    assert_eq!(length_problem("a.rs", 900, Some(900)), None);
    assert!(length_problem("a.rs", 901, Some(900)).is_some());
    assert!(length_problem("a.rs", 800, Some(900)).is_some());
}

#[test]
fn block_type_names_are_found_in_identifiers() {
    for named in [
        "Paragraph",
        "TableBlock",
        "is_heading",
        "lists",
        "CodeBlock",
        "code_blocks",
        "ThematicBreak",
        "HTMLTable",
    ] {
        assert!(names_a_block_type(named), "{named}");
    }
    for plain in [
        "Block",
        "BlockId",
        "key_code",
        "listener",
        "tablet",
        "line_break",
    ] {
        assert!(!names_a_block_type(plain), "{plain}");
    }
}

#[test]
fn comments_literals_and_test_modules_name_nothing() {
    let source = r####"// a table
/* a /* nested */ table */
/// Doc about a table.
const A: &str = "table \" table";
const B: &str = r#"a "table" here"#;
const C: (char, char, &[u8]) = ('"', '\'', br"table\");
#[cfg(test)]
mod tests {
    fn table() {}
}
fn after<'a>(r#type: &'a Heading) {}
"####;
    assert_eq!(block_type_names(source), [(11, "Heading".to_owned())]);
}
