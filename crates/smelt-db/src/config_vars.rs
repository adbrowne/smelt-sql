//! Phase B (meta-language) — pure `smelt.yml` `vars:` resolver.
//!
//! This module provides:
//! - [`parse_vars_from_yaml`]: extract the `vars:` mapping from raw `smelt.yml` text.
//! - [`coerce_yaml_scalar_to_text`]: coerce a YAML scalar to a `Text` string, with
//!   a `ConfigVarNullCoercion` sentinel for `null` values.
//! - [`is_string_literal_expr`]: detect whether a CST `Expr` node is a bare string literal.
//!
//! Pure-function rule (CLAUDE.md): this module contains no Salsa imports. All Salsa
//! wrappers live in `lib.rs` (`smelt_yml_vars_query`).

use std::collections::BTreeMap;

use smelt_parser::SyntaxKind;

/// Parse the `vars:` block from raw `smelt.yml` text.
///
/// Returns a `BTreeMap<String, serde_yaml::Value>` with one entry per key in
/// the `vars:` block, or `None` when the YAML cannot be parsed or has no `vars:`
/// key.
///
/// Pure function — does not touch the filesystem.
pub fn parse_vars_from_yaml(text: &str) -> Option<BTreeMap<String, serde_yaml::Value>> {
    if text.is_empty() {
        return Some(BTreeMap::new());
    }
    let value: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let map = value.as_mapping()?;
    let vars_key = serde_yaml::Value::String("vars".to_string());
    let vars_value = map.get(&vars_key)?;
    let vars_mapping = vars_value.as_mapping()?;

    let mut result = BTreeMap::new();
    for (k, v) in vars_mapping {
        if let Some(key_str) = k.as_str() {
            result.insert(key_str.to_string(), v.clone());
        }
    }
    Some(result)
}

/// Coerce a YAML scalar value to a `Text` string.
///
/// Returns `(text_value, warning_name_or_none)`:
/// - Strings: returned as-is.
/// - Booleans: rendered as `"true"` or `"false"`.
/// - Integers and floats: rendered as their decimal representation.
/// - `null`: rendered as `""` and the second element is `Some(name.to_string())`,
///   which the caller should convert to a `ConfigVarNullCoercion` warning.
/// - Other types (sequences, mappings): rendered as `"<complex>"` — these are
///   edge cases that callers can treat as errors or warn.
///
/// Pure function — no Salsa dependency.
pub fn coerce_yaml_scalar_to_text(v: &serde_yaml::Value, name: &str) -> (String, Option<String>) {
    match v {
        serde_yaml::Value::String(s) => (s.clone(), None),
        serde_yaml::Value::Bool(b) => (b.to_string(), None),
        serde_yaml::Value::Number(n) => (n.to_string(), None),
        serde_yaml::Value::Null => (
            String::new(),
            Some(name.to_string()), // sentinel for ConfigVarNullCoercion
        ),
        // Sequences and mappings are not scalars. Render as empty with a warning.
        serde_yaml::Value::Sequence(_) | serde_yaml::Value::Mapping(_) => {
            (String::new(), Some(name.to_string()))
        }
        // Tagged values (rare)
        serde_yaml::Value::Tagged(_) => (String::new(), Some(name.to_string())),
    }
}

/// Returns `true` when `expr` is a bare string literal (a single `STRING` token
/// or an `EXPRESSION` wrapping a single `STRING` token).
///
/// Used to distinguish `smelt.config.var('x')` (literal — valid) from
/// `smelt.config.var(some_var)` (non-literal — emits `ConfigVarNameNotLiteral`).
///
/// Pure function — no Salsa dependency.
pub fn is_string_literal_expr(expr: &smelt_parser::ast::Expr) -> bool {
    // The expression node may itself be a STRING token, or wrap one in an
    // EXPRESSION node. Walk the children to find a STRING token.
    let syntax = expr.syntax();

    // Direct STRING token?
    for child in syntax.children_with_tokens() {
        if let rowan::NodeOrToken::Token(t) = child {
            if t.kind() == SyntaxKind::STRING {
                return true;
            }
        }
    }

    // Wrapped EXPRESSION node containing a STRING?
    for child_node in syntax.children() {
        for inner in child_node.children_with_tokens() {
            if let rowan::NodeOrToken::Token(t) = inner {
                if t.kind() == SyntaxKind::STRING {
                    return true;
                }
            }
        }
    }

    false
}

/// Extract the string value from a string-literal `Expr` node.
///
/// Returns the unquoted string content, or `None` if the expression is not a
/// string literal. The returned string does NOT include the surrounding quotes.
///
/// Pure function — no Salsa dependency.
pub fn extract_string_literal_value(expr: &smelt_parser::ast::Expr) -> Option<String> {
    let syntax = expr.syntax();

    // Helper to strip surrounding single or double quotes.
    let strip_quotes = |s: &str| -> String {
        if s.len() >= 2 {
            let inner = &s[1..s.len() - 1];
            // Unescape doubled quotes (SQL standard).
            if s.starts_with('\'') {
                inner.replace("''", "'")
            } else {
                inner.replace("\"\"", "\"")
            }
        } else {
            s.to_string()
        }
    };

    // Direct STRING token?
    for child in syntax.children_with_tokens() {
        if let rowan::NodeOrToken::Token(t) = child {
            if t.kind() == SyntaxKind::STRING {
                return Some(strip_quotes(t.text()));
            }
        }
    }

    // Wrapped EXPRESSION node?
    for child_node in syntax.children() {
        for inner in child_node.children_with_tokens() {
            if let rowan::NodeOrToken::Token(t) = inner {
                if t.kind() == SyntaxKind::STRING {
                    return Some(strip_quotes(t.text()));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vars_empty_text() {
        let result = parse_vars_from_yaml("");
        assert!(
            result.as_ref().map(|m| m.is_empty()).unwrap_or(false),
            "empty text must return empty vars map, got: {:?}",
            result
        );
    }

    #[test]
    fn parse_vars_no_vars_key() {
        let yaml = "name: my_project\ntargets: {}\n";
        let result = parse_vars_from_yaml(yaml);
        // No `vars:` key → returns None.
        assert!(
            result.is_none(),
            "YAML without vars: must return None, got: {:?}",
            result
        );
    }

    #[test]
    fn parse_vars_basic() {
        let yaml = "name: my_project\nvars:\n  region: us-west-2\n  debug: false\ntargets: {}\n";
        let vars = parse_vars_from_yaml(yaml).expect("must parse");
        assert_eq!(vars.len(), 2);
        assert!(vars.contains_key("region"));
        assert!(vars.contains_key("debug"));
    }

    #[test]
    fn coerce_string_scalar() {
        let v = serde_yaml::Value::String("hello".to_string());
        let (text, warn) = coerce_yaml_scalar_to_text(&v, "x");
        assert_eq!(text, "hello");
        assert!(warn.is_none());
    }

    #[test]
    fn coerce_bool_scalar() {
        let v = serde_yaml::Value::Bool(true);
        let (text, warn) = coerce_yaml_scalar_to_text(&v, "x");
        assert_eq!(text, "true");
        assert!(warn.is_none());
    }

    #[test]
    fn coerce_number_scalar() {
        let v: serde_yaml::Value = serde_yaml::from_str("42").unwrap();
        let (text, warn) = coerce_yaml_scalar_to_text(&v, "x");
        assert_eq!(text, "42");
        assert!(warn.is_none());
    }

    #[test]
    fn coerce_null_warns() {
        let v = serde_yaml::Value::Null;
        let (text, warn) = coerce_yaml_scalar_to_text(&v, "nullable");
        assert_eq!(text, "");
        assert_eq!(warn, Some("nullable".to_string()));
    }
}
