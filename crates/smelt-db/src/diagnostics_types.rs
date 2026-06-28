//! Diagnostic types and message builders for smelt-db.
//!
//! Pure data types and pure functions. No Salsa dependency.

use rowan::TextRange;

/// Diagnostic codes for pattern-matching in code actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    ParseError,
    InvalidModel,
    UndefinedModelRef,
    UndefinedSource,
    CannotInferType,
    /// Emitted (Error) when a schema-layer projection produces a column whose
    /// type is `Unknown` for a **compiler-resolvable** reason (`UnknownReason::Unresolved`)
    /// — i.e. the current inference rules cannot type it, but a better rule
    /// could. Distinguished from genuinely-dynamic `Unknown` (`Dynamic`/`Propagated`)
    /// which are diagnostic-free by construction. Anchored at the projection
    /// (SELECT item) that produced the column. Owned by
    /// `function_schema_inference.md` (schema-propagation rules) and `types.md`
    /// (the `Unknown` reason-discriminant).
    ColumnTypeUnresolved,
    UndeclaredColumn,
    TypeMismatch,
    CircularDependency,
    UnsupportedConstruct,
    YamlParseError,
    SourceTypeError,
    MalformedSource,
    AmbiguousColumn,
    UnknownCastType,
    UnrecognizedFunction,
    /// Emitted when two `smelt.define` declarations share a function name.
    /// Anchored at the *second* (sorted-by-path) declaration's name span; the
    /// first declaration wins. Introduced in Phase 3 of smelt-functions.
    DuplicateFunctionDefinition,
    /// Emitted when a `smelt.define` parameter or return-type annotation
    /// can't be parsed into a structured [`smelt_types::signatures::SmeltType`]
    /// — e.g. `Expr<Foo>`, `Expr<Expr<Integer>>`, or `TableExpr<T>` (the latter
    /// is reserved for Step 3). Anchored at the `TypeRef` span. Introduced in
    /// Phase 4 of smelt-functions.
    InvalidFunctionTypeRef,
    /// Emitted when a `smelt.define` body contains a type mismatch —
    /// e.g. `x + 'text'` when `x: Expr<Integer>`. Distinct from generic
    /// `TypeMismatch` because body diagnostics will carry additional frame
    /// context in Phase 6 (`ExpansionFrames`). Anchored at the *inner* bad
    /// subexpression, not the whole body. Introduced in Phase 5 of
    /// smelt-functions.
    FunctionBodyTypeMismatch,
    /// Emitted when a `smelt.define` body references a name that is neither a
    /// declared parameter nor resolvable in any enclosing scope (sources,
    /// models, CTEs — though none of those exist inside a bare function body
    /// in Step 1). Anchored at the identifier's span. Introduced in Phase 5
    /// of smelt-functions.
    UnknownIdentifier,
    /// Emitted when two parameters in a single `smelt.define` share a name.
    /// Anchored at the *second* occurrence's name span. Introduced in Phase 5
    /// of smelt-functions.
    DuplicateParameterName,
    /// Emitted when a `smelt.fn.<path>(...)` call references a function name
    /// that is not registered in the workspace. Anchored at the CALL_PATH
    /// span. Introduced in Phase 6 of smelt-functions.
    UnknownSmeltFn,
    /// Emitted when a `smelt.fn.*` call omits a required parameter (one that
    /// has no default value). Anchored at the call-path span. Introduced in
    /// Phase 6 of smelt-functions.
    MissingArgument,
    /// Emitted when a `smelt.fn.*` call passes an argument whose type does not
    /// satisfy the corresponding declared parameter's `TypeConstraint`.
    /// Anchored at the offending argument's span. Introduced in Phase 6 of
    /// smelt-functions.
    ArgTypeMismatch,
    /// Emitted when a `smelt.extern` declares a name that already exists in
    /// the built-in registry (e.g. `smelt.extern lower(...)`). Anchored at
    /// the extern name span. Introduced in Phase 10 of smelt-functions.
    ExternCollidesWithBuiltin,
    /// Emitted when a `smelt.define`'s frontmatter declares a `backends:`
    /// set that is broader than what the body implies — e.g.
    /// `backends: [duckdb, spark]` on a body that calls
    /// `duckdb.read_parquet(...)`. Also emitted when the frontmatter
    /// itself is malformed. Anchored at the declaration's name range.
    /// Introduced in Phase 11 of smelt-functions.
    BackendsWideningNotAllowed,
    /// Emitted when the frontmatter YAML block on a `smelt.define` or
    /// `smelt.extern` declaration could not be parsed (severity Error) or
    /// contained an unknown key / malformed sub-entry (severity Warning).
    /// Anchored at the declaration's name range. Introduced in Phase 43 of
    /// smelt-functions.
    FrontmatterParseError,
    /// Emitted when an expression carrying [`smelt_types::ExprKind::Window`]
    /// (a window-function call, or any expression dominated by one)
    /// appears in a splice point that only accepts scalar / aggregate
    /// expressions — currently `WHERE` and `GROUP BY`. Anchored at the
    /// offending expression's span (Phase 14 of smelt-functions, §16 #24).
    WindowInScalarContext,
    /// Emitted at call-site expansion when an `Expr<T>`-kinded parameter
    /// name overlaps a column in a sibling `TableExpr`-kinded parameter's
    /// caller-supplied schema (§16 #1). Warning severity — the body still
    /// typechecks because parameters resolve before FROM-scope columns,
    /// but the user probably meant the column. Anchored at the Expr<T>
    /// parameter's declaration range (Phase 15 of smelt-functions).
    ParameterShadowsColumn,
    /// Emitted at call-site expansion when a `TableExpr<{…}>` parameter
    /// has a row requirement the caller's schema cannot satisfy — a
    /// required column is missing, has an incompatible type, or there
    /// are extra columns when the requirement declared no tail. The
    /// diagnostic is anchored at the argument expression (not inside
    /// the body), and the body check is short-circuited so no cascade
    /// diagnostics surface from inside the callee (Phase 16 of
    /// smelt-functions).
    RowRequirementUnsatisfied,
    /// Emitted when the context identifier in `Expr<T, ctx>` does not
    /// resolve to any parameter in the same `smelt.define` declaration.
    /// Anchored at the `TypeRef` span of the offending parameter
    /// (Phase 19 of smelt-functions).
    UnknownContext,
    /// Emitted when a CTE in a `smelt.define` body forms a cyclic reference
    /// (directly or transitively). Anchored at the CTE name span.
    /// Introduced in Phase 20 of smelt-functions.
    CteCycle,
    /// Emitted when a model's top-level CTE shares a name with a CTE declared
    /// in the body of a transparent function the model directly calls. Error
    /// severity — v1 refuses and asks the author to rename one CTE; automatic
    /// alpha-rename is deferred to v2. Anchored at the call-site expression in
    /// the model. Introduced in Phase C3 of the codegen-soundness plan.
    CteShadowsCallerCte,
    /// Emitted when an explicit `Expr<T, ctx_name>` annotation disagrees with
    /// the context inferred from the parameter's splice point in the function
    /// body. Anchored at the `TypeRef` span of the offending parameter.
    /// Introduced in Phase 20 of smelt-functions.
    ContextMismatch,
    /// Emitted at a `smelt.fn.*` call site when a caller-provided fragment
    /// argument for a context-annotated `Expr<T>` parameter references a
    /// column that is not in the parameter's inferred splice context. Anchored
    /// at the offending column reference inside the argument expression.
    /// Introduced in Phase 21 of smelt-functions.
    FragmentColumnMissing,
    /// Emitted when an explicit `Expr<T, ctx_name>` annotation claims access
    /// to columns that are not present in the inferred splice context for that
    /// parameter. The annotation is "wider" than what the body actually
    /// exposes. Anchored at the argument expression at the call site.
    /// Introduced in Phase 21 of smelt-functions.
    AnnotationTooWide,
    /// Emitted when a caller-provided fragment for a `SelectItems<Kind>`
    /// parameter is of a lower expression kind than required (e.g., scalar
    /// expression passed for `SelectItems<Agg>`). Anchored at the argument
    /// expression. Introduced in Phase 21 of smelt-functions.
    FragmentKindMismatch,
    /// Emitted when a Tier 3 function's body synthesises a return type that
    /// does not match the declared `-> Expr<T>` return annotation. Anchored
    /// at the body expression span (not the function name). Introduced in
    /// Phase 24 of smelt-functions.
    ReturnTypeMismatch,
    /// Emitted when a `PASSING name AS (...)` clause names a parameter that is
    /// not declared in the callee's signature. Anchored at the `PASSING_NAME`
    /// span. Introduced in Phase 29 of smelt-functions.
    UnknownPassingParameter,
    /// Emitted when a function's frontmatter uses the `provenance:` key but
    /// the workspace's `smelt.yml` does not have `unstable_schema: true`.
    /// The `provenance:` key is an unstable feature gated behind this flag.
    /// Anchored at the function declaration's name span. Introduced in
    /// Phase 31 of smelt-functions.
    UnstableSchemaRequired,
    /// Emitted when `smelt.as_struct()` is used in a function body but the
    /// function's declared backend set includes a backend that does not
    /// support struct literal syntax. Anchored at the `smelt.as_struct` call
    /// span. Introduced in Phase 38 of smelt-functions.
    AsStructUnsupportedBackend,
    /// Emitted when the transparent-function call graph contains a cycle —
    /// directly (`A` calls `A`) or transitively (`A` → `B` → `A`).  Anchored
    /// at the offending function declaration's name span.  The
    /// `smelt-db::logical_plan` cycle pre-pass aborts splicing for every
    /// `fn_id` participating in the cycle so the planner does not attempt to
    /// inline a non-terminating expansion.  Introduced in Phase 41 of
    /// smelt-functions.
    FunctionCallCycle,
    /// Emitted when a function's declared `provenance:` entry lists a source
    /// column not read by the body expression, or the body reads a column not
    /// listed in the declared provenance. Anchored at the declaration's name
    /// range. Introduced in Phase 51.
    ProvenanceMismatch,
    /// Emitted when a function's declared `joins:` entry names a table that
    /// does not appear as a join alias in the body's outermost FROM clause.
    /// Anchored at the declaration's name range. Introduced in Phase 51.
    JoinsMismatch,
    /// Emitted (Severity::Warning) for every declared join whose `cardinality`
    /// field is non-empty. Cardinality is trusted, not verified against data
    /// (§20E soundness caveat). Anchored at the declaration's name range.
    /// Introduced in Phase 51.
    DeclaredCardinalityUnverifiable,
    /// Emitted (Severity::Hint) when a transparent function is called from
    /// a SELECT that has a WHERE clause but the function lacks declared
    /// provenance, which would allow filter pushdown. Introduced in Phase 52.
    MissingProvenancePushdownAdvisory,
    /// Emitted when a `smelt.extern` declaration has a parameter whose type
    /// is a fragment sort (`SelectItems`, `OrderSpec`). Fragment-sort params
    /// are only meaningful with PASSING clauses, which `smelt.extern` does
    /// not support (§16 #18). Introduced in Phase 52.
    ExternFragmentParamUnsupported,
    /// Emitted when a `smelt.<path>` reference resolves to an entity whose
    /// kind is not valid in the surrounding position — for example,
    /// `FROM smelt.tests.foo` (test in a `TableExpr` position). Anchored
    /// at the path-form ref's text range. Introduced in Phase 2a of the
    /// smelt-`<path>` migration (architecture Surface §"Resolution").
    KindMismatch,
    /// Emitted (Warning) for a seed CSV file that has no sibling `.yml`
    /// sidecar. Schema is inferred at compile time from the first 100 rows
    /// and may drift when the CSV changes. Resolve by adding a sidecar.
    /// Introduced in Phase 7 of the seeds plan.
    MissingSeedSidecar,
    /// Emitted when an empty list literal `[]` appears in a position where
    /// no target sort context is available to infer the element type.
    /// Message: "cannot infer element type for empty list literal".
    /// Anchored at the list literal span. Introduced in Phase 3 of the
    /// meta-language plan (Phase A).
    MetaListEmptyTypeUnknown,
    /// Emitted when a list literal's elements have incompatible types that
    /// cannot be unified under LUB. Message: "list elements have
    /// incompatible types: {T0}, {Tk}". Anchored at the list literal span.
    /// Introduced in Phase 3 of the meta-language plan (Phase A).
    MetaListHeterogeneous,
    /// Emitted when a spread operator `...xs` appears in a position that
    /// does not permit spread: WHERE clause, FROM clause without an explicit
    /// reducer, boolean composition, or named-argument value. Message:
    /// "spread is not allowed in {position name}". Anchored at the spread's
    /// span. Introduced in Phase 3 of the meta-language plan (Phase A).
    MetaSpreadInForbiddenPosition,
    /// Emitted when the operand of a spread `...x` is not a `List<T>` (its
    /// inferred type is some other sort). Message: "spread expects List<T>;
    /// found {actual type}". The spread is dropped and type-checking of the
    /// surrounding form continues. Anchored at the spread's span.
    /// Introduced in Phase 3 of the meta-language plan (Phase A).
    MetaSpreadOnNonList,
    /// Emitted when a `List<T>`-typed expression reaches a Data-World scalar /
    /// SELECT-item position without being consumed by a spread, a HOF, a
    /// reducer, a record, a map, or a generator (e.g. `SELECT [1, 2, 3]` or
    /// `SELECT xs |> map(fn c => c * 2)` left bare). A list cannot materialise
    /// as a scalar value and there is no implicit auto-spread; the explicit
    /// `...xs` spread is the only path from a list into a comma position.
    /// Message: "a List<T> cannot be used as a scalar value here; consume it
    /// with a spread (`...xs`), a reducer (`reduce(xs, …)`), or a HOF before
    /// splicing". Anchored at the offending select-item / scalar expression.
    /// This select-shape check runs for every model, including FROM-less ones.
    MetaListInScalarPosition,

    // ── Phase B (meta-language) diagnostic codes ─────────────────────────
    /// Emitted when a `fn x => body` lambda appears outside a HOF positional
    /// argument position (e.g. top-level expression, list element, named-arg
    /// value). Anchored at the `fn` keyword span. Message:
    /// "lambda is only valid as an argument to a higher-order function".
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    LambdaInForbiddenPosition,
    /// Emitted when a lambda passed to a HOF has a different arity from what
    /// the HOF expects. For `map`/`filter` arity-1 is required; `reduce` takes
    /// a reducer (not a lambda). Mismatch → `LambdaArityMismatch`.
    /// Message: "{hof} expects a lambda of arity {expected}; found arity {actual}".
    /// Anchored at the lambda span.
    /// Introduced in Phase F of the meta-language plan.
    LambdaArityMismatch,
    /// Emitted when a lambda has zero parameters (`fn () => body`).
    /// Message: "lambda must declare at least one parameter".
    /// Anchored at the lambda parameter list span (or the `fn` keyword).
    /// Introduced in Phase F of the meta-language plan.
    LambdaZeroParameters,
    /// Emitted when a lambda parameter list contains the same name twice.
    /// Message: "parameter `{name}` already appears in this lambda's parameter list".
    /// Anchored at the second occurrence's IDENT span.
    /// Introduced in Phase F of the meta-language plan.
    LambdaDuplicateParameter,
    /// Emitted when the lambda body's synthesised type is incompatible with
    /// the HOF's required result shape (e.g. `filter` requires `Boolean`).
    /// Message: "{hof} requires lambda result {expected}; found {actual}".
    /// Anchored at the body expression span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    LambdaResultTypeMismatch,
    /// Emitted when the second argument to `map` or `filter` is not a lambda.
    /// Message: "{hof} expects a lambda; found {actual type}".
    /// Anchored at the second-argument span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    HofExpectsLambda,
    /// Emitted when the second argument to `reduce` is not a registered reducer.
    /// Message: "reduce expects a reducer; found {actual}".
    /// Anchored at the second-argument span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    HofExpectsReducer,
    /// Emitted when a `smelt.define` declaration uses a HOF name (`map`, `filter`,
    /// `reduce`). Message: "{name} is a reserved higher-order function name".
    /// Anchored at the declaration's name token.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    HofNameShadowed,
    /// Emitted when a `smelt.define` declaration uses a reducer name from the
    /// closed registry. Message: "{name} is a reserved reducer name".
    /// Anchored at the declaration's name token.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ReducerNameShadowed,
    /// Emitted when the RHS of `|>` is not syntactically a call expression.
    /// Message: "pipe right-hand side must be a function call".
    /// Anchored at the RHS span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    PipeRhsNotCall,
    /// Emitted when a pipe expression `|>` appears in a Data-World grammar
    /// position (e.g. inside a WHERE predicate). Message:
    /// "|> is meta-only; use SQL composition in this position".
    /// Anchored at the pipe expression span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    PipeInDataPosition,
    /// Emitted when a reducer is applied to a list whose element type is
    /// incompatible with the reducer's declared input constraint.
    /// Message: "reducer {r} expects List<{T_in}>; found List<{T_actual}>".
    /// Anchored at the second-argument (reducer name) span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ReducerInputTypeMismatch,
    /// Emitted when `union_all` or `intersect_all` reduces an empty list
    /// (no identity element). Message: "reducer {r} has no identity for an empty list".
    /// Anchored at the `reduce` call span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ReducerEmptyNoIdentity,
    /// Emitted when a parameterised reducer call has the wrong number of
    /// positional arguments.
    /// Message: "reducer {r} expects {expected} argument(s); found {actual}".
    /// Anchored at the `REDUCER_CALL` node span.
    /// Introduced in Phase F of the meta-language plan.
    ReducerArityMismatch,
    /// Emitted when a parameterised reducer argument has the wrong type.
    /// Message: "reducer {r}'s argument `{param}` expects {expected}; found {actual}".
    /// Anchored at the offending argument expression span.
    /// Introduced in Phase F of the meta-language plan.
    ReducerArgTypeMismatch,
    /// Emitted when a parameterised reducer argument is a runtime expression
    /// rather than a compile-time value.
    /// Message: "reducer {r}'s argument `{param}` must be a compile-time value; found {actual}".
    /// Anchored at the offending argument expression span.
    /// Introduced in Phase F of the meta-language plan.
    ReducerArgNotCompileTime,
    /// Emitted when a parameterised reducer call uses named arguments.
    /// Message: "reducer {r} takes positional arguments only".
    /// Anchored at the named argument span.
    /// Introduced in Phase F of the meta-language plan.
    ReducerNamedArgument,
    /// Emitted when a HOF call (`map`, `filter`, `reduce`) passes any argument
    /// by name (e.g. `map(list: xs, fn c => c)`). HOFs take positional
    /// arguments only. Fires before the lambda/kind check so a named-lambda
    /// still surfaces this code rather than being silently accepted. Anchored
    /// at the first named-argument span in the HOF's argument list.
    HofNamedArgument,
    /// Emitted when the ternary condition expression is not Boolean.
    /// Message: "ternary condition expects Boolean; found {actual}".
    /// Anchored at the condition expression span.
    /// Introduced in Phase F of the meta-language plan.
    TernaryConditionNotBoolean,
    /// Emitted when the then-branch and else-branch of a ternary have
    /// incompatible types that cannot be unified.
    /// Message: "ternary branches have incompatible types: {then_type} vs {else_type}".
    /// Anchored at the `else` keyword span.
    /// Introduced in Phase F of the meta-language plan.
    TernaryBranchTypeMismatch,
    /// Emitted when a `smelt.define`, `smelt.record`, or lambda parameter is
    /// declared with a name that is a reserved ternary keyword.
    /// Message: "{name} is a reserved meta-language keyword".
    /// Anchored at the offending name token.
    /// Introduced in Phase F of the meta-language plan.
    TernaryKeywordShadowed,
    /// Emitted when a ternary expression appears in a Data-World (SQL) splice
    /// position. `if-then-else` is meta-only; SQL has `CASE WHEN`.
    /// Message: "if-then-else is meta-only; use SQL CASE WHEN in this position".
    /// Anchored at the `if` keyword span.
    ///
    /// Note: pure-inference may not have enough parent-context to detect this.
    /// Phase 3 (`check_file_diagnostics`) wires the splice-context check.
    /// Introduced in Phase F of the meta-language plan.
    TernaryInDataPosition,
    /// Emitted when a `then` keyword appears outside of an `if ... then ...` form.
    /// Message: "unexpected `then` keyword outside of `if ... then ...` form".
    /// Anchored at the `then` token.
    /// Introduced in Phase F of the meta-language plan.
    TernaryDanglingThen,
    /// Emitted when an `else` keyword appears outside of a `... then ... else` form.
    /// Message: "unexpected `else` keyword outside of `... then ... else` form".
    /// Anchored at the `else` token.
    /// Introduced in Phase F of the meta-language plan.
    TernaryDanglingElse,
    /// Emitted when `smelt.config.var(<name>)` is called and `<name>` is not
    /// present in `smelt.yml` `vars:`. Message:
    /// "compile-time variable {name} not declared in smelt.yml vars".
    /// Anchored at the call site.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ConfigVarNotFound,
    /// Emitted when `smelt.config.var` is called with a non-literal-Text argument.
    /// Message: "smelt.config.var name must be a string literal".
    /// Anchored at the argument expression span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ConfigVarNameNotLiteral,
    /// Emitted (Warning) when a YAML `null` value is coerced to empty string
    /// at a `smelt.config.var` site. Message:
    /// "null variable {name} coerced to empty string; declare a default in smelt.yml".
    /// Anchored at the call site.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    ConfigVarNullCoercion,

    // ── Phase C (meta-language) diagnostic codes ─────────────────────────
    /// Emitted when `smelt.columns_of(x)` is called and `x` synthesises to a
    /// type that is not assignable to `TableExpr`. Message:
    /// "smelt.columns_of expects TableExpr; found {actual}".
    /// Anchored at the argument expression span.
    /// Introduced in Phase 1 of the meta-language plan (Phase C).
    ColumnsOfRequiresTableExpr,
    /// Emitted when `smelt.columns_of` is called with a named argument
    /// (e.g. `smelt.columns_of(t => orders)`). Message:
    /// "smelt.columns_of takes one positional argument; named arguments are not supported".
    /// Anchored at the named-argument span.
    /// Introduced in Phase 1 of the meta-language plan (Phase C).
    ColumnsOfNamedArgument,
    /// Emitted when a field access on a `ColumnRef`-typed value uses a field
    /// identifier outside the closed field set `{name, type, is_numeric}`. Message:
    /// "ColumnRef has no field {name}; expected one of: name, type, is_numeric".
    /// Anchored at the field-name token span (not the base expression).
    /// Introduced in Phase 1 of the meta-language plan (Phase C).
    ColumnRefFieldUnknown,
    /// Emitted at expansion time when `smelt.columns_of(t)` is called and
    /// `t`'s schema cannot be statically resolved (the upstream returns
    /// `Unknown` — the model does not exist, has an unresolvable schema, or
    /// refers to an opaque expression). Message:
    /// "cannot resolve column list for {t}; upstream schema is unknown".
    /// Anchored at the `smelt.columns_of(t)` call site span.
    /// Drop-on-error recovery: the surrounding HOF splice drops without
    /// further diagnostics (same policy as `MetaSpreadInForbiddenPosition`).
    /// Introduced in Phase 3 of the meta-language plan (Phase C).
    ColumnsOfUnresolvableSchema,

    // ── Phase D (meta-language) diagnostic codes ─────────────────────────
    /// Emitted when `smelt.models.with_tag(x)` or `smelt.sources.with_tag(x)`
    /// is called and `x` synthesises to a type not assignable to compile-time
    /// `Text` (e.g. a runtime `Expr<Text>` like `UPPER('x')` or an integer
    /// literal). Message: "with_tag expects a compile-time Text; found {actual}".
    /// Anchored at the argument expression span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    WithTagRequiresText,
    /// Emitted when `with_tag` is called with a named argument
    /// (e.g. `smelt.models.with_tag(tag => 'core')`). Message:
    /// "with_tag takes one positional argument; named arguments are not supported".
    /// Anchored at the named-argument span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    WithTagNamedArgument,
    /// Emitted when `smelt.models.<name>` or `smelt.sources.<name>` refers to
    /// an accessor name outside the closed set `{with_tag, all}`. Message:
    /// "smelt.{models,sources} has no accessor `{name}`; expected one of: with_tag, all".
    /// Anchored at the accessor-name token span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    WideReflectionUnknownAccessor,
    /// Emitted when `smelt.models.all` or `smelt.sources.all` is called with
    /// any argument (positional or named). Message: "{accessor} takes no arguments".
    /// Anchored at the offending argument's span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    WideReflectionUnexpectedArgument,
    /// Emitted when field access on a `ModelRef`-typed value uses a field
    /// identifier outside the closed field set `{path, name, tags, columns}`.
    /// Message: "ModelRef has no field `{name}`; expected one of: path, name, tags, columns".
    /// Anchored at the field-name token span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    ModelRefFieldUnknown,
    /// Emitted when field access on a `SourceRef`-typed value uses a field
    /// identifier outside the closed field set `{path, name, tags, columns}`.
    /// Message: "SourceRef has no field `{name}`; expected one of: path, name, tags, columns".
    /// Anchored at the field-name token span.
    /// Introduced in Phase 1 of the meta-language plan (Phase D).
    SourceRefFieldUnknown,

    // ── Phase E1 (meta-language) record diagnostic codes ─────────────────────
    /// A second `smelt.record` declaration in the workspace shares an existing
    /// record's name. First-declaration-wins. Anchored at the second
    /// declaration's name token.
    /// Message: "record `{name}` is already declared in {path}; record names must be unique workspace-wide"
    SmeltRecordRedefinition,
    /// Field projection or literal field name outside the target's declared
    /// field set. Anchored at the offending field-name token.
    /// Message: "record `{type}` has no field `{name}`; expected one of: {fields}"
    RecordFieldUnknown,
    /// A record literal omits a field required by the target type.
    /// Anchored at the literal's closing brace.
    /// Message: "record literal for `{type}` is missing required field `{name}`"
    RecordFieldMissing,
    /// A record literal names the same field twice.
    /// Anchored at the second occurrence's name token.
    /// Message: "field `{name}` already appears in this record literal"
    RecordFieldDuplicate,
    /// A literal field value's type is not assignable to the declared field type.
    /// Anchored at the offending value expression.
    /// Message: "record field `{name}` expects {expected}; found {actual}"
    RecordFieldTypeMismatch,
    /// A record literal appears in a position with no inferable target type.
    /// Anchored at the literal's opening brace.
    /// Message: "cannot infer record type from context; annotate the target type"
    RecordLiteralUnknownTarget,
    /// Mid-chain field projection through a non-record-typed value.
    /// Anchored at the offending projection token.
    /// Message: "value of type {type} has no fields; projection `{field}` is not valid"
    RecordFieldNotProjectable,
    /// A `smelt.record` field type references a meta-only witness
    /// (`ColumnRef`, `ModelRef`, `SourceRef`) or `Lambda`.
    /// Anchored at the field's type expression.
    /// Message: "record field types may not reference {type}; reflection witnesses are not user-writable"
    RecordFieldTypeForbidden,
    /// A record declaration references its own name directly or transitively,
    /// forming a cycle. v1 records must form a DAG.
    /// Anchored at the cycle's introducing field-type expression.
    /// Message: "record `{name}` forms a cycle; recursive record declarations are not supported in v1"
    RecordCyclicDeclaration,
    /// A record-typed value is referenced in a Data-World (SQL) position
    /// outside a splice context. Records are pure meta-world values.
    /// Anchored at the binding reference.
    /// Message: "record-typed value may not appear in a Data-World (SQL) position; use field projection to produce a spliced value"
    RecordInDataWorld,

    // ── Phase E1 (meta-language) map diagnostic codes ─────────────────────────
    /// A `Map<K, V>` type expression with `K` other than `Text`.
    /// Message: "Map key type must be Text in v1; found {type}"
    MapKeyTypeNotText,
    /// Method-call on a `Map<K, V>` value with a name outside the closed Map API.
    /// Message: "Map has no method `{name}`; expected one of: entries, keys, values, get, has"
    MapApiUnknown,
    /// `m.get` or `m.has` called with other than one positional argument.
    /// Message: "Map.{method} expects one positional argument; found {n}"
    MapApiArityMismatch,
    /// A Map API method called with a named argument.
    /// Message: "Map.{method} does not support named arguments"
    MapApiNamedArgument,
    /// `m.entries`, `m.keys`, or `m.values` called with any argument.
    /// Message: "Map.{method} takes no arguments"
    MapApiUnexpectedArgument,
    /// `m.get(k)` with statically-known `k` absent from `m`.
    /// Message: "Map has no binding for key `{key}`"
    MapGetMissingKey,
    /// `m.get(k)` or `m.has(k)` with `k`'s type not assignable to `K`.
    /// Message: "Map.{method} expects key of type {expected}; found {actual}"
    MapApiArgTypeMismatch,

    // ── Phase E1 (meta-language) loader diagnostic codes ─────────────────────
    /// Loader `path` argument is not a string literal.
    /// Message: "loader path must be a string literal; found {expr}"
    ConfigLoaderPathNotLiteral,
    /// Path is absolute, contains `..` escape, or has a scheme prefix.
    /// Message: "loader path must be a workspace-relative path; found {path}"
    ConfigLoaderPathEscapesWorkspace,
    /// Path contains `\`.
    /// Message: "loader paths use `/` as the path separator; found `\` in {path}"
    ConfigLoaderPathBackslash,
    /// Resolved file does not exist in the workspace.
    /// Message: "loader file `{path}` not found in workspace"
    ConfigLoaderFileNotFound,
    /// Schema argument is not an admissible shape.
    /// Message: "loader schema must be a record type, `List<record>`, or `Map<Text, record>`; found {actual}"
    ConfigLoaderSchemaForbidden,
    /// `smelt.config.load_toml` is called.
    /// Message: "smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1"
    ConfigLoaderTomlNotYetSupported,
    /// The file is not valid YAML / JSON.
    /// Message: "failed to parse {format} file `{path}`: {parser_error}"
    ConfigLoaderParseError,
    /// A loaded value omits a field required by the schema.
    /// Message: "field `{name}` required by schema is missing"
    ConfigLoaderRequiredFieldMissing,
    /// A loaded value contains a field not in the schema.
    /// Message: "field `{name}` is not declared in the schema; expected one of: {fields}"
    ConfigLoaderUnknownField,
    /// A loaded value's type does not match the schema's declared type.
    /// Message: "field `{name}` expects {expected}; got {actual}"
    ConfigLoaderTypeMismatch,
    /// The file's top-level shape does not match the schema's expected root shape.
    /// Message: "schema `{type}` expects {expected_shape}; file's top level is {actual_shape}"
    ConfigLoaderRootShapeMismatch,
    /// A `Map<Text, S>`-shaped file contains the same key twice.
    /// Message: "duplicate map key `{key}` at {row}; earlier appearance at {first_row}"
    ConfigLoaderDuplicateMapKey,
    /// A YAML `null` scalar coerces to an empty `Text` value at a schema field declared `Text`.
    /// Severity: Warning.
    /// Message: "null value at {row} coerced to empty string; declare a default in the source file"
    ConfigLoaderNullCoercion,

    // ── Multi-model production diagnostic codes ──────────────────────────────
    /// `generates:` value other than `models` was supplied.
    /// Anchored at the YAML value token.
    /// Message: "generates must be `models`; found {value}"
    GeneratesUnknownValue,
    /// `generates: models` frontmatter combined with `name:` field or with
    /// Layer-1 `--- name: foo ---` section delimiters.
    /// Anchored at the offending key / delimiter.
    /// Message: "generates: models cannot coexist with bare-model identity (name field or section delimiter)"
    GeneratesMixedWithBareModel,
    /// Generator file body contains a top-level bare SELECT / WITH / VALUES.
    /// Anchored at the offending statement.
    /// Message: "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape"
    GenerateFileBareSelectForbidden,
    /// Generator file body synthesises a type not assignable to `List<ModelDef>`.
    /// Anchored at the body expression.
    /// Message: "generator file body must evaluate to List<ModelDef>; found {actual}"
    GenerateFileBodyTypeError,
    /// `ModelDef {…}` record literal in a non-generator-file context.
    /// Anchored at the literal's opening brace.
    /// Message: "ModelDef literals are only valid inside a `generates: models` file body"
    ModelDefOutsideGeneratorFile,
    /// `ModelDef.name` value is empty or contains non-path-safe characters.
    /// Anchored at the field value expression.
    /// Message: "ModelDef.name must be a non-empty Text of [A-Za-z0-9_]+; found {value}"
    ModelDefInvalidName,
    /// `ModelDef.materialization` value not in `{'view', 'table', 'incremental'}`.
    /// Anchored at the field value expression.
    /// Message: "ModelDef.materialization must be one of view, table, incremental; found {value}"
    ModelDefInvalidMaterialization,
    /// Two `ModelDef`s in the same generator emit with the same `name`.
    /// Anchored at the second occurrence's name field value.
    /// Message: "duplicate ModelDef.name `{name}` in this generator file"
    ModelDefDuplicateName,
    /// Generator-emitted path collides with a hand-authored model or another
    /// generator's emission.
    /// Anchored at the offending `ModelDef`'s name field value.
    /// Message: "ModelDef emits `{smelt_path}` which collides with {other_path}"
    ModelDefHandAuthoredCollision,
    /// A generator's body invokes `smelt.models.with_tag` or `smelt.models.all`.
    /// Anchored at the `smelt.models.*` call site.
    /// Message: "smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references"
    GeneratorBodyForbidsModelReflection,

    // ── Multi-model section structure diagnostic codes ───────────────────────
    /// SQL content (non-comment, non-empty) appears before the first
    /// `--- name: model_name ---` section delimiter in a multi-model file.
    /// This makes the file structurally invalid: SQL must be inside a named
    /// section. Anchored at the top of the file (offset 0). Error severity.
    /// Message: "malformed multi-model section delimiter at line {n}: SQL content must be inside a '--- name: model_name ---' section; found non-section content before the first delimiter"
    MalformedSectionDelimiter,
    /// A `---` frontmatter opening in a single-model or multi-model file has
    /// no matching closing `---`. Anchored at the top of the file (offset 0).
    /// Error severity.
    /// Message: "frontmatter not closed: missing closing '---'"
    UnclosedFrontmatter,

    // ── Timeseries diagnostic codes ──────────────────────────────────────────
    /// A model declares `incremental:` without a sibling `timeseries:` block.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "TimeseriesRequiredForIncremental: model declares `incremental:` but has no `timeseries:` block — add a `timeseries:` block with event_time_column, partition_column, and granularity"
    TimeseriesRequiredForIncremental,
    /// The `timeseries:` block parses but violates a structural rule.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "MalformedTimeseries: {message}"
    MalformedTimeseries,

    // ── VALUES/CTE alias-column-list diagnostic codes ────────────────────────
    /// Emitted when the alias column list in `(VALUES …) AS t(c₁, c₂, …)` or
    /// `WITH cte(c₁, c₂, …) AS (SELECT …)` has a different length from the
    /// underlying relation's actual column count.  Anchored at the
    /// `ALIAS_COLUMN_LIST` span (the parenthesised column-name list).
    /// Message: "alias column list has {alias_count} name(s) but the relation
    /// has {col_count} column(s)"
    AliasColumnArityMismatch,
    /// Emitted when a `(VALUES …)` derived table contains no rows and therefore
    /// cannot produce a typed schema.  Anchored at the VALUES clause span.
    /// Message: "VALUES clause has no rows; cannot infer column types"
    EmptyValuesClause,

    /// Emitted when a Python `@model` function returns a SQL string whose
    /// frontmatter declares a `name:` field that differs from the function name.
    /// The model name is always the function name; the `name:` field in the
    /// returned frontmatter must either be absent or exactly equal the function
    /// name. A mismatch means the frontmatter would silently apply to a model
    /// the author did not intend (or be silently dropped). Anchored at the
    /// beginning of the file (range zero). Severity: Error.
    /// Message: "python model name mismatch: frontmatter declares name '{fm_name}'
    /// but function name is '{fn_name}'; remove the name field or set it to
    /// '{fn_name}'"
    PythonModelNameMismatch,

    // ── Planner-rule diagnostic codes (surfaced via the uniform rule →
    //    diagnostics interface; see `smelt_logical::rules::rule_diagnostics`) ────
    /// `cumulative_aggregate` SELECT has no GROUP BY (the key columns).
    CumulativeRequiresGroupBy,
    /// A `cumulative_aggregate` projection uses a non-allowlisted aggregator or
    /// a composite expression over aggregates.
    CumulativeUnknownAggregator,
    /// The `cumulative_aggregate` GROUP BY contains the driving source's
    /// `partition_column` (a per-partition shape, not the cumulative one).
    CumulativeGroupByContainsPartitionColumn,
    /// Window functions (`OVER (...)`) are not allowed in a `cumulative_aggregate`.
    CumulativeForbidsWindowFunctions,
    /// A non-deterministic function appears in a `cumulative_aggregate` SELECT.
    CumulativeForbidsNondeterministic,
    /// No source in a `cumulative_aggregate`'s FROM declares a `timeseries:` block.
    CumulativeNoDrivingSource,
    /// Multiple timeseries-tagged sources in a `cumulative_aggregate`'s FROM (v1
    /// supports exactly one driving source).
    CumulativeMultipleDrivingSources,
    /// A `cumulative_aggregate` SELECT could not be parsed for classification.
    CumulativeSqlNotParseable,
    /// A `cumulative_aggregate` model incorrectly declares a `timeseries:` block.
    /// The cumulative output has no partition column; the rule reads it from the
    /// driving source. Anchored at offset 0. Error severity.
    CumulativeForbidsTimeseries,
    /// A `cumulative_aggregate` model incorrectly declares an `incremental:` block.
    /// The two materializations have different equivalence contracts; pick one.
    /// Anchored at offset 0. Error severity.
    CumulativeForbidsIncremental,
    /// Advisory (`Warning`): an `incremental` model's SQL is not batch-safe
    /// under the planner's incremental safety classifier (the build does not
    /// hard-refuse — its dispatch falls back to a safe chunking strategy).
    IncrementalNotBatchSafe,
    /// An incremental model's `event_time_column` is not accessible at the
    /// outermost SELECT where the time filter is injected — either because the
    /// query is a set operation (UNION/INTERSECT/EXCEPT) or because the FROM
    /// clause is a subquery that does not project the column. Error severity.
    EventTimeColumnNotVisibleAtOuterSelect,
    /// Emitted when two files in the same project resolve to the same
    /// `smelt.<path>` address across any entity kind (model, function, seed,
    /// source). Hard workspace-load error; the colliding entities do not load.
    /// Project-scoped per the project-isolation rule (same address in two
    /// different projects is independent). Error severity. Anchored at the
    /// second (later-discovered) file's path, at offset 0.
    DuplicateAddress,
    /// Emitted when two persisted entities in the same project resolve to the
    /// same `(active-target schema, address-joined-by-_)` emitted table name,
    /// even though their `smelt.<path>` addresses differ (the `_`-join is not
    /// injective — e.g. `smelt.staging.orders` and `smelt.staging_orders` both
    /// emit `main.staging_orders`). Prevents a silent table clobber. Error
    /// severity. Project-scoped. Anchored at the second entity's path, offset 0.
    DuplicateEmittedName,
    /// Emitted when a `smelt.define` default expression references another
    /// parameter in the same signature, violating Semantics #9 ("a default
    /// expression must not reference other parameters"). Anchored at the
    /// default expression's range. Error severity.
    DefaultReferencesParameter,
    /// Emitted when a `smelt.define` or `smelt.extern` parameter or return-type
    /// annotation has a `Struct<{…}>` shape whose field type text cannot be
    /// parsed as a concrete `DataType` — e.g. `{a: Integer, b: Bogus}` where
    /// `Bogus` is unrecognised. Anchored at the **individual field's** `TYPE_REF`
    /// span (more precise than `InvalidFunctionTypeRef` which covers the whole
    /// annotation). Error severity.
    UnknownStructFieldType,
    /// Emitted when a decimal arithmetic expression or UNION coercion computes
    /// a result precision p' > 38. Anchored at the operator token span
    /// (arithmetic) or UNION keyword span (UNION coercion). The result type
    /// degrades to Unknown.
    DecimalPrecisionOverflow,
    /// Emitted when portable code declares or uses a non-binary collation on a
    /// string (§17). Non-binary collations (case-insensitive, accent-insensitive,
    /// locale-aware) are not in the portable surface: `Binary` (`COLLATE "C"`,
    /// `COLLATE BINARY`, `COLLATE UTF8_BINARY`, `COLLATE POSIX`) is the only
    /// cross-engine collation. Anchored at the `COLLATE` clause span. Recovery:
    /// the expression type degrades to `Unknown` (reason `Unresolved`). The user
    /// must compare byte-wise (the default binary collation) or declare an engine
    /// on the model to use the engine's native collation.
    NonPortableCollation,

    // ── Virtual environments diagnostic codes (D-46/D-47) ───────────────────
    /// Emitted when a model's frontmatter declares a `state.mode` that is
    /// higher in the posture lattice than the project's `state.mode` (models
    /// may narrow but not widen; D-47). Error severity. Anchored at offset 0.
    /// Message: "model declares state.mode {model_mode} but project posture is
    /// {project_mode}; models may narrow but not widen the project posture"
    StateModeWidening,
    /// Emitted (Error) when a `smelt.<model>#<cte>` CTE reference appears
    /// outside a `smelt.test` body. The `#` operator is test-local: it may
    /// only be used inside a `smelt.test` declaration body to address one
    /// internal CTE of the referenced model. Using it in a model body, a
    /// `smelt.define` body, a `smelt.check` body, or any other position is
    /// a hard error. Anchored at the `#` operator token.
    CteRefOutsideTest,
    /// Emitted (Error) when a `smelt.check` declaration carries a `PASSING`
    /// or `EXPECT` clause. These clauses are valid only on `smelt.test`
    /// declarations; a check has no mocks and no expected output — it is a
    /// failing-rows query against real built data. Anchored at the offending
    /// clause's opening keyword (`PASSING` or `EXPECT`).
    CheckHasTestClause,
    /// Emitted (Error) at `smelt test` run time when a `PASSING <dep>` clause
    /// in a `smelt.test` declaration names a dependency that is not a reachable
    /// external `smelt.<path>` dep of the assertion query. Catches typos such
    /// as `PASSING order AS (...)` when the actual dep is `orders`.  A typo'd
    /// PASSING clause would otherwise produce a false green (the dep gets an
    /// empty mock CTE). Anchored at the offending PASSING_NAME span.
    UnknownTestInput,
    /// Emitted (Error) at `smelt test` run time when a `smelt.<model>#<cte>`
    /// reference in a `smelt.test` body names a CTE that is absent from the
    /// referenced model's `WITH` clause. Anchored at the `#<cte>` suffix token.
    UnknownTestCte,
}

/// Structured metadata attached to diagnostics for code actions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticData {
    UndefinedRef {
        model_name: String,
    },
    UndefinedSource {
        source_name: String,
        table_name: String,
    },
    CannotInferType {
        column_name: String,
    },
    UndeclaredColumn {
        qualifier: Option<String>,
        column_name: String,
    },
    TypeMismatch {
        column_name: String,
        ref_name: String,
        actual_type: String,
        expected_type: String,
    },
    /// Single-level (Phase 6) or multi-level (Phase 12) expansion trace
    /// attached to diagnostics emitted at or inside a `smelt.fn.*` call. Frames
    /// are ordered innermost-first → outermost-last; Phase 6's LSP renderer
    /// reads only `frames.last()` (the outermost call site the user wrote) to
    /// produce a single trailing "in expansion of `X`, `p` was bound to
    /// <type>" line, while Phase 12 will iterate the whole vector.
    ///
    /// Existing LSP clients that don't know this variant simply drop the
    /// `data` payload — the diagnostic's primary `message` and `range` are
    /// unaffected.
    ExpansionFrames(Vec<smelt_types::FrameInfo>),
    /// Attached to `MissingSeedSidecar` diagnostics. Carries the CSV path
    /// and the expected sidecar path so the code-action provider can create
    /// the sibling `.yml` without re-deriving it from the diagnostic range.
    MissingSeedSidecar {
        csv_path: std::path::PathBuf,
        sidecar_path: std::path::PathBuf,
    },
}

/// Represents a diagnostic (error, warning, info)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub range: TextRange,
    pub code: Option<DiagnosticCode>,
    pub data: Option<DiagnosticData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Render the diagnostic message for Phase A (meta-language) list and spread
/// diagnostic codes.
///
/// Parameters:
/// - `code`: one of the four Phase A `DiagnosticCode` variants.
/// - `first_type`: the first element's rendered type (for `MetaListHeterogeneous`).
/// - `other_type`: the incompatible/actual type (for `MetaListHeterogeneous` and
///   `MetaSpreadOnNonList`).
/// - `position_name`: the human-readable position name (for
///   `MetaSpreadInForbiddenPosition`), e.g. `"WHERE clause"`.
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic
/// codes".
pub fn meta_list_diagnostic_message(
    code: DiagnosticCode,
    first_type: Option<&str>,
    other_type: Option<&str>,
    position_name: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::MetaListEmptyTypeUnknown => {
            "cannot infer element type for empty list literal".to_string()
        }
        DiagnosticCode::MetaListHeterogeneous => {
            let t0 = first_type.unwrap_or("?");
            let tk = other_type.unwrap_or("?");
            format!("list elements have incompatible types: {}, {}", t0, tk)
        }
        DiagnosticCode::MetaSpreadInForbiddenPosition => {
            let pos = position_name.unwrap_or("unknown position");
            format!("spread is not allowed in {}", pos)
        }
        DiagnosticCode::MetaSpreadOnNonList => {
            let actual = other_type.unwrap_or("?");
            format!("spread expects List<T>; found {}", actual)
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-list diagnostic codes
        _ => panic!("meta_list_diagnostic_message called with non-Phase-A code"),
    }
}

/// Render the diagnostic message for Phase B / Phase F (meta-language) HOF, lambda,
/// pipe, reducer, ternary, and `smelt.config.var` diagnostic codes.
///
/// Parameters:
/// - `code`: one of the Phase B/F `DiagnosticCode` variants.
/// - `hof`: HOF name for `LambdaResultTypeMismatch`, `HofExpectsLambda`,
///   `LambdaArityMismatch` (e.g. `"map"`).
/// - `name`: function/reducer/variable/keyword name for `HofNameShadowed`,
///   `ReducerNameShadowed`, `ConfigVarNotFound`, `ConfigVarNullCoercion`,
///   `TernaryKeywordShadowed`, `LambdaDuplicateParameter`.
/// - `expected`: expected type or arity string.
/// - `actual`: actual type or arity string.
/// - `reducer`: reducer name for `ReducerInputTypeMismatch`, `ReducerEmptyNoIdentity`,
///   `ReducerArityMismatch`, `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`,
///   `ReducerNamedArgument`.
/// - `t_in`: expected input element type string for `ReducerInputTypeMismatch`; or
///   parameter name for `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`.
/// - `t_actual`: actual input element type string for `ReducerInputTypeMismatch`,
///   or actual type for `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`.
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_hof_diagnostic_message(
    code: DiagnosticCode,
    hof: Option<&str>,
    name: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
    reducer: Option<&str>,
    t_in: Option<&str>,
    t_actual: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::LambdaInForbiddenPosition => {
            "lambda is only valid as an argument to a higher-order function".to_string()
        }
        DiagnosticCode::LambdaArityMismatch => {
            let h = hof.unwrap_or("HOF");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!(
                "{} expects a lambda of arity {}; found arity {}",
                h, exp, act
            )
        }
        DiagnosticCode::LambdaZeroParameters => {
            "lambda must declare at least one parameter".to_string()
        }
        DiagnosticCode::LambdaDuplicateParameter => {
            let n = name.unwrap_or("?");
            format!(
                "parameter `{}` already appears in this lambda's parameter list",
                n
            )
        }
        DiagnosticCode::LambdaResultTypeMismatch => {
            let h = hof.unwrap_or("HOF");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("{} requires lambda result {}; found {}", h, exp, act)
        }
        DiagnosticCode::HofExpectsLambda => {
            let h = hof.unwrap_or("HOF");
            let act = actual.unwrap_or("?");
            format!("{} expects a lambda; found {}", h, act)
        }
        DiagnosticCode::HofExpectsReducer => {
            let act = actual.unwrap_or("?");
            format!("reduce expects a reducer; found {}", act)
        }
        DiagnosticCode::HofNameShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved higher-order function name", n)
        }
        DiagnosticCode::ReducerNameShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved reducer name", n)
        }
        DiagnosticCode::PipeRhsNotCall => {
            "pipe right-hand side must be a function call".to_string()
        }
        DiagnosticCode::PipeInDataPosition => {
            "|> is meta-only; use SQL composition in this position".to_string()
        }
        DiagnosticCode::ReducerInputTypeMismatch => {
            let r = reducer.unwrap_or("?");
            let ti = t_in.unwrap_or("?");
            let ta = t_actual.unwrap_or("?");
            format!("reducer {} expects List<{}>; found List<{}>", r, ti, ta)
        }
        DiagnosticCode::ReducerEmptyNoIdentity => {
            let r = reducer.unwrap_or("?");
            format!("reducer {} has no identity for an empty list", r)
        }
        DiagnosticCode::ReducerArityMismatch => {
            let r = reducer.unwrap_or("?");
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("reducer {} expects {} argument(s); found {}", r, exp, act)
        }
        DiagnosticCode::ReducerArgTypeMismatch => {
            let r = reducer.unwrap_or("?");
            let param = t_in.unwrap_or("?");
            let exp = expected.unwrap_or("?");
            let act = t_actual.unwrap_or("?");
            format!(
                "reducer {}'s argument `{}` expects {}; found {}",
                r, param, exp, act
            )
        }
        DiagnosticCode::ReducerArgNotCompileTime => {
            let r = reducer.unwrap_or("?");
            let param = t_in.unwrap_or("?");
            let act = t_actual.unwrap_or("?");
            format!(
                "reducer {}'s argument `{}` must be a compile-time value; found {}",
                r, param, act
            )
        }
        DiagnosticCode::ReducerNamedArgument => {
            let r = reducer.unwrap_or("?");
            format!("reducer {} takes positional arguments only", r)
        }
        DiagnosticCode::HofNamedArgument => {
            let h = hof.unwrap_or("HOF");
            format!(
                "{} takes positional arguments only; named arguments are not supported",
                h
            )
        }
        DiagnosticCode::TernaryConditionNotBoolean => {
            let act = actual.unwrap_or("?");
            format!("ternary condition expects Boolean; found {}", act)
        }
        DiagnosticCode::TernaryBranchTypeMismatch => {
            // t_in = then_type, t_actual = else_type
            let then_ty = t_in.unwrap_or("?");
            let else_ty = t_actual.unwrap_or("?");
            format!(
                "ternary branches have incompatible types: {} vs {}",
                then_ty, else_ty
            )
        }
        DiagnosticCode::TernaryKeywordShadowed => {
            let n = name.unwrap_or("?");
            format!("{} is a reserved meta-language keyword", n)
        }
        DiagnosticCode::TernaryInDataPosition => {
            "if-then-else is meta-only; use SQL CASE WHEN in this position".to_string()
        }
        DiagnosticCode::TernaryDanglingThen => {
            "unexpected `then` keyword outside of `if ... then ...` form".to_string()
        }
        DiagnosticCode::TernaryDanglingElse => {
            "unexpected `else` keyword outside of `... then ... else` form".to_string()
        }
        DiagnosticCode::ConfigVarNotFound => {
            let n = name.unwrap_or("?");
            format!("compile-time variable {} not declared in smelt.yml vars", n)
        }
        DiagnosticCode::ConfigVarNameNotLiteral => {
            "smelt.config.var name must be a string literal".to_string()
        }
        DiagnosticCode::ConfigVarNullCoercion => {
            let n = name.unwrap_or("?");
            format!(
                "null variable {} coerced to empty string; declare a default in smelt.yml",
                n
            )
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-HOF diagnostic codes
        _ => panic!("meta_hof_diagnostic_message called with non-Phase-B/F code"),
    }
}

/// Render the diagnostic message for Phase C (meta-language) reflection diagnostic codes.
///
/// Parameters:
/// - `code`: one of the four Phase C `DiagnosticCode` variants.
/// - `actual`: the actual synthesised type (for `ColumnsOfRequiresTableExpr`).
/// - `field_name`: the unknown field name (for `ColumnRefFieldUnknown`).
/// - `table_expr`: the text of the table expression (for `ColumnsOfUnresolvableSchema`).
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic
/// codes (new in Phase C)".
pub fn meta_reflection_diagnostic_message(
    code: DiagnosticCode,
    actual: Option<&str>,
    field_name: Option<&str>,
) -> String {
    meta_reflection_diagnostic_message_with_table_expr(code, actual, field_name, None)
}

/// Extended form of [`meta_reflection_diagnostic_message`] that also accepts a
/// `table_expr` string for the `ColumnsOfUnresolvableSchema` variant.
pub fn meta_reflection_diagnostic_message_with_table_expr(
    code: DiagnosticCode,
    actual: Option<&str>,
    field_name: Option<&str>,
    table_expr: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::ColumnsOfRequiresTableExpr => {
            let act = actual.unwrap_or("?");
            format!("smelt.columns_of expects TableExpr; found {act}")
        }
        DiagnosticCode::ColumnsOfNamedArgument => {
            "smelt.columns_of takes one positional argument; named arguments are not supported"
                .to_string()
        }
        DiagnosticCode::ColumnRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!(
                "ColumnRef has no field {name}; expected one of: \
                 name, type, is_numeric, is_decimal, is_string, is_temporal, is_integer, is_boolean"
            )
        }
        DiagnosticCode::ColumnsOfUnresolvableSchema => {
            let t = table_expr.unwrap_or("t");
            format!("cannot resolve column list for {t}; upstream schema is unknown")
        }
        // Phase D diagnostic messages
        DiagnosticCode::WithTagRequiresText => {
            let act = actual.unwrap_or("?");
            format!("with_tag expects a compile-time Text; found {act}")
        }
        DiagnosticCode::WithTagNamedArgument => {
            "with_tag takes one positional argument; named arguments are not supported".to_string()
        }
        DiagnosticCode::WideReflectionUnknownAccessor => {
            // `actual` carries the namespace ("models" or "sources"),
            // `field_name` carries the unknown accessor name.
            let ns = actual.unwrap_or("models");
            let name = field_name.unwrap_or("?");
            format!("smelt.{ns} has no accessor `{name}`; expected one of: with_tag, all")
        }
        DiagnosticCode::WideReflectionUnexpectedArgument => {
            // `actual` carries the full accessor name ("smelt.models.all", etc.).
            let accessor = actual.unwrap_or("all");
            format!("{accessor} takes no arguments")
        }
        DiagnosticCode::ModelRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!("ModelRef has no field `{name}`; expected one of: path, name, tags, columns")
        }
        DiagnosticCode::SourceRefFieldUnknown => {
            let name = field_name.unwrap_or("?");
            format!("SourceRef has no field `{name}`; expected one of: path, name, tags, columns")
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-reflection diagnostic codes
        _ => panic!(
            "meta_reflection_diagnostic_message called with non-Phase-C/D code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) record diagnostic codes.
///
/// Parameters vary by code — see each variant's doc comment for the placeholders.
/// All `Option<&str>` parameters default to `"?"` when `None`.
///
/// Returns the exact message string specified in `meta_language.md`
/// §"Record diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_record_diagnostic_message(
    code: DiagnosticCode,
    type_name: Option<&str>,
    field_name: Option<&str>,
    path: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
    fields: Option<&str>,
) -> String {
    let ty = type_name.unwrap_or("?");
    let name = field_name.unwrap_or("?");
    match code {
        DiagnosticCode::SmeltRecordRedefinition => {
            let p = path.unwrap_or("?");
            format!(
                "record `{ty}` is already declared in {p}; record names must be unique workspace-wide"
            )
        }
        DiagnosticCode::RecordFieldUnknown => {
            let fs = fields.unwrap_or("?");
            format!("record `{ty}` has no field `{name}`; expected one of: {fs}")
        }
        DiagnosticCode::RecordFieldMissing => {
            format!("record literal for `{ty}` is missing required field `{name}`")
        }
        DiagnosticCode::RecordFieldDuplicate => {
            format!("field `{name}` already appears in this record literal")
        }
        DiagnosticCode::RecordFieldTypeMismatch => {
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("record field `{name}` expects {exp}; found {act}")
        }
        DiagnosticCode::RecordLiteralUnknownTarget => {
            "cannot infer record type from context; annotate the target type".to_string()
        }
        DiagnosticCode::RecordFieldNotProjectable => {
            format!("value of type {ty} has no fields; projection `{name}` is not valid")
        }
        DiagnosticCode::RecordFieldTypeForbidden => {
            format!(
                "record field types may not reference {ty}; reflection witnesses are not user-writable"
            )
        }
        DiagnosticCode::RecordCyclicDeclaration => {
            format!(
                "record `{ty}` forms a cycle; recursive record declarations are not supported in v1"
            )
        }
        DiagnosticCode::RecordInDataWorld => {
            "record-typed value may not appear in a Data-World (SQL) position; use field projection to produce a spliced value".to_string()
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-record diagnostic codes
        _ => panic!(
            "meta_record_diagnostic_message called with non-record code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) map diagnostic codes.
///
/// Parameters vary by code:
/// - `method`: the Map API method name (for arity/named-arg/unexpected-arg codes).
/// - `name`: the unknown method name (for `MapApiUnknown`).
/// - `key`: the missing key (for `MapGetMissingKey`).
/// - `n`: the actual argument count as a string (for `MapApiArityMismatch`).
/// - `expected`: expected key type (for `MapApiArgTypeMismatch`).
/// - `actual`: actual type/found value (various).
///
/// Returns the exact message string specified in `meta_language.md`
/// §"Map diagnostic codes".
#[allow(clippy::too_many_arguments)]
pub fn meta_map_diagnostic_message(
    code: DiagnosticCode,
    method: Option<&str>,
    name: Option<&str>,
    key: Option<&str>,
    n: Option<&str>,
    type_name: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
) -> String {
    let m = method.unwrap_or("?");
    match code {
        DiagnosticCode::MapKeyTypeNotText => {
            let ty = type_name.unwrap_or("?");
            format!("Map key type must be Text in v1; found {ty}")
        }
        DiagnosticCode::MapApiUnknown => {
            let n = name.unwrap_or("?");
            format!("Map has no method `{n}`; expected one of: entries, keys, values, get, has")
        }
        DiagnosticCode::MapApiArityMismatch => {
            let count = n.unwrap_or("?");
            format!("Map.{m} expects one positional argument; found {count}")
        }
        DiagnosticCode::MapApiNamedArgument => {
            format!("Map.{m} does not support named arguments")
        }
        DiagnosticCode::MapApiUnexpectedArgument => {
            format!("Map.{m} takes no arguments")
        }
        DiagnosticCode::MapGetMissingKey => {
            let k = key.unwrap_or("?");
            format!("Map has no binding for key `{k}`")
        }
        DiagnosticCode::MapApiArgTypeMismatch => {
            let exp = expected.unwrap_or("?");
            let act = actual.unwrap_or("?");
            format!("Map.{m} expects key of type {exp}; found {act}")
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-map diagnostic codes
        _ => panic!(
            "meta_map_diagnostic_message called with non-map code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for Phase E1 (meta-language) loader diagnostic codes.
///
/// Parameters vary by code:
/// - `expr`: the expression text (for `ConfigLoaderPathNotLiteral`).
/// - `path`: the path text (for path-related codes).
/// - `format`: the file format name (for `ConfigLoaderParseError`).
/// - `parser_error`: the parser error string (for `ConfigLoaderParseError`).
/// - `name`: field name (for field-related codes).
/// - `fields`: comma-separated list of valid fields (for `ConfigLoaderUnknownField`).
/// - `expected_type`, `actual_type`: type strings (for `ConfigLoaderTypeMismatch`,
///   `ConfigLoaderRootShapeMismatch`).
/// - `expected_shape`, `actual_shape`: shape strings (for `ConfigLoaderRootShapeMismatch`).
/// - `key`: the duplicate key (for `ConfigLoaderDuplicateMapKey`).
/// - `row`, `first_row`: row references (for `ConfigLoaderDuplicateMapKey`,
///   `ConfigLoaderNullCoercion`).
///
/// Returns the exact message string specified in `meta_config_loading.md`
/// §"Validation diagnostics".
#[allow(clippy::too_many_arguments)]
pub fn meta_loader_diagnostic_message(
    code: DiagnosticCode,
    expr: Option<&str>,
    path: Option<&str>,
    format: Option<&str>,
    parser_error: Option<&str>,
    name: Option<&str>,
    fields: Option<&str>,
    expected_type: Option<&str>,
    actual_type: Option<&str>,
    expected_shape: Option<&str>,
    actual_shape: Option<&str>,
    key: Option<&str>,
    row: Option<&str>,
    first_row: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::ConfigLoaderPathNotLiteral => {
            let e = expr.unwrap_or("?");
            format!("loader path must be a string literal; found {e}")
        }
        DiagnosticCode::ConfigLoaderPathEscapesWorkspace => {
            let p = path.unwrap_or("?");
            format!("loader path must be a workspace-relative path; found {p}")
        }
        DiagnosticCode::ConfigLoaderPathBackslash => {
            let p = path.unwrap_or("?");
            format!(r"loader paths use `/` as the path separator; found `\` in {p}")
        }
        DiagnosticCode::ConfigLoaderFileNotFound => {
            let p = path.unwrap_or("?");
            format!("loader file `{p}` not found in workspace")
        }
        DiagnosticCode::ConfigLoaderSchemaForbidden => {
            let act = actual_type.unwrap_or("?");
            format!("loader schema must be a record type, `List<record>`, or `Map<Text, record>`; found {act}")
        }
        DiagnosticCode::ConfigLoaderTomlNotYetSupported => {
            "smelt.config.load_toml is reserved; only YAML and JSON loaders are supported in v1"
                .to_string()
        }
        DiagnosticCode::ConfigLoaderParseError => {
            let fmt = format.unwrap_or("?");
            let p = path.unwrap_or("?");
            let err = parser_error.unwrap_or("?");
            format!("failed to parse {fmt} file `{p}`: {err}")
        }
        DiagnosticCode::ConfigLoaderRequiredFieldMissing => {
            let n = name.unwrap_or("?");
            format!("field `{n}` required by schema is missing")
        }
        DiagnosticCode::ConfigLoaderUnknownField => {
            let n = name.unwrap_or("?");
            let fs = fields.unwrap_or("?");
            format!("field `{n}` is not declared in the schema; expected one of: {fs}")
        }
        DiagnosticCode::ConfigLoaderTypeMismatch => {
            let n = name.unwrap_or("?");
            let exp = expected_type.unwrap_or("?");
            let act = actual_type.unwrap_or("?");
            format!("field `{n}` expects {exp}; got {act}")
        }
        DiagnosticCode::ConfigLoaderRootShapeMismatch => {
            let ty = expected_type.unwrap_or("?");
            let exp = expected_shape.unwrap_or("?");
            let act = actual_shape.unwrap_or("?");
            format!("schema `{ty}` expects {exp}; file's top level is {act}")
        }
        DiagnosticCode::ConfigLoaderDuplicateMapKey => {
            let k = key.unwrap_or("?");
            let r = row.unwrap_or("?");
            let fr = first_row.unwrap_or("?");
            format!("duplicate map key `{k}` at {r}; earlier appearance at {fr}")
        }
        DiagnosticCode::ConfigLoaderNullCoercion => {
            let r = row.unwrap_or("?");
            format!(
                "null value at {r} coerced to empty string; declare a default in the source file"
            )
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-loader diagnostic codes
        _ => panic!(
            "meta_loader_diagnostic_message called with non-loader code: {:?}",
            code
        ),
    }
}

/// Render the diagnostic message for multi-model production diagnostic codes.
///
/// Parameters vary by code — see the spec table in `meta_language.md`
/// §"Multi-model production diagnostic codes" for the full message formats.
///
/// All `Option<&str>` parameters default to `"?"` when `None`.
pub fn meta_multi_model_diagnostic_message(
    code: DiagnosticCode,
    value: Option<&str>,
    actual: Option<&str>,
    name: Option<&str>,
    smelt_path: Option<&str>,
    other_path: Option<&str>,
) -> String {
    match code {
        DiagnosticCode::GeneratesUnknownValue => {
            let v = value.unwrap_or("?");
            format!("generates must be `models`; found {v}")
        }
        DiagnosticCode::GeneratesMixedWithBareModel => {
            "generates: models cannot coexist with bare-model identity (name field or section delimiter)".to_string()
        }
        DiagnosticCode::GenerateFileBareSelectForbidden => {
            "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape".to_string()
        }
        DiagnosticCode::GenerateFileBodyTypeError => {
            let act = actual.unwrap_or("?");
            format!("generator file body must evaluate to List<ModelDef>; found {act}")
        }
        DiagnosticCode::ModelDefOutsideGeneratorFile => {
            "ModelDef literals are only valid inside a `generates: models` file body".to_string()
        }
        DiagnosticCode::ModelDefInvalidName => {
            let v = value.unwrap_or("?");
            format!("ModelDef.name must be a non-empty Text of [A-Za-z0-9_]+; found {v}")
        }
        DiagnosticCode::ModelDefInvalidMaterialization => {
            let v = value.unwrap_or("?");
            format!("ModelDef.materialization must be one of view, table, incremental; found {v}")
        }
        DiagnosticCode::ModelDefDuplicateName => {
            let n = name.unwrap_or("?");
            format!("duplicate ModelDef.name `{n}` in this generator file")
        }
        DiagnosticCode::ModelDefHandAuthoredCollision => {
            let sp = smelt_path.unwrap_or("?");
            let op = other_path.unwrap_or("?");
            format!("ModelDef emits `{sp}` which collides with {op}")
        }
        DiagnosticCode::GeneratorBodyForbidsModelReflection => {
            "smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references".to_string()
        }
        // invariant: unreachable from user input — caller is dispatched only for meta-multi_model diagnostic codes
        _ => panic!(
            "meta_multi_model_diagnostic_message called with non-E2 code: {:?}",
            code
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every multi-model diagnostic code exists in `DiagnosticCode` and its
    /// rendered message matches the spec table verbatim.
    #[test]
    fn diagnostic_codes_multi_model_set_complete() {
        // GeneratesUnknownValue
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::GeneratesUnknownValue,
            Some("views"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(msg, "generates must be `models`; found views");

        // GeneratesMixedWithBareModel
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::GeneratesMixedWithBareModel,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "generates: models cannot coexist with bare-model identity (name field or section delimiter)"
        );

        // GenerateFileBareSelectForbidden
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::GenerateFileBareSelectForbidden,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape"
        );

        // GenerateFileBodyTypeError
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::GenerateFileBodyTypeError,
            None,
            Some("List<Text>"),
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "generator file body must evaluate to List<ModelDef>; found List<Text>"
        );

        // ModelDefOutsideGeneratorFile
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::ModelDefOutsideGeneratorFile,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "ModelDef literals are only valid inside a `generates: models` file body"
        );

        // ModelDefInvalidName
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::ModelDefInvalidName,
            Some("bad-name!"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "ModelDef.name must be a non-empty Text of [A-Za-z0-9_]+; found bad-name!"
        );

        // ModelDefInvalidMaterialization
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::ModelDefInvalidMaterialization,
            Some("external"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "ModelDef.materialization must be one of view, table, incremental; found external"
        );

        // ModelDefDuplicateName
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::ModelDefDuplicateName,
            None,
            None,
            Some("my_model"),
            None,
            None,
        );
        assert_eq!(
            msg,
            "duplicate ModelDef.name `my_model` in this generator file"
        );

        // ModelDefHandAuthoredCollision
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::ModelDefHandAuthoredCollision,
            None,
            None,
            None,
            Some("models/revenue.sql"),
            Some("models/revenue.sql"),
        );
        assert_eq!(
            msg,
            "ModelDef emits `models/revenue.sql` which collides with models/revenue.sql"
        );

        // GeneratorBodyForbidsModelReflection
        let msg = meta_multi_model_diagnostic_message(
            DiagnosticCode::GeneratorBodyForbidsModelReflection,
            None,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            msg,
            "smelt.models.* is not available inside a generator body; use smelt.sources.* or literal smelt.<path> references"
        );
    }
}
