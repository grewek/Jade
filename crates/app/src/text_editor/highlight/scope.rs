//! Maps TextMate scopes (`keyword.control.rust`) to theme syntax roles.

use crate::theme::Role;

/// Scope prefixes and their role. The longest prefix that ends on a `.`
/// boundary wins, so `keyword.operator` beats `keyword`; order is free.
const TABLE: &[(&str, Role)] = &[
    ("comment", Role::SyntaxComment),
    ("punctuation.definition.comment", Role::SyntaxComment),
    ("string", Role::SyntaxString),
    ("punctuation.definition.string", Role::SyntaxString),
    ("constant.numeric", Role::SyntaxNumber),
    ("constant.character.escape", Role::SyntaxConstant),
    ("constant", Role::SyntaxConstant),
    ("support.constant", Role::SyntaxConstant),
    ("keyword", Role::SyntaxKeyword),
    ("keyword.operator", Role::SyntaxOperator),
    ("keyword.operator.word", Role::SyntaxKeyword),
    ("storage", Role::SyntaxKeyword),
    ("storage.type.numeric", Role::SyntaxType),
    ("storage.type.primitive", Role::SyntaxType),
    ("variable.language", Role::SyntaxKeyword),
    ("entity.name.tag", Role::SyntaxKeyword),
    ("entity.name.function", Role::SyntaxFunction),
    ("variable.function", Role::SyntaxFunction),
    ("support.function", Role::SyntaxFunction),
    ("entity.name.type", Role::SyntaxType),
    ("entity.name.class", Role::SyntaxType),
    ("entity.name.struct", Role::SyntaxType),
    ("entity.name.enum", Role::SyntaxType),
    ("entity.name.trait", Role::SyntaxType),
    ("entity.name.namespace", Role::SyntaxType),
    ("entity.other.inherited-class", Role::SyntaxType),
    ("entity.other.attribute-name", Role::SyntaxType),
    ("support.type", Role::SyntaxType),
    ("support.class", Role::SyntaxType),
    ("variable", Role::SyntaxVariable),
    ("punctuation", Role::SyntaxPunctuation),
];

/// The role for one scope, or `None` (plain text) when no prefix matches.
pub(in crate::text_editor) fn role_for_scope(scope: &str) -> Option<Role> {
    TABLE
        .iter()
        .filter(|(prefix, _)| is_prefix(prefix, scope))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|&(_, role)| role)
}

/// The role for a scope stack, outermost first: the innermost scope that has
/// a role decides (`string.quoted` inside `meta.function` is a string).
pub(in crate::text_editor) fn role_for_stack<'a>(
    stack: impl DoubleEndedIterator<Item = &'a str>,
) -> Option<Role> {
    stack.rev().find_map(role_for_scope)
}

fn is_prefix(prefix: &str, scope: &str) -> bool {
    scope
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::{TABLE, role_for_scope, role_for_stack};
    use crate::theme::Role;

    #[test]
    fn scopes_map_by_longest_prefix() {
        let cases = [
            ("comment.line.double-slash.rust", Some(Role::SyntaxComment)),
            (
                "punctuation.definition.comment.rust",
                Some(Role::SyntaxComment),
            ),
            ("string.quoted.double.rust", Some(Role::SyntaxString)),
            (
                "punctuation.definition.string.begin",
                Some(Role::SyntaxString),
            ),
            ("constant.numeric.integer.decimal", Some(Role::SyntaxNumber)),
            ("constant.language.boolean", Some(Role::SyntaxConstant)),
            ("constant.character.escape.rust", Some(Role::SyntaxConstant)),
            ("keyword.control.rust", Some(Role::SyntaxKeyword)),
            ("keyword.operator.arithmetic", Some(Role::SyntaxOperator)),
            ("keyword.operator.word.python", Some(Role::SyntaxKeyword)),
            ("storage.type.function.rust", Some(Role::SyntaxKeyword)),
            ("storage.type.numeric.rust", Some(Role::SyntaxType)),
            ("entity.name.function.rust", Some(Role::SyntaxFunction)),
            ("entity.name.type.struct", Some(Role::SyntaxType)),
            ("support.type.primitive", Some(Role::SyntaxType)),
            ("variable.parameter.rust", Some(Role::SyntaxVariable)),
            ("variable.language.self", Some(Role::SyntaxKeyword)),
            (
                "punctuation.section.block.begin",
                Some(Role::SyntaxPunctuation),
            ),
            ("keyword", Some(Role::SyntaxKeyword)),
        ];
        for (scope, role) in cases {
            assert_eq!(role_for_scope(scope), role, "{scope}");
        }
    }

    #[test]
    fn unknown_scopes_stay_plain() {
        for scope in [
            "",
            "source.rust",
            "meta.function.rust",
            "text.html",
            "invalid.illegal",
        ] {
            assert_eq!(role_for_scope(scope), None, "{scope}");
        }
    }

    #[test]
    fn a_prefix_must_end_on_a_dot() {
        assert_eq!(role_for_scope("stringy.thing"), None);
        assert_eq!(role_for_scope("keywordish"), None);
        assert_eq!(role_for_scope("string."), Some(Role::SyntaxString));
    }

    #[test]
    fn the_innermost_scope_with_a_role_decides() {
        let stack = [
            "source.rust",
            "meta.function.rust",
            "string.quoted.double",
            "meta.x",
        ];
        assert_eq!(role_for_stack(stack.into_iter()), Some(Role::SyntaxString));
        let none = ["source.rust", "meta.block.rust"];
        assert_eq!(role_for_stack(none.into_iter()), None);
        assert_eq!(role_for_stack([].into_iter()), None);
    }

    #[test]
    fn the_table_has_no_duplicates_and_reaches_every_syntax_role() {
        for (i, (prefix, _)) in TABLE.iter().enumerate() {
            assert!(TABLE[i + 1..].iter().all(|(p, _)| p != prefix), "{prefix}");
            assert!(!prefix.is_empty() && !prefix.ends_with('.'), "{prefix}");
        }
        for role in Role::SYNTAX {
            assert!(
                TABLE.iter().any(|&(_, r)| r == role),
                "{role:?} is unreachable"
            );
        }
    }
}
