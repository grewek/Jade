//! Fenced code blocks, line by line. The one definition of where a fence
//! opens and closes, shared by the editor's line classification and by every
//! pass over a note's text that must leave code alone.

/// Whether a fence is open entering a line, and if so, its character and the
/// length of its opening run: the whole cross-line state, which is what lets
/// a caller resume from any line.
pub type FenceState = Option<(u8, usize)>;

/// What a line is, as far as fences go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceRole {
    /// Outside any fence; a layout marker always is.
    Plain,
    /// The line that opens or closes a fence.
    Delimiter,
    /// Inside a fence: literal text, never Markdown.
    Content,
}

/// How every line that shapes a layout (columns, asides, floats) starts. The
/// rest of the grammar is the editor's.
pub const MARKER_PREFIX: &str = "<!-- jade:";
/// How such a line ends.
pub const MARKER_SUFFIX: &str = "-->";

/// Whether `line` is a layout marker: unindented, and the whole line.
pub fn is_layout_marker(line: &str) -> bool {
    line.starts_with(MARKER_PREFIX) && line.trim_end().ends_with(MARKER_SUFFIX)
}

/// The line's role and the fence state entering the next line, given the
/// state entering `line`. An unclosed fence runs to the next layout marker, or
/// to the end of the text. A marker ends a fence from its own line alone,
/// whether or not it forms a layout: asking that would need the markers
/// paired first, which needs to know what is code.
#[inline]
pub fn step(state: FenceState, line: &str) -> (FenceRole, FenceState) {
    match state {
        // A marker opens no fence, so only an open one has to look for it.
        None => match fence_delimiter(line) {
            Some(fence) => (FenceRole::Delimiter, Some(fence)),
            None => (FenceRole::Plain, None),
        },
        Some((fence_char, min_len)) => {
            if is_layout_marker(line) {
                (FenceRole::Plain, None)
            } else if closes_fence(line, fence_char, min_len) {
                (FenceRole::Delimiter, None)
            } else {
                (FenceRole::Content, Some((fence_char, min_len)))
            }
        }
    }
}

/// Walks a note's lines in order, saying for each whether it is code.
#[derive(Default)]
pub struct FenceWalk(FenceState);

impl FenceWalk {
    pub fn role(&mut self, line: &str) -> FenceRole {
        let (role, next) = step(self.0, line);
        self.0 = next;
        role
    }
}

/// The `\n`-separated lines of `text` with their zero-based numbers and
/// roles.
pub fn lines_with_roles(text: &str) -> impl Iterator<Item = (usize, &str, FenceRole)> {
    let mut walk = FenceWalk::default();
    text.split('\n')
        .enumerate()
        .map(move |(n, line)| (n, line, walk.role(line)))
}

/// The lines of `text` outside fenced code, with their zero-based numbers:
/// what a pass that looks for Markdown, not code, should read.
pub fn plain_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    lines_with_roles(text)
        .filter_map(|(n, line, role)| (role == FenceRole::Plain).then_some((n, line)))
}

/// `line`'s fence character and run length, if it opens a fence: three or
/// more `` ` `` or `~` at the very start of the line (CommonMark's allowance
/// of up to three leading spaces is not handled), optionally followed by an
/// info string.
fn fence_delimiter(line: &str) -> Option<(u8, usize)> {
    let fence_char = *line.as_bytes().first()?;
    if fence_char != b'`' && fence_char != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == fence_char).count();
    (len >= 3).then_some((fence_char, len))
}

/// Whether `line` closes a fence opened with `fence_char` repeated at least
/// `min_len` times: a run of that character, at least as long, followed by
/// nothing but blank space (the `\r` of a Windows line ending included).
fn closes_fence(line: &str, fence_char: u8, min_len: usize) -> bool {
    let len = line.bytes().take_while(|&b| b == fence_char).count();
    len >= min_len
        && line[len..]
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\r'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(text: &str) -> Vec<FenceRole> {
        let mut state = None;
        text.split('\n')
            .map(|line| {
                let (role, next) = step(state, line);
                state = next;
                role
            })
            .collect()
    }

    #[test]
    fn a_fence_has_delimiters_and_content() {
        use FenceRole::*;
        assert_eq!(
            roles("a\n```rust\nb\n```\nc"),
            [Plain, Delimiter, Content, Delimiter, Plain]
        );
    }

    /// As in CommonMark, only blank space may follow a closing run; the `\r`
    /// of a Windows line ending counts as that.
    #[test]
    fn a_closer_may_be_followed_by_blank_space() {
        use FenceRole::*;
        for closer in ["``` ", "```\t", "```  \t ", "```\r", "```` "] {
            assert_eq!(
                roles(&format!("```\nx\n{closer}\nafter")),
                [Delimiter, Content, Delimiter, Plain],
                "{closer:?}"
            );
        }
        assert_eq!(
            roles("```\r\nx\r\n```\r\ny"),
            [Delimiter, Content, Delimiter, Plain]
        );
        for text_after in ["``` x", "```x ", "``` ```"] {
            assert_eq!(
                roles(&format!("```\n{text_after}")),
                [Delimiter, Content],
                "{text_after:?}"
            );
        }
    }

    /// A layout's end must not depend on the code inside it being finished
    /// (#126).
    #[test]
    fn a_layout_marker_ends_an_open_fence() {
        use FenceRole::*;
        assert_eq!(
            roles("```\nx\n<!-- jade:end -->\nafter\n```\ny"),
            [Delimiter, Content, Plain, Plain, Delimiter, Content]
        );
        assert_eq!(
            roles("~~~\n<!-- jade:float left 30% -->  \r\nx"),
            [Delimiter, Plain, Plain]
        );
        for code in [
            " <!-- jade:end -->",
            "<!-- jade:end --> x",
            "<!-- note -->",
            "<!--jade:end-->",
            "---",
            ":::",
        ] {
            assert_eq!(
                roles(&format!("```\n{code}")),
                [Delimiter, Content],
                "{code:?}"
            );
        }
    }

    #[test]
    fn a_shorter_or_mixed_closer_does_not_close() {
        use FenceRole::*;
        assert_eq!(
            roles("````\n```\n~~~~\n````"),
            [Delimiter, Content, Content, Delimiter]
        );
        assert_eq!(roles("``\nx"), [Plain, Plain], "two backticks open nothing");
        assert_eq!(roles("```\nx"), [Delimiter, Content], "unclosed runs on");
    }
}
