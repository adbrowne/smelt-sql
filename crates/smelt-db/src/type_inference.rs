/// Type inference for SQL expressions
///
/// This module provides type inference capabilities for SQL expressions,
/// including literals, column references, CAST expressions, and aggregates.
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltFnCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, SmeltType,
    TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::Mutex;

/// Context for type inference - provides source and upstream model schemas
#[derive(Debug, Default)]
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
    /// Column lookups that returned None (for property-based test column detection)
    missed_lookups: Mutex<Vec<(Option<String>, String)>>,
}

impl PartialEq for TypeContext {
    fn eq(&self, other: &Self) -> bool {
        self.source_columns == other.source_columns
            && self.model_columns == other.model_columns
            && self.cte_columns == other.cte_columns
            && self.cte_names == other.cte_names
            && self.aliases == other.aliases
            && self.function_params == other.function_params
            && self.function_signatures == other.function_signatures
            && self.tableexpr_param_schemas == other.tableexpr_param_schemas
            && self.row_var_env == other.row_var_env
            && self.opaque_ctes == other.opaque_ctes
            && self.fragment_param_kinds == other.fragment_param_kinds
            && self.expected_return == other.expected_return
        // missed_lookups is intentionally excluded — it's transient tracking state
    }
}

impl Eq for TypeContext {}

impl Clone for TypeContext {
    fn clone(&self) -> Self {
        Self {
            source_columns: self.source_columns.clone(),
            model_columns: self.model_columns.clone(),
            cte_columns: self.cte_columns.clone(),
            cte_names: self.cte_names.clone(),
            aliases: self.aliases.clone(),
            function_params: self.function_params.clone(),
            function_signatures: self.function_signatures.clone(),
            tableexpr_param_schemas: self.tableexpr_param_schemas.clone(),
            row_var_env: self.row_var_env.clone(),
            opaque_ctes: self.opaque_ctes.clone(),
            fragment_param_kinds: self.fragment_param_kinds.clone(),
            expected_return: self.expected_return.clone(),
            missed_lookups: Mutex::new(Vec::new()), // Don't clone tracking state
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

    /// Unqualified-or-qualified identifier lookup that honours the
    /// function-parameter scope.
    ///
    /// Per §16 #1 of the smelt-functions research, function parameters
    /// resolve **before** any SQL scope. This matters in Step 1 only inside
    /// `smelt.define` bodies (no FROM scope is in play), but the
    /// mechanism is wired in now so Phase 6+ composition is trivial.
    ///
    /// - Qualified lookups (`qualifier.is_some()`) bypass the function-param
    ///   map entirely — params are always bare names.
    /// - Unqualified lookups return a function param before falling through
    ///   to [`TypeContext::lookup_column`].
    pub fn lookup_identifier(&self, qualifier: Option<&str>, name: &str) -> Option<&TypedColumn> {
        if qualifier.is_none() {
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

/// Infer the type of an SQL expression
pub fn infer_expression_type(expr: &Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    let text = expr.text().trim().to_string();

    // Try CAST expression first
    if let Some(cast_expr) = expr.as_cast() {
        return infer_cast_type(&cast_expr, ctx);
    }

    // Try CASE expression
    if let Some(case_expr) = expr.as_case() {
        return infer_case_expr_type(&case_expr, ctx);
    }

    // Try subquery (scalar subquery)
    if let Some(subquery) = expr.as_subquery() {
        return infer_subquery_type(&subquery, ctx);
    }

    // Try EXTRACT expression
    if let Some(extract_expr) = expr.as_extract() {
        return infer_extract_type(&extract_expr);
    }

    // Try function call (aggregates, etc.)
    if let Some(func) = expr.as_function_call() {
        return infer_function_type(&func, ctx);
    }

    // Try smelt.fn.<name>(...) user-function call site. Returns the declared
    // return type of the resolved signature, or Unknown if the function
    // cannot be resolved / lacks a return annotation. Diagnostics for
    // unresolved functions / arg mismatches are emitted elsewhere
    // (`function_body_check::check_smelt_fn_call`), so this path is
    // type-only.
    if let Some(call) = expr.as_smelt_fn_call() {
        return infer_smelt_fn_call_type(&call, ctx);
    }

    // Try smelt.as_struct(alias [EXCEPT cols]) — Phase 38.
    // Resolves the alias's columns from the TypeContext, filters EXCEPT
    // columns, and returns DataType::Struct(remaining_fields).
    if let Some(call) = expr.as_smelt_as_struct_call() {
        return infer_as_struct_type(&call, ctx);
    }

    // Try binary expression
    if let Some(binary) = expr.as_binary() {
        return infer_binary_expr_type(&binary, ctx);
    }

    // Try BETWEEN expression - always returns Boolean
    if expr.as_between().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // Could be NULL if any operand is NULL
        });
    }

    // Try IN expression - always returns Boolean
    if expr.as_in().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // Could be NULL if expr or any value is NULL
        });
    }

    // Try EXISTS expression - always returns Boolean (never NULL)
    if expr.as_exists().is_some() {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false, // EXISTS always returns TRUE or FALSE, never NULL
        });
    }

    // Try array literal
    if let Some(array_lit) = expr.as_array_literal() {
        return infer_array_literal_type(&array_lit, ctx);
    }

    // Try array subscript
    if let Some(_subscript) = expr.as_array_subscript() {
        return infer_array_subscript_type(expr, ctx);
    }

    // Try array slice
    if let Some(_slice) = expr.as_array_slice() {
        return infer_array_slice_type(expr, ctx);
    }

    // Try ROW constructor
    if let Some(row) = expr.as_row_constructor() {
        return infer_row_constructor_type(&row, ctx);
    }

    // Try struct literal
    if let Some(struct_lit) = expr.as_struct_literal() {
        return infer_struct_literal_type(&struct_lit, ctx);
    }

    // Try column reference (includes struct field access for qualified refs like s.field_name)
    if let Some(col_ref) = expr.as_column_ref() {
        // Use `lookup_identifier` so that seeded function parameters (§16 #1)
        // resolve before any SQL FROM scope. For `TypeContext`s with no
        // function params seeded (the common case — all pre-Phase-5 call
        // sites), this is semantically identical to `lookup_column`.
        if let Some(typed_col) = ctx.lookup_identifier(col_ref.qualifier(), col_ref.name()) {
            return Some(typed_col.clone());
        }
        // If qualified ref didn't resolve as a column, try struct field access:
        // treat qualifier as a column name and name as a field name
        if let Some(qualifier) = col_ref.qualifier() {
            if let Some(struct_col) = ctx.lookup_column(None, qualifier) {
                if let DataType::Struct(fields) = &struct_col.data_type {
                    let field_lower = col_ref.name().to_lowercase();
                    for (name, dt) in fields {
                        if name.to_lowercase() == field_lower {
                            return Some(TypedColumn {
                                data_type: dt.clone(),
                                nullable: true, // Field access may be null
                            });
                        }
                    }
                }
            }
        }
        // Qualified column reference (e.g. "p.product_id") that couldn't be resolved —
        // return None rather than falling through to infer_literal_type which would
        // misinterpret the dot as a decimal point.
        // Unqualified refs (e.g. "INTERVAL") must still fall through so that
        // infer_literal_type can recognize typed literals like INTERVAL '1' DAY.
        if col_ref.qualifier().is_some() {
            return None;
        }
    }

    // Try literal inference (also handles typed literals like DATE '2025-01-15')
    infer_literal_type(&text)
}

/// Infer the [`ExprKind`] (Scalar / Agg / Window) of an expression
/// (Phase 14, §16 #24).
///
/// The kind is synthesised in the same pure pass as [`infer_expression_type`]
/// — column references and literals are `Scalar`; arithmetic / case / cast
/// take the ceiling of their sub-kinds; function calls consult the
/// [`BuiltinRegistry`]. A call site that carries an `OVER (…)` clause
/// produces [`ExprKind::Window`] regardless of the callee's seeded kind
/// (the canonical SQL dual-mode behaviour for aggregates).
///
/// Pure: no Salsa access, deterministic, side-effect free.
pub fn infer_expression_kind(expr: &Expr, ctx: &TypeContext) -> ExprKind {
    // Any expression with an attached OVER (...) clause is a window
    // expression. This dominates the callee's seeded kind — `SUM(x) OVER
    // (...)` is `Window`, not `Agg`.
    if expr.window_spec().is_some() {
        return ExprKind::Window;
    }

    // CAST(<inner> AS T) inherits the inner expression's kind.
    if let Some(cast_expr) = expr.as_cast() {
        return cast_expr
            .expression()
            .as_ref()
            .map(|inner| infer_expression_kind(inner, ctx))
            .unwrap_or(ExprKind::Scalar);
    }

    // CASE: ceiling over WHEN result branches and the optional ELSE.
    if let Some(case_expr) = expr.as_case() {
        let mut kinds: Vec<ExprKind> = Vec::new();
        for when in case_expr.when_clauses() {
            if let Some(result) = when.result() {
                kinds.push(infer_expression_kind(&result, ctx));
            }
            if let Some(cond) = when.condition() {
                kinds.push(infer_expression_kind(&cond, ctx));
            }
        }
        if let Some(else_expr) = case_expr.else_expr() {
            kinds.push(infer_expression_kind(&else_expr, ctx));
        }
        return kind_ceiling(&kinds);
    }

    // EXTRACT(field FROM expr) inherits the inner expression's kind.
    if let Some(extract_expr) = expr.as_extract() {
        return extract_expr
            .expression()
            .as_ref()
            .map(|inner| infer_expression_kind(inner, ctx))
            .unwrap_or(ExprKind::Scalar);
    }

    // Subquery: scalar — the subquery's inner kinds are checked against
    // its own splice points, not propagated outward. The subquery itself
    // is a Scalar value at the outer position.
    if expr.as_subquery().is_some() {
        return ExprKind::Scalar;
    }

    // SQL built-in / aggregate / window function call.
    if let Some(func) = expr.as_function_call() {
        return infer_function_call_kind(&func, ctx);
    }

    // smelt.fn.* user-defined call: scalar today (Phase 14 doesn't track
    // kind through user-defined fragments; that's a later phase). Until
    // then, treat as the most permissive kind so call sites in WHERE
    // don't false-positive.
    if expr.as_smelt_fn_call().is_some() {
        return ExprKind::Scalar;
    }

    // Binary expr: ceiling over LHS and RHS.
    if let Some(binary) = expr.as_binary() {
        let lhs = binary
            .left()
            .as_ref()
            .map(|e| infer_expression_kind(e, ctx))
            .unwrap_or(ExprKind::Scalar);
        let rhs = binary
            .right()
            .as_ref()
            .map(|e| infer_expression_kind(e, ctx))
            .unwrap_or(ExprKind::Scalar);
        return kind_ceiling(&[lhs, rhs]);
    }

    // BETWEEN / IN / EXISTS / array / row / struct: walk children and
    // take their ceiling. Most are scalar but if any sub-expr is Agg or
    // Window the wrapper inherits it.
    let mut kinds: Vec<ExprKind> = Vec::new();
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            kinds.push(infer_expression_kind(&child_expr, ctx));
        }
    }
    if !kinds.is_empty() {
        return kind_ceiling(&kinds);
    }

    // Column refs, literals, identifiers — Scalar.
    // Phase 44b exception: a bare unqualified identifier that matches a
    // registered fragment-typed parameter inherits that parameter's declared
    // kind. This lets `PASSING metrics AS (metrics)` forward a
    // `SelectItems<Agg>` parameter without producing a `FragmentKindMismatch`.
    if let Some(col_ref) = expr.as_column_ref() {
        if col_ref.qualifier().is_none() {
            if let Some(kind) = ctx.lookup_fragment_param_kind(col_ref.name()) {
                return kind;
            }
        }
    }
    ExprKind::Scalar
}

/// Compute the [`ExprKind`] of a SQL function-call site.
///
/// Looks the function up in the [`BuiltinRegistry`] for its seeded kind.
/// Unknown functions fall back to [`ExprKind::Scalar`]. (Aggregates with
/// an attached `OVER (…)` clause are handled by the caller — see
/// [`infer_expression_kind`]'s window check.)
fn infer_function_call_kind(func: &FunctionCall, _ctx: &TypeContext) -> ExprKind {
    let Some(name) = func.name() else {
        return ExprKind::Scalar;
    };
    let upper = name.to_uppercase();
    BuiltinRegistry::resolve(&upper)
        .map(|sig| sig.kind)
        .unwrap_or(ExprKind::Scalar)
}

/// Structured info about a window-in-scalar-context error (Phase 14).
///
/// Returned by [`check_window_in_scalar_contexts`] for each WHERE /
/// GROUP BY position whose expression resolves to [`ExprKind::Window`].
/// The caller (`check_file_diagnostics`) maps these into
/// [`crate::DiagnosticCode::WindowInScalarContext`] entries.
#[derive(Debug, Clone)]
pub struct WindowInScalarContextInfo {
    /// Free-form clause name (`"WHERE"`, `"GROUP BY"`) for the message.
    pub clause: &'static str,
    /// Source span of the offending expression.
    pub range: TextRange,
    /// Trimmed text of the offending expression — quoted in the message.
    pub expression_text: String,
}

/// Pure check: collect every expression in WHERE / GROUP BY / HAVING whose
/// synthesised kind is [`ExprKind::Window`] (Phase 14, §16 #24).
///
/// Also recurses into scalar subqueries nested inside those clauses so that
/// `WHERE col > (SELECT ROW_NUMBER() OVER (...) FROM t)` is flagged as a
/// `"WHERE"` violation (Phase 49).
///
/// FROM-clause subqueries are intentionally excluded: they are not scalar
/// contexts, and window functions are valid inside derived-table SELECT lists.
pub fn check_window_in_scalar_contexts(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<WindowInScalarContextInfo> {
    let mut out = Vec::new();

    if let Some(where_clause) = select_stmt.where_clause() {
        if let Some(expr) = where_clause.expression() {
            check_expr_and_scalar_subqueries(&expr, "WHERE", ctx, &mut out);
        }
    }

    if let Some(group_by) = select_stmt.group_by_clause() {
        for expr in group_by.expressions() {
            check_expr_and_scalar_subqueries(&expr, "GROUP BY", ctx, &mut out);
        }
    }

    if let Some(having_clause) = select_stmt.having_clause() {
        if let Some(expr) = having_clause.expression() {
            check_expr_and_scalar_subqueries(&expr, "HAVING", ctx, &mut out);
        }
    }

    out
}

/// Check a single expression in a scalar clause (WHERE / GROUP BY / HAVING):
///
/// 1. If the expression itself is `Window`-kinded, emit an error.
/// 2. Recursively descend into any scalar subqueries found within the
///    expression tree. For each scalar subquery found:
///    - Check its SELECT list for Window-kinded expressions (a window function
///      inside a scalar-subquery SELECT list is invalid because the subquery
///      must return a scalar value).
///    - Call [`check_window_in_scalar_contexts`] on it to catch violations in
///      its nested WHERE / GROUP BY / HAVING clauses.
///
/// FROM-clause subqueries are **not** visited here — they live under
/// `TABLE_REF` nodes, which are children of `FROM_CLAUSE`, which is a child
/// of `SELECT_STMT`, not of an expression node.  Since we only enter this
/// function from expression positions (WHERE / GROUP BY / HAVING), every
/// `SUBQUERY` descendant of `expr` is guaranteed to be a scalar subquery.
fn check_expr_and_scalar_subqueries(
    expr: &Expr,
    clause: &'static str,
    ctx: &TypeContext,
    out: &mut Vec<WindowInScalarContextInfo>,
) {
    use smelt_parser::SyntaxKind;

    // Top-level: if this expression is Window-kinded, report it directly.
    if infer_expression_kind(expr, ctx) == ExprKind::Window {
        out.push(WindowInScalarContextInfo {
            clause,
            range: expr.text_range(),
            expression_text: expr.text().trim().to_string(),
        });
    }

    // Recurse into scalar subqueries nested inside this expression.
    // All SUBQUERY nodes in an expression tree are scalar contexts (they are
    // not FROM-clause derived tables), so we check their inner SELECT
    // statements with the same outer clause name.
    for node in expr.syntax().descendants() {
        if node.kind() == SyntaxKind::SUBQUERY {
            if let Some(subquery) = Subquery::cast(node) {
                if let Some(inner_select) = subquery.select_stmt() {
                    // (a) Check the inner SELECT's own SELECT list: a window
                    // function in the select list of a scalar subquery is
                    // invalid because the subquery must produce a scalar value.
                    check_scalar_subquery_select_list(&inner_select, clause, out);

                    // (b) Recurse into the inner SELECT's WHERE/GROUP BY/HAVING
                    // clauses (and any further nested scalar subqueries there).
                    out.extend(check_window_in_scalar_contexts(&inner_select, ctx));
                }
            }
        }
    }
}

/// For a [`SelectStmt`] that appears as a scalar subquery, check each item in
/// its SELECT list. If any item contains a Window-kinded expression (directly
/// or buried inside an aggregate wrapping a window call), emit an entry with
/// `clause` preserved from the outer scalar context.
///
/// This is needed because `infer_expression_kind` treats outer function calls
/// by their registry kind (e.g. `MAX(ROW_NUMBER() OVER (...))` → `Agg`), so a
/// raw top-level kind check would miss the nested window expression. Here we
/// walk the expression's descendants looking for any node with an OVER clause.
fn check_scalar_subquery_select_list(
    inner_select: &SelectStmt,
    clause: &'static str,
    out: &mut Vec<WindowInScalarContextInfo>,
) {
    use smelt_parser::SyntaxKind;

    let Some(select_list) = inner_select.select_list() else {
        return;
    };
    for item in select_list.items() {
        let Some(item_expr) = item.expression() else {
            continue;
        };
        // Walk every descendant of this select-item expression looking for
        // any EXPRESSION node that carries an OVER clause (i.e. a window
        // function call).
        //
        // We look for EXPRESSION nodes (not FUNCTION_CALL) because the parser
        // puts the WINDOW_SPEC as a sibling of the FUNCTION_CALL inside a
        // parent EXPRESSION: `EXPRESSION { FUNCTION_CALL { ARG_LIST } WINDOW_SPEC }`.
        // An `Expr` wrapping that EXPRESSION node will find the WINDOW_SPEC via
        // `window_spec()`, while an `Expr` wrapping the inner FUNCTION_CALL won't.
        //
        // We do NOT use `infer_expression_kind` on the top-level item because an
        // aggregate wrapping a window call (e.g. `MAX(ROW_NUMBER() OVER (...))`)
        // would be classified as `Agg` by the registry lookup, hiding the inner
        // window function.
        for desc_node in item_expr.syntax().descendants() {
            if desc_node.kind() == SyntaxKind::EXPRESSION {
                if let Some(desc_expr) = Expr::cast(desc_node) {
                    if desc_expr.window_spec().is_some() {
                        out.push(WindowInScalarContextInfo {
                            clause,
                            range: desc_expr.text_range(),
                            expression_text: desc_expr.text().trim().to_string(),
                        });
                        // One hit per select item is sufficient.
                        break;
                    }
                }
            }
        }
    }
}

/// Infer the type of a CAST expression.
/// Preserves nullability from the input expression: if the input is non-nullable,
/// the CAST result is also non-nullable.
fn infer_cast_type(cast_expr: &CastExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let type_spec = cast_expr.type_spec()?;
    let type_text = type_spec.full_text();

    // Parse the type specification
    let data_type = parse_type(&type_text).ok()?;

    // Normalize FLOAT to DOUBLE: DuckDB treats FLOAT as a 4-byte float but
    // smelt normalizes to DOUBLE to avoid spurious type mismatches downstream.
    let data_type = match data_type {
        DataType::Float => DataType::Double,
        other => other,
    };

    // Check if the input expression is nullable
    let nullable = cast_expr
        .expression()
        .and_then(|e| infer_expression_type(&e, ctx))
        .is_none_or(|t| t.nullable);

    Some(TypedColumn {
        data_type,
        nullable,
    })
}

/// Infer the type of an EXTRACT(field FROM expr) expression.
fn infer_extract_type(extract_expr: &ExtractExpr) -> Option<TypedColumn> {
    let field = extract_expr.field_name().unwrap_or_default();
    let data_type = match field.as_str() {
        "EPOCH" => DataType::Double,
        "YEAR" | "MONTH" | "DAY" | "HOUR" | "MINUTE" | "SECOND" | "DOW" | "DOY" | "QUARTER"
        | "WEEK" | "DAYOFWEEK" | "DAYOFYEAR" | "ISODOW" | "ISOYEAR" | "MICROSECOND"
        | "MICROSECONDS" | "MILLISECOND" | "MILLISECONDS" | "TIMEZONE" | "TIMEZONE_HOUR"
        | "TIMEZONE_MINUTE" => DataType::BigInt,
        _ => DataType::BigInt, // default for unknown fields
    };
    Some(TypedColumn {
        data_type,
        nullable: true,
    })
}

/// Infer the type of a CASE expression.
/// The result type is the type of the first THEN expression (or ELSE if no WHEN clauses).
/// Non-nullable only when an ELSE clause is present AND all branches (THEN + ELSE) are non-nullable.
/// Without ELSE, the implicit default is NULL, making the result always nullable.
fn infer_case_expr_type(case_expr: &CaseExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let has_else = case_expr.else_expr().is_some();

    // Collect types from all WHEN/THEN branches, promoting across all of them
    let mut accumulated: Option<TypedColumn> = None;
    let mut all_branches_non_nullable = true;

    let merge = |acc: Option<TypedColumn>, branch: TypedColumn| -> Option<TypedColumn> {
        match acc {
            None => Some(branch),
            Some(existing) => Some(promote_types(&existing, &branch)),
        }
    };

    for when_clause in case_expr.when_clauses() {
        if let Some(result_expr) = when_clause.result() {
            if let Some(result_type) = infer_expression_type(&result_expr, ctx) {
                if result_type.nullable {
                    all_branches_non_nullable = false;
                }
                if !matches!(result_type.data_type, DataType::Unknown | DataType::Null) {
                    accumulated = merge(accumulated, result_type);
                }
            } else {
                all_branches_non_nullable = false;
            }
        } else {
            all_branches_non_nullable = false;
        }
    }

    // Check ELSE branch
    if let Some(else_expr) = case_expr.else_expr() {
        if let Some(else_type) = infer_expression_type(&else_expr, ctx) {
            if else_type.nullable {
                all_branches_non_nullable = false;
            }
            if !matches!(else_type.data_type, DataType::Unknown | DataType::Null) {
                accumulated = merge(accumulated, else_type);
            }
        } else {
            all_branches_non_nullable = false;
        }
    }

    let accumulated = accumulated?;
    let data_type = accumulated.data_type;

    // Non-nullable only when ELSE is present and all branches are non-nullable.
    // Without ELSE, the implicit default is NULL.
    let nullable = !(has_else && all_branches_non_nullable);

    Some(TypedColumn {
        data_type,
        nullable,
    })
}

/// Infer the type of a scalar subquery
/// The result type is the type of the first column in the SELECT list
fn infer_subquery_type(subquery: &Subquery, ctx: &TypeContext) -> Option<TypedColumn> {
    let select_stmt = subquery.select_stmt()?;

    // Build a new context that includes any CTEs defined in this subquery
    let subquery_ctx = build_subquery_context(&select_stmt, ctx);

    let select_list = select_stmt.select_list()?;

    // Get the first select item and infer its type
    if let Some(first_item) = select_list.items().next() {
        if let Some(expr) = first_item.expression() {
            if let Some(expr_type) = infer_expression_type(&expr, &subquery_ctx) {
                return Some(TypedColumn {
                    data_type: expr_type.data_type,
                    // Scalar subqueries are always nullable (could return no rows)
                    nullable: true,
                });
            }
        }
    }

    None
}

/// Build a TypeContext for a subquery that includes any nested CTEs
///
/// This creates a new context that inherits from the parent context
/// and adds any CTEs defined in the subquery's WITH clause.
pub fn build_subquery_context(select_stmt: &SelectStmt, parent_ctx: &TypeContext) -> TypeContext {
    let mut ctx = parent_ctx.clone();

    // Phase 27: Do not propagate `expected_return` into subquery contexts.
    // The outer function's bidirectional hint applies to the top-level body
    // expression only. Propagating it into subqueries would incorrectly widen
    // registry-migrated generics inside nested SELECT statements, producing
    // wrong inferred types for sub-expressions that have no declared return.
    ctx.expected_return = None;

    // Process any WITH clause in this subquery
    if let Some(with_clause) = select_stmt.with_clause() {
        for cte in with_clause.ctes() {
            if let Some(cte_name) = cte.name() {
                // For recursive CTEs with explicit column list, bootstrap with Unknown types
                if with_clause.is_recursive() {
                    for col_name in cte.column_names() {
                        ctx.add_cte_column(
                            &cte_name,
                            &col_name,
                            TypedColumn {
                                data_type: DataType::Unknown,
                                nullable: true,
                            },
                        );
                    }
                }

                // Infer columns from CTE query
                let columns = infer_cte_columns(&cte, &ctx);
                for (col_name, typed_col) in columns {
                    ctx.add_cte_column(&cte_name, &col_name, typed_col);
                }

                // Register CTE name as alias
                ctx.add_alias(&cte_name, &cte_name);
            }
        }
    }

    ctx
}

/// Infer the return type of a `smelt.fn.<name>(...)` call site.
///
/// Uses the workspace function-signature index seeded on [`TypeContext`]
/// (via [`TypeContext::add_function_signature`]) — no Salsa access. Returns:
///   - `Some(TypedColumn)` with the declared return type when the signature
///     resolves and carries a `-> Expr<Concrete(T)>` annotation.
///   - `Some(TypedColumn { data_type: Double, .. })` when the return is
///     `Expr<Numeric>` — matches `param_binding_type`'s widening rule in
///     `function_body_check.rs` so callers doing `CAST(... AS DOUBLE) /
///     safe_divide(...)` stay well-typed.
///   - `Some(TypedColumn { data_type: Unknown, .. })` for `Expr<Any>`,
///     malformed annotations, or missing annotations — diagnostic emission
///     is the call-site checker's job, not inference's.
///   - `None` only when the function cannot be resolved in this context.
fn infer_smelt_fn_call_type(call: &SmeltFnCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let segments = call.call_path().map(|p| p.segments()).unwrap_or_default();
    let name = segments.last()?;
    let sig = ctx.lookup_function_signature(name)?;

    let dt = match &sig.return_type {
        Some(Ok(SmeltType::Expr(TypeConstraint::Concrete(dt)))) => dt.clone(),
        Some(Ok(SmeltType::Expr(TypeConstraint::Numeric))) => DataType::Double,
        // `Ordered` (Phase 7) is only reachable via generics in v1 signatures
        // (§16 #14) — Phase 8 adds the inference machinery. In the monomorphic
        // `smelt.define` path we stay conservative: no precise return type
        // known yet, surface `Unknown` like `Any`.
        Some(Ok(SmeltType::Expr(TypeConstraint::Ordered))) => DataType::Unknown,
        Some(Ok(SmeltType::Expr(TypeConstraint::Any))) => DataType::Unknown,
        // `TableExpr` return (Phase 15) — scalar inference has no
        // DataType for a whole row set. Downstream Phase 17 plumbs the
        // inferred output schema; for now the call-site sees an opaque
        // Unknown.
        Some(Ok(SmeltType::TableExpr(_))) => DataType::Unknown,
        // `SelectItems<Kind>` (Phase 21) is not a scalar type.
        Some(Ok(SmeltType::SelectItems { .. })) => DataType::Unknown,
        // Phase 37: `Struct<{declared_fields, ..r}>` return type — resolve
        // the row variable `r` by examining the call-site argument that
        // corresponds to the first struct parameter.  When the extras can
        // be determined we build a concrete `DataType::Struct` from the
        // declared fields plus the extras; otherwise fall back to Unknown.
        Some(Ok(SmeltType::Struct {
            fields: ret_fields,
            tail,
        })) => resolve_struct_return_type(call, ctx, sig, ret_fields, tail),
        Some(Err(_)) => DataType::Unknown,
        None => DataType::Unknown,
    };
    Some(TypedColumn::nullable(dt))
}

/// Resolve a `Struct<{declared_fields, ..r}>` return type to a concrete
/// `DataType::Struct` by consulting the call-site argument schema (Phase 37).
///
/// Algorithm:
/// 1. Find the first struct parameter (one whose type is `SmeltType::Struct`).
/// 2. Get the corresponding call-site argument expression.
/// 3. Resolve the argument to a column set via `ctx.columns_for_qualifier`.
/// 4. Run `check_struct_row_var_binding` to compute the extras for the row var.
/// 5. Return `DataType::Struct(ret_fields + extras)`.
///
/// Falls back to `DataType::Unknown` whenever any step cannot be completed.
fn resolve_struct_return_type(
    call: &SmeltFnCall,
    ctx: &TypeContext,
    sig: &smelt_types::signatures::FunctionSig,
    ret_fields: &[(String, DataType)],
    tail: &smelt_types::signatures::StructRowTail,
) -> DataType {
    use crate::function_body_check::{check_struct_row_var_binding, struct_param_fields};
    use smelt_types::signatures::StructRowTail;

    // If no named row var, just return the declared fields as a concrete struct.
    let var_name = match tail {
        StructRowTail::Named(n) => n.as_str(),
        StructRowTail::Anon | StructRowTail::None => {
            // No row variable — build concrete struct from declared fields only.
            let concrete: Vec<(String, DataType)> = ret_fields.to_vec();
            return DataType::Struct(concrete);
        }
    };

    // Find the struct parameter index.
    let struct_param_idx = sig
        .params
        .iter()
        .position(|p| matches!(&p.type_ref, Some(Ok(SmeltType::Struct { .. }))));
    let Some(idx) = struct_param_idx else {
        return DataType::Unknown;
    };

    // Get the corresponding argument expression.
    let arg_list = call.arg_list();
    let positional: Vec<_> = arg_list
        .as_ref()
        .map(|al| al.positional_args())
        .unwrap_or_default();
    let arg_expr = positional.get(idx).cloned().or_else(|| {
        // Named argument lookup.
        let param_name = &sig.params[idx].name;
        let named: Vec<_> = arg_list
            .as_ref()
            .map(|al| al.named_params().collect())
            .unwrap_or_default();
        named.into_iter().find_map(|np| {
            if np.name().as_deref() == Some(param_name.as_str()) {
                np.value_expr()
            } else {
                None
            }
        })
    });
    let Some(arg) = arg_expr else {
        return DataType::Unknown;
    };

    // Resolve the argument to a column set.
    let qualifier = arg.text().trim().to_string();
    if qualifier.is_empty() {
        return DataType::Unknown;
    }
    let cols: Vec<(String, DataType)> = ctx
        .columns_for_qualifier(&qualifier)
        .into_iter()
        .map(|(col_name, tc)| (col_name.to_string(), tc.data_type.clone()))
        .collect();
    if cols.is_empty() {
        return DataType::Unknown;
    }

    // Extract declared fields from the struct parameter to compute extras.
    let param = &sig.params[idx];
    let Some((declared_fields, param_tail)) = struct_param_fields(param) else {
        return DataType::Unknown;
    };

    // Run struct row-var unification to get extras.
    let extras = match check_struct_row_var_binding(declared_fields, &cols, param_tail) {
        Ok(Some(extras)) => extras,
        Ok(None) => vec![],
        Err(_) => return DataType::Unknown,
    };

    // Check that the row var name matches between param and return type.
    let param_var_matches = match param_tail {
        StructRowTail::Named(param_var) => param_var.as_str() == var_name,
        _ => false,
    };
    if !param_var_matches {
        return DataType::Unknown;
    }

    // Build the concrete return type: declared return fields + extras.
    let mut concrete: Vec<(String, DataType)> = ret_fields.to_vec();
    concrete.extend(extras);
    DataType::Struct(concrete)
}

/// Infer the type of a `smelt.as_struct(alias [EXCEPT col1, col2])` call
/// (Phase 38).
///
/// Algorithm:
/// 1. Read the alias name from the `SmeltAsStructCall`.
/// 2. Collect columns for that qualifier via `ctx.columns_for_qualifier`.
/// 3. Remove columns named in the EXCEPT list.
/// 4. Return `DataType::Struct(remaining_fields)`.
///
/// Returns `None` when the alias cannot be resolved in the context.
fn infer_as_struct_type(call: &SmeltAsStructCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let alias = call.alias()?;
    let except = call.except_columns();
    let cols = ctx.columns_for_qualifier(&alias);
    if cols.is_empty() {
        return None;
    }
    let fields: Vec<(String, DataType)> = cols
        .into_iter()
        .filter(|(name, _)| !except.contains(&name.to_string()))
        .map(|(name, tc)| (name.to_string(), tc.data_type.clone()))
        .collect();
    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false,
    })
}

/// LUB adapter: the canonical numeric-promotion routine lives in
/// [`promote_types`] (this module) but signatures-side [`unify_call`] needs a
/// plain `Fn(&DataType, &DataType) -> DataType`. This wrapper keeps
/// `smelt-types` dependency-free per the plan's cross-phase design choice.
fn registry_lub(a: &DataType, b: &DataType) -> DataType {
    let lhs = TypedColumn {
        data_type: a.clone(),
        nullable: true,
    };
    let rhs = TypedColumn {
        data_type: b.clone(),
        nullable: true,
    };
    promote_types(&lhs, &rhs).data_type
}

/// Names we've migrated from the hand-written match to registry-driven
/// inference. Phase 9 deliberately keeps this allowlist small — each entry is
/// one that the registry's `unify_call` shape reproduces the legacy
/// `infer_function_type` behaviour for. Entries NOT on this list continue
/// through the legacy match unchanged (see coverage-spike findings in the
/// Phase 9 implementer report).
///
/// The list is ordered by family for easy review.
const REGISTRY_MIGRATED: &[&str] = &[
    // Aggregates — simple identity-return or fixed-return.
    "AVG",   // registry: <T: Numeric>(T) → Double — matches legacy (Double, nullable=true)
    "MIN",   // registry: <T: Ordered>(T) → T — matches legacy first-arg + nullable=true
    "MAX",   // same as MIN
    "COUNT", // registry: (Any) → BigInt — matches legacy (BigInt, nullable=false)
    // Arithmetic scalars.
    "ABS", // registry: <T: Numeric>(T) → T — matches legacy (preserves arg type + nullable)
    // Text scalars (registry demands Text arg; legacy is permissive. On a
    // constraint violation we fall back to legacy).
    "LOWER",
    "UPPER",
    "TRIM",
    "CONCAT",
    // Date/time basics (fixed returns).
    "DATE",
    "NOW",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "DATE_TRUNC",
];

/// Policy for deriving [`TypedColumn::nullable`] on a registry-resolved call.
///
/// The registry itself doesn't track nullability (§16 defers it — see "Out of
/// scope" in the plan), so Phase 9 mirrors the legacy per-function rule via a
/// tiny lookup table. Migrating a new entry to the registry means adding a row
/// here.
fn registry_result_nullable(name: &str, arg_nullable: &[bool]) -> bool {
    match name {
        // Non-nullable aggregates / niladic clocks.
        "COUNT" | "NOW" | "CURRENT_DATE" | "CURRENT_TIMESTAMP" => false,
        // ABS preserves its arg's nullability — legacy returns the arg
        // TypedColumn verbatim when a single-arg inference succeeds.
        "ABS" => arg_nullable.first().copied().unwrap_or(true),
        // Everything else is nullable per legacy.
        _ => true,
    }
}

/// Registry-first inference for the allowlisted subset of built-ins.
///
/// Returns:
/// * `Some(Some(tc))` when the registry resolved the call cleanly — the caller
///   uses this directly and skips the legacy match.
/// * `Some(None)` when the function is known to the registry but arg types
///   couldn't be inferred up-front — the caller should fall through to the
///   legacy match which handles Unknown args more gracefully.
/// * `None` when the function isn't on [`REGISTRY_MIGRATED`] or the registry
///   doesn't know about it — caller uses the legacy match.
fn try_registry_inference(
    upper_name: &str,
    func: &FunctionCall,
    ctx: &TypeContext,
) -> Option<Option<TypedColumn>> {
    if !REGISTRY_MIGRATED.contains(&upper_name) {
        return None;
    }
    let sig = BuiltinRegistry::resolve(upper_name)?;

    // Collect arg DataTypes + nullability. If any arg fails to infer, defer
    // to the legacy match — it has per-function fallback behaviour for
    // Unknown args that the registry doesn't model.
    let args = func.arguments();
    let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len());
    let mut arg_nullable: Vec<bool> = Vec::with_capacity(args.len());
    for arg in &args {
        match infer_expression_type(arg, ctx) {
            Some(tc) => {
                arg_types.push(tc.data_type);
                arg_nullable.push(tc.nullable);
            }
            None => {
                // Missing arg inference — fall back to legacy, which has
                // function-specific Unknown handling (e.g. `first_arg_type_or`
                // supplies a sensible default for MIN/MAX).
                return Some(None);
            }
        }
    }

    match unify_call_with_expected(sig, &arg_types, ctx.expected_return.as_ref(), &registry_lub) {
        Ok(result) => {
            let nullable = registry_result_nullable(upper_name, &arg_nullable);
            Some(Some(TypedColumn {
                data_type: result.return_type,
                nullable,
            }))
        }
        // Unification failed — fall back to the legacy match so permissive
        // behaviour (e.g. LOWER on Integer, MIN on Unknown) is preserved.
        Err(_) => Some(None),
    }
}

/// Infer the type of a function call (aggregates, etc.)
fn infer_function_type(func: &FunctionCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let name = func.name()?.to_uppercase();
    let sql_func = SqlFunction::from_name(&name)?;

    // Phase 9: registry-first lookup for the allowlisted subset. When the
    // registry returns a concrete result we use it; on miss/fall-through we
    // continue into the legacy match below.
    if let Some(Some(tc)) = try_registry_inference(&name, func, ctx) {
        return Some(tc);
    }

    /// Helper: return the type of the first argument, or `fallback` if inference fails.
    /// For COALESCE and similar, this intentionally only checks the first arg —
    /// using a later arg's type would risk being incorrect if earlier args are Unknown.
    fn first_arg_type_or(
        func: &FunctionCall,
        ctx: &TypeContext,
        fallback: DataType,
        nullable: bool,
    ) -> Option<TypedColumn> {
        if let Some(arg) = func.arguments().first() {
            if let Some(arg_type) = infer_expression_type(arg, ctx) {
                return Some(TypedColumn {
                    data_type: arg_type.data_type,
                    nullable,
                });
            }
        }
        Some(TypedColumn {
            data_type: fallback,
            nullable,
        })
    }

    match sql_func {
        SqlFunction::Count => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::Sum => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    // DuckDB SUM widening rules:
                    //   SUM(SMALLINT|INTEGER|BIGINT)   -> BIGINT (HUGEINT in DuckDB,
                    //                                     but smelt models that as BIGINT)
                    //   SUM(DOUBLE|FLOAT)              -> DOUBLE
                    //   SUM(DECIMAL(p, s))             -> DECIMAL(38, s)
                    //
                    // The Decimal precision widen-to-38 is critical: real
                    // pipelines accumulate ~1e6 rows of DECIMAL(10,2) values
                    // which overflow precision 10 quickly. Keeping the input
                    // precision silently corrupts results.
                    let result_type = match &arg_type.data_type {
                        DataType::SmallInt | DataType::Integer => DataType::BigInt,
                        DataType::BigInt => DataType::BigInt,
                        DataType::Float | DataType::Double => DataType::Double,
                        DataType::Decimal { scale, .. } => DataType::Decimal {
                            precision: 38,
                            scale: *scale,
                        },
                        // Unknown / mixed: defer to BIGINT (the historical
                        // fallback) — but the caller is expected to give us
                        // a populated TypeContext so this path is rare.
                        _ => DataType::BigInt,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::Avg => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Min | SqlFunction::Max => {
            first_arg_type_or(func, ctx, DataType::Unknown, true)
        }

        SqlFunction::Coalesce => {
            // Try all arguments, return first concrete (non-Unknown, non-Null) type.
            // COALESCE is non-nullable when at least one argument is non-nullable
            // or is a non-null literal, because the result will always have a value.
            let mut result_type = None;
            let mut has_non_nullable_arg = false;
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !arg_type.nullable {
                        has_non_nullable_arg = true;
                    }
                    if result_type.is_none()
                        && !matches!(arg_type.data_type, DataType::Unknown | DataType::Null)
                    {
                        result_type = Some(arg_type.data_type.clone());
                    }
                }
            }
            let data_type = result_type.unwrap_or(DataType::Unknown);
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable_arg,
            })
        }

        SqlFunction::Nullif => first_arg_type_or(func, ctx, DataType::Unknown, true),

        SqlFunction::Ifnull => {
            // IFNULL(a, b) is equivalent to COALESCE(a, b).
            // Non-nullable when either argument is non-nullable.
            let args = func.arguments();
            let first_type = args.first().and_then(|a| infer_expression_type(a, ctx));
            let second_type = args.get(1).and_then(|a| infer_expression_type(a, ctx));
            let data_type = first_type
                .as_ref()
                .filter(|t| !matches!(t.data_type, DataType::Unknown | DataType::Null))
                .or(second_type.as_ref())
                .map(|t| t.data_type.clone())
                .unwrap_or(DataType::Unknown);
            let has_non_nullable = first_type.as_ref().is_some_and(|t| !t.nullable)
                || second_type.as_ref().is_some_and(|t| !t.nullable);
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable,
            })
        }

        SqlFunction::RowNumber
        | SqlFunction::Rank
        | SqlFunction::DenseRank
        | SqlFunction::Ntile => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::CumeDist | SqlFunction::PercentRank => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Lag
        | SqlFunction::Lead
        | SqlFunction::FirstValue
        | SqlFunction::LastValue
        | SqlFunction::NthValue => first_arg_type_or(func, ctx, DataType::Unknown, true),

        SqlFunction::Now | SqlFunction::CurrentTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: false,
        }),

        SqlFunction::CurrentDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        }),

        SqlFunction::Date => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::DateTrunc => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

        SqlFunction::Concat
        | SqlFunction::Upper
        | SqlFunction::Lower
        | SqlFunction::Trim
        | SqlFunction::Ltrim
        | SqlFunction::Rtrim
        | SqlFunction::Substring
        | SqlFunction::Substr => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Length | SqlFunction::CharLength | SqlFunction::CharacterLength => {
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::ToChar => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BoolAnd | SqlFunction::BoolOr | SqlFunction::Every => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        SqlFunction::Abs => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Sign => Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: true,
        }),

        SqlFunction::Round | SqlFunction::Trunc | SqlFunction::Truncate => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Ceil | SqlFunction::Ceiling | SqlFunction::Floor => {
            // DuckDB: CEIL/FLOOR(DECIMAL(p,s)) → Decimal(p,0), all others → Double
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    let result_type = match &arg_type.data_type {
                        DataType::Decimal { precision, .. } => DataType::Decimal {
                            precision: *precision,
                            scale: 0,
                        },
                        _ => DataType::Double,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: arg_type.nullable,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Power
        | SqlFunction::Pow
        | SqlFunction::Sqrt
        | SqlFunction::Exp
        | SqlFunction::Ln
        | SqlFunction::Log
        | SqlFunction::Log10
        | SqlFunction::Log2 => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Mod => first_arg_type_or(func, ctx, DataType::Integer, true),

        SqlFunction::Sin
        | SqlFunction::Cos
        | SqlFunction::Tan
        | SqlFunction::Asin
        | SqlFunction::Acos
        | SqlFunction::Atan
        | SqlFunction::Atan2
        | SqlFunction::Sinh
        | SqlFunction::Cosh
        | SqlFunction::Tanh => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Pi | SqlFunction::Random => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Extract
        | SqlFunction::DatePart
        | SqlFunction::Year
        | SqlFunction::Month
        | SqlFunction::Day
        | SqlFunction::DayOfWeek
        | SqlFunction::Quarter => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::MakeDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::MakeTime => Some(TypedColumn {
            data_type: DataType::Time,
            nullable: true,
        }),

        SqlFunction::MakeTimestamp | SqlFunction::MakeTimestamptz => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

        SqlFunction::Age => Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: true,
        }),

        SqlFunction::Replace
        | SqlFunction::Translate
        | SqlFunction::Reverse
        | SqlFunction::Repeat
        | SqlFunction::Lpad
        | SqlFunction::Rpad
        | SqlFunction::Initcap
        | SqlFunction::QuoteIdent
        | SqlFunction::QuoteLiteral => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Left | SqlFunction::Right => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Position | SqlFunction::Strpos => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::SplitPart => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Greatest | SqlFunction::Least => {
            // Try all arguments, return first concrete type
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !matches!(arg_type.data_type, DataType::Unknown | DataType::Null) {
                        return Some(TypedColumn {
                            data_type: arg_type.data_type,
                            nullable: true,
                        });
                    }
                }
            }
            first_arg_type_or(func, ctx, DataType::Unknown, true)
        }

        SqlFunction::ArrayAgg => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: DataType::Array(Box::new(arg_type.data_type)),
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Array(Box::new(DataType::Unknown)),
                nullable: true,
            })
        }

        SqlFunction::StringAgg | SqlFunction::Listagg => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonObject
        | SqlFunction::JsonArray
        | SqlFunction::ToJson
        | SqlFunction::JsonExtract
        | SqlFunction::JsonExtractText => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonArrayLength => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::JsonObjectKeys => Some(TypedColumn {
            data_type: DataType::Array(Box::new(DataType::Text)),
            nullable: true,
        }),

        SqlFunction::JsonContains => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        }),

        // Aggregate functions from optimizer that don't have specialized type inference yet
        SqlFunction::Stddev
        | SqlFunction::Variance
        | SqlFunction::StddevPop
        | SqlFunction::StddevSamp
        | SqlFunction::VarPop
        | SqlFunction::VarSamp
        | SqlFunction::Corr
        | SqlFunction::CovarPop
        | SqlFunction::CovarSamp
        | SqlFunction::RegrSlope => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Median => first_arg_type_or(func, ctx, DataType::Double, true),

        SqlFunction::Mode => first_arg_type_or(func, ctx, DataType::Unknown, true),

        SqlFunction::PercentileCont | SqlFunction::PercentileDisc => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::ApproxCountDistinct => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::AnyValue | SqlFunction::First | SqlFunction::Last => {
            first_arg_type_or(func, ctx, DataType::Unknown, true)
        }

        SqlFunction::GroupConcat => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BitAnd | SqlFunction::BitOr | SqlFunction::BitXor => {
            first_arg_type_or(func, ctx, DataType::BigInt, true)
        }
    }
}

/// Infer the type of an array literal (ARRAY[1, 2, 3]).
/// All elements must have the same type; mixed-type arrays return None (error).
/// Empty arrays return Array(Unknown).
fn infer_array_literal_type(
    array_lit: &smelt_parser::ArrayLiteral,
    ctx: &TypeContext,
) -> Option<TypedColumn> {
    let elements = array_lit.elements();

    if elements.is_empty() {
        return Some(TypedColumn {
            data_type: DataType::Array(Box::new(DataType::Unknown)),
            nullable: false,
        });
    }

    // Infer element types
    let mut element_typed: Option<TypedColumn> = None;

    for elem in &elements {
        if let Some(typed) = infer_expression_type(elem, ctx) {
            match &element_typed {
                None => {
                    // First element sets the type (skip Null — it's compatible with anything)
                    if typed.data_type != DataType::Null {
                        element_typed = Some(typed);
                    }
                }
                Some(existing) => {
                    if typed.data_type == DataType::Null {
                        // NULL is compatible with any element type
                        continue;
                    }
                    if typed.data_type != existing.data_type {
                        // Try promotion
                        let promoted = promote_types(existing, &typed);
                        if promoted.data_type == DataType::Unknown {
                            // Mixed types that can't be promoted — reject
                            return None;
                        }
                        element_typed = Some(promoted);
                    }
                }
            }
        } else {
            // Can't infer element type
            return None;
        }
    }

    let elem_type = element_typed.map(|t| t.data_type).unwrap_or(DataType::Null);
    Some(TypedColumn {
        data_type: DataType::Array(Box::new(elem_type)),
        nullable: false, // The array itself is not nullable; elements may be
    })
}

/// Infer the type of an array subscript (arr[i]).
/// Returns the element type of the array.
fn infer_array_subscript_type(expr: &smelt_parser::Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    // The expr contains both the base expression and the ARRAY_SUBSCRIPT node as children.
    // We need to find the base expression (which should be a column ref or other expr
    // that evaluates to an array type) and extract the element type.

    // Find the first child Expr that is NOT inside the ARRAY_SUBSCRIPT
    let base_exprs: Vec<_> = expr
        .syntax()
        .children()
        .filter_map(smelt_parser::Expr::cast)
        .collect();

    // The first Expr child should be the base (e.g., the column reference)
    if let Some(base_expr) = base_exprs.first() {
        if let Some(base_type) = infer_expression_type(base_expr, ctx) {
            if let DataType::Array(inner) = base_type.data_type {
                return Some(TypedColumn {
                    data_type: *inner,
                    nullable: true, // Array element access can always be NULL (out of bounds)
                });
            }
        }
    }

    None
}

/// Infer the type of an array slice (arr[start:end]).
/// Returns the same array type as the base.
fn infer_array_slice_type(expr: &smelt_parser::Expr, ctx: &TypeContext) -> Option<TypedColumn> {
    // Similar to subscript — find the base expression
    let base_exprs: Vec<_> = expr
        .syntax()
        .children()
        .filter_map(smelt_parser::Expr::cast)
        .collect();

    if let Some(base_expr) = base_exprs.first() {
        if let Some(base_type) = infer_expression_type(base_expr, ctx) {
            if let DataType::Array(_) = &base_type.data_type {
                return Some(TypedColumn {
                    data_type: base_type.data_type,
                    nullable: true, // Slice result could be NULL
                });
            }
        }
    }

    None
}

/// Infer the type of a ROW constructor: ROW(1, 2, 3) → Struct with positional fields.
fn infer_row_constructor_type(row: &RowConstructor, ctx: &TypeContext) -> Option<TypedColumn> {
    let elements = row.elements();
    let mut fields = Vec::new();

    for (i, elem) in elements.iter().enumerate() {
        let typed = infer_expression_type(elem, ctx)?;
        // Positional fields: v1, v2, v3, ...
        fields.push((format!("v{}", i + 1), typed.data_type));
    }

    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false, // The struct itself is not nullable
    })
}

/// Infer the type of a struct literal: STRUCT(1 AS a, 'hello' AS b) → Struct with named fields.
fn infer_struct_literal_type(struct_lit: &StructLiteral, ctx: &TypeContext) -> Option<TypedColumn> {
    let fields_ast = struct_lit.fields();
    let mut fields = Vec::new();

    for (i, (expr, name)) in fields_ast.iter().enumerate() {
        let typed = infer_expression_type(expr, ctx)?;
        let field_name = name.clone().unwrap_or_else(|| format!("v{}", i + 1));
        fields.push((field_name, typed.data_type));
    }

    Some(TypedColumn {
        data_type: DataType::Struct(fields),
        nullable: false, // The struct itself is not nullable
    })
}

/// Infer the type of a literal value
fn infer_literal_type(text: &str) -> Option<TypedColumn> {
    let text = text.trim();

    // NULL literal
    if text.eq_ignore_ascii_case("NULL") {
        return Some(TypedColumn {
            data_type: DataType::Null,
            nullable: true,
        });
    }

    // Boolean literals
    if text.eq_ignore_ascii_case("TRUE") || text.eq_ignore_ascii_case("FALSE") {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        });
    }

    // String literals (single or double quoted)
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        return Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        });
    }

    // Numeric literals
    if let Some(num_type) = infer_numeric_literal_type(text) {
        return Some(TypedColumn {
            data_type: num_type,
            nullable: false,
        });
    }

    // SQL standard typed literals: DATE '...', TIMESTAMP '...', TIME '...', INTERVAL '...'
    let upper = text.to_uppercase();
    if upper.starts_with("DATE ") || upper.starts_with("DATE'") {
        return Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        });
    }
    if upper.starts_with("TIMESTAMP ") || upper.starts_with("TIMESTAMP'") {
        return Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: false,
        });
    }
    if upper.starts_with("TIME ") || upper.starts_with("TIME'") {
        return Some(TypedColumn {
            data_type: DataType::Time,
            nullable: false,
        });
    }
    if upper.starts_with("INTERVAL ") || upper.starts_with("INTERVAL'") {
        return Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: false,
        });
    }

    None
}

/// Infer the type of a numeric literal
fn infer_numeric_literal_type(text: &str) -> Option<DataType> {
    // Check for decimal point
    if text.contains('.') {
        // Could be DECIMAL or DOUBLE
        // If it has 'e' or 'E', it's a floating point
        if text.contains('e') || text.contains('E') {
            return Some(DataType::Double);
        }

        // Count digits for precision/scale
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() == 2 {
            let precision = parts[0].trim_start_matches('-').len() + parts[1].len();
            let scale = parts[1].len();
            return Some(DataType::Decimal {
                precision: precision.min(38) as u8,
                scale: scale.min(38) as u8,
            });
        }

        return Some(DataType::Double);
    }

    // Integer literal - check range
    if let Ok(n) = text.parse::<i64>() {
        return Some(if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
            DataType::SmallInt
        } else if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
            DataType::Integer
        } else {
            DataType::BigInt
        });
    }

    // Try parsing as unsigned for very large numbers
    if text.parse::<u64>().is_ok() {
        return Some(DataType::BigInt);
    }

    None
}

/// Infer the type of a binary operand by finding the nth Expr child node.
fn infer_binary_operand(binary: &BinaryExpr, nth: usize, ctx: &TypeContext) -> Option<TypedColumn> {
    let expr = binary.node().children().filter_map(Expr::cast).nth(nth)?;
    infer_expression_type(&expr, ctx)
}

/// Promote two numeric operands to their common widest type.
/// Priority: Double > Float > Decimal > BigInt > Integer > SmallInt
fn promote_numeric_operands(
    left: Option<DataType>,
    right: Option<DataType>,
) -> Option<TypedColumn> {
    match (left, right) {
        (Some(DataType::Double), _) | (_, Some(DataType::Double)) => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),
        (Some(DataType::Float), _) | (_, Some(DataType::Float)) => Some(TypedColumn {
            data_type: DataType::Float,
            nullable: true,
        }),
        (Some(DataType::Decimal { .. }), _) | (_, Some(DataType::Decimal { .. })) => {
            Some(TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            })
        }
        (Some(DataType::BigInt), _) | (_, Some(DataType::BigInt)) => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),
        (Some(DataType::Integer), _) | (_, Some(DataType::Integer)) => Some(TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        }),
        (Some(DataType::SmallInt), _) | (_, Some(DataType::SmallInt)) => Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: true,
        }),
        (Some(l), _) => Some(TypedColumn {
            data_type: l,
            nullable: true,
        }),
        _ => None,
    }
}

/// Infer the result type of a binary expression
fn infer_binary_expr_type(binary: &BinaryExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let op = binary.operator()?;

    match op.to_uppercase().as_str() {
        // Logical operators - always return Boolean
        "AND" | "OR" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // NOT operator (unary) - always returns Boolean
        "NOT" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true, // NOT NULL = NULL
        }),

        // Comparison operators - always return Boolean
        "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" | "IS" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false, // Comparisons always return true/false
        }),

        // Pattern matching operators - always return Boolean
        "LIKE" | "ILIKE" | "~" | "~*" | "!~" | "!~*" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // String concatenation - always returns Text
        "||" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        // Addition — handles numeric promotion and temporal arithmetic
        "+" => {
            let left = infer_binary_operand(binary, 0, ctx);
            let right = infer_binary_operand(binary, 1, ctx);
            let lt = left.as_ref().map(|t| &t.data_type);
            let rt = right.as_ref().map(|t| &t.data_type);

            // Temporal arithmetic for +
            match (lt, rt) {
                // DATE + INTERVAL → Timestamp, INTERVAL + DATE → Timestamp
                (Some(DataType::Date), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Date)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Timestamp {
                            with_timezone: false,
                        },
                        nullable: true,
                    });
                }
                // TIMESTAMP + INTERVAL → Timestamp, INTERVAL + TIMESTAMP → Timestamp
                (Some(DataType::Timestamp { with_timezone }), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Timestamp { with_timezone })) => {
                    return Some(TypedColumn {
                        data_type: DataType::Timestamp {
                            with_timezone: *with_timezone,
                        },
                        nullable: true,
                    });
                }
                // TIME + INTERVAL → Time, INTERVAL + TIME → Time
                (Some(DataType::Time), Some(DataType::Interval))
                | (Some(DataType::Interval), Some(DataType::Time)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Time,
                        nullable: true,
                    });
                }
                // INTERVAL + INTERVAL → Interval
                (Some(DataType::Interval), Some(DataType::Interval)) => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                _ => {}
            }

            // Numeric promotion
            Some(promote_numeric_operands(
                left.map(|t| t.data_type),
                right.map(|t| t.data_type),
            )?)
        }

        // Multiplication, division, and modulo — handles numeric promotion and INTERVAL * numeric
        "*" | "/" | "%" => {
            let left = infer_binary_operand(binary, 0, ctx);
            let right = infer_binary_operand(binary, 1, ctx);
            let lt = left.as_ref().map(|t| &t.data_type);
            let rt = right.as_ref().map(|t| &t.data_type);

            // INTERVAL * numeric → Interval, numeric * INTERVAL → Interval
            // INTERVAL / numeric → Interval
            match (lt, rt) {
                (Some(DataType::Interval), Some(r)) if r.is_numeric() => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                (Some(l), Some(DataType::Interval)) if l.is_numeric() => {
                    return Some(TypedColumn {
                        data_type: DataType::Interval,
                        nullable: true,
                    });
                }
                _ => {}
            }

            // Numeric promotion
            Some(promote_numeric_operands(
                left.map(|t| t.data_type),
                right.map(|t| t.data_type),
            )?)
        }

        // Minus can be binary (a - b) or unary (-a)
        "-" => {
            if binary.is_unary() {
                // Unary minus: -expr preserves the numeric type
                // First try to get operand type from expression
                if let Some(operand_type) =
                    binary.left().and_then(|e| infer_expression_type(&e, ctx))
                {
                    return Some(TypedColumn {
                        data_type: operand_type.data_type,
                        nullable: operand_type.nullable,
                    });
                }

                // For unary expressions with bare identifier operands, look up the column
                if let Some(col_ref) = binary.unary_operand_column() {
                    if let Some(typed_col) = ctx.lookup_column(col_ref.qualifier(), col_ref.name())
                    {
                        return Some(TypedColumn {
                            data_type: typed_col.data_type.clone(),
                            nullable: typed_col.nullable,
                        });
                    }
                }

                None
            } else {
                // Binary minus: a - b
                let left = infer_binary_operand(binary, 0, ctx);
                let right = infer_binary_operand(binary, 1, ctx);
                let lt = left.as_ref().map(|t| &t.data_type);
                let rt = right.as_ref().map(|t| &t.data_type);

                // Temporal arithmetic for -
                match (lt, rt) {
                    // DATE - DATE → Interval
                    (Some(DataType::Date), Some(DataType::Date)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // TIMESTAMP - TIMESTAMP → Interval
                    (Some(DataType::Timestamp { .. }), Some(DataType::Timestamp { .. })) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // TIME - TIME → Interval
                    (Some(DataType::Time), Some(DataType::Time)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    // DATE - INTERVAL → Timestamp
                    (Some(DataType::Date), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Timestamp {
                                with_timezone: false,
                            },
                            nullable: true,
                        });
                    }
                    // TIMESTAMP - INTERVAL → Timestamp
                    (Some(DataType::Timestamp { with_timezone }), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Timestamp {
                                with_timezone: *with_timezone,
                            },
                            nullable: true,
                        });
                    }
                    // TIME - INTERVAL → Time
                    (Some(DataType::Time), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Time,
                            nullable: true,
                        });
                    }
                    // INTERVAL - INTERVAL → Interval
                    (Some(DataType::Interval), Some(DataType::Interval)) => {
                        return Some(TypedColumn {
                            data_type: DataType::Interval,
                            nullable: true,
                        });
                    }
                    _ => {}
                }

                // Numeric promotion
                Some(promote_numeric_operands(
                    left.map(|t| t.data_type),
                    right.map(|t| t.data_type),
                )?)
            }
        }

        // JSON operators — both return Text because smelt represents JSON as Text
        // internally (no DataType::Json variant). Semantically, -> returns JSON
        // (navigable further) while ->> returns plain text. We keep separate arms
        // to preserve this distinction for future DataType::Json support.
        "->" | "#>" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        "->>" | "#>>" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        "@>" | "<@" => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        }),

        _ => None,
    }
}

/// Recursively walk all sub-expressions, calling `visitor` for each column
/// reference encountered. Also triggers `ctx.lookup_column()` for
/// missed-lookup tracking. Unlike `infer_expression_type` which
/// short-circuits (e.g., `||` returns Text without inspecting operands),
/// this function visits ALL operands.
///
/// `type_hint` propagates type context from the parent expression (e.g.,
/// SUM/AVG arguments get a Double hint, binary expression operands get
/// cross-side type inference).
/// Callback type for column reference visitors.
/// Parameters: (qualifier, column_name, type_hint, text_range)
#[allow(clippy::type_complexity)]
pub type ColumnRefVisitor<'a> =
    &'a mut dyn FnMut(Option<&str>, &str, Option<&TypedColumn>, TextRange);

pub fn walk_expression_columns_with_visitor(
    expr: &Expr,
    ctx: &TypeContext,
    type_hint: Option<&TypedColumn>,
    visitor: ColumnRefVisitor<'_>,
) {
    // Leaf: column reference — trigger lookup and visitor
    // Only treat as a leaf if there are no child expression nodes
    // (avoids false-positive from as_column_ref on complex expressions
    // where a bare IDENT token coexists with BINARY_EXPR children)
    let has_expr_children = expr.syntax().children().any(|c| Expr::cast(c).is_some());
    if !has_expr_children {
        if let Some(col_ref) = expr.as_column_ref() {
            let _ = ctx.lookup_column(col_ref.qualifier(), col_ref.name());
            visitor(
                col_ref.qualifier(),
                col_ref.name(),
                type_hint,
                expr.text_range(),
            );
            return;
        }
    }

    // Subquery/EXISTS — skip (different scope)
    if expr.as_exists().is_some() || expr.as_subquery().is_some() {
        return;
    }

    // CASE — special handling for when_clauses/else (no hint propagation)
    if let Some(case_expr) = expr.as_case() {
        if let Some(case_value) = case_expr.case_value() {
            walk_expression_columns_with_visitor(&case_value, ctx, None, visitor);
        }
        for when_clause in case_expr.when_clauses() {
            if let Some(condition) = when_clause.condition() {
                walk_expression_columns_with_visitor(&condition, ctx, None, visitor);
            }
            if let Some(result) = when_clause.result() {
                walk_expression_columns_with_visitor(&result, ctx, None, visitor);
            }
        }
        if let Some(else_expr) = case_expr.else_expr() {
            walk_expression_columns_with_visitor(&else_expr, ctx, None, visitor);
        }
        return;
    }

    // Function call — walk all arguments with type hints for aggregates
    if let Some(func) = expr.as_function_call() {
        let func_name = func.name().map(|n| n.to_uppercase()).unwrap_or_default();
        let arg_hint = match SqlFunction::from_name(&func_name) {
            Some(SqlFunction::Sum | SqlFunction::Avg) => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            _ => None,
        };
        for arg in func.arguments() {
            walk_expression_columns_with_visitor(&arg, ctx, arg_hint.as_ref(), visitor);
        }
        if let Some(filter) = func.filter_clause() {
            if let Some(filter_expr) = filter.expression() {
                walk_expression_columns_with_visitor(&filter_expr, ctx, None, visitor);
            }
        }
        return;
    }

    // Binary expression — apply cross-side type inference when there are
    // exactly 2 child Expr operands (simple binary like `a = 1`). For
    // chained operators (3+ operands) we fall through to the generic handler.
    if expr.as_binary().is_some() {
        let child_exprs: Vec<Expr> = expr.syntax().children().filter_map(Expr::cast).collect();
        if child_exprs.len() == 2 {
            let lhs = &child_exprs[0];
            let rhs = &child_exprs[1];

            let lhs_type = infer_expression_type(lhs, ctx);
            let rhs_type = infer_expression_type(rhs, ctx);

            let lhs_is_col = lhs.as_column_ref().is_some();
            let rhs_is_col = rhs.as_column_ref().is_some();

            let lhs_hint = if lhs_is_col && !rhs_is_col {
                rhs_type.as_ref()
            } else {
                type_hint
            };
            walk_expression_columns_with_visitor(lhs, ctx, lhs_hint, visitor);

            let rhs_hint = if rhs_is_col && !lhs_is_col {
                lhs_type.as_ref()
            } else {
                type_hint
            };
            walk_expression_columns_with_visitor(rhs, ctx, rhs_hint, visitor);
            return;
        }
        // For chained binary operators, fall through to the generic handler
    }

    // For all other expression types (CAST, BETWEEN, IN, chained binary, etc.):
    // Walk all child nodes that can be cast to Expr.
    for child in expr.syntax().children() {
        if let Some(child_expr) = Expr::cast(child) {
            walk_expression_columns_with_visitor(&child_expr, ctx, type_hint, visitor);
        }
    }
}

/// Walk all sub-expressions, calling `lookup_column` on every column reference.
/// Thin wrapper around `walk_expression_columns_with_visitor` with no visitor
/// or type hints — used by property-based tests to detect missing columns.
pub fn walk_expression_columns(expr: &Expr, ctx: &TypeContext) {
    walk_expression_columns_with_visitor(expr, ctx, None, &mut |_, _, _, _| {});
}

/// Walk all expressions in a SELECT statement with a visitor callback.
/// Covers SELECT list, WHERE, GROUP BY, HAVING, QUALIFY, JOIN ON, and ORDER BY.
pub fn walk_select_columns_with_visitor(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
    type_hint: Option<&TypedColumn>,
    visitor: ColumnRefVisitor<'_>,
) {
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            if let Some(expr) = item.expression() {
                walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
            }
        }
    }
    if let Some(where_clause) = select_stmt.where_clause() {
        if let Some(expr) = where_clause.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(from_clause) = select_stmt.from_clause() {
        for join in from_clause.joins() {
            if let Some(condition) = join.condition() {
                if let Some(on_expr) = condition.on_expression() {
                    walk_expression_columns_with_visitor(&on_expr, ctx, type_hint, visitor);
                }
            }
        }
    }
    if let Some(group_by) = select_stmt.group_by_clause() {
        for expr in group_by.expressions() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(having) = select_stmt.having_clause() {
        if let Some(expr) = having.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
    if let Some(order_by) = select_stmt.order_by_clause() {
        for item in order_by.items() {
            if let Some(expr) = item.expression() {
                walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
            }
        }
    }
    if let Some(qualify) = select_stmt.qualify_clause() {
        if let Some(expr) = qualify.expression() {
            walk_expression_columns_with_visitor(&expr, ctx, type_hint, visitor);
        }
    }
}

/// Walk all expressions in a SELECT statement to trigger column lookups.
/// Covers SELECT list, WHERE, GROUP BY, HAVING, QUALIFY, JOIN ON, and ORDER BY.
pub fn walk_select_columns(select_stmt: &SelectStmt, ctx: &TypeContext) {
    walk_select_columns_with_visitor(select_stmt, ctx, None, &mut |_, _, _, _| {});
}

/// Check for column references that don't resolve against declared schemas.
/// Returns diagnostics with accurate source positions.
/// Structured info about an undeclared column
pub struct UndeclaredColumnInfo {
    pub message: String,
    pub range: TextRange,
    pub qualifier: Option<String>,
    pub column_name: String,
}

pub fn check_undeclared_columns(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    let mut undeclared = Vec::new();

    // Collect SELECT aliases — these are valid references in GROUP BY / ORDER BY / HAVING
    let mut select_aliases = std::collections::HashSet::new();
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            if let Some(alias) = item.alias() {
                select_aliases.insert(alias.to_lowercase());
            }
        }
    }

    walk_select_columns_with_visitor(
        select_stmt,
        ctx,
        None,
        &mut |qualifier, col_name, _, range| {
            // Skip SQL keywords that may be parsed as identifiers
            let lower = col_name.to_lowercase();
            if matches!(lower.as_str(), "true" | "false" | "null") {
                return;
            }

            // Skip unqualified references to SELECT aliases (valid in GROUP BY/ORDER BY)
            if qualifier.is_none() && select_aliases.contains(&lower) {
                return;
            }

            // Use `lookup_identifier` so bound function parameters
            // (seeded via `add_function_param` at call-site expansion
            // for `Expr<T>` kinds) resolve before falling back to the
            // FROM scopes. Phase 17 hinge: a SELECT-shaped function
            // body that references `ts_col` / `gap` must see the
            // Expr<Timestamp> / Expr<Interval> bindings populated by
            // the call-site checker.
            if ctx.lookup_identifier(qualifier, col_name).is_some() {
                return;
            }

            let message = if let Some(q) = qualifier {
                if let Some(desc) = ctx.describe_qualifier(q) {
                    format!("Column '{}' not found in {}", col_name, desc)
                } else {
                    format!("Column '{}.{}' not found", q, col_name)
                }
            } else {
                "Column '{}' not found in any source, model, or CTE".replace("{}", col_name)
            };

            undeclared.push(UndeclaredColumnInfo {
                message,
                range,
                qualifier: qualifier.map(|s| s.to_string()),
                column_name: col_name.to_string(),
            });
        },
    );

    undeclared
}

///
/// This extracts columns from the CTE's query and optionally overrides
/// the inferred names with explicit column names if provided.
pub fn infer_cte_columns(cte: &Cte, ctx: &TypeContext) -> Vec<(String, TypedColumn)> {
    // Get the CTE's query (SELECT statement)
    let select_stmt = match cte.query().and_then(|q| q.select_stmt()) {
        Some(s) => s,
        None => return Vec::new(),
    };

    // Phase 46: factor the per-select-item inference into a sibling
    // helper so derived-table / inline-subquery argument resolution
    // (in `tableexpr_schema_lookup`) can share the same path.
    let mut columns = infer_select_output_schema(&select_stmt, ctx);

    // CTE-specific concern: the WITH clause may declare explicit
    // column names that override any inferred names from the SELECT
    // list. Apply the override after the shared inference runs.
    let explicit_names = cte.column_names();
    for (i, name) in explicit_names.iter().enumerate() {
        if i < columns.len() {
            columns[i].0 = name.clone();
        }
    }

    columns
}

/// Phase 46: infer the output schema of a SELECT statement (shared
/// helper used by CTE inference and by `TableExpr` argument resolution
/// for derived tables / inline subqueries).
///
/// Walks the SELECT list, deriving each column's name (from explicit
/// `AS alias`, falling back to a name inferred from the expression, or
/// a generated `colN` when neither applies) and inferring its type via
/// `infer_expression_type` against a context that includes any nested
/// `WITH` clauses in the SELECT.
pub fn infer_select_output_schema(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<(String, TypedColumn)> {
    let mut columns = Vec::new();

    // Build a context that includes any nested CTEs in this SELECT
    let inner_ctx = build_subquery_context(select_stmt, ctx);

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return columns,
    };

    for (i, item) in select_list.items().enumerate() {
        let col_name = if let Some(alias) = item.alias() {
            alias
        } else if let Some(expr) = item.expression() {
            infer_column_name(&expr).unwrap_or_else(|| format!("col{}", i + 1))
        } else {
            format!("col{}", i + 1)
        };

        let typed_col = if let Some(expr) = item.expression() {
            infer_expression_type(&expr, &inner_ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        } else {
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            }
        };

        columns.push((col_name, typed_col));
    }

    columns
}

/// Infer a column name from an expression
///
/// For simple column references, returns the column name.
/// For function calls, returns the function name.
/// For other expressions, returns None.
fn infer_column_name(expr: &Expr) -> Option<String> {
    // Try column reference
    if let Some(col_ref) = expr.as_column_ref() {
        return Some(col_ref.name().to_string());
    }

    // Try EXTRACT expression
    if let Some(_extract) = expr.as_extract() {
        return Some("extract".to_string());
    }

    // Try CASE expression — no natural name, but return a placeholder
    if expr.as_case().is_some() {
        return Some("case_expr".to_string());
    }

    // Try function call - use function name
    if let Some(func) = expr.as_function_call() {
        return func.name();
    }

    // For other expressions, we can't infer a name
    None
}

/// Promote two types to their widest compatible type for UNION operations.
///
/// The result type is the type that can hold values from both input types.
/// For example:
/// - INTEGER + BIGINT → BIGINT
/// - VARCHAR(10) + VARCHAR(20) → Text (we don't track length)
/// - INTEGER + DOUBLE → DOUBLE
/// - Unknown + T → T (Unknown is dominated by any known type)
pub fn promote_types(t1: &TypedColumn, t2: &TypedColumn) -> TypedColumn {
    // If either is Unknown or Null, prefer the other (Null makes result nullable)
    if matches!(t1.data_type, DataType::Unknown | DataType::Null) {
        return TypedColumn {
            data_type: t2.data_type.clone(),
            nullable: t1.nullable || t2.nullable || matches!(t1.data_type, DataType::Null),
        };
    }
    if matches!(t2.data_type, DataType::Unknown | DataType::Null) {
        return TypedColumn {
            data_type: t1.data_type.clone(),
            nullable: t1.nullable || t2.nullable || matches!(t2.data_type, DataType::Null),
        };
    }

    // If same type, return it
    if std::mem::discriminant(&t1.data_type) == std::mem::discriminant(&t2.data_type) {
        // For decimals, take the larger precision/scale
        if let (
            DataType::Decimal {
                precision: p1,
                scale: s1,
            },
            DataType::Decimal {
                precision: p2,
                scale: s2,
            },
        ) = (&t1.data_type, &t2.data_type)
        {
            return TypedColumn {
                data_type: DataType::Decimal {
                    precision: (*p1).max(*p2),
                    scale: (*s1).max(*s2),
                },
                nullable: t1.nullable || t2.nullable,
            };
        }
        return TypedColumn {
            data_type: t1.data_type.clone(),
            nullable: t1.nullable || t2.nullable,
        };
    }

    // Check if both types are in the same family before cross-type promotion
    let both_numeric = t1.data_type.is_numeric() && t2.data_type.is_numeric();
    let both_string = t1.data_type.is_string() && t2.data_type.is_string();
    let both_temporal = t1.data_type.is_temporal() && t2.data_type.is_temporal();

    let promoted_type = match (&t1.data_type, &t2.data_type) {
        // Numeric type promotion: SmallInt < Integer < BigInt < Float < Decimal < Double
        _ if both_numeric => match (&t1.data_type, &t2.data_type) {
            (DataType::Double, _) | (_, DataType::Double) => DataType::Double,
            (DataType::Float, _) | (_, DataType::Float) => DataType::Float,
            // When a Decimal combines with an integer type, widen to Decimal(38,10)
            // to avoid overflow. E.g. CASE WHEN ... THEN 150::INTEGER ELSE 0.5::DECIMAL(2,1)
            // should not produce DECIMAL(2,1) which can only hold up to 9.9.
            (
                DataType::Decimal { .. },
                DataType::SmallInt | DataType::Integer | DataType::BigInt,
            )
            | (
                DataType::SmallInt | DataType::Integer | DataType::BigInt,
                DataType::Decimal { .. },
            ) => DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            (DataType::Decimal { precision, scale }, _)
            | (_, DataType::Decimal { precision, scale }) => DataType::Decimal {
                precision: *precision,
                scale: *scale,
            },
            (DataType::BigInt, _) | (_, DataType::BigInt) => DataType::BigInt,
            (DataType::Integer, _) | (_, DataType::Integer) => DataType::Integer,
            _ => t1.data_type.clone(),
        },

        // String type promotion: all string types → Text
        _ if both_string => DataType::Text,

        // Temporal type promotion
        _ if both_temporal => match (&t1.data_type, &t2.data_type) {
            (
                DataType::Timestamp { with_timezone: tz1 },
                DataType::Timestamp { with_timezone: tz2 },
            ) => DataType::Timestamp {
                with_timezone: *tz1 || *tz2,
            },
            (DataType::Timestamp { with_timezone }, _)
            | (_, DataType::Timestamp { with_timezone }) => DataType::Timestamp {
                with_timezone: *with_timezone,
            },
            (DataType::Date, DataType::Time) | (DataType::Time, DataType::Date) => {
                DataType::Timestamp {
                    with_timezone: false,
                }
            }
            _ => DataType::Unknown,
        },

        // For incompatible type families, return Unknown (could be an error in strict mode)
        _ => DataType::Unknown,
    };

    TypedColumn {
        data_type: promoted_type,
        nullable: t1.nullable || t2.nullable,
    }
}

/// Infer column types for a SELECT statement, handling UNION if present.
///
/// For a simple SELECT, returns the types of each column in the select list.
/// For a UNION, combines types from all branches using type promotion.
pub fn infer_select_column_types(select_stmt: &SelectStmt, ctx: &TypeContext) -> Vec<TypedColumn> {
    let mut column_types = Vec::new();

    // Get types from the first SELECT's select list
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            let typed_col = if let Some(expr) = item.expression() {
                infer_expression_type(&expr, ctx).unwrap_or(TypedColumn {
                    data_type: DataType::Unknown,
                    nullable: true,
                })
            } else {
                TypedColumn {
                    data_type: DataType::Unknown,
                    nullable: true,
                }
            };
            column_types.push(typed_col);
        }
    }

    // If there's a set operation (UNION/INTERSECT/EXCEPT), recursively get types and combine
    if select_stmt.has_set_operation() {
        if let Some(next_select) = select_stmt.set_operation_select() {
            let next_types = infer_select_column_types(&next_select, ctx);

            // Combine types - use the wider type for each column position
            for (i, next_type) in next_types.into_iter().enumerate() {
                if i < column_types.len() {
                    column_types[i] = promote_types(&column_types[i], &next_type);
                }
                // If next has more columns, they're ignored (SQL requires same column count)
            }
        }
    }

    column_types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_type_inference() {
        // SmallInt (small values fit in SmallInt)
        assert_eq!(
            infer_literal_type("42"),
            Some(TypedColumn {
                data_type: DataType::SmallInt,
                nullable: false,
            })
        );

        // Integer (larger values that don't fit in SmallInt)
        assert_eq!(
            infer_literal_type("100000"),
            Some(TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            })
        );

        // BigInt
        assert_eq!(
            infer_literal_type("9999999999"),
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            })
        );

        // Decimal
        let decimal_type = infer_literal_type("123.45").unwrap();
        assert!(matches!(decimal_type.data_type, DataType::Decimal { .. }));
        assert!(!decimal_type.nullable);

        // Double (scientific notation)
        assert_eq!(
            infer_literal_type("1.5e10"),
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: false,
            })
        );

        // String
        assert_eq!(
            infer_literal_type("'hello'"),
            Some(TypedColumn {
                data_type: DataType::Text,
                nullable: false,
            })
        );

        // Boolean
        assert_eq!(
            infer_literal_type("TRUE"),
            Some(TypedColumn {
                data_type: DataType::Boolean,
                nullable: false,
            })
        );

        // NULL
        assert_eq!(
            infer_literal_type("NULL"),
            Some(TypedColumn {
                data_type: DataType::Null,
                nullable: true,
            })
        );
    }

    #[test]
    fn test_type_context_lookup() {
        let mut ctx = TypeContext::new();

        ctx.add_source_column(
            "raw",
            "users",
            "id",
            TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            },
        );

        ctx.add_model_column(
            "staging_users",
            "user_id",
            TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            },
        );

        // Look up source column with qualifier
        let result = ctx.lookup_column(Some("users"), "id");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::Integer);

        // Look up model column with qualifier
        let result = ctx.lookup_column(Some("staging_users"), "user_id");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);

        // Look up without qualifier (unambiguous)
        let result = ctx.lookup_column(None, "id");
        assert!(result.is_some());
    }

    #[test]
    fn test_aggregate_function_types() {
        let ctx = TypeContext::new();

        // Create a mock expression text for COUNT
        // Note: In real usage, we'd use the actual AST
        let count_type = infer_function_type_by_name("COUNT", &ctx).unwrap();
        assert_eq!(count_type.data_type, DataType::BigInt);
        assert!(!count_type.nullable);

        // AVG returns Double
        let avg_type = infer_function_type_by_name("AVG", &ctx).unwrap();
        assert_eq!(avg_type.data_type, DataType::Double);
        assert!(avg_type.nullable);

        // SUM returns Decimal
        let sum_type = infer_function_type_by_name("SUM", &ctx).unwrap();
        assert!(matches!(sum_type.data_type, DataType::Decimal { .. }));
    }

    // Helper for testing function types without AST
    fn infer_function_type_by_name(name: &str, _ctx: &TypeContext) -> Option<TypedColumn> {
        match name.to_uppercase().as_str() {
            // Aggregate functions
            "COUNT" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: false,
            }),
            "AVG" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            "SUM" => Some(TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            }),
            // Math functions
            "SQRT" | "POWER" | "POW" | "EXP" | "LN" | "LOG" | "LOG10" | "LOG2" => {
                Some(TypedColumn {
                    data_type: DataType::Double,
                    nullable: true,
                })
            }
            "PI" | "RANDOM" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: false,
            }),
            "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" => Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            }),
            // Date/time functions
            "EXTRACT" | "DATE_PART" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            }),
            "MAKE_DATE" => Some(TypedColumn {
                data_type: DataType::Date,
                nullable: true,
            }),
            "AGE" => Some(TypedColumn {
                data_type: DataType::Interval,
                nullable: true,
            }),
            // String functions
            "REPLACE" | "SPLIT_PART" | "LEFT" | "RIGHT" | "LPAD" | "RPAD" => Some(TypedColumn {
                data_type: DataType::Text,
                nullable: true,
            }),
            "POSITION" | "STRPOS" => Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            }),
            "STRING_AGG" | "LISTAGG" => Some(TypedColumn {
                data_type: DataType::Text,
                nullable: true,
            }),
            _ => None,
        }
    }

    #[test]
    fn test_cte_column_lookup() {
        let mut ctx = TypeContext::new();

        // Add a CTE column
        ctx.add_cte_column(
            "daily_totals",
            "day",
            TypedColumn {
                data_type: DataType::Date,
                nullable: false,
            },
        );

        ctx.add_cte_column(
            "daily_totals",
            "total",
            TypedColumn {
                data_type: DataType::Decimal {
                    precision: 38,
                    scale: 10,
                },
                nullable: true,
            },
        );

        // Check that CTE is registered
        assert!(ctx.is_cte("daily_totals"));
        assert!(!ctx.is_cte("nonexistent"));

        // Look up CTE column with qualifier
        let result = ctx.lookup_column(Some("daily_totals"), "day");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::Date);

        // Look up CTE column without qualifier
        let result = ctx.lookup_column(None, "total");
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap().data_type,
            DataType::Decimal { .. }
        ));
    }

    #[test]
    fn test_cte_shadows_source() {
        let mut ctx = TypeContext::new();

        // Add a source column with name "orders"
        ctx.add_source_column(
            "raw",
            "orders",
            "amount",
            TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            },
        );

        // Add a CTE with the same name "orders" but different column type
        ctx.add_cte_column(
            "orders",
            "amount",
            TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            },
        );

        // CTE should shadow the source - BigInt should be returned, not Integer
        let result = ctx.lookup_column(Some("orders"), "amount");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);

        // Unqualified lookup should also return CTE column
        let result = ctx.lookup_column(None, "amount");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data_type, DataType::BigInt);
    }

    #[test]
    fn test_extended_function_types() {
        let ctx = TypeContext::new();

        // Math functions
        let sqrt = infer_function_type_by_name("SQRT", &ctx).unwrap();
        assert_eq!(sqrt.data_type, DataType::Double);

        let power = infer_function_type_by_name("POWER", &ctx).unwrap();
        assert_eq!(power.data_type, DataType::Double);

        let pi = infer_function_type_by_name("PI", &ctx).unwrap();
        assert_eq!(pi.data_type, DataType::Double);
        assert!(!pi.nullable); // PI is never null

        let sin = infer_function_type_by_name("SIN", &ctx).unwrap();
        assert_eq!(sin.data_type, DataType::Double);

        // Date/time functions
        let extract = infer_function_type_by_name("EXTRACT", &ctx).unwrap();
        assert_eq!(extract.data_type, DataType::BigInt);

        let make_date = infer_function_type_by_name("MAKE_DATE", &ctx).unwrap();
        assert_eq!(make_date.data_type, DataType::Date);

        let age = infer_function_type_by_name("AGE", &ctx).unwrap();
        assert_eq!(age.data_type, DataType::Interval);

        // String functions
        let replace = infer_function_type_by_name("REPLACE", &ctx).unwrap();
        assert_eq!(replace.data_type, DataType::Text);

        let position = infer_function_type_by_name("POSITION", &ctx).unwrap();
        assert_eq!(position.data_type, DataType::BigInt);

        let split_part = infer_function_type_by_name("SPLIT_PART", &ctx).unwrap();
        assert_eq!(split_part.data_type, DataType::Text);

        // String aggregate
        let string_agg = infer_function_type_by_name("STRING_AGG", &ctx).unwrap();
        assert_eq!(string_agg.data_type, DataType::Text);
    }

    /// Parse a SQL SELECT and return the inferred types of all columns.
    fn infer_sql(sql: &str) -> Vec<TypedColumn> {
        infer_sql_with_ctx(sql, &TypeContext::new())
    }

    fn infer_sql_with_ctx(sql: &str, ctx: &TypeContext) -> Vec<TypedColumn> {
        use smelt_parser::ast::File;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("failed to cast to File");
        let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");
        infer_select_column_types(&select_stmt, ctx)
    }

    #[test]
    fn test_coalesce_nullability() {
        // COALESCE with a non-null literal → non-nullable
        let types = infer_sql("SELECT COALESCE(NULL, 42)");
        assert_eq!(types[0].data_type, DataType::SmallInt);
        assert!(
            !types[0].nullable,
            "COALESCE with non-null literal should be non-nullable"
        );

        // COALESCE with all nullable columns → nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
        ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT COALESCE(a, b) FROM t",
            &ctx,
        );
        assert_eq!(types[0].data_type, DataType::Integer);
        assert!(
            types[0].nullable,
            "COALESCE with all nullable args should be nullable"
        );

        // COALESCE where second arg is non-nullable → non-nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
        ctx.add_cte_column("t", "b", TypedColumn::not_null(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT COALESCE(a, b) FROM t",
            &ctx,
        );
        assert!(
            !types[0].nullable,
            "COALESCE with non-nullable arg should be non-nullable"
        );

        // COALESCE with a non-null literal as fallback → non-nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS a) SELECT COALESCE(a, 0) FROM t",
            &ctx,
        );
        assert!(
            !types[0].nullable,
            "COALESCE with literal fallback should be non-nullable"
        );
    }

    #[test]
    fn test_case_nullability() {
        // CASE without ELSE → always nullable (implicit NULL)
        let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 END");
        assert_eq!(types[0].data_type, DataType::SmallInt);
        assert!(types[0].nullable, "CASE without ELSE should be nullable");

        // CASE with ELSE, all branches non-nullable → non-nullable
        let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 ELSE 0 END");
        assert!(
            !types[0].nullable,
            "CASE with ELSE and non-nullable branches should be non-nullable"
        );

        // CASE with ELSE, but a branch returns NULL → nullable
        let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN NULL ELSE 0 END");
        assert!(
            types[0].nullable,
            "CASE with NULL branch should be nullable"
        );

        // CASE with ELSE that is NULL → nullable
        let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 ELSE NULL END");
        assert!(types[0].nullable, "CASE with NULL ELSE should be nullable");

        // CASE with multiple WHEN branches, all non-nullable + ELSE → non-nullable
        let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 WHEN 2 = 2 THEN 99 ELSE 0 END");
        assert!(
            !types[0].nullable,
            "CASE with all non-nullable branches and ELSE should be non-nullable"
        );
    }

    #[test]
    fn test_cast_nullability() {
        // CAST of non-nullable literal → non-nullable
        let types = infer_sql("SELECT CAST(42 AS VARCHAR)");
        assert_eq!(types[0].data_type, DataType::Varchar { max_length: None });
        assert!(
            !types[0].nullable,
            "CAST of non-nullable literal should be non-nullable"
        );

        // CAST of NULL → nullable
        let types = infer_sql("SELECT CAST(NULL AS INTEGER)");
        assert!(types[0].nullable, "CAST of NULL should be nullable");

        // CAST of non-nullable column → non-nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "x", TypedColumn::not_null(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS x) SELECT CAST(x AS VARCHAR) FROM t",
            &ctx,
        );
        assert!(
            !types[0].nullable,
            "CAST of non-nullable column should be non-nullable"
        );

        // CAST of nullable column → nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "x", TypedColumn::nullable(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS x) SELECT CAST(x AS VARCHAR) FROM t",
            &ctx,
        );
        assert!(
            types[0].nullable,
            "CAST of nullable column should be nullable"
        );
    }

    #[test]
    fn test_ifnull_nullability() {
        // IFNULL with non-null literal fallback → non-nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
        let types =
            infer_sql_with_ctx("WITH t AS (SELECT 1 AS a) SELECT IFNULL(a, 0) FROM t", &ctx);
        assert_eq!(types[0].data_type, DataType::Integer);
        assert!(
            !types[0].nullable,
            "IFNULL with non-null literal fallback should be non-nullable"
        );

        // IFNULL with both nullable → nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
        ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT IFNULL(a, b) FROM t",
            &ctx,
        );
        assert!(
            types[0].nullable,
            "IFNULL with both nullable should be nullable"
        );

        // IFNULL where first arg is non-nullable → non-nullable
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "a", TypedColumn::not_null(DataType::Integer));
        ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT IFNULL(a, b) FROM t",
            &ctx,
        );
        assert!(
            !types[0].nullable,
            "IFNULL with non-nullable first arg should be non-nullable"
        );
    }

    #[test]
    fn test_temporal_arithmetic_date_interval() {
        // DATE + INTERVAL → Timestamp
        let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) + INTERVAL '1' DAY");
        assert_eq!(
            types[0].data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // DATE - INTERVAL → Timestamp
        let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) - INTERVAL '1' DAY");
        assert_eq!(
            types[0].data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // DATE - DATE → Interval
        let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) - CAST('2024-01-02' AS DATE)");
        assert_eq!(types[0].data_type, DataType::Interval);
    }

    #[test]
    fn test_temporal_arithmetic_timestamp_interval() {
        // TIMESTAMP + INTERVAL → Timestamp
        let types = infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) + INTERVAL '1' HOUR");
        assert_eq!(
            types[0].data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // TIMESTAMP - INTERVAL → Timestamp
        let types = infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) - INTERVAL '1' HOUR");
        assert_eq!(
            types[0].data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // TIMESTAMP - TIMESTAMP → Interval
        let types =
            infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) - CAST('2024-01-02' AS TIMESTAMP)");
        assert_eq!(types[0].data_type, DataType::Interval);
    }

    #[test]
    fn test_temporal_arithmetic_interval_ops() {
        // INTERVAL + INTERVAL → Interval
        let types = infer_sql("SELECT INTERVAL '1' DAY + INTERVAL '2' HOUR");
        assert_eq!(types[0].data_type, DataType::Interval);

        // INTERVAL - INTERVAL → Interval
        let types = infer_sql("SELECT INTERVAL '1' DAY - INTERVAL '2' HOUR");
        assert_eq!(types[0].data_type, DataType::Interval);

        // INTERVAL * numeric → Interval
        let types = infer_sql("SELECT INTERVAL '1' DAY * 3");
        assert_eq!(types[0].data_type, DataType::Interval);

        // numeric * INTERVAL → Interval
        let types = infer_sql("SELECT 3 * INTERVAL '1' DAY");
        assert_eq!(types[0].data_type, DataType::Interval);

        // INTERVAL / numeric → Interval
        let types = infer_sql("SELECT INTERVAL '6' HOUR / 2");
        assert_eq!(types[0].data_type, DataType::Interval);
    }

    #[test]
    fn test_temporal_arithmetic_time() {
        // TIME + INTERVAL → Time
        let types = infer_sql("SELECT CAST('12:00:00' AS TIME) + INTERVAL '1' HOUR");
        assert_eq!(types[0].data_type, DataType::Time);

        // TIME - INTERVAL → Time
        let types = infer_sql("SELECT CAST('12:00:00' AS TIME) - INTERVAL '1' HOUR");
        assert_eq!(types[0].data_type, DataType::Time);

        // TIME - TIME → Interval
        let types = infer_sql("SELECT CAST('12:00:00' AS TIME) - CAST('10:00:00' AS TIME)");
        assert_eq!(types[0].data_type, DataType::Interval);
    }

    #[test]
    fn test_temporal_arithmetic_with_columns() {
        // Test with typed columns from context
        let mut ctx = TypeContext::new();
        ctx.add_cte_column("t", "d", TypedColumn::not_null(DataType::Date));
        ctx.add_cte_column(
            "t",
            "ts",
            TypedColumn::not_null(DataType::Timestamp {
                with_timezone: false,
            }),
        );
        ctx.add_cte_column("t", "i", TypedColumn::not_null(DataType::Interval));

        // Column DATE + INTERVAL → Timestamp
        let types = infer_sql_with_ctx(
            "WITH t AS (SELECT 1 AS d) SELECT d + INTERVAL '1' DAY FROM t",
            &ctx,
        );
        assert_eq!(
            types[0].data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // Column TIMESTAMP - Column TIMESTAMP → Interval
        let types = infer_sql_with_ctx("WITH t AS (SELECT 1 AS ts) SELECT ts - ts FROM t", &ctx);
        assert_eq!(types[0].data_type, DataType::Interval);
    }

    #[test]
    fn test_promote_types_numeric_hierarchy() {
        let mk = |dt: DataType| TypedColumn {
            data_type: dt,
            nullable: false,
        };

        // SmallInt + Integer → Integer
        assert_eq!(
            promote_types(&mk(DataType::SmallInt), &mk(DataType::Integer)).data_type,
            DataType::Integer
        );
        // Integer + BigInt → BigInt
        assert_eq!(
            promote_types(&mk(DataType::Integer), &mk(DataType::BigInt)).data_type,
            DataType::BigInt
        );
        // BigInt + Float → Float
        assert_eq!(
            promote_types(&mk(DataType::BigInt), &mk(DataType::Float)).data_type,
            DataType::Float
        );
        // Float + Double → Double
        assert_eq!(
            promote_types(&mk(DataType::Float), &mk(DataType::Double)).data_type,
            DataType::Double
        );
        // Float + Decimal → Float
        assert_eq!(
            promote_types(
                &mk(DataType::Float),
                &mk(DataType::Decimal {
                    precision: 10,
                    scale: 2
                })
            )
            .data_type,
            DataType::Float
        );
        // Decimal + Integer → Decimal(38,10) (widened to prevent overflow)
        // e.g. CASE WHEN ... THEN 150::INTEGER ELSE col::DECIMAL(10,2) must hold integer values
        assert_eq!(
            promote_types(
                &mk(DataType::Decimal {
                    precision: 10,
                    scale: 2
                }),
                &mk(DataType::Integer)
            )
            .data_type,
            DataType::Decimal {
                precision: 38,
                scale: 10
            }
        );
    }

    #[test]
    fn test_promote_types_null_handling() {
        let mk = |dt: DataType| TypedColumn {
            data_type: dt,
            nullable: false,
        };

        // Null + Integer → Integer (nullable)
        let result = promote_types(&mk(DataType::Null), &mk(DataType::Integer));
        assert_eq!(result.data_type, DataType::Integer);
        assert!(result.nullable);

        // Integer + Null → Integer (nullable)
        let result = promote_types(&mk(DataType::Integer), &mk(DataType::Null));
        assert_eq!(result.data_type, DataType::Integer);
        assert!(result.nullable);

        // Unknown + Text → Text
        let result = promote_types(&mk(DataType::Unknown), &mk(DataType::Text));
        assert_eq!(result.data_type, DataType::Text);
    }

    #[test]
    fn test_promote_types_temporal() {
        let mk = |dt: DataType| TypedColumn {
            data_type: dt,
            nullable: false,
        };

        // Date + Timestamp → Timestamp
        assert_eq!(
            promote_types(
                &mk(DataType::Date),
                &mk(DataType::Timestamp {
                    with_timezone: false
                })
            )
            .data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );

        // Date + Time → Timestamp
        assert_eq!(
            promote_types(&mk(DataType::Date), &mk(DataType::Time)).data_type,
            DataType::Timestamp {
                with_timezone: false
            }
        );
    }

    #[test]
    fn test_promote_types_string() {
        let mk = |dt: DataType| TypedColumn {
            data_type: dt,
            nullable: false,
        };

        // Varchar + Text → Text
        assert_eq!(
            promote_types(
                &mk(DataType::Varchar {
                    max_length: Some(10)
                }),
                &mk(DataType::Text)
            )
            .data_type,
            DataType::Text
        );

        // Varchar + Varchar → Text (different discriminant doesn't matter, same variant)
        assert_eq!(
            promote_types(
                &mk(DataType::Varchar {
                    max_length: Some(10)
                }),
                &mk(DataType::Varchar {
                    max_length: Some(20)
                })
            )
            .data_type,
            DataType::Varchar {
                max_length: Some(10)
            } // same discriminant, returns first
        );
    }

    #[test]
    fn test_union_type_inference() {
        // UNION of SmallInt + Integer → Integer
        let types =
            infer_sql("SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x");
        assert_eq!(types[0].data_type, DataType::Integer);

        // UNION of Integer + BigInt → BigInt
        let types =
            infer_sql("SELECT CAST(1 AS INTEGER) AS x UNION ALL SELECT CAST(2 AS BIGINT) AS x");
        assert_eq!(types[0].data_type, DataType::BigInt);

        // 3-way UNION: SmallInt + Integer + BigInt → BigInt
        let types = infer_sql(
            "SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x UNION ALL SELECT CAST(3 AS BIGINT) AS x"
        );
        assert_eq!(types[0].data_type, DataType::BigInt);
    }

    #[test]
    fn test_intersect_except_type_inference() {
        // INTERSECT should also promote types
        let types =
            infer_sql("SELECT CAST(1 AS SMALLINT) AS x INTERSECT SELECT CAST(2 AS INTEGER) AS x");
        assert_eq!(types[0].data_type, DataType::Integer);

        // EXCEPT should also promote types
        let types =
            infer_sql("SELECT CAST(1 AS INTEGER) AS x EXCEPT SELECT CAST(2 AS BIGINT) AS x");
        assert_eq!(types[0].data_type, DataType::BigInt);
    }

    #[test]
    fn test_promote_types_decimal_precision() {
        let mk = |dt: DataType| TypedColumn {
            data_type: dt,
            nullable: false,
        };

        // Decimal(10,2) + Decimal(18,4) → Decimal(18,4) (takes max)
        assert_eq!(
            promote_types(
                &mk(DataType::Decimal {
                    precision: 10,
                    scale: 2
                }),
                &mk(DataType::Decimal {
                    precision: 18,
                    scale: 4
                })
            )
            .data_type,
            DataType::Decimal {
                precision: 18,
                scale: 4
            }
        );
    }

    #[test]
    fn test_array_literal_integer() {
        let types = infer_sql("SELECT ARRAY[1, 2, 3]");
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::SmallInt))
        );
        assert!(!types[0].nullable, "array literal should be non-nullable");
    }

    #[test]
    fn test_array_literal_string() {
        let types = infer_sql("SELECT ARRAY['a', 'b', 'c']");
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::Text))
        );
    }

    #[test]
    fn test_array_literal_empty() {
        let types = infer_sql("SELECT ARRAY[]");
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::Unknown))
        );
    }

    #[test]
    fn test_array_literal_with_null() {
        // ARRAY[1, NULL, 3] — NULL is compatible, element type is SmallInt
        let types = infer_sql("SELECT ARRAY[1, NULL, 3]");
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::SmallInt))
        );
    }

    #[test]
    fn test_array_literal_numeric_promotion() {
        // ARRAY[1, 2.5] — SmallInt + Decimal should promote
        let types = infer_sql("SELECT ARRAY[1, 100000]");
        // 1 is SmallInt, 100000 is Integer → promoted to Integer
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::Integer))
        );
    }

    #[test]
    fn test_array_literal_mixed_types_rejected() {
        // ARRAY[1, 'hello'] — Integer + Text can't be promoted → should fail inference
        let types = infer_sql("SELECT ARRAY[1, 'hello']");
        // Mixed types return Unknown since the array literal inference returns None
        assert_eq!(types[0].data_type, DataType::Unknown);
    }

    #[test]
    fn test_array_subscript_from_column() {
        // With a column of Array(Integer) type, subscript should return Integer
        let mut ctx = TypeContext::new();
        ctx.add_model_column(
            "t",
            "arr",
            TypedColumn::not_null(DataType::Array(Box::new(DataType::Integer))),
        );
        let types = infer_sql_with_ctx("SELECT arr[1]", &ctx);
        assert_eq!(types[0].data_type, DataType::Integer);
        assert!(types[0].nullable, "array element access should be nullable");
    }

    #[test]
    fn test_array_slice_from_column() {
        // Slice should return the same array type
        let mut ctx = TypeContext::new();
        ctx.add_model_column(
            "t",
            "arr",
            TypedColumn::not_null(DataType::Array(Box::new(DataType::Integer))),
        );
        let types = infer_sql_with_ctx("SELECT arr[1:3]", &ctx);
        assert_eq!(
            types[0].data_type,
            DataType::Array(Box::new(DataType::Integer))
        );
    }

    #[test]
    fn test_row_constructor_type() {
        // ROW(1, 'hello', TRUE) → Struct with positional fields
        let types = infer_sql("SELECT ROW(1, 'hello', TRUE)");
        assert_eq!(
            types[0].data_type,
            DataType::Struct(vec![
                ("v1".to_string(), DataType::SmallInt),
                ("v2".to_string(), DataType::Text),
                ("v3".to_string(), DataType::Boolean),
            ])
        );
        assert!(!types[0].nullable); // Struct itself is not nullable
    }

    #[test]
    fn test_struct_literal_named_fields() {
        // STRUCT(1 AS a, 'hello' AS b) → Struct with named fields
        let types = infer_sql("SELECT STRUCT(1 AS a, 'hello' AS b)");
        assert_eq!(
            types[0].data_type,
            DataType::Struct(vec![
                ("a".to_string(), DataType::SmallInt),
                ("b".to_string(), DataType::Text),
            ])
        );
        assert!(!types[0].nullable);
    }

    #[test]
    fn test_struct_literal_unnamed_fields() {
        // STRUCT(1, 2, 3) without AS → positional names
        let types = infer_sql("SELECT STRUCT(1, 2, 3)");
        assert_eq!(
            types[0].data_type,
            DataType::Struct(vec![
                ("v1".to_string(), DataType::SmallInt),
                ("v2".to_string(), DataType::SmallInt),
                ("v3".to_string(), DataType::SmallInt),
            ])
        );
    }

    #[test]
    fn test_struct_literal_mixed_named_unnamed() {
        // STRUCT(1 AS a, 'hello') → mix of named and positional
        let types = infer_sql("SELECT STRUCT(1 AS a, 'hello')");
        assert_eq!(
            types[0].data_type,
            DataType::Struct(vec![
                ("a".to_string(), DataType::SmallInt),
                ("v2".to_string(), DataType::Text),
            ])
        );
    }

    #[test]
    fn test_struct_field_access() {
        // Field access on a struct-typed column
        let mut ctx = TypeContext::new();
        ctx.add_model_column(
            "t",
            "s",
            TypedColumn::not_null(DataType::Struct(vec![
                ("name".to_string(), DataType::Text),
                ("age".to_string(), DataType::Integer),
            ])),
        );
        let types = infer_sql_with_ctx("SELECT s.name", &ctx);
        assert_eq!(types[0].data_type, DataType::Text);
        assert!(types[0].nullable); // Field access is nullable (struct could be null)
    }

    #[test]
    fn test_struct_field_access_case_insensitive() {
        let mut ctx = TypeContext::new();
        ctx.add_model_column(
            "t",
            "s",
            TypedColumn::not_null(DataType::Struct(vec![("Name".to_string(), DataType::Text)])),
        );
        let types = infer_sql_with_ctx("SELECT s.name", &ctx);
        assert_eq!(types[0].data_type, DataType::Text);
    }

    #[test]
    fn test_struct_display() {
        let dt = DataType::Struct(vec![
            ("a".to_string(), DataType::Integer),
            ("b".to_string(), DataType::Text),
        ]);
        assert_eq!(dt.to_sql(), "STRUCT(a INTEGER, b TEXT)");
    }

    #[test]
    fn test_modulo_operator() {
        // Integer % Integer → Integer
        let types = infer_sql("SELECT 10 % 3");
        assert_eq!(types[0].data_type, DataType::SmallInt);

        // CAST to explicit types
        let types = infer_sql("SELECT CAST(10 AS INTEGER) % CAST(3 AS INTEGER)");
        assert_eq!(types[0].data_type, DataType::Integer);

        // BigInt % Integer → BigInt (promotion)
        let types = infer_sql("SELECT CAST(10 AS BIGINT) % CAST(3 AS INTEGER)");
        assert_eq!(types[0].data_type, DataType::BigInt);

        // Double % Double → Double
        let types = infer_sql("SELECT CAST(10.5 AS DOUBLE) % CAST(3.0 AS DOUBLE)");
        assert_eq!(types[0].data_type, DataType::Double);
    }

    // Bug #7: promote_types should widen narrow decimal when combined with wider integer type
    // CASE WHEN cond THEN integer_col ELSE decimal_literal END should not produce a narrow type
    #[test]
    fn test_decimal_case_widening() {
        // CASE result combining Integer and Decimal{2,1}: should widen to at least Decimal{38,10}
        // so that integer values like 100 don't overflow the decimal type
        let types = infer_sql(
            "SELECT CASE WHEN TRUE THEN CAST(150 AS INTEGER) ELSE CAST(0.5 AS DECIMAL(2,1)) END",
        );
        match &types[0].data_type {
            DataType::Decimal { precision, scale } => {
                // precision - scale = integer digits available; must be >= 3 for value 150
                let integer_digits = precision - scale;
                assert!(
                    integer_digits >= 3,
                    "CASE of Integer/Decimal should widen to allow values like 150, got DECIMAL({precision},{scale})"
                );
            }
            other => panic!("Expected Decimal, got {other:?}"),
        }
    }

    // Bug #8: CAST(x AS FLOAT) should infer as Double (FLOAT normalizes to DOUBLE)
    #[test]
    fn test_cast_float_normalizes_to_double() {
        let types = infer_sql("SELECT CAST(1 AS FLOAT)");
        assert_eq!(
            types[0].data_type,
            DataType::Double,
            "CAST AS FLOAT should infer as Double"
        );
    }

    /// Phase 5 unit: seeded function parameters shadow outer column scope.
    ///
    /// §16 #1 of the smelt-functions research pins the resolution order:
    /// params resolve *before* any SQL scope. This test proves the
    /// ordering in isolation — no parser, no Salsa — so Phase 6 and
    /// beyond can compose on top of it with confidence.
    #[test]
    fn param_shadows_outer_name_lookup_logic() {
        let mut ctx = TypeContext::new();
        // Seed a model column `bar.x: Integer` — this is what
        // `lookup_column` would return if we consulted it directly.
        ctx.add_model_column("bar", "x", TypedColumn::nullable(DataType::Integer));

        // Sanity: `lookup_column(None, "x")` currently sees only the model
        // column, returning Integer.
        let via_column = ctx
            .lookup_column(None, "x")
            .expect("model column should be resolvable before param binding");
        assert_eq!(via_column.data_type, DataType::Integer);

        // Now seed a function param `x: Double`. Per §16 #1, the param
        // wins on unqualified lookups through `lookup_identifier`.
        ctx.add_function_param("x", TypedColumn::nullable(DataType::Double));
        assert!(ctx.has_function_param("x"));

        let via_identifier = ctx
            .lookup_identifier(None, "x")
            .expect("seeded param should resolve through lookup_identifier");
        assert_eq!(
            via_identifier.data_type,
            DataType::Double,
            "param type must shadow model type on unqualified lookups"
        );

        // Qualified lookups still bypass the param scope — params are
        // bare names.
        let via_qualified = ctx.lookup_identifier(Some("bar"), "x");
        assert_eq!(
            via_qualified.map(|c| c.data_type.clone()),
            Some(DataType::Integer),
            "qualified lookup must ignore function params"
        );
    }

    // ── Phase 49: check_window_in_scalar_contexts recurses into subqueries ──

    fn parse_select(sql: &str) -> smelt_parser::ast::SelectStmt {
        use smelt_parser::ast::File;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("failed to cast to File");
        file.select_stmt().expect("no SelectStmt in parsed SQL")
    }

    /// WHERE contains a scalar subquery whose body includes a window function.
    /// Expected: at least one WindowInScalarContextInfo with clause "WHERE".
    #[test]
    fn where_subquery_with_window_func_errors() {
        let sql = "SELECT col FROM t WHERE col > \
                   (SELECT MAX(ROW_NUMBER() OVER (PARTITION BY col ORDER BY col)) FROM t)";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let infos = check_window_in_scalar_contexts(&select, &ctx);
        assert!(
            infos.iter().any(|i| i.clause == "WHERE"),
            "expected a WindowInScalarContext error in WHERE for a subquery containing \
             a window function, got: {infos:?}"
        );
    }

    /// HAVING contains a scalar subquery whose body includes a window function.
    /// Expected: at least one WindowInScalarContextInfo with clause "HAVING".
    #[test]
    fn having_subquery_with_window_func_errors() {
        let sql = "SELECT col, COUNT(*) FROM t GROUP BY col \
                   HAVING COUNT(*) > (SELECT AVG(RANK() OVER (ORDER BY col)) FROM t)";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let infos = check_window_in_scalar_contexts(&select, &ctx);
        assert!(
            infos.iter().any(|i| i.clause == "HAVING"),
            "expected a WindowInScalarContext error in HAVING for a subquery containing \
             a window function, got: {infos:?}"
        );
    }

    /// Window function in SELECT-list subquery — must NOT produce any error
    /// (regression guard: only WHERE / GROUP BY / HAVING are restricted).
    ///
    /// The outer query intentionally includes a WHERE clause so that the
    /// checker has a non-trivial scalar context to walk.  A buggy
    /// implementation that descended into SELECT-list subqueries and
    /// misattributed the inner window function to the outer WHERE would
    /// emit a spurious error here — this test catches that regression.
    #[test]
    fn select_list_subquery_with_window_func_allowed() {
        let sql = "SELECT (SELECT ROW_NUMBER() OVER (ORDER BY col) FROM inner_t) AS rn, col \
             FROM outer_t \
             WHERE col > 0";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let infos = check_window_in_scalar_contexts(&select, &ctx);
        assert!(
            infos.is_empty(),
            "window function inside a SELECT-list subquery must not be flagged \
             even when the outer query has a WHERE clause, got: {infos:?}"
        );
    }

    /// Window function in FROM-clause derived-table — must NOT produce any error
    /// (regression guard: FROM subqueries are not scalar contexts).
    #[test]
    fn from_clause_subquery_with_window_func_allowed() {
        let sql = "SELECT * FROM (SELECT ROW_NUMBER() OVER (ORDER BY col) AS rn FROM t) sub";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let infos = check_window_in_scalar_contexts(&select, &ctx);
        assert!(
            infos.is_empty(),
            "window function inside a FROM-clause subquery must not be flagged, \
             got: {infos:?}"
        );
    }
}
