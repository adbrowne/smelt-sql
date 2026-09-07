use super::*;
use smelt_parser::TextRange;
use std::path::PathBuf;

/// Pre-resolution context binding attached to an `Expr<T, ctx>` /
/// `AggExpr<T, ctx>` / `WindowExpr<T, ctx>` parameter (Phase 19).
///
/// Stores the raw identifier written by the user (e.g. `"source"`). Phase 20
/// will add a post-resolution pointer to the actual parameter or CTE that
/// `ctx` names. For now this is just a newtype around `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRef(pub String);

impl ContextRef {
    /// The raw name written in the `Expr<T, ctx>` annotation.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// Description of a single parameter in a `smelt.define`.
///
/// `type_ref_text` is the raw source text of the `TypeRef` node (e.g.
/// `"Expr<Numeric>"`) or `None` when the parameter is unannotated. Phase 4
/// adds a parsed `type_ref` alongside it; callers that need the structured
/// form should prefer that field. `type_ref_text` stays in place (Option A
/// from the plan) so downstream integration tests that read the raw text
/// don't need to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The parameter's declared name.
    pub name: String,
    /// Source range of the parameter-name identifier, suitable for anchoring
    /// diagnostics (notably `DuplicateParameterName` in Phase 5). `None` only
    /// if the declaration was so malformed that no IDENT token was present in
    /// the PARAM node.
    pub name_range: Option<TextRange>,
    /// Raw text of the declared type, or `None` if unannotated.
    pub type_ref_text: Option<String>,
    /// Structured parse of `type_ref_text`, or `None` if unannotated.
    /// `Some(Err(...))` when an annotation was written but couldn't be parsed
    /// — these errors are surfaced as diagnostics by higher layers.
    pub type_ref: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the `TypeRef` node for this parameter, suitable for
    /// anchoring diagnostic spans. `None` when the parameter has no
    /// annotation at all.
    pub type_ref_range: Option<TextRange>,
    /// `true` when the parameter has a default value.
    pub has_default: bool,
    /// Pre-resolution context binding from `Expr<T, ctx>` / `AggExpr<T, ctx>`
    /// / `WindowExpr<T, ctx>` annotations (Phase 19). `None` when no context
    /// argument is present.
    pub context: Option<ContextRef>,
    /// `true` when the parameter carries a `NOT NULL` qualifier (Phase 5,
    /// nullability-soundness). A non-nullable parameter requires a non-nullable
    /// argument at the call site and binds the parameter as non-nullable in the
    /// function body. Bare annotations (without `NOT NULL`) are always nullable.
    pub not_null: bool,
}

/// A single frame of expansion context attached to a body/call-site
/// diagnostic.
///
/// Phase 6 populated a 0-or-1-element `Vec<FrameInfo>` on every
/// `smelt.fn.*`-originated diagnostic. Phase 12 extends the checker to
/// stack frames at each level of recursive body expansion, producing
/// multi-level diagnostics:
///
/// - When the body's own type-check surfaces an error (no expansion
///   happened), the vec is empty.
/// - When the error fires *inside* an expanded body, the vec contains
///   one frame per nested expansion, **innermost-first →
///   outermost-last**. `frames.last()` is the outermost call site
///   (the one the user wrote in their source); `frames.first()` is the
///   deepest nested call. The renderer in `smelt-lsp` reverses this
///   iteration to present frames outer-to-inner in the message body.
///
/// `bound_type` is the concrete type that the parameter was bound to at the
/// call site, rendered via `DataType::to_string()` for display.
///
/// Phase 12 also adds decl-site and call-site location data so LSP
/// clients can surface each frame as a `DiagnosticRelatedInformation`
/// pointing at the declaring file / call-path. All three location fields
/// are `Option` — older frames (or sig-lookup misses during degraded
/// scenarios) simply carry `None` and the LSP renderer falls back to
/// inline-only messaging for those frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    /// Name of the function whose expansion is responsible for this frame
    /// (e.g. `"safe_divide"`).
    pub function: String,
    /// Name of the parameter inside that function whose binding produced
    /// the inner error (e.g. `"numerator"`).
    pub param: String,
    /// Textual rendering of the type that `param` was bound to at this
    /// call site (e.g. `"INTEGER"`, `"VARCHAR"`).
    pub bound_type: String,
    /// Path to the file that declares the function (`decl_path`). `None`
    /// when the declaration couldn't be located (e.g. sig-lookup miss
    /// during degraded scenarios).
    pub decl_path: Option<PathBuf>,
    /// Range of the `DEFINE_NAME` / `EXTERN_NAME` identifier in
    /// `decl_path`. Used by LSP clients to open the declaration site
    /// when a user clicks the frame's related-information link.
    pub decl_range: Option<TextRange>,
    /// Range of the call-path span at this frame's call site (in the
    /// file that *contains* the call — distinct from `decl_path`, which
    /// points at where the callee is defined).
    pub call_site_range: Option<TextRange>,
    /// Identifier of the declaring function in the function registry.
    /// `None` for anonymous frames (e.g. HOF inline-expansion frames produced
    /// by `map`, `filter`, `reduce`). Named `smelt.define` frames carry `Some`.
    pub fn_id: Option<String>,
    /// Zero-based index into the source list literal at the HOF call site,
    /// identifying which element the expanded lambda body was operating on.
    /// `None` when the source list was not a literal or the information is
    /// not statically available (the common v1 case).
    pub element_index: Option<usize>,
    /// Phase C (meta-language) extension: the source span of the column's
    /// declaration in the upstream `ModelSchema`, when the HOF source list
    /// came from `smelt.columns_of(t)`. `None` for literal-sourced lists,
    /// unresolvable schemas, or non-`columns_of` HOF sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub column_origin: Option<smelt_parser::TextRange>,
    /// Wide-reflection extension: source-model provenance for HOF frames whose
    /// source list came from `smelt.models.*`. Carries the model's workspace-
    /// relative path and the frontmatter declaration span when statically
    /// traceable. `None` for all other HOF frame sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub model_origin: Option<ModelOrigin>,
    /// Wide-reflection extension: source-yaml provenance for HOF frames whose
    /// source list came from `smelt.sources.*`. Carries the source YAML's
    /// workspace-relative path and the YAML declaration span when statically
    /// traceable. `None` for all other HOF frame sources.
    ///
    /// Producer-side only in v1 — the LSP renderer does not yet surface this
    /// field (tracked as a Known Divergence in `expansion.md`).
    pub source_origin: Option<SourceOrigin>,
}

/// Phase C (meta-language): concrete value produced by `smelt.columns_of`.
///
/// Each element of the `List<ColumnRef>` returned by `smelt.columns_of(t)`
/// is one `ColumnRefValue`. The list preserves the source schema's declared
/// column order (Phase C spec §"ColumnRef ordering").
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa query
/// `columns_of_for_table_expr`) and any future consumer (e.g. the planner)
/// can use it without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRefValue {
    /// The column's declared name (identifier text from the source schema).
    pub name: String,
    /// The column's declared data type (from `TypedColumn::data_type`).
    /// `None` when the column's type was not statically known (e.g. no
    /// type annotation and inference could not determine it).
    pub data_type: Option<crate::DataType>,
    /// Whether the column's type satisfies the `Numeric` constraint —
    /// `DataType::is_numeric()` per `types.md` §"Type constraints".
    /// `false` when `data_type` is `None`.
    pub is_numeric: bool,
    /// Source span of this column's declaration in the upstream `ModelSchema`
    /// (the `Column::range` field). `None` when the span is not statically
    /// resolvable (e.g. source came from an external YAML with no SQL range).
    pub source_span: Option<smelt_parser::TextRange>,
}

/// Source-model provenance attached to a wide-reflection HOF frame
/// (Phase D, `smelt.models.*`-sourced lists).
///
/// Captures the model's workspace-relative path and the frontmatter
/// declaration span (if statically resolvable). The producer stamps this
/// on each per-element anonymous HOF frame when the source list comes from
/// `smelt.models.with_tag` or `smelt.models.all`.
///
/// Producer-side only in v1 — the LSP renderer does not yet surface this
/// field (tracked as a Known Divergence in `expansion.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOrigin {
    /// Workspace-relative path of the model's source `.sql` file, with `/`
    /// separators. This is the same path used in `ModelRefValue::path`.
    pub path: String,
    /// Source span of the model's frontmatter block (if present and parseable).
    /// `None` when the model has no frontmatter or when the span is otherwise
    /// not statically resolvable.
    pub frontmatter_span: Option<smelt_parser::TextRange>,
}

/// Source-yaml provenance attached to a wide-reflection HOF frame
/// (Phase D, `smelt.sources.*`-sourced lists).
///
/// Captures the source YAML file's workspace-relative path and the
/// YAML declaration span (if statically resolvable). The producer stamps
/// this on each per-element anonymous HOF frame when the source list comes
/// from `smelt.sources.with_tag` or `smelt.sources.all`.
///
/// Producer-side only in v1 — the LSP renderer does not yet surface this
/// field (tracked as a Known Divergence in `expansion.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOrigin {
    /// Workspace-relative path of the source's YAML file, with `/` separators.
    /// This is the same path used in `SourceRefValue::path`.
    pub path: String,
    /// Source span of the YAML declaration (currently always `None` in v1
    /// since source YAMLs are not tracked by the Rowan CST — reserved for
    /// a future pass that parses span information from the YAML).
    pub declaration_span: Option<smelt_parser::TextRange>,
}

/// Concrete value produced by `smelt.models.with_tag` / `smelt.models.all`
/// at expansion time (Phase D, wide reflection).
///
/// Each element of the `List<ModelRef>` returned by those accessors is
/// one `ModelRefValue`. The list is sorted ascending by `path`
/// (byte-lexicographic on the workspace-relative path with `/` separators).
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa queries
/// `models_with_tag` / `models_all`) and any future consumer can use it
/// without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRefValue {
    /// Workspace-relative file path with `/` separators (e.g.
    /// `"models/orders.sql"`).
    pub path: String,
    /// Model name — the final path segment without the `.sql` extension
    /// (e.g. `"orders"`).
    pub name: String,
    /// Merged tag set: union of `smelt.yml` `models.<name>.tags` and SQL
    /// frontmatter `tags:`, deduplicated by `Config::get_tags`. In the
    /// order returned by that function (smelt.yml tags first, then
    /// frontmatter tags not already present).
    pub tags: Vec<String>,
    /// The model name used to route `m.columns` through `columns_of_for_table_expr`.
    /// This is the model's short name (same as `name`) — the Salsa query
    /// accepts this to resolve the column list at expansion time.
    pub model_name_for_columns: String,
}

/// Concrete value produced by `smelt.sources.with_tag` / `smelt.sources.all`
/// at expansion time (Phase D, wide reflection).
///
/// Each element of the `List<SourceRef>` returned by those accessors is
/// one `SourceRefValue`. The list is sorted ascending by `path`
/// (byte-lexicographic on the workspace-relative path with `/` separators).
///
/// This struct is pure (no Salsa dependency) and lives in `smelt-types` so
/// that both `smelt-db` (which produces the list via the Salsa queries
/// `sources_with_tag` / `sources_all`) and any future consumer can use it
/// without gaining a Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefValue {
    /// Workspace-relative file path of the source YAML with `/` separators
    /// (e.g. `"models/sources/raw/users.yml"`).
    pub path: String,
    /// Source name — the final path segment without the `.yml` / `.yaml`
    /// extension (e.g. `"users"`).
    pub name: String,
    /// Tag set as declared in the source YAML's `tags:` list. No merge with
    /// a second source (source YAMLs are the single source of tag truth for
    /// sources).
    pub tags: Vec<String>,
    /// Address segments for routing `s.columns` through the source resolution
    /// machinery. These are the `address_segments` from `SourceInfo`.
    pub address_segments: Vec<String>,
}

/// Tier of a function, derived from annotation completeness.
///
/// - `Tier::Three`: every parameter annotated AND return type annotated.
/// - `Tier::Two`: every parameter annotated, return type missing.
/// - `Tier::One`: at least one parameter unannotated (return-type state
///   irrelevant for this tier — unannotated inputs force Tier 1).
///
/// Behavioural differences between tiers (bidirectional checking, param
/// coercion, etc.) land in Steps 5+.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    One,
    Two,
    Three,
}

/// Origin of a user-declared function signature.
///
/// Phase 10 introduces `smelt.extern` declarations alongside the existing
/// `smelt.define`. Externs have no body (they bind a name to a
/// backend-provided function), but otherwise share the same indexing,
/// resolution, and type-check surface as user `smelt.define` declarations.
/// The two share `FunctionSig` with this discriminator so downstream lookups
/// stay uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigOrigin {
    /// A `smelt.define` declaration with a body.
    Define,
    /// A `smelt.extern` declaration — signature only.
    Extern,
}
