//! Reference extraction for `smelt.<path>` forms.
//!
//! Phase 4 of the smelt-`<path>` migration removes the `smelt.ref` and
//! `smelt.source` legacy syntax entirely. Phase 5b removes `smelt.fn.*`
//! entirely; only the unified `smelt.<path>` form remains.
//!
//! The unifying enum is [`SmeltRef`]:
//! - [`SmeltRef::Path`] — the unified `smelt.<path>` form. Already a
//!   path tuple; resolution dispatches on file format/content.

use rowan::TextRange;
use smelt_parser::ast::{File as AstFile, SmeltPathCall, SmeltPathRef, TableRef};

/// A unified ref carrier. Every AST node is adapted into one of
/// these at the boundary; downstream code is keyed on path tuples.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SmeltRef {
    /// Path-form: `smelt.<seg>(.<seg>)*`. Already in the unified shape.
    Path(Vec<String>),
}

impl SmeltRef {
    /// Adapt this ref to the unified path tuple.
    ///
    /// - [`SmeltRef::Path`] returns its segments unchanged.
    pub fn to_path(&self) -> Vec<String> {
        match self {
            SmeltRef::Path(segs) => segs.clone(),
        }
    }

    /// Display-friendly leaf name for diagnostics. The last segment.
    pub fn leaf_name(&self) -> String {
        match self {
            SmeltRef::Path(segs) => segs.last().cloned().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub has_named_params: bool,
    pub range: TextRange,
    /// Unified ref carrier. Callers derive the leaf name via
    /// `smelt_ref.leaf_name()` and the full dot-path via
    /// `smelt_ref.to_path().join(".")`.
    pub smelt_ref: SmeltRef,
}

/// Extract every smelt-extension ref appearing in a parsed file —
/// unified path forms only. Path-form call sites are also detected
/// (path calls with arg lists), but only the value-form refs (no `(`)
/// are surfaced as model dependencies.
///
/// Note: `smelt.ref()`, `smelt.source()`, and `smelt.fn.*` are parse
/// errors and will not appear in a valid CST.
pub fn extract_refs(file: &AstFile) -> Vec<RefInfo> {
    let mut out = Vec::new();
    for node in file.syntax().descendants() {
        // Unified path-form value refs (FROM smelt.models.foo).
        if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
            // Check whether this path ref is nested inside a SMELT_PATH_CALL.
            let enclosing_call = node.ancestors().skip(1).find_map(SmeltPathCall::cast);

            if let Some(_call) = enclosing_call {
                // Path refs inside a smelt.functions.* call are real model
                // dependencies (e.g. `source => smelt.silver.events_parsed`).
                // Include them so the logical graph can order the build correctly.
                // Only include if the path ref is inside an ARG_LIST (argument
                // position), not in the path prefix of the call itself.
                let in_arg_list = node
                    .ancestors()
                    .skip(1)
                    .take(4)
                    .any(|a| a.kind() == smelt_parser::SyntaxKind::ARG_LIST);
                if !in_arg_list {
                    continue;
                }
                // Fall through: add as a dependency.
            }

            let segments = path_ref.segments();
            out.push(RefInfo {
                has_named_params: false,
                range: path_ref.text_range(),
                smelt_ref: SmeltRef::Path(segments),
            });
            continue;
        }

        // Unified path-form call refs (smelt.functions.x.y(args)).
        if let Some(path_call) = SmeltPathCall::cast(node.clone()) {
            let segments = path_call.segments();
            let has_named_params = path_call
                .arg_list()
                .map(|al| {
                    al.syntax()
                        .descendants()
                        .any(|n| n.kind() == smelt_parser::SyntaxKind::NAMED_PARAM)
                })
                .unwrap_or(false);
            out.push(RefInfo {
                has_named_params,
                range: path_call.text_range(),
                smelt_ref: SmeltRef::Path(segments),
            });
            continue;
        }
    }
    out
}

/// Helper used by `lib.rs` re-exports: extract refs from a `TableRef`
/// (the FROM-clause node), returning a single `SmeltRef` if any is
/// present. Used by callers that need to peek at one position rather
/// than walk the whole file.
pub fn ref_from_table_ref(table_ref: &TableRef) -> Option<SmeltRef> {
    if let Some(path_ref) = table_ref.smelt_path_ref() {
        return Some(SmeltRef::Path(path_ref.segments()));
    }
    if let Some(path_call) = table_ref.smelt_path_call() {
        return Some(SmeltRef::Path(path_call.segments()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_refs_path_form() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.models.raw_events
GROUP BY user_id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].smelt_ref.to_path().join("."), "models.raw_events");
        assert!(!refs[0].has_named_params);
        assert!(matches!(refs[0].smelt_ref, SmeltRef::Path(_)));
    }

    #[test]
    fn test_extract_refs_path_form_multiple() {
        let sql = r#"
SELECT
    a.user_id,
    b.session_id
FROM smelt.models.model_a a
INNER JOIN smelt.models.model_b b ON a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].smelt_ref.to_path().join("."), "models.model_a");
        assert_eq!(refs[1].smelt_ref.to_path().join("."), "models.model_b");
    }

    #[test]
    fn path_to_path_returns_segments() {
        let path = SmeltRef::Path(vec![
            "models".to_string(),
            "marts".to_string(),
            "foo".to_string(),
        ])
        .to_path();
        assert_eq!(path, vec!["models", "marts", "foo"]);
    }
}
