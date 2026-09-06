//! Diagnostic types and message builders for smelt-db.
//!
//! Pure data types and pure functions. No Salsa dependency.

use rowan::TextRange;

/// Diagnostic codes for pattern-matching in code actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    ParseError,
    /// Top-level content remains after the declarations and the (at most one)
    /// model body have been parsed — e.g. a second `SELECT`, stray tokens
    /// after the query, or the tail of a construct the grammar does not
    /// support. The leftover tokens are wrapped in an `ERROR` node in the
    /// CST; they are never absorbed silently.
    TrailingTopLevelContent,
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
    /// `ModelDef.timeseries` or `ModelDef.safety_overrides` is present on a
    /// `ModelDef` literal whose `materialization` is not `'incremental'`.
    /// Anchored at the offending field's name token.
    /// Message: "ModelDef.{field} is only valid when materialization is 'incremental'"
    ModelDefOverrideRequiresIncremental,

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
    /// A model declares `refresh: batched` without a sibling `timeseries:` block.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "TimeseriesRequiredForPartitionGrain: model declares `refresh: batched` but has no `timeseries:` block — add a `timeseries:` block with event_time_column, partition_column, and granularity"
    TimeseriesRequiredForPartitionGrain,
    /// The `timeseries:` block parses but violates a structural rule.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "MalformedTimeseries: {message}"
    MalformedTimeseries,
    /// A `columns.<c>.contract: plausible` declaration names a column that
    /// also serves as the model's `event_time_column`, `partition_column`,
    /// or a `unique_key` member.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "PlausibleContractOnSkeletonColumn: `columns.{column}.contract: plausible` cannot be declared — '{column}' is {role}, which must stay deterministic"
    PlausibleContractOnSkeletonColumn,
    /// A `functional_dependencies:` entry is structurally invalid: an empty
    /// `key`/`determines`, a `determines` column also listed in `key`, or a
    /// `key`/`determines` column absent from the model's SQL body.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "MalformedFunctionalDependency: {message}"
    MalformedFunctionalDependency,
    /// A `bounded_domain:` declaration is structurally invalid: an absent
    /// (already a YAML parse error) or non-positive `max_cardinality`, an
    /// empty `column`, or a `column` absent from the model's SQL body.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "MalformedBoundedDomain: {message}"
    MalformedBoundedDomain,
    /// A model declares `refresh: incremental` without a sibling `grain:`.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "GrainRequiredForIncremental: model declares `refresh: incremental` but declares neither `timeseries:` nor `unique_key:` — add at least one shape-defining fact (or the check-only `grain: partition | key` assertion)"
    GrainRequiredForIncremental,
    /// A model declares `grain:` without `refresh: incremental`.
    /// Anchored at the top of the file (line 0, column 0).
    /// Message: "GrainRequiresIncremental: model declares `grain:` but is not `refresh: incremental` — add `refresh: incremental` or remove the `grain:` key"
    GrainRequiresIncremental,
    /// A written `grain:` check-only assertion disagrees with the label
    /// derived from the declared shape-defining facts (`timeseries:` /
    /// `unique_key:`). Anchored at the top of the file (line 0, column 0).
    /// Message: "GrainAssertionMismatch: declared `grain: {asserted}` disagrees with the grain derived from the declared shape facts (`grain: {derived}`) — fix the `grain:` assertion or the facts it derives from"
    GrainAssertionMismatch,

    // ── Declarative column test diagnostic codes (docs/specs/data_tests.md) ──
    /// A `columns.<c>.tests` entry does not match `not_null`, `unique`,
    /// `accepted_values`, or `relationships`. Anchored at the offending
    /// entry (currently the top of the file — precise per-entry anchoring
    /// is not yet wired).
    /// Message: "UnknownColumnTestKind: column '{column}' has a `tests` entry '{entry}' which is not one of the recognized kinds (not_null, unique, accepted_values, relationships)"
    UnknownColumnTestKind,
    /// A `columns.<c>.tests` entry names a column absent from the model's
    /// inferred output schema. Anchored at the column key (currently the
    /// top of the file — precise per-entry anchoring is not yet wired).
    /// Message: "ColumnTestOnUnknownColumn: model '{model}' declares tests on column '{column}' which is absent from the model's inferred output schema"
    ColumnTestOnUnknownColumn,

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
    /// A `refresh: keyed` SELECT has no GROUP BY (the key columns).
    KeyedRequiresGroupBy,
    /// A `refresh: keyed` projection uses a non-allowlisted aggregator or
    /// a composite expression over aggregates.
    KeyedUnknownCombiner,
    /// The `refresh: keyed` GROUP BY contains the driving source's
    /// `partition_column` (a per-partition shape, not the keyed one).
    KeyedGroupByContainsPartitionColumn,
    /// Window functions (`OVER (...)`) are not allowed in a `refresh: keyed` model.
    KeyedForbidsWindowFunctions,
    /// A non-deterministic function appears in a `refresh: keyed` SELECT.
    KeyedForbidsNondeterministic,
    /// A `refresh: keyed` model has no clocked driving source, and no
    /// single unambiguous source could be resolved to derive the
    /// snapshot-reconcile run shape either.
    KeyedSnapshotPostureUnsupported,
    /// A fold-family column (additive, extremal/lattice, or order-monotone
    /// overwrite) is refused under the derived snapshot-reconcile run shape
    /// (`docs/specs/incremental_shapes.md` §"Admission matrix").
    KeyedSnapshotSourceUnsupportedColumn,
    /// Multiple timeseries-tagged sources in a `refresh: keyed` model's FROM
    /// (v1 supports exactly one driving source).
    KeyedMultipleDrivingSources,
    /// A `refresh: keyed` SELECT could not be parsed for classification.
    KeyedSqlNotParseable,
    /// A `refresh: keyed` `COALESCE`-shaped once-write column has no
    /// once-write provenance proof (`incremental_shapes.md` §"The
    /// column-family catalogue"). Names the column and the three fixes:
    /// key-derived form, declared functional dependency, or remodelling.
    KeyedOnceWriteUnproven,
    /// A hidden decomposed-state column (`docs/specs/incremental_models.md`
    /// §"Decomposed state (rung 2) in keyed models") collides with a
    /// user-declared or projected output column of the same name.
    KeyedStateColumnCollision,
    /// A `refresh: keyed` model incorrectly declares a `timeseries:` block
    /// (key temporal locality is not established). The keyed output has no
    /// partition column by default; the rule reads it from the driving
    /// source. Anchored at offset 0. Error severity.
    KeyedForbidsTimeseries,
    /// A `refresh: keyed` model's route-3 statically-derived recurrence
    /// bound disagrees with a declared `key_recurrence` over the same key
    /// (key-grain rule 16, `incremental_shapes.md` §"Key temporal
    /// locality"). Names both values; the derived value is authoritative.
    /// Anchored at offset 0. Error severity.
    KeyedRecurrenceDeclarationMismatch,
    /// A key-addressed model (`grain: key`, resolved) declares
    /// `safety_overrides:` (top-level or the folded `batched.safety_overrides`
    /// sub-block). A keyed model has no partition-shaped output for a safety
    /// override to apply to. Anchored at offset 0. Error severity.
    KeyedForbidsSafetyOverrides,
    /// A `refresh: materialized_view` model incorrectly declares a
    /// `timeseries:` block. Like `keyed`, the engine-maintained output
    /// has no partition column. Anchored at offset 0. Error severity.
    MaterializedViewForbidsTimeseries,
    /// A `refresh: materialized_view` model incorrectly declares a
    /// `batched:` block. The engine, not smelt, owns freshness for this
    /// mode. Anchored at offset 0. Error severity.
    MaterializedViewForbidsPartitionGrain,
    /// Advisory (`Warning`): a `batched` model's SQL is not batch-safe
    /// under the planner's batch safety classifier (the build does not
    /// hard-refuse — its dispatch falls back to a safe chunking strategy).
    PartitionGrainNotSafe,
    /// An incremental model's `event_time_column` is not accessible at the
    /// outermost SELECT where the time filter is injected — either because the
    /// query is a set operation (UNION/INTERSECT/EXCEPT) or because the FROM
    /// clause is a subquery that does not project the column. Error severity.
    EventTimeColumnNotVisibleAtOuterSelect,
    /// A `grain: partition` model's body calls `smelt.metric(...)`. The
    /// composition of metric expansion with time-filter injection is
    /// deliberately unspecified (`incremental_shapes.md` §"Functions inside
    /// partition-grain bodies"), so the combination refuses ahead of
    /// execution rather than composing unpredictably. Error severity.
    PartitionGrainForbidsMetrics,
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
    /// Emitted when a `|>` in a FROM-first pipe query is followed by a token
    /// that is not a recognised pipe operator keyword.
    /// Message: `unknown pipe operator '<kw>'`.
    /// Anchored at the unrecognised token span.
    PipeUnknownOperator,
    /// Emitted when a `|>` in a FROM-first pipe query is followed by a
    /// recognised-but-deferred operator (`PIVOT`/`UNPIVOT`/`WINDOW`/`CALL`/
    /// `TABLESAMPLE`/`ASSERT`). Using a deferred operator is a hard error.
    /// Message: `pipe operator '<kw>' is not supported — <reason>`.
    /// Anchored at the operator keyword span.
    PipeOperatorUnsupported,
    /// Emitted when a pipe stage body does not parse against the operator's
    /// clause grammar (e.g. `|> WHERE` with no predicate expression).
    /// Message: `malformed '<kw>' pipe stage`.
    /// Anchored at the stage span.
    PipeStageMalformed,
    /// Emitted when no maintenance technique survives a plan cell's
    /// admission (`incremental_models.md` §"Per-cell admission"). Names the
    /// cell's trigger and why every candidate technique was refused —
    /// includes the `maintenance.cells[]` two-group column-span error (a
    /// cell whose declared `columns` span more than one derived column
    /// group can never address a single coherent cell). Anchored at the
    /// model SQL body start.
    MaintenanceNoAdmissibleTechnique,
    /// Emitted (the K8 guardrail) when a derived scan or write footprint
    /// cannot be partition-bounded and no `allow_full_scan` acceptance was
    /// declared for that source (`incremental_models.md` §"Partition-local
    /// maintenance (the K8 guardrail)"). Anchored at the model SQL body
    /// start.
    MaintenanceScanUnbounded,
    /// Emitted (Error) when a model's definition-change `Trigger::
    /// ColumnAdded` names a column that occupies a row-membership/identity
    /// (skeleton) position — a grain change, never a column backfill
    /// (EX-39, `definition_deltas.md` §"The verdict per column group").
    /// Anchored at the model SQL body start.
    MaintenanceSkeletonChanged,
    /// Emitted (Error) when a maintained model's declared
    /// `timeseries.partition_column` differs from the address recorded in
    /// the deployed-schema snapshot at last deploy — the address every
    /// partition-grain maintenance write targets
    /// (`docs/specs/incremental_shapes.md` §"The partition grain"). Names
    /// both the recorded and current column; the remedy is
    /// `--full-refresh` or `smelt migrate`. Anchored at the model SQL body
    /// start.
    MaintenancePartitionColumnChanged,
    /// Emitted (Warning) when a model's definition-change `Trigger::
    /// ColumnAdded` names a non-skeleton column that cannot be backfilled in
    /// place (unbounded scan, no admissible technique, unresolvable
    /// expression, or group disagreement) — the run still proceeds, ALTERing
    /// the column in and leaving historical rows `NULL` until `smelt
    /// migrate` backfills them (`definition_deltas.md` §"Detection").
    /// Anchored at the model SQL body start.
    MaintenanceColumnAddNotBackfillable,
    /// Emitted (Error) when a model's declared `timeseries.granularity`
    /// disagrees with the truncation/grid unit its own `partition_column`
    /// SELECT-list projection actually derives to (e.g. declaring `day`
    /// while the SQL groups on `date_trunc('hour', …)`) —
    /// (`incremental_models.md` §Design "Grain is declared": the graph
    /// layer's edge grain is the declaration, never derived, but the
    /// classifier checks the declaration against the SQL's own grouping).
    /// Anchored at the model SQL body start.
    MaintenanceGranularityMismatch,
    /// Emitted (Error) when a model declares `refresh: incremental` with a
    /// `grain:` maintenance-plan derivation does not yet support (currently
    /// `key_per_partition`) — refused fail-loud rather than silently
    /// collapsed into an ordinary keyed plan with an empty `unique_key`
    /// (`incremental_models.md` §Known Divergences). Names the grain and the
    /// plan tracking the missing support. Anchored at the model SQL body
    /// start.
    MaintenanceUnsupportedGrain,
    /// Emitted (Error) when a `maintenance.cells[].write` pin names a write
    /// pattern the open registry does not recognise, or one the target
    /// backend's write-pattern capability does not provide
    /// (`incremental_models.md` §"Per-cell write addressing" → "User
    /// pins"). Names the pattern and the backend; never a silent
    /// downgrade to a different addressing. Anchored at the model SQL body
    /// start.
    MaintenanceWritePatternUnavailable,
    /// Emitted (Error) when a `maintenance.cells[].write` pin names a
    /// write pattern the registry recognises and the target backend can
    /// execute, but the pinned cell's own facts cannot uphold that
    /// pattern's equivalence obligation (e.g. `write: keyed` on an output
    /// with no declared identity) — `incremental_models.md` §"Per-cell
    /// write addressing" → "User pins". Names the cell and the refused
    /// pattern; the pin never silently resolves to a substituted
    /// technique. Anchored at the model SQL body start.
    MaintenanceWriteAddressingRefused,
    /// Emitted (Error) when `smelt run` would fold a data delta over an
    /// unapproved, non-eclipsed definition delta — the recorded and current
    /// definitions diverge and no matching plan-hash approval is on record
    /// (`definition_deltas.md` §Detection). The fix is `smelt migrate
    /// <model>` (review) then `--apply`, or `--full-refresh`. Never fires
    /// for a `--full-refresh` run, which is not a fold. Anchored at the
    /// model SQL body start.
    DefinitionDeltaPending,

    // ── Contract lattice diagnostic codes ────────────────────────────────────
    /// A `contract.frozen_horizon` is unparseable or declared on a
    /// non-partition-grain model (`incremental_models.md` §"Contract
    /// relaxations (`contract:`)"). Covers both the frontmatter-parse-time
    /// format failure (`smelt_core::metadata::MetadataError::
    /// ContractFrozenHorizonInvalid`) and the grain-admissibility check made
    /// by `smelt_logical::contract::frozen_horizon::validate_frozen_horizon`.
    /// Anchored at the top of the file (line 0, column 0).
    ContractFrozenHorizonInvalid,
    /// A `contract.deferral` (model-level or `contract.cells[].deferral`) is
    /// unparseable or negative, or declared with no interval-representable
    /// clock to measure lag against (`incremental_models.md` §"Contract
    /// relaxations (`contract:`)"). Covers both the frontmatter-parse-time
    /// format failure (`smelt_core::metadata::MetadataError::
    /// ContractDeferralInvalid`) and the clock-admissibility check made by
    /// `smelt_logical::contract::deferral::validate_deferral`. Anchored at
    /// the top of the file (line 0, column 0).
    ContractDeferralInvalid,
    /// A `contract.retain_departed` is neither a bare bool nor
    /// `{tombstone: <col>}`, is declared on anything other than a keyed
    /// shape consuming a mutable snapshot, or names a tombstone column
    /// absent from the model's output (`incremental_models.md` §"Contract
    /// relaxations (`contract:`)"). Covers both the frontmatter-parse-time
    /// format failure (`smelt_core::metadata::MetadataError::
    /// ContractRetainDepartedInvalid`) and the posture/tombstone-column
    /// check made by
    /// `smelt_logical::contract::retain_departed::validate`. Anchored at the
    /// top of the file (line 0, column 0).
    ContractRetainDepartedInvalid,

    /// A user-written top-level SELECT-item alias begins with the reserved
    /// `_smelt_` prefix (`multi_backend.md` §"Output-schema type
    /// conformance"). The prefix is reserved for smelt's own generated
    /// identifiers — most visibly the synthesized `_smelt_col{n}` alias a
    /// nameless projection item receives — and a user alias colliding with
    /// it would make that synthesis ambiguous. Anchored at the alias token.
    /// Error severity.
    ReservedProjectionAliasPrefix,

    /// A model uses a built-in or operator the registry declares
    /// `Emission::Unsupported` on the selected backend's dialect
    /// (`multi_backend.md` §"Operator lowering"). The compiler refuses rather
    /// than emitting SQL the engine will reject — or, worse, accept with
    /// different semantics — at runtime. Carries the registry's own reason
    /// text, which names the construct and suggests the portable spelling.
    /// Anchored at the offending expression. Error severity.
    UnsupportedOnBackend,

    /// Emitted (Error) when a `grain: key` model folds a retractable
    /// enrichment-join contribution — a join whose per-key contribution
    /// feeds a decrementing aggregate or a value that must be un-seen — and
    /// the repair family cannot admit a per-group recompute for the
    /// retraction (`incremental_shapes.md` §"Enrichment joins",
    /// `incremental_models.md` §"The repair family"). Names the failing
    /// repair obligation. Never fires on join spelling alone — only a
    /// genuinely retractable contribution is refused; steers to
    /// `refresh: materialized_view` or DAG composition. Anchored at the
    /// model SQL body start.
    KeyedRetractableContribution,

    /// A model's property profile at the working tree is worse than at the
    /// baseline ref along one dimension (`property_diff.md` §"Direction").
    /// Editor-only: the LSP raises one `PropertyDowngrade` diagnostic per
    /// downgrade on a shifted model file, anchored at the shift's reported
    /// location; the CLI reports the same fact as a `▼` line in
    /// `smelt explain --diff` and via `--fail-on`, never as a diagnostic.
    /// Warning severity.
    PropertyDowngrade,

    /// `smelt explain --diff` was requested but the baseline could not be
    /// resolved: the project is not inside a git work tree, the ref does not
    /// resolve, or the resolved ref has no `smelt.yml` at the project's path
    /// (`property_diff.md` §"Baseline materialisation"). CLI only; exits
    /// `2`. Never falls back to an empty diff.
    PropertyDiffBaselineUnavailable,
    /// Emitted (Warning) when availability resolution
    /// (`smelt_logical::maintenance::availability::resolve_availability`)
    /// downgrades a plan cell's technique to its recompute-family
    /// equivalent because the state structure it needs has no available
    /// realisation on a declared backend — no ledger builder,
    /// `state.warehouse_tables: none`, or a posture that excludes it
    /// (`docs/specs/state.md` §"The degradation contract",
    /// §Diagnostics). Names the cell, the original technique, the missing
    /// structure, and the backend the downgrade was observed against.
    /// Printed by `smelt explain`. Anchored at the model SQL body start.
    MaintenanceStateDowngraded,
    /// Emitted (Error) when a declared contract-lattice point whose
    /// semantics require a state structure (e.g. `contract.deferral`'s
    /// ledger-measured lag) is declared in a project whose posture,
    /// backend, or `state.warehouse_tables: none` opt-out cannot supply it
    /// (`docs/specs/state.md` §Diagnostics). Names the declaration and the
    /// missing structure. Anchored at the model SQL body start.
    DeclaredContractRequiresState,

    // ── Succession grain ──────────────────────────────────────────────
    // `docs/specs/diagnostics.md` §"Succession grain": the ten refusal
    // reasons the keyed-succession leaf classifier
    // (`smelt_logical::analysis::succession::classify_keyed_succession`)
    // can return, plus the one advisory. Raised (Error, except the
    // advisory) from `smelt_logical::maintenance::Refusal::
    // SuccessionNotRecognized`'s classifier reason via `smelt-db`'s
    // `MaintenanceRefusal::SuccessionNotRecognized` mapping in
    // `queries/maintenance/refusal_diag.rs::diagnostic_for_refusal`, folded
    // into `check_file_diagnostics` exactly like every other
    // `Maintenance*` refusal — LSP and CLI see the same set. Anchored at
    // the model SQL body start. `SuccessionClockTie` (runtime) has no
    // variant here yet — it lands with phase 5's runtime dispatch.
    /// A window function in the projection is not `LEAD(t)`/`LAG(t)` over
    /// the clock column with the default offset of 1, or not a scalar
    /// expression over one.
    SuccessionWindowFunctionNotLead,
    /// Two or more window functions in the projection partition by
    /// different column sets, by a column set the classifier cannot
    /// resolve to a stable per-row key, or by a column not proven `NOT
    /// NULL`.
    SuccessionPartitionKeyMismatch,
    /// A window's `ORDER BY` column does not trace as a strictly monotone
    /// clock to the driving source's declared `event_time_column`, is not
    /// proven `NOT NULL`, or the `ORDER BY` is descending or carries a
    /// second sort key.
    SuccessionOrderNotMonotoneClock,
    /// A projected column that is not a window function (or an expression
    /// over one) is itself an aggregate, a further window function, or
    /// otherwise not row-local.
    SuccessionRowLocalColumnViolation,
    /// A key column or the clock column is not projected row-locally, so
    /// the derived `(k, t)` identity cannot be recovered from the
    /// presented table.
    SuccessionIdentityNotProjected,
    /// The `FROM` clause is not exactly one source reference — a join,
    /// CTE, subquery, or set operation is present.
    SuccessionSingleSourceOnly,
    /// The driving source does not declare `mutation_profile.kind:
    /// append_only`, or declares no `timeseries:` block.
    SuccessionDrivingSourceNotAppendOnly,
    /// A filter precedes the window projection but is not a deterministic
    /// row-local predicate over the driving source's own columns, or more
    /// than one such filter is present.
    SuccessionPreFilterNotRowLocal,
    /// A `QUALIFY` clause exists but is not exactly `QUALIFY NOT
    /// <row-local boolean column>`, the flag column is not proven `NOT
    /// NULL`, or a same-scope `WHERE` tests a window-derived column.
    SuccessionDeleteFilterMisplaced,
    /// Warning. The pre-window `WHERE` is a bare negated boolean column.
    /// Admitted unchanged; suggests `QUALIFY NOT <col>` if it is a CDC
    /// delete flag, since a flag filtered before the window never closes
    /// its predecessor's interval. Advisory only — never changes
    /// admission.
    SuccessionPreFilterNegatesFlag,
    /// `refresh: incremental` with no `unique_key`, no `timeseries:`, and a
    /// SQL shape none of the succession rules above individually names — a
    /// `DISTINCT`, `GROUP BY`, `HAVING`, `ORDER BY`, or `LIMIT` on the
    /// scope, or a model resembling no admitted grain.
    SuccessionPatternUnrecognized,
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

mod meta_messages;
pub use meta_messages::*;

#[cfg(test)]
mod tests;
