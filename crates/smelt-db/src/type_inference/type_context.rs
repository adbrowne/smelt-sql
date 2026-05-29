//! Type context: source/model/CTE/alias bookkeeping used by all type inference.
//!
#![allow(unused_imports)]
//! Extracted from `type_inference.rs` — pure functions, no Salsa.

use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct TypeContext {
    // NOTE: PartialEq, Eq, Clone are implemented manually below to handle missed_lookups
    /// Source columns: source_name.table_name.column_name -> type
    source_columns: HashMap<String, TypedColumn>,
    /// Model columns: model_name.column_name -> type
    model_columns: HashMap<String, TypedColumn>,
    /// CTE columns: cte_name.column_name -> type
    cte_columns: HashMap<String, TypedColumn>,
    /// Known CTE names (for checking if a qualifier is a CTE)
    cte_names: std::collections::HashSet<String>,
    /// Aliases in scope: alias -> qualified name
    aliases: HashMap<String, String>,
    /// Bound function parameters (param name → type). Shadows all SQL scopes
    /// per §16 #1 of `docs/research/20260413-smelt-functions.md`: params
    /// resolve **before** any SQL scope. Seeded by the Phase 5
    /// `check_function_body` pure function when checking a `smelt.define`
    /// body. Unqualified lookups via [`TypeContext::lookup_identifier`]
    /// check this map first.
    function_params: HashMap<String, TypedColumn>,
    /// Workspace function-signature index (name → signature), populated by
    /// Phase 6 callers so `infer_expression_type` can resolve the return
    /// type of a `smelt.fn.<name>(...)` call site. Empty for non-call
    /// contexts — pure type inference doesn't care whether the resolver
    /// came from Salsa or an in-memory map.
    function_signatures: HashMap<String, FunctionSig>,
    /// `TableExpr`-parameter name → caller-supplied columns (Phase 15).
    /// Populated at call-site expansion so the body's SQL FROM scope
    /// sees bare column names from the caller's schema, and shadow-
    /// warning detection can enumerate columns per parameter.
    tableexpr_param_schemas: HashMap<String, Vec<(String, TypedColumn)>>,
    /// Per-call row-variable bindings produced by a successful
    /// [`smelt_types::signatures::check_schema_requirement`] against a
    /// named row tail (`..r`). Phase 16 records the captured extras
    /// so later phases (17 / 37) can reference the variable from the
    /// body and return type. In Phase 16 itself the map is write-only
    /// from the call-site's point of view — body expressions cannot
    /// reference `r` yet, so no lookup path consults it.
    ///
    /// Doc-hidden pub accessor; exposed for unit tests only.
    row_var_env: HashMap<String, Vec<(String, smelt_types::DataType)>>,
    /// CTE names whose output schema could not be determined (e.g. a CTE
    /// that does `SELECT * FROM smelt.fn.some_fn(...)` where the function's
    /// output columns aren't known at pure-function-check time). Any unqualified
    /// column lookup that otherwise fails will match against these names and
    /// return `Unknown` instead of failing — suppressing false-positive
    /// `UnknownIdentifier` diagnostics for columns that come from opaque-schema
    /// CTEs. Introduced in Phase 22 of smelt-functions.
    opaque_ctes: std::collections::HashSet<String>,
    /// Fragment-typed parameters (`SelectItems<Kind>`) seeded during function
    /// body checks. Maps param name → declared [`ExprKind`] so that
    /// [`infer_expression_kind`] can return the correct kind for bare
    /// references to these params inside `PASSING` bodies (Phase 44b).
    ///
    /// A bare identifier that matches a key in this map inherits its declared
    /// kind instead of falling through to the default `Scalar` return value.
    fragment_param_kinds: HashMap<String, ExprKind>,
    /// Expected return type for the expression currently being inferred
    /// (bidirectional inference, Phase 27, §16 #14 Decision 14).
    ///
    /// Set by the caller when a specific return type is expected (e.g. a
    /// `Tier2CallSite(expected_ret)` check mode). `try_registry_inference`
    /// passes this to [`unify_call_with_expected`] so that a built-in generic
    /// call in a checking context can widen its type-variable binding to match
    /// the expected type (e.g. `COALESCE(1, 2)` in a `Double` context yields
    /// `Double` rather than `Integer`).
    ///
    /// `None` in all other contexts — preserves the pre-Phase-27 behaviour.
    pub expected_return: Option<DataType>,
    /// Lambda parameters pushed onto the context during Phase B HOF body inference
    /// (Phase B meta-language). Maps parameter name → typed column. Lambda
    /// parameters shadow `function_params` and all SQL scopes per spec
    /// `scoping.md` §"Resolution order" (lambda scope is innermost).
    ///
    /// Seeded by [`TypeContext::add_lambda_param`] before re-inferring the lambda
    /// body; cleared by removing the binding after the body walk (or cloning a
    /// context snapshot).
    lambda_params: HashMap<String, TypedColumn>,
    /// Full `SmeltType` for function parameters whose declared type is not
    /// representable as a plain `DataType` (Phase B meta-language).
    ///
    /// `function_params` stores only the `DataType` projection (e.g. `Unknown`
    /// for `List<T>` parameters).  This parallel map preserves the full
    /// `SmeltType` so that HOF inference can look up a bare identifier like
    /// `xs` and learn it is `List<Expr<Integer>>` rather than `Unknown`.
    ///
    /// Populated by [`TypeContext::add_function_param_smelt_type`].
    /// Consulted in the non-literal first-argument path of
    /// [`infer_hof_call_from_function_call_with_expected`].
    function_param_smelt_types: HashMap<String, SmeltType>,
    /// Workspace record declaration registry (Phase E1, Phase 3).
    ///
    /// Carries the set of `smelt.record` declarations visible to type inference.
    /// Empty by default — pre-Phase-5 callers do not populate it. Phase 5 will
    /// wire the Salsa side; Phase 3 establishes the shape and inference paths.
    ///
    /// `Arc` so that cloning `TypeContext` (e.g. for nested lambda scopes) does
    /// not copy the full registry. The registry is built once per workspace
    /// compilation pass and is immutable thereafter.
    record_registry: Arc<RecordRegistry>,
    /// Column lookups that returned None (for property-based test column detection)
    missed_lookups: Mutex<Vec<(Option<String>, String)>>,
    /// Whether the expression currently being type-checked lives inside a
    /// generator file (a file whose frontmatter declares `generates: models`).
    ///
    /// When `true`, `ModelDef { … }` record literals are permitted and do not
    /// emit `ModelDefOutsideGeneratorFile`. When `false` (the default), any
    /// `ModelDef` literal emits that diagnostic.
    ///
    /// Set by the Phase 4 Salsa caller (`infer_generator_file_body`) before
    /// delegating to the pure inference layer. Phase 3 tests set this field
    /// directly on a `TypeContext::new()` instance.
    pub is_inside_generator_file: bool,
    /// Whether the workspace shape (the set of hand-authored models) has been
    /// fully resolved at the time this context is used.
    ///
    /// When `true` (the default), literal `smelt.<path>` references inside a
    /// generator body resolve against the hand-authored-model set (excluding
    /// generator emissions — see spec Semantics rule 4). When `false`, smelt
    /// path references are left unresolved (used by Phase 3 pure tests that
    /// don't wire Salsa workspace state).
    pub workspace_shape_includes_generators: bool,
}

impl PartialEq for TypeContext {
    fn eq(&self, other: &Self) -> bool {
        self.source_columns == other.source_columns
            && self.model_columns == other.model_columns
            && self.cte_columns == other.cte_columns
            && self.cte_names == other.cte_names
            && self.aliases == other.aliases
            && self.function_params == other.function_params
            && self.lambda_params == other.lambda_params
            && self.function_signatures == other.function_signatures
            && self.tableexpr_param_schemas == other.tableexpr_param_schemas
            && self.row_var_env == other.row_var_env
            && self.opaque_ctes == other.opaque_ctes
            && self.fragment_param_kinds == other.fragment_param_kinds
            && self.expected_return == other.expected_return
            && self.function_param_smelt_types == other.function_param_smelt_types
            && self.is_inside_generator_file == other.is_inside_generator_file
            && self.workspace_shape_includes_generators == other.workspace_shape_includes_generators
        // record_registry and missed_lookups are intentionally excluded —
        // the registry is shared state and missed_lookups is transient tracking state.
    }
}

impl Eq for TypeContext {}

impl Default for TypeContext {
    fn default() -> Self {
        Self {
            source_columns: HashMap::new(),
            model_columns: HashMap::new(),
            cte_columns: HashMap::new(),
            cte_names: std::collections::HashSet::new(),
            aliases: HashMap::new(),
            function_params: HashMap::new(),
            lambda_params: HashMap::new(),
            function_signatures: HashMap::new(),
            tableexpr_param_schemas: HashMap::new(),
            row_var_env: HashMap::new(),
            opaque_ctes: std::collections::HashSet::new(),
            fragment_param_kinds: HashMap::new(),
            expected_return: None,
            function_param_smelt_types: HashMap::new(),
            record_registry: Arc::new(RecordRegistry::default()),
            missed_lookups: Mutex::new(Vec::new()),
            is_inside_generator_file: false,
            workspace_shape_includes_generators: true,
        }
    }
}

impl Clone for TypeContext {
    fn clone(&self) -> Self {
        Self {
            source_columns: self.source_columns.clone(),
            model_columns: self.model_columns.clone(),
            cte_columns: self.cte_columns.clone(),
            cte_names: self.cte_names.clone(),
            aliases: self.aliases.clone(),
            function_params: self.function_params.clone(),
            lambda_params: self.lambda_params.clone(),
            function_signatures: self.function_signatures.clone(),
            tableexpr_param_schemas: self.tableexpr_param_schemas.clone(),
            row_var_env: self.row_var_env.clone(),
            opaque_ctes: self.opaque_ctes.clone(),
            fragment_param_kinds: self.fragment_param_kinds.clone(),
            expected_return: self.expected_return.clone(),
            function_param_smelt_types: self.function_param_smelt_types.clone(),
            record_registry: Arc::clone(&self.record_registry),
            missed_lookups: Mutex::new(Vec::new()), // Don't clone tracking state
            is_inside_generator_file: self.is_inside_generator_file,
            workspace_shape_includes_generators: self.workspace_shape_includes_generators,
        }
    }
}

impl TypeContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source column to the context
    pub fn add_source_column(
        &mut self,
        source_name: &str,
        table_name: &str,
        column_name: &str,
        typed_column: TypedColumn,
    ) {
        let key = format!("{}.{}.{}", source_name, table_name, column_name);
        self.source_columns.insert(key, typed_column.clone());

        // Also add without source qualifier for simple lookups
        let simple_key = format!("{}.{}", table_name, column_name);
        self.source_columns
            .entry(simple_key)
            .or_insert(typed_column);
    }

    /// Add a model column to the context
    pub fn add_model_column(
        &mut self,
        model_name: &str,
        column_name: &str,
        typed_column: TypedColumn,
    ) {
        let key = format!("{}.{}", model_name, column_name);
        self.model_columns.insert(key, typed_column);
    }

    /// Add an alias mapping
    pub fn add_alias(&mut self, alias: &str, qualified_name: &str) {
        self.aliases
            .insert(alias.to_string(), qualified_name.to_string());
    }

    /// Mark a CTE as having an opaque (unknowable at analysis time) output
    /// schema. Any column lookup against this CTE will return
    /// `Unknown`-typed instead of failing. Used for CTEs whose body SELECTs
    /// from a `smelt.fn.*` call with a wildcard — we can't expand the
    /// wildcard without the function's body AST (Phase 22).
    pub fn mark_cte_opaque(&mut self, cte_name: &str) {
        self.opaque_ctes.insert(cte_name.to_string());
        // Also register as a CTE name so `is_cte` returns true.
        self.cte_names.insert(cte_name.to_string());
    }

    /// Return `true` when `cte_name` was marked as having an opaque schema
    /// (its output columns cannot be determined at pure-function-check time).
    pub fn is_opaque_cte(&self, cte_name: &str) -> bool {
        self.opaque_ctes.contains(cte_name)
    }

    /// Add a CTE column to the context
    pub fn add_cte_column(&mut self, cte_name: &str, column_name: &str, typed_column: TypedColumn) {
        let key = format!("{}.{}", cte_name, column_name);
        self.cte_columns.insert(key, typed_column);
        self.cte_names.insert(cte_name.to_string());
    }

    /// Check if a name is a known CTE
    pub fn is_cte(&self, name: &str) -> bool {
        self.cte_names.contains(name)
    }

    /// Resolve an alias to its qualified name
    pub fn resolve_alias(&self, alias: &str) -> Option<String> {
        self.aliases.get(alias).cloned()
    }

    /// Get all CTE names in scope
    pub fn cte_names(&self) -> impl Iterator<Item = &str> {
        self.cte_names.iter().map(|s| s.as_str())
    }

    /// Get columns for a specific CTE
    pub fn cte_columns(&self, cte_name: &str) -> Vec<(&str, &TypedColumn)> {
        let prefix = format!("{}.", cte_name);
        self.cte_columns
            .iter()
            .filter_map(move |(key, typed_col)| {
                key.strip_prefix(&prefix)
                    .map(|col_name| (col_name, typed_col))
            })
            .collect()
    }

    /// Check if a qualifier (table name/alias) resolves to a known source, model, or CTE.
    /// Returns a human-readable description like "source 'raw.sessions'" or "model 'upstream'".
    pub fn describe_qualifier(&self, qualifier: &str) -> Option<String> {
        // Check aliases first
        let resolved = self
            .aliases
            .get(qualifier)
            .map(|s| s.as_str())
            .unwrap_or(qualifier);

        // Check CTEs
        if self.cte_names.contains(resolved) {
            return Some(format!("CTE '{}'", resolved));
        }

        // Check model columns
        let model_prefix = format!("{}.", resolved);
        if self
            .model_columns
            .keys()
            .any(|k| k.starts_with(&model_prefix))
        {
            return Some(format!("model '{}'", resolved));
        }

        // Check source columns (could be table_name or source_name.table_name)
        for key in self.source_columns.keys() {
            if key.starts_with(&model_prefix) {
                return Some(format!("source '{}'", resolved));
            }
        }

        None
    }

    /// Look up a column type by name (with optional qualifier).
    /// CTEs shadow outer scope, so we check them first.
    /// Records missed lookups (when None is returned) for property-based test
    /// column detection via `take_missed_lookups()`.
    pub fn lookup_column(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        let result = self.lookup_column_inner(qualifier, name);
        if result.is_none() {
            if let Ok(mut lookups) = self.missed_lookups.lock() {
                lookups.push((qualifier.map(|s| s.to_string()), name.to_string()));
            }
        }
        result
    }

    // Static sentinel for opaque-CTE fallback returns — avoids creating a
    // `TypedColumn` on the heap for every opaque-CTE column lookup.
    fn opaque_column() -> &'static TypedColumn {
        static OPAQUE: std::sync::OnceLock<TypedColumn> = std::sync::OnceLock::new();
        OPAQUE.get_or_init(|| TypedColumn {
            data_type: DataType::Unknown,
            nullable: true,
        })
    }

    fn lookup_column_inner(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        // If we have a qualifier, use it directly
        if let Some(q) = qualifier {
            // Check if qualifier is an alias
            let resolved_qualifier = self.aliases.get(q).map(|s| s.as_str()).unwrap_or(q);

            // Try CTE columns first (CTEs shadow outer scope)
            let cte_key = format!("{}.{}", resolved_qualifier, name);
            if let Some(t) = self.cte_columns.get(&cte_key) {
                return Some(t);
            }

            // Phase 22: if the qualifier resolves to an opaque CTE (one
            // whose schema couldn't be inferred from a `smelt.fn.*` call),
            // return Unknown rather than failing — we can't validate column
            // names against an unknown schema.
            if self.opaque_ctes.contains(resolved_qualifier) {
                return Some(Self::opaque_column());
            }

            // Try model columns
            let model_key = format!("{}.{}", resolved_qualifier, name);
            if let Some(t) = self.model_columns.get(&model_key) {
                return Some(t);
            }

            // Try source columns
            if let Some(t) = self.source_columns.get(&model_key) {
                return Some(t);
            }

            // Try with full source path
            for (key, typed_col) in &self.source_columns {
                if key.ends_with(&format!("{}.{}", resolved_qualifier, name)) {
                    return Some(typed_col);
                }
            }
        }

        // Unqualified lookup - search all sources
        // First try CTE columns (CTEs shadow outer scope)
        for (key, typed_col) in &self.cte_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        // Then try model columns
        for (key, typed_col) in &self.model_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        // Then try source columns
        for (key, typed_col) in &self.source_columns {
            if key.ends_with(&format!(".{}", name)) {
                return Some(typed_col);
            }
        }

        // Phase 22: if there are any opaque CTEs in scope, an unqualified
        // column lookup that failed all known scopes may still be valid —
        // it could reference a column from the opaque CTE's unknown schema.
        // Return Unknown rather than None to suppress false-positive
        // `UnknownIdentifier` diagnostics in function bodies that SELECT
        // from `smelt.fn.*`-derived CTEs.
        if !self.opaque_ctes.is_empty() {
            return Some(Self::opaque_column());
        }

        None
    }

    /// Seed a function parameter binding into the context.
    ///
    /// Phase 5 hinge: `check_function_body` calls this once per declared
    /// parameter before re-checking the body. Subsequent unqualified lookups
    /// via [`TypeContext::lookup_identifier`] will return the bound type
    /// instead of falling through to source/model/CTE scopes.
    ///
    /// Pure — no Salsa interaction.
    pub fn add_function_param(&mut self, name: &str, col: TypedColumn) {
        self.function_params.insert(name.to_string(), col);
    }

    /// Is `name` bound as a function parameter in this context?
    pub fn has_function_param(&self, name: &str) -> bool {
        self.function_params.contains_key(name)
    }

    /// Store the full [`SmeltType`] for a function parameter (Phase B
    /// meta-language).
    ///
    /// Called alongside [`add_function_param`] for parameters whose declared
    /// type is not representable as a plain `DataType` — specifically
    /// `List<T>` parameters.  The HOF non-literal first-argument path
    /// consults this map (via [`lookup_function_param_smelt_type`]) to recover
    /// the full `SmeltType::List(...)` that `DataType::Unknown` would otherwise
    /// discard.
    ///
    /// Pure — no Salsa interaction.
    pub fn add_function_param_smelt_type(&mut self, name: &str, ty: SmeltType) {
        self.function_param_smelt_types.insert(name.to_string(), ty);
    }

    /// Look up the full [`SmeltType`] for a function parameter by name.
    ///
    /// Returns `None` when `name` was not registered via
    /// [`add_function_param_smelt_type`].  The HOF non-literal path calls this
    /// before falling back to `infer_expression_type` so that a bare identifier
    /// naming a `List<T>` parameter resolves to `SmeltType::List(...)` rather
    /// than `SmeltType::Expr(Concrete(Unknown))`.
    pub fn lookup_function_param_smelt_type(&self, name: &str) -> Option<&SmeltType> {
        self.function_param_smelt_types.get(name)
    }

    /// Record that `name` is a `SelectItems<Kind>` fragment parameter with
    /// the given declared [`ExprKind`] (Phase 44b).
    ///
    /// After this call, [`infer_expression_kind`] will return `kind` for a
    /// bare unqualified identifier expression whose text matches `name`, so
    /// that forwarding a fragment-typed parameter through to an inner call's
    /// PASSING body doesn't produce spurious `FragmentKindMismatch` errors.
    ///
    /// Pure — no Salsa interaction.
    pub fn add_fragment_param_kind(&mut self, name: &str, kind: ExprKind) {
        self.fragment_param_kinds.insert(name.to_string(), kind);
    }

    /// Look up the declared [`ExprKind`] for a fragment-typed parameter by
    /// name (Phase 44b). Returns `None` when `name` was not registered via
    /// [`TypeContext::add_fragment_param_kind`].
    pub fn lookup_fragment_param_kind(&self, name: &str) -> Option<ExprKind> {
        self.fragment_param_kinds.get(name).copied()
    }

    /// Return `true` when `name` is a fragment-typed (SelectItems<Kind>)
    /// parameter registered in this context (Phase 44b).
    pub fn is_fragment_param(&self, name: &str) -> bool {
        self.fragment_param_kinds.contains_key(name)
    }

    /// Seed a `TableExpr` parameter's caller-supplied schema as a
    /// FROM-scope entry (Phase 15, §16 #7).
    ///
    /// The columns are registered both as bare-column lookups (so
    /// unqualified `revenue` inside the body resolves via
    /// [`TypeContext::lookup_column`]) and as qualified lookups under
    /// the parameter's name (so `source.revenue` resolves through the
    /// same path any other model alias would).
    ///
    /// Parameter names are also tracked separately via
    /// [`TypeContext::tableexpr_param_columns`] so the Phase-15 shadow
    /// warning checker can enumerate which columns a caller supplied.
    ///
    /// Pure — no Salsa interaction. Called at call-site expansion by the
    /// `smelt-functions` call-site checker.
    pub fn add_tableexpr_param(&mut self, param_name: &str, columns: &[(String, TypedColumn)]) {
        for (col_name, typed_col) in columns {
            // `add_model_column` keys on `param_name.col_name`; the
            // existing unqualified-suffix lookup in `lookup_column_inner`
            // then resolves bare `col_name` via this entry.
            self.add_model_column(param_name, col_name, typed_col.clone());
        }
        // Bind the parameter name as an alias to itself so
        // `describe_qualifier` / qualified lookups succeed without
        // special-casing.
        self.add_alias(param_name, param_name);
        // Record the schema separately — used by the shadow-warning
        // check which needs to enumerate columns by parameter without
        // round-tripping through `model_columns` keys.
        self.tableexpr_param_schemas
            .insert(param_name.to_string(), columns.to_vec());
    }

    /// Return the caller-supplied columns for a `TableExpr` parameter,
    /// if one with this name has been seeded via
    /// [`TypeContext::add_tableexpr_param`].
    pub fn tableexpr_param_columns(&self, param_name: &str) -> Option<&[(String, TypedColumn)]> {
        self.tableexpr_param_schemas
            .get(param_name)
            .map(|v| v.as_slice())
    }

    /// Iterate all `TableExpr` parameter schemas seeded in this context.
    pub fn tableexpr_param_schemas_iter(
        &self,
    ) -> impl Iterator<Item = (&str, &[(String, TypedColumn)])> {
        self.tableexpr_param_schemas
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Record a named row-variable binding in the per-call
    /// `row_var_env` (Phase 16).
    ///
    /// `name` is the variable's source name (e.g. `"r"`); `extras` is
    /// the ordered `(column_name, data_type)` list of caller columns
    /// captured by the row tail. Overwrites any previous binding for
    /// `name` — matching the simple "innermost wins" rule; later
    /// phases may introduce scope stacking if user code can actually
    /// reference `r`.
    ///
    /// Doc-hidden — Phase 16 only needs this to exist so subsequent
    /// phases can build atop it; user-visible binding behaviour lands
    /// in Phases 17+.
    #[doc(hidden)]
    pub fn set_row_var_binding(
        &mut self,
        name: &str,
        extras: Vec<(String, smelt_types::DataType)>,
    ) {
        self.row_var_env.insert(name.to_string(), extras);
    }

    /// Unit-test-only accessor for the named row-variable bindings
    /// recorded by [`TypeContext::set_row_var_binding`] (Phase 16).
    ///
    /// Returns the captured `(column_name, data_type)` pairs for the
    /// named row variable, or `None` when no binding exists. The
    /// [`doc(hidden)`] marker keeps this out of the public surface
    /// while still permitting cross-crate tests to observe the map
    /// — matches the project's existing pattern for internal hooks
    /// exposed for test access (e.g. property-test infrastructure).
    #[doc(hidden)]
    pub fn row_var_binding(&self, name: &str) -> Option<&[(String, smelt_types::DataType)]> {
        self.row_var_env.get(name).map(|v| v.as_slice())
    }

    /// Register a resolved [`FunctionSig`] so `smelt.fn.<name>` call-site
    /// type inference can look up the declared return type.
    ///
    /// Pure — no Salsa interaction. Phase 6 callers build the map by
    /// asking Salsa for each unique name that appears in the file.
    pub fn add_function_signature(&mut self, name: &str, sig: FunctionSig) {
        self.function_signatures.insert(name.to_string(), sig);
    }

    /// Resolve a `smelt.fn.<name>` reference to its [`FunctionSig`].
    pub fn lookup_function_signature(&self, name: &str) -> Option<&FunctionSig> {
        self.function_signatures.get(name)
    }

    /// Iterate all registered function signatures.
    ///
    /// Used by Phase 22's CTE schema extraction to propagate the caller's
    /// workspace-function map into the body context so nested
    /// `smelt.fn.*` calls inside CTE bodies (e.g.
    /// `SELECT * FROM smelt.fn.sessionize(...)`) can resolve their return
    /// schemas during wildcard expansion.
    pub fn function_signatures_iter(&self) -> impl Iterator<Item = (&str, &FunctionSig)> {
        self.function_signatures
            .iter()
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Seed the workspace `RecordRegistry` (Phase E1, Phase 3).
    ///
    /// Called by Phase 5 when wiring the Salsa-backed query. Phase 3 callers
    /// (pure tests) call this directly with a registry built by
    /// `build_record_registry`. Pre-Phase-5 callers that do not call this
    /// method see an empty registry (no `smelt.record` declarations in scope).
    ///
    /// Pure — no Salsa interaction.
    pub fn set_record_registry(&mut self, registry: Arc<RecordRegistry>) {
        self.record_registry = registry;
    }

    /// Look up a named record declaration from the workspace registry.
    ///
    /// Returns `None` when the name is not a declared `smelt.record` type.
    ///
    /// Pure — no Salsa interaction.
    pub fn lookup_record_decl(
        &self,
        name: &str,
    ) -> Option<&smelt_types::signatures::SmeltRecordDeclaration> {
        self.record_registry.lookup(name)
    }

    /// Seed a lambda parameter binding into the context (Phase B meta-language).
    ///
    /// Lambda parameters shadow `function_params` and all wider SQL scopes.
    /// Per `scoping.md` §"Resolution order": lambda parameter scope is the
    /// innermost — it resolves before enclosing function parameters.
    ///
    /// Callers clone the context before pushing lambda params and discard the
    /// clone after the lambda body walk to restore the outer scope.
    ///
    /// Pure — no Salsa interaction.
    pub fn add_lambda_param(&mut self, name: &str, col: TypedColumn) {
        self.lambda_params.insert(name.to_string(), col);
    }

    /// Seed a record-typed lambda binding so that qualified field accesses
    /// like `c.region` (where `c` is the lambda parameter and `region` is a
    /// field on its record type) resolve correctly inside emission bodies.
    ///
    /// For each `(field_name, field_smelt_type)` pair in the record's fields,
    /// this registers `"<name>.<field_name>"` as a model column so that
    /// `lookup_column(Some(name), field_name)` returns `Some(typed_col)`.
    ///
    /// Non-Record types are not seeded — multi-arg lambda support over
    /// non-record element types is not yet implemented. The method is a
    /// graceful no-op for those types; callers may still emit diagnostics
    /// through the normal `UndeclaredColumn` path.
    ///
    /// Pure — no Salsa interaction.
    pub fn register_lambda_binding(&mut self, name: &str, ty: &smelt_types::signatures::SmeltType) {
        use smelt_types::signatures::SmeltType;
        use smelt_types::{DataType, TypedColumn};

        if let SmeltType::Record { fields, .. } = ty {
            for (field_name, field_ty) in fields {
                let data_type = match field_ty {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(dt)) => {
                        dt.clone()
                    }
                    _ => DataType::Unknown,
                };
                self.add_model_column(
                    name,
                    field_name,
                    TypedColumn {
                        data_type,
                        nullable: true,
                    },
                );
            }
        }
        // Non-Record types: no-op. Multi-arg lambda support over non-record
        // element types is not yet implemented.
    }

    /// Unqualified-or-qualified identifier lookup that honours the
    /// function-parameter scope.
    ///
    /// Per §16 #1 of the smelt-functions research, function parameters
    /// resolve **before** any SQL scope. This matters in Step 1 only inside
    /// `smelt.define` bodies (no FROM scope is in play), but the
    /// mechanism is wired in now so Phase 6+ composition is trivial.
    ///
    /// Phase B adds lambda parameter scope (innermost): lambda params shadow
    /// function params which shadow SQL scopes.
    ///
    /// - Qualified lookups (`qualifier.is_some()`) bypass the function-param
    ///   map entirely — params are always bare names.
    /// - Unqualified lookups return a lambda param first, then a function param,
    ///   before falling through to [`TypeContext::lookup_column`].
    pub fn lookup_identifier(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        if qualifier.is_none() {
            // Lambda params are innermost scope — shadow function params.
            if let Some(col) = self.lambda_params.get(name) {
                return Some(col);
            }
            if let Some(col) = self.function_params.get(name) {
                return Some(col);
            }
            // Phase 37: a bare identifier that resolves as a table alias (e.g. `e`
            // in `smelt.fn.with_hour(e)`) is a valid reference even though it is
            // not itself a column.  Treat it as Unknown-typed to suppress false-
            // positive `UndeclaredColumn` diagnostics for table aliases used as
            // struct arguments.
            if self.aliases.contains_key(name) {
                return Some(Self::opaque_column());
            }
        }
        self.lookup_column(qualifier, name)
    }

    /// Return all `(column_name, typed_column)` pairs whose qualifier
    /// (the part before the `.`) matches `qualifier` in any of the
    /// source, model, or CTE column maps.
    ///
    /// Used by Phase 36 struct-parameter resolution to enumerate the
    /// concrete columns reachable through a table alias passed as a
    /// struct argument (e.g. `smelt.fn.event_hour(e)` where `e` is an
    /// alias for `smelt.sources.source.events AS e`).
    ///
    /// Alias resolution is applied first so `e` maps to `events` (or
    /// whatever the underlying table name is). Returns an empty `Vec`
    /// when no columns are found under `qualifier`.
    pub fn columns_for_qualifier(&self, qualifier: &str) -> Vec<(&str, &TypedColumn)> {
        // Resolve alias first.
        let resolved = self
            .aliases
            .get(qualifier)
            .map(|s| s.as_str())
            .unwrap_or(qualifier);
        let prefix = format!("{}.", resolved);

        let mut result: Vec<(&str, &TypedColumn)> = Vec::new();

        // CTE columns.
        for (key, tc) in &self.cte_columns {
            if let Some(col) = key.strip_prefix(&prefix) {
                result.push((col, tc));
            }
        }
        // Model columns.
        for (key, tc) in &self.model_columns {
            if let Some(col) = key.strip_prefix(&prefix) {
                result.push((col, tc));
            }
        }
        // Source columns (stored as `src.tbl.col` or `tbl.col`).
        // We match both the simple `<table>.col` form and the
        // fully-qualified `<source>.<table>.col` form.
        for (key, tc) in &self.source_columns {
            if let Some(col) = key.strip_prefix(&prefix) {
                // Avoid nested qualifiers: `col` must be a bare name.
                if !col.contains('.') {
                    result.push((col, tc));
                }
            }
        }

        result
    }

    /// Take and clear the list of column lookups that returned None.
    /// Used by property-based tests to discover missing columns.
    pub fn take_missed_lookups(&self) -> Vec<(Option<String>, String)> {
        match self.missed_lookups.lock() {
            Ok(mut lookups) => std::mem::take(&mut *lookups),
            Err(_) => Vec::new(),
        }
    }
}
