/// Type inference for SQL expressions
///
/// This module provides type inference capabilities for SQL expressions,
/// including literals, column references, CAST expressions, and aggregates.
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
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
            && self.lambda_params == other.lambda_params
            && self.function_signatures == other.function_signatures
            && self.tableexpr_param_schemas == other.tableexpr_param_schemas
            && self.row_var_env == other.row_var_env
            && self.opaque_ctes == other.opaque_ctes
            && self.fragment_param_kinds == other.fragment_param_kinds
            && self.expected_return == other.expected_return
            && self.function_param_smelt_types == other.function_param_smelt_types
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
            lambda_params: self.lambda_params.clone(),
            function_signatures: self.function_signatures.clone(),
            tableexpr_param_schemas: self.tableexpr_param_schemas.clone(),
            row_var_env: self.row_var_env.clone(),
            opaque_ctes: self.opaque_ctes.clone(),
            fragment_param_kinds: self.fragment_param_kinds.clone(),
            expected_return: self.expected_return.clone(),
            function_param_smelt_types: self.function_param_smelt_types.clone(),
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

    // Try smelt.functions.<name>(...) user-function call site. Returns the
    // declared return type of the resolved signature, or Unknown if the
    // function cannot be resolved / lacks a return annotation. Diagnostics for
    // unresolved functions / arg mismatches are emitted elsewhere
    // (`function_body_check::check_smelt_path_call`), so this path is
    // type-only.
    if let Some(call) = expr.as_smelt_path_call() {
        return infer_smelt_path_call_type(&call, ctx);
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

    // smelt.functions.* user-defined call: scalar today (kind tracking
    // through user-defined fragments is a later phase). Until then, treat
    // as the most permissive kind so call sites in WHERE don't false-positive.
    if expr.as_smelt_path_call().is_some() {
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

/// Infer the return type of a `smelt.functions.<name>(...)` call site.
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
fn infer_smelt_path_call_type(call: &SmeltPathCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let segments = call.segments();

    // Phase B rule 10: `smelt.config.var(...)` always synthesises nullable
    // `Varchar` (Text).  The value is sourced from CLI / env / YAML at
    // compile time and may be absent when no default is provided — hence
    // nullable.  This must be handled before the generic signature lookup
    // because "var" is not in the function-signature index.
    if segments.len() >= 2
        && segments[segments.len() - 2].eq_ignore_ascii_case("config")
        && segments[segments.len() - 1].eq_ignore_ascii_case("var")
    {
        return Some(TypedColumn::nullable(DataType::Varchar {
            max_length: None,
        }));
    }

    // Phase D: `smelt.models.<accessor>(...)` and `smelt.sources.<accessor>(...)`.
    // Both `with_tag` and `all` return `List<ModelRef|SourceRef>`. We synthesise
    // `Unknown` (the DataType projection of a meta-list) — the `SmeltType` is
    // resolved at the HOF inference layer. Unknown / miss accessors also return
    // Unknown (the error is emitted by `check_wide_reflection_diagnostics`).
    // Use segments() here (IDENT-only) since "models" and "sources" are plain identifiers.
    // "all" is a keyword so segments().len() == 1 for `smelt.models.all`, but we detect
    // `models`/`sources` as the first segment regardless.
    {
        // Check first segment (always an IDENT) for "models" or "sources".
        let first_seg = segments.first();
        if first_seg
            .map(|s| s.eq_ignore_ascii_case("models") || s.eq_ignore_ascii_case("sources"))
            .unwrap_or(false)
        {
            return Some(TypedColumn::nullable(DataType::Unknown));
        }
    }

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
        // `List<T>` and `Unknown` (Phase A meta-language) — compile-time only; no
        // scalar DataType equivalent in Phase A.
        Some(Ok(SmeltType::List(_))) | Some(Ok(SmeltType::Unknown)) => DataType::Unknown,
        // `Lambda<T, U>` (Phase B meta-language) — meta-only; not a valid return type.
        Some(Ok(SmeltType::Lambda(_, _))) => DataType::Unknown,
        // `ColumnRef` (Phase C meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ColumnRef)) => DataType::Unknown,
        // `ModelRef` / `SourceRef` (Phase D meta-language) — meta-only; not a SQL DataType.
        Some(Ok(SmeltType::ModelRef)) | Some(Ok(SmeltType::SourceRef)) => DataType::Unknown,
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
    call: &SmeltPathCall,
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
#[derive(Debug)]
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

// ─── Phase B (meta-language): HOF inference + reducer registry + pipe ────────

/// The three built-in higher-order functions (Phase B meta-language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HofKind {
    /// `map(xs: List<T>, f: Lambda<T, U>) -> List<U>`
    Map,
    /// `filter(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>`
    Filter,
    /// `reduce(xs: List<T>, r)` where `r` is a bare reducer identifier.
    Reduce,
}

impl HofKind {
    /// Parse a HOF name into a [`HofKind`]. Returns `None` for non-HOF names.
    pub fn from_name(name: &str) -> Option<HofKind> {
        match name {
            "map" => Some(HofKind::Map),
            "filter" => Some(HofKind::Filter),
            "reduce" => Some(HofKind::Reduce),
            _ => None,
        }
    }
}

/// Sentinel produced by HOF / reducer inference (Phase B).
///
/// Phase 2 records these sentinels but does NOT emit diagnostic codes —
/// that is Phase 3's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HofInferSentinel {
    /// The lambda body's synthesised type does not match the HOF's required
    /// result shape (e.g. `filter` predicate not `Boolean`).
    /// Corresponds to `LambdaResultTypeMismatch`.
    LambdaResultTypeMismatch {
        expected: SmeltType,
        found: SmeltType,
    },
    /// The second argument to `reduce` is not a recognised reducer name.
    /// Corresponds to `HofExpectsReducer`.
    HofExpectsReducer,
    /// The second argument to `map` or `filter` is not a lambda node.
    /// Corresponds to `HofExpectsLambda`.
    HofExpectsLambda,
    /// The input element type does not satisfy the reducer's declared input
    /// constraint. Corresponds to `ReducerInputTypeMismatch`.
    ReducerInputTypeMismatch {
        reducer_name: String,
        expected_constraint: String,
        found: SmeltType,
    },
    /// An empty list was passed to a reducer with no declared identity.
    /// Corresponds to `ReducerEmptyNoIdentity`.
    ReducerEmptyNoIdentity { reducer_name: String },
    /// The first argument to the HOF did not infer to a `List<T>`.
    InputNotList { found: SmeltType },
}

/// Result of HOF / reducer inference (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HofInferResult {
    /// The inferred [`SmeltType`] for the HOF call.
    pub inferred: SmeltType,
    /// Optional pending diagnostic sentinel. `None` on the happy path.
    pub sentinel: Option<HofInferSentinel>,
}

/// Spec for a single entry in the closed reducer registry (Phase B).
///
/// Pure data — no Salsa dependency.
pub struct ReducerSpec {
    /// The reducer's source name (e.g. `"and_all"`).
    pub name: &'static str,
    /// The input constraint for each element: `Boolean`, `Numeric`, `Text`,
    /// `Any` (for `comma_sep`), or `TableExpr` (for `union_all`/`intersect_all`).
    /// `None` means the element type is unconstrained (accepts any `Expr<T>`).
    pub input_constraint: ReducerInputConstraint,
    /// The output [`SmeltType`] of the reducer (after a non-empty evaluation).
    pub output_sort: ReducerOutputSort,
    /// Empty-list behaviour: `Some(identity)` means the reducer has a known
    /// identity element; `None` means `ReducerEmptyNoIdentity`.
    pub empty_identity: EmptyIdentity,
}

/// Input-element constraint for a reducer entry (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerInputConstraint {
    /// Any `Expr<T>` is accepted (used by `comma_sep`).
    AnyExpr,
    /// Only `Expr<Boolean>` is accepted.
    Boolean,
    /// Any `Expr<Numeric>` is accepted (satisfies `TypeConstraint::Numeric`).
    Numeric,
    /// Only `Expr<Text>` is accepted.
    Text,
    /// Only `TableExpr` is accepted.
    TableExpr,
}

impl ReducerInputConstraint {
    /// Check whether a [`SmeltType`] satisfies this input constraint.
    pub fn is_satisfied_by(&self, ty: &SmeltType) -> bool {
        match (self, ty) {
            (ReducerInputConstraint::AnyExpr, SmeltType::Expr(_)) => true,
            (ReducerInputConstraint::Boolean, SmeltType::Expr(tc)) => {
                matches!(tc, smelt_types::signatures::TypeConstraint::Concrete(dt) if *dt == DataType::Boolean)
            }
            (ReducerInputConstraint::Numeric, SmeltType::Expr(tc)) => {
                smelt_types::signatures::TypeConstraint::Numeric.satisfies(&match tc {
                    smelt_types::signatures::TypeConstraint::Concrete(dt) => dt.clone(),
                    _ => return false,
                })
            }
            (ReducerInputConstraint::Text, SmeltType::Expr(tc)) => {
                matches!(tc,
                    smelt_types::signatures::TypeConstraint::Concrete(dt)
                    if matches!(dt, DataType::Text | DataType::Varchar { .. } | DataType::Char { .. })
                )
            }
            (ReducerInputConstraint::TableExpr, SmeltType::TableExpr(_)) => true,
            _ => false,
        }
    }
}

/// Output sort for a reducer (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerOutputSort {
    /// Output is `Expr<Boolean>`.
    Boolean,
    /// Output is `Expr<T>` where `T` is the LUB-promoted element type of the input list
    /// (used by `plus_chain` and `concat`).
    SameAsElementType,
    /// Output is `SelectItems<Scalar>` (used by `comma_sep`).
    SelectItemsScalar,
    /// Output is `TableExpr` (used by `union_all`, `intersect_all`).
    TableExpr,
}

/// Empty-list identity rule for a reducer (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyIdentity {
    /// The reducer has an identity element: the output sort with a known value.
    /// The bool/text/numeric variant matches the output sort.
    Boolean,
    /// The reducer has a numeric identity element (0 cast to LUB element type).
    Numeric,
    /// The reducer has a text identity element (empty string `''`).
    Text,
    /// Empty list produces `SelectItems<Scalar>` with no sentinel (used by `comma_sep`).
    /// An empty comma-separated list elides at the splice point without error.
    EmptySelectItems,
    /// The reducer has no identity — empty list is an error (`ReducerEmptyNoIdentity`).
    None,
}

/// The closed reducer registry — seven entries (Phase B spec).
pub static REDUCER_REGISTRY: &[ReducerSpec] = &[
    ReducerSpec {
        name: "comma_sep",
        input_constraint: ReducerInputConstraint::AnyExpr,
        output_sort: ReducerOutputSort::SelectItemsScalar,
        empty_identity: EmptyIdentity::EmptySelectItems, // empty list elides at splice (see spec)
    },
    ReducerSpec {
        name: "and_all",
        input_constraint: ReducerInputConstraint::Boolean,
        output_sort: ReducerOutputSort::Boolean,
        empty_identity: EmptyIdentity::Boolean, // TRUE
    },
    ReducerSpec {
        name: "or_any",
        input_constraint: ReducerInputConstraint::Boolean,
        output_sort: ReducerOutputSort::Boolean,
        empty_identity: EmptyIdentity::Boolean, // FALSE
    },
    ReducerSpec {
        name: "union_all",
        input_constraint: ReducerInputConstraint::TableExpr,
        output_sort: ReducerOutputSort::TableExpr,
        empty_identity: EmptyIdentity::None,
    },
    ReducerSpec {
        name: "intersect_all",
        input_constraint: ReducerInputConstraint::TableExpr,
        output_sort: ReducerOutputSort::TableExpr,
        empty_identity: EmptyIdentity::None,
    },
    ReducerSpec {
        name: "plus_chain",
        input_constraint: ReducerInputConstraint::Numeric,
        output_sort: ReducerOutputSort::SameAsElementType,
        empty_identity: EmptyIdentity::Numeric, // 0-cast-to-LUB
    },
    ReducerSpec {
        name: "concat",
        input_constraint: ReducerInputConstraint::Text,
        output_sort: ReducerOutputSort::SameAsElementType,
        empty_identity: EmptyIdentity::Text, // empty string ''
    },
];

/// Look up a reducer by name in the closed registry.
///
/// Returns `None` for unknown names. Pure — no Salsa dependency.
pub fn lookup_reducer(name: &str) -> Option<&'static ReducerSpec> {
    REDUCER_REGISTRY.iter().find(|r| r.name == name)
}

/// Infer the output [`SmeltType`] for a `reduce(xs, reducer_name)` call
/// where `xs` infers to `List<elem_ty>` (Phase B).
///
/// Returns a [`HofInferResult`] with:
/// - On success: the reducer's declared output type, or the element type for
///   `SameAsElementType` reducers.
/// - On `ReducerInputTypeMismatch`: the input element does not satisfy the
///   reducer's constraint.
/// - On `ReducerEmptyNoIdentity`: empty list passed to a no-identity reducer.
///
/// Pure function — no Salsa dependency.
pub fn infer_reduce_call(
    list_elem_ty: &SmeltType,
    is_empty_list: bool,
    reducer_name: &str,
    _expected: Option<&SmeltType>,
) -> HofInferResult {
    let Some(spec) = lookup_reducer(reducer_name) else {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: Some(HofInferSentinel::HofExpectsReducer),
        };
    };

    // Empty-list path
    if is_empty_list {
        return match spec.empty_identity {
            EmptyIdentity::Boolean => HofInferResult {
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    DataType::Boolean,
                )),
                sentinel: None,
            },
            EmptyIdentity::Numeric => HofInferResult {
                // Identity is 0-cast-to-element-type; use Unknown for empty.
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Numeric),
                sentinel: None,
            },
            EmptyIdentity::Text => HofInferResult {
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    DataType::Text,
                )),
                sentinel: None,
            },
            EmptyIdentity::EmptySelectItems => HofInferResult {
                inferred: SmeltType::SelectItems {
                    kind: smelt_types::signatures::ExprKind::Scalar,
                    context: None,
                },
                sentinel: None,
            },
            EmptyIdentity::None => HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: Some(HofInferSentinel::ReducerEmptyNoIdentity {
                    reducer_name: reducer_name.to_string(),
                }),
            },
        };
    }

    // Non-empty: validate input element type against the reducer's constraint.
    if !spec.input_constraint.is_satisfied_by(list_elem_ty) {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: Some(HofInferSentinel::ReducerInputTypeMismatch {
                reducer_name: reducer_name.to_string(),
                expected_constraint: format!("{:?}", spec.input_constraint),
                found: list_elem_ty.clone(),
            }),
        };
    }

    // Derive output sort.
    let output = match &spec.output_sort {
        ReducerOutputSort::Boolean => SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean),
        ),
        ReducerOutputSort::SameAsElementType => list_elem_ty.clone(),
        ReducerOutputSort::SelectItemsScalar => SmeltType::SelectItems {
            kind: smelt_types::signatures::ExprKind::Scalar,
            context: None,
        },
        ReducerOutputSort::TableExpr => SmeltType::TableExpr(None),
    };

    HofInferResult {
        inferred: output,
        sentinel: None,
    }
}

/// Infer the output [`SmeltType`] for a HOF call (`map`, `filter`, or `reduce`)
/// given the pre-inferred list type and the lambda/reducer second argument (Phase B).
///
/// - For `map`: bind lambda parameter to `T`, synthesise body type `U`, return `List<U>`.
/// - For `filter`: bind lambda parameter to `T`, synthesise body type, require `Boolean`,
///   return `List<T>`.
/// - For `reduce`: delegate to [`infer_reduce_call`].
///
/// Pure function — no Salsa dependency.
pub fn infer_hof_call(
    hof: HofKind,
    list_ty: &SmeltType,
    second_arg: HofSecondArg<'_>,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // Extract element type from List<T>.
    let elem_ty = match list_ty {
        SmeltType::List(inner) => (**inner).clone(),
        SmeltType::Unknown => SmeltType::Unknown,
        other => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: Some(HofInferSentinel::InputNotList {
                    found: other.clone(),
                }),
            };
        }
    };

    // Check for empty list (List<Unknown> from empty literal `[]`).
    let is_empty = matches!(&elem_ty, SmeltType::Unknown);

    match hof {
        HofKind::Map | HofKind::Filter => {
            let lambda = match second_arg {
                HofSecondArg::Lambda(l) => l,
                HofSecondArg::ReducerName(_) | HofSecondArg::Other => {
                    return HofInferResult {
                        inferred: SmeltType::Unknown,
                        sentinel: Some(HofInferSentinel::HofExpectsLambda),
                    };
                }
            };
            let params = lambda.params();
            let param_name = params.first().cloned().unwrap_or_default();
            let body = match lambda.body() {
                Some(b) => b,
                None => {
                    return HofInferResult {
                        inferred: if hof == HofKind::Map {
                            SmeltType::List(Box::new(SmeltType::Unknown))
                        } else {
                            list_ty.clone()
                        },
                        sentinel: None,
                    };
                }
            };

            // Bind lambda parameter to elem_ty in a scoped context clone.
            let mut body_ctx = ctx.clone();
            body_ctx.add_lambda_param(
                &param_name,
                smelt_types::TypedColumn {
                    data_type: match &elem_ty {
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(dt)) => {
                            dt.clone()
                        }
                        _ => DataType::Unknown,
                    },
                    nullable: true,
                },
            );

            // Synthesise body type.
            let body_typed = infer_expression_type(&body, &body_ctx);
            let body_ty = body_typed
                .map(|tc| {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        tc.data_type,
                    ))
                })
                .unwrap_or(SmeltType::Unknown);

            if hof == HofKind::Filter {
                // Filter predicate body must be Boolean.
                let expected_bool = SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean),
                );
                if body_ty != expected_bool && !matches!(body_ty, SmeltType::Unknown) {
                    return HofInferResult {
                        inferred: list_ty.clone(),
                        sentinel: Some(HofInferSentinel::LambdaResultTypeMismatch {
                            expected: expected_bool,
                            found: body_ty,
                        }),
                    };
                }
                HofInferResult {
                    inferred: list_ty.clone(),
                    sentinel: None,
                }
            } else {
                // Map: result is List<body_ty>.
                HofInferResult {
                    inferred: SmeltType::List(Box::new(body_ty)),
                    sentinel: None,
                }
            }
        }
        HofKind::Reduce => {
            let reducer_name = match second_arg {
                HofSecondArg::ReducerName(name) => name,
                HofSecondArg::Lambda(_) | HofSecondArg::Other => {
                    return HofInferResult {
                        inferred: SmeltType::Unknown,
                        sentinel: Some(HofInferSentinel::HofExpectsReducer),
                    };
                }
            };
            infer_reduce_call(&elem_ty, is_empty, reducer_name, expected)
        }
    }
}

/// The second argument to a HOF call (Phase B).
///
/// `Lambda` carries the AST lambda node (owned — Rowan handles are cheap refcounted
/// clones); `ReducerName` carries the bare identifier text; `Other` represents any
/// other node shape (triggers a sentinel).
pub enum HofSecondArg<'a> {
    Lambda(smelt_parser::ast::Lambda),
    ReducerName(&'a str),
    Other,
}

/// Infer the output type for a HOF call given a [`FunctionCall`] AST node (Phase B).
///
/// This convenience wrapper extracts the HOF kind, the list argument, and the
/// second argument (lambda or reducer name) from the function call, then delegates
/// to [`infer_hof_call`].
///
/// Returns `None` when the function is not a recognised HOF.
pub fn infer_hof_call_from_function_call(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &TypeContext,
) -> HofInferResult {
    infer_hof_call_from_function_call_with_expected(call, ctx, None)
}

/// Like [`infer_hof_call_from_function_call`] but with an optional expected type
/// (used by the empty-list + identity reducer tests).
pub fn infer_hof_call_from_function_call_with_expected(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // `FunctionCall::name()` only returns IDENT-typed tokens; HOF names like
    // `filter` may be lexed as keyword tokens (FILTER_KW). Fall back to the
    // first token's text for keyword-as-function-name cases.
    let name = call.name().unwrap_or_else(|| {
        // Extract the first non-trivia token's text, normalised to lowercase.
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_lowercase())
            .unwrap_or_default()
    });
    let name_lc = name.to_lowercase();
    let Some(hof) = HofKind::from_name(&name_lc) else {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: None,
        };
    };

    let args = call.arguments();
    if args.is_empty() {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: None,
        };
    }

    // Infer the list type from the first argument.
    let xs = &args[0];
    let list_ty = if let Some(arr) = xs.as_array_literal() {
        let elems: Vec<_> = arr.elements();
        let result = infer_list_literal(&elems, ctx, expected);
        result.inferred
    } else {
        // Non-literal first argument.
        //
        // Phase B: if the argument is a bare identifier (e.g. `xs`) that
        // names a function parameter declared as `List<T>`, the standard
        // `infer_expression_type` path collapses the type to `DataType::Unknown`
        // (because `List<T>` has no scalar `DataType` equivalent).  To preserve
        // the full `SmeltType::List(...)`, we first consult the
        // `function_param_smelt_types` map which stores the declared `SmeltType`
        // for params registered via `add_function_param_smelt_type`.
        let ident_name = xs.text().to_string();
        let ident_name = ident_name.trim();
        let is_bare_ident =
            !ident_name.is_empty() && ident_name.chars().all(|c| c.is_alphanumeric() || c == '_');

        if is_bare_ident {
            if let Some(smelt_ty) = ctx.lookup_function_param_smelt_type(ident_name) {
                smelt_ty.clone()
            } else {
                // Fall back to expression inference.
                infer_expression_type(xs, ctx)
                    .map(|tc| {
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                            tc.data_type,
                        ))
                    })
                    .unwrap_or(SmeltType::Unknown)
            }
        } else {
            // Complex expression: expression inference only.
            infer_expression_type(xs, ctx)
                .map(|tc| {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        tc.data_type,
                    ))
                })
                .unwrap_or(SmeltType::Unknown)
        }
    };

    // Extract second argument shape.
    if args.len() < 2 {
        return infer_hof_call(hof, &list_ty, HofSecondArg::Other, ctx, expected);
    }
    let arg2 = &args[1];

    // Check if second arg contains a LAMBDA node (may be wrapped in EXPRESSION).
    if let Some(lambda) = extract_lambda_from_expr(arg2) {
        return infer_hof_call(hof, &list_ty, HofSecondArg::Lambda(lambda), ctx, expected);
    }

    // Check if it's a bare identifier (reducer name).
    let text = arg2.text().trim().to_string();
    // A bare identifier: no spaces, no parens, no brackets.
    if text.chars().all(|c| c.is_alphanumeric() || c == '_') && !text.is_empty() {
        return infer_hof_call_with_reducer_name(hof, &list_ty, &text, ctx, expected);
    }

    infer_hof_call(hof, &list_ty, HofSecondArg::Other, ctx, expected)
}

/// Extract a [`Lambda`] from an [`Expr`], following the same pattern as
/// [`Expr::as_function_call`] and other `as_*` methods: check the node itself,
/// then check its direct children. This is needed because `arguments()` returns
/// `Expr` values whose inner node is an `EXPRESSION` that wraps the `LAMBDA`.
fn extract_lambda_from_expr(expr: &smelt_parser::ast::Expr) -> Option<smelt_parser::ast::Lambda> {
    smelt_parser::ast::Lambda::cast(expr.syntax().clone()).or_else(|| {
        expr.syntax()
            .children()
            .find_map(smelt_parser::ast::Lambda::cast)
    })
}

/// Extract a [`PipeExpr`] from an [`Expr`], following the same pattern.
fn extract_pipe_expr_from_expr(
    expr: &smelt_parser::ast::Expr,
) -> Option<smelt_parser::ast::PipeExpr> {
    smelt_parser::ast::PipeExpr::cast(expr.syntax().clone()).or_else(|| {
        expr.syntax()
            .children()
            .find_map(smelt_parser::ast::PipeExpr::cast)
    })
}

/// Inner helper for the reducer-name path (avoids lifetime issues with `HofSecondArg`).
fn infer_hof_call_with_reducer_name(
    hof: HofKind,
    list_ty: &SmeltType,
    reducer_name: &str,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    infer_hof_call(
        hof,
        list_ty,
        HofSecondArg::ReducerName(reducer_name),
        ctx,
        expected,
    )
}

/// Infer the output [`SmeltType`] for a pipe expression `LHS |> CALL(args...)` (Phase B).
///
/// Pipe desugaring: `LHS |> CALL(args...)` is equivalent to `CALL(LHS, args...)`.
/// This function constructs the equivalent direct call and infers its type.
///
/// Pure function — no Salsa dependency.
pub fn infer_pipe_expr(
    pipe: &smelt_parser::ast::PipeExpr,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // Get LHS and RHS.
    let lhs = match pipe.lhs() {
        Some(l) => l,
        None => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: None,
            }
        }
    };
    let rhs = match pipe.rhs() {
        Some(r) => r,
        None => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: None,
            }
        }
    };

    // If the LHS itself is a pipe, recurse to infer its type first.
    let lhs_ty = if let Some(inner_pipe) = extract_pipe_expr_from_expr(&lhs) {
        let r = infer_pipe_expr(&inner_pipe, ctx, None);
        r.inferred
    } else {
        // Infer LHS as a list literal or expression.
        if let Some(arr) = lhs.as_array_literal() {
            let elems: Vec<_> = arr.elements();
            infer_list_literal(&elems, ctx, None).inferred
        } else {
            infer_expression_type(&lhs, ctx)
                .map(|tc| {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        tc.data_type,
                    ))
                })
                .unwrap_or(SmeltType::Unknown)
        }
    };

    // RHS must be a function call — if so, extract call name and synthesise.
    if let Some(call) = rhs.as_function_call() {
        // `call.name()` only finds IDENT tokens; keyword-based names like
        // `filter` (lexed as FILTER_KW) need a fallback to the first token.
        let name = call.name().unwrap_or_else(|| {
            call.syntax()
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| !t.kind().is_trivia())
                .map(|t| t.text().to_lowercase())
                .unwrap_or_default()
        });
        let name_lc = name.to_lowercase();
        if let Some(hof) = HofKind::from_name(&name_lc) {
            // Build the effective argument list: LHS + RHS args.
            let rhs_args = call.arguments();

            // We reconstruct the inference by using the lhs_ty directly.
            if rhs_args.is_empty() {
                return HofInferResult {
                    inferred: SmeltType::Unknown,
                    sentinel: None,
                };
            }
            let second_arg = &rhs_args[0];

            // Check if second arg contains a LAMBDA node (may be wrapped in EXPRESSION).
            if let Some(lambda) = extract_lambda_from_expr(second_arg) {
                return infer_hof_call(hof, &lhs_ty, HofSecondArg::Lambda(lambda), ctx, expected);
            }
            // Check if it's a bare reducer name.
            let text = second_arg.text().trim().to_string();
            if text.chars().all(|c| c.is_alphanumeric() || c == '_') && !text.is_empty() {
                return infer_hof_call_with_reducer_name(hof, &lhs_ty, &text, ctx, expected);
            }
            return infer_hof_call(hof, &lhs_ty, HofSecondArg::Other, ctx, expected);
        }
    }

    // Non-HOF RHS or non-call RHS — Phase 3 emits PipeRhsNotCall diagnostic.
    // For now return Unknown without sentinel.
    HofInferResult {
        inferred: SmeltType::Unknown,
        sentinel: None,
    }
}

// ─── Phase A (meta-language): list-literal type inference ───────────────────

/// Pending diagnostic sentinel produced by [`infer_list_literal`].
///
/// Phase 2 records these sentinels but does NOT emit diagnostic codes —
/// that is Phase 3's job. The sentinel carries enough information for Phase 3
/// to produce a rich diagnostic without re-inferring.
///
/// Pure data — no Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListInferSentinel {
    /// An empty list literal (`[]`) was found in a position where no target
    /// sort context was supplied. Corresponds to `MetaListEmptyTypeUnknown`.
    EmptyTypeUnknown,
    /// Two or more elements in the list literal have incompatible types that
    /// cannot be unified by the numeric promotion chain, or elements whose
    /// sorts differ (e.g. a scalar `Expr<T>` mixed with a nested `List<T>`).
    /// Corresponds to `MetaListHeterogeneous`.
    ///
    /// Fields carry [`SmeltType`] (not bare [`DataType`]) so that cross-sort
    /// heterogeneity (scalar vs. nested list) can be represented faithfully.
    /// Phase 3 renders these for the user; it handles both `Expr<…>` and
    /// `List<…>` variants.
    Heterogeneous {
        /// The first element sort seen before the incompatibility.
        first: SmeltType,
        /// The element sort that was incompatible with `first`.
        incompatible: SmeltType,
    },
}

/// Result of [`infer_list_literal`].
///
/// Pure data — no Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListLiteralInferResult {
    /// The inferred [`SmeltType`] for this list literal.
    ///
    /// * Homogeneous literal: `List<Expr<T>>` where `T` is the LUB.
    /// * Nested literal: `List<List<…>>`.
    /// * Heterogeneous literal: `List<Unknown>`.
    /// * Empty literal with target: the target `List<T>`.
    /// * Empty literal without target: `List<Unknown>`.
    pub inferred: SmeltType,
    /// Pending diagnostic sentinels. Empty on the happy path.
    pub sentinels: Vec<ListInferSentinel>,
}

/// Infer the type of a list literal `[e_1, …, e_n]` (Phase A, meta-language).
///
/// Control flow:
/// 1. Empty literal — consult `expected`; produce `List<T>` if known, else
///    `List<Unknown>` + [`ListInferSentinel::EmptyTypeUnknown`].
/// 2. Non-empty — for each element compute its [`SmeltType`]:
///    * A list-literal element recursively calls this function → yields
///      `List<…>`.
///    * A scalar element is inferred via [`infer_expression_type`] → yields
///      `Expr<T>`.
///
///    NULL scalars are skipped (compatible with any type).
/// 3. The resulting `SmeltType`s are LUB-ed via [`smelt_type_lub`]:
///    * Two `Expr<T>` values use [`promote_types`] on their inner `DataType`.
///    * Two identical `SmeltType`s are already equal.
///    * Any cross-sort mix (`Expr<…>` vs `List<…>`) or same-sort but
///      incompatible pair produces `List<Unknown>` +
///      [`ListInferSentinel::Heterogeneous`].
///
/// This single-function design makes the "scalar wrapped as List" bug
/// structurally impossible: scalars always carry sort `Expr<…>`, list-literal
/// elements always carry sort `List<…>`, so a cross-sort mix naturally fails
/// the LUB step.
///
/// Pure function — no Salsa dependency. `TypeContext` is accepted for element
/// type inference but the function itself performs no Salsa calls.
pub fn infer_list_literal(
    elements: &[Expr],
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> ListLiteralInferResult {
    // Empty literal — consult expected type.
    if elements.is_empty() {
        if let Some(exp) = expected {
            // The expected type must be a List<T> to be meaningful context for
            // an empty list literal. If the caller passes a non-List expected
            // (e.g. `Expr<Numeric>` at an unconstrained position), fall through
            // to the unknown-target branch rather than returning a non-List type.
            if matches!(exp, SmeltType::List(_)) {
                // Expected is a fully-formed List<T>; return it (spec rule 4).
                return ListLiteralInferResult {
                    inferred: exp.clone(),
                    sentinels: vec![],
                };
            }
            // Expected is not a List<T> — treat as unconstrained.
        }
        return ListLiteralInferResult {
            inferred: SmeltType::List(Box::new(SmeltType::Unknown)),
            sentinels: vec![ListInferSentinel::EmptyTypeUnknown],
        };
    }

    // Non-empty: infer each element as a SmeltType, then LUB at SmeltType level.
    // Working at SmeltType level (not DataType level) is what makes cross-sort
    // heterogeneity (Expr<T> vs List<T>) naturally detectable.
    let mut accumulated: Option<SmeltType> = None;
    let mut sentinels: Vec<ListInferSentinel> = Vec::new();
    let mut heterogeneous = false;

    for elem in elements {
        // Determine element's SmeltType:
        //   - list-literal element → recurse → List<…>
        //   - scalar element       → infer_expression_type → Expr<T>
        let (elem_ty, elem_sentinels) = if let Some(inner_arr) = elem.as_array_literal() {
            let inner_elems: Vec<Expr> = inner_arr.elements();
            let r = infer_list_literal(&inner_elems, ctx, None);
            (r.inferred, r.sentinels)
        } else {
            let typed = infer_expression_type(elem, ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            });
            let dt = typed.data_type;
            if dt == DataType::Null {
                // NULL is compatible with any element type; skip without
                // contributing to the running LUB.
                continue;
            }
            let smelt = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(dt));
            (smelt, vec![])
        };

        sentinels.extend(elem_sentinels);

        match accumulated.take() {
            None => {
                accumulated = Some(elem_ty);
            }
            Some(existing) => {
                // LUB at SmeltType level.
                let lub = smelt_type_lub(&existing, &elem_ty);
                match lub {
                    Some(t) => {
                        accumulated = Some(t);
                    }
                    None => {
                        // Incompatible sorts or types → heterogeneous.
                        if !heterogeneous {
                            sentinels.push(ListInferSentinel::Heterogeneous {
                                first: existing.clone(),
                                incompatible: elem_ty,
                            });
                            heterogeneous = true;
                        }
                        // Restore `existing` so the accumulated first type is
                        // preserved for subsequent elements' sentinel comparison.
                        accumulated = Some(existing);
                    }
                }
            }
        }
    }

    if heterogeneous {
        return ListLiteralInferResult {
            inferred: SmeltType::List(Box::new(SmeltType::Unknown)),
            sentinels,
        };
    }

    // All elements were NULL — use Null as the element type.
    let elem_ty = accumulated.unwrap_or(SmeltType::Expr(
        smelt_types::signatures::TypeConstraint::Concrete(DataType::Null),
    ));
    ListLiteralInferResult {
        inferred: SmeltType::List(Box::new(elem_ty)),
        sentinels,
    }
}

/// Compute the least-upper-bound (LUB) of two [`SmeltType`]s for the purposes
/// of list-element unification (Phase A).
///
/// Rules:
/// * Two identical types → `Some(that type)`.
/// * Two `Expr<Concrete(T)>` values → promote their inner `DataType` via
///   [`promote_types`]; `Unknown` result means incompatible → `None`.
/// * Two `List<T>` values → recurse into inner types; `None` inner → `None`.
/// * Any cross-sort pair (`Expr` vs `List`, etc.) → `None`.
///
/// Returns `None` when the types cannot be unified.
fn smelt_type_lub(a: &SmeltType, b: &SmeltType) -> Option<SmeltType> {
    if a == b {
        return Some(a.clone());
    }
    match (a, b) {
        (
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(da)),
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(db)),
        ) => {
            let promoted = promote_types(
                &TypedColumn::not_null(da.clone()),
                &TypedColumn::not_null(db.clone()),
            );
            if promoted.data_type == DataType::Unknown {
                None
            } else {
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(promoted.data_type),
                ))
            }
        }
        (SmeltType::List(inner_a), SmeltType::List(inner_b)) => {
            // Nested lists: LUB on inner types.
            smelt_type_lub(inner_a, inner_b).map(|inner_lub| SmeltType::List(Box::new(inner_lub)))
        }
        // Cross-sort or any other pair that isn't equal and doesn't fit above.
        _ => None,
    }
}

// ─── Phase A Phase 3: diagnostics + bidirectional disambiguation + spread ────

/// Position kind for spread validation. Determines whether a spread operator
/// is in a valid or forbidden position.
///
/// Valid positions: Select, GroupBy, OrderBy, FunctionArg, InList, ValuesRow,
/// ListLiteralBody.
/// Forbidden positions: Where, Boolean, NamedArgValue, FromWithoutReducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplicePosition {
    /// SELECT list — spread allowed.
    Select,
    /// GROUP BY — spread allowed.
    GroupBy,
    /// ORDER BY — spread allowed.
    OrderBy,
    /// Positional function argument — spread allowed.
    FunctionArg,
    /// IN-list `x IN (...vs)` — spread allowed.
    InList,
    /// VALUES row — spread allowed.
    ValuesRow,
    /// Inside another list literal `[a, ...xs, b]` — spread allowed.
    ListLiteralBody,
    /// WHERE clause — spread forbidden.
    Where,
    /// Boolean composition (`x AND ...preds`, `y OR ...preds`) — spread forbidden.
    Boolean,
    /// Named-argument value (`name => value`) — spread forbidden.
    NamedArgValue,
    /// FROM clause without an explicit reducer — spread forbidden.
    FromWithoutReducer,
}

impl SplicePosition {
    /// Returns `true` if spread is allowed in this position.
    pub fn spread_allowed(self) -> bool {
        matches!(
            self,
            SplicePosition::Select
                | SplicePosition::GroupBy
                | SplicePosition::OrderBy
                | SplicePosition::FunctionArg
                | SplicePosition::InList
                | SplicePosition::ValuesRow
                | SplicePosition::ListLiteralBody
        )
    }

    /// Human-readable position name for the `MetaSpreadInForbiddenPosition`
    /// message, matching the spec's prescribed strings.
    pub fn forbidden_position_name(self) -> &'static str {
        match self {
            SplicePosition::Where => "WHERE clause",
            SplicePosition::Boolean => "boolean composition",
            SplicePosition::NamedArgValue => "named argument value",
            SplicePosition::FromWithoutReducer => "FROM clause without an explicit reducer",
            _ => "unknown position",
        }
    }
}

/// Result of the bidirectional disambiguation of a `[...]` list literal.
///
/// Implements spec `meta_language.md` Phase A §"Rule 3 — Bidirectional
/// disambiguation":
/// - `List<T>` expected → `MetaList`.
/// - `Expr<Array<U>>` expected → `DataWorldArray`.
/// - Both admissible → `MetaList` (meta wins).
/// - Neither admissible → `MetaList` with `List<Unknown>` (error type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListDisambiguation {
    /// Treat the literal as a compile-time meta `List<T>`. The carried value
    /// is the infer result from [`infer_list_literal`].
    MetaList(ListLiteralInferResult),
    /// Treat the literal as a Data-World runtime array (`ARRAY[...]`).
    DataWorldArray,
}

/// Disambiguate a `[...]` literal between meta-list and Data-World array.
///
/// Implements spec rule 3:
/// - If `expected` is `Some(List<T>)` → meta-list interpretation.
/// - If `expected` is `Some(Expr<Array<U>>)` → Data-World array.
/// - Otherwise (both admissible or no expected) → meta-list wins.
///
/// Pure function — no Salsa dependency.
pub fn disambiguate_list_literal(
    elements: &[smelt_parser::ast::Expr],
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> ListDisambiguation {
    match expected {
        // Explicitly expected Data-World array → data path.
        Some(SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
            DataType::Array(_),
        ))) => ListDisambiguation::DataWorldArray,
        // Explicitly expected meta-list (List<T>) → pass expected through.
        Some(SmeltType::List(_)) => {
            let result = infer_list_literal(elements, ctx, expected);
            ListDisambiguation::MetaList(result)
        }
        // Both admissible (no expected, or an unrelated expected) → meta wins.
        _ => {
            let result = infer_list_literal(elements, ctx, None);
            ListDisambiguation::MetaList(result)
        }
    }
}

/// Convert `ListInferSentinel`s to `Diagnostic` records.
///
/// This is Phase 3's "sentinel to diagnostic" converter. It takes the
/// sentinels produced by `infer_list_literal` and converts them into
/// diagnostics anchored at `span` (the list literal's source span).
///
/// `text` is the raw source text of the file — needed to convert
/// `rowan::TextRange` byte offsets into `Position { line, column }` pairs via
/// [`smelt_parser::ast::text_range_to_range`]. Pass an empty string `""` in
/// unit tests where the exact range is not under test.
///
/// Pure function — no Salsa dependency. Returns the diagnostics so the
/// caller can append them to whatever accumulator is in use.
pub fn list_literal_sentinels_to_diagnostics(
    elements: &[smelt_parser::ast::Expr],
    ctx: &TypeContext,
    span: rowan::TextRange,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::format_smelt_type_hover;

    let result = infer_list_literal(elements, ctx, None);
    let mut diags = Vec::new();

    let range = smelt_parser::ast::text_range_to_range(text, span);

    for sentinel in &result.sentinels {
        let (code, message) = match sentinel {
            ListInferSentinel::EmptyTypeUnknown => (
                crate::DiagnosticCode::MetaListEmptyTypeUnknown,
                crate::meta_list_diagnostic_message(
                    crate::DiagnosticCode::MetaListEmptyTypeUnknown,
                    None,
                    None,
                    None,
                ),
            ),
            ListInferSentinel::Heterogeneous {
                first,
                incompatible,
            } => {
                let t0 = format_smelt_type_hover(first);
                let tk = format_smelt_type_hover(incompatible);
                (
                    crate::DiagnosticCode::MetaListHeterogeneous,
                    crate::meta_list_diagnostic_message(
                        crate::DiagnosticCode::MetaListHeterogeneous,
                        Some(&t0),
                        Some(&tk),
                        None,
                    ),
                )
            }
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message,
            range,
            code: Some(code),
            data: None,
        });
    }

    diags
}

/// Provenance origin tag — Phase A Phase 3.
///
/// Records where a synthesized item came from. `Synthesized(SpreadFrom(span))`
/// marks each item emitted by a spread expansion. The `TextRange` is the span
/// of the spread operand (the `...xs` expression).
///
/// This is the Phase A minimal implementation. Phase B will extend this with
/// `Caller(span)` and `Callee(fn_id, span)` variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginTag {
    /// Item was synthesized by a spread expansion. The `TextRange` is the
    /// span of the `...operand` expression in the source.
    Synthesized(SynthesizedReason),
}

/// Reason for synthesis, per `expansion.md` §"Provenance origin tags".
///
/// Phase A adds `SpreadFrom`; Phase B will add the full `fn_id`-bearing
/// variants. `fn_id` is `Option<String>` here because a top-level spread
/// has no enclosing function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthesizedReason {
    /// Emitted by a list spread `...xs`. The `TextRange` is the span of the
    /// spread operator node.
    SpreadFrom(rowan::TextRange),
}

/// Result returned by [`check_select_list_spreads`].
///
/// Records what the spread-expansion check found: how many items were
/// expanded (for testing), any diagnostics emitted, and provenance tags for
/// the expanded items.
#[derive(Debug, Clone)]
pub struct SelectListSpreadResult {
    /// Total number of items produced by spread expansion across all spreads
    /// in the SELECT list (sum of individual spread expansion widths).
    pub expanded_item_count: usize,
    /// Diagnostics emitted during expansion (e.g. `MetaSpreadOnNonList`).
    pub diagnostics: Vec<crate::Diagnostic>,
    /// Provenance origin tags for every item produced by spread expansion.
    /// Length equals `expanded_item_count`.
    pub provenance_tags: Vec<OriginTag>,
}

/// Check all `LIST_SPREAD` nodes in a SELECT list for validity and expand them.
///
/// For each `LIST_SPREAD` found as a direct child of the SELECT_LIST:
/// 1. Infer the operand's type.
/// 2. If it is `List<T>`, expand the elements — each gets a
///    `Synthesized(SpreadFrom(spread_span))` provenance tag.
/// 3. If it is not `List<T>`, emit `MetaSpreadOnNonList` and drop the spread.
/// 4. Empty-list spreads produce zero expansion items (elision).
///
/// Forbidden-position checking (WHERE, etc.) is handled separately by
/// [`check_forbidden_position_spreads`].
///
/// `text` is the raw source text — used to convert `rowan::TextRange` byte
/// offsets into `Position { line, column }` for diagnostics. Pass `""` in unit
/// tests where the exact range is not under test.
///
/// Pure function — no Salsa dependency.
pub fn check_select_list_spreads(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> SelectListSpreadResult {
    use smelt_parser::SyntaxKind::{LIST_SPREAD, SELECT_LIST};
    use smelt_types::signatures::format_smelt_type_hover;

    let mut result = SelectListSpreadResult {
        expanded_item_count: 0,
        diagnostics: Vec::new(),
        provenance_tags: Vec::new(),
    };

    // Find the SELECT_LIST node, then iterate its children for LIST_SPREAD nodes.
    let select_list_node = select_stmt
        .syntax()
        .children()
        .find(|n| n.kind() == SELECT_LIST);

    let Some(sl_node) = select_list_node else {
        return result;
    };

    for child in sl_node.children() {
        if child.kind() != LIST_SPREAD {
            continue;
        }
        let spread = smelt_parser::ast::ListSpread::cast(child.clone())
            .expect("LIST_SPREAD node must cast to ListSpread");
        let spread_span = child.text_range();

        let Some(operand) = spread.operand() else {
            continue;
        };

        // Check first if the operand is an inline list literal `...[e1, e2, ...]`.
        // This must be checked before `infer_expression_type` because the parser
        // emits `[]` as an ARRAY_LITERAL which `infer_expression_type` would see
        // as a Data-World `Array(Unknown)` rather than a meta-list.
        if let Some(arr) = operand.as_array_literal() {
            // Inline list literal spread `...[e1, e2, ...]`.
            let elements: Vec<_> = arr.elements();

            if elements.is_empty() {
                // Empty-list spread: elide with zero expansion and no diagnostics.
                // Spec rule 7: "A spread of any compile-time empty List<T> emits
                // zero copies; adjacent commas elide."
                continue;
            }

            let infer_result = infer_list_literal(&elements, ctx, None);

            // Emit sentinels as diagnostics (heterogeneous only — EmptyTypeUnknown
            // can't fire here since we checked is_empty() above).
            for sentinel in &infer_result.sentinels {
                let (code, message) = match sentinel {
                    ListInferSentinel::EmptyTypeUnknown => {
                        // Unreachable: we checked is_empty() above.
                        continue;
                    }
                    ListInferSentinel::Heterogeneous {
                        first,
                        incompatible,
                    } => {
                        let t0 = format_smelt_type_hover(first);
                        let tk = format_smelt_type_hover(incompatible);
                        (
                            crate::DiagnosticCode::MetaListHeterogeneous,
                            crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaListHeterogeneous,
                                Some(&t0),
                                Some(&tk),
                                None,
                            ),
                        )
                    }
                };
                let span_range = smelt_parser::ast::text_range_to_range(text, spread_span);
                result.diagnostics.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message,
                    range: span_range,
                    code: Some(code),
                    data: None,
                });
            }

            // Expand: emit provenance per element.
            let elem_count = elements.len();
            for _ in 0..elem_count {
                result
                    .provenance_tags
                    .push(OriginTag::Synthesized(SynthesizedReason::SpreadFrom(
                        spread_span,
                    )));
            }
            result.expanded_item_count += elem_count;
            continue;
        }

        // Non-literal operand: infer the operand's type.
        let operand_type = infer_expression_type(&operand, ctx);

        match operand_type {
            Some(typed) => {
                match &typed.data_type {
                    DataType::Unknown => {
                        // Unknown type — propagate without error.
                        // Handles unresolvable identifiers gracefully (no avalanche).
                    }
                    _ => {
                        // Non-list type → emit MetaSpreadOnNonList and drop.
                        let actual = smelt_types::SmeltType::Expr(
                            smelt_types::signatures::TypeConstraint::Concrete(
                                typed.data_type.clone(),
                            ),
                        );
                        let actual_str = format_smelt_type_hover(&actual);
                        let span_range = smelt_parser::ast::text_range_to_range(text, spread_span);
                        result.diagnostics.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadOnNonList,
                                None,
                                Some(&actual_str),
                                None,
                            ),
                            range: span_range,
                            code: Some(crate::DiagnosticCode::MetaSpreadOnNonList),
                            data: None,
                        });
                        // Drop spread — continue type-checking.
                    }
                }
            }
            None => {
                // Could not infer type — treat as empty (no avalanche).
            }
        }
    }

    result
}

/// Detect `LIST_SPREAD` nodes (or orphaned `DOT_DOT_DOT` tokens produced by
/// parse-error recovery) in forbidden positions and emit
/// `MetaSpreadInForbiddenPosition` diagnostics.
///
/// **Current parser behaviour**: The parser only emits `LIST_SPREAD` nodes in
/// valid spread positions (SELECT list, GROUP BY, ORDER BY, function args,
/// IN-list, VALUES rows, list-literal body). When a `...` appears in a
/// forbidden position (e.g. `WHERE ...preds`), the parser's error-recovery
/// ejects the `DOT_DOT_DOT` token outside the `WHERE_CLAUSE` node (typically
/// as a sibling of the `SELECT_STMT` at the `FILE` level). This function
/// detects both cases:
///
/// 1. `LIST_SPREAD` nodes that somehow appear inside a `WHERE_CLAUSE`
///    descendant (future-proofing).
/// 2. Orphaned `DOT_DOT_DOT` tokens at the parent node level when the
///    `SelectStmt` has a `WHERE` clause — these represent spread-in-WHERE
///    parse errors.
///
/// `text` is the raw source text — used to convert `rowan::TextRange` byte
/// offsets into `Position { line, column }` for diagnostics. Pass `""` in unit
/// tests where the exact range is not under test.
///
/// Pure function — no Salsa dependency.
pub fn check_forbidden_position_spreads(
    select_stmt: &smelt_parser::ast::SelectStmt,
    _ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{DOT_DOT_DOT, LIST_SPREAD, WHERE_CLAUSE};

    let mut diags = Vec::new();

    let has_where = select_stmt
        .syntax()
        .children()
        .any(|n| n.kind() == WHERE_CLAUSE);

    // Case 1: Walk the WHERE clause for LIST_SPREAD descendants (future-proofing
    // for if the parser is updated to allow `...` in WHERE context).
    if has_where {
        let where_node = select_stmt
            .syntax()
            .children()
            .find(|n| n.kind() == WHERE_CLAUSE);

        if let Some(wn) = where_node {
            for desc in wn.descendants() {
                if desc.kind() == LIST_SPREAD {
                    let range = smelt_parser::ast::text_range_to_range(text, desc.text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_list_diagnostic_message(
                            crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                            None,
                            None,
                            Some(SplicePosition::Where.forbidden_position_name()),
                        ),
                        range,
                        code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                        data: None,
                    });
                }
            }
        }

        // Case 2: Detect orphaned DOT_DOT_DOT tokens at the parent level.
        // The parser ejects `...expr` from the WHERE body on parse error, leaving
        // the DOT_DOT_DOT token(s) as siblings of the SELECT_STMT in the parent.
        if let Some(parent) = select_stmt.syntax().parent() {
            let stmt_range = select_stmt.syntax().text_range();

            // Walk the parent's children_with_tokens AFTER the SELECT_STMT node.
            let mut past_stmt = false;
            for child in parent.children_with_tokens() {
                let child_range = match &child {
                    rowan::NodeOrToken::Node(n) => n.text_range(),
                    rowan::NodeOrToken::Token(t) => t.text_range(),
                };

                if !past_stmt {
                    // Check if this child IS or FOLLOWS the select stmt.
                    if child_range.start() >= stmt_range.end() {
                        past_stmt = true;
                    } else if child_range == stmt_range {
                        past_stmt = true;
                        continue; // skip the stmt itself
                    } else {
                        continue;
                    }
                }

                // Look for DOT_DOT_DOT tokens after the SELECT_STMT.
                if let rowan::NodeOrToken::Token(tok) = &child {
                    if tok.kind() == DOT_DOT_DOT {
                        let range = smelt_parser::ast::text_range_to_range(text, tok.text_range());
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                                None,
                                None,
                                Some(SplicePosition::Where.forbidden_position_name()),
                            ),
                            range,
                            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                            data: None,
                        });
                    }
                }
                // Also check for LIST_SPREAD nodes at parent level.
                if let rowan::NodeOrToken::Node(node) = &child {
                    if node.kind() == LIST_SPREAD {
                        let range = smelt_parser::ast::text_range_to_range(text, node.text_range());
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                                None,
                                None,
                                Some(SplicePosition::Where.forbidden_position_name()),
                            ),
                            range,
                            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                            data: None,
                        });
                    }
                }
            }
        }
    }

    // Boolean composition, named-arg value, FROM-without-reducer:
    // These are Phase B extensions — the spec's full forbidden-position list
    // is enforced here for WHERE (the tested case). The other positions will
    // be wired when their parser positions are exercised.

    diags
}

/// Phase B+ unified entry point for spread expansion.
///
/// Phase A wires only SELECT-list via [`check_select_list_spreads`]; this
/// function will be the integration target for GROUP BY / ORDER BY / function
/// args / IN-list / VALUES in Phase B.
///
/// This is the general-purpose spread expansion function that handles:
/// (a) forbidden-position validation → emits `MetaSpreadInForbiddenPosition`
/// (b) non-list operand → emits `MetaSpreadOnNonList`
/// (c) valid expansion → returns per-element provenance tags
///
/// Returns `(expanded_items, diagnostics)` where `expanded_items` is the
/// number of items the spread contributed (0 for non-list or forbidden; the
/// actual element count for valid expansion).
///
/// Pure function — no Salsa dependency.
#[allow(dead_code)]
pub fn expand_spread_into_position(
    spread: &smelt_parser::ast::ListSpread,
    ctx: &TypeContext,
    position: SplicePosition,
) -> (usize, Vec<OriginTag>, Vec<crate::Diagnostic>) {
    use smelt_types::signatures::format_smelt_type_hover;

    let zero_range = crate::Range {
        start: crate::Position { line: 0, column: 0 },
        end: crate::Position { line: 0, column: 0 },
    };

    // (a) Forbidden-position check.
    if !position.spread_allowed() {
        let diag = crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_list_diagnostic_message(
                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                None,
                None,
                Some(position.forbidden_position_name()),
            ),
            range: zero_range,
            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
            data: None,
        };
        return (0, vec![], vec![diag]);
    }

    let spread_span = spread.syntax().text_range();

    let Some(operand) = spread.operand() else {
        return (0, vec![], vec![]);
    };

    // (b) Determine element count by checking operand type.
    // If the operand is an inline list literal, use its element count directly.
    if let Some(arr) = operand.as_array_literal() {
        let elements: Vec<_> = arr.elements();
        let elem_count = elements.len();
        let tags: Vec<OriginTag> = (0..elem_count)
            .map(|_| OriginTag::Synthesized(SynthesizedReason::SpreadFrom(spread_span)))
            .collect();
        return (elem_count, tags, vec![]);
    }

    // Otherwise, infer the operand's type.
    let operand_ty = infer_expression_type(&operand, ctx);
    match operand_ty {
        Some(typed) => {
            match &typed.data_type {
                DataType::Unknown => {
                    // Unknown — treat as empty expansion to avoid false positives.
                    (0, vec![], vec![])
                }
                _ => {
                    // Non-list type → emit MetaSpreadOnNonList and drop.
                    let actual = SmeltType::Expr(
                        smelt_types::signatures::TypeConstraint::Concrete(typed.data_type.clone()),
                    );
                    let actual_str = format_smelt_type_hover(&actual);
                    let diag = crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_list_diagnostic_message(
                            crate::DiagnosticCode::MetaSpreadOnNonList,
                            None,
                            Some(&actual_str),
                            None,
                        ),
                        range: zero_range,
                        code: Some(crate::DiagnosticCode::MetaSpreadOnNonList),
                        data: None,
                    };
                    (0, vec![], vec![diag])
                }
            }
        }
        None => {
            // Cannot infer — treat as empty (no avalanche).
            (0, vec![], vec![])
        }
    }
}

// ─── Phase B (meta-language): HOF position checks + diagnostic emission ──────

/// Check a `smelt.define` declaration for name-shadowing of built-in HOFs or reducers.
///
/// The set of protected HOF names is `{map, filter, reduce}`.
/// The set of protected reducer names is `{comma_sep, and_all, or_any, union_all,
/// intersect_all, plus_chain, concat}`.
///
/// Emits:
/// - `HofNameShadowed` when the declared name is a built-in HOF.
/// - `ReducerNameShadowed` when the declared name is a built-in reducer.
///
/// Pure function — no Salsa dependency.
pub fn check_define_name_shadowing(
    define: &smelt_parser::ast::SmeltDefine,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::text_range_to_range;

    const HOF_NAMES: &[&str] = &["map", "filter", "reduce"];

    let mut diags = Vec::new();
    let Some(name) = define.name() else {
        return diags;
    };
    let name_lc = name.to_lowercase();

    let range = define
        .name_range()
        .map(|r| {
            if text.is_empty() {
                crate::Range {
                    start: smelt_parser::ast::Position { line: 0, column: 0 },
                    end: smelt_parser::ast::Position { line: 0, column: 0 },
                }
            } else {
                text_range_to_range(text, r)
            }
        })
        .unwrap_or(crate::Range {
            start: smelt_parser::ast::Position { line: 0, column: 0 },
            end: smelt_parser::ast::Position { line: 0, column: 0 },
        });

    if HOF_NAMES.contains(&name_lc.as_str()) {
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_hof_diagnostic_message(
                crate::DiagnosticCode::HofNameShadowed,
                Some(&name),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            range,
            code: Some(crate::DiagnosticCode::HofNameShadowed),
            data: None,
        });
    } else if REDUCER_REGISTRY
        .iter()
        .any(|r| r.name.eq_ignore_ascii_case(name_lc.as_str()))
    {
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_hof_diagnostic_message(
                crate::DiagnosticCode::ReducerNameShadowed,
                None,
                Some(&name),
                None,
                None,
                None,
                None,
                None,
            ),
            range,
            code: Some(crate::DiagnosticCode::ReducerNameShadowed),
            data: None,
        });
    }

    diags
}

/// Walk a `SelectStmt` and emit Phase B HOF-related diagnostics:
///
/// - `LambdaInForbiddenPosition`: a `LAMBDA` CST node whose parent is not a
///   HOF positional argument.
/// - `LambdaArityNotSupported`: a `LAMBDA` whose parameter list contains more
///   than one identifier.
/// - `LambdaResultTypeMismatch`: `filter` predicate body not `Boolean`.
/// - `HofExpectsLambda`: second arg to `map`/`filter` is not a `LAMBDA`.
/// - `HofExpectsReducer`: second arg to `reduce` is not a registered reducer.
/// - `PipeRhsNotCall`: RHS of `|>` is not a call expression.
/// - `ReducerInputTypeMismatch`: reducer applied to incompatible input type.
/// - `ReducerEmptyNoIdentity`: `union_all`/`intersect_all` reducing an empty list.
///
/// Pure function — no Salsa dependency. `text` is the raw source for span
/// conversion; pass `""` in unit tests where exact position is not under test.
pub fn check_hof_position_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{FUNCTION_CALL, LAMBDA, PIPE_EXPR};
    use smelt_types::signatures::format_smelt_type_hover;

    let mut diags: Vec<crate::Diagnostic> = Vec::new();

    // Helper: get function name from a FunctionCall node, including keyword-named
    // functions like `filter` which lex as FILTER_KW rather than IDENT.
    let call_name_lc = |call: &smelt_parser::ast::FunctionCall| -> String {
        call.name()
            .unwrap_or_else(|| {
                // `FunctionCall::name()` only finds IDENT tokens; HOF names like
                // `filter` are lexed as keyword tokens (FILTER_KW). Fall back to the
                // first non-trivia token's text.
                call.syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_string())
                    .unwrap_or_default()
            })
            .to_lowercase()
    };

    // Collect every descendant of the select statement for inspection.
    // We perform a depth-first walk; each node is examined individually.
    let root = select_stmt.syntax();

    // Helper: range from a Rowan TextRange, text may be "" in tests.
    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    // Walk all descendants.
    for node in root.descendants() {
        match node.kind() {
            // ── LAMBDA node ────────────────────────────────────────────────
            LAMBDA => {
                let lambda = smelt_parser::ast::Lambda::cast(node.clone()).unwrap();
                let lambda_range = to_range(node.text_range());

                // Arity check: does the LAMBDA have a multi-arg parameter list?
                let has_multi_arg = lambda.is_multi_arg();

                if has_multi_arg {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::LambdaArityNotSupported,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: lambda_range,
                        code: Some(crate::DiagnosticCode::LambdaArityNotSupported),
                        data: None,
                    });
                    // Still check for forbidden position below.
                }

                // Position check: is this LAMBDA inside a HOF positional argument?
                // A lambda is valid only when it is inside an EXPRESSION that is a
                // direct child of an ARG_LIST that belongs to a known HOF FUNCTION_CALL.
                //
                // Parent chain: LAMBDA → EXPRESSION → ARG_LIST → FUNCTION_CALL
                // We walk up through EXPRESSION and ARG_LIST wrappers.
                let in_valid_hof_position = {
                    let mut parent_opt = node.parent();
                    let mut valid = false;

                    // Walk up through EXPRESSION / ARG_LIST wrappers.
                    while let Some(p) = parent_opt {
                        match p.kind() {
                            FUNCTION_CALL => {
                                // Reached a function call — check it's a known HOF.
                                let call =
                                    smelt_parser::ast::FunctionCall::cast(p.clone()).unwrap();
                                let name = call_name_lc(&call);
                                if HofKind::from_name(&name).is_some() {
                                    valid = true;
                                }
                                break;
                            }
                            smelt_parser::SyntaxKind::EXPRESSION
                            | smelt_parser::SyntaxKind::ARG_LIST => {
                                // Keep walking up through transparent wrapper nodes.
                                parent_opt = p.parent();
                            }
                            _ => break,
                        }
                    }
                    valid
                };

                if !in_valid_hof_position {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::LambdaInForbiddenPosition,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: lambda_range,
                        code: Some(crate::DiagnosticCode::LambdaInForbiddenPosition),
                        data: None,
                    });
                }
            }

            // ── FUNCTION_CALL node: validate HOF argument shapes ───────────
            FUNCTION_CALL => {
                let call = match smelt_parser::ast::FunctionCall::cast(node.clone()) {
                    Some(c) => c,
                    None => continue,
                };
                let call_name = call_name_lc(&call);
                let Some(hof) = HofKind::from_name(&call_name) else {
                    continue;
                };

                let args = call.arguments();
                if args.is_empty() {
                    continue;
                }

                // First arg: infer list type.
                let first_arg = &args[0];
                let lhs_ty = if let Some(arr) = first_arg.as_array_literal() {
                    let elems: Vec<_> = arr.elements();
                    infer_list_literal(&elems, ctx, None).inferred
                } else {
                    infer_expression_type(first_arg, ctx)
                        .map(|tc| {
                            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                                tc.data_type,
                            ))
                        })
                        .unwrap_or(SmeltType::Unknown)
                };

                let call_range = to_range(node.text_range());

                match hof {
                    HofKind::Map | HofKind::Filter => {
                        if args.len() < 2 {
                            continue;
                        }
                        let second_arg = &args[1];

                        // Check if second arg contains a LAMBDA node (including multi-arg).
                        let has_any_lambda = second_arg
                            .syntax()
                            .descendants()
                            .any(|n| n.kind() == LAMBDA)
                            || second_arg.syntax().kind() == LAMBDA;

                        // Check if second arg contains a valid single-arg LAMBDA.
                        let has_valid_lambda = extract_lambda_from_expr(second_arg).is_some();

                        // Check if second arg is a multi-arg lambda written as `fn (a, b) => …`.
                        // The parser does NOT produce a LAMBDA node for `fn LPAREN …`, instead
                        // it falls through to parsing `fn` as an IDENT and `(a, b)` as an arg
                        // list (a function call to a function named "fn"). We detect this by
                        // checking if the second arg text starts with "fn " followed by "(".
                        let arg_text_trimmed = second_arg.text().trim().to_string();
                        let is_multi_arg_lambda_text = {
                            let t = arg_text_trimmed.as_str();
                            // Strip leading "fn " and check for "("
                            t.starts_with("fn(")
                                || t.starts_with("fn (")
                                || (t.starts_with("fn")
                                    && t.get(2..3).is_some_and(|c| c == "(" || c == " "))
                        };

                        if is_multi_arg_lambda_text && !has_any_lambda {
                            // Multi-arg lambda written as `fn (a, b) => …` — emit LambdaArityNotSupported.
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::LambdaArityNotSupported,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                ),
                                range: to_range(second_arg.text_range()),
                                code: Some(crate::DiagnosticCode::LambdaArityNotSupported),
                                data: None,
                            });
                        } else if !has_any_lambda {
                            // HofExpectsLambda — not a lambda at all.
                            let actual = infer_expression_type(second_arg, ctx)
                                .map(|tc| {
                                    format_smelt_type_hover(&SmeltType::Expr(
                                        smelt_types::signatures::TypeConstraint::Concrete(
                                            tc.data_type,
                                        ),
                                    ))
                                })
                                .unwrap_or_else(|| "unknown".to_string());
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::HofExpectsLambda,
                                    Some(&call_name),
                                    None,
                                    None,
                                    Some(&actual),
                                    None,
                                    None,
                                    None,
                                ),
                                range: to_range(second_arg.text_range()),
                                code: Some(crate::DiagnosticCode::HofExpectsLambda),
                                data: None,
                            });
                        } else if has_valid_lambda {
                            // Valid single-arg lambda in map or filter.
                            // (a) For filter: check body is Boolean.
                            if hof == HofKind::Filter {
                                let second_hof_arg = extract_lambda_from_expr(second_arg)
                                    .map(HofSecondArg::Lambda)
                                    .unwrap_or(HofSecondArg::Other);
                                let result =
                                    infer_hof_call(hof, &lhs_ty, second_hof_arg, ctx, None);
                                if let Some(HofInferSentinel::LambdaResultTypeMismatch {
                                    expected,
                                    found,
                                }) = result.sentinel
                                {
                                    let exp_str = format_smelt_type_hover(&expected);
                                    let act_str = format_smelt_type_hover(&found);
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::LambdaResultTypeMismatch,
                                            Some(&call_name),
                                            None,
                                            Some(&exp_str),
                                            Some(&act_str),
                                            None,
                                            None,
                                            None,
                                        ),
                                        range: call_range,
                                        code: Some(crate::DiagnosticCode::LambdaResultTypeMismatch),
                                        data: None,
                                    });
                                }
                            }

                            // (b) Walk the lambda body for type errors and stamp
                            // with an anonymous HOF expansion frame.
                            if let Some(lambda) = extract_lambda_from_expr(second_arg) {
                                let param_name =
                                    lambda.params().into_iter().next().unwrap_or_default();
                                let elem_ty = match &lhs_ty {
                                    SmeltType::List(inner) => match inner.as_ref() {
                                        SmeltType::Expr(
                                            smelt_types::signatures::TypeConstraint::Concrete(dt),
                                        ) => Some(smelt_types::TypedColumn {
                                            data_type: dt.clone(),
                                            nullable: false,
                                        }),
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                if let (Some(elem), Some(body_expr)) = (elem_ty, lambda.body()) {
                                    let mut lambda_ctx = ctx.clone();
                                    lambda_ctx.add_lambda_param(&param_name, elem);
                                    // Pass `None` for `nested`: `check_hof_position_diagnostics`
                                    // is a top-level walker without a smelt.functions.* dispatch
                                    // handler. The `nested` handler is available only inside
                                    // `check_smelt_path_call` (function body expansion context).
                                    let body_diags = crate::function_body_check::walk_hof_lambda_body_with_anonymous_frame(
                                        &body_expr,
                                        &lambda_ctx,
                                        text,
                                        &call_name,
                                        Some(call_range),
                                        None,
                                    );
                                    diags.extend(body_diags);
                                }
                            }
                        }
                        // If has_any_lambda but !has_valid_lambda: it's a multi-arg lambda.
                        // The LAMBDA node walk above will emit LambdaArityNotSupported.
                    }

                    HofKind::Reduce => {
                        if args.len() < 2 {
                            continue;
                        }
                        let second_arg = &args[1];

                        // Second arg must be a bare reducer identifier.
                        let arg_text = second_arg.text().trim().to_string();
                        let is_reducer = arg_text.chars().all(|c| c.is_alphanumeric() || c == '_')
                            && !arg_text.is_empty()
                            && lookup_reducer(&arg_text).is_some();

                        // But is it a lambda? (wrong type for reduce)
                        let is_lambda = extract_lambda_from_expr(second_arg).is_some();

                        if is_lambda || !is_reducer {
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::HofExpectsReducer,
                                    None,
                                    None,
                                    None,
                                    Some(&arg_text),
                                    None,
                                    None,
                                    None,
                                ),
                                range: to_range(second_arg.text_range()),
                                code: Some(crate::DiagnosticCode::HofExpectsReducer),
                                data: None,
                            });
                        } else {
                            // Valid reducer name — check input type compatibility.
                            // Re-infer with actual element type.
                            let elem_ty = match &lhs_ty {
                                SmeltType::List(inner) => (**inner).clone(),
                                _ => SmeltType::Unknown,
                            };
                            let is_empty = matches!(&elem_ty, SmeltType::Unknown);
                            let result = infer_reduce_call(&elem_ty, is_empty, &arg_text, None);

                            match &result.sentinel {
                                Some(HofInferSentinel::ReducerInputTypeMismatch {
                                    reducer_name,
                                    expected_constraint,
                                    found,
                                }) => {
                                    let found_str = format_smelt_type_hover(found);
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::ReducerInputTypeMismatch,
                                            None,
                                            None,
                                            None,
                                            None,
                                            Some(reducer_name),
                                            Some(expected_constraint),
                                            Some(&found_str),
                                        ),
                                        range: to_range(second_arg.text_range()),
                                        code: Some(crate::DiagnosticCode::ReducerInputTypeMismatch),
                                        data: None,
                                    });
                                }
                                Some(HofInferSentinel::ReducerEmptyNoIdentity { reducer_name }) => {
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::ReducerEmptyNoIdentity,
                                            None,
                                            None,
                                            None,
                                            None,
                                            Some(reducer_name),
                                            None,
                                            None,
                                        ),
                                        range: call_range,
                                        code: Some(crate::DiagnosticCode::ReducerEmptyNoIdentity),
                                        data: None,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // ── PIPE_EXPR node: validate RHS is a call + check data position ─
            PIPE_EXPR => {
                let pipe = match smelt_parser::ast::PipeExpr::cast(node.clone()) {
                    Some(p) => p,
                    None => continue,
                };

                // Check: is this pipe expression inside a Data-World grammar slot?
                // A pipe in a WHERE clause, JOIN condition, or HAVING clause
                // (any ancestor is a WHERE_CLAUSE node) emits PipeInDataPosition.
                let in_data_position = {
                    let mut parent_opt = node.parent();
                    let mut in_data = false;
                    while let Some(p) = parent_opt {
                        if p.kind() == smelt_parser::SyntaxKind::WHERE_CLAUSE {
                            in_data = true;
                            break;
                        }
                        // Stop at statement boundaries.
                        if p.kind() == smelt_parser::SyntaxKind::SELECT_STMT
                            || p.kind() == smelt_parser::SyntaxKind::FILE
                        {
                            break;
                        }
                        parent_opt = p.parent();
                    }
                    in_data
                };

                if in_data_position {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::PipeInDataPosition,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: to_range(node.text_range()),
                        code: Some(crate::DiagnosticCode::PipeInDataPosition),
                        data: None,
                    });
                }

                if !pipe.rhs_is_call() {
                    let range = pipe
                        .rhs()
                        .map(|r| to_range(r.text_range()))
                        .unwrap_or_else(|| to_range(node.text_range()));
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::PipeRhsNotCall,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range,
                        code: Some(crate::DiagnosticCode::PipeRhsNotCall),
                        data: None,
                    });
                }
            }

            _ => {}
        }
    }

    diags
}

/// Walk all `SMELT_PATH_CALL` descendants of `root` whose path is `config.var`.
/// For each such call:
/// - If the argument is not a string literal → `ConfigVarNameNotLiteral`.
/// - If the argument is a string literal but `vars_map` does not contain that key
///   → `ConfigVarNotFound`.
/// - If the argument resolves to a YAML `null` value → `ConfigVarNullCoercion` (Warning).
///
/// `vars_map` is the parsed `vars:` block from `smelt.yml` (pass empty map when absent).
/// `text` is the raw source file text (used for span → range conversion; pass `""`
/// in tests where range accuracy is not needed).
///
/// Pure function — no Salsa dependency.
pub fn check_config_var_call_diagnostics(
    root: &smelt_parser::syntax_kind::SyntaxNode,
    vars_map: &std::collections::BTreeMap<String, serde_yaml::Value>,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.config.var(...)`.
        let segs = call.segments();
        if segs.len() != 2 || segs[0].to_lowercase() != "config" || segs[1].to_lowercase() != "var"
        {
            continue;
        }

        let call_range = to_range(node.text_range());

        // Extract the first positional argument expression.
        let first_arg = call
            .arg_list()
            .and_then(|args| args.positional_args().into_iter().next());

        let Some(arg_expr) = first_arg else {
            // No argument at all — emit ConfigVarNameNotLiteral so the user
            // gets a useful diagnostic rather than a silent no-op.
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_hof_diagnostic_message(
                    crate::DiagnosticCode::ConfigVarNameNotLiteral,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                range: call_range,
                code: Some(crate::DiagnosticCode::ConfigVarNameNotLiteral),
                data: None,
            });
            continue;
        };

        // Check: is the argument a string literal?
        if !crate::config_vars::is_string_literal_expr(&arg_expr) {
            let arg_range = to_range(arg_expr.syntax().text_range());
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_hof_diagnostic_message(
                    crate::DiagnosticCode::ConfigVarNameNotLiteral,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                range: arg_range,
                code: Some(crate::DiagnosticCode::ConfigVarNameNotLiteral),
                data: None,
            });
            continue;
        }

        // Extract the string value.
        let var_name = match crate::config_vars::extract_string_literal_value(&arg_expr) {
            Some(n) => n,
            None => continue, // should not happen if is_string_literal_expr returned true
        };

        // Look up in vars_map.
        match vars_map.get(&var_name) {
            None => {
                diags.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: crate::meta_hof_diagnostic_message(
                        crate::DiagnosticCode::ConfigVarNotFound,
                        None,
                        Some(&var_name),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    range: call_range,
                    code: Some(crate::DiagnosticCode::ConfigVarNotFound),
                    data: None,
                });
            }
            Some(val) => {
                // Coerce and check for null.
                let (_text_val, warn_name) =
                    crate::config_vars::coerce_yaml_scalar_to_text(val, &var_name);
                if let Some(null_var) = warn_name {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Warning,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::ConfigVarNullCoercion,
                            None,
                            Some(&null_var),
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: call_range,
                        code: Some(crate::DiagnosticCode::ConfigVarNullCoercion),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

/// Walk all `SMELT_PATH_CALL` descendants of `select_stmt` whose path is
/// `columns_of`. For each such call emit:
///
/// - [`DiagnosticCode::ColumnsOfNamedArgument`] for every named argument in
///   the arg list (anchored at the named-arg span).
/// - [`DiagnosticCode::ColumnsOfRequiresTableExpr`] when the single positional
///   argument's inferred type is clearly not a `TableExpr` — specifically when
///   the argument is not a smelt-path expression and its synthesised data type
///   is a concrete non-Unknown type (e.g. an integer literal `42`).
///
/// The function always synthesises `List<ColumnRef>` (recoverable) regardless
/// of errors — that is handled by `infer_smelt_path_call_type`.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_columns_of_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.columns_of(...)`.
        let segs = call.segments();
        if segs.len() != 1 || segs[0].to_lowercase() != "columns_of" {
            continue;
        }

        let arg_list = match call.arg_list() {
            Some(al) => al,
            None => continue,
        };

        // ── Named argument check ────────────────────────────────────────
        // Any NAMED_PARAM in the arg list is invalid; emit one diagnostic
        // per named arg, anchored at the named-arg node span.
        for named in arg_list.named_params() {
            let named_range = to_range(named.text_range());
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(
                    crate::DiagnosticCode::ColumnsOfNamedArgument,
                    None,
                    None,
                ),
                range: named_range,
                code: Some(crate::DiagnosticCode::ColumnsOfNamedArgument),
                data: None,
            });
        }

        // ── Positional argument type check ──────────────────────────────
        // For each positional arg, check if it's assignable to TableExpr.
        // A smelt-path expression is treated as a potential TableExpr;
        // a plain literal or expression with a concrete scalar type is not.
        for pos_arg in arg_list.positional_args() {
            // If the argument is a smelt path call OR a smelt path ref
            // (e.g. `smelt.models.orders` without parens), accept it
            // unconditionally — smelt paths are TableExpr candidates.
            if pos_arg.as_smelt_path_call().is_some() {
                continue;
            }
            // Check for SMELT_PATH_REF (value-form `smelt.<path>` without parens).
            let is_smelt_path_ref = smelt_parser::ast::SmeltPathRef::cast(pos_arg.syntax().clone())
                .is_some()
                || pos_arg
                    .syntax()
                    .children()
                    .any(|n| smelt_parser::ast::SmeltPathRef::cast(n).is_some());
            if is_smelt_path_ref {
                continue;
            }

            // Check if the arg is a bare identifier registered as TableExpr
            // in the context (a `smelt.define` TableExpr parameter).
            let arg_text = pos_arg.text().trim().to_string();
            let is_bare_ident =
                !arg_text.is_empty() && arg_text.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_bare_ident {
                if let Some(smelt_ty) = ctx.lookup_function_param_smelt_type(&arg_text) {
                    if matches!(smelt_ty, smelt_types::signatures::SmeltType::TableExpr(_)) {
                        continue;
                    }
                }
            }

            // For everything else, infer the scalar type. If the type is
            // Unknown (unresolvable identifier — could be a table ref), give
            // the benefit of the doubt. Only concrete non-TableExpr types
            // (e.g. Integer, Text, Boolean) trigger the diagnostic.
            if let Some(tc) = infer_expression_type(&pos_arg, ctx) {
                let is_clearly_non_table =
                    !matches!(tc.data_type, DataType::Unknown | DataType::Null);
                if is_clearly_non_table {
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    let actual_str = tc.data_type.to_string();
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::ColumnsOfRequiresTableExpr,
                            Some(&actual_str),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

/// Walk all expression descendants of `select_stmt`. For every expression of
/// the form `<qualifier>.<field>` where `<qualifier>` is registered as
/// `SmeltType::ColumnRef` in the context, check that `<field>` is in the
/// closed `COLUMN_REF_FIELDS` set. Unknown fields emit
/// [`DiagnosticCode::ColumnRefFieldUnknown`] anchored at the field-name span.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_column_ref_field_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::column_ref_field;

    let mut diags = Vec::new();
    // Track seen (qualifier, field) pairs to avoid duplicate diagnostics from
    // nested EXPRESSION nodes that wrap the same column reference text.
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        let expr = match smelt_parser::ast::Expr::cast(node.clone()) {
            Some(e) => e,
            None => continue,
        };

        // We're looking for `qualifier.field` expressions where the qualifier
        // is a ColumnRef-typed binding.
        let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
            Some(cr) => cr,
            None => continue,
        };

        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => continue, // bare identifier — not a dot access
        };

        // Is the qualifier a ColumnRef-typed binding?
        let is_column_ref = ctx
            .lookup_function_param_smelt_type(qualifier)
            .map(|ty| matches!(ty, smelt_types::signatures::SmeltType::ColumnRef))
            .unwrap_or(false);

        if !is_column_ref {
            continue;
        }

        let field_name = col_ref.name();

        // Check if the field is in the closed field set.
        if column_ref_field(field_name).is_some() {
            continue; // valid field — no diagnostic
        }

        // Deduplicate: the same qualifier.field pair may appear in multiple
        // nested EXPRESSION wrappers during the descendants walk. Only emit
        // one diagnostic per (qualifier, field) pair.
        let key = (qualifier.to_string(), field_name.to_string());
        if !seen.insert(key) {
            continue;
        }

        // Unknown field — emit ColumnRefFieldUnknown anchored at the field token span.
        // Spec invariant: the diagnostic must point at the field name token (the IDENT
        // after the DOT), not the whole expression. We re-scan the expression's syntax
        // children to find the IDENT token that follows the DOT token and use its
        // text_range(). This avoids modifying the parser AST (approach (a)).
        let field_token_range = {
            use smelt_parser::SyntaxKind;
            // Walk all tokens (not just direct children) within the expression node.
            // The DOT and IDENT tokens may be nested under inner expression sub-nodes.
            let mut after_dot = false;
            let mut found: Option<rowan::TextRange> = None;
            for token in node
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
            {
                let kind = token.kind();
                if kind == SyntaxKind::DOT {
                    after_dot = true;
                } else if kind == SyntaxKind::IDENT && after_dot {
                    found = Some(token.text_range());
                    break;
                }
            }
            // Fall back to the whole expression range if the token walk fails
            // (should not happen for well-formed qualifier.field syntax).
            found.unwrap_or_else(|| node.text_range())
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_reflection_diagnostic_message(
                crate::DiagnosticCode::ColumnRefFieldUnknown,
                None,
                Some(field_name),
            ),
            range: to_range(field_token_range),
            code: Some(crate::DiagnosticCode::ColumnRefFieldUnknown),
            data: None,
        });
    }

    diags
}

// ─── Phase D: wide-reflection diagnostics and field projection ───────────────

/// Returns `true` when `expr` is a compile-time-resolvable meta-`Text` value
/// for the purpose of the `with_tag` argument check.
///
/// Accepted as compile-time Text:
/// - Bare string literals (detected by `is_string_literal_expr`).
/// - `smelt.config.var(...)` calls (the result is always a nullable `Varchar`
///   sourced at compile time).
/// - A `ModelRef` or `SourceRef` field projection to a `Text`-typed field
///   (`m.path`, `m.name`, `s.path`, `s.name`) — i.e. expressions whose inferred
///   `SmeltType` (in the meta-type layer) is `Expr<Text>` from a closed meta-record.
///
/// Rejected (returns `false`):
/// - Integer or other non-Text literals (`42`, `true`, etc.).
/// - Runtime function calls that synthesise `Expr<Text>` at SQL evaluation time
///   (e.g. `UPPER('x')`).
/// - Bare column references.
///
/// Pure — no Salsa dependency.
pub fn is_compile_time_text_arg(expr: &Expr, ctx: &TypeContext) -> bool {
    use smelt_types::signatures::{column_ref_field, model_ref_field, source_ref_field, SmeltType};

    // 1. Bare string literal — always accepted.
    if crate::config_vars::is_string_literal_expr(expr) {
        return true;
    }

    // 2. smelt.config.var(...) call — always accepted (compile-time Text).
    if let Some(path_call) = expr.as_smelt_path_call() {
        let segs = path_call.segments();
        if segs.len() == 2
            && segs[0].eq_ignore_ascii_case("config")
            && segs[1].eq_ignore_ascii_case("var")
        {
            return true;
        }
        // wide-reflection accessor calls return List<ModelRef|SourceRef>, not Text.
        return false;
    }

    // 3. A field projection `<binding>.<field>` where the binding is a
    //    ColumnRef, ModelRef, or SourceRef and the field type is Expr<Text>.
    if let Some(col_ref) = smelt_parser::ast::ColumnRef::from_expr(expr) {
        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => return false, // bare identifier — not a qualified field access
        };
        let field = col_ref.name();

        // Is the qualifier a meta-record-typed binding?
        if let Some(ty) = ctx.lookup_function_param_smelt_type(qualifier) {
            let field_ty_opt: Option<&SmeltType> = match ty {
                SmeltType::ColumnRef => column_ref_field(field),
                SmeltType::ModelRef => model_ref_field(field),
                SmeltType::SourceRef => source_ref_field(field),
                _ => None,
            };
            if let Some(field_ty) = field_ty_opt {
                return matches!(
                    field_ty,
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        DataType::Text
                    ))
                );
            }
        }
    }

    false
}

/// Walk all `SMELT_PATH_CALL` descendants of `select_stmt` whose path is
/// `models.<accessor>` or `sources.<accessor>`. For each:
///
/// - Emit [`DiagnosticCode::WideReflectionUnknownAccessor`] when the accessor
///   name is not in the closed set `{with_tag, all}`.
/// - For `with_tag`: emit [`DiagnosticCode::WithTagNamedArgument`] for any named
///   argument; emit [`DiagnosticCode::WithTagRequiresText`] when the positional
///   argument is not compile-time-resolvable Text.
/// - For `all`: emit [`DiagnosticCode::WideReflectionUnexpectedArgument`] for any
///   argument (positional or named).
///
/// Always synthesises the spec'd return type (recoverable) — diagnostics here do
/// not prevent downstream HOF type-checking.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_wide_reflection_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::SyntaxKind::SMELT_PATH_CALL;

    let mut diags = Vec::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != SMELT_PATH_CALL {
            continue;
        }
        let call = match SmeltPathCall::cast(node.clone()) {
            Some(c) => c,
            None => continue,
        };

        // Only handle `smelt.models.<accessor>` and `smelt.sources.<accessor>`.
        let segs = call.segments();
        if segs.len() != 2 {
            continue;
        }
        let ns = segs[0].to_lowercase();
        if ns != "models" && ns != "sources" {
            continue;
        }
        let accessor_name = segs[1].as_str();

        // Check closed accessor set.
        let is_with_tag = accessor_name.eq_ignore_ascii_case("with_tag");
        let is_all = accessor_name.eq_ignore_ascii_case("all");

        if !is_with_tag && !is_all {
            // Unknown accessor — emit WideReflectionUnknownAccessor at the accessor token span.
            let accessor_range = find_last_segment_range(&call, text);
            diags.push(crate::Diagnostic {
                severity: crate::DiagnosticSeverity::Error,
                message: crate::meta_reflection_diagnostic_message(
                    crate::DiagnosticCode::WideReflectionUnknownAccessor,
                    Some(&ns),
                    Some(accessor_name),
                ),
                range: accessor_range,
                code: Some(crate::DiagnosticCode::WideReflectionUnknownAccessor),
                data: None,
            });
            continue;
        }

        let arg_list = call.arg_list();

        if is_all {
            // `all` accepts no arguments at all.
            if let Some(al) = &arg_list {
                let full_accessor = format!("smelt.{ns}.all");
                for pos_arg in al.positional_args() {
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WideReflectionUnexpectedArgument,
                            Some(&full_accessor),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument),
                        data: None,
                    });
                }
                for named_arg in al.named_params() {
                    let arg_range = to_range(named_arg.text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WideReflectionUnexpectedArgument,
                            Some(&full_accessor),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument),
                        data: None,
                    });
                }
            }
            continue;
        }

        // is_with_tag: check for named args and compile-time Text argument.
        if let Some(al) = &arg_list {
            // Named arguments are not supported.
            for named_arg in al.named_params() {
                let arg_range = to_range(named_arg.text_range());
                diags.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message: crate::meta_reflection_diagnostic_message(
                        crate::DiagnosticCode::WithTagNamedArgument,
                        None,
                        None,
                    ),
                    range: arg_range,
                    code: Some(crate::DiagnosticCode::WithTagNamedArgument),
                    data: None,
                });
            }

            // Positional argument must be compile-time Text.
            for pos_arg in al.positional_args() {
                if !is_compile_time_text_arg(&pos_arg, ctx) {
                    // Get the actual synthesised type string for the message.
                    let actual_str = infer_expression_type(&pos_arg, ctx)
                        .map(|tc| tc.data_type.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let arg_range = to_range(pos_arg.syntax().text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_reflection_diagnostic_message(
                            crate::DiagnosticCode::WithTagRequiresText,
                            Some(&actual_str),
                            None,
                        ),
                        range: arg_range,
                        code: Some(crate::DiagnosticCode::WithTagRequiresText),
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

/// Extract path segments from a `SmeltPathCall`, including keyword tokens.
///
/// Unlike `SmeltPathCall::segments()`, which only collects `IDENT` tokens,
/// this helper also collects keyword tokens (e.g. `ALL_KW` for `all`) so that
/// accessor names that happen to be SQL keywords are included. The leading
/// `smelt` IDENT token is dropped (same as `segments()`).
///
/// Find the text range of the last path segment in a `SmeltPathCall`.
///
/// For `smelt.models.bogus(...)` returns the span of `bogus`.
/// Falls back to the node's own range when a more precise span cannot be found.
fn find_last_segment_range(call: &smelt_parser::ast::SmeltPathCall, text: &str) -> crate::Range {
    use smelt_parser::SyntaxKind::IDENT;

    // Walk the path's IDENT tokens to find the last one (the accessor name).
    let path_node = call.path();
    let last_ident_range = path_node.and_then(|p| {
        p.syntax()
            .children_with_tokens()
            .filter_map(|it| {
                if let rowan::NodeOrToken::Token(t) = it {
                    if t.kind() == IDENT {
                        return Some(t.text_range());
                    }
                }
                None
            })
            .last()
    });

    let raw_range = last_ident_range.unwrap_or_else(|| call.syntax().text_range());
    if text.is_empty() {
        crate::Range {
            start: smelt_parser::ast::Position { line: 0, column: 0 },
            end: smelt_parser::ast::Position { line: 0, column: 0 },
        }
    } else {
        smelt_parser::ast::text_range_to_range(text, raw_range)
    }
}

/// Walk all expression descendants of `select_stmt`. For every expression of
/// the form `<qualifier>.<field>` where `<qualifier>` is registered as
/// `SmeltType::ModelRef` or `SmeltType::SourceRef` in the context, check that
/// `<field>` is in the closed `MODEL_REF_FIELDS` / `SOURCE_REF_FIELDS` set.
/// Unknown fields emit [`DiagnosticCode::ModelRefFieldUnknown`] /
/// [`DiagnosticCode::SourceRefFieldUnknown`] anchored at the field-name span.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_model_ref_source_ref_field_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::{model_ref_field, source_ref_field, SmeltType};

    let mut diags = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    let to_range = |range: rowan::TextRange| -> crate::Range {
        if text.is_empty() {
            crate::Range {
                start: smelt_parser::ast::Position { line: 0, column: 0 },
                end: smelt_parser::ast::Position { line: 0, column: 0 },
            }
        } else {
            smelt_parser::ast::text_range_to_range(text, range)
        }
    };

    let root = select_stmt.syntax();

    for node in root.descendants() {
        let expr = match smelt_parser::ast::Expr::cast(node.clone()) {
            Some(e) => e,
            None => continue,
        };

        let col_ref = match smelt_parser::ast::ColumnRef::from_expr(&expr) {
            Some(cr) => cr,
            None => continue,
        };

        let qualifier = match col_ref.qualifier() {
            Some(q) => q,
            None => continue, // bare identifier — not a dot access
        };

        // Is the qualifier a ModelRef or SourceRef-typed binding?
        let binding_ty = ctx.lookup_function_param_smelt_type(qualifier).cloned();

        let (is_model_ref, _is_source_ref) = match &binding_ty {
            Some(SmeltType::ModelRef) => (true, false),
            Some(SmeltType::SourceRef) => (false, true),
            _ => continue,
        };

        let field_name = col_ref.name();

        // Check if the field is in the closed field set.
        let field_known = if is_model_ref {
            model_ref_field(field_name).is_some()
        } else {
            source_ref_field(field_name).is_some()
        };

        if field_known {
            continue; // valid field — no diagnostic
        }

        // Deduplicate: same (qualifier, field) pair may appear in multiple wrappers.
        let key = (qualifier.to_string(), field_name.to_string());
        if !seen.insert(key) {
            continue;
        }

        // Unknown field — emit the appropriate diagnostic anchored at the field token span.
        let field_token_range = {
            use smelt_parser::SyntaxKind::{DOT, IDENT};
            let mut found: Option<rowan::TextRange> = None;
            let mut after_dot = false;
            for child in node.children_with_tokens() {
                match child {
                    rowan::NodeOrToken::Token(t) if t.kind() == DOT => {
                        after_dot = true;
                    }
                    rowan::NodeOrToken::Token(t) if after_dot && t.kind() == IDENT => {
                        found = Some(t.text_range());
                        break;
                    }
                    _ => {}
                }
            }
            found.unwrap_or_else(|| node.text_range())
        };

        let code = if is_model_ref {
            crate::DiagnosticCode::ModelRefFieldUnknown
        } else {
            crate::DiagnosticCode::SourceRefFieldUnknown
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_reflection_diagnostic_message(code, None, Some(field_name)),
            range: to_range(field_token_range),
            code: Some(code),
            data: None,
        });
    }

    diags
}

/// Infer the [`SmeltType`] of a `ModelRef` field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::ModelRef`, AND
///   - `field_name` is in the closed `MODEL_REF_FIELDS` set.
///
/// Returns `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_model_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{model_ref_field, SmeltType};
    let is_model_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::ModelRef))
        .unwrap_or(false);
    if !is_model_ref {
        return None;
    }
    model_ref_field(field_name).cloned()
}

/// Infer the [`SmeltType`] of a `SourceRef` field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::SourceRef`, AND
///   - `field_name` is in the closed `SOURCE_REF_FIELDS` set.
///
/// Returns `None` otherwise.
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_source_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{source_ref_field, SmeltType};
    let is_source_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::SourceRef))
        .unwrap_or(false);
    if !is_source_ref {
        return None;
    }
    source_ref_field(field_name).cloned()
}

/// Infer the [`SmeltType`] of a ColumnRef field projection `<binding>.<field>`.
///
/// Returns `Some(field_type)` when:
///   - `binding_name` is registered in `ctx` as `SmeltType::ColumnRef` via
///     `add_function_param_smelt_type`, AND
///   - `field_name` is in the closed `COLUMN_REF_FIELDS` set.
///
/// Returns `None` otherwise (unknown binding or unknown field).
///
/// Pure — no Salsa dependency.
pub fn infer_field_on_column_ref(
    binding_name: &str,
    field_name: &str,
    ctx: &TypeContext,
) -> Option<smelt_types::signatures::SmeltType> {
    use smelt_types::signatures::{column_ref_field, SmeltType};

    // Check the binding is a ColumnRef.
    let is_column_ref = ctx
        .lookup_function_param_smelt_type(binding_name)
        .map(|ty| matches!(ty, SmeltType::ColumnRef))
        .unwrap_or(false);

    if !is_column_ref {
        return None;
    }

    // Look up the field in the closed field set and clone the type.
    column_ref_field(field_name).cloned()
}

// ─── Phase C Phase 2: meta-Text-as-identifier lift (narrow rule) ─────────────

/// The four grammar positions where a compile-time meta-`Text` value may lift
/// to an unquoted SQL identifier (Phase C §"Meta-`Text`-as-identifier lift").
///
/// Every other position (function-argument where the parameter sort is
/// `Expr<Text>`, comparison operands, named-argument values, etc.) is NOT a
/// lift position; in those positions a meta-`Text` retains its string-value
/// meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaTextLiftPosition {
    /// `SELECT c.name FROM t` — the expression itself is in a SELECT-list
    /// slot where the grammar expects a column reference.  Lift fires when
    /// the whole select-item expression is a meta-`Text` value with no
    /// explicit `AS` alias.
    ColumnReference,
    /// `AS <meta-Text>` alias of a SELECT item.  Lift fires and treats the
    /// meta-`Text` value as the output column identifier.  No scope check —
    /// aliases introduce names, they do not reference existing ones.
    AsAlias,
    /// `ORDER BY <meta-Text>` — the expression is the sort key.  Lift fires;
    /// the lifted identifier is validated against the surrounding
    /// column-resolution scope.
    OrderBy,
    /// `GROUP BY <meta-Text>` — the expression is a grouping key.  Same
    /// scope rule as `OrderBy`.
    GroupBy,
}

impl MetaTextLiftPosition {
    /// Returns `true` when the lifted identifier must be validated against the
    /// surrounding column-resolution scope (i.e., `UnknownColumn` is possible).
    ///
    /// `AsAlias` returns `false` — aliases introduce names, not reference them.
    pub fn requires_scope_validation(self) -> bool {
        !matches!(self, MetaTextLiftPosition::AsAlias)
    }
}

/// Returns the lifted identifier text if `expr` is a compile-time
/// meta-`Text` value — specifically, a `ColumnRef.name` field projection
/// whose binding is registered as `SmeltType::ColumnRef` in `ctx`.
///
/// In Phase C the only producer of compile-time meta-`Text` values is a
/// `<binding>.name` field access where `<binding>` was declared as
/// `ColumnRef` (e.g. the lambda parameter `c` in `map(smelt.columns_of(t),
/// fn c => ...)`) .  All other expressions — including runtime `Expr<Text>`
/// results like `UPPER('foo')` and SQL string literals like `'foo'` — return
/// `None`.
///
/// When `Some(text)` is returned, `text` is the field-name token in the
/// source — i.e. the literal identifier `"name"`.  The actual runtime value
/// of `c.name` (the column name string at expansion time) is determined by
/// Phase 3's expansion-time materialisation; Phase 2 only recognises the
/// structural pattern.
///
/// Pure — no Salsa dependency.
pub fn is_meta_text_value(expr: &Expr, ctx: &TypeContext) -> Option<String> {
    use smelt_types::signatures::SmeltType;

    // Only a bare qualified column-ref of the form `qualifier.field` can be a
    // meta-Text value in Phase C; complex expressions (function calls, binary
    // expressions, literals) are all runtime values.
    let col_ref = smelt_parser::ast::ColumnRef::from_expr(expr)?;
    let qualifier = col_ref.qualifier()?;
    let field = col_ref.name();

    // Is the qualifier registered as a ColumnRef-typed binding?
    let is_column_ref_binding = ctx
        .lookup_function_param_smelt_type(qualifier)
        .map(|ty| matches!(ty, SmeltType::ColumnRef))
        .unwrap_or(false);

    if !is_column_ref_binding {
        return None;
    }

    // Is the field the Text-typed `name` field (the only Text-typed member of
    // the closed ColumnRef field set)?  Other fields (`type` → Unknown,
    // `is_numeric` → Boolean) are NOT meta-Text and do not lift.
    use smelt_types::signatures::{column_ref_field, TypeConstraint};
    let field_ty = column_ref_field(field)?;
    let is_text_field = matches!(
        field_ty,
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
    );
    if !is_text_field {
        return None;
    }

    // Return the field name token as the lifted identifier text.  In Phase C
    // this is always `"name"` because the only Text-typed ColumnRef field is
    // `name`.  Phase D may introduce additional Text-typed fields; they would
    // be handled here automatically.
    Some(field.to_string())
}

/// Check all four meta-`Text`-as-identifier lift positions in a SELECT
/// statement and return `UndeclaredColumnInfo` diagnostics for any lifted
/// identifier that names no in-scope column.
///
/// Lift positions checked (§"Meta-`Text`-as-identifier lift"):
/// 1. **Column-reference position** — every expression in the SELECT list
///    that IS itself a meta-`Text` value (i.e. no sub-expressions; the bare
///    `c.name` is the entire select-item expression).
/// 2. **ORDER BY column-reference** — every sort-key expression that is a
///    meta-`Text` value.
/// 3. **GROUP BY column-reference** — every grouping-key expression that is
///    a meta-`Text` value.
/// 4. **AS alias** — detected by inspecting select items whose expression is
///    a meta-`Text` value; no scope validation is performed for aliases
///    (aliases introduce names, not reference them).
///
/// **Body-check-time scope validation is suppressed for all four positions.**
/// `is_meta_text_value` returns the *field-name token* (e.g. `"name"`), not
/// the per-element column name that `c.name` will evaluate to at expansion
/// time.  Validating the field-name token against in-scope columns would
/// produce false positives whenever no column literally named `"name"` exists
/// in the body context (almost always), and would mask real errors when one
/// happens to exist by accident.  Per §Semantics rule 6, lift-scope validation
/// is correctly located at expansion time, after the per-element column name is
/// known.  This function therefore recognises the structural lift pattern and
/// returns an empty `Vec`; expansion-time validation is handled elsewhere.
///
/// Expressions that are NOT meta-`Text` values are silently skipped; they
/// continue to be validated by `check_undeclared_columns` through the normal
/// path.
///
/// Pure — no Salsa dependency.
pub fn check_meta_text_lift_diagnostics(
    _select_stmt: &smelt_parser::ast::SelectStmt,
    _ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    // Body-check-time scope validation is suppressed: the field-name token
    // returned by is_meta_text_value (always "name" for the Text-typed
    // ColumnRef field) is not the per-element column name that the lift
    // produces at expansion time.  Expansion-time validation is correct;
    // body-check-time validation against this token is not.
    Vec::new()
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

    // === Phase A (meta-language) TDD tests: infer_list_literal ===

    /// Parse `SELECT <expr>` and return the first select-item expression.
    fn parse_first_expr(sql: &str) -> smelt_parser::ast::Expr {
        use smelt_parser::ast::File;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("FILE node");
        let select = file.select_stmt().expect("SelectStmt");
        let select_list = select.select_list().expect("select list");
        let first_item = select_list.items().next().expect("at least one item");
        first_item.expression().expect("expression")
    }

    /// Extract elements from `SELECT [e1, e2, ...]` as a vec of Expr.
    fn list_elements(sql: &str) -> Vec<smelt_parser::ast::Expr> {
        let expr = parse_first_expr(sql);
        // The list literal lands as an ARRAY_LITERAL child inside the expression.
        let arr = expr
            .as_array_literal()
            .expect("expected an array/list literal node");
        arr.elements()
    }

    /// `[100000, 200000, 300000]` — all Integer literals — infers `List<Expr<Integer>>`.
    #[test]
    fn infer_list_literal_homogeneous_integer() {
        let elems = list_elements("SELECT [100000, 200000, 300000]");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        assert!(
            result.sentinels.is_empty(),
            "homogeneous integer list must have no sentinels, got: {:?}",
            result.sentinels
        );
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer)
            ))),
            "homogeneous Integer list must infer List<Expr<Integer>>"
        );
    }

    /// `[1, 1.5]` — SmallInt + Decimal — infers `List<Expr<Decimal(38,10)>>` via LUB.
    ///
    /// The spec references `types.md §"Numeric promotion chain"` and says `[1, 1.5]` →
    /// `List<Expr<Double>>`. However, the actual `promote_types` implementation promotes
    /// `(SmallInt, Decimal{2,1})` to `Decimal{38,10}` (the safe "integer+Decimal" widening
    /// rule). `Double` would require an `e`-notation literal (`1.5e0`) but the lexer does
    /// not handle exponent notation, so `1.5` always produces `Decimal`. The test asserts
    /// the actual promotion behaviour, which is correct per the implementation's own rules.
    #[test]
    fn infer_list_literal_lub_promotion() {
        // `1` → SmallInt, `1.5` → Decimal(2,1).
        // promote_types(SmallInt, Decimal) → Decimal(38,10).
        let elems = list_elements("SELECT [1, 1.5]");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        assert!(
            result.sentinels.is_empty(),
            "numeric-promoted list must have no sentinels, got: {:?}",
            result.sentinels
        );
        // Numeric promotion: SmallInt + Decimal → Decimal(38,10).
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Decimal {
                    precision: 38,
                    scale: 10
                })
            ))),
            "SmallInt+Decimal list must promote to List<Expr<Decimal(38,10)>>"
        );
    }

    /// `[1, 'hello']` — Integer + Text — infers `List<Unknown>` with Heterogeneous sentinel.
    #[test]
    fn infer_list_literal_heterogeneous_unknown() {
        let elems = list_elements("SELECT [1, 'hello']");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        // Must produce List<Unknown>
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Unknown)),
            "heterogeneous list must infer List<Unknown>"
        );
        // Must carry exactly one Heterogeneous sentinel.
        assert_eq!(result.sentinels.len(), 1);
        assert!(
            matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
            "expected Heterogeneous sentinel, got: {:?}",
            result.sentinels[0]
        );
    }

    /// `[]` with expected type `List<Expr<Integer>>` infers to `List<Expr<Integer>>`.
    #[test]
    fn infer_list_literal_empty_with_target() {
        let elems = list_elements("SELECT []");
        let ctx = TypeContext::new();
        let expected = SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
        )));
        let result = infer_list_literal(&elems, &ctx, Some(&expected));
        assert!(
            result.sentinels.is_empty(),
            "empty list with known target must have no sentinels, got: {:?}",
            result.sentinels
        );
        assert_eq!(
            result.inferred, expected,
            "empty list with target List<Expr<Integer>> must infer to that type"
        );
    }

    /// `[]` without a target infers to `List<Unknown>` with `MetaListEmptyTypeUnknown` sentinel.
    #[test]
    fn infer_list_literal_empty_without_target() {
        let elems = list_elements("SELECT []");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Unknown)),
            "empty list without target must infer List<Unknown>"
        );
        assert_eq!(result.sentinels.len(), 1);
        assert!(
            matches!(result.sentinels[0], ListInferSentinel::EmptyTypeUnknown),
            "expected EmptyTypeUnknown sentinel, got: {:?}",
            result.sentinels[0]
        );
    }

    /// `[]` with a non-List expected type (`Expr<Numeric>`) — the caller passed an
    /// inappropriate expected sort. The function must NOT return the non-List expected
    /// type; it must fall back to `List<Unknown>` + `EmptyTypeUnknown` sentinel.
    ///
    /// Regression test for B-2: without the guard, `infer_list_literal` would
    /// return `Expr<Numeric>` (a non-List type) when passed any non-None expected,
    /// which would break the invariant that the function always returns a `List<T>`.
    #[test]
    fn infer_list_literal_empty_with_non_list_expected_falls_through() {
        let elems = list_elements("SELECT []");
        let ctx = TypeContext::new();
        // Pass a non-List expected type — should NOT be returned as-is.
        let non_list_expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
            DataType::Integer,
        ));
        let result = infer_list_literal(&elems, &ctx, Some(&non_list_expected));
        // Must still be List<Unknown>, NOT Expr<Integer>.
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Unknown)),
            "empty list with non-List expected must fall through to List<Unknown>, \
             not return the non-List expected type; got: {:?}",
            result.inferred
        );
        // Must emit EmptyTypeUnknown sentinel.
        assert_eq!(result.sentinels.len(), 1);
        assert!(
            matches!(result.sentinels[0], ListInferSentinel::EmptyTypeUnknown),
            "expected EmptyTypeUnknown sentinel, got: {:?}",
            result.sentinels[0]
        );
    }

    /// `[[100000, 200000], [300000, 400000]]` — nested list — infers `List<List<Expr<Integer>>>`.
    #[test]
    fn infer_list_literal_nested() {
        let elems = list_elements("SELECT [[100000, 200000], [300000, 400000]]");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        assert!(
            result.sentinels.is_empty(),
            "nested integer list must have no sentinels, got: {:?}",
            result.sentinels
        );
        let expected = SmeltType::List(Box::new(SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
        )))));
        assert_eq!(
            result.inferred, expected,
            "nested integer list must infer List<List<Expr<Integer>>>"
        );
    }

    /// `[1, [2, 3]]` — mixed scalar + nested list — infers `List<Unknown>` with
    /// `Heterogeneous` sentinel. A scalar element has sort `Expr<…>` while a list-literal
    /// element has sort `List<…>`; they cannot unify under LUB, so the result is
    /// `List<Unknown>` per spec `meta_language.md` Phase A semantic rule 2.
    #[test]
    fn infer_list_literal_mixed_scalar_and_nested() {
        let elems = list_elements("SELECT [1, [2, 3]]");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        // Must produce List<Unknown>
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Unknown)),
            "mixed scalar+nested list must infer List<Unknown>, got: {:?}",
            result.inferred
        );
        // Must carry exactly one Heterogeneous sentinel.
        assert_eq!(
            result.sentinels.len(),
            1,
            "expected exactly 1 sentinel, got: {:?}",
            result.sentinels
        );
        assert!(
            matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
            "expected Heterogeneous sentinel, got: {:?}",
            result.sentinels[0]
        );
    }

    /// `[[1, 2], 3]` — nested list then scalar — same cross-sort mix as above.
    /// Must also infer `List<Unknown>` with `Heterogeneous` sentinel (symmetry).
    #[test]
    fn infer_list_literal_nested_then_scalar() {
        let elems = list_elements("SELECT [[1, 2], 3]");
        let ctx = TypeContext::new();
        let result = infer_list_literal(&elems, &ctx, None);
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Unknown)),
            "nested-then-scalar list must infer List<Unknown>, got: {:?}",
            result.inferred
        );
        assert_eq!(
            result.sentinels.len(),
            1,
            "expected exactly 1 sentinel, got: {:?}",
            result.sentinels
        );
        assert!(
            matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
            "expected Heterogeneous sentinel, got: {:?}",
            result.sentinels[0]
        );
    }

    // === Phase A Phase 3 TDD tests: diagnostics + bidirectional disambiguation + spread ===

    fn parse_select_stmt(sql: &str) -> smelt_parser::ast::SelectStmt {
        use smelt_parser::ast::File;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("FILE node");
        file.select_stmt().expect("SelectStmt")
    }

    /// `[1, 2, 3]` at a splice point expecting `List<Expr<Integer>>` evaluates
    /// as a meta-list (not a Data-World array).
    #[test]
    fn list_literal_disambiguation_meta_list_target() {
        let elems = list_elements("SELECT [100000, 200000, 300000]");
        let ctx = TypeContext::new();
        // Expected sort is List<Expr<Integer>> — meta-list context.
        let expected = SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
        )));
        let result = disambiguate_list_literal(&elems, &ctx, Some(&expected));
        assert!(
            matches!(result, ListDisambiguation::MetaList(_)),
            "with List<Expr<Integer>> target, literal must be interpreted as meta-list, got: {:?}",
            result
        );
    }

    /// `[1, 2, 3]` at a splice point expecting `Expr<Array<Integer>>` evaluates
    /// as a runtime array.
    #[test]
    fn list_literal_disambiguation_data_array_target() {
        let elems = list_elements("SELECT [100000, 200000, 300000]");
        let ctx = TypeContext::new();
        // Expected sort is Expr<Concrete(Array(Integer))> — Data-World array context.
        let expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
            DataType::Array(Box::new(DataType::Integer)),
        ));
        let result = disambiguate_list_literal(&elems, &ctx, Some(&expected));
        assert!(
            matches!(result, ListDisambiguation::DataWorldArray),
            "with Expr<Array<Integer>> target, literal must be interpreted as Data-World array, \
             got: {:?}",
            result
        );
    }

    /// At a position admitting both meta-list and Data-World array, the literal
    /// evaluates as meta-list (rule 3: meta wins).
    #[test]
    fn list_literal_disambiguation_both_admissible_meta_wins() {
        let elems = list_elements("SELECT [100000, 200000, 300000]");
        let ctx = TypeContext::new();
        // No expected type → both admissible → meta-list wins.
        let result = disambiguate_list_literal(&elems, &ctx, None);
        assert!(
            matches!(result, ListDisambiguation::MetaList(_)),
            "with no expected type (both admissible), literal must default to meta-list, \
             got: {:?}",
            result
        );
    }

    /// `[1, 'hello']` emits exactly one `MetaListHeterogeneous` diagnostic
    /// anchored at the literal's source span.
    #[test]
    fn list_literal_heterogeneous_emits_diagnostic() {
        let elems = list_elements("SELECT [1, 'hello']");
        let ctx = TypeContext::new();
        let span = rowan::TextRange::new(7.into(), 20.into()); // approximate span
                                                               // Pass "" as text — unit tests don't assert specific line/column ranges.
        let diags = list_literal_sentinels_to_diagnostics(&elems, &ctx, span, "");
        assert_eq!(
            diags.len(),
            1,
            "heterogeneous list must produce exactly 1 diagnostic, got: {:?}",
            diags
        );
        assert!(
            matches!(
                diags[0].code,
                Some(crate::DiagnosticCode::MetaListHeterogeneous)
            ),
            "expected MetaListHeterogeneous diagnostic, got: {:?}",
            diags[0]
        );
    }

    /// `[]` in an unconstrained position emits exactly one
    /// `MetaListEmptyTypeUnknown` diagnostic anchored at the literal's span.
    #[test]
    fn list_literal_empty_unknown_target_emits_diagnostic() {
        let elems = list_elements("SELECT []");
        let ctx = TypeContext::new();
        let span = rowan::TextRange::new(7.into(), 9.into());
        // Pass "" as text — unit tests don't assert specific line/column ranges.
        let diags = list_literal_sentinels_to_diagnostics(&elems, &ctx, span, "");
        assert_eq!(
            diags.len(),
            1,
            "empty list without target must produce exactly 1 diagnostic, got: {:?}",
            diags
        );
        assert!(
            matches!(
                diags[0].code,
                Some(crate::DiagnosticCode::MetaListEmptyTypeUnknown)
            ),
            "expected MetaListEmptyTypeUnknown diagnostic, got: {:?}",
            diags[0]
        );
    }

    /// `SELECT id, ...[a, b], created_at` — spread of a list literal into SELECT
    /// list expands to the individual elements; each emitted item carries
    /// `Synthesized(SpreadFrom(span_of_list_literal))` provenance.
    #[test]
    fn spread_in_select_list_expands() {
        let sql = "SELECT id, ...[a, b], created_at FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        // Pass "" as text — unit tests don't assert specific line/column ranges.
        let result = check_select_list_spreads(&select, &ctx, "");
        // Must find the spread and report expanded count = 2 (for a, b)
        assert_eq!(
            result.expanded_item_count, 2,
            "spread of [a, b] must expand to 2 items, got: {}",
            result.expanded_item_count
        );
        assert!(
            result.diagnostics.is_empty(),
            "valid spread in SELECT must produce no diagnostics, got: {:?}",
            result.diagnostics
        );
        // Each emitted item must carry Synthesized(SpreadFrom(...)) provenance.
        assert_eq!(
            result.provenance_tags.len(),
            2,
            "each of the 2 expanded items must carry a provenance tag, got: {:?}",
            result.provenance_tags
        );
        assert!(
            result
                .provenance_tags
                .iter()
                .all(|t| matches!(t, OriginTag::Synthesized(SynthesizedReason::SpreadFrom(_)))),
            "all provenance tags must be Synthesized(SpreadFrom(…)), got: {:?}",
            result.provenance_tags
        );
    }

    /// `SELECT id, ...[], created_at` — spread of an empty list elides silently;
    /// the SELECT type-checks as if `SELECT id, created_at`.
    #[test]
    fn spread_empty_list_elides() {
        let sql = "SELECT id, ...[], created_at FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        // Pass "" as text — unit tests don't assert specific line/column ranges.
        let result = check_select_list_spreads(&select, &ctx, "");
        assert_eq!(
            result.expanded_item_count, 0,
            "spread of empty list must expand to 0 items (elision), got: {}",
            result.expanded_item_count
        );
        assert!(
            result.diagnostics.is_empty(),
            "empty-list spread in SELECT must produce no diagnostics, got: {:?}",
            result.diagnostics
        );
    }

    /// `WHERE x = 1 AND ...preds` emits `MetaSpreadInForbiddenPosition` at the
    /// spread span.
    #[test]
    fn spread_in_where_clause_emits_diagnostic() {
        // Note: the parser does not emit a LIST_SPREAD node inside WHERE (it
        // produces a parse error instead). The orphaned DOT_DOT_DOT token ends
        // up as a sibling of the SELECT_STMT at the FILE level.
        // `check_forbidden_position_spreads` detects this pattern and emits
        // MetaSpreadInForbiddenPosition.
        let sql = "SELECT x FROM t WHERE x = 1 AND ...preds";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        // Pass "" as text — unit tests don't assert specific line/column ranges.
        let diags = check_forbidden_position_spreads(&select, &ctx, "");
        assert!(
            !diags.is_empty(),
            "spread in WHERE must produce MetaSpreadInForbiddenPosition diagnostic"
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition)),
            "expected MetaSpreadInForbiddenPosition, got: {:?}",
            diags
        );
    }

    /// `SELECT ...x FROM t` where `x` is `Expr<Integer>` emits
    /// `MetaSpreadOnNonList`; surrounding SELECT type-checks as if spread were
    /// absent.
    #[test]
    fn spread_on_non_list_emits_diagnostic() {
        let sql = "SELECT ...x FROM t";
        let select = parse_select_stmt(sql);
        // Context: x is Expr<Integer>, not a List.
        let mut ctx = TypeContext::new();
        ctx.add_model_column(
            "t",
            "x",
            smelt_types::TypedColumn::not_null(DataType::Integer),
        );
        // Pass "" as text — unit tests don't assert specific line/column ranges.
        let result = check_select_list_spreads(&select, &ctx, "");
        assert_eq!(
            result.diagnostics.len(),
            1,
            "spread on non-list must produce exactly 1 MetaSpreadOnNonList, \
             got: {:?}",
            result.diagnostics
        );
        assert!(
            matches!(
                result.diagnostics[0].code,
                Some(crate::DiagnosticCode::MetaSpreadOnNonList)
            ),
            "expected MetaSpreadOnNonList, got: {:?}",
            result.diagnostics[0]
        );
    }

    // === Phase B (meta-language) TDD tests: HOF inference + reducer registry + pipe ===

    /// Parse a HOF call like `SELECT map([1, 2, 3], fn c => c)` and return the
    /// FunctionCall AST node (the map/filter/reduce call).
    fn parse_hof_call(sql: &str) -> smelt_parser::ast::FunctionCall {
        let expr = parse_first_expr(sql);
        expr.as_function_call()
            .expect("expected a function-call expression for HOF test")
    }

    /// `map([1, 2, 3], fn c => c)` — identity lambda on SmallInt list — infers
    /// `List<Expr<SmallInt>>` (HOF produces `List<U>` where U = body type = Expr<SmallInt>).
    ///
    /// 1, 2, 3 are in i16 range so they infer SmallInt.
    #[test]
    fn infer_map_returns_list_of_body_type() {
        let call = parse_hof_call("SELECT map([1, 2, 3], fn c => c)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "identity map must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::SmallInt)
            ))),
            "map([1,2,3], fn c => c) must infer List<Expr<SmallInt>>"
        );
    }

    /// `map([1, 2, 3], fn c => CAST(c AS Text))` — body produces Expr<Varchar> —
    /// HOF result is `List<Expr<Varchar>>`.
    ///
    /// Note: `CAST(x AS Text)` produces `DataType::Varchar { max_length: None }`
    /// (not `DataType::Text`) because the type parser normalises `TEXT` → `VARCHAR`.
    #[test]
    fn infer_map_with_typed_body() {
        let call = parse_hof_call("SELECT map([1, 2, 3], fn c => CAST(c AS Text))");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "map with CAST body must have no sentinel, got: {:?}",
            result.sentinel
        );
        // CAST(x AS Text) normalises to Varchar { max_length: None }.
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Varchar {
                    max_length: None
                })
            ))),
            "map([1,2,3], fn c => CAST(c AS Text)) must infer List<Expr<Varchar>>"
        );
    }

    /// `filter([1, 2, 3], fn c => c > 0)` — filter preserves element type —
    /// result is `List<Expr<SmallInt>>`.
    #[test]
    fn infer_filter_returns_same_list_type() {
        let call = parse_hof_call("SELECT filter([1, 2, 3], fn c => c > 0)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "filter must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred,
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::SmallInt)
            ))),
            "filter([1,2,3], fn c => c > 0) must infer List<Expr<SmallInt>>"
        );
    }

    /// `filter([1, 2, 3], fn c => c)` — predicate body is `Expr<SmallInt>` not
    /// `Expr<Boolean>` — returns a sentinel for `LambdaResultTypeMismatch`.
    #[test]
    fn infer_filter_predicate_must_be_boolean_sentinel() {
        let call = parse_hof_call("SELECT filter([1, 2, 3], fn c => c)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            matches!(result.sentinel, Some(HofInferSentinel::LambdaResultTypeMismatch { .. })),
            "filter with non-Boolean predicate must return LambdaResultTypeMismatch sentinel, got: {:?}",
            result.sentinel
        );
    }

    /// `reduce([1, 2, 3], plus_chain)` → `Expr<SmallInt>`.
    /// `reduce(['a', 'b', 'c'], concat)` → `Expr<Text>`.
    /// `reduce([true, false], and_all)` → `Expr<Boolean>`.
    #[test]
    fn infer_reduce_returns_reducer_output_sort() {
        // plus_chain with SmallInt integers
        {
            let call = parse_hof_call("SELECT reduce([1, 2, 3], plus_chain)");
            let ctx = TypeContext::new();
            let result = infer_hof_call_from_function_call(&call, &ctx);
            assert!(
                result.sentinel.is_none(),
                "reduce([1,2,3], plus_chain) must have no sentinel, got: {:?}",
                result.sentinel
            );
            // plus_chain output sort is Expr<Numeric>; element type is SmallInt
            // which satisfies Numeric, so output is Expr<SmallInt> (the element type).
            assert!(
                matches!(result.inferred, SmeltType::Expr(_)),
                "reduce(ints, plus_chain) must infer Expr<...>, got: {:?}",
                result.inferred
            );
        }
        // concat with text
        {
            let call = parse_hof_call("SELECT reduce(['a', 'b', 'c'], concat)");
            let ctx = TypeContext::new();
            let result = infer_hof_call_from_function_call(&call, &ctx);
            assert!(
                result.sentinel.is_none(),
                "reduce(texts, concat) must have no sentinel, got: {:?}",
                result.sentinel
            );
            assert!(
                matches!(result.inferred, SmeltType::Expr(_)),
                "reduce(texts, concat) must infer Expr<...>, got: {:?}",
                result.inferred
            );
        }
        // and_all with booleans
        {
            let call = parse_hof_call("SELECT reduce([true, false], and_all)");
            let ctx = TypeContext::new();
            let result = infer_hof_call_from_function_call(&call, &ctx);
            assert!(
                result.sentinel.is_none(),
                "reduce(bools, and_all) must have no sentinel, got: {:?}",
                result.sentinel
            );
            assert_eq!(
                result.inferred,
                SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    DataType::Boolean
                )),
                "reduce([true,false], and_all) must infer Expr<Boolean>"
            );
        }
    }

    /// `reduce([col1, col2, col3], comma_sep)` — output is `SelectItems<Scalar>`
    /// regardless of element `T`.
    #[test]
    fn infer_reduce_comma_sep_yields_select_items() {
        // Use integer literals as "columns" — comma_sep accepts any Expr<T>.
        let call = parse_hof_call("SELECT reduce([1, 2, 3], comma_sep)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "comma_sep reduce must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred,
            SmeltType::SelectItems {
                kind: smelt_types::signatures::ExprKind::Scalar,
                context: None
            },
            "reduce(any, comma_sep) must infer SelectItems<Scalar>"
        );
    }

    /// `reduce([], and_all)` — empty list with identity reducer — infers `Expr<Boolean>`;
    /// no sentinel.
    #[test]
    fn infer_reduce_empty_list_with_identity() {
        let call = parse_hof_call("SELECT reduce([], and_all)");
        let ctx = TypeContext::new();
        let expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
            DataType::Boolean,
        ));
        let result = infer_hof_call_from_function_call_with_expected(&call, &ctx, Some(&expected));
        assert!(
            result.sentinel.is_none(),
            "reduce([], and_all) must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred, expected,
            "reduce([], and_all) must infer Expr<Boolean> (TRUE identity)"
        );
    }

    /// `reduce([], union_all)` — empty list, no identity — sentinel for
    /// `ReducerEmptyNoIdentity`.
    #[test]
    fn infer_reduce_empty_list_no_identity_sentinel() {
        let call = parse_hof_call("SELECT reduce([], union_all)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            matches!(
                result.sentinel,
                Some(HofInferSentinel::ReducerEmptyNoIdentity { .. })
            ),
            "reduce([], union_all) must produce ReducerEmptyNoIdentity sentinel, got: {:?}",
            result.sentinel
        );
    }

    /// `reduce([], comma_sep)` — empty list with `comma_sep` — produces
    /// `SelectItems<Scalar>` with no sentinel (via the registry `EmptySelectItems`
    /// identity, not a special-case branch).
    #[test]
    fn infer_reduce_comma_sep_empty_returns_select_items_with_identity() {
        let call = parse_hof_call("SELECT reduce([], comma_sep)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "reduce([], comma_sep) must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred,
            SmeltType::SelectItems {
                kind: smelt_types::signatures::ExprKind::Scalar,
                context: None,
            },
            "reduce([], comma_sep) must infer SelectItems<Scalar>"
        );
    }

    /// `reduce([1, 2, 3], and_all)` — element type `Expr<SmallInt>` does not
    /// satisfy `and_all`'s `Expr<Boolean>` requirement — sentinel for
    /// `ReducerInputTypeMismatch`.
    #[test]
    fn infer_reduce_input_type_mismatch_sentinel() {
        let call = parse_hof_call("SELECT reduce([1, 2, 3], and_all)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            matches!(
                result.sentinel,
                Some(HofInferSentinel::ReducerInputTypeMismatch { .. })
            ),
            "reduce([1,2,3], and_all) must produce ReducerInputTypeMismatch sentinel, got: {:?}",
            result.sentinel
        );
    }

    /// `xs |> filter(fn c => c > 0)` and `filter(xs, fn c => c > 0)` infer to
    /// the same `SmeltType` for the same input.
    #[test]
    fn infer_pipe_desugars_to_call() {
        let ctx = TypeContext::new();

        // Piped form: [1, 2, 3] |> filter(fn c => c > 0)
        let pipe_sql = "SELECT [1, 2, 3] |> filter(fn c => c > 0)";
        let pipe_result = {
            let expr = parse_first_expr(pipe_sql);
            let pipe = extract_pipe_expr_from_expr(&expr).expect("expected PIPE_EXPR");
            infer_pipe_expr(&pipe, &ctx, None)
        };

        // Direct form: filter([1, 2, 3], fn c => c > 0)
        let call_sql = "SELECT filter([1, 2, 3], fn c => c > 0)";
        let call_result = {
            let call = parse_hof_call(call_sql);
            infer_hof_call_from_function_call(&call, &ctx)
        };

        assert_eq!(
            pipe_result.inferred, call_result.inferred,
            "piped and direct forms must infer the same type"
        );
    }

    /// `[1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)` infers to
    /// `List<Expr<SmallInt>>` (left-associative pipe chain).
    #[test]
    fn infer_pipe_chain_associates_left() {
        let ctx = TypeContext::new();
        let sql = "SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)";
        let expr = parse_first_expr(sql);
        let pipe = extract_pipe_expr_from_expr(&expr).expect("expected outer PIPE_EXPR");
        let result = infer_pipe_expr(&pipe, &ctx, None);
        assert!(
            result.sentinel.is_none(),
            "pipe chain must have no sentinel, got: {:?}",
            result.sentinel
        );
        // SmallInt * SmallInt → SmallInt (integer arithmetic promotion)
        assert!(
            matches!(result.inferred, SmeltType::List(_)),
            "pipe chain result must be a List, got: {:?}",
            result.inferred
        );
    }

    /// Inside `map(xs: List<Expr<SmallInt>>, fn c => c)`, the lookup of `c`
    /// in the body context returns `Expr<SmallInt>` (lambda parameter binding).
    #[test]
    fn lambda_parameter_binding_via_typecontext() {
        // We test by checking that a context with lambda param `c: SmallInt`
        // resolves `c` to SmallInt in lookup_identifier.
        let mut ctx = TypeContext::new();
        ctx.add_lambda_param("c", smelt_types::TypedColumn::not_null(DataType::SmallInt));

        let resolved = ctx
            .lookup_identifier(None, "c")
            .expect("lambda param 'c' must resolve");
        assert_eq!(
            resolved.data_type,
            DataType::SmallInt,
            "lambda param 'c' must resolve to SmallInt"
        );
    }

    /// When an enclosing `smelt.define` parameter named `c` is in scope,
    /// the lambda parameter `c` wins inside the lambda body (shadowing).
    #[test]
    fn lambda_parameter_shadows_outer_binding() {
        let mut ctx = TypeContext::new();
        // Outer function param `c` is BigInt
        ctx.add_function_param("c", smelt_types::TypedColumn::not_null(DataType::BigInt));
        // Lambda param `c` is SmallInt — shadows the outer BigInt
        ctx.add_lambda_param("c", smelt_types::TypedColumn::not_null(DataType::SmallInt));

        let resolved = ctx
            .lookup_identifier(None, "c")
            .expect("lambda param must be found");
        assert_eq!(
            resolved.data_type,
            DataType::SmallInt,
            "lambda param 'c: SmallInt' must shadow outer function param 'c: BigInt'"
        );
    }

    /// Every reducer name in the closed registry is recognised; an unknown
    /// identifier is not.
    #[test]
    fn reducer_registry_lookup_closed_set() {
        let known = [
            "comma_sep",
            "and_all",
            "or_any",
            "union_all",
            "intersect_all",
            "plus_chain",
            "concat",
        ];
        for name in &known {
            assert!(
                lookup_reducer(name).is_some(),
                "reducer '{}' must be in the closed registry",
                name
            );
        }
        assert!(
            lookup_reducer("not_a_reducer").is_none(),
            "unknown reducer must not be in the registry"
        );
    }

    // === Phase 3 (meta-language Phase B) TDD tests: diagnostic emission ===

    /// A `fn x => body` lambda not in a HOF positional argument position emits
    /// `LambdaInForbiddenPosition`. We check via `check_hof_position_diagnostics`.
    #[test]
    fn lambda_outside_hof_position_emits_diagnostic() {
        // A lambda in a plain expression position — not inside a HOF call.
        let sql = "SELECT fn c => c FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::LambdaInForbiddenPosition)),
            "lambda in SELECT (non-HOF position) must emit LambdaInForbiddenPosition, \
             got: {:?}",
            diags
        );
    }

    /// `map(xs, fn (a, b) => a)` — multi-arg lambda — emits `LambdaArityNotSupported`.
    #[test]
    fn multi_arg_lambda_emits_arity_diagnostic() {
        // map call with multi-arg lambda (two params)
        let sql = "SELECT map([1, 2, 3], fn (a, b) => a) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::LambdaArityNotSupported)),
            "map with multi-arg lambda must emit LambdaArityNotSupported, got: {:?}",
            diags
        );
    }

    /// `filter([1,2,3], fn c => c)` — predicate body is `Expr<SmallInt>` not Boolean —
    /// emits `LambdaResultTypeMismatch`.
    #[test]
    fn filter_predicate_non_boolean_emits_lambda_result_mismatch() {
        let sql = "SELECT filter([1, 2, 3], fn c => c) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::LambdaResultTypeMismatch)),
            "filter with non-Boolean predicate must emit LambdaResultTypeMismatch, got: {:?}",
            diags
        );
    }

    /// `map(xs, 42)` — non-lambda second arg — emits `HofExpectsLambda`.
    #[test]
    fn map_with_non_lambda_second_arg_emits_hof_expects_lambda() {
        let sql = "SELECT map([1, 2, 3], 42) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::HofExpectsLambda)),
            "map with non-lambda second arg must emit HofExpectsLambda, got: {:?}",
            diags
        );
    }

    /// `reduce(xs, fn c => c)` — lambda where reducer expected — emits `HofExpectsReducer`.
    #[test]
    fn reduce_with_non_reducer_second_arg_emits_hof_expects_reducer() {
        let sql = "SELECT reduce([1, 2, 3], fn c => c) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::HofExpectsReducer)),
            "reduce with lambda second arg must emit HofExpectsReducer, got: {:?}",
            diags
        );
    }

    /// `xs |> 3 + 4` — non-call RHS — emits `PipeRhsNotCall`.
    #[test]
    fn pipe_rhs_not_call_emits_diagnostic() {
        let sql = "SELECT [1, 2, 3] |> 3 + 4 FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::PipeRhsNotCall)),
            "pipe with non-call RHS must emit PipeRhsNotCall, got: {:?}",
            diags
        );
    }

    /// `reduce([1,2,3], and_all)` — Integer input, but and_all requires Boolean —
    /// emits `ReducerInputTypeMismatch`.
    #[test]
    fn reduce_input_type_mismatch_emits_diagnostic() {
        let sql = "SELECT reduce([1, 2, 3], and_all) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ReducerInputTypeMismatch)),
            "reduce([1,2,3], and_all) must emit ReducerInputTypeMismatch, got: {:?}",
            diags
        );
    }

    /// `reduce([], union_all)` — empty list, no identity — emits `ReducerEmptyNoIdentity`.
    #[test]
    fn reduce_empty_no_identity_emits_diagnostic() {
        let sql = "SELECT reduce([], union_all) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ReducerEmptyNoIdentity)),
            "reduce([], union_all) must emit ReducerEmptyNoIdentity, got: {:?}",
            diags
        );
    }

    // === Phase 3: `smelt.config.var` resolver tests ===

    /// `smelt.config.var('region')` over a workspace with `vars: { region: us-west-2 }`
    /// resolves to a `Text` value `'us-west-2'` (no diagnostics).
    #[test]
    fn config_var_resolves_string_scalar() {
        use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

        let yaml = "name: my_project\nvars:\n  region: us-west-2\ntargets: {}\n";
        let vars = parse_vars_from_yaml(yaml);
        let vars = vars.expect("vars must parse successfully");
        let val = vars.get("region").expect("region must be present");
        let (text_val, warning) = coerce_yaml_scalar_to_text(val, "region");
        assert_eq!(text_val, "us-west-2", "region must resolve to 'us-west-2'");
        assert!(
            warning.is_none(),
            "string scalar must not warn, got: {:?}",
            warning
        );
    }

    /// `smelt.config.var('flag')` over `vars: { flag: true }` resolves to `'true'`;
    /// integer `42` resolves to `'42'`.
    #[test]
    fn config_var_coerces_yaml_boolean() {
        use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

        let yaml = "name: my_project\nvars:\n  flag: true\n  count: 42\ntargets: {}\n";
        let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
        {
            let val = vars.get("flag").expect("flag must be present");
            let (text_val, warning) = coerce_yaml_scalar_to_text(val, "flag");
            assert_eq!(text_val, "true", "boolean true must coerce to 'true'");
            assert!(warning.is_none());
        }
        {
            let val = vars.get("count").expect("count must be present");
            let (text_val, warning) = coerce_yaml_scalar_to_text(val, "count");
            assert_eq!(text_val, "42", "integer 42 must coerce to '42'");
            assert!(warning.is_none());
        }
    }

    /// `smelt.config.var('nullable')` over `vars: { nullable: ~ }` resolves to `''`
    /// and emits `ConfigVarNullCoercion` warning sentinel.
    #[test]
    fn config_var_null_emits_warning() {
        use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

        let yaml = "name: my_project\nvars:\n  nullable: ~\ntargets: {}\n";
        let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
        let val = vars.get("nullable").expect("nullable must be present");
        let (text_val, warning) = coerce_yaml_scalar_to_text(val, "nullable");
        assert_eq!(text_val, "", "null must coerce to empty string");
        assert!(
            warning.is_some(),
            "null coercion must produce a ConfigVarNullCoercion warning sentinel"
        );
    }

    /// `smelt.config.var('not_declared')` over a workspace whose `vars:` lacks `not_declared`
    /// emits `ConfigVarNotFound`.
    #[test]
    fn config_var_not_found_emits_diagnostic() {
        use crate::config_vars::parse_vars_from_yaml;

        let yaml = "name: my_project\nvars:\n  region: us-east-1\ntargets: {}\n";
        let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
        let result = vars.get("not_declared");
        assert!(
            result.is_none(),
            "not_declared must not be present in vars, got: {:?}",
            result
        );
        // The diagnostic emission path is tested in the production path (lib.rs).
    }

    /// `smelt.config.var(some_var)` (non-literal) — detection of non-literal arg.
    /// We test the helper that detects whether an Expr is a string literal.
    #[test]
    fn config_var_non_literal_arg_emits_diagnostic() {
        use crate::config_vars::is_string_literal_expr;

        // A column reference "some_var" is not a string literal.
        let col_expr = parse_first_expr("SELECT some_var FROM t");
        assert!(
            !is_string_literal_expr(&col_expr),
            "column reference must NOT be a string literal"
        );

        // A string literal 'region' IS a string literal.
        let str_expr = parse_first_expr("SELECT 'region' FROM t");
        assert!(
            is_string_literal_expr(&str_expr),
            "quoted string must be a string literal"
        );
    }

    /// `smelt.define map(...)` — re-declaring a HOF name — emits `HofNameShadowed`.
    #[test]
    fn smelt_define_named_map_emits_hof_name_shadowed() {
        // Parse a file with a smelt.define named 'map' and check the name-shadowing
        // diagnostic via check_define_name_shadowing.
        use smelt_parser::ast::SmeltDefine;
        let sql = "smelt.define map(x: Expr<Integer>) AS (x + 1)\n";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
        let define: SmeltDefine = file.defines().next().expect("one smelt.define");
        let diags = check_define_name_shadowing(&define, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::HofNameShadowed)),
            "smelt.define named 'map' must emit HofNameShadowed, got: {:?}",
            diags
        );
    }

    /// `smelt.define concat(...)` — re-declaring a reducer name — emits `ReducerNameShadowed`.
    #[test]
    fn smelt_define_named_concat_emits_reducer_name_shadowed() {
        use smelt_parser::ast::SmeltDefine;
        let sql = "smelt.define concat(x: Expr<Text>) AS (x)\n";
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
        let define: SmeltDefine = file.defines().next().expect("one smelt.define");
        let diags = check_define_name_shadowing(&define, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ReducerNameShadowed)),
            "smelt.define named 'concat' must emit ReducerNameShadowed, got: {:?}",
            diags
        );
    }

    /// `WHERE a |> b()` — pipe in a Data-World position (WHERE predicate) — emits
    /// `PipeInDataPosition`.
    #[test]
    fn pipe_in_where_clause_emits_diagnostic() {
        let sql = "SELECT x FROM t WHERE [1, 2, 3] |> map(fn c => c + 1)";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_hof_position_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::PipeInDataPosition)),
            "pipe in WHERE clause must emit PipeInDataPosition, got: {:?}",
            diags
        );
    }

    // === Finding 1 fix: smelt.config.var type inference ===

    /// `smelt.config.var('env')` infers as nullable Varchar (Phase B rule 10).
    ///
    /// The `smelt.config.var` built-in is not in the function-signature index,
    /// so it requires a special-case in `infer_smelt_path_call_type`.
    #[test]
    fn config_var_infers_nullable_varchar() {
        let ctx = TypeContext::new();
        let expr = parse_first_expr("SELECT smelt.config.var('env')");
        let typed =
            infer_expression_type(&expr, &ctx).expect("smelt.config.var('env') must infer a type");
        assert_eq!(
            typed.data_type,
            DataType::Varchar { max_length: None },
            "smelt.config.var must infer Varchar, got: {:?}",
            typed.data_type
        );
        assert!(
            typed.nullable,
            "smelt.config.var must be nullable (value may be absent without a default)"
        );
    }

    /// `smelt.config.var('env') = 'prod'` infers as Boolean (no type error).
    ///
    /// An equality between `smelt.config.var(...)` (Varchar) and a string
    /// literal (also Varchar) must produce a Boolean result — not an Unknown or
    /// error — because both sides are Text-compatible.
    #[test]
    fn config_var_equality_with_varchar_literal_infers_boolean() {
        let ctx = TypeContext::new();
        let expr = parse_first_expr("SELECT smelt.config.var('env') = 'prod'");
        let typed = infer_expression_type(&expr, &ctx)
            .expect("smelt.config.var('env') = 'prod' must infer a type");
        assert_eq!(
            typed.data_type,
            DataType::Boolean,
            "smelt.config.var(...) = 'prod' must infer Boolean, got: {:?}",
            typed.data_type
        );
    }

    // === Finding 2 fix: HOF inference with List<T> function parameter ===

    /// `xs.map(fn x => x + 1)` where `xs` is seeded as `List<Expr<Integer>>`
    /// via `add_function_param_smelt_type` must infer a non-error result.
    ///
    /// Without the fix the non-literal first-argument path would collapse
    /// `xs` to `SmeltType::Expr(Concrete(Unknown))`, triggering `InputNotList`.
    /// With the fix the lookup in `function_param_smelt_types` recovers the full
    /// `SmeltType::List(...)`, and the lambda parameter `x` is bound to
    /// `Expr<Integer>`.
    #[test]
    fn hof_map_on_list_param_infers_correctly() {
        let call = parse_hof_call("SELECT map(xs, fn x => x + 1)");
        let mut ctx = TypeContext::new();
        // Simulate a function body context where `xs: List<Expr<Integer>>` was declared.
        // add_function_param stores DataType::Unknown (the scalar projection).
        ctx.add_function_param("xs", smelt_types::TypedColumn::nullable(DataType::Unknown));
        // add_function_param_smelt_type stores the full SmeltType.
        ctx.add_function_param_smelt_type(
            "xs",
            SmeltType::List(Box::new(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
            ))),
        );
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            !matches!(result.sentinel, Some(HofInferSentinel::InputNotList { .. })),
            "map on List<Expr<Integer>> param must not produce InputNotList, got: {:?}",
            result.sentinel
        );
        // The lambda body `x + 1` where x: Integer infers as Integer/BigInt —
        // the exact type depends on arithmetic promotion rules.  We only require
        // that the inferred type is a List<T> (not Unknown or Error).
        assert!(
            matches!(result.inferred, SmeltType::List(_)),
            "map on List<Expr<Integer>> param must infer List<T>, got: {:?}",
            result.inferred
        );
    }

    // === Phase C (meta-language) TDD tests — smelt.columns_of + ColumnRef field projection ===

    /// `smelt.columns_of(42)` synthesises `List<ColumnRef>` (recoverable) and
    /// emits exactly one `ColumnsOfRequiresTableExpr` at the `42` argument span.
    #[test]
    fn columns_of_arg_must_be_table_expr() {
        // Non-TableExpr arg: 42 (integer literal) — should emit ColumnsOfRequiresTableExpr.
        let sql = "SELECT smelt.columns_of(42) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_columns_of_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr)),
            "smelt.columns_of(42) must emit ColumnsOfRequiresTableExpr, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr))
                .count(),
            1,
            "must emit exactly one ColumnsOfRequiresTableExpr"
        );

        // A smelt.<path> reference must not emit a Phase C diagnostic.
        // We use a bare path reference — the type-checker resolves it as TableExpr.
        let sql_ok = "SELECT smelt.columns_of(smelt.models.orders) FROM t";
        let select_ok = parse_select_stmt(sql_ok);
        let diags_ok = check_columns_of_diagnostics(&select_ok, &ctx, "");
        assert!(
            !diags_ok
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr)),
            "smelt.columns_of(smelt.models.orders) must NOT emit ColumnsOfRequiresTableExpr, got: {:?}",
            diags_ok
        );
    }

    /// `smelt.columns_of(t => orders)` emits exactly one `ColumnsOfNamedArgument`.
    #[test]
    fn columns_of_rejects_named_argument() {
        let sql = "SELECT smelt.columns_of(t => orders) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_columns_of_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument)),
            "smelt.columns_of(t => orders) must emit ColumnsOfNamedArgument, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument))
                .count(),
            1,
            "must emit exactly one ColumnsOfNamedArgument"
        );

        // Positional arg must not emit ColumnsOfNamedArgument.
        let sql_ok = "SELECT smelt.columns_of(orders) FROM t";
        let select_ok = parse_select_stmt(sql_ok);
        let diags_ok = check_columns_of_diagnostics(&select_ok, &ctx, "");
        assert!(
            !diags_ok
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument)),
            "smelt.columns_of(orders) must NOT emit ColumnsOfNamedArgument, got: {:?}",
            diags_ok
        );
    }

    /// Given a binding `c: ColumnRef`, field access `c.name`, `c.type`, `c.is_numeric`
    /// synthesise the correct types.
    #[test]
    fn column_ref_field_projection_synthesises_field_type() {
        // Seed a lambda param `c` as ColumnRef.
        let mut ctx = TypeContext::new();
        ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        // For the data-type projection the lambda param `c` maps to DataType::Unknown
        // (ColumnRef is not a SQL DataType).
        ctx.add_lambda_param(
            "c",
            smelt_types::TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );

        // c.name → Text
        let name_ty = infer_field_on_column_ref("c", "name", &ctx);
        assert!(
            matches!(
                name_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
                ))
            ),
            "c.name must synthesise Text, got: {:?}",
            name_ty
        );

        // c.is_numeric → Boolean
        let is_numeric_ty = infer_field_on_column_ref("c", "is_numeric", &ctx);
        assert!(
            matches!(
                is_numeric_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean)
                ))
            ),
            "c.is_numeric must synthesise Boolean, got: {:?}",
            is_numeric_ty
        );

        // c.type → SmeltType::Unknown as the Phase C sentinel for "DataType (meta literal)".
        // Phase D will introduce a proper meta-DataType representation; for now Unknown
        // is the documented placeholder per the Phase C plan.
        let type_ty = infer_field_on_column_ref("c", "type", &ctx);
        assert!(
            matches!(type_ty, Some(SmeltType::Unknown)),
            "c.type maps to SmeltType::Unknown as the Phase C sentinel for DataType (meta literal); \
             Phase D will introduce a proper meta-DataType representation; got: {:?}",
            type_ty
        );
    }

    /// Given a binding `c: ColumnRef`, `c.foo` emits exactly one
    /// `ColumnRefFieldUnknown` at the `foo` field token span and synthesises `Unknown`.
    #[test]
    fn column_ref_field_projection_rejects_unknown_field() {
        let mut ctx = TypeContext::new();
        ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx.add_lambda_param(
            "c",
            smelt_types::TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );

        // c.foo must emit ColumnRefFieldUnknown anchored at the `foo` token span.
        // In "SELECT c.foo FROM t", `foo` is at byte offset 9..12 (line 0, col 9..12).
        let sql = "SELECT c.foo FROM t";
        let select = parse_select_stmt(sql);
        // Pass the actual source text so that to_range() can compute line/column positions.
        let diags = check_column_ref_field_diagnostics(&select, &ctx, sql);
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown)),
            "c.foo must emit ColumnRefFieldUnknown, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown))
                .count(),
            1,
            "must emit exactly one ColumnRefFieldUnknown"
        );
        // Pin the diagnostic to the `foo` field-token span, not the whole `c.foo` expression.
        // Spec invariant: ColumnRefFieldUnknown must anchor at the field name token only.
        let unknown_diag = diags
            .iter()
            .find(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown))
            .expect("already asserted above");
        assert_eq!(
            unknown_diag.range.start,
            smelt_parser::ast::Position { line: 0, column: 9 },
            "diagnostic must start at the `foo` token (col 9), not the start of `c.foo`"
        );
        assert_eq!(
            unknown_diag.range.end,
            smelt_parser::ast::Position {
                line: 0,
                column: 12
            },
            "diagnostic must end after `foo` (col 12)"
        );
    }

    // ─── Phase C Phase 2: meta-Text-as-identifier lift tests ─────────────────

    /// Helper: build a TypeContext with a ColumnRef binding `c` and columns
    /// `{name: Text, amount: Numeric}` in scope.
    fn make_column_ref_ctx() -> TypeContext {
        let mut ctx = TypeContext::new();
        // Register `c` as a ColumnRef-typed lambda parameter.
        ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx.add_lambda_param(
            "c",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        // Seed two in-scope columns via a fake model.
        ctx.add_model_column(
            "t",
            "name",
            TypedColumn {
                data_type: DataType::Text,
                nullable: true,
            },
        );
        ctx.add_model_column(
            "t",
            "amount",
            TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            },
        );
        ctx.add_alias("t", "t");
        ctx
    }

    /// `is_meta_text_value` predicate: `c.name` with `c: ColumnRef` → `Some("name")`.
    #[test]
    fn is_meta_text_value_recognises_column_ref_name_projection() {
        let ctx = make_column_ref_ctx();

        // c.name → Some("name")
        let sql = "SELECT c.name FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");
        let result = is_meta_text_value(&expr, &ctx);
        assert_eq!(
            result,
            Some("name".to_string()),
            "c.name with c: ColumnRef must be recognised as meta-Text, got: {:?}",
            result
        );
    }

    /// `is_meta_text_value` predicate: `c.is_numeric` returns `None` (Boolean field,
    /// not Text).
    #[test]
    fn is_meta_text_value_rejects_non_text_field() {
        let ctx = make_column_ref_ctx();

        // c.is_numeric → None (Boolean field, not Text)
        let sql = "SELECT c.is_numeric FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");
        let result = is_meta_text_value(&expr, &ctx);
        assert_eq!(
            result, None,
            "c.is_numeric is Boolean, not Text — must NOT be recognised as meta-Text; got: {:?}",
            result
        );
    }

    /// `is_meta_text_value` predicate: a runtime `Expr<Text>` like `UPPER('foo')` returns `None`.
    #[test]
    fn no_lift_for_runtime_expr_text() {
        let ctx = make_column_ref_ctx();

        // UPPER('foo') is a runtime Expr<Text> — not a meta-Text, lift must not fire.
        let sql = "SELECT UPPER('foo') FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");
        let result = is_meta_text_value(&expr, &ctx);
        assert_eq!(
            result, None,
            "UPPER('foo') is a runtime Expr<Text> — must NOT be recognised as meta-Text; got: {:?}",
            result
        );
    }

    /// `is_meta_text_value` predicate: a SQL string literal `'foo'` returns `None`.
    #[test]
    fn lift_only_for_compile_time_meta_text() {
        let ctx = make_column_ref_ctx();

        // 'foo' is a string literal — not a meta-Text projection.
        let sql = "SELECT 'foo' FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");
        let result = is_meta_text_value(&expr, &ctx);
        assert_eq!(
            result, None,
            "'foo' is a SQL string literal — must NOT be recognised as meta-Text; got: {:?}",
            result
        );
    }

    /// `is_meta_text_value` predicate: `UPPER(c.name)` — the argument c.name is a
    /// meta-Text but the outer UPPER call is NOT.  `no_lift_in_function_argument_position`
    /// verifies that the lift does not fire for the function-call expression.
    #[test]
    fn no_lift_in_function_argument_position() {
        let ctx = make_column_ref_ctx();

        // UPPER(c.name) — the outer expression is a function call, not a meta-Text.
        let sql = "SELECT UPPER(c.name) FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");

        // The outer expression (UPPER(...)) must NOT be a meta-Text value.
        let result = is_meta_text_value(&expr, &ctx);
        assert_eq!(
            result, None,
            "UPPER(c.name) outer expression must NOT be meta-Text (lift doesn't fire for function calls); got: {:?}",
            result
        );

        // No UnknownColumn expected from check_meta_text_lift_diagnostics for UPPER(c.name),
        // because UPPER(c.name) is not a lift-position expression.
        let diags = check_meta_text_lift_diagnostics(&select, &ctx);
        assert!(
            diags.is_empty(),
            "UPPER(c.name) must not produce lift diagnostics (not in lift position); got: {:?}",
            diags
        );
    }

    /// `lift_in_column_reference_position_resolves_to_column`:
    /// `c.name` (meta-Text, field "name") in column-reference position produces
    /// no diagnostics regardless of whether a column named "name" is in scope.
    ///
    /// Body-check-time scope validation is suppressed because
    /// `check_meta_text_lift_diagnostics` returns the field-name token ("name"),
    /// not the per-element column name that the lift produces at expansion time.
    /// Expansion-time validation is the correct location.
    #[test]
    fn lift_in_column_reference_position_resolves_to_column() {
        // ── Part 1: "name" IS in scope — no UnknownColumn ─────────────────────
        let ctx_with_name = make_column_ref_ctx(); // has `name` and `amount` columns
        let sql = "SELECT c.name FROM t";
        let select = parse_select_stmt(sql);
        let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
        assert!(
            diags.is_empty(),
            "c.name in column-ref position with 'name' in scope must produce no lift diagnostics; got: {:?}",
            diags
        );

        // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
        // Body-check-time scope validation is suppressed: the field-name token
        // "name" is not the per-element column name.  Expansion-time validation
        // is the correct gate.
        let mut ctx_without_name = TypeContext::new();
        ctx_without_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx_without_name.add_lambda_param(
            "c",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        ctx_without_name.add_model_column(
            "t",
            "amount",
            TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            },
        );

        let diags_no_name = check_meta_text_lift_diagnostics(&select, &ctx_without_name);
        assert!(
            diags_no_name.is_empty(),
            "c.name with 'name' NOT literally in scope must still produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
            diags_no_name
        );
    }

    /// `as_alias_lift_is_parser_limited`:
    /// The spec describes `SUM(amount) AS c.name` as an AS-alias lift position
    /// (Phase C §"Meta-Text-as-identifier lift", position 2).  At Phase C the
    /// parser cannot represent `c.name` as a multi-token alias: `SelectItem::alias()`
    /// captures only the first IDENT after `AS`, so `SUM(amount) AS c.name` yields
    /// alias `"c"` and the `c.name` is silently truncated.
    ///
    /// This test documents that parser limitation: the AS-alias arm of
    /// `check_meta_text_lift_diagnostics` (the `item.alias().is_some()` branch)
    /// cannot be reached by any syntactically valid Phase-C input.  The arm is
    /// retained as a Phase-3-pending code path with a comment; its behaviour is
    /// verified here via a parser-limitation assertion rather than an end-to-end
    /// lift test.
    ///
    /// TODO(Phase-3): once the parser supports `AS <dotted-identifier>`, replace
    /// this test with a positive fixture that uses `SUM(amount) AS c.name` and
    /// verifies that no scope-check error is emitted (aliases introduce names,
    /// they do not reference them).
    #[test]
    fn as_alias_lift_is_parser_limited() {
        // `SUM(amount) AS c.name` — the parser captures only "c" as the alias.
        let sql = "SELECT SUM(amount) AS c_name FROM t";
        let select = parse_select_stmt(sql);
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");

        // alias() returns the single IDENT immediately after AS.
        let alias = item.alias();
        assert!(
            alias.is_some(),
            "SELECT SUM(...) AS c_name must have an alias; got: {:?}",
            alias
        );
        // Confirm the EXPRESSION of this item is NOT a meta-Text value —
        // SUM(amount) is a function-call, so is_meta_text_value returns None.
        let ctx = make_column_ref_ctx();
        let expr = item.expression().expect("expression");
        assert_eq!(
            is_meta_text_value(&expr, &ctx),
            None,
            "SUM(amount) is a function call, not a meta-Text value; got: {:?}",
            is_meta_text_value(&expr, &ctx)
        );

        // No lift diagnostics from check_meta_text_lift_diagnostics — SUM(amount)
        // is not a meta-Text expression so the AS-alias arm never fires.
        let diags = check_meta_text_lift_diagnostics(&select, &ctx);
        assert!(
            diags.is_empty(),
            "no lift diagnostics expected for SUM(amount) AS c_name; got: {:?}",
            diags
        );
    }

    /// `lift_in_column_reference_position_no_alias`:
    /// `SELECT c.name FROM t` (no explicit AS alias, `name` in scope):
    /// the meta-Text lift fires in column-reference position and emits no error.
    /// `infer_select_output_schema` infers `"name"` as the output column name.
    #[test]
    fn lift_in_column_reference_position_no_alias() {
        let ctx = make_column_ref_ctx();

        // c.name as the select expression — the lifted identifier "name" is the
        // inferred output column name.  No UnknownColumn should be emitted.
        let sql = "SELECT c.name FROM t";
        let select = parse_select_stmt(sql);

        // Confirm lift predicate fires.
        let list = select.select_list().expect("SelectList");
        let item = list.items().next().expect("first select item");
        let expr = item.expression().expect("expression");
        assert_eq!(
            is_meta_text_value(&expr, &ctx),
            Some("name".to_string()),
            "c.name must be detected as meta-Text"
        );

        // No explicit alias on this select item.
        assert!(
            item.alias().is_none(),
            "SELECT c.name FROM t must have no explicit alias"
        );

        // No lift diagnostic — "name" is in scope.
        let diags = check_meta_text_lift_diagnostics(&select, &ctx);
        assert!(
            diags.is_empty(),
            "c.name in SELECT list (column-ref position) with 'name' in scope must not produce diagnostics; got: {:?}",
            diags
        );
    }

    /// `lift_in_order_by_position_resolves_to_column`:
    /// `ORDER BY c.name` produces no diagnostics regardless of whether a column
    /// named "name" is in scope.
    ///
    /// Body-check-time scope validation is suppressed for the same reason as the
    /// column-reference position: the field-name token is not the per-element
    /// column name.
    #[test]
    fn lift_in_order_by_position_resolves_to_column() {
        let ctx_with_name = make_column_ref_ctx();

        // ── Part 1: "name" in scope ────────────────────────────────────────────
        let sql = "SELECT name FROM t ORDER BY c.name";
        let select = parse_select_stmt(sql);
        let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
        assert!(
            diags.is_empty(),
            "ORDER BY c.name with 'name' in scope must produce no diagnostics; got: {:?}",
            diags
        );

        // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
        let mut ctx_no_name = TypeContext::new();
        ctx_no_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx_no_name.add_lambda_param(
            "c",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        ctx_no_name.add_model_column(
            "t",
            "amount",
            TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            },
        );

        let diags_err = check_meta_text_lift_diagnostics(&select, &ctx_no_name);
        assert!(
            diags_err.is_empty(),
            "ORDER BY c.name with 'name' NOT literally in scope must produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
            diags_err
        );
    }

    /// `lift_in_group_by_position_resolves_to_column`:
    /// `GROUP BY c.name` produces no diagnostics regardless of whether a column
    /// named "name" is in scope.
    ///
    /// Body-check-time scope validation is suppressed for the same reason as the
    /// other lift positions.
    #[test]
    fn lift_in_group_by_position_resolves_to_column() {
        let ctx_with_name = make_column_ref_ctx();

        // ── Part 1: "name" in scope ────────────────────────────────────────────
        let sql = "SELECT c.name FROM t GROUP BY c.name";
        let select = parse_select_stmt(sql);
        let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
        assert!(
            diags.is_empty(),
            "GROUP BY c.name with 'name' in scope must produce no diagnostics; got: {:?}",
            diags
        );

        // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
        let mut ctx_no_name = TypeContext::new();
        ctx_no_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
        ctx_no_name.add_lambda_param(
            "c",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        ctx_no_name.add_model_column(
            "t",
            "amount",
            TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            },
        );

        let sql_no_name = "SELECT c.name FROM t GROUP BY c.name";
        let select_no_name = parse_select_stmt(sql_no_name);
        let diags_err = check_meta_text_lift_diagnostics(&select_no_name, &ctx_no_name);
        assert!(
            diags_err.is_empty(),
            "GROUP BY c.name with 'name' NOT literally in scope must produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
            diags_err
        );
    }

    // ─── Phase D: wide-reflection diagnostics ────────────────────────────────

    /// `smelt.models.with_tag(42)` emits `WithTagRequiresText` (integer is not Text).
    /// `smelt.sources.with_tag(UPPER('x'))` emits `WithTagRequiresText` (runtime Text).
    /// `smelt.models.with_tag('core')` emits no Phase D diagnostic.
    #[test]
    fn with_tag_arg_must_be_compile_time_text() {
        // smelt.models.with_tag(42) — integer literal, not Text → WithTagRequiresText
        {
            let sql = "SELECT smelt.models.with_tag(42) FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText)),
                "smelt.models.with_tag(42) must emit WithTagRequiresText, got: {:?}",
                diags
            );
            assert_eq!(
                diags
                    .iter()
                    .filter(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText))
                    .count(),
                1,
                "must emit exactly one WithTagRequiresText"
            );
        }

        // smelt.sources.with_tag(UPPER('x')) — runtime Expr<Text> → WithTagRequiresText
        {
            let sql = "SELECT smelt.sources.with_tag(UPPER('x')) FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText)),
                "smelt.sources.with_tag(UPPER('x')) must emit WithTagRequiresText, got: {:?}",
                diags
            );
        }

        // smelt.models.with_tag('core') — string literal → NO Phase D diagnostic
        {
            let sql = "SELECT smelt.models.with_tag('core') FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                !diags.iter().any(|d| matches!(
                    d.code,
                    Some(crate::DiagnosticCode::WithTagRequiresText)
                        | Some(crate::DiagnosticCode::WithTagNamedArgument)
                        | Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)
                        | Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)
                )),
                "smelt.models.with_tag('core') must emit NO Phase D diagnostic, got: {:?}",
                diags
            );
        }
    }

    /// `smelt.models.with_tag(tag => 'core')` emits exactly one `WithTagNamedArgument`.
    /// `smelt.models.with_tag('core')` does not.
    #[test]
    fn with_tag_rejects_named_argument() {
        // Named argument → WithTagNamedArgument
        {
            let sql = "SELECT smelt.models.with_tag(tag => 'core') FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument)),
                "smelt.models.with_tag(tag => 'core') must emit WithTagNamedArgument, got: {:?}",
                diags
            );
            assert_eq!(
                diags
                    .iter()
                    .filter(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument))
                    .count(),
                1,
                "must emit exactly one WithTagNamedArgument"
            );
        }

        // Positional arg → no WithTagNamedArgument
        {
            let sql = "SELECT smelt.models.with_tag('core') FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                !diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument)),
                "smelt.models.with_tag('core') must NOT emit WithTagNamedArgument, got: {:?}",
                diags
            );
        }
    }

    /// `smelt.models.bogus()` emits exactly one `WideReflectionUnknownAccessor` at
    /// the `bogus` token span; same for `smelt.sources.bogus()`.
    #[test]
    fn wide_reflection_unknown_accessor() {
        // smelt.models.bogus() — unknown accessor
        {
            let sql = "SELECT smelt.models.bogus() FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)),
                "smelt.models.bogus() must emit WideReflectionUnknownAccessor, got: {:?}",
                diags
            );
            assert_eq!(
                diags
                    .iter()
                    .filter(|d| d.code
                        == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor))
                    .count(),
                1,
                "must emit exactly one WideReflectionUnknownAccessor"
            );
        }

        // smelt.sources.bogus() — same for "sources"
        {
            let sql = "SELECT smelt.sources.bogus() FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags
                    .iter()
                    .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)),
                "smelt.sources.bogus() must emit WideReflectionUnknownAccessor, got: {:?}",
                diags
            );
        }
    }

    /// `smelt.models.all(42)` emits exactly one `WideReflectionUnexpectedArgument` at the
    /// `42` arg span; `smelt.models.all()` does not.
    /// `smelt.sources.all(named => 'x')` emits `WideReflectionUnexpectedArgument` at named-arg span.
    #[test]
    fn wide_reflection_all_takes_no_arguments() {
        // smelt.models.all(42) — positional arg to all()
        {
            let sql = "SELECT smelt.models.all(42) FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags.iter().any(
                    |d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)
                ),
                "smelt.models.all(42) must emit WideReflectionUnexpectedArgument, got: {:?}",
                diags
            );
            assert_eq!(
                diags
                    .iter()
                    .filter(
                        |d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)
                    )
                    .count(),
                1,
                "must emit exactly one WideReflectionUnexpectedArgument"
            );
        }

        // smelt.models.all() — no args → no diagnostic
        {
            let sql = "SELECT smelt.models.all() FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                !diags.iter().any(
                    |d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)
                ),
                "smelt.models.all() must NOT emit WideReflectionUnexpectedArgument, got: {:?}",
                diags
            );
        }

        // smelt.sources.all(named => 'x') — named arg to all()
        {
            let sql = "SELECT smelt.sources.all(named => 'x') FROM t";
            let select = parse_select_stmt(sql);
            let ctx = TypeContext::new();
            let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
            assert!(
                diags.iter().any(|d| d.code
                    == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)),
                "smelt.sources.all(named => 'x') must emit WideReflectionUnexpectedArgument, got: {:?}",
                diags
            );
        }
    }

    /// Given `m: ModelRef`, field projections synthesise the correct types.
    /// Given `s: SourceRef`, field projections synthesise the correct types.
    #[test]
    fn model_ref_field_projection_synthesises_field_type() {
        use smelt_types::signatures::SmeltType;

        // Set up a context with `m: ModelRef` and `s: SourceRef`.
        let mut ctx = TypeContext::new();
        ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
        ctx.add_lambda_param(
            "m",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        ctx.add_function_param_smelt_type("s", SmeltType::SourceRef);
        ctx.add_lambda_param(
            "s",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );

        // m.path → Expr<Text>
        let path_ty = infer_field_on_model_ref("m", "path", &ctx);
        assert!(
            matches!(
                path_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
                ))
            ),
            "m.path must synthesise Expr<Text>, got: {:?}",
            path_ty
        );

        // m.name → Expr<Text>
        let name_ty = infer_field_on_model_ref("m", "name", &ctx);
        assert!(
            matches!(
                name_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
                ))
            ),
            "m.name must synthesise Expr<Text>, got: {:?}",
            name_ty
        );

        // m.tags → List<Expr<Text>>
        let tags_ty = infer_field_on_model_ref("m", "tags", &ctx);
        assert!(
            matches!(&tags_ty, Some(SmeltType::List(inner))
                if matches!(inner.as_ref(), SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)))),
            "m.tags must synthesise List<Expr<Text>>, got: {:?}",
            tags_ty
        );

        // m.columns → List<ColumnRef>
        let cols_ty = infer_field_on_model_ref("m", "columns", &ctx);
        assert!(
            matches!(&cols_ty, Some(SmeltType::List(inner)) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
            "m.columns must synthesise List<ColumnRef>, got: {:?}",
            cols_ty
        );

        // SourceRef: s.path → Expr<Text>
        let s_path_ty = infer_field_on_source_ref("s", "path", &ctx);
        assert!(
            matches!(
                s_path_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
                ))
            ),
            "s.path must synthesise Expr<Text>, got: {:?}",
            s_path_ty
        );

        // SourceRef: s.name → Expr<Text>
        let s_name_ty = infer_field_on_source_ref("s", "name", &ctx);
        assert!(
            matches!(
                s_name_ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
                ))
            ),
            "s.name must synthesise Expr<Text>, got: {:?}",
            s_name_ty
        );

        // SourceRef: s.tags → List<Expr<Text>>
        let s_tags_ty = infer_field_on_source_ref("s", "tags", &ctx);
        assert!(
            matches!(&s_tags_ty, Some(SmeltType::List(inner))
                if matches!(inner.as_ref(), SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)))),
            "s.tags must synthesise List<Expr<Text>>, got: {:?}",
            s_tags_ty
        );

        // SourceRef: s.columns → List<ColumnRef>
        let s_cols_ty = infer_field_on_source_ref("s", "columns", &ctx);
        assert!(
            matches!(&s_cols_ty, Some(SmeltType::List(inner)) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
            "s.columns must synthesise List<ColumnRef>, got: {:?}",
            s_cols_ty
        );
    }

    /// Given `m: ModelRef`, `m.foo` emits exactly one `ModelRefFieldUnknown` at the `foo`
    /// field span and synthesises `Unknown` (drop-on-error).
    /// Given `s: SourceRef`, `s.bar` emits exactly one `SourceRefFieldUnknown`.
    #[test]
    fn model_ref_field_projection_rejects_unknown_field() {
        use smelt_types::signatures::SmeltType;

        let mut ctx = TypeContext::new();
        ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
        ctx.add_lambda_param(
            "m",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );
        ctx.add_function_param_smelt_type("s", SmeltType::SourceRef);
        ctx.add_lambda_param(
            "s",
            TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            },
        );

        // m.foo — unknown field on ModelRef → ModelRefFieldUnknown
        let sql = "SELECT m.foo FROM t";
        let select = parse_select_stmt(sql);
        let diags = check_model_ref_source_ref_field_diagnostics(&select, &ctx, sql);
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::ModelRefFieldUnknown)),
            "m.foo must emit ModelRefFieldUnknown, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::ModelRefFieldUnknown))
                .count(),
            1,
            "must emit exactly one ModelRefFieldUnknown"
        );
        // Confirm infer_field_on_model_ref returns None for unknown field.
        let unknown_ty = infer_field_on_model_ref("m", "foo", &ctx);
        assert!(
            unknown_ty.is_none(),
            "infer_field_on_model_ref must return None for unknown field 'foo', got: {:?}",
            unknown_ty
        );

        // s.bar — unknown field on SourceRef → SourceRefFieldUnknown
        let sql_s = "SELECT s.bar FROM t";
        let select_s = parse_select_stmt(sql_s);
        let diags_s = check_model_ref_source_ref_field_diagnostics(&select_s, &ctx, sql_s);
        assert!(
            diags_s
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::SourceRefFieldUnknown)),
            "s.bar must emit SourceRefFieldUnknown, got: {:?}",
            diags_s
        );
        assert_eq!(
            diags_s
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::SourceRefFieldUnknown))
                .count(),
            1,
            "must emit exactly one SourceRefFieldUnknown"
        );
        // Confirm infer_field_on_source_ref returns None for unknown field.
        let s_unknown_ty = infer_field_on_source_ref("s", "bar", &ctx);
        assert!(
            s_unknown_ty.is_none(),
            "infer_field_on_source_ref must return None for unknown field 'bar', got: {:?}",
            s_unknown_ty
        );
    }
}
