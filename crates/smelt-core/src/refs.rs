//! Reference extraction for `smelt.<path>` and the legacy `smelt.ref` /
//! `smelt.source` / `smelt.fn.<path>` forms.
//!
//! Phase 2a of the smelt-`<path>` migration moves the data plane onto a
//! single internal key — the workspace-relative path tuple
//! (`Vec<String>`). Legacy AST nodes still produced by Phase 1 are
//! adapted into the same key here so downstream code (`DependencyGraph`,
//! `smelt-db` resolvers) sees one shape regardless of which source
//! syntax the user wrote.
//!
//! The unifying enum is [`SmeltRef`]:
//! - [`SmeltRef::Path`] — the unified `smelt.<path>` form. Already a
//!   path tuple; resolution dispatches on file format/content.
//! - [`SmeltRef::LegacyRef`] — `smelt.ref('name')`. Adapted to a path
//!   tuple by [`SmeltRef::to_path`] using a [`PathLocator`] hint
//!   (typically the workspace's discovered models).
//! - [`SmeltRef::LegacySource`] — `smelt.source('schema.table')`.
//!   Adapted to `["sources", "<schema>", "<table>"]` (matching the
//!   recommended layout under `sources/<schema>/<table>.yml`).
//! - [`SmeltRef::LegacyFn`] — `smelt.fn.<seg>(.<seg>)*`. Adapted to
//!   `["functions", <segments>...]`.

use rowan::TextRange;
use smelt_parser::ast::{
    File as AstFile, FunctionCall, RefCall, SmeltFnCall, SmeltPathCall, SmeltPathRef, SourceCall,
    TableRef,
};

/// A unified ref carrier. Every legacy AST node is adapted into one of
/// these at the boundary; downstream code is keyed on path tuples.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SmeltRef {
    /// Path-form: `smelt.<seg>(.<seg>)*`. Already in the unified shape.
    Path(Vec<String>),
    /// Legacy `smelt.ref('name')`.
    LegacyRef(String),
    /// Legacy `smelt.source('schema.table')`.
    LegacySource(String),
    /// Legacy `smelt.fn.<seg>(.<seg>)*` call. Segments after the
    /// `smelt.fn.` prefix.
    LegacyFn(Vec<String>),
}

/// Maps a bare model name to a workspace-relative path tuple. Used by
/// [`SmeltRef::to_path`] to adapt legacy `smelt.ref('name')` into the
/// unified key. Implementations typically wrap a `Vec<ModelFile>`.
pub trait PathLocator {
    fn locate_model(&self, name: &str) -> Option<Vec<String>>;
}

/// `PathLocator` that always returns `None`. Used in tests / shape-only
/// extraction paths where no workspace context is available.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLocator;

impl PathLocator for NoLocator {
    fn locate_model(&self, _name: &str) -> Option<Vec<String>> {
        None
    }
}

impl SmeltRef {
    /// Adapt this ref to the unified path tuple.
    ///
    /// - [`SmeltRef::Path`] returns its segments unchanged.
    /// - [`SmeltRef::LegacyRef`] consults `locator` to find the model;
    ///   falls back to `["models", <name>]` if the locator has no
    ///   answer (preserves the old "look in models/" default).
    /// - [`SmeltRef::LegacySource`] returns `["sources", parts...]`.
    /// - [`SmeltRef::LegacyFn`] returns `["functions", segments...]`.
    pub fn to_path<L: PathLocator + ?Sized>(&self, locator: &L) -> Vec<String> {
        match self {
            SmeltRef::Path(segs) => segs.clone(),
            SmeltRef::LegacyRef(name) => locator
                .locate_model(name)
                .unwrap_or_else(|| vec!["models".to_string(), name.clone()]),
            SmeltRef::LegacySource(qualified) => {
                let mut out = vec!["sources".to_string()];
                out.extend(qualified.split('.').map(|s| s.to_string()));
                out
            }
            SmeltRef::LegacyFn(segs) => {
                let mut out = vec!["functions".to_string()];
                out.extend(segs.iter().cloned());
                out
            }
        }
    }

    /// Display-friendly leaf name for diagnostics. For path forms the
    /// last segment; for legacy refs the model/source name; for legacy
    /// fn calls the last segment.
    pub fn leaf_name(&self) -> String {
        match self {
            SmeltRef::Path(segs) => segs.last().cloned().unwrap_or_default(),
            SmeltRef::LegacyRef(name) => name.clone(),
            SmeltRef::LegacySource(qual) => qual.clone(),
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
/// unified path forms and legacy `smelt.ref` / `smelt.source` /
/// `smelt.fn.*` forms. Path-form callsites are also detected (path
/// calls with arg lists), but only the value-form refs (no `(`) and
/// legacy refs are surfaced as model dependencies. Legacy
/// `smelt.source` callsites are intentionally **not** included here so
/// the existing string-keyed dependency graph (which only tracks model
/// dependencies, with sources looked up separately via `sources.yml`)
/// keeps its current shape — Phase 2a's path-tuple graph adds them
/// uniformly.
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

        // Legacy `smelt.ref('name')`.
        if let Some(func) = FunctionCall::cast(node.clone()) {
            if let Some(ref_call) = RefCall::from_function_call(func.clone()) {
                if let Some(name) = ref_call.model_name() {
                    let has_params = ref_call.named_params().count() > 0;
                    out.push(RefInfo {
                        model_name: name.clone(),
                        has_named_params: has_params,
                        range: ref_call.range(),
                        smelt_ref: SmeltRef::LegacyRef(name),
                    });
                    continue;
                }
            }
            // We do NOT register `smelt.source(...)` as a model ref —
            // sources are looked up via the project's `sources.yml`.
            // The path-tuple builder consumes them separately.
            let _ = SourceCall::from_function_call(func);
            continue;
        }

        // Legacy `smelt.fn.<path>(...)` calls. We register them so
        // path-tuple consumers (Phase 2a graph) can see function
        // dependencies. The string-keyed legacy graph filters them out
        // because legacy fn calls are not models.
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
    if let Some(func) = table_ref.function_call() {
        if let Some(ref_call) = RefCall::from_function_call(func.clone()) {
            return Some(SmeltRef::LegacyRef(ref_call.model_name()?));
        }
        if let Some(src_call) = SourceCall::from_function_call(func) {
            return Some(SmeltRef::LegacySource(src_call.qualified_name()?));
        }
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
    fn test_extract_refs_legacy() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.ref('raw_events')
GROUP BY user_id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].model_name, "raw_events");
        assert!(!refs[0].has_named_params);
        assert!(matches!(refs[0].smelt_ref, SmeltRef::LegacyRef(_)));
    }

    #[test]
    fn test_extract_refs_with_named_params() {
        let sql = r#"
SELECT user_id
FROM smelt.ref('raw_events', filter => event_type = 'page_view')
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].model_name, "raw_events");
        assert!(refs[0].has_named_params);
    }

    #[test]
    fn test_multiple_legacy_refs() {
        let sql = r#"
SELECT
    a.user_id,
    b.session_id
FROM smelt.ref('model_a') a
INNER JOIN smelt.ref('model_b') b ON a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
    }

    #[test]
    fn legacy_ref_to_path_uses_locator() {
        struct LocateTo<'a>(&'a [(&'a str, Vec<&'a str>)]);
        impl<'a> PathLocator for LocateTo<'a> {
            fn locate_model(&self, name: &str) -> Option<Vec<String>> {
                self.0.iter().find_map(|(n, segs)| {
                    if *n == name {
                        Some(segs.iter().map(|s| s.to_string()).collect())
                    } else {
                        None
                    }
                })
            }
        }
        let locator = LocateTo(&[("foo", vec!["models", "marts", "foo"])]);
        let path = SmeltRef::LegacyRef("foo".to_string()).to_path(&locator);
        assert_eq!(path, vec!["models", "marts", "foo"]);

        // Fallback: if locator has no answer, default to ["models", name].
        let path = SmeltRef::LegacyRef("bar".to_string()).to_path(&NoLocator);
        assert_eq!(path, vec!["models", "bar"]);
    }

    #[test]
    fn legacy_source_to_path() {
        let path = SmeltRef::LegacySource("raw.events".to_string()).to_path(&NoLocator);
        assert_eq!(path, vec!["sources", "raw", "events"]);
    }

    #[test]
    fn legacy_fn_to_path() {
        let path = SmeltRef::LegacyFn(vec!["core".to_string(), "safe_divide".to_string()])
            .to_path(&NoLocator);
        assert_eq!(path, vec!["functions", "core", "safe_divide"]);
    }
}
