//! How a note's name relates to its file: the `.md` extension and picking a
//! free name. One place, so creating, renaming, resolving and rewriting links
//! agree on what a note is called.

const MD: &str = ".md";

/// `name` split into the part before a trailing `.md` (any case) and that
/// extension as written, which is empty when there is none.
pub(crate) fn split_md(name: &str) -> (&str, &str) {
    match name.len().checked_sub(MD.len()) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(MD) => {
            name.split_at(at)
        }
        _ => (name, ""),
    }
}

/// The file name of the note titled `title`.
pub(crate) fn file_name(title: &str) -> String {
    format!("{title}{MD}")
}

/// `base`, or the first of `base 1`, `base 2`, ... that `taken` does not
/// reject. Ends because only finitely many names are taken.
pub(crate) fn first_free(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut candidate = base.to_string();
    let mut suffix = 1;
    while taken(&candidate) {
        candidate = format!("{base} {suffix}");
        suffix += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_trailing_md_in_any_case() {
        assert_eq!(split_md("Note.md"), ("Note", ".md"));
        assert_eq!(split_md("Note.MD"), ("Note", ".MD"));
        assert_eq!(split_md("Note"), ("Note", ""));
        assert_eq!(split_md("a.md.txt"), ("a.md.txt", ""));
        assert_eq!(split_md(".md"), ("", ".md"));
        assert_eq!(split_md("\u{e9}"), ("\u{e9}", ""), "not mid-character");
        assert_eq!(file_name("A b"), "A b.md");
    }

    #[test]
    fn first_free_numbers_from_one() {
        let taken = ["T", "T 1", "T 2"];
        assert_eq!(first_free("T", |c| taken.contains(&c)), "T 3");
        assert_eq!(first_free("U", |c| taken.contains(&c)), "U");
    }
}
