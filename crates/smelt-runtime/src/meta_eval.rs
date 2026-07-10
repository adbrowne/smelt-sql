//! Compile-time meta-language evaluation on the build path.
//!
//! The analyzer (`smelt-db`) type-checks and validates the meta-language
//! surface (`List<T>`, spread, HOFs, reflection, …) but does not itself
//! lower it to Data-World SQL. This module performs the *expansion* the
//! analyzer's validation presupposes, so that a meta construct the analyzer
//! accepts compiles to plain SQL rather than reaching the database engine
//! verbatim. It is the build-path half of the diagnostic-parity guarantee
//! (`docs/specs/architecture.md` §"Diagnostic parity rule") and realises
//! `meta_language.md` §Semantics "Lists and spread" rule 10 ("the Data-World
//! CST handed to codegen contains no `ARRAY_LITERAL` and no spread node").
//!
//! Currently implemented: SELECT-list **list-spread** of inline literals
//! (`SELECT id, ...[a, b] FROM t` → `SELECT id, a, b FROM t`); the **HOFs**
//! `map` / `filter` and the `reduce` reducer fold; the **pipe** operator and
//! **lambdas**; the meta-world **ternary**; **`smelt.config.var`**; and
//! **`smelt.columns_of`** reflection (its `List<ColumnRef>` is materialised from
//! the resolved schema and its `map` / `filter` / spread lowered to plain select
//! items, including when the `columns_of` call lives inside a `smelt.define`
//! body the model expands); and **wide reflection** (`smelt.models.with_tag` /
//! `.all`, `smelt.sources.with_tag` / `.all`), whose `List<ModelRef>` /
//! `List<SourceRef>` is materialised from the workspace's model / source
//! listings and its `map` / `filter` / spread lowered to plain select items
//! (the `m.name` / `m.path` projections rendered as `Text` string literals).
//! Each is evaluated at compile time and lowered to plain Data-World SQL. The
//! loader family (`smelt.config.load_yaml` / `load_json`) is evaluated by the
//! analyzer but not yet lowered here — see `meta_language.md` §"Known
//! Divergences".

use std::collections::BTreeMap;

use smelt_parser::ast::{
    BinaryExpr, Expr, FunctionCall, Lambda, ListSpread, PipeExpr, ReducerCall, SelectList,
    SmeltPathCall, SmeltPathRef, TernaryExpr,
};
use smelt_parser::syntax_kind::SyntaxNode;
use smelt_parser::SyntaxKind::{
    DOT, IDENT, LIST_SPREAD, SELECT_ITEM, SELECT_LIST, SMELT_PATH_CALL,
};

use crate::fn_bodies::FnBodyMap;

/// One `ColumnRef` reflected from an upstream model/source/seed schema, used by
/// the build-path evaluator to materialise `smelt.columns_of(t)`. The fields
/// mirror the closed `ColumnRef` record (`meta_language.md` §"Reflection") and
/// are pre-rendered to the Data-World text the meta evaluator splices: `name`
/// is the column identifier, `is_numeric` is the analyzer's numeric test, and
/// `type_name` is the column's `DataType` display string. The `is_*` head-constructor
/// predicates mirror `meta_language.md` §"Reflection: `smelt.columns_of`, `ColumnRef`".
#[derive(Clone, Debug)]
pub struct ColumnRefMeta {
    pub name: String,
    pub type_name: String,
    pub is_numeric: bool,
    pub is_decimal: bool,
    pub is_string: bool,
    pub is_temporal: bool,
    pub is_integer: bool,
    pub is_boolean: bool,
}

/// One `ModelRef` / `SourceRef` reflected from the workspace's model / source
/// listings, used by the build-path evaluator to materialise
/// `smelt.models.with_tag(t)` / `.all` and `smelt.sources.with_tag(t)` / `.all`.
/// The fields mirror the closed `ModelRef` / `SourceRef` records
/// (`meta_language.md` §"Reflection: `smelt.models`, …"): `path` is the
/// workspace-relative source path (`/`-normalised), `name` is the entity's leaf
/// identifier, and `tags` is its merged tag set (used to filter `with_tag`).
/// `path` and `name` are `Text` *data values* — they lower to SQL string
/// literals, not column references.
#[derive(Clone, Debug)]
pub struct EntityRefMeta {
    pub path: String,
    pub name: String,
    pub tags: Vec<String>,
}

/// One registered loader file's raw content, threaded into the build-path
/// evaluator so a `smelt.config.load_yaml` / `load_json` call can be materialised
/// to its concrete `List<record>` value at compile time. Mirrors the analyzer's
/// `LoaderFileInput` (raw text + existence bit); keyed by the workspace-relative
/// loader path the call's first argument names.
#[derive(Clone, Debug)]
pub struct LoaderFileMeta {
    pub text: String,
    pub exists: bool,
}

/// Compile-time context for the build-path meta evaluator.
///
/// - `vars`: the workspace `vars:` (already coerced to `Text`) for `smelt.config.var`.
/// - `columns`: upstream model/source/seed column lists keyed by the entity's
///   leaf name, for `smelt.columns_of(smelt.<path>)` reflection.
/// - `fn_bodies`: the project's `smelt.define` bodies. When a model spreads a
///   function call whose body is a reflection chain (`columns_of`), the
///   evaluator substitutes the call's arguments into the body and evaluates it
///   directly; `None` disables that (the call is left to the printer).
/// - `models` / `sources`: the workspace's model / source listings (with merged
///   tags), for `smelt.models.*` / `smelt.sources.*` wide reflection.
/// - `loaders`: registered loader files (raw text + existence) keyed by their
///   workspace-relative path, for `smelt.config.load_yaml` / `load_json`.
pub struct MetaEvalContext<'a> {
    pub vars: &'a BTreeMap<String, String>,
    pub columns: BTreeMap<String, Vec<ColumnRefMeta>>,
    pub fn_bodies: Option<&'a FnBodyMap>,
    pub models: &'a [EntityRefMeta],
    pub sources: &'a [EntityRefMeta],
    pub loaders: &'a BTreeMap<String, LoaderFileMeta>,
}

impl<'a> MetaEvalContext<'a> {
    /// A context carrying only `vars:` — no reflection column metadata, no
    /// function bodies, no model / source listings, and no loader files. Used by
    /// call sites (e.g. the ephemeral-CTE compiler) that do not have upstream
    /// schemas, a function-body map, a workspace listing, or loader content.
    pub fn vars_only(vars: &'a BTreeMap<String, String>) -> Self {
        Self {
            vars,
            columns: BTreeMap::new(),
            fn_bodies: None,
            models: &[],
            sources: &[],
            loaders: empty_loaders(),
        }
    }
}

/// A shared empty loader map for contexts that carry no loader files, so
/// `vars_only` and the test constructors can borrow a `'static` empty map
/// without each allocating one.
fn empty_loaders() -> &'static BTreeMap<String, LoaderFileMeta> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<BTreeMap<String, LoaderFileMeta>> = OnceLock::new();
    EMPTY.get_or_init(BTreeMap::new)
}

/// Run every in-model meta-language build-path expansion over `sql`, returning
/// the rewritten SQL. The single entry point compile sites call before parsing
/// a user model so codegen never sees a meta construct.
///
/// Passes, in order:
///   1. `smelt.config.var('x')` → the variable's `Text` literal (resolved first
///      so a ternary condition / reducer argument that reads a var is concrete);
///   2. select-item meta evaluation — HOFs (`map`/`filter`), the pipe operator,
///      lambdas, reducers (`reduce`), and the meta-world ternary, each collapsed
///      to plain Data-World SQL;
///   3. SELECT-list spread of inline list literals (`...[a, b]`).
///
/// Pure and idempotent: a second pass over already-expanded SQL is a no-op
/// (there are no meta nodes left to rewrite), so it is safe to call at every
/// compile entry point even when SQL flows through several of them. Each pass
/// is guarded by a cheap substring check so a model with no meta constructs
/// pays no reparse.
pub fn expand_in_model_meta(sql: &str, ctx: &MetaEvalContext) -> String {
    let s = expand_config_vars(sql, ctx);
    let s = expand_select_item_meta(&s, ctx);
    expand_select_list_spreads(&s, ctx)
}

/// A meta value produced by the build-path evaluator: either a `List<T>`
/// (a vector of element values), a `Map<K, V>` (a BTreeMap of key→element),
/// or a scalar Data-World SQL fragment.
enum MetaValue {
    List(Vec<MetaElem>),
    Scalar(String),
    /// A `Map<K, V>` loader value. Entries are sorted ascending by key
    /// (BTreeMap guarantees this). Each value is a `MetaElem` (scalar or record).
    Map(BTreeMap<String, MetaElem>),
}

/// One element of a build-path meta `List<T>`: either a plain Data-World SQL
/// scalar fragment (a list-literal element, a mapped value) or a record (a
/// `ColumnRef` from `smelt.columns_of`) whose fields are projected by name
/// inside a lambda body.
#[derive(Clone)]
enum MetaElem {
    Scalar(String),
    Record(BTreeMap<String, String>),
}

impl MetaElem {
    /// The scalar SQL text of this element, or `None` if it is an unprojected
    /// record (a record cannot itself be spliced as a select item / fold input).
    fn as_scalar(&self) -> Option<&str> {
        match self {
            MetaElem::Scalar(s) => Some(s),
            MetaElem::Record(_) => None,
        }
    }
}

/// Wrap `content` with the leading/trailing whitespace of `node_text`. A CST
/// node's `text()` can include trailing trivia (the space before a following
/// `AS` alias, the newline before `FROM`), so a bare splice would glue the
/// replacement to the next token (`true` + `AS` → `trueAS`). Preserving the
/// original surrounding whitespace keeps the spliced SQL well-formed.
fn splice_preserving_ws(node_text: &str, content: &str) -> String {
    let lead = &node_text[..node_text.len() - node_text.trim_start().len()];
    let trail = &node_text[node_text.trim_end().len()..];
    format!("{lead}{content}{trail}")
}

/// Apply `(start, end, replacement)` edits to `sql`, highest offset first so
/// earlier offsets stay valid. Shared by every pass in this module.
fn apply_edits(sql: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    if edits.is_empty() {
        return sql.to_string();
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = sql.to_string();
    for (start, end, repl) in edits {
        out.replace_range(start..end, &repl);
    }
    out
}

/// Pass 1 — resolve every `smelt.config.var('x')` to its `Text` literal.
///
/// Mirrors the analyzer's `check_config_var_call_diagnostics`: a `config.var`
/// call with a string-literal argument present in `vars` is replaced by the
/// variable's coerced value rendered as a single-quoted SQL `Text` literal. A
/// call whose name is absent from `vars` is left verbatim — the analyzer gates
/// it (`ConfigVarNotFound`), so it never reaches a clean build.
fn expand_config_vars(sql: &str, ctx: &MetaEvalContext) -> String {
    if !sql.contains("config.var") {
        return sql.to_string();
    }
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let Some(call) = SmeltPathCall::cast(node.clone()) else {
            continue;
        };
        let segs = call.segments();
        if segs.len() != 2 || segs[0].to_lowercase() != "config" || segs[1].to_lowercase() != "var"
        {
            continue;
        }
        let Some(arg) = call
            .arg_list()
            .and_then(|a| a.positional_args().into_iter().next())
        else {
            continue;
        };
        let Some(name) = smelt_db::config_vars::extract_string_literal_value(&arg) else {
            continue;
        };
        let Some(value) = ctx.vars.get(&name) else {
            continue; // absent → leave verbatim (analyzer gates ConfigVarNotFound)
        };
        let literal = format!("'{}'", value.replace('\'', "''"));
        let replacement = splice_preserving_ws(&node.text().to_string(), &literal);
        let range = node.text_range();
        edits.push((range.start().into(), range.end().into(), replacement));
    }
    apply_edits(sql, edits)
}

/// Pass 2 — evaluate meta-valued SELECT items to plain Data-World SQL.
///
/// For each top-level SELECT item whose expression is a meta construct that
/// evaluates to a scalar (a `reduce` fold, a meta-world ternary, or a
/// config-var literal), splice the evaluated SQL in place of the item
/// expression. Items that evaluate to a bare `List<T>` are left untouched —
/// the analyzer rejects them (`MetaListInScalarPosition`), so a clean build
/// never reaches one.
fn expand_select_item_meta(sql: &str, ctx: &MetaEvalContext) -> String {
    if !sql.contains("reduce")
        && !sql.contains("map")
        && !sql.contains("filter")
        && !sql.contains("|>")
        && !sql.contains("if ")
        && !sql.contains(".keys(")
        && !sql.contains(".values(")
        && !sql.contains(".entries(")
    {
        return sql.to_string();
    }
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for node in root.descendants() {
        if node.kind() != SELECT_LIST {
            continue;
        }
        let Some(select_list) = SelectList::cast(node.clone()) else {
            continue;
        };
        for item in select_list.items() {
            let Some(expr) = item.expression() else {
                continue;
            };
            if let Some(MetaValue::Scalar(s)) = eval_meta(&expr, ctx) {
                let replacement = splice_preserving_ws(&expr.syntax().text().to_string(), &s);
                let range = expr.syntax().text_range();
                edits.push((range.start().into(), range.end().into(), replacement));
            }
        }
    }
    apply_edits(sql, edits)
}

/// Evaluate a meta expression to a [`MetaValue`], or `None` if `expr` is not a
/// recognized meta construct (a plain Data-World expression) or cannot be
/// evaluated at compile time (in which case the caller leaves it verbatim).
fn eval_meta(expr: &Expr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    // Pipe `LHS |> f(args)` → evaluate as `f(LHS, args)`. Checked FIRST: a pipe's
    // LHS may itself be a `columns_of` / function call, and `as_smelt_path_call`
    // digs into a pipe's subtree — so a columns_of/fn check here would wrongly
    // match the LHS and drop the trailing pipe stages.
    if let Some(pipe) = as_pipe(expr) {
        return eval_pipe(&pipe, ctx);
    }
    // Meta-world ternary `if cond then a else b`.
    if let Some(tern) = as_ternary(expr) {
        return eval_ternary(&tern, ctx);
    }
    // Reflection `smelt.columns_of(smelt.<path>)` → List<ColumnRef>.
    if let Some(list) = eval_columns_of(expr, ctx) {
        return Some(list);
    }
    // Wide reflection `smelt.models.*` / `smelt.sources.*` → List<ModelRef|SourceRef>.
    if let Some(list) = eval_wide_reflection(expr, ctx) {
        return Some(list);
    }
    // Config loader `smelt.config.load_yaml/load_json('path', List<…>|Map<…>)` → List/Map.
    if let Some(value) = eval_loader(expr, ctx) {
        return Some(value);
    }
    // MAP_METHOD_CALL: `expr.method()` where method is a Map API name.
    // E.g. `smelt.config.load_yaml('p', Map<Text, S>).keys()` → List<Text>.
    // Must be checked AFTER eval_loader so that the receiver (a loader call)
    // is already recognized and can be evaluated recursively.
    if expr.as_map_method_call().is_some() {
        if let Some(v) = eval_map_method_call(expr, ctx) {
            return Some(v);
        }
    }
    // A `smelt.define` call whose body is a reflection chain → substitute the
    // call's arguments into the body and evaluate it (no printer reprint).
    if let Some(call) = expr.as_smelt_path_call() {
        if let Some(v) = eval_smelt_fn_call(&call, ctx) {
            return Some(v);
        }
    }
    // List literal `[a, b, c]` → List of element SQL fragments.
    if let Some(arr) = expr.as_array_literal() {
        let mut elems = Vec::new();
        for el in arr.elements() {
            elems.push(MetaElem::Scalar(render_scalar(&el, ctx)?));
        }
        return Some(MetaValue::List(elems));
    }
    // HOF call `map(xs, f)` / `filter(xs, p)` / `reduce(xs, r)`.
    if let Some(call) = expr.as_function_call() {
        if let Some(kind) = hof_kind_of(&call) {
            let args = call.arguments();
            let first = eval_meta(args.first()?, ctx)?;
            return apply_hof(kind, first, &args[1..]);
        }
    }
    None
}

/// Evaluate `smelt.columns_of(smelt.<path>)` to a `List<ColumnRef>` using the
/// reflected column metadata in `ctx`. Returns `None` when `expr` is not a
/// `columns_of` call, the argument is not a resolvable `smelt.<path>` ref, or
/// the entity's columns are not in the context (the analyzer's drop-on-error
/// policy: leave the construct verbatim — a clean build never reaches this).
fn eval_columns_of(expr: &Expr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let call = expr.as_smelt_path_call()?;
    let segs = call.segments();
    if segs.len() != 1 || !segs[0].eq_ignore_ascii_case("columns_of") {
        return None;
    }
    let arg = call
        .arg_list()
        .and_then(|a| a.positional_args().into_iter().next())?;
    let leaf = columns_of_target_leaf(&arg)?;
    let cols = ctx.columns.get(&leaf)?;
    let elems = cols
        .iter()
        .map(|c| {
            let mut fields = BTreeMap::new();
            fields.insert("name".to_string(), c.name.clone());
            fields.insert(
                "is_numeric".to_string(),
                if c.is_numeric { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "is_decimal".to_string(),
                if c.is_decimal { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "is_string".to_string(),
                if c.is_string { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "is_temporal".to_string(),
                if c.is_temporal { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "is_integer".to_string(),
                if c.is_integer { "true" } else { "false" }.to_string(),
            );
            fields.insert(
                "is_boolean".to_string(),
                if c.is_boolean { "true" } else { "false" }.to_string(),
            );
            fields.insert("type".to_string(), c.type_name.clone());
            MetaElem::Record(fields)
        })
        .collect();
    Some(MetaValue::List(elems))
}

/// Extract the leaf entity name of a `smelt.columns_of` argument — the last
/// segment of a `smelt.<path>` reference (`smelt.orders` → `"orders"`,
/// `smelt.sources.raw.orders` → `"orders"`). Mirrors how the analyzer's
/// reflection resolution keys on the leaf segment.
fn columns_of_target_leaf(arg: &Expr) -> Option<String> {
    if let Some(path_ref) = SmeltPathRef::cast(arg.syntax().clone())
        .or_else(|| arg.syntax().children().find_map(SmeltPathRef::cast))
    {
        return path_ref.segments().last().cloned();
    }
    if let Some(call) = arg.as_smelt_path_call() {
        return call.segments().last().cloned();
    }
    None
}

/// Evaluate wide reflection — `smelt.models.with_tag(t)` / `smelt.models.all`
/// and `smelt.sources.with_tag(t)` / `smelt.sources.all` — to a
/// `List<ModelRef>` / `List<SourceRef>` of record elements, using the workspace
/// model / source listings in `ctx`. `with_tag` filters the listing by tag
/// membership (the analyzer's `Config::get_tags`-merged set); `all` returns the
/// whole listing (already path-sorted by the producer). Returns `None` when
/// `expr` is not a `smelt.models.*` / `smelt.sources.*` accessor, when the
/// accessor is outside the closed `{with_tag, all}` set, or when a `with_tag`
/// argument is not a resolvable `Text` literal — all of which the analyzer
/// gates, so a clean build never reaches one (drop-on-error: leave verbatim).
fn eval_wide_reflection(expr: &Expr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let (namespace, accessor, tag) = wide_reflection_parts(expr)?;
    let listing = match namespace.as_str() {
        "models" => &ctx.models,
        "sources" => &ctx.sources,
        _ => return None,
    };
    let elems: Vec<MetaElem> = match accessor.as_str() {
        "all" => listing.iter().map(entity_ref_record).collect(),
        "with_tag" => {
            let tag = tag?;
            listing
                .iter()
                .filter(|e| e.tags.contains(&tag))
                .map(entity_ref_record)
                .collect()
        }
        _ => return None,
    };
    Some(MetaValue::List(elems))
}

/// Decompose a wide-reflection accessor **call** into `(namespace, accessor,
/// tag)`. Only the call form is recognised — `smelt.models.with_tag('x')`,
/// `smelt.models.all()`, and the `smelt.sources` equivalents — because that is
/// the surface the analyzer accepts: the analyzer recognises wide reflection
/// only at a `SmeltPathCall` (`infer_smelt_path_call_type`), and a *bare*
/// `smelt.models.all` path ref (no parens) is resolved as an ordinary model ref
/// and rejected as `UndefinedModelRef`. Restricting the build to the call form
/// keeps the build-accepted surface equal to the analyzer-accepted surface
/// (`architecture.md` §"Diagnostic parity rule"). `namespace` is `"models"` /
/// `"sources"`; `tag` is the `with_tag` argument's string-literal value (or
/// `None` for `all` / an unresolvable argument — `smelt.config.var` arguments
/// are already resolved to literals by the earlier `expand_config_vars` pass).
/// Returns `None` unless the call's path is exactly `<namespace>.<accessor>`.
fn wide_reflection_parts(expr: &Expr) -> Option<(String, String, Option<String>)> {
    let call = expr.as_smelt_path_call()?;
    let segs = call.segments();
    if segs.len() != 2 {
        return None;
    }
    let namespace = segs[0].to_lowercase();
    if namespace != "models" && namespace != "sources" {
        return None;
    }
    let tag = call
        .arg_list()
        .and_then(|a| a.positional_args().into_iter().next())
        .and_then(|arg| smelt_db::config_vars::extract_string_literal_value(&arg));
    Some((namespace, segs[1].to_lowercase(), tag))
}

/// Build a `ModelRef` / `SourceRef` record element from an [`EntityRefMeta`].
/// `name` and `path` are rendered as SQL `Text` string literals (a model name
/// is a data value, not a column reference). `tags` (a `List<Text>`) is not
/// projected here — a `m.tags` projection has no scalar form and is left to the
/// conservative drop-on-error path (`meta_language.md` §"Known Divergences").
fn entity_ref_record(e: &EntityRefMeta) -> MetaElem {
    let mut fields = BTreeMap::new();
    fields.insert("name".to_string(), sql_text_literal(&e.name));
    fields.insert("path".to_string(), sql_text_literal(&e.path));
    MetaElem::Record(fields)
}

/// Render a `Text` value as a single-quoted SQL string literal (doubling any
/// embedded single quote).
fn sql_text_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Evaluate a config loader call `smelt.config.load_yaml('path', List<…>)` /
/// `load_json` to a `List<record>` of record elements, using the registered
/// loader file content in `ctx`. The file is parsed with the analyzer's own
/// `parse_yaml` / `parse_json` and validated against the call's schema via
/// `validate_against_schema`, so the build materialises exactly the value the
/// analyzer validated (`architecture.md` §"Diagnostic parity rule"). Each loaded
/// record's scalar fields are pre-rendered to Data-World SQL literals (`Text` →
/// `'…'`, `Integer` → bare number, `Boolean` → `TRUE`/`FALSE`); a record's
/// non-scalar field (a nested record/list) is omitted (it has no scalar
/// projection form).
///
/// Returns `None` — leaving the call verbatim — when `expr` is not a loader
/// call, the schema is not `List<…>` (a record-root or `Map<…>` loader is not
/// yet lowered here — see `meta_config_loading.md` §"Known Divergences"), the
/// path is not a resolvable registered loader file, the file fails to parse, or
/// validation produced any `Error` (all of which the analyzer gates, so a clean
/// build never reaches one — drop-on-error).
fn eval_loader(expr: &Expr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let call = expr.as_smelt_path_call()?;
    let segs = call.segments();
    if segs.len() != 2 || segs[0].to_lowercase() != "config" {
        return None;
    }
    let loader = segs[1].to_lowercase();
    if loader != "load_yaml" && loader != "load_json" {
        return None;
    }
    let arg_list = call.arg_list()?;
    let positional = arg_list.positional_args();
    let path_arg = positional.first()?;
    let schema_arg = positional.get(1)?;

    let path = smelt_db::config_vars::extract_string_literal_value(path_arg)?;
    let file = ctx.loaders.get(&path)?;
    if !file.exists {
        return None;
    }

    // `List<…>` and `Map<…>` loaders are both lowered at build; a record-root
    // schema is left verbatim (analyzer-validated, not yet lowered).
    let schema_text = schema_arg.syntax().text().to_string();
    let schema_text = schema_text.trim();
    if !schema_text.starts_with("List<") && !schema_text.starts_with("Map<") {
        return None;
    }
    let schema_ty =
        smelt_db::queries::loader::parse_smelt_type_from_field_annotation(schema_text, "");
    if !smelt_db::loader::is_admissible_loader_schema(&schema_ty) {
        return None;
    }

    let parsed = if path.ends_with(".json") {
        smelt_db::loader::parse_json(&file.text, &path)
    } else {
        smelt_db::loader::parse_yaml(&file.text, &path)
    }
    .ok()?;

    let range = smelt_parser::TextRange::new(0.into(), 0.into());
    let result = smelt_db::loader::validate_against_schema(&parsed, &schema_ty, range);
    if result
        .diagnostics
        .iter()
        .any(|d| d.severity == smelt_db::loader::LoaderDiagnosticSeverity::Error)
    {
        return None; // analyzer gates content errors; leave verbatim defensively
    }
    loader_value_to_meta(&result.value)
}

/// Convert a validated loader value into a build-path [`MetaValue`].
///
/// - A `List` becomes `MetaValue::List`: each element is a `MetaElem` (record
///   with scalar fields, or a scalar rendered to a Data-World SQL literal).
/// - A `Map` becomes `MetaValue::Map`: each entry maps a key string to a
///   `MetaElem` (same element rules as List). The BTreeMap preserves
///   lexicographic key order, which the Map API methods (`keys`, `values`,
///   `entries`) expose in ascending order.
/// - Any other variant returns `None` (drop-on-error).
fn loader_value_to_meta(value: &smelt_db::loader::MetaValue) -> Option<MetaValue> {
    use smelt_db::loader::MetaValue as LV;
    match value {
        LV::List(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for elem in elems {
                match elem {
                    LV::Record(fields) => {
                        let mut rec = BTreeMap::new();
                        for (k, v) in fields {
                            if let Some(s) = render_loader_scalar(v) {
                                rec.insert(k.clone(), s);
                            }
                        }
                        out.push(MetaElem::Record(rec));
                    }
                    // a non-scalar, non-record element is unrenderable
                    scalar => out.push(MetaElem::Scalar(render_loader_scalar(scalar)?)),
                }
            }
            Some(MetaValue::List(out))
        }
        LV::Map(entries) => {
            let mut out = BTreeMap::new();
            for (k, v) in entries {
                let elem = match v {
                    LV::Record(fields) => {
                        let mut rec = BTreeMap::new();
                        for (fk, fv) in fields {
                            if let Some(s) = render_loader_scalar(fv) {
                                rec.insert(fk.clone(), s);
                            }
                        }
                        MetaElem::Record(rec)
                    }
                    scalar => MetaElem::Scalar(render_loader_scalar(scalar)?),
                };
                out.insert(k.clone(), elem);
            }
            Some(MetaValue::Map(out))
        }
        _ => None,
    }
}

/// Render a scalar loader value to its Data-World SQL literal. `Text` →
/// single-quoted (quotes doubled); `Integer` / `Float` → the numeric literal;
/// `Boolean` → `TRUE` / `FALSE`. A composite value (record / list / map) or an
/// error node has no scalar form → `None`.
fn render_loader_scalar(value: &smelt_db::loader::MetaValue) -> Option<String> {
    use smelt_db::loader::MetaValue as LV;
    match value {
        LV::Text(s) => Some(sql_text_literal(s)),
        LV::Integer(i) => Some(i.to_string()),
        LV::Float(f) => Some(f.to_string()),
        LV::Boolean(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        LV::Record(_) | LV::List(_) | LV::Map(_) | LV::Error => None,
    }
}

/// Evaluate a `MAP_METHOD_CALL` expression — `expr.method(args)` where
/// `method` is one of the known Map API names (entries, keys, values, get, has).
///
/// The receiver is evaluated to a `MetaValue::Map`; the method is then applied
/// to produce a new `MetaValue`:
///
/// - `keys()` → `MetaValue::List` of `MetaElem::Scalar` (each key rendered as
///   a SQL `Text` literal, sorted ascending by the BTreeMap's natural order).
/// - `values()` → `MetaValue::List` of the map's values (MetaElem, same order).
/// - `entries()` → `MetaValue::List` of `MetaElem::Record` with a `key` field
///   (the SQL `Text` literal) plus the value's fields (scalar: `value` field;
///   record: each field promoted to the top-level record, so a lambda body can
///   use `e.key` and `e.plan` rather than `e.value.plan`).
/// - `get` / `has` — data-world ops; not lowered at build time (return `None`
///   to leave verbatim). The analyzer gates receiver-type errors so a clean
///   build never calls this for a non-Map receiver.
///
/// Returns `None` on any drop-on-error condition (non-Map receiver, unknown
/// receiver, unsupported method at compile time).
fn eval_map_method_call(expr: &Expr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let call = expr.as_map_method_call()?;
    let method = call.method_name()?.to_lowercase();
    let receiver_expr = call.receiver_expr()?;
    let receiver = eval_meta(&receiver_expr, ctx)?;
    let MetaValue::Map(map) = receiver else {
        return None; // receiver did not evaluate to a Map — drop-on-error
    };
    match method.as_str() {
        "keys" => {
            // BTreeMap is already sorted ascending by key.
            let elems: Vec<MetaElem> = map
                .keys()
                .map(|k| MetaElem::Scalar(sql_text_literal(k)))
                .collect();
            Some(MetaValue::List(elems))
        }
        "values" => {
            let elems: Vec<MetaElem> = map.into_values().collect();
            Some(MetaValue::List(elems))
        }
        "entries" => {
            // Each entry becomes a record with a `key` field (SQL `Text` literal)
            // plus the value's fields.  For scalar values a `value` field carries
            // the SQL literal; for record values the record's own fields are
            // promoted to the top-level so a lambda body uses `e.key` and `e.plan`
            // rather than `e.value.plan` (pragmatic flat-record implementation).
            let elems: Vec<MetaElem> = map
                .into_iter()
                .map(|(k, v)| {
                    let mut rec = BTreeMap::new();
                    rec.insert("key".to_string(), sql_text_literal(&k));
                    match v {
                        MetaElem::Scalar(s) => {
                            rec.insert("value".to_string(), s);
                        }
                        MetaElem::Record(fields) => {
                            // Promote each record field to the top-level entry record.
                            for (fk, fv) in fields {
                                rec.insert(fk, fv);
                            }
                        }
                    }
                    MetaElem::Record(rec)
                })
                .collect();
            Some(MetaValue::List(elems))
        }
        // `get` / `has` are data-world ops; leave verbatim at build time.
        _ => None,
    }
}

/// Evaluate a `smelt.define` call (`smelt.functions.foo(args)` / `smelt.fn.foo`
/// / `smelt.foo`) whose body is a reflection chain (contains `columns_of`):
/// substitute the call's printed arguments into the function body, parse the
/// substituted body, and evaluate it as a meta expression. This lets a
/// `columns_of` that lives inside a function body (the `meta_columns`
/// `coalesce_numeric` shape) lower at compile time without inlining through the
/// printer (which would reprint and disturb the spread's surrounding SQL).
///
/// Returns `None` for a non-function call, an unknown function, or a non-reflection
/// body — those are left to the printer's normal function expansion.
fn eval_smelt_fn_call(call: &SmeltPathCall, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let bodies = ctx.fn_bodies?;
    let leaf = call.segments().last()?.clone();
    let (params, body) = bodies.get(&leaf)?;
    if !body.contains("columns_of") {
        return None;
    }
    let arg_list = call.arg_list();
    let positional: Vec<String> = arg_list
        .as_ref()
        .map(|al| {
            al.positional_args()
                .into_iter()
                .map(|a| a.syntax().text().to_string().trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    let named: Vec<(String, String)> = arg_list
        .as_ref()
        .map(|al| {
            al.named_params()
                .filter_map(|np| {
                    Some((
                        np.name()?,
                        np.value_expr()?
                            .syntax()
                            .text()
                            .to_string()
                            .trim()
                            .to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let substituted =
        crate::compile::substitute_params_with_named(body, params, &positional, &named);
    let wrapped = format!("SELECT {substituted}");
    let parse = smelt_parser::parse(&wrapped);
    let expr = parse
        .syntax()
        .descendants()
        .find_map(SelectList::cast)?
        .items()
        .next()?
        .expression()?;
    eval_meta(&expr, ctx)
}

/// Render a meta expression as a scalar SQL fragment. A recognized meta scalar
/// (ternary, reduce, …) yields its evaluated SQL; a `List<T>` cannot be a
/// scalar (`None`); a plain Data-World expression yields its source text
/// (the identity case for list elements and lambda-body leaves).
fn render_scalar(expr: &Expr, ctx: &MetaEvalContext) -> Option<String> {
    match eval_meta(expr, ctx) {
        Some(MetaValue::Scalar(s)) => Some(s),
        Some(MetaValue::List(_)) | Some(MetaValue::Map(_)) => None,
        None => Some(expr.syntax().text().to_string().trim().to_string()),
    }
}

/// Evaluate a pipe `LHS |> f(args)` as `f(LHS, args)`.
fn eval_pipe(pipe: &PipeExpr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let lhs = pipe.lhs()?;
    let first = eval_meta(&lhs, ctx)?;
    let rhs = pipe.rhs()?;
    let call = rhs.as_function_call()?;
    let kind = hof_kind_of(&call)?;
    apply_hof(kind, first, &call.arguments())
}

/// Apply a HOF (`map` / `filter` / `reduce`) to a `first` argument value
/// (the source list) and the remaining argument expressions (the lambda or
/// reducer).
fn apply_hof(kind: &str, first: MetaValue, rest: &[Expr]) -> Option<MetaValue> {
    let MetaValue::List(list) = first else {
        return None; // map/filter/reduce require a List operand
    };
    match kind {
        "map" => {
            let lambda = extract_lambda(rest.first()?)?;
            let param = lambda.params().into_iter().next()?;
            let body = lambda.body()?;
            let mut out = Vec::new();
            for elem in &list {
                out.push(MetaElem::Scalar(substitute_lambda(
                    body.syntax(),
                    &param,
                    elem,
                )?));
            }
            Some(MetaValue::List(out))
        }
        "filter" => {
            let lambda = extract_lambda(rest.first()?)?;
            let param = lambda.params().into_iter().next()?;
            let body = lambda.body()?;
            let mut out = Vec::new();
            for elem in &list {
                let pred = substitute_lambda(body.syntax(), &param, elem)?;
                match eval_const_bool_from_text(&pred) {
                    Some(true) => out.push(elem.clone()),
                    Some(false) => {}
                    None => return None, // non-constant predicate → bail (leave verbatim)
                }
            }
            Some(MetaValue::List(out))
        }
        "reduce" => {
            let reducer_expr = rest.first()?;
            let (name, sep) = if let Some(rc) = as_reducer_call(reducer_expr) {
                let sep = rc
                    .args()
                    .and_then(|args| args.children().find_map(Expr::cast))
                    .map(|e| e.syntax().text().to_string().trim().to_string());
                (rc.name()?.to_lowercase(), sep)
            } else {
                (
                    reducer_expr
                        .syntax()
                        .text()
                        .to_string()
                        .trim()
                        .to_lowercase(),
                    None,
                )
            };
            // A reducer folds scalar fragments; an unprojected record element
            // (a bare `ColumnRef`) has no scalar form → bail (leave verbatim).
            let scalars: Option<Vec<String>> = list
                .iter()
                .map(|e| e.as_scalar().map(str::to_string))
                .collect();
            fold_reducer(&name, sep.as_deref(), &scalars?).map(MetaValue::Scalar)
        }
        _ => None,
    }
}

/// Substitute a lambda parameter in `body` for one list element.
///
/// For a scalar element this is plain identifier substitution. For a record
/// element (a `ColumnRef`) the body projects fields by name (`c.name`,
/// `c.is_numeric`, `c.type`); each `<param>.<field>` is replaced by the field's
/// pre-rendered value. A bare `<param>` (no field projection) on a record, or a
/// projection of a field the record lacks, returns `None` so the caller leaves
/// the construct verbatim.
fn substitute_lambda(body: &SyntaxNode, param: &str, elem: &MetaElem) -> Option<String> {
    match elem {
        MetaElem::Scalar(s) => Some(substitute_ident(body, param, s)),
        MetaElem::Record(fields) => substitute_record_fields(body, param, fields),
    }
}

/// Reconstruct `body`'s text, replacing each `<param>.<field>` projection with
/// the record's value for `<field>`. Token-level with trivia-skipping lookahead
/// so `c.name` (and the rarer `c . name`) both match without touching a
/// substring inside another identifier.
fn substitute_record_fields(
    body: &SyntaxNode,
    param: &str,
    fields: &BTreeMap<String, String>,
) -> Option<String> {
    let toks: Vec<(smelt_parser::SyntaxKind, String)> = body
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .map(|t| (t.kind(), t.text().to_string()))
        .collect();
    let next_significant = |from: usize| (from..toks.len()).find(|&i| !toks[i].1.trim().is_empty());

    let mut out = String::new();
    let mut i = 0;
    while i < toks.len() {
        let (kind, text) = &toks[i];
        if *kind == IDENT && text == param {
            // Expect `. <field>` immediately following (skipping trivia).
            if let Some(j) = next_significant(i + 1) {
                if toks[j].0 == DOT {
                    if let Some(k) = next_significant(j + 1) {
                        if toks[k].0 == IDENT {
                            let value = fields.get(&toks[k].1)?;
                            out.push_str(value);
                            i = k + 1;
                            continue;
                        }
                    }
                }
            }
            // Bare `param` on a record cannot be rendered as a scalar.
            return None;
        }
        out.push_str(text);
        i += 1;
    }
    Some(out.trim().to_string())
}

/// Render a reducer's left-fold over `elems` into a single SQL fragment.
/// Mirrors `meta_language.md` §Semantics "Contextual reducers" rule 1.
///
/// Each element is parenthesised for the boolean reducers (`and_all` / `or_any`):
/// a non-atomic element such as `a OR b` folded under AND must stay grouped
/// (`(a OR b) AND c`), since bare ` AND ` would re-associate to `a OR (b AND c)`.
/// Numeric (`plus_chain`) and `Text` (`concat` / `concat_with`) folds are safe
/// without parens — their operators are left-associative and no element-level
/// operator binds looser than the fold operator.
fn fold_reducer(name: &str, sep: Option<&str>, elems: &[String]) -> Option<String> {
    let empty = elems.is_empty();
    let parenthesised = || -> Vec<String> { elems.iter().map(|e| format!("({e})")).collect() };
    let folded = match name {
        "comma_sep" => elems.join(", "),
        "and_all" => {
            if empty {
                "TRUE".to_string()
            } else {
                parenthesised().join(" AND ")
            }
        }
        "or_any" => {
            if empty {
                "FALSE".to_string()
            } else {
                parenthesised().join(" OR ")
            }
        }
        "plus_chain" => {
            if empty {
                "0".to_string()
            } else {
                elems.join(" + ")
            }
        }
        "concat" => {
            if empty {
                "''".to_string()
            } else {
                elems.join(" || ")
            }
        }
        "union_all" => {
            if empty {
                return None; // no identity — analyzer gates ReducerEmptyNoIdentity
            }
            elems.join(" UNION ALL ")
        }
        "intersect_all" => {
            if empty {
                return None; // no identity — analyzer gates ReducerEmptyNoIdentity
            }
            elems.join(" INTERSECT ")
        }
        "concat_with" => {
            let sep = sep?;
            if empty {
                "''".to_string()
            } else {
                elems.join(&format!(" || {sep} || "))
            }
        }
        _ => return None,
    };
    Some(folded)
}

/// Evaluate a meta-world ternary `if COND then A else B`: resolve `COND` at
/// compile time and return the chosen branch's SQL (the other branch is not
/// evaluated). `meta_language.md` §Semantics "Meta-world ternary" rule 3.
fn eval_ternary(tern: &TernaryExpr, ctx: &MetaEvalContext) -> Option<MetaValue> {
    let cond = tern.condition()?;
    let chosen = if eval_const_bool_node(&cond)? {
        tern.then_branch()?
    } else {
        tern.else_branch()?
    };
    render_scalar(&chosen, ctx).map(MetaValue::Scalar)
}

// ── small AST helpers ────────────────────────────────────────────────────────

/// Cast an expression (looking through its `EXPRESSION` wrapper) to a pipe.
fn as_pipe(expr: &Expr) -> Option<PipeExpr> {
    let node = expr.syntax();
    node.children()
        .find_map(PipeExpr::cast)
        .or_else(|| PipeExpr::cast(node.clone()))
}

/// Cast an expression (looking through its `EXPRESSION` wrapper) to a ternary.
fn as_ternary(expr: &Expr) -> Option<TernaryExpr> {
    let node = expr.syntax();
    node.children()
        .find_map(TernaryExpr::cast)
        .or_else(|| TernaryExpr::cast(node.clone()))
}

/// Find a parameterised `REDUCER_CALL` (`concat_with(' OR ')`) in `expr`.
fn as_reducer_call(expr: &Expr) -> Option<ReducerCall> {
    expr.syntax().descendants().find_map(ReducerCall::cast)
}

/// Find the `LAMBDA` node inside a HOF argument expression.
fn extract_lambda(expr: &Expr) -> Option<Lambda> {
    expr.syntax().descendants().find_map(Lambda::cast)
}

/// The HOF kind (`map` / `filter` / `reduce`) of a call, or `None`. `filter`
/// lexes as a keyword, so `name()` may be `None`; fall back to the first token.
fn hof_kind_of(call: &FunctionCall) -> Option<&'static str> {
    let name = call.name().map(|n| n.to_lowercase()).or_else(|| {
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .map(|t| t.text().to_lowercase())
            .find(|t| t == "map" || t == "filter" || t == "reduce")
    });
    match name.as_deref() {
        Some("map") => Some("map"),
        Some("filter") => Some("filter"),
        Some("reduce") => Some("reduce"),
        _ => None,
    }
}

/// Reconstruct `node`'s text with every `IDENT` token equal to `param` replaced
/// by `replacement` (lambda-parameter substitution). Token-level so it never
/// touches a substring inside another identifier.
fn substitute_ident(node: &SyntaxNode, param: &str, replacement: &str) -> String {
    let mut out = String::new();
    for tok in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        if tok.kind() == IDENT && tok.text() == param {
            out.push_str(replacement);
        } else {
            out.push_str(tok.text());
        }
    }
    out.trim().to_string()
}

// ── compile-time constant evaluator (filter predicates, ternary conditions) ──

/// A compile-time constant value.
#[derive(Debug, Clone)]
enum ConstVal {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
}

/// Parse and evaluate the boolean value of `text` (a substituted predicate or a
/// ternary condition). `None` when it does not reduce to a compile-time boolean.
fn eval_const_bool_from_text(text: &str) -> Option<bool> {
    let parse = smelt_parser::parse(&format!("SELECT {text}"));
    let root = parse.syntax();
    let expr = root
        .descendants()
        .find_map(SelectList::cast)?
        .items()
        .next()?
        .expression()?;
    eval_const_bool_node(&expr)
}

/// Evaluate `expr` to a boolean constant.
fn eval_const_bool_node(expr: &Expr) -> Option<bool> {
    match eval_const(expr)? {
        ConstVal::Bool(b) => Some(b),
        _ => None,
    }
}

/// Evaluate `expr` to a compile-time constant value.
fn eval_const(expr: &Expr) -> Option<ConstVal> {
    if let Some(bin) = expr.as_binary() {
        return eval_const_binary(&bin);
    }
    parse_const_literal(expr.syntax().text().to_string().trim())
}

fn eval_const_binary(bin: &BinaryExpr) -> Option<ConstVal> {
    let left = eval_const(&bin.left()?)?;
    let right = eval_const(&bin.right()?)?;
    let op = bin.operator()?.to_uppercase();
    apply_binop(&op, &left, &right)
}

fn parse_const_literal(text: &str) -> Option<ConstVal> {
    let t = text.trim();
    match t.to_lowercase().as_str() {
        "true" => return Some(ConstVal::Bool(true)),
        "false" => return Some(ConstVal::Bool(false)),
        _ => {}
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        let inner = &t[1..t.len() - 1];
        return Some(ConstVal::Str(inner.replace("''", "'")));
    }
    if let Ok(i) = t.parse::<i128>() {
        return Some(ConstVal::Int(i));
    }
    if let Ok(f) = t.parse::<f64>() {
        return Some(ConstVal::Float(f));
    }
    None
}

fn as_f64(v: &ConstVal) -> Option<f64> {
    match v {
        ConstVal::Int(i) => Some(*i as f64),
        ConstVal::Float(f) => Some(*f),
        _ => None,
    }
}

fn apply_binop(op: &str, l: &ConstVal, r: &ConstVal) -> Option<ConstVal> {
    use std::cmp::Ordering;
    // Arithmetic on numerics.
    if matches!(op, "+" | "-" | "*" | "/") {
        if let (ConstVal::Int(a), ConstVal::Int(b)) = (l, r) {
            return match op {
                "+" => Some(ConstVal::Int(a + b)),
                "-" => Some(ConstVal::Int(a - b)),
                "*" => Some(ConstVal::Int(a * b)),
                "/" if *b != 0 => Some(ConstVal::Int(a / b)),
                _ => None,
            };
        }
        let (a, b) = (as_f64(l)?, as_f64(r)?);
        return match op {
            "+" => Some(ConstVal::Float(a + b)),
            "-" => Some(ConstVal::Float(a - b)),
            "*" => Some(ConstVal::Float(a * b)),
            "/" if b != 0.0 => Some(ConstVal::Float(a / b)),
            _ => None,
        };
    }
    // Equality (numerics, strings, booleans).
    if matches!(op, "=" | "==" | "<>" | "!=") {
        let eq = match (l, r) {
            (ConstVal::Str(a), ConstVal::Str(b)) => a == b,
            (ConstVal::Bool(a), ConstVal::Bool(b)) => a == b,
            _ => (as_f64(l)? - as_f64(r)?).abs() < f64::EPSILON,
        };
        return Some(ConstVal::Bool(if matches!(op, "=" | "==") {
            eq
        } else {
            !eq
        }));
    }
    // Ordering (numerics only).
    if matches!(op, "<" | ">" | "<=" | ">=") {
        let ord = as_f64(l)?.partial_cmp(&as_f64(r)?)?;
        let b = match op {
            "<" => ord == Ordering::Less,
            ">" => ord == Ordering::Greater,
            "<=" => ord != Ordering::Greater,
            ">=" => ord != Ordering::Less,
            _ => unreachable!(),
        };
        return Some(ConstVal::Bool(b));
    }
    // Boolean composition.
    if matches!(op, "AND" | "OR") {
        if let (ConstVal::Bool(a), ConstVal::Bool(b)) = (l, r) {
            return Some(ConstVal::Bool(if op == "AND" {
                *a && *b
            } else {
                *a || *b
            }));
        }
    }
    None
}

/// Expand inline list-literal spreads in SELECT lists to plain comma-separated
/// SELECT items at compile time.
///
/// For every `SELECT_LIST` that contains a `LIST_SPREAD`, the list is rebuilt
/// from its entries: a regular item is kept verbatim, and a spread of an inline
/// list literal `...[e_1, …, e_n]` contributes its `n` element expressions in
/// order. An empty-list spread `...[]` contributes nothing — eliding itself and
/// its adjacent commas (`meta_language.md` §Semantics rule 7).
///
/// A spread whose operand is a reflection / HOF chain that evaluates to a
/// `List` of scalar fragments (e.g. `...smelt.columns_of(t) |> map(fn c => c.name)`)
/// is lowered to those fragments as select items. A spread whose operand does
/// not evaluate to a scalar list — an unresolvable reflection, or a list whose
/// elements are unprojected records — is left untouched: the whole containing
/// SELECT list is passed through verbatim rather than partially rewritten. The
/// same conservative pass-through applies to a list literal carrying a nested
/// spread, and to any SELECT list whose children are not exclusively items and
/// spreads.
pub fn expand_select_list_spreads(sql: &str, ctx: &MetaEvalContext) -> String {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();

    // Collect (start, end, replacement) edits for every rewritable SELECT_LIST.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for node in root.descendants() {
        if node.kind() != SELECT_LIST {
            continue;
        }
        // Only rewrite lists that actually contain a spread.
        if !node.children().any(|c| c.kind() == LIST_SPREAD) {
            continue;
        }
        // Conservative bail: only rewrite a list whose every child node is a
        // SELECT_ITEM or LIST_SPREAD. Anything else (a bare `*`, an unforeseen
        // node) means our entry-based rebuild could silently drop a child, so
        // we leave the list verbatim.
        if node
            .children()
            .any(|c| c.kind() != SELECT_ITEM && c.kind() != LIST_SPREAD)
        {
            continue;
        }

        let mut items: Vec<String> = Vec::new();
        let mut rewritable = true;
        for child in node.children() {
            if child.kind() == SELECT_ITEM {
                items.push(child.text().to_string().trim().to_string());
                continue;
            }
            // Must be a LIST_SPREAD (the bail above guarantees it).
            let Some(spread) = ListSpread::cast(child.clone()) else {
                rewritable = false;
                break;
            };
            let Some(operand) = spread.operand() else {
                rewritable = false;
                break;
            };
            if let Some(arr) = operand.as_array_literal() {
                if has_nested_spread(&child) {
                    // Literal carrying a nested spread: not yet lowered here.
                    rewritable = false;
                    break;
                }
                for el in arr.elements() {
                    items.push(el.syntax().text().to_string().trim().to_string());
                }
            } else if let Some(MetaValue::List(elems)) = eval_meta(&operand, ctx) {
                // Reflection / HOF chain → splice its scalar elements. An
                // unprojected record element has no select-item form: bail.
                let mut scalars = Vec::with_capacity(elems.len());
                for e in &elems {
                    match e.as_scalar() {
                        Some(s) => scalars.push(s.to_string()),
                        None => {
                            rewritable = false;
                            break;
                        }
                    }
                }
                if !rewritable {
                    break;
                }
                items.extend(scalars);
            } else {
                // Operand does not evaluate to a scalar list — leave verbatim.
                rewritable = false;
                break;
            }
        }
        if !rewritable {
            continue;
        }
        // Degenerate case: every entry was an empty spread, so the list would
        // become empty (`SELECT ...[] FROM t` → `SELECT  FROM t`, which is not
        // valid SQL). Leave it verbatim rather than emit broken SQL — a SELECT
        // with no projected columns is unbuildable either way.
        if items.is_empty() {
            continue;
        }

        // Preserve the node's surrounding whitespace exactly — the SELECT_LIST
        // range can include trailing trivia (the newline before FROM) — and
        // replace only the content with the joined items.
        let original = node.text().to_string();
        let lead = &original[..original.len() - original.trim_start().len()];
        let trail = &original[original.trim_end().len()..];
        let replacement = format!("{lead}{}{trail}", items.join(", "));
        let range = node.text_range();
        edits.push((range.start().into(), range.end().into(), replacement));
    }

    if edits.is_empty() {
        return sql.to_string();
    }

    // Apply edits from the highest offset down so earlier offsets stay valid.
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = sql.to_string();
    for (start, end, repl) in edits {
        out.replace_range(start..end, &repl);
    }
    out
}

/// True if the `LIST_SPREAD` node's operand list literal contains a nested
/// spread element (`...[a, ...xs, b]`) — a shape this pass does not yet lower.
/// The spread node's own `LIST_SPREAD` is the first in its subtree; any further
/// `LIST_SPREAD` descendant is a nested spread inside the operand literal.
fn has_nested_spread(spread_node: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    spread_node
        .descendants()
        .filter(|n| n.kind() == LIST_SPREAD)
        .count()
        > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty `vars` map for tests that don't exercise `config.var`.
    fn no_vars() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Run only the spread pass with an empty (vars-only) context.
    fn spreads(sql: &str) -> String {
        let v = no_vars();
        expand_select_list_spreads(sql, &MetaEvalContext::vars_only(&v))
    }

    #[test]
    fn literal_spread_expands_to_items() {
        let sql = "SELECT\n    id,\n    ...[1, 2, 3]\nFROM smelt.sources.raw.users\n";
        let out = spreads(sql);
        assert_eq!(
            out,
            "SELECT\n    id, 1, 2, 3\nFROM smelt.sources.raw.users\n"
        );
    }

    #[test]
    fn column_ref_spread_expands_to_items() {
        let sql = "SELECT\n    id,\n    ...[name, email]\nFROM t\n";
        let out = spreads(sql);
        assert_eq!(out, "SELECT\n    id, name, email\nFROM t\n");
    }

    #[test]
    fn multi_spread_with_surrounding_items() {
        let sql = "SELECT\n    id,\n    ...[name, email],\n    created_at\nFROM t\n";
        let out = spreads(sql);
        assert_eq!(out, "SELECT\n    id, name, email, created_at\nFROM t\n");
    }

    #[test]
    fn empty_spread_elides_with_adjacent_commas() {
        let sql = "SELECT\n    id,\n    ...[],\n    created_at\nFROM t\n";
        let out = spreads(sql);
        assert_eq!(out, "SELECT\n    id, created_at\nFROM t\n");
    }

    #[test]
    fn no_spread_is_unchanged() {
        let sql = "SELECT id, name FROM t\n";
        let v = no_vars();
        assert_eq!(spreads(sql), sql);
        assert_eq!(
            expand_in_model_meta(sql, &MetaEvalContext::vars_only(&v)),
            sql
        );
    }

    #[test]
    fn idempotent_second_pass_is_noop() {
        let sql = "SELECT id, ...[a, b] FROM t";
        let once = spreads(sql);
        assert_eq!(once, "SELECT id, a, b FROM t");
        assert_eq!(spreads(&once), once);
    }

    #[test]
    fn all_empty_spread_list_left_verbatim() {
        // A SELECT whose only items are empty spreads would rebuild to an
        // empty list (`SELECT  FROM t`); leave it verbatim instead.
        let sql = "SELECT ...[] FROM t";
        assert_eq!(spreads(sql), sql);
    }

    #[test]
    fn spread_of_piped_list_expands_correctly() {
        // `...[a, b, c] |> map(fn c => c)` — spread operand is a PIPE_EXPR
        // (D-15: spread is the outermost operator). The piped list still lowers
        // to its scalar elements via eval_meta.
        assert_eq!(
            spreads("SELECT id, ...[a, b, c] |> map(fn c => c) FROM t"),
            "SELECT id, a, b, c FROM t"
        );
    }

    #[test]
    fn non_literal_spread_operand_left_verbatim() {
        // A `List<T>` variable spread is not yet lowered here; the list is
        // passed through unchanged rather than partially rewritten.
        let sql = "SELECT id, ...xs FROM t";
        assert_eq!(spreads(sql), sql);
    }

    #[test]
    fn guard_skips_reparse_without_meta_token() {
        // expand_in_model_meta short-circuits when there is no meta construct.
        let sql = "SELECT a, b, c FROM t";
        let v = no_vars();
        assert_eq!(
            expand_in_model_meta(sql, &MetaEvalContext::vars_only(&v)),
            sql
        );
    }

    // ── P6 build-path evaluator tests ────────────────────────────────────────

    fn eval(sql: &str, vars: &BTreeMap<String, String>) -> String {
        expand_in_model_meta(sql, &MetaEvalContext::vars_only(vars))
    }

    #[test]
    fn reduce_plus_chain_folds_with_plus() {
        assert_eq!(
            eval("SELECT reduce([1, 2, 3], plus_chain)", &no_vars()),
            "SELECT 1 + 2 + 3"
        );
    }

    #[test]
    fn reduce_and_all_folds_with_and() {
        // Each element is parenthesised so a non-atomic element cannot
        // re-associate the fold (the AND binds the whole element, not its tail).
        assert_eq!(
            eval("SELECT reduce([true, false, true], and_all)", &no_vars()),
            "SELECT (true) AND (false) AND (true)"
        );
    }

    #[test]
    fn reduce_and_all_parenthesises_non_atomic_elements() {
        // `[true OR false, false]` folded under AND must stay grouped as
        // `(true OR false) AND (false)` — bare ` AND ` would mis-parse as
        // `true OR (false AND false)` and silently flip the result.
        assert_eq!(
            eval("SELECT reduce([true OR false, false], and_all)", &no_vars()),
            "SELECT (true OR false) AND (false)"
        );
    }

    #[test]
    fn reduce_concat_with_uses_separator_literal() {
        assert_eq!(
            eval(
                "SELECT reduce(['alpha', 'beta', 'gamma'], concat_with(' OR '))",
                &no_vars()
            ),
            "SELECT 'alpha' || ' OR ' || 'beta' || ' OR ' || 'gamma'"
        );
    }

    #[test]
    fn pipe_filter_map_then_reduce() {
        // [1,2,3] |> filter(c>0) |> map(c*2) = [2,4,6]; plus_chain folds to +.
        assert_eq!(
            eval(
                "SELECT reduce([1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2), plus_chain)",
                &no_vars()
            ),
            "SELECT 1 * 2 + 2 * 2 + 3 * 2"
        );
    }

    #[test]
    fn filter_drops_failing_elements() {
        // filter keeps only c > 1, then plus_chain folds the survivors.
        assert_eq!(
            eval(
                "SELECT reduce([1, 2, 3] |> filter(fn c => c > 1), plus_chain)",
                &no_vars()
            ),
            "SELECT 2 + 3"
        );
    }

    #[test]
    fn config_var_resolves_to_text_literal() {
        let mut vars = BTreeMap::new();
        vars.insert("region".to_string(), "us-west-2".to_string());
        vars.insert("min_revenue".to_string(), "100".to_string());
        assert_eq!(
            eval(
                "SELECT smelt.config.var('region'), smelt.config.var('min_revenue')",
                &vars
            ),
            "SELECT 'us-west-2', '100'"
        );
    }

    #[test]
    fn config_var_absent_left_verbatim() {
        // Analyzer gates ConfigVarNotFound; the evaluator leaves it untouched.
        let sql = "SELECT smelt.config.var('missing')";
        assert_eq!(eval(sql, &no_vars()), sql);
    }

    #[test]
    fn ternary_picks_branch_from_config_var() {
        let mut vars = BTreeMap::new();
        vars.insert("env".to_string(), "dev".to_string());
        assert_eq!(
            eval(
                "SELECT if smelt.config.var('env') = 'prod' then 'strict' else 'permissive'",
                &vars
            ),
            "SELECT 'permissive'"
        );
        let mut prod = BTreeMap::new();
        prod.insert("env".to_string(), "prod".to_string());
        assert_eq!(
            eval(
                "SELECT if smelt.config.var('env') = 'prod' then 'strict' else 'permissive'",
                &prod
            ),
            "SELECT 'strict'"
        );
    }

    #[test]
    fn reduce_idempotent_second_pass() {
        let once = eval("SELECT reduce([1, 2, 3], plus_chain)", &no_vars());
        assert_eq!(eval(&once, &no_vars()), once);
    }

    #[test]
    fn multiple_select_lists_each_expanded() {
        let sql = "SELECT ...[a, b] FROM (SELECT ...[c, d] FROM t) s";
        let out = spreads(sql);
        assert_eq!(out, "SELECT a, b FROM (SELECT c, d FROM t) s");
    }

    // ── P7a build-path reflection (`smelt.columns_of`) tests ──────────────────

    /// A `base` model with id (numeric), label (non-numeric), amount (numeric).
    fn base_columns() -> BTreeMap<String, Vec<ColumnRefMeta>> {
        let mut m = BTreeMap::new();
        m.insert(
            "base".to_string(),
            vec![
                ColumnRefMeta {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    is_numeric: true,
                    is_decimal: false,
                    is_string: false,
                    is_temporal: false,
                    is_integer: true,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "label".to_string(),
                    type_name: "VARCHAR".to_string(),
                    is_numeric: false,
                    is_decimal: false,
                    is_string: true,
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "amount".to_string(),
                    type_name: "DOUBLE".to_string(),
                    is_numeric: true,
                    is_decimal: false,
                    is_string: false,
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: false,
                },
            ],
        );
        m
    }

    /// Evaluate with a column-metadata + function-body context.
    fn eval_reflect(
        sql: &str,
        columns: BTreeMap<String, Vec<ColumnRefMeta>>,
        fn_bodies: &FnBodyMap,
    ) -> String {
        let vars = no_vars();
        let ctx = MetaEvalContext {
            vars: &vars,
            columns,
            fn_bodies: Some(fn_bodies),
            models: &[],
            sources: &[],
            loaders: empty_loaders(),
        };
        expand_in_model_meta(sql, &ctx)
    }

    /// The `numeric_names` function body: columns_of → filter(numeric) → map(name).
    fn numeric_names_bodies() -> FnBodyMap {
        let mut m = FnBodyMap::new();
        m.insert(
            "numeric_names".to_string(),
            (
                vec![("t".to_string(), None)],
                "smelt.columns_of(t)\n      |> filter(fn c => c.is_numeric)\n      |> map(fn c => c.name)"
                    .to_string(),
            ),
        );
        m
    }

    #[test]
    fn columns_of_via_function_filter_map_spread() {
        // `...smelt.functions.numeric_names(smelt.base)` lowers to the numeric
        // column names of `base` (id, amount) spread into the SELECT list.
        let out = eval_reflect(
            "SELECT ...smelt.functions.numeric_names(smelt.base) FROM smelt.base",
            base_columns(),
            &numeric_names_bodies(),
        );
        assert_eq!(out, "SELECT id, amount FROM smelt.base");
    }

    #[test]
    fn columns_of_filter_keeps_only_numeric() {
        // A surrounding non-spread item is preserved; the spread contributes the
        // numeric columns only (label is dropped).
        let out = eval_reflect(
            "SELECT label, ...smelt.functions.numeric_names(smelt.base) FROM smelt.base",
            base_columns(),
            &numeric_names_bodies(),
        );
        assert_eq!(out, "SELECT label, id, amount FROM smelt.base");
    }

    #[test]
    fn columns_of_unresolvable_left_verbatim() {
        // `columns_of` on an entity absent from the context resolves to nothing —
        // the spread (and function call) are left untouched (analyzer drop-on-error).
        let sql = "SELECT ...smelt.functions.numeric_names(smelt.unknown) FROM smelt.unknown";
        let out = eval_reflect(sql, BTreeMap::new(), &numeric_names_bodies());
        // `columns_of(smelt.unknown)` does not resolve (entity absent from the
        // context), so the function-call spread operand does not evaluate to a
        // scalar list and the whole spread is left verbatim (analyzer drop-on-error).
        assert_eq!(out, sql, "unresolvable reflection spread is left verbatim");
    }

    // ── D-21: head-constructor predicates (is_decimal/is_string/…) ─────────────

    /// Columns covering each head-constructor family.
    ///
    /// Note: `Interval` is NOT in the `is_temporal` family per `meta_language.md`
    /// §ColumnRef (the temporal family = Date/Timestamp/Time only, not Interval).
    fn typed_columns() -> BTreeMap<String, Vec<ColumnRefMeta>> {
        let mut m = BTreeMap::new();
        m.insert(
            "typed".to_string(),
            vec![
                ColumnRefMeta {
                    name: "dec_col".to_string(),
                    type_name: "DECIMAL(10,2)".to_string(),
                    is_numeric: true,
                    is_decimal: true,
                    is_string: false,
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "txt_col".to_string(),
                    type_name: "VARCHAR".to_string(),
                    is_numeric: false,
                    is_decimal: false,
                    is_string: true,
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "ts_col".to_string(),
                    type_name: "TIMESTAMP".to_string(),
                    is_numeric: false,
                    is_decimal: false,
                    is_string: false,
                    is_temporal: true,
                    is_integer: false,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "interval_col".to_string(),
                    type_name: "INTERVAL".to_string(),
                    is_numeric: false,
                    is_decimal: false,
                    is_string: false,
                    // Interval is NOT in the temporal family for the ColumnRef predicate
                    // (meta_language.md §ColumnRef: temporal = Date/Timestamp/Time only).
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "int_col".to_string(),
                    type_name: "INTEGER".to_string(),
                    is_numeric: true,
                    is_decimal: false,
                    is_string: false,
                    is_temporal: false,
                    is_integer: true,
                    is_boolean: false,
                },
                ColumnRefMeta {
                    name: "bool_col".to_string(),
                    type_name: "BOOLEAN".to_string(),
                    is_numeric: false,
                    is_decimal: false,
                    is_string: false,
                    is_temporal: false,
                    is_integer: false,
                    is_boolean: true,
                },
            ],
        );
        m
    }

    #[test]
    fn is_decimal_predicate_filters_decimal_columns() {
        let bodies = {
            let mut m = FnBodyMap::new();
            m.insert(
                "decimal_names".to_string(),
                (
                    vec![("t".to_string(), None)],
                    "smelt.columns_of(t) |> filter(fn c => c.is_decimal) |> map(fn c => c.name)"
                        .to_string(),
                ),
            );
            m
        };
        let out = eval_reflect(
            "SELECT ...smelt.functions.decimal_names(smelt.typed) FROM smelt.typed",
            typed_columns(),
            &bodies,
        );
        assert_eq!(out, "SELECT dec_col FROM smelt.typed");
    }

    #[test]
    fn is_string_predicate_filters_string_columns() {
        let bodies = {
            let mut m = FnBodyMap::new();
            m.insert(
                "string_names".to_string(),
                (
                    vec![("t".to_string(), None)],
                    "smelt.columns_of(t) |> filter(fn c => c.is_string) |> map(fn c => c.name)"
                        .to_string(),
                ),
            );
            m
        };
        let out = eval_reflect(
            "SELECT ...smelt.functions.string_names(smelt.typed) FROM smelt.typed",
            typed_columns(),
            &bodies,
        );
        assert_eq!(out, "SELECT txt_col FROM smelt.typed");
    }

    #[test]
    fn is_temporal_predicate_excludes_interval() {
        // meta_language.md §ColumnRef: temporal family = Date/Timestamp/Time only.
        // Interval must NOT be included even though DataType::is_temporal() returns true
        // for Interval (that method is for the type-constraint Temporal set, not this predicate).
        let bodies = {
            let mut m = FnBodyMap::new();
            m.insert(
                "temporal_names".to_string(),
                (
                    vec![("t".to_string(), None)],
                    "smelt.columns_of(t) |> filter(fn c => c.is_temporal) |> map(fn c => c.name)"
                        .to_string(),
                ),
            );
            m
        };
        let out = eval_reflect(
            "SELECT ...smelt.functions.temporal_names(smelt.typed) FROM smelt.typed",
            typed_columns(),
            &bodies,
        );
        // ts_col (TIMESTAMP) → true; interval_col (INTERVAL) → false per spec.
        assert_eq!(out, "SELECT ts_col FROM smelt.typed");
    }

    #[test]
    fn is_integer_predicate_filters_integer_columns() {
        let bodies = {
            let mut m = FnBodyMap::new();
            m.insert(
                "integer_names".to_string(),
                (
                    vec![("t".to_string(), None)],
                    "smelt.columns_of(t) |> filter(fn c => c.is_integer) |> map(fn c => c.name)"
                        .to_string(),
                ),
            );
            m
        };
        let out = eval_reflect(
            "SELECT ...smelt.functions.integer_names(smelt.typed) FROM smelt.typed",
            typed_columns(),
            &bodies,
        );
        assert_eq!(out, "SELECT int_col FROM smelt.typed");
    }

    // ── P7b build-path wide reflection (`smelt.models.*` / `smelt.sources.*`) ──

    /// Three cohort models (path-sorted), two tagged `cohort`.
    fn cohort_models() -> Vec<EntityRefMeta> {
        vec![
            EntityRefMeta {
                path: "models/cohort_a.sql".to_string(),
                name: "cohort_a".to_string(),
                tags: vec!["cohort".to_string(), "core".to_string()],
            },
            EntityRefMeta {
                path: "models/cohort_b.sql".to_string(),
                name: "cohort_b".to_string(),
                tags: vec!["cohort".to_string()],
            },
            EntityRefMeta {
                path: "models/other.sql".to_string(),
                name: "other".to_string(),
                tags: vec!["misc".to_string()],
            },
        ]
    }

    /// Two sources, one tagged `audit`.
    fn audit_sources() -> Vec<EntityRefMeta> {
        vec![
            EntityRefMeta {
                path: "models/sources/raw/events.yml".to_string(),
                name: "events".to_string(),
                tags: vec!["audit".to_string(), "raw".to_string()],
            },
            EntityRefMeta {
                path: "models/sources/raw/clicks.yml".to_string(),
                name: "clicks".to_string(),
                tags: vec!["raw".to_string()],
            },
        ]
    }

    /// Evaluate with workspace model / source listings.
    fn eval_wide(sql: &str, models: Vec<EntityRefMeta>, sources: Vec<EntityRefMeta>) -> String {
        let vars = no_vars();
        let ctx = MetaEvalContext {
            vars: &vars,
            columns: BTreeMap::new(),
            fn_bodies: None,
            models: &models,
            sources: &sources,
            loaders: empty_loaders(),
        };
        expand_in_model_meta(sql, &ctx)
    }

    #[test]
    fn models_with_tag_map_name_spread_to_literals() {
        // `...map(smelt.models.with_tag('cohort'), fn m => m.name)` lowers to
        // one string-literal select item per matching model (cohort_a, cohort_b),
        // path-sorted; `other` (tag misc) is excluded.
        let out = eval_wide(
            "SELECT ...map(smelt.models.with_tag('cohort'), fn m => m.name)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(out, "SELECT 'cohort_a', 'cohort_b'");
    }

    #[test]
    fn sources_with_tag_map_name_spread_to_literals() {
        // The plan's worked example: list audit source names.
        let out = eval_wide(
            "SELECT ...map(smelt.sources.with_tag('audit'), fn s => s.name)",
            Vec::new(),
            audit_sources(),
        );
        assert_eq!(out, "SELECT 'events'");
    }

    #[test]
    fn models_with_tag_map_path_spread_to_literals() {
        // The `m.path` projection renders the workspace-relative path literal.
        let out = eval_wide(
            "SELECT ...map(smelt.models.with_tag('cohort'), fn m => m.path)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(out, "SELECT 'models/cohort_a.sql', 'models/cohort_b.sql'");
    }

    #[test]
    fn models_all_call_form_spread() {
        // `smelt.models.all()` (call form) returns the whole listing.
        let out = eval_wide(
            "SELECT ...map(smelt.models.all(), fn m => m.name)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(out, "SELECT 'cohort_a', 'cohort_b', 'other'");
    }

    #[test]
    fn models_all_bare_ref_left_verbatim() {
        // A *bare* `smelt.models.all` (no parens) is not wide reflection to the
        // analyzer — it resolves as an ordinary model ref and is rejected as
        // `UndefinedModelRef`. The build only lowers the call form, so the bare
        // form is left verbatim and the analyzer gate refuses the build (build-
        // accepted surface == analyzer-accepted surface, per the parity rule).
        let sql = "SELECT ...map(smelt.models.all, fn m => m.name)";
        assert_eq!(eval_wide(sql, cohort_models(), Vec::new()), sql);
    }

    #[test]
    fn with_tag_config_var_argument_resolved() {
        // The analyzer accepts `smelt.config.var(...)` as a compile-time-Text
        // `with_tag` argument. The earlier `expand_config_vars` pass resolves it
        // to a literal before wide reflection runs, so `with_tag(config.var(x))`
        // sees a plain string literal and filters as expected.
        let mut vars = BTreeMap::new();
        vars.insert("seg".to_string(), "cohort".to_string());
        let models = cohort_models();
        let ctx = MetaEvalContext {
            vars: &vars,
            columns: BTreeMap::new(),
            fn_bodies: None,
            models: &models,
            sources: &[],
            loaders: empty_loaders(),
        };
        let out = expand_in_model_meta(
            "SELECT reduce(map(smelt.models.with_tag(smelt.config.var('seg')), fn m => m.name), concat_with(', '))",
            &ctx,
        );
        assert_eq!(out, "SELECT 'cohort_a' || ', ' || 'cohort_b'");
    }

    #[test]
    fn all_then_filter_then_map_keeps_only_matches() {
        // A `filter` over the reflected list composes with `all()`.
        let out = eval_wide(
            "SELECT ...map(smelt.models.all() |> filter(fn m => m.name = 'cohort_a'), fn m => m.name)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(out, "SELECT 'cohort_a'");
    }

    #[test]
    fn empty_match_left_verbatim() {
        // No model matches `nonexistent` → the spread would rebuild to an empty
        // SELECT list, so it is left verbatim (analyzer gates the empty result).
        let sql = "SELECT ...map(smelt.models.with_tag('nonexistent'), fn m => m.name)";
        assert_eq!(eval_wide(sql, cohort_models(), Vec::new()), sql);
    }

    #[test]
    fn wide_reflection_idempotent_second_pass() {
        let once = eval_wide(
            "SELECT ...map(smelt.sources.with_tag('audit'), fn s => s.name)",
            Vec::new(),
            audit_sources(),
        );
        assert_eq!(eval_wide(&once, Vec::new(), audit_sources()), once);
    }

    // ── D-16: ModelRef/SourceRef name/path identifier-lift carve-out ─────────────

    #[test]
    fn model_ref_name_renders_as_string_literal() {
        // D-16: `m.name` renders as a SQL *string literal* even in a SELECT
        // expression (column-reference) position — the position where an
        // unquoted identifier would be a column reference.  Wide-reflection
        // Text from ModelRef/SourceRef name/path is a data value carved out of
        // the four-position identifier lift (meta_language.md §"Meta-Text-as-
        // identifier lift" rule 8).  The quoted form confirms the carve-out:
        // if `m.name` were lifted to a bare identifier the way `c.name` is, we
        // would see `SELECT cohort_a, cohort_b` instead.
        let out = eval_wide(
            "SELECT ...map(smelt.models.with_tag('cohort'), fn m => m.name)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(
            out, "SELECT 'cohort_a', 'cohort_b'",
            "m.name must render as a quoted string literal, not a bare identifier (D-16 carve-out)"
        );

        // `m.path` likewise: generator-file path is data, not a column name.
        let out_path = eval_wide(
            "SELECT ...map(smelt.models.with_tag('cohort'), fn m => m.path)",
            cohort_models(),
            Vec::new(),
        );
        assert_eq!(
            out_path, "SELECT 'models/cohort_a.sql', 'models/cohort_b.sql'",
            "m.path must render as a quoted string literal (D-16)"
        );
    }

    #[test]
    fn column_ref_name_still_lifts_to_identifier() {
        // D-16 contrast: `ColumnRef.name` in the same SELECT expression
        // (column-reference) position lifts to a bare identifier.  The carve-
        // out in rule 8 is specific to ModelRef/SourceRef name/path;
        // `ColumnRef.name` is a column name and IS subject to the four-
        // position identifier lift (meta_language.md §"Reflection:
        // smelt.columns_of, ColumnRef, identifier lift" rule 6).
        let bodies = FnBodyMap::new();
        let out = eval_reflect(
            "SELECT ...map(smelt.columns_of(smelt.base), fn c => c.name)",
            base_columns(),
            &bodies,
        );
        assert_eq!(
            out,
            "SELECT id, label, amount",
            "ColumnRef.name must lift to a bare identifier in SELECT position (not a quoted string)"
        );
    }

    // ── P7c build-path config-loader (`smelt.config.load_yaml`) tests ─────────

    /// A `cohorts.yaml` with three records (mixed Text / Integer fields).
    fn cohorts_loaders() -> BTreeMap<String, LoaderFileMeta> {
        let mut m = BTreeMap::new();
        m.insert(
            "configs/cohorts.yaml".to_string(),
            LoaderFileMeta {
                text: "- name: us_west\n  region: us-west-2\n  threshold: 100\n\
                       - name: us_east\n  region: us-east-1\n  threshold: 100\n\
                       - name: eu\n  region: eu-west-1\n  threshold: 50\n"
                    .to_string(),
                exists: true,
            },
        );
        m
    }

    /// Evaluate with a loader-file context.
    fn eval_load(sql: &str, loaders: &BTreeMap<String, LoaderFileMeta>) -> String {
        let vars = no_vars();
        let ctx = MetaEvalContext {
            vars: &vars,
            columns: BTreeMap::new(),
            fn_bodies: None,
            models: &[],
            sources: &[],
            loaders,
        };
        expand_in_model_meta(sql, &ctx)
    }

    #[test]
    fn loader_list_map_field_spread_to_literals() {
        // `...load_yaml(...) |> map(fn c => c.region)` lowers to one string
        // literal per loaded record's `region` field.
        let out = eval_load(
            "SELECT ...smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>) |> map(fn c => c.region)",
            &cohorts_loaders(),
        );
        assert_eq!(out, "SELECT 'us-west-2', 'us-east-1', 'eu-west-1'");
    }

    #[test]
    fn loader_list_map_integer_field_spread() {
        // An Integer field projects to a bare numeric literal.
        let out = eval_load(
            "SELECT ...smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>) |> map(fn c => c.threshold)",
            &cohorts_loaders(),
        );
        assert_eq!(out, "SELECT 100, 100, 50");
    }

    #[test]
    fn loader_list_reduce_concat_folds_to_scalar() {
        // `reduce(map(load_yaml(...), fn c => c.region), concat_with(', '))`
        // folds the loaded regions into a single scalar — the executable form.
        let out = eval_load(
            "SELECT reduce(map(smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>), fn c => c.region), concat_with(', ')) AS regions",
            &cohorts_loaders(),
        );
        assert_eq!(
            out,
            "SELECT 'us-west-2' || ', ' || 'us-east-1' || ', ' || 'eu-west-1' AS regions"
        );
    }

    #[test]
    fn loader_missing_file_left_verbatim() {
        // A loader path absent from the context resolves to nothing — the
        // spread is left verbatim (analyzer drop-on-error).
        let sql = "SELECT ...smelt.config.load_yaml('configs/missing.yaml', List<{name: Text}>) |> map(fn c => c.name)";
        assert_eq!(eval_load(sql, &BTreeMap::new()), sql);
    }

    #[test]
    fn loader_map_schema_left_verbatim() {
        // A `Map<…>`-schema loader is not yet lowered at build (Known
        // Divergence) — the call is left verbatim.
        let mut m = BTreeMap::new();
        m.insert(
            "configs/tenants.yaml".to_string(),
            LoaderFileMeta {
                text: "tenant_a:\n  plan: pro\n".to_string(),
                exists: true,
            },
        );
        let sql = "SELECT reduce(smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text}>) |> m => m.keys(), concat_with(', ')) AS names";
        assert_eq!(eval_load(sql, &m), sql);
    }

    #[test]
    fn loader_idempotent_second_pass() {
        let once = eval_load(
            "SELECT reduce(map(smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>), fn c => c.region), concat_with(', ')) AS regions",
            &cohorts_loaders(),
        );
        assert_eq!(eval_load(&once, &cohorts_loaders()), once);
    }
}
