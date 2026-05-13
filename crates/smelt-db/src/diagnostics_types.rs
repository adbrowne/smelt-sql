//! Diagnostic types and message builders for smelt-db.
//!
//! Pure data types and pure functions. No Salsa dependency.

use crate::Range;

/// Diagnostic codes for pattern-matching in code actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    ParseError,
    InvalidModel,
    UndefinedModelRef,
    UndefinedSource,
    CannotInferType,
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

    // ── Phase B (meta-language) diagnostic codes ─────────────────────────
    /// Emitted when a `fn x => body` lambda appears outside a HOF positional
    /// argument position (e.g. top-level expression, list element, named-arg
    /// value). Anchored at the `fn` keyword span. Message:
    /// "lambda is only valid as an argument to a higher-order function".
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    LambdaInForbiddenPosition,
    /// Emitted when a lambda with more than one parameter (`fn (a, b) => body`)
    /// is used in Phase B. Multi-arg lambdas are reserved for Phase F.
    /// Message: "multi-argument lambdas are not supported in v1; use a single parameter".
    /// Anchored at the lambda parameter list span.
    /// Introduced in Phase 3 of the meta-language plan (Phase B).
    LambdaArityNotSupported,
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
    pub range: Range,
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
        _ => panic!("meta_list_diagnostic_message called with non-Phase-A code"),
    }
}

/// Render the diagnostic message for Phase B (meta-language) HOF, lambda, pipe,
/// reducer, and `smelt.config.var` diagnostic codes.
///
/// Parameters:
/// - `code`: one of the fourteen Phase B `DiagnosticCode` variants.
/// - `hof`: HOF name for `LambdaResultTypeMismatch`, `HofExpectsLambda` (e.g. `"map"`).
/// - `name`: function/reducer/variable name for `HofNameShadowed`, `ReducerNameShadowed`,
///   `ConfigVarNotFound`, `ConfigVarNullCoercion`.
/// - `expected`: expected type string for `LambdaResultTypeMismatch`.
/// - `actual`: actual type string for `LambdaResultTypeMismatch`, `HofExpectsLambda`,
///   `HofExpectsReducer`.
/// - `reducer`: reducer name for `ReducerInputTypeMismatch`, `ReducerEmptyNoIdentity`.
/// - `t_in`: expected input element type string for `ReducerInputTypeMismatch`.
/// - `t_actual`: actual input element type string for `ReducerInputTypeMismatch`.
///
/// Returns the exact message string specified in `meta_language.md` §"Diagnostic
/// codes (new in Phase B)".
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
        DiagnosticCode::LambdaArityNotSupported => {
            "multi-argument lambdas are not supported in v1; use a single parameter".to_string()
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
        _ => panic!("meta_hof_diagnostic_message called with non-Phase-B code"),
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
            format!("ColumnRef has no field {name}; expected one of: name, type, is_numeric")
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
        _ => panic!(
            "meta_loader_diagnostic_message called with non-loader code: {:?}",
            code
        ),
    }
}

