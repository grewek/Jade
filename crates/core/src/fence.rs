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
    /// Outside any fence.
    Plain,
    /// The line that opens or closes a fence.
    Delimiter,
    /// Inside a fence: literal text, never Markdown.
    Content,
}

/// The line's role and the fence state entering the next line, given the
/// state entering `line`. An unclosed fence runs to the end of the text.
pub fn step(state: FenceState, line: &str) -> (FenceRole, FenceState) {
    match state {
        None => match fence_delimiter(line) {
            Some(fence) => (FenceRole::Delimiter, Some(fence)),
            None => (FenceRole::Plain, None),
        },
        Some((fence_char, min_len)) => {
            if closes_fence(line, fence_char, min_len) {
                (FenceRole::Delimiter, None)
            } else {
                (FenceRole::Content, Some((fence_char, min_len)))
            }
        }
    }
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
/// `min_len` times: a run of exactly that character, at least as long, and
/// nothing else on the line.
fn closes_fence(line: &str, fence_char: u8, min_len: usize) -> bool {
    let len = line.bytes().take_while(|&b| b == fence_char).count();
    len >= min_len && len == line.len()
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
