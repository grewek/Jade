//! Changing one key of `config.toml` in place (#58 H1). The text is edited,
//! not re-serialised, so the comments, key order and layout of a hand-written
//! file survive a change made in the Settings dialog.

/// `text` with `key = value` set in `[section]` (the top of the file when
/// `section` is `None`): an existing key keeps its place and trailing comment,
/// a new one goes after the section's last key, a missing section is appended.
/// `value` is TOML source (`16`, `"name"`, `[1, 2]`). The result is checked to
/// parse, so a broken file or value is refused, not written.
pub fn set_key(
    text: &str,
    section: Option<&str>,
    key: &str,
    value: &str,
) -> Result<String, String> {
    // A file that does not parse is never touched: editing it could hide
    // what is wrong, or drop what the author meant.
    text.parse::<toml::Table>()
        .map_err(|e| format!("config.toml: {e}; not changed"))?;
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = section_span(&lines, section);
    let found = (start..end).find(|&i| key_of(lines[i]) == Some(key));
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    match (found, section) {
        (Some(i), _) => {
            let last = value_end(&lines, i);
            let comment = trailing_comment(lines[last]);
            out[i] = format!("{} = {value}{comment}", key_of(lines[i]).unwrap_or(key));
            out.drain(i + 1..=last);
        }
        (None, _) if section_exists(&lines, section) => {
            let at = (start..end)
                .rev()
                .find(|&i| !lines[i].trim().is_empty())
                .map_or(start, |i| i + 1);
            out.insert(at, format!("{key} = {value}"));
        }
        (None, Some(name)) => {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{name}]"));
            out.push(format!("{key} = {value}"));
        }
        (None, None) => out.insert(start, format!("{key} = {value}")),
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
        .parse::<toml::Table>()
        .map(|_| result.clone())
        .map_err(|e| format!("config.toml: {e}; not changed"))
}

/// The header name of a `[table]` line.
fn header_of(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?;
    if inner.starts_with('[') {
        return Some("[[");
    }
    inner.split(']').next().map(str::trim)
}

fn section_exists(lines: &[&str], section: Option<&str>) -> bool {
    section.is_none_or(|name| lines.iter().any(|l| header_of(l) == Some(name)))
}

/// The line range of a section's body: after its header (or from the top) up to
/// the next header.
fn section_span(lines: &[&str], section: Option<&str>) -> (usize, usize) {
    let start = match section {
        None => 0,
        Some(name) => match lines.iter().position(|l| header_of(l) == Some(name)) {
            Some(i) => i + 1,
            None => return (lines.len(), lines.len()),
        },
    };
    let end = (start..lines.len())
        .find(|&i| header_of(lines[i]).is_some())
        .unwrap_or(lines.len());
    (start, end)
}

/// The key a `key = value` line sets.
fn key_of(line: &str) -> Option<&str> {
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    plain.then_some(key)
}

/// The last line of the value that starts on line `i`: a multi-line array runs
/// to the line that closes it.
fn value_end(lines: &[&str], i: usize) -> usize {
    let value = lines[i].split_once('=').map_or("", |(_, v)| v.trim_start());
    if !value.starts_with('[') {
        return i;
    }
    let mut depth = 0i32;
    for (j, line) in lines.iter().enumerate().skip(i) {
        let text = if j == i { value } else { line };
        let mut quoted = false;
        for c in text.chars() {
            match c {
                '"' => quoted = !quoted,
                '#' if !quoted => break,
                '[' if !quoted => depth += 1,
                ']' if !quoted => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            return j;
        }
    }
    lines.len() - 1
}

/// The ` # comment` ending `line` (with its leading spaces), or nothing.
fn trailing_comment(line: &str) -> String {
    let body = line.split_once('=').map_or(line, |(_, v)| v);
    let mut quoted = false;
    for (at, c) in body.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => {
                let before = body[..at].trim_end().len();
                return format!("{}{}", &body[before..at], &body[at..])
                    .trim_end()
                    .to_owned();
            }
            _ => {}
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::set_key;

    const SRC: &str = "justify = false\ntheme = \"a\" # the look\n\n[font]\n# body\nsize = 16   # px\n\n[heading]\nscale = [\n  1.0, # h1\n  2.0,\n]\nrule = true\n";

    #[test]
    fn an_existing_key_keeps_its_place_and_comment() {
        let out = set_key(SRC, Some("font"), "size", "20").unwrap();
        assert!(out.contains("size = 20   # px"), "{out}");
        assert!(out.contains("# body"));
        let out = set_key(SRC, None, "theme", "\"b\"").unwrap();
        assert!(out.contains("theme = \"b\" # the look"), "{out}");
    }

    #[test]
    fn a_multi_line_array_is_replaced_whole() {
        let out = set_key(SRC, Some("heading"), "scale", "[1.5, 1.0]").unwrap();
        assert!(out.contains("scale = [1.5, 1.0]\nrule = true"), "{out}");
        assert!(!out.contains("h1"));
    }

    #[test]
    fn a_new_key_joins_its_section_or_a_new_section_is_added() {
        let out = set_key(SRC, Some("font"), "line_height", "1.5").unwrap();
        assert!(
            out.contains("size = 16   # px\nline_height = 1.5\n\n[heading]"),
            "{out}"
        );
        let out = set_key(SRC, Some("shape"), "radius", "6").unwrap();
        assert!(
            out.ends_with("rule = true\n\n[shape]\nradius = 6\n"),
            "{out}"
        );
        let out = set_key(SRC, None, "vault", "\"/v\"").unwrap();
        assert!(
            out.starts_with("justify = false\ntheme = \"a\" # the look\nvault = \"/v\"\n"),
            "{out}"
        );
    }

    #[test]
    fn an_empty_file_and_a_broken_one() {
        assert_eq!(
            set_key("", Some("font"), "size", "18").unwrap(),
            "[font]\nsize = 18\n"
        );
        assert!(
            set_key("a = [", None, "b", "1").is_err(),
            "a broken file is refused"
        );
        assert!(set_key("a = [", None, "a", "1").is_err());
        assert!(set_key(SRC, None, "b", "[").is_err());
    }

    #[test]
    fn a_key_of_another_section_is_left_alone() {
        let src = "[a]\nsize = 1\n[b]\nx = 2\n";
        let out = set_key(src, Some("b"), "size", "9").unwrap();
        assert_eq!(out, "[a]\nsize = 1\n[b]\nx = 2\nsize = 9\n");
    }
}
