//! Reference extraction for `smelt.<path>` and the legacy `smelt.fn.<path>`
//! form.
//!
//! Phase 4 of the smelt-`<path>` migration removes the `smelt.ref` and
//! `smelt.source` legacy syntax entirely. The parser now rejects those forms.
//! Only `smelt.fn.*` legacy calls remain (deferred until function diagnostics
//! are ported to `SmeltPathCall`).
//!
//! The unifying enum is [`SmeltRef`]:
//! - [`SmeltRef::Path`] — the unified `smelt.<path>` form. Already a
//!   path tuple; resolution dispatches on file format/content.
//! - [`SmeltRef::LegacyFn`] — `smelt.fn.<seg>(.<seg>)*`. Adapted to
//!   `["functions", <segments>...]`.

use rowan::TextRange;
use smelt_parser::ast::{File as AstFile, SmeltFnCall, SmeltPathCall, SmeltPathRef, TableRef};

/// A unified ref carrier. Every legacy AST node is adapted into one of
/// these at the boundary; downstream code is keyed on path tuples.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SmeltRef {
    /// Path-form: `smelt.<seg>(.<seg>)*`. Already in the unified shape.
    Path(Vec<String>),
    /// Legacy `smelt.fn.<seg>(.<seg>)*` call. Segments after the
    /// `smelt.fn.` prefix. Kept until function diagnostics are ported
    /// to `SmeltPathCall`.
    LegacyFn(Vec<String>),
}

impl SmeltRef {
    /// Adapt this ref to the unified path tuple.
    ///
    /// - [`SmeltRef::Path`] returns its segments unchanged.
    /// - [`SmeltRef::LegacyFn`] returns `["functions", segments...]`.
    pub fn to_path(&self) -> Vec<String> {
        match self {
            SmeltRef::Path(segs) => segs.clone(),
            SmeltRef::LegacyFn(segs) => {
                let mut out = vec!["functions".to_string()];
                out.extend(segs.iter().cloned());
                out
            }
        }
    }

    /// Display-friendly leaf name for diagnostics. For path forms the
    /// last segment; for legacy fn calls the last segment.
    pub fn leaf_name(&self) -> String {
        match self {
            SmeltRef::Path(segs) => segs.last().cloned().unwrap_or_default(),
            SmeltRef::LegacyFn(segs) => segs.last().cloned().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    /// Legacy field — preserved for backwards compatibility with the
    /// existing string-keyed `DependencyGraph::build` code path. New
    /// callers should consume [`smelt_ref`](RefInfo::smelt_ref).
    pub model_name: String,
    pub has_named_params: bool,
    pub range: TextRange,
    /// Unified ref carrier (Phase 2a).
    pub smelt_ref: SmeltRef,
}

/// Extract every smelt-extension ref appearing in a parsed file —
/// unified path forms and legacy `smelt.fn.*` calls. Path-form call
/// sites are also detected (path calls with arg lists), but only the
/// value-form refs (no `(`) are surfaced as model dependencies.
///
/// Note: `smelt.ref()` and `smelt.source()` are parse errors in Phase 4
/// and will not appear in a valid CST.
pub fn extract_refs(file: &AstFile) -> Vec<RefInfo> {
    let mut out = Vec::new();
    for node in file.syntax().descendants() {
        // Unified path-form value refs (FROM smelt.models.foo).
        if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
            // Skip path refs nested inside a SMELT_PATH_CALL — those are
            // not "value form" refs.
            if node
                .ancestors()
                .skip(1)
                .any(|a| SmeltPathCall::cast(a).is_some())
            {
                continue;
            }
            let segments = path_ref.segments();
            let leaf = segments.last().cloned().unwrap_or_default();
            out.push(RefInfo {
                model_name: leaf,
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
            let leaf = segments.last().cloned().unwrap_or_default();
            out.push(RefInfo {
                model_name: leaf,
                has_named_params,
                range: path_call.text_range(),
                smelt_ref: SmeltRef::Path(segments),
            });
            continue;
        }

        // Legacy `smelt.fn.<path>(...)` calls. We register them so
        // path-tuple consumers can see function dependencies.
        if let Some(fn_call) = SmeltFnCall::cast(node.clone()) {
            let segments = fn_call
                .call_path()
                .map(|p| p.segments())
                .unwrap_or_default();
            let leaf = segments.last().cloned().unwrap_or_default();
            out.push(RefInfo {
                model_name: leaf,
                has_named_params: false,
                range: fn_call.syntax().text_range(),
                smelt_ref: SmeltRef::LegacyFn(segments),
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
    if let Some(fn_call) = table_ref.smelt_fn_call() {
        let segments = fn_call
            .call_path()
            .map(|p| p.segments())
            .unwrap_or_default();
        return Some(SmeltRef::LegacyFn(segments));
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
        assert_eq!(refs[0].model_name, "raw_events");
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
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
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

    #[test]
    fn legacy_fn_to_path() {
        let path =
            SmeltRef::LegacyFn(vec!["core".to_string(), "safe_divide".to_string()]).to_path();
        assert_eq!(path, vec!["functions", "core", "safe_divide"]);
    }
}
