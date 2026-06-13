---
feature: diagnostics
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Diagnostics

> **What this is.** The cross-feature diagnostic-code catalogue: the index of every `DiagnosticCode` smelt can emit, with each code's severity and canonical trigger. Per-code *semantics* — when a code fires and what it anchors to — are owned by the feature spec each catalogue row cites; this spec is the registry those specs must agree with, not the owner of any single code's behaviour.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

Diagnostics are surfaced through two paths:
- **LSP**: the language server reports them in real time as the user edits.
- **CLI**: `smelt build` / `smelt run` / `smelt type` print them and set a
  non-zero exit code when any Error-severity diagnostic is present.

Every diagnostic carries:
- **Severity** — `Error`, `Warning`, `Info`, or `Hint`.
- **Code** — a `DiagnosticCode` variant (enables code-action lookups and stable
  cross-references).
- **Range** — a `rowan::TextRange` (byte offsets into the source file).

### Fail-loud invariants

The diagnostic system enforces a *fail-loud* discipline:

1. Every path that can encounter an unrecognisable user input **must** emit a
   diagnostic rather than silently falling back to an inferred or unknown value.
2. Specifically, every `DataType::Unknown` site in production code is either
   classified as *legitimate* (a deliberate meta-language placeholder) or
   covered by a diagnostic (the guard test
   `crates/smelt-types/tests/unknown_census.rs` enforces this).

## Catalogue

The full catalogue of `DiagnosticCode` variants follows. Variants are grouped
by owning feature. The coverage gate
`crates/smelt-db/tests/diagnostics_catalogue.rs::every_diagnostic_code_is_catalogued`
asserts every variant in the enum appears here (wrapped in backticks).

---

### Models & core analysis

Owned by `docs/specs/models.md` and the core analysis queries in
`crates/smelt-db/src/queries/`.

| Code | Severity | Trigger |
|------|----------|---------|
| `ParseError` | Error | The SQL source file could not be parsed (syntax error). |
| `InvalidModel` | Error | The model's frontmatter or structure violates a structural rule. |
| `MalformedSectionDelimiter` | Error | A multi-model section header (`--- name: model_name ---`) is malformed, or SQL content appears before the first section delimiter in a multi-model file. |
| `UnclosedFrontmatter` | Error | A frontmatter block opened with `---` (or a `--- name: … ---` section header) is missing its closing `---`. |
| `UndefinedModelRef` | Error | A `smelt.<path>` reference in value position resolves to nothing. This is the **default** code for a bare unresolved `smelt.<path>` — when no entity exists at the path, the intended kind is unknowable, so the generic model-ref code fires. |
| `UndefinedSource` | Error | A `smelt.<path>` reference resolves to a *source* declaration that is itself missing or invalid (reserved for the source-resolution case; the bare-unresolved case is `UndefinedModelRef`). |
| `CannotInferType` | Error | The type of a column or expression cannot be inferred from context. |
| `UndeclaredColumn` | Error | A column name referenced in a query is not present in the inferred schema. |
| `ColumnTypeUnresolved` | Error | A column's type degrades to `Unknown` for a compiler-resolvable reason rather than a genuinely dynamic one (e.g. a `smelt.functions.*`-derived column the schema rules cannot type). Fires at the projection that produced it. Owned by `function_schema_inference.md` (schema-propagation rules) and `types.md` (the `Unknown` reason-discriminant). |
| `TypeMismatch` | Error | A column's inferred type does not match the expected type in a join or reference. |
| `CircularDependency` | Error | The model dependency graph contains a cycle. |
| `UnsupportedConstruct` | Error | A SQL construct is syntactically valid but not supported by smelt's analysis. |
| `YamlParseError` | Error | A YAML sidecar or frontmatter block could not be parsed. |
| `AmbiguousColumn` | Error | A column reference matches columns from more than one table in scope. |
| `UnknownCastType` | Error | A `CAST(x AS T)` uses a type name `T` that smelt does not recognise. |
| `UnrecognizedFunction` | Error | A SQL function call uses a function name that smelt does not recognise. |
| `DuplicateAddress` | Error | Two files in the same project resolve to the same `smelt.<path>` address. Anchored at the second (later-discovered) file, offset 0. |
| `DuplicateEmittedName` | Error | Two persisted entities in the same project resolve to the same `(target schema, joined table name)` for the active target, even though their `smelt.<path>` addresses differ (the `_`-join is not injective — e.g. `smelt.staging.orders` and `smelt.staging_orders` both emit `main.staging_orders`). Prevents a silent table clobber. Anchored at the second entity. See `architecture.md` §"Default materialization name mapping". |
| `KindMismatch` | Error | A `smelt.<path>` reference resolves to an entity whose kind is invalid in the surrounding position (e.g., a test ref in a `TableExpr` position). |

---

### Sources

Owned by `docs/specs/sources.md`.

| Code | Severity | Trigger |
|------|----------|---------|
| `SourceTypeError` | Error | A source YAML declares a type that smelt does not recognise. |
| `MalformedSource` | Error | A source YAML block violates a structural rule. |

---

### Seeds

Owned by `docs/specs/seeds.md` (or `docs/specs/models.md` seed section).

| Code | Severity | Trigger |
|------|----------|---------|
| `MissingSeedSidecar` | Warning | A seed CSV file has no sibling `.yml` sidecar; schema is inferred at compile time and may drift. |

---

### Timeseries

Owned by `docs/specs/timeseries.md`.

| Code | Severity | Trigger |
|------|----------|---------|
| `TimeseriesRequiredForIncremental` | Error | A model declares `incremental:` but has no `timeseries:` block. |
| `MalformedTimeseries` | Error | The `timeseries:` block parses but violates a structural rule. |

---

### Incremental

Owned by `docs/specs/incremental_models.md`.

| Code | Severity | Trigger |
|------|----------|---------|
| `IncrementalNotBatchSafe` | Warning | An `incremental` model's SQL is not batch-safe under the planner's incremental safety classifier; execution falls back to a safe chunking strategy. |

---

### Cumulative aggregate

Owned by `docs/specs/cumulative_aggregate.md`.

| Code | Severity | Trigger |
|------|----------|---------|
| `CumulativeRequiresGroupBy` | Error | A `cumulative_aggregate` SELECT has no GROUP BY (key columns are required). |
| `CumulativeUnknownAggregator` | Error | A `cumulative_aggregate` projection uses a non-allowlisted aggregator or composite expression over aggregates. |
| `CumulativeGroupByContainsPartitionColumn` | Error | The `cumulative_aggregate` GROUP BY contains the driving source's `partition_column`. |
| `CumulativeForbidsWindowFunctions` | Error | Window functions (`OVER (...)`) appear in a `cumulative_aggregate`. |
| `CumulativeForbidsNondeterministic` | Error | A non-deterministic function appears in a `cumulative_aggregate` SELECT. |
| `CumulativeNoDrivingSource` | Error | No source in a `cumulative_aggregate`'s FROM declares a `timeseries:` block. |
| `CumulativeMultipleDrivingSources` | Error | Multiple timeseries-tagged sources in a `cumulative_aggregate`'s FROM (v1 supports exactly one). |
| `CumulativeSqlNotParseable` | Error | A `cumulative_aggregate` SELECT could not be parsed for aggregator classification. |
| `CumulativeForbidsTimeseries` | Error | A `cumulative_aggregate` model incorrectly declares a `timeseries:` block. Anchored at offset 0. |
| `CumulativeForbidsIncremental` | Error | A `cumulative_aggregate` model incorrectly declares an `incremental:` block. Anchored at offset 0. |

---

### Types

Owned by `docs/specs/types.md` and the VALUES/alias-column-list analysis.

| Code | Severity | Trigger |
|------|----------|---------|
| `AliasColumnArityMismatch` | Error | An alias column list in `(VALUES …) AS t(c₁, …)` or `WITH cte(c₁, …) AS (…)` has a different length from the relation's actual column count. |
| `EmptyValuesClause` | Error | A `(VALUES …)` derived table contains no rows and cannot produce a typed schema. |
| `DecimalPrecisionOverflow` | Error | A decimal arithmetic expression (`+`, `-`, `*`, `%`) produces a result whose precision exceeds the 38-digit portable maximum. |

---

### Python models

Owned by `docs/specs/models.md` (Python model section).

| Code | Severity | Trigger |
|------|----------|---------|
| `PythonModelNameMismatch` | Error | A Python `@model` function returns SQL whose frontmatter `name:` field differs from the function name. |

---

### Functions & expansion

Owned by `docs/specs/functions.md`.

| Code | Severity | Trigger |
|------|----------|---------|
| `DuplicateFunctionDefinition` | Error | Two `smelt.define` declarations share a function name. Anchored at the second declaration's name span. |
| `InvalidFunctionTypeRef` | Error | A `smelt.define` parameter or return-type annotation cannot be parsed into a `SmeltType` (e.g. `Expr<Foo>`, unsupported nesting). |
| `FunctionBodyTypeMismatch` | Error | A `smelt.define` body contains a type mismatch (e.g. `x + 'text'` when `x: Expr<Integer>`). Anchored at the inner bad subexpression. |
| `UnknownIdentifier` | Error | A `smelt.define` body references a name that is neither a declared parameter nor resolvable in any enclosing scope. |
| `DuplicateParameterName` | Error | Two parameters in a single `smelt.define` share a name. Anchored at the second occurrence. |
| `UnknownSmeltFn` | Error | A `smelt.<path>(…)` call resolves to a function name not registered in the project. |
| `MissingArgument` | Error | A `smelt.<path>(…)` call omits a required parameter (one without a default value). |
| `ArgTypeMismatch` | Error | A `smelt.<path>(…)` call passes an argument whose type does not satisfy the declared parameter's `TypeConstraint`. |
| `ExternCollidesWithBuiltin` | Error | A `smelt.extern` declares a name that already exists in the built-in registry. |
| `BackendsWideningNotAllowed` | Error | A `smelt.define`'s `backends:` set is broader than what the body implies. (Malformed frontmatter is not this code's concern — it routes to `FrontmatterParseError`.) |
| `FrontmatterParseError` | Error/Warning | A `smelt.define` or `smelt.extern` frontmatter YAML block could not be parsed (Error) or contained an unknown key/malformed sub-entry (Warning). |
| `WindowInScalarContext` | Error | A window-function expression appears in a splice point that only accepts scalar/aggregate expressions (e.g. WHERE, GROUP BY). |
| `ParameterShadowsColumn` | Warning | An `Expr<T>`-kinded parameter name overlaps a column in a sibling `TableExpr`-kinded parameter's schema. |
| `RowRequirementUnsatisfied` | Error | A `TableExpr<{…}>` parameter has a row requirement the caller's schema cannot satisfy. |
| `UnknownContext` | Error | The context identifier in `Expr<T, ctx>` does not resolve to any parameter in the same `smelt.define` declaration. |
| `CteCycle` | Error | A CTE in a `smelt.define` body forms a cyclic reference. Anchored at the CTE name span. |
| `CteShadowsCallerCte` | Error | A model's top-level CTE shares a name with a CTE declared in a transparent function the model directly calls. |
| `ContextMismatch` | Error | An explicit `Expr<T, ctx_name>` annotation disagrees with the context inferred from the parameter's splice point. |
| `FragmentColumnMissing` | Error | A caller-provided fragment for a context-annotated `Expr<T>` parameter references a column not in the parameter's inferred splice context. |
| `AnnotationTooWide` | Error | An explicit `Expr<T, ctx_name>` annotation claims access to columns not present in the inferred splice context. |
| `FragmentKindMismatch` | Error | A caller-provided fragment for a `SelectItems<Kind>` parameter is of a lower expression kind than required. |
| `ReturnTypeMismatch` | Error | A Tier 3 function's body synthesises a return type that does not match the declared `-> Expr<T>` return annotation. |
| `UnknownPassingParameter` | Error | A `PASSING name AS (…)` clause names a parameter not declared in the callee's signature. |
| `UnstableSchemaRequired` | Error | A function's frontmatter uses the `provenance:` key but the project's `smelt.yml` does not have `unstable_schema: true`. |
| `AsStructUnsupportedBackend` | Error | `smelt.as_struct()` is used in a function body but the function's declared backend set includes a backend that does not support struct literal syntax. |
| `FunctionCallCycle` | Error | The transparent-function call graph contains a cycle (directly or transitively). Anchored at the offending function declaration's name span. |
| `ProvenanceMismatch` | Error | A function's declared `provenance:` entry lists a source column not read by the body, or the body reads a column not listed. |
| `JoinsMismatch` | Error | A function's declared `joins:` entry names a table that does not appear as a join alias in the body's outermost FROM clause. |
| `DeclaredCardinalityUnverifiable` | Warning | A declared join has a non-empty `cardinality` field; cardinality is trusted, not verified against data. |
| `MissingProvenancePushdownAdvisory` | Hint | A transparent function is called from a SELECT with a WHERE clause but the function lacks declared provenance (which would enable filter pushdown). |

> **Ownership of the four planner-validation codes.** `ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable`, and `MissingProvenancePushdownAdvisory` are **owned by `planner_integration.md`** — the planner consumes `provenance:`/`joins:` declarations and emits these during plan validation. `functions.md` owns only the *grammar* of those frontmatter keys. They are listed here (rather than under a separate group header) because they sit beside the function-frontmatter codes that share their inputs.
| `ExternFragmentParamUnsupported` | Error | A `smelt.extern` declaration has a parameter whose type is a fragment sort (`SelectItems`, `OrderSpec`); fragment-sort params require PASSING clauses which `smelt.extern` does not support. |
| `DefaultReferencesParameter` | Error | A `smelt.define` default expression references another parameter in the same signature. |
| `UnknownStructFieldType` | Error | A `smelt.define` or `smelt.extern` parameter or return-type annotation has a `Struct<{…}>` shape whose field type text cannot be parsed as a recognised `DataType`. Anchored at the individual field's `TYPE_REF` span. |

#### Detailed example: `UnknownStructFieldType`

**Anchor**: the individual field's `TYPE_REF` span (inside the struct
annotation, not the whole parameter span)

Emitted when a `smelt.define` or `smelt.extern` parameter or return-type
annotation has a `Struct<{…}>` shape whose field type text cannot be parsed
as a recognised concrete `DataType`.

Example:
```sql
-- Error on the `Bogus` span:
smelt.define my_fn(t: Expr<Struct<{a: Integer, b: Bogus}>>) -> Expr<Integer> AS (
  t.a
)
```

The struct value is still constructed (with `DataType::Unknown` as the field
type) for downstream use, but this diagnostic ensures the author is told
exactly which field name is unrecognised rather than receiving a later,
context-free `Unknown`-propagation error.

---

### Meta-language

Owned by `docs/specs/meta_language.md`.

#### Lists and spread

| Code | Severity | Trigger |
|------|----------|---------|
| `MetaListEmptyTypeUnknown` | Error | An empty list literal `[]` appears where no target sort context is available to infer the element type. |
| `MetaListHeterogeneous` | Error | A list literal's elements have incompatible types that cannot be unified under LUB. |
| `MetaSpreadInForbiddenPosition` | Error | A spread operator `...xs` appears in a position that does not permit spread (WHERE, FROM without a reducer, boolean composition, or named-arg value). |
| `MetaSpreadOnNonList` | Error | The operand of a spread `...x` is not a `List<T>`. |
| `MetaListInScalarPosition` | Error | A `List<T>`-typed expression reaches a Data-World scalar/SELECT-item position without being consumed by a spread, HOF, reducer, record, map, or generator. |

#### Lambdas and higher-order functions

| Code | Severity | Trigger |
|------|----------|---------|
| `LambdaInForbiddenPosition` | Error | A `fn x => body` lambda appears outside a HOF positional argument position. |
| `LambdaArityMismatch` | Error | A lambda passed to a HOF has a different arity from what the HOF expects. |
| `LambdaZeroParameters` | Error | A lambda has zero parameters (`fn () => body`). |
| `LambdaDuplicateParameter` | Error | A lambda parameter list contains the same name twice. |
| `LambdaResultTypeMismatch` | Error | The lambda body's synthesised type is incompatible with the HOF's required result shape (e.g. `filter` requires `Boolean`). |
| `HofExpectsLambda` | Error | The second argument to `map` or `filter` is not a lambda. |
| `HofExpectsReducer` | Error | The second argument to `reduce` is not a registered reducer. |
| `HofNameShadowed` | Error | A `smelt.define` declaration uses a HOF name (`map`, `filter`, `reduce`). |

#### Reducers

| Code | Severity | Trigger |
|------|----------|---------|
| `ReducerNameShadowed` | Error | A `smelt.define` declaration uses a name from the closed reducer registry. |
| `ReducerInputTypeMismatch` | Error | A reducer is applied to a list whose element type is incompatible with the reducer's declared input constraint. |
| `ReducerEmptyNoIdentity` | Error | `union_all` or `intersect_all` reduces an empty list (no identity element). |
| `ReducerArityMismatch` | Error | A parameterised reducer call has the wrong number of positional arguments. |
| `ReducerArgTypeMismatch` | Error | A parameterised reducer argument has the wrong type. |
| `ReducerArgNotCompileTime` | Error | A parameterised reducer argument is a runtime expression rather than a compile-time value. |
| `ReducerNamedArgument` | Error | A parameterised reducer call uses named arguments (reducers take positional arguments only). |

#### Pipes

| Code | Severity | Trigger |
|------|----------|---------|
| `PipeRhsNotCall` | Error | The RHS of a `|>` is not syntactically a call expression. |
| `PipeInDataPosition` | Error | A pipe expression `|>` appears in a Data-World grammar position (e.g. inside a WHERE predicate). |

#### Ternary (if-then-else)

| Code | Severity | Trigger |
|------|----------|---------|
| `TernaryConditionNotBoolean` | Error | The ternary condition expression is not Boolean. |
| `TernaryBranchTypeMismatch` | Error | The then-branch and else-branch of a ternary have incompatible types that cannot be unified. |
| `TernaryKeywordShadowed` | Error | A parameter is declared with a name that is a reserved ternary keyword (`if`, `then`, `else`). |
| `TernaryInDataPosition` | Error | A ternary expression (`if-then-else`) appears in a Data-World (SQL) splice position. |
| `TernaryDanglingThen` | Error | A `then` keyword appears outside of an `if … then …` form. |
| `TernaryDanglingElse` | Error | An `else` keyword appears outside of a `… then … else` form. |

#### Compile-time configuration variables

| Code | Severity | Trigger |
|------|----------|---------|
| `ConfigVarNotFound` | Error | `smelt.config.var(<name>)` is called and `<name>` is not present in `smelt.yml` `vars:`. |
| `ConfigVarNameNotLiteral` | Error | `smelt.config.var` is called with a non-literal-Text argument. |
| `ConfigVarNullCoercion` | Warning | A YAML `null` value is coerced to empty string at a `smelt.config.var` site. |

#### Reflection — `smelt.columns_of`

| Code | Severity | Trigger |
|------|----------|---------|
| `ColumnsOfRequiresTableExpr` | Error | `smelt.columns_of(x)` is called and `x` synthesises to a type not assignable to `TableExpr`. |
| `ColumnsOfNamedArgument` | Error | `smelt.columns_of` is called with a named argument. |
| `ColumnRefFieldUnknown` | Error | Field access on a `ColumnRef`-typed value uses a field identifier outside the closed field set `{name, type, is_numeric}`. |
| `ColumnsOfUnresolvableSchema` | Error | `smelt.columns_of(t)` is called and `t`'s schema cannot be statically resolved (upstream returns `Unknown`). |

#### Reflection — `smelt.models.*` and `smelt.sources.*`

| Code | Severity | Trigger |
|------|----------|---------|
| `WithTagRequiresText` | Error | `smelt.models.with_tag(x)` or `smelt.sources.with_tag(x)` is called and `x` synthesises to a type not assignable to compile-time `Text`. |
| `WithTagNamedArgument` | Error | `with_tag` is called with a named argument. |
| `WideReflectionUnknownAccessor` | Error | `smelt.models.<name>` or `smelt.sources.<name>` refers to an accessor name outside the closed set `{with_tag, all}`. |
| `WideReflectionUnexpectedArgument` | Error | `smelt.models.all` or `smelt.sources.all` is called with any argument. |
| `ModelRefFieldUnknown` | Error | Field access on a `ModelRef`-typed value uses a field identifier outside the closed field set `{path, name, tags, columns}`. |
| `SourceRefFieldUnknown` | Error | Field access on a `SourceRef`-typed value uses a field identifier outside the closed field set `{path, name, tags, columns}`. |

---

### Records, maps & config loaders

Owned by `docs/specs/meta_language.md` (records/maps/loaders sections).

#### Records

| Code | Severity | Trigger |
|------|----------|---------|
| `SmeltRecordRedefinition` | Error | A second `smelt.record` declaration in the same project shares an existing record's name. Anchored at the second declaration's name token. |
| `RecordFieldUnknown` | Error | Field projection or literal field name is outside the target's declared field set. |
| `RecordFieldMissing` | Error | A record literal omits a field required by the target type. |
| `RecordFieldDuplicate` | Error | A record literal names the same field twice. |
| `RecordFieldTypeMismatch` | Error | A literal field value's type is not assignable to the declared field type. |
| `RecordLiteralUnknownTarget` | Error | A record literal appears in a position with no inferable target type. |
| `RecordFieldNotProjectable` | Error | Mid-chain field projection through a non-record-typed value. |
| `RecordFieldTypeForbidden` | Error | A `smelt.record` field type references a meta-only witness (`ColumnRef`, `ModelRef`, `SourceRef`) or `Lambda`. |
| `RecordCyclicDeclaration` | Error | A `smelt.record` declaration references its own name directly or transitively (v1 records must form a DAG). |
| `RecordInDataWorld` | Error | A record-typed value is referenced in a Data-World (SQL) position outside a splice context. |

#### Maps

| Code | Severity | Trigger |
|------|----------|---------|
| `MapKeyTypeNotText` | Error | A `Map<K, V>` type expression has `K` other than `Text`. |
| `MapApiUnknown` | Error | Method call on a `Map<K, V>` value uses a name outside the closed Map API (`entries`, `keys`, `values`, `get`, `has`). |
| `MapApiArityMismatch` | Error | `m.get` or `m.has` is called with other than one positional argument. |
| `MapApiNamedArgument` | Error | A Map API method is called with a named argument. |
| `MapApiUnexpectedArgument` | Error | `m.entries`, `m.keys`, or `m.values` is called with any argument. |
| `MapGetMissingKey` | Error | `m.get(k)` with a statically-known `k` that is absent from `m`. |
| `MapApiArgTypeMismatch` | Error | `m.get(k)` or `m.has(k)` with `k`'s type not assignable to `K`. |

#### Config loaders

| Code | Severity | Trigger |
|------|----------|---------|
| `ConfigLoaderPathNotLiteral` | Error | A loader `path` argument is not a string literal. |
| `ConfigLoaderPathEscapesWorkspace` | Error | A loader path is absolute, contains `..` escapes, or has a scheme prefix. |
| `ConfigLoaderPathBackslash` | Error | A loader path contains `\` (use `/` as the path separator). |
| `ConfigLoaderFileNotFound` | Error | The resolved loader file does not exist in the project. |
| `ConfigLoaderSchemaForbidden` | Error | A loader schema argument is not a record type, `List<record>`, or `Map<Text, record>`. |
| `ConfigLoaderTomlNotYetSupported` | Error | `smelt.config.load_toml` is called (only YAML and JSON loaders are supported in v1). |
| `ConfigLoaderParseError` | Error | The loaded file is not valid YAML or JSON. |
| `ConfigLoaderRequiredFieldMissing` | Error | A loaded value omits a field required by the schema. |
| `ConfigLoaderUnknownField` | Error | A loaded value contains a field not declared in the schema. |
| `ConfigLoaderTypeMismatch` | Error | A loaded value's type does not match the schema's declared type. |
| `ConfigLoaderRootShapeMismatch` | Error | The file's top-level shape does not match the schema's expected root shape. |
| `ConfigLoaderDuplicateMapKey` | Error | A `Map<Text, S>`-shaped file contains the same key twice. |
| `ConfigLoaderNullCoercion` | Warning | A YAML `null` scalar coerces to an empty `Text` value at a schema field declared `Text`. |

---

### Multi-model production (generator files)

Owned by `docs/specs/meta_language.md` (multi-model production section).

| Code | Severity | Trigger |
|------|----------|---------|
| `GeneratesUnknownValue` | Error | A `generates:` frontmatter value other than `models` was supplied. |
| `GeneratesMixedWithBareModel` | Error | `generates: models` frontmatter is combined with a `name:` field or Layer-1 section delimiters. |
| `GenerateFileBareSelectForbidden` | Error | A generator file body contains a top-level bare SELECT/WITH/VALUES. |
| `GenerateFileBodyTypeError` | Error | A generator file body synthesises a type not assignable to `List<ModelDef>`. |
| `ModelDefOutsideGeneratorFile` | Error | A `ModelDef {…}` record literal appears in a non-generator-file context. |
| `ModelDefInvalidName` | Error | `ModelDef.name` value is empty or contains non-path-safe characters. |
| `ModelDefInvalidMaterialization` | Error | `ModelDef.materialization` value is not in `{'view', 'table', 'incremental'}`. |
| `ModelDefDuplicateName` | Error | Two `ModelDef`s in the same generator emit with the same `name`. |
| `ModelDefHandAuthoredCollision` | Error | A generator-emitted path collides with a hand-authored model or another generator's emission. |
| `GeneratorBodyForbidsModelReflection` | Error | A generator body invokes `smelt.models.with_tag` or `smelt.models.all`. |

---

## Known divergences

None currently open.

## Open questions

None currently open.

## References

- **Code**: `crates/smelt-db/src/diagnostics_types.rs` — full `DiagnosticCode` enum
- **Code**: `crates/smelt-types/src/signatures.rs` — `struct_field_unknown_ranges` pure helper
- **Code**: `crates/smelt-db/src/queries/function_diagnostics.rs` — `struct_field_type_unknown_diagnostics_for_file`
- **Tests**: `crates/smelt-db/tests/diagnostics_catalogue.rs` — coverage gate: every `DiagnosticCode` variant must appear in this catalogue
- **Tests**: `crates/smelt-db/tests/struct_field_type.rs`
- **Tests**: `crates/smelt-types/tests/unknown_census.rs` — guards every `DataType::Unknown` site
- **Plans (history)**: `docs/plans/20260608-silent-failures-hardening.md`
- **Plans (history)**: `docs/plans/20260611-docs-gap-remediation.md`
- **Related specs**: `docs/specs/architecture.md` §"Fail-loud invariants"
