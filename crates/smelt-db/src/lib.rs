//! Salsa database for incremental compilation (salsa 0.26 API).
//!
//! This module defines the Salsa inputs and tracked queries that power the
//! LSP and optimizer. Salsa automatically handles incremental recomputation
//! when inputs change.
//!
//! # Architecture: Pure Function Rule
//!
//! All analysis logic must be implemented as **pure functions** that take AST
//! nodes and plain data structures (e.g., `TypeContext`, `ModelSchema`).
//! Salsa tracked functions should be thin wrappers: gather inputs via queries,
//! call the pure function, return the result.
//!
//! **DO**: `fn infer_type(expr: &Expr, ctx: &TypeContext) -> DataType`
//! **DON'T**: Put `db.some_query()` calls inside analysis logic.
//!
//! This keeps the core compiler logic reusable outside Salsa (batch CLI, planner)
//! and independently testable. See `type_inference.rs` and `schema.rs` for examples.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rowan::TextRange;
use salsa::{Accumulator, Setter};
use serde::Deserialize;
use smelt_parser::{self, ast::SmeltPathRef, File as AstFile, TableRef};
use smelt_types::signatures::{extract_function_signatures_with_raw, FunctionSig};
pub use smelt_types::TypedColumn;
use smelt_types::{parse_type, DataType};

pub mod backends;
pub mod code_actions;
pub mod config_vars;
pub mod function_body_check;
pub mod provenance_validator;
pub mod references;
pub mod schema;
pub mod type_inference;
pub mod yaml_edits;

pub use function_body_check::{
    check_fragment_context_bindings, check_struct_row_var_binding, check_tier3_return_type,
    declared_return_hover_text, expand_brace_struct_body, extract_function_body_cte_schemas,
    infer_splice_contexts, infer_tableexpr_return_schema, is_tier2_function,
};
pub use schema::{
    Column, ColumnConstraint, ColumnSource, FunctionInput, FunctionOutput, InputConstraint,
    ModelFunctionType, ModelSchema, RefKind, ResolvedSchema, RowExtension, TypedField,
};
pub use type_inference::{
    check_window_in_scalar_contexts, infer_cte_columns, infer_expression_kind,
    infer_expression_type, infer_select_column_types, walk_expression_columns_with_visitor,
    walk_select_columns_with_visitor, TypeContext, WindowInScalarContextInfo,
};

// Source types re-exported from smelt-core
pub use smelt_core::{
    SeedInfo, SourceColumn, SourceColumnDef, SourceDef, SourceInfo, SourceTableDef, SourcesConfig,
};

// ============================================================================
// Salsa inputs
// ============================================================================

/// A source file tracked by Salsa. Holds the file's current text and the
/// project root it belongs to. Looked up by path via the Database's
/// internal registry (`Database::source_file`).
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub text: String,
    #[returns(ref)]
    pub project_root: PathBuf,
}

/// A project-level input storing the raw `sources.yml` text for the project.
/// Looked up by project root path via `Database::project_input`.
#[salsa::input]
pub struct ProjectInput {
    #[returns(ref)]
    pub root: PathBuf,
    #[returns(ref)]
    pub sources_yaml: String,
    /// Raw text of the workspace's `smelt.yml` file. Empty string when not
    /// loaded (treated as no unstable flags set). Updated by the LSP whenever
    /// the file changes on disk, keeping Salsa's change detection valid.
    #[returns(ref)]
    pub smelt_yml_text: String,
}

/// Workspace-level singleton input tracking the full set of files and projects.
/// Queries that need to enumerate everything (e.g. `all_models`, `resolve_ref`)
/// read from this singleton so they're invalidated when the file set changes.
#[salsa::input(singleton)]
pub struct Workspace {
    #[returns(ref)]
    pub files: Vec<SourceFile>,
    #[returns(ref)]
    pub projects: Vec<ProjectInput>,
}

// ============================================================================
// Diagnostics accumulator
// ============================================================================

#[salsa::accumulator]
pub struct DiagnosticAcc(pub Diagnostic);

// ============================================================================
// Database
// ============================================================================

/// The Salsa database for smelt. The `files` and `projects` maps provide a
/// path-keyed registry for looking up input structs — Salsa itself doesn't
/// do this automatically.
#[salsa::db]
#[derive(Clone, Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
    files: Arc<RwLock<HashMap<PathBuf, SourceFile>>>,
    projects: Arc<RwLock<HashMap<PathBuf, ProjectInput>>>,
}

#[salsa::db]
impl salsa::Database for Database {}

impl Database {
    /// Create or retrieve the `SourceFile` input for `path`, seeding its fields.
    /// If a `SourceFile` already exists, its text/project_root are updated to
    /// the provided values.
    pub fn set_source_file(
        &mut self,
        path: PathBuf,
        text: String,
        project_root: PathBuf,
    ) -> SourceFile {
        let existing = self.files.read().unwrap().get(&path).copied();
        match existing {
            Some(file) => {
                file.set_text(self).to(text);
                file.set_project_root(self).to(project_root);
                file
            }
            None => {
                let file = SourceFile::new(self, path.clone(), text, project_root);
                self.files.write().unwrap().insert(path, file);
                file
            }
        }
    }

    /// Create or retrieve the `ProjectInput` for `root`, seeding its yaml.
    ///
    /// Also reads `smelt.yml` from `root` (if present) and stores it in
    /// `smelt_yml_text` so that `project_unstable_schema` is Salsa-tracked
    /// without further call-site changes. The LSP can call
    /// `set_project_smelt_yml` later to propagate in-editor edits.
    pub fn set_project_input(&mut self, root: PathBuf, sources_yaml: String) -> ProjectInput {
        let smelt_yml_text = std::fs::read_to_string(root.join("smelt.yml")).unwrap_or_default();
        let existing = self.projects.read().unwrap().get(&root).copied();
        match existing {
            Some(project) => {
                project.set_sources_yaml(self).to(sources_yaml);
                project.set_smelt_yml_text(self).to(smelt_yml_text);
                project
            }
            None => {
                let project = ProjectInput::new(self, root.clone(), sources_yaml, smelt_yml_text);
                self.projects.write().unwrap().insert(root, project);
                project
            }
        }
    }

    /// Update the `smelt.yml` text for an already-registered project. Called by
    /// the LSP whenever the file changes on disk; Salsa propagates the
    /// invalidation through `project_unstable_schema` and any query that reads it.
    pub fn set_project_smelt_yml(&mut self, root: &Path, smelt_yml_text: String) {
        let project = self.projects.read().unwrap().get(root).copied();
        if let Some(project) = project {
            project.set_smelt_yml_text(self).to(smelt_yml_text);
        }
    }

    /// Look up an already-registered `SourceFile` by path.
    pub fn source_file(&self, path: &Path) -> Option<SourceFile> {
        self.files.read().unwrap().get(path).copied()
    }

    /// Look up an already-registered `ProjectInput` by root path.
    pub fn project_input(&self, root: &Path) -> Option<ProjectInput> {
        self.projects.read().unwrap().get(root).copied()
    }

    /// Set (or create) the workspace singleton with the given file and project lists.
    pub fn set_workspace(&mut self, files: Vec<SourceFile>, projects: Vec<ProjectInput>) {
        match Workspace::try_get(self) {
            Some(ws) => {
                ws.set_files(self).to(files);
                ws.set_projects(self).to(projects);
            }
            None => {
                Workspace::new(self, files, projects);
            }
        }
    }

    /// Convenience accessor: the workspace singleton, creating it empty if missing.
    pub fn workspace(&mut self) -> Workspace {
        match Workspace::try_get(self) {
            Some(ws) => ws,
            None => Workspace::new(self, Vec::new(), Vec::new()),
        }
    }
}

// ============================================================================
// Public data types
// ============================================================================

/// Represents a model (SQL file in models/ directory)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub path: PathBuf,
    /// Physical file path on disk (same as `path` for single-model files,
    /// differs for multi-model files where `path` is a virtual key).
    pub source_path: PathBuf,
}

/// Reference location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLocation {
    pub name: String,
    pub range: Range,
}

/// Source location with position information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub source_name: String,
    pub table_name: String,
    pub qualified_name: String,
    pub range: Range,
}

/// Position in a file (line, column)
pub type Position = smelt_parser::ast::Position;

/// Range in a file (start, end)
pub type Range = smelt_parser::ast::Range;

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
        _ => panic!(
            "meta_reflection_diagnostic_message called with non-Phase-C code: {:?}",
            code
        ),
    }
}

/// YAML parse error with location information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlParseError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// Invalid type in sources.yml column definition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTypeError {
    pub source_name: String,
    pub table_name: String,
    pub column_name: String,
    pub invalid_type: String,
}

// ============================================================================
// Syntax queries
// ============================================================================

#[salsa::tracked(returns(ref))]
pub fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> smelt_parser::Parse {
    let text = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text);
    smelt_parser::parse(&clean_text)
}

#[salsa::tracked]
pub fn parse_model(db: &dyn salsa::Database, file: SourceFile) -> Option<Arc<Model>> {
    let path = file.path(db).clone();
    // Extract model name: from virtual path suffix (multi-model) or file stem (single-model)
    let path_str = path.to_str().unwrap_or("");
    let (model_name, source_path) = if let Some((file_part, name)) = path_str.rsplit_once("::") {
        (name.to_string(), PathBuf::from(file_part))
    } else {
        (path.file_stem()?.to_str()?.to_string(), path.clone())
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    ast.select_stmt()?;

    Some(Arc::new(Model {
        name: model_name,
        path,
        source_path,
    }))
}

/// Legacy `smelt.models.name` locations.
///
/// Phase 4: `smelt.ref()` is now a parse error; this query always returns an
/// empty vec. Kept so callers compile without changes during the migration.
/// Will be removed in a follow-up cleanup.
#[salsa::tracked]
pub fn model_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<RefLocation>> {
    // smelt.ref() is a parse error — no legacy RefCall nodes can appear.
    let _ = (db, file);
    Arc::new(Vec::new())
}

/// Path-form refs (`smelt.<path>` in value position) extracted with
/// their resolution metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRefLocation {
    pub path: Vec<String>,
    pub range: Range,
    /// True when this ref appears in a `TableExpr` (FROM/JOIN) position.
    /// Phase 2a uses this to gate kind-mismatch diagnostics: a path
    /// resolving to a `Test` is invalid in `TableExpr` positions.
    pub in_table_expr_position: bool,
}

/// Extract every unified `smelt.<path>` value-form ref from a file.
///
/// Phase 2a — Salsa-tracked sister of [`model_refs`] for the new path
/// surface. The legacy `model_refs` query still surfaces
/// `smelt.models.name` callsites; this query surfaces the unified
/// path-form value refs (no `(`) so the diagnostic pass can validate
/// them through [`resolve_ref_path`]. Call-form path refs are not
/// included here — they're consumed by the function-call pipeline.
#[salsa::tracked]
pub fn model_path_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<PathRefLocation>> {
    let parse = parse_file(db, file);
    let text = file.text(db);
    let syntax = parse.syntax();

    let Some(ast) = AstFile::cast(syntax) else {
        return Arc::new(Vec::new());
    };

    let mut out = Vec::new();
    for path_ref in ast.syntax().descendants().filter_map(SmeltPathRef::cast) {
        // Skip nested path refs that are part of a SmeltPathCall.
        if path_ref
            .syntax()
            .ancestors()
            .skip(1)
            .any(|a| smelt_parser::ast::SmeltPathCall::cast(a).is_some())
        {
            continue;
        }
        let in_table_expr_position = is_in_table_expr_position(path_ref.syntax());
        let path = path_ref.segments();
        let range = smelt_parser::ast::text_range_to_range(text, path_ref.text_range());
        out.push(PathRefLocation {
            path,
            range,
            in_table_expr_position,
        });
    }
    Arc::new(out)
}

/// True when this `SMELT_PATH_REF` node sits in a `TableExpr` position
/// — i.e. it's the body of a TableRef under a FROM/JOIN clause.
fn is_in_table_expr_position(node: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    use smelt_parser::syntax_kind::SyntaxKind as Sk;
    node.ancestors()
        .any(|a| matches!(a.kind(), Sk::TABLE_REF | Sk::FROM_CLAUSE | Sk::JOIN_CLAUSE))
}

/// Legacy `smelt.sources.schema.table` locations.
///
/// Phase 4: `smelt.source()` is now a parse error; this query always returns
/// an empty vec. Kept so callers compile without changes during the migration.
/// Will be removed in a follow-up cleanup.
#[salsa::tracked]
pub fn model_sources(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<SourceLocation>> {
    // smelt.source() is a parse error — no legacy SourceCall nodes can appear.
    let _ = (db, file);
    Arc::new(Vec::new())
}

#[salsa::tracked]
pub fn sources_config(db: &dyn salsa::Database, project: ProjectInput) -> Arc<SourcesConfig> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(SourcesConfig::default());
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
        Ok(config) => Arc::new(config),
        Err(_) => Arc::new(SourcesConfig::default()),
    }
}

/// Return `true` when the project's `smelt.yml` contains `unstable_schema: true`.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// The LSP updates the text whenever `smelt.yml` changes on disk, so this
/// query is automatically invalidated and re-evaluated on each change.
#[salsa::tracked]
pub fn project_unstable_schema(db: &dyn salsa::Database, project: ProjectInput) -> bool {
    smelt_core::parse_unstable_schema_flag(project.smelt_yml_text(db))
}

/// Return the set of *active* backend names for `project` — i.e. the
/// distinct `target_type` values in `smelt.yml`'s `targets:` map.
///
/// Phase 42: this is the set the
/// [`as_struct_backend_diagnostics_for_file`] gate intersects against
/// when a function declares `BackendSet::All` (no explicit
/// `backends:` frontmatter). Returning `None` means we could not parse
/// the workspace config — callers should treat that as "no constraint
/// on active backends" and fall back to the Phase 38 behaviour
/// (only check explicitly-declared `BackendSet::Only`).
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
#[salsa::tracked]
pub fn project_active_backends(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<Vec<String>> {
    smelt_core::parse_active_backends(project.smelt_yml_text(db))
}

/// Return the `vars:` block from `smelt.yml` as a `BTreeMap<String, serde_yaml::Value>`.
///
/// Returns an empty map when the `vars:` key is absent or the YAML cannot be
/// parsed. The `None` case is mapped to an empty map to avoid bubbling parse
/// errors into callers — callers that need to distinguish "vars absent" from
/// "vars present but empty" should use `config_vars::parse_vars_from_yaml` directly.
///
/// Reads from `ProjectInput::smelt_yml_text`, which is tracked by Salsa.
/// Automatically invalidated and re-evaluated whenever `smelt.yml` changes.
#[salsa::tracked]
pub fn smelt_yml_vars_query(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<std::collections::BTreeMap<String, serde_yaml::Value>> {
    let text = project.smelt_yml_text(db);
    Arc::new(config_vars::parse_vars_from_yaml(text).unwrap_or_default())
}

/// Return the project's `paths:` scan-root list from `smelt.yml`, or an
/// empty list when the config text is missing or fails to parse.
///
/// Reads from `ProjectInput::smelt_yml_text` so the result is Salsa-tracked
/// and reused across every per-file resolver call. Without this, callers
/// like `resolve_ref_path` re-read and re-parse `smelt.yml` once per
/// workspace file, turning per-file diagnostics into an O(N^2) operation
/// (this was the root cause of the multi-hour CI bench regression).
///
/// The empty-on-error fallback matches the previous `Config::load(...)
/// .map(|c| c.paths).unwrap_or_default()` behaviour in the resolver: when
/// the workspace has no parseable config, no scan-root prefix is stripped
/// and the full directory path becomes part of the resolved tuple.
#[salsa::tracked]
pub fn project_paths(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<String>> {
    let text = project.smelt_yml_text(db);
    let paths = smelt_core::Config::parse_with_warnings(text)
        .map(|(c, _warnings)| c.paths)
        .unwrap_or_default();
    Arc::new(paths)
}

/// Discover seed CSV files for a project root and infer their column types.
///
/// Reads from disk (not a tracked Salsa input) — seeds that change on disk
/// require a tool restart to be detected. The query is keyed on `ProjectInput`
/// so it's recomputed when the project's sources_yaml changes, but not when
/// CSV files change.
#[salsa::tracked]
pub fn project_seeds(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SeedInfo>> {
    let project_root = project.root(db).clone();
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    // Phase 5: use with_sidecars so ephemeral materialization is tracked.
    Arc::new(smelt_core::discover_seed_infos_with_sidecars(
        &project_root,
        &paths,
    ))
}

/// Discover per-entity source YAML files for a project root.
///
/// Phase 6: sources live as standalone `.yml` files (no sibling `.csv`) under
/// the project's `paths:` directories. The query is keyed on `ProjectInput`
/// so it re-runs when `smelt.yml` changes (e.g. `paths:` updated) but not on
/// every source file change (LSP restarts are acceptable for source changes).
#[salsa::tracked]
pub fn project_sources(db: &dyn salsa::Database, project: ProjectInput) -> Arc<Vec<SourceInfo>> {
    let project_root = project.root(db).clone();
    let paths = smelt_core::Config::load(&project_root)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    Arc::new(smelt_core::discover_source_infos(&project_root, &paths))
}

#[salsa::tracked]
pub fn sources_yaml_error(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Option<YamlParseError> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return None;
    }
    match serde_yaml::from_str::<SourcesConfig>(yaml) {
        Ok(_) => None,
        Err(e) => {
            let (line, column) = e
                .location()
                .map(|loc| (Some(loc.line()), Some(loc.column())))
                .unwrap_or((None, None));
            Some(YamlParseError {
                message: e.to_string(),
                line,
                column,
            })
        }
    }
}

#[salsa::tracked]
pub fn sources_type_errors(
    db: &dyn salsa::Database,
    project: ProjectInput,
) -> Arc<Vec<SourceTypeError>> {
    let yaml = project.sources_yaml(db);
    if yaml.is_empty() {
        return Arc::new(Vec::new());
    }

    #[derive(Deserialize)]
    struct RawSourcesConfig {
        #[serde(default)]
        sources: Vec<RawSource>,
    }

    #[derive(Deserialize)]
    struct RawSource {
        name: String,
        #[serde(default)]
        tables: Vec<RawTable>,
    }

    #[derive(Deserialize)]
    struct RawTable {
        name: String,
        #[serde(default)]
        columns: Vec<RawColumn>,
    }

    #[derive(Deserialize)]
    struct RawColumn {
        name: String,
        #[serde(default, rename = "type")]
        type_str: Option<String>,
    }

    let config: RawSourcesConfig = match serde_yaml::from_str(yaml) {
        Ok(c) => c,
        Err(_) => return Arc::new(Vec::new()),
    };

    let mut errors = Vec::new();
    for source in &config.sources {
        for table in &source.tables {
            for column in &table.columns {
                if let Some(type_str) = &column.type_str {
                    if parse_type(type_str).is_err() {
                        errors.push(SourceTypeError {
                            source_name: source.name.clone(),
                            table_name: table.name.clone(),
                            column_name: column.name.clone(),
                            invalid_type: type_str.clone(),
                        });
                    }
                }
            }
        }
    }
    Arc::new(errors)
}

#[salsa::tracked]
pub fn all_models(db: &dyn salsa::Database, workspace: Workspace) -> Arc<HashMap<PathBuf, Model>> {
    let mut models = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        if let Some(model) = parse_model(db, file) {
            models.insert(file.path(db).clone(), (*model).clone());
        }
    }
    Arc::new(models)
}

// ============================================================================
// Function signature index (Phase 3, smelt-functions Step 1)
// ============================================================================
//
// Per §20H of `docs/research/20260413-smelt-functions.md`, signature lookups
// (used by downstream type-checking) must not be invalidated by edits to a
// function *body*. Split:
//   - `file_signature_inputs` / `functions_in_file` — signatures only. Its
//     return value is content-equal across body-only edits, so Salsa's
//     by-value backdating stops the re-run cascade at the boundary.
//   - `function_body` — CST of the body expression, re-computed on any edit
//     but independent of the signature query.
//
// All of these are thin wrappers over the pure
// `smelt_types::signatures::extract_function_signatures` function — per the
// pure-function rule in CLAUDE.md.

/// Extract function signatures from a single file. Pure-function wrapper
/// around `smelt_types::signatures::extract_function_signatures`.
///
/// This query's output only changes when *signature* tokens change. Body
/// edits do not affect the returned `Vec<FunctionSig>`, so Salsa's durability
/// check prevents downstream consumers from re-running. This is the §20H
/// invalidation hinge.
#[salsa::tracked]
pub fn file_signature_inputs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw);
    if let Some(ast) = AstFile::cast(syntax) {
        Arc::new(extract_function_signatures_with_raw(
            &ast,
            &clean_text,
            text_raw,
        ))
    } else {
        Arc::new(Vec::new())
    }
}

/// All function signatures declared in `file`, in declaration order.
///
/// Exposed as a distinct public name from `file_signature_inputs` per the
/// plan; internally it is the same query.
#[salsa::tracked]
pub fn functions_in_file(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    file_signature_inputs(db, file)
}

/// Look up a single function's signature by name within one file.
///
/// Memoized by `(file, name)`. Re-uses `file_signature_inputs` so edits to
/// other declarations in the same file don't necessarily invalidate this
/// lookup either (though Salsa's current implementation cannot detect that
/// granularity — it still goes through `file_signature_inputs`'s output).
#[salsa::tracked]
pub fn function_signature(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let sigs = file_signature_inputs(db, file);
    sigs.iter()
        .find(|s| s.name == name)
        .map(|s| Arc::new(s.clone()))
}

/// Byte range of a function body in the stripped source text.
///
/// Rowan's `SyntaxNode` is `!Send`, so we cannot store it in a Salsa tracked
/// output directly. Instead, this query returns the byte range of the body
/// within the parsed (frontmatter-stripped) source. Callers can re-parse or
/// re-read the CST via `parse_file` and locate the body using this range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyRange {
    /// Inclusive start byte offset into the stripped source.
    pub start: u32,
    /// Exclusive end byte offset into the stripped source.
    pub end: u32,
}

/// Byte range of `name`'s body in `file`'s stripped source text, if any.
///
/// Depends directly on `parse_file` — not on `file_signature_inputs` — so
/// that body-only edits invalidate this query without invalidating the
/// signature query. (A body edit changes the `Parse` output, which changes
/// the body's text range if body length changed, and re-parsing anyway
/// — in practice this query re-computes on any file edit. The invariant
/// that matters is the asymmetric direction: `function_signature`
/// is *not* invalidated by body edits.)
#[salsa::tracked]
pub fn function_body(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<BodyRange> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    for define in ast.defines() {
        if define.name().as_deref() == Some(name.as_str()) {
            let body = define.body()?;
            let range = body.syntax().text_range();
            return Some(BodyRange {
                start: u32::from(range.start()),
                end: u32::from(range.end()),
            });
        }
    }
    None
}

/// Resolve a function name to the first matching `FunctionSig` in the
/// workspace. Files are enumerated in sorted-by-path order for deterministic
/// diagnostics (the first file declares; later files collide).
#[salsa::tracked]
pub fn resolve_function(
    db: &dyn salsa::Database,
    workspace: Workspace,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in files {
        let sigs = file_signature_inputs(db, f);
        if let Some(sig) = sigs.iter().find(|s| s.name == name) {
            return Some(Arc::new(sig.clone()));
        }
    }
    None
}

/// Workspace-wide duplicate-function-name diagnostics. Each returned tuple is
/// `(path, diagnostic)` where `path` is the offending file and `diagnostic`
/// points at the colliding `DEFINE_NAME` span inside that file.
///
/// Iteration is sorted-by-path so the "first declaration wins, later ones
/// emit diagnostics" rule is deterministic.
#[salsa::tracked]
pub fn workspace_function_diagnostics(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<Vec<(PathBuf, Diagnostic)>> {
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut diagnostics: Vec<(PathBuf, Diagnostic)> = Vec::new();

    for f in files {
        let path = f.path(db).clone();
        let sigs = file_signature_inputs(db, f);
        for sig in sigs.iter() {
            // Phase 10: Every `smelt.extern` whose name collides with a
            // built-in in the canonical registry is an error. Checked before
            // the duplicate-user-definition check so externs always surface
            // the registry-collision message (more actionable than "already
            // defined in <other extern>" when both are shadowing the same
            // built-in).
            if sig.origin == smelt_types::SigOrigin::Extern
                && smelt_types::BuiltinRegistry::resolve(&sig.name).is_some()
            {
                diagnostics.push((
                    path.clone(),
                    Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Function `{}` is a built-in and cannot be redeclared with `smelt.extern`",
                            sig.name
                        ),
                        range: sig.name_range,
                        code: Some(DiagnosticCode::ExternCollidesWithBuiltin),
                        data: None,
                    },
                ));
                // Still fall through to the seen-map tracking so a second
                // extern with the same name also flags DuplicateFunctionDefinition.
            }

            if let Some(first_path) = seen.get(&sig.name) {
                diagnostics.push((
                    path.clone(),
                    Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Function `{}` is already defined in {}",
                            sig.name,
                            first_path.display()
                        ),
                        range: sig.name_range,
                        code: Some(DiagnosticCode::DuplicateFunctionDefinition),
                        data: None,
                    },
                ));
            } else {
                seen.insert(sig.name.clone(), path.clone());
            }
        }
    }

    Arc::new(diagnostics)
}

/// Filter `workspace_function_diagnostics` to a single file.
pub fn duplicate_function_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let target = file.path(db);
    workspace_function_diagnostics(db, workspace)
        .iter()
        .filter(|(p, _)| p == target)
        .map(|(_, d)| d.clone())
        .collect()
}

/// Per-file diagnostics for `smelt.define` bodies (Phase 5).
///
/// For each function declared in `file`, invokes the pure
/// [`function_body_check::check_function_body`] against the extracted
/// [`FunctionSig`] and the body AST. Emitted diagnostic codes:
///   - [`DiagnosticCode::DuplicateParameterName`]
///   - [`DiagnosticCode::UnknownIdentifier`]
///   - [`DiagnosticCode::FunctionBodyTypeMismatch`]
///
/// Pure-function-rule note: this helper is the thin Salsa wrapper; all logic
/// lives in the pure `function_body_check::check_function_body`. It reads
/// `parse_file` (for the body AST) and `file_signature_inputs` (for the
/// signatures). Body-only edits re-run this query but do not invalidate the
/// signature query, preserving §20H.
pub fn function_body_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::{SmeltType, SmeltTypeParseError};
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw).to_string();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };
    let sigs = file_signature_inputs(db, file);
    // Phase 13 deferred sorts: `AggExpr`, `WindowExpr`, and `SelectItems`
    // still produce `UnsupportedSort` at signature-parse time (Phases
    // 17+). Skip their bodies here to keep `example_diagnostics` green.
    // Phase 15: bare `TableExpr` is a valid sort whose body check must
    // happen at call-site expansion (no caller schema is available
    // here), so we also skip signature-time body checking for defines
    // with any `TableExpr` parameter.
    let has_deferred_phase13_param = |sig: &FunctionSig| -> bool {
        sig.params.iter().any(|p| match &p.type_ref {
            Some(Err(SmeltTypeParseError::UnsupportedSort { sort, .. }))
                if matches!(
                    sort.as_str(),
                    "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems"
                ) =>
            {
                true
            }
            Some(Ok(SmeltType::TableExpr(_))) => true,
            _ => false,
        })
    };

    // Phase 26: Build closures for nested Tier 1 expansion, mirroring the
    // closure setup in `smelt_fn_call_diagnostics_for_file`. When the Tier 2
    // body contains a `smelt.functions.*` call to a Tier 1 callee, we expand
    // it using the Tier 2 context's concrete parameter types so errors cascade
    // to the Tier 2 body check site with full frame stacks.
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));

    let sig_lookup = |name: &str| -> Option<FunctionSig> {
        resolve_function(db, workspace, name.to_string()).map(|arc| (*arc).clone())
    };

    let builtin_lookup = |name: &str| -> Option<&'static smelt_types::signatures::Signature> {
        smelt_types::BuiltinRegistry::resolve(name)
    };

    let lub = |a: &DataType, b: &DataType| -> DataType {
        let lhs = TypedColumn {
            data_type: a.clone(),
            nullable: true,
        };
        let rhs = TypedColumn {
            data_type: b.clone(),
            nullable: true,
        };
        type_inference::promote_types(&lhs, &rhs).data_type
    };

    // Phase 41: short-circuit body cascade for cycle members so the
    // existing nested-call body re-walk does not infinite-recurse on a
    // function-call cycle.
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    let body_lookup = |sig: &FunctionSig| -> Option<(String, function_body_check::BodyShape)> {
        if sig.origin == smelt_types::SigOrigin::Extern {
            return None;
        }
        if cycle_set.contains(&sig.name) {
            return None;
        }
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                let f_text = f.text(db);
                let f_clean = smelt_parser::strip_frontmatter(f_text).to_string();
                let f_parse = parse_file(db, *f);
                let f_syntax = f_parse.syntax();
                if let Some(ast) = AstFile::cast(f_syntax) {
                    for define in ast.defines() {
                        if define.name().as_deref() == Some(&sig.name) {
                            if let Some(body) = define.body() {
                                if let Some(select_stmt) = body.select_stmt() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Select(select_stmt),
                                    ));
                                }
                                if let Some(body_expr) = body.expression() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Expression(body_expr),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    };

    let decl_lookup = |sig: &smelt_types::signatures::FunctionSig| -> Option<std::path::PathBuf> {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                return Some(f.path(db).clone());
            }
        }
        None
    };

    let tableexpr_schema_lookup = |_arg_expr: &smelt_parser::ast::Expr,
                                   _ctx: &TypeContext|
     -> Option<Vec<(String, TypedColumn)>> {
        // Tier 2 body checks have no call-site schema for TableExpr params.
        // Those are skipped by `has_deferred_phase13_param` above anyway.
        None
    };

    let table_ref_schema_lookup =
        |_table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
            // Tier 2 body checks don't expand TableExpr-param bodies, so the
            // join-alias visitor never runs on this path.
            None
        };

    let default_type_lookup = |sig: &FunctionSig, param_name: &str| -> Option<DataType> {
        let decl_path = decl_lookup(sig)?;
        let f = files.iter().find(|f| f.path(db) == &decl_path)?;
        let f_parse = parse_file(db, *f);
        let ast = AstFile::cast(f_parse.syntax())?;
        for define in ast.defines() {
            if define.name().as_deref() != Some(&sig.name) {
                continue;
            }
            let Some(param_list) = define.param_list() else {
                continue;
            };
            for p in param_list.params() {
                if p.name().as_deref() != Some(param_name) {
                    continue;
                }
                let default_expr = p.default_value_expr()?;
                let empty_ctx = TypeContext::new();
                return type_inference::infer_expression_type(&default_expr, &empty_ctx)
                    .map(|t| t.data_type);
            }
        }
        None
    };

    // TB-5: see the parallel closure in `smelt_fn_call_diagnostics_for_file`.
    // This is the body-cascade variant — when a Tier 2 body contains a
    // `smelt.<dir>.<name>(...)` call, we need to enforce the same path-prefix
    // rule. Builds an identical validator over the workspace's files.
    //
    // Phase 2 (unified-paths): strip the matching scan-root prefix from the
    // file's directory so that a function in `models/fn.sql` is addressable
    // as `smelt.fn_name()` (empty dir_segments) rather than requiring
    // `smelt.models.fn_name()`.
    let scan_roots_for_body: Arc<Vec<String>> = {
        let proj_root = file.project_root(db).clone();
        match find_project(db, workspace, &proj_root) {
            Some(p) => project_paths(db, p),
            None => Arc::new(Vec::new()),
        }
    };
    let path_prefix_validator = |dir_segments: &[String], name: &str| -> bool {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if !sigs.iter().any(|s| s.name == name) {
                continue;
            }
            let abs_path = f.path(db);
            let proj_root = f.project_root(db);
            let Ok(rel) = abs_path.strip_prefix(proj_root) else {
                continue;
            };
            let effective_dir = rel.parent().unwrap_or(std::path::Path::new(""));
            // Strip the first matching scan-root prefix.
            let stripped_dir = scan_roots_for_body
                .iter()
                .find_map(|sr| effective_dir.strip_prefix(sr.as_str()).ok())
                .unwrap_or(effective_dir);
            let file_dir_segments: Vec<String> = stripped_dir
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            if file_dir_segments == dir_segments {
                return true;
            }
        }
        false
    };

    let nested_handler = |call: &smelt_parser::ast::SmeltPathCall,
                          nested_ctx: &TypeContext,
                          nested_text: &str|
     -> Vec<Diagnostic> {
        function_body_check::check_smelt_path_call(
            call,
            nested_ctx,
            nested_text,
            &sig_lookup,
            &builtin_lookup,
            &lub,
            &body_lookup,
            &decl_lookup,
            &tableexpr_schema_lookup,
            &default_type_lookup,
            &table_ref_schema_lookup,
            &|_: &smelt_parser::ast::SmeltPathCall| None,
            &path_prefix_validator,
        )
    };

    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else {
            continue;
        };
        let Some(sig) = sigs.iter().find(|s| s.name == name) else {
            continue;
        };
        if has_deferred_phase13_param(sig) {
            continue;
        }
        // Phase 41: skip body checks for cycle members. The cycle pre-pass
        // emits `FunctionCallCycle` for every participant; running the body
        // checker would risk re-entering the same cycle through nested
        // expansions even with `body_lookup`'s guard, and we already know
        // the diagnostic is wrong-shaped (the body itself is fine, the call
        // graph is not).
        if cycle_set.contains(&name) {
            continue;
        }
        let Some(body_expr) = define.body().and_then(|b| b.expression()) else {
            continue;
        };
        // Phase 26: Use `check_function_body_with_expansion` for Tier 2/3
        // functions so that nested `smelt.fn.*` calls to Tier 1 callees are
        // expanded inline using the Tier 2 context's concrete parameter types.
        // For Tier 1 functions (unannotated), `check_function_body` already
        // runs expansion at call-site only; the nested handler still handles
        // Tier 1 → Tier 1 chains correctly via the same guard in
        // `check_smelt_path_call`.
        out.extend(function_body_check::check_function_body_with_expansion(
            sig,
            &body_expr,
            &clean_text,
            &nested_handler,
        ));
        // Phase 24: Tier 3 return type check.
        if sig.tier == smelt_types::signatures::Tier::Three {
            out.extend(function_body_check::check_tier3_return_type(
                sig,
                &body_expr,
                &clean_text,
            ));
        }
    }
    out
}

/// Final backend set for a function in `file`, applying the narrow-only
/// rule (§16 #23).
///
/// Steps:
///   1. Look up the function's [`FunctionSig`] in this file.
///   2. For defines, walk the body and intersect each nested
///      `smelt.fn.*` callee's declared set + each `<backend>.<foo>` SQL
///      call into a running inferred set.
///   3. Apply the narrow-only rule: declared ⊆ inferred.
///
/// Returns `None` when no signature with the given name exists in the
/// file. When the narrow rule is violated the query still returns a
/// best-effort set (the inferred one) so downstream call-site checks
/// don't cascade — the diagnostic is emitted separately by
/// `backends_widening_diagnostics_for_file`.
///
/// Pure-function-rule note: the heavy lifting lives in
/// [`backends::infer_body_backends`] / [`backends::apply_narrow_rule`].
/// This wrapper builds a signature-lookup closure over the workspace
/// and re-parses the body via `parse_file`.
pub fn function_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    name: String,
) -> Option<smelt_types::BackendSet> {
    let sig = file_signature_inputs(db, file)
        .iter()
        .find(|s| s.name == name)
        .cloned()?;
    Some(compute_function_backends(db, workspace, file, &sig))
}

/// Non-Salsa helper: compute the final backend set for `sig` in `file`
/// using the given workspace for cross-file `smelt.fn.*` lookup.
fn compute_function_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    sig: &FunctionSig,
) -> smelt_types::BackendSet {
    use smelt_types::BackendSet;
    if sig.origin == smelt_types::SigOrigin::Extern {
        return backends::resolve_backends(sig, None).unwrap_or(BackendSet::All);
    }

    // Walk the body to infer.
    let inferred =
        body_inferred_backends(db, workspace, file, &sig.name).unwrap_or(BackendSet::All);
    backends::resolve_backends(sig, Some(inferred.clone())).unwrap_or(inferred)
}

/// Walk the body of `name` in `file` to compute its inferred backend
/// set. Returns `None` when the define has no resolvable body (e.g. a
/// parse-error recovery fragment).
fn body_inferred_backends(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
    name: &str,
) -> Option<smelt_types::BackendSet> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    for define in ast.defines() {
        if define.name().as_deref() == Some(name) {
            let body = define.body()?.expression()?;
            let sig_lookup = |callee_name: &str| -> Option<FunctionSig> {
                resolve_function(db, workspace, callee_name.to_string()).map(|arc| (*arc).clone())
            };
            return Some(backends::infer_body_backends(&body, &sig_lookup));
        }
    }
    None
}

/// Per-file diagnostics for the Phase 11 narrow-only rule. For each
/// `smelt.define` in `file` whose frontmatter declares a `backends:`
/// set broader than the body's inferred set, emit
/// [`DiagnosticCode::BackendsWideningNotAllowed`] anchored at the
/// declaration's name range. Also surfaces malformed-frontmatter errors
/// under the same code.
pub fn backends_widening_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        if let Some(msg) = sig.frontmatter_parse_error.as_ref() {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid frontmatter for `{}`: {}", sig.name, msg),
                range: sig.name_range,
                code: Some(DiagnosticCode::BackendsWideningNotAllowed),
                data: None,
            });
            continue;
        }
        if sig.origin == smelt_types::SigOrigin::Extern {
            // Externs with both a frontmatter `backends:` and the
            // dotted-backend sugar could disagree — but we accept the
            // frontmatter as authoritative and skip narrow checks
            // (there is no body to infer from).
            continue;
        }
        let Some(declared) = &sig.declared_backends else {
            continue;
        };
        let Some(inferred) = body_inferred_backends(db, workspace, file, &sig.name) else {
            continue;
        };
        if let Err(msg) = backends::apply_narrow_rule(Some(declared), &inferred) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Function `{}`: {}", sig.name, msg),
                range: sig.name_range,
                code: Some(DiagnosticCode::BackendsWideningNotAllowed),
                data: None,
            });
        }
    }
    out
}

/// Per-file diagnostics for the Phase 38 / Phase 42 `smelt.as_struct()`
/// backend-capability gate.
///
/// For each `smelt.define` in `file`, walks the body for `smelt.as_struct()`
/// calls. The set of backends to check against is determined by the function's
/// `declared_backends`:
///
/// - `Some(BackendSet::Only(names))` (Phase 38): the explicit declared set.
/// - `None` or `Some(BackendSet::All)` (Phase 42): the workspace's *active*
///   backend set — the distinct `target_type` values in `smelt.yml`'s
///   `targets:` map. Pass `active_backends = None` to fall back to the
///   Phase 38 behaviour (skip functions without an explicit `backends:`
///   declaration), e.g. when no `smelt.yml` is present in a synthetic test
///   workspace.
///
/// If any backend in the resolved set does not support struct literal
/// syntax, emits [`DiagnosticCode::AsStructUnsupportedBackend`] anchored at
/// the call span.
pub fn as_struct_backend_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
    active_backends: Option<&[String]>,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltAsStructCall;
    use smelt_types::signatures::BackendSet;

    let sigs = file_signature_inputs(db, file);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };
    let raw_text = file.text(db);
    let text = smelt_parser::strip_frontmatter(raw_text);

    let mut out = Vec::new();
    for sig in sigs.iter() {
        // Resolve which backends to check against. Functions with an explicit
        // `Only(names)` keep the Phase 38 behaviour; functions with `All`
        // (no `backends:` declaration) fall back to the workspace's active
        // backends — the diagnostic now fires for the implicit-default case
        // too. When `active_backends` is `None` and the function declares no
        // explicit set, we cannot compute a meaningful intersection and skip
        // the check (Phase 38 behaviour).
        let backends_to_check: Vec<String> = match &sig.declared_backends {
            Some(BackendSet::Only(names)) => names.clone(),
            Some(BackendSet::All) | None => match active_backends {
                Some(active) => active.to_vec(),
                None => continue,
            },
        };
        // Walk the define's body looking for SMELT_AS_STRUCT_CALL nodes.
        let define = ast
            .defines()
            .find(|d| d.name().as_deref() == Some(sig.name.as_str()));
        let Some(define) = define else { continue };

        // Collect all SMELT_AS_STRUCT_CALL descendants.
        let body_syntax = define.syntax().descendants();
        for node in body_syntax {
            if let Some(call) = SmeltAsStructCall::cast(node) {
                // Check each backend in the resolved set.
                for backend in &backends_to_check {
                    if !function_body_check::backend_supports_struct_literal(backend) {
                        let range =
                            smelt_parser::ast::text_range_to_range(&text, call.text_range());
                        out.push(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "smelt.as_struct() is not supported on backend `{backend}` \
                                 which does not have struct-literal capability"
                            ),
                            range,
                            code: Some(DiagnosticCode::AsStructUnsupportedBackend),
                            data: None,
                        });
                        break; // One diagnostic per call site is enough.
                    }
                }
            }
        }
    }
    out
}

/// Per-file diagnostics for the Phase 31 `provenance:` unstable-schema gate.
///
/// For each `smelt.define` or `smelt.extern` in `file` that declares
/// `provenance:` in its frontmatter, emits
/// [`DiagnosticCode::UnstableSchemaRequired`] anchored at the declaration's
/// name range when `unstable_schema` is `false`.
///
/// The `unstable_schema` flag should be read from the workspace's `smelt.yml`
/// by the caller (a Salsa tracked function) before invoking this pure helper.
///
/// Phase 43 note: frontmatter-parse diagnostics (malformed YAML, unknown
/// keys) are emitted by [`frontmatter_parse_diagnostics_for_file`] instead;
/// they fire unconditionally regardless of the `unstable_schema` flag, so
/// production workspaces that opt into `unstable_schema: true` still surface
/// parse errors.
pub fn provenance_unstable_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
    unstable_schema: bool,
) -> Vec<Diagnostic> {
    use smelt_planner::logical::Provenance;

    if unstable_schema {
        return Vec::new();
    }

    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Re-use the cached signature list to get accurate name_range values.
    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();

    // Check smelt.define declarations.
    for define in ast.defines() {
        let Some(fm) = define.frontmatter(raw_text) else {
            continue;
        };
        let (props, _fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        let name = define.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        if matches!(props.provenance, Provenance::Declared(_)) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "function `{name}` declares `provenance:` in its frontmatter \
                     but the workspace does not have `unstable_schema: true` in smelt.yml; \
                     set `unstable_schema: true` to enable this unstable feature"
                ),
                range,
                code: Some(DiagnosticCode::UnstableSchemaRequired),
                data: None,
            });
        }
    }

    // Check smelt.extern declarations.
    for ext in ast.externs() {
        let Some(fm) = ext.frontmatter(raw_text) else {
            continue;
        };
        let (props, _fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        let name = ext.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        if matches!(props.provenance, Provenance::Declared(_)) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "function `{name}` declares `provenance:` in its frontmatter \
                     but the workspace does not have `unstable_schema: true` in smelt.yml; \
                     set `unstable_schema: true` to enable this unstable feature"
                ),
                range,
                code: Some(DiagnosticCode::UnstableSchemaRequired),
                data: None,
            });
        }
    }

    out
}

/// Per-file diagnostics for malformed / unknown-key frontmatter on
/// `smelt.define` and `smelt.extern` declarations.
///
/// For each declaration's frontmatter (if any), runs the pure
/// [`smelt_planner::logical::parse_function_properties`] parser and converts
/// every [`smelt_planner::logical::FrontmatterDiagnostic`] it returns into a
/// full [`Diagnostic`] anchored at the declaration's name range.
///
/// Unlike [`provenance_unstable_diagnostics_for_file`], this helper does
/// **not** consult the `unstable_schema` flag — frontmatter parse errors and
/// unknown-key warnings fire unconditionally so they remain visible on
/// workspaces that opt into `unstable_schema: true`. (Phase 43.)
pub fn frontmatter_parse_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    // Re-use the cached signature list to get accurate name_range values.
    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();

    // Check smelt.define declarations.
    for define in ast.defines() {
        let Some(fm) = define.frontmatter(raw_text) else {
            continue;
        };
        let (_props, fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        if fm_diags.is_empty() {
            continue;
        }
        let name = define.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        for fm_diag in fm_diags {
            out.push(frontmatter_diag_to_diagnostic(fm_diag, range));
        }
    }

    // Check smelt.extern declarations.
    for ext in ast.externs() {
        let Some(fm) = ext.frontmatter(raw_text) else {
            continue;
        };
        let (_props, fm_diags) = smelt_planner::logical::parse_function_properties(&fm);
        if fm_diags.is_empty() {
            continue;
        }
        let name = ext.name().unwrap_or_default();
        let range = sigs
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| sig.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        for fm_diag in fm_diags {
            out.push(frontmatter_diag_to_diagnostic(fm_diag, range));
        }
    }

    out
}

/// Translate a parser-side [`smelt_planner::logical::FrontmatterDiagnostic`]
/// into a full [`Diagnostic`] anchored at the declaration's name range.
/// Phase 43.
fn frontmatter_diag_to_diagnostic(
    fm: smelt_planner::logical::FrontmatterDiagnostic,
    range: Range,
) -> Diagnostic {
    use smelt_planner::logical::FrontmatterSeverity;
    let severity = match fm.severity {
        FrontmatterSeverity::Error => DiagnosticSeverity::Error,
        FrontmatterSeverity::Warning => DiagnosticSeverity::Warning,
    };
    Diagnostic {
        severity,
        message: fm.message,
        range,
        code: Some(DiagnosticCode::FrontmatterParseError),
        data: None,
    }
}

/// Phase 52 — per-file check: reject `smelt.extern` declarations with
/// fragment-sort parameters (`SelectItems`, `OrderSpec`).
///
/// Fragment sorts require PASSING clauses, which `smelt.extern` does not
/// support (§16 #18 deferral). This catches `SelectItems<…>` (a valid
/// `SmeltType` that passes type-ref parse) and `OrderSpec` (which is an
/// `UnsupportedSort` parse error, but with a specific sort name).
pub fn extern_fragment_param_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::SmeltType;
    use smelt_types::signatures::SmeltTypeParseError;

    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();

    for sig in sigs.iter() {
        if sig.origin != smelt_types::SigOrigin::Extern {
            continue;
        }
        for param in &sig.params {
            let is_fragment = match &param.type_ref {
                // `SelectItems<…>` parses as Ok(SmeltType::SelectItems)
                Some(Ok(SmeltType::SelectItems { .. })) => true,
                // `OrderSpec<…>` (with angle brackets) parses as UnsupportedSort;
                // check the sort name to distinguish fragment sorts from
                // genuinely unknown types (which emit InvalidFunctionTypeRef).
                Some(Err(SmeltTypeParseError::UnsupportedSort { sort, .. })) => {
                    sort.as_str() == "OrderSpec"
                }
                // Bare `OrderSpec` (no angle brackets) parses as Malformed because
                // `parse_smelt_type` requires `<…>` for non-TableExpr sorts. Detect
                // it by inspecting the span_text's leading identifier.
                Some(Err(SmeltTypeParseError::Malformed { span_text })) => {
                    let head = span_text
                        .trim()
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("");
                    head == "OrderSpec"
                }
                _ => false,
            };
            if is_fragment {
                if let Some(range) = param.type_ref_range {
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "parameter `{}` of `smelt.extern {}` uses a fragment-sort type; \
                             fragment sorts (`SelectItems`, `OrderSpec`) require PASSING clauses \
                             which `smelt.extern` does not support (§16 #18)",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::ExternFragmentParamUnsupported),
                        data: None,
                    });
                }
            }
        }
    }
    out
}

/// Phase 52 — per-file advisory: when a transparent function lacks declared
/// provenance and a WHERE clause sits above its call site, emit a Hint.
///
/// Runs on all files (model and function). Anchors the Hint at the WHERE
/// clause's text range. Only fires when `unstable_schema: true` (so the
/// provenance system is active).
pub fn missing_provenance_advisory_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_planner::logical::Provenance;

    let raw_text = file.text(db);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    // Walk all SELECT statements in the file.
    for node in ast.syntax().descendants() {
        let Some(select) = smelt_parser::ast::SelectStmt::cast(node) else {
            continue;
        };
        // Only interested in SELECTs with a WHERE clause.
        let Some(where_clause) = select.where_clause() else {
            continue;
        };
        let where_range =
            smelt_parser::ast::text_range_to_range(raw_text, where_clause.text_range());

        // Find smelt.functions.* calls in the FROM clause.
        let Some(from_clause) = select.from_clause() else {
            continue;
        };
        for table_ref in from_clause.table_refs() {
            let Some(call) = table_ref.smelt_path_call() else {
                continue;
            };
            // Derive the function name from the call path segments (last segment = name).
            let segments = call.segments();
            let Some(fn_name) = segments.last().cloned() else {
                continue;
            };

            let Some(sig) = resolve_function(db, workspace, fn_name.clone()) else {
                continue;
            };
            // Only transparent (smelt.define) functions.
            if sig.origin != smelt_types::SigOrigin::Define {
                continue;
            }

            // Check if the function has declared provenance.
            // Sort by path to match resolve_function's deterministic "first wins" order.
            let mut sorted_files: Vec<SourceFile> = workspace.files(db).to_vec();
            sorted_files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
            let has_provenance = sorted_files
                .iter()
                .copied()
                .find(|f| {
                    file_signature_inputs(db, *f)
                        .iter()
                        .any(|s| s.name == fn_name)
                })
                .and_then(|decl_file| {
                    let decl_raw = decl_file.text(db).clone();
                    let decl_parse = parse_file(db, decl_file);
                    let decl_syntax = decl_parse.syntax();
                    let decl_ast = AstFile::cast(decl_syntax)?;
                    let fm = decl_ast
                        .defines()
                        .find(|d| d.name().as_deref() == Some(fn_name.as_str()))
                        .and_then(|d| d.frontmatter(&decl_raw))?;
                    let (props, _) = smelt_planner::logical::parse_function_properties(&fm);
                    Some(matches!(props.provenance, Provenance::Declared(_)))
                })
                .unwrap_or(false);

            if !has_provenance {
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Hint,
                    message: format!(
                        "function `{fn_name}` is transparent but has no declared `provenance:` \
                         — filter pushdown into the function body will be skipped; \
                         add `provenance:` frontmatter to enable this optimisation"
                    ),
                    range: where_range,
                    code: Some(DiagnosticCode::MissingProvenancePushdownAdvisory),
                    data: None,
                });
            }
        }
    }
    out
}

/// Per-file diagnostics for `smelt.functions.<name>(...)` call sites.
///
/// For every `SMELT_PATH_CALL` AST node in `file`, runs the pure
/// [`function_body_check::check_smelt_path_call`] with closures over
/// [`resolve_function`] (for signature lookup) and [`parse_file`] (for
/// body lookup). Call-site diagnostics emitted:
///   - [`DiagnosticCode::UnknownSmeltFn`]
///   - [`DiagnosticCode::MissingArgument`]
///   - [`DiagnosticCode::ArgTypeMismatch`]
///   - Any body-cascading [`DiagnosticCode::FunctionBodyTypeMismatch`] /
///     [`DiagnosticCode::UnknownIdentifier`] with
///     [`DiagnosticData::ExpansionFrames`] attached.
///
/// Pure-function-rule note: the analysis lives in
/// `function_body_check::check_smelt_path_call`; this wrapper builds the
/// call-site [`TypeContext`] (seeded with the workspace's signatures) and
/// threads Salsa-backed closures through for signature / body lookup.
pub fn smelt_fn_call_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_parser::ast::SmeltPathCall;
    use smelt_parser::syntax_kind::SyntaxKind;

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw).to_string();

    // Build the set of define names whose bodies are deferred (i.e. skipped
    // by `function_body_diagnostics_for_file` because they have TableExpr /
    // SelectItems params). Calls inside those bodies are NOT dispatched
    // through the body re-walk, so we must check them here.
    use smelt_types::signatures::SmeltType;
    let deferred_define_names: std::collections::HashSet<String> = {
        let ast_file = AstFile::cast(syntax.clone());
        ast_file
            .as_ref()
            .map(|af| {
                af.defines()
                    .filter(|def| {
                        let Some(sig) = def.name().and_then(|n| {
                            let sigs = file_signature_inputs(db, file);
                            sigs.iter().find(|s| s.name == n).cloned()
                        }) else {
                            return false;
                        };
                        sig.params.iter().any(|p| match &p.type_ref {
                            Some(Err(
                                smelt_types::signatures::SmeltTypeParseError::UnsupportedSort {
                                    sort,
                                    ..
                                },
                            )) if matches!(
                                sort.as_str(),
                                "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems"
                            ) =>
                            {
                                true
                            }
                            Some(Ok(SmeltType::TableExpr(_))) => true,
                            _ => false,
                        })
                    })
                    .filter_map(|def| def.name())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Collect SMELT_PATH_CALL nodes (smelt.functions.* form).
    //
    // For nodes outside a DEFINE_BODY: always include (top-level call sites
    // in models / expression-only defines).
    //
    // For nodes inside a DEFINE_BODY: include only when the enclosing define
    // is deferred (has TableExpr/SelectItems params) — those bodies are
    // skipped by `function_body_diagnostics_for_file`, so any nested
    // `smelt.functions.*` calls would otherwise go unchecked. For
    // non-deferred defines, the body re-walk dispatches nested calls through
    // `nested_path_handler`, so we exclude them here to avoid duplication.
    let path_call_nodes: Vec<SmeltPathCall> = syntax
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::SMELT_PATH_CALL)
        .filter(|n| {
            // Skip `smelt.config.var(...)` — those are handled by
            // `check_config_var_call_diagnostics` in `check_file_diagnostics`,
            // not by the smelt-function call checker.
            if let Some(call) = SmeltPathCall::cast(n.clone()) {
                let segs = call.segments();
                if segs.len() == 2
                    && segs[0].to_lowercase() == "config"
                    && segs[1].to_lowercase() == "var"
                {
                    return false;
                }
            }
            let inside_define_body = n.ancestors().any(|a| a.kind() == SyntaxKind::DEFINE_BODY);
            if !inside_define_body {
                return true; // Top-level call site — always check.
            }
            // Inside a define body: only check if the define is deferred.
            // Walk up to find the enclosing DEFINE node and check its name.
            use smelt_parser::syntax_kind::SyntaxKind as Sk;
            let enclosing_define_name = n
                .ancestors()
                .find(|a| a.kind() == Sk::SMELT_DEFINE)
                .and_then(|def| {
                    use smelt_parser::ast::SmeltDefine;
                    SmeltDefine::cast(def).and_then(|d| d.name())
                });
            enclosing_define_name
                .map(|nm| deferred_define_names.contains(&nm))
                .unwrap_or(false)
        })
        .filter_map(SmeltPathCall::cast)
        .collect();

    if path_call_nodes.is_empty() {
        return Vec::new();
    }

    // Build the call-site type context. For model-files we reuse the
    // model's TypeContext (source/CTE/model columns all in scope) and
    // layer the workspace's FunctionSig map on top. For pure function-only
    // files there is no SELECT, so `type_context` returns an empty TC —
    // that's fine, the body walk doesn't need SQL scope.
    let mut ctx: TypeContext = (*type_context(db, workspace, file)).clone();

    // Seed every workspace-visible `FunctionSig` so path-call type inference
    // can resolve nested function returns. Iterating all files is O(N) but
    // only runs when a call site is present — pure function files with no
    // calls skip this entirely.
    let mut files: Vec<SourceFile> = workspace.files(db).to_vec();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in &files {
        let sigs = file_signature_inputs(db, *f);
        for sig in sigs.iter() {
            ctx.add_function_signature(&sig.name, sig.clone());
        }
    }

    // Closures for pure checker. `sig_lookup` wraps `resolve_function`;
    // `body_lookup` re-parses the declaring file and locates the matching
    // `smelt.define` body. `builtin_lookup` dispatches to the built-in
    // registry so calls like `smelt.fn.COALESCE(...)` go through
    // `unify_call` when no user-declared function shadows the name.
    let sig_lookup = |name: &str| -> Option<FunctionSig> {
        resolve_function(db, workspace, name.to_string()).map(|arc| (*arc).clone())
    };

    let builtin_lookup = |name: &str| -> Option<&'static smelt_types::signatures::Signature> {
        smelt_types::BuiltinRegistry::resolve(name)
    };

    // LUB closure for `unify_call` — forwards to the canonical numeric
    // promotion routine in `type_inference`.
    let lub = |a: &DataType, b: &DataType| -> DataType {
        let lhs = TypedColumn {
            data_type: a.clone(),
            nullable: true,
        };
        let rhs = TypedColumn {
            data_type: b.clone(),
            nullable: true,
        };
        type_inference::promote_types(&lhs, &rhs).data_type
    };

    // Phase 41: capture the cycle set so the body cascade short-circuits
    // for cycle-participant functions. Without this guard, calling
    // `body_lookup(cycle_a)` triggers a re-walk of cycle_a's body which
    // calls cycle_b, which calls back into cycle_a — overflowing the stack.
    // The cycle pre-pass surfaces a `FunctionCallCycle` diagnostic instead.
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    let body_lookup = |sig: &FunctionSig| -> Option<(String, function_body_check::BodyShape)> {
        // Externs have no body — skip. Defines alone carry a re-walkable body.
        if sig.origin == smelt_types::SigOrigin::Extern {
            return None;
        }
        if cycle_set.contains(&sig.name) {
            return None;
        }
        // Find the file declaring this function.
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                let f_text = f.text(db);
                let f_clean = smelt_parser::strip_frontmatter(f_text).to_string();
                let f_parse = parse_file(db, *f);
                let f_syntax = f_parse.syntax();
                if let Some(ast) = AstFile::cast(f_syntax) {
                    for define in ast.defines() {
                        if define.name().as_deref() == Some(&sig.name) {
                            if let Some(body) = define.body() {
                                // Phase 15: prefer the SELECT shape when
                                // present (e.g. `TableExpr`-returning
                                // defines). Fall back to the scalar
                                // expression shape for Phase-5-shaped
                                // bodies.
                                if let Some(select_stmt) = body.select_stmt() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Select(select_stmt),
                                    ));
                                }
                                if let Some(body_expr) = body.expression() {
                                    return Some((
                                        f_clean,
                                        function_body_check::BodyShape::Expression(body_expr),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    };

    // Phase 12: resolve the declaring file path for a `FunctionSig` so that
    // each expansion frame records `decl_path` + `decl_range`. The renderer
    // and the LSP use these to build outer-to-inner frame trailers and
    // `DiagnosticRelatedInformation` entries linking back to each declaration.
    let decl_lookup = |sig: &smelt_types::signatures::FunctionSig| -> Option<std::path::PathBuf> {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if sigs.iter().any(|s| s.name == sig.name) {
                return Some(f.path(db).clone());
            }
        }
        None
    };

    // Phase 15: resolve a `TableExpr` argument expression to its
    // caller-supplied column schema. Today we recognise two shapes:
    //   - `smelt.models.model_name` — look up the referenced model's
    //     typed schema through the Salsa provider.
    //   - `smelt.sources.src.table` — look up the source's declared
    //     columns (Phase 15 fixtures only use `smelt.ref`, but extending
    //     is cheap).
    // Any other shape (bare subquery, another function call, …)
    // resolves to `None`; the body checker then sees no FROM-scope
    // entries for that parameter and bare columns emit
    // `UnknownIdentifier` with a frame rooted at the call site.
    let tableexpr_schema_lookup = |arg_expr: &smelt_parser::ast::Expr,
                                   ctx: &TypeContext|
     -> Option<Vec<(String, TypedColumn)>> {
        // Try to extract a `smelt.models.X` from the argument's function
        // call node, if any. We accept the call nested inside an
        // EXPRESSION wrapper.
        use smelt_parser::ast::Subquery;
        use smelt_parser::syntax_kind::SyntaxKind as Sk;

        // Phase 4: unified `smelt.<path>` value form. Walks the
        // expression for any `SMELT_PATH_REF`; resolves through the
        // path-tuple resolver and dispatches on the resolved kind.
        for node in arg_expr.syntax().descendants() {
            if node.kind() == Sk::SMELT_PATH_REF {
                let path_ref = match SmeltPathRef::cast(node) {
                    Some(p) => p,
                    None => continue,
                };
                let path = path_ref.segments();
                if let Some(resolved) = resolve_ref_path(db, workspace, path.clone()) {
                    match resolved.kind {
                        RefKind::Model => {
                            if let Some(sf) = resolved.source_file {
                                let schema = resolved_model_schema(db, workspace, sf);
                                let cols: Vec<(String, TypedColumn)> = schema
                                    .columns
                                    .iter()
                                    .map(|c| {
                                        let tc = c.data_type.clone().unwrap_or(TypedColumn {
                                            data_type: DataType::Unknown,
                                            nullable: true,
                                        });
                                        (c.name.clone(), tc)
                                    })
                                    .collect();
                                if !cols.is_empty() {
                                    return Some(cols);
                                }
                            }
                        }
                        RefKind::Seed => {
                            // Seed columns come from `discover_seed_infos`;
                            // reuse the legacy provider helper.  The lookup
                            // key is `address_segments.join("_")` which equals
                            // `path.join("_")` since resolve_ref_path matches
                            // seeds by address_segments == path.
                            let key = path.join("_");
                            let provider = SalsaRefSchemaProvider::new(db, workspace);
                            if let Some(cols) = provider.seed_columns(&key) {
                                return Some(cols);
                            }
                        }
                        RefKind::Source => {
                            // Path tuple shape: ["sources", <src>, <tbl>].
                            if path.len() >= 3 {
                                let source_name = path[path.len() - 2].clone();
                                let table_name = path[path.len() - 1].clone();
                                let sources = sources_config(
                                    db,
                                    *workspace
                                        .projects(db)
                                        .first()
                                        .expect("at least one project"),
                                );
                                for s in &sources.sources {
                                    if s.name != source_name {
                                        continue;
                                    }
                                    for t in &s.tables {
                                        if t.name == table_name {
                                            let cols: Vec<(String, TypedColumn)> = t
                                                .columns
                                                .iter()
                                                .map(|c| {
                                                    (
                                                        c.name.clone(),
                                                        TypedColumn {
                                                            data_type: c
                                                                .data_type
                                                                .clone()
                                                                .unwrap_or(DataType::Unknown),
                                                            nullable: true,
                                                        },
                                                    )
                                                })
                                                .collect();
                                            return Some(cols);
                                        }
                                    }
                                }
                            }
                        }
                        RefKind::Function | RefKind::Test => {
                            // A function or test in `TableExpr` arg
                            // position is a kind mismatch; surface the
                            // failure to downstream UnknownIdentifier
                            // diagnostics — no schema is provided.
                            return None;
                        }
                    }
                }
            }
        }

        // Phase 46: derived-table / inline-subquery argument shape —
        // `(SELECT … [FROM y])`. The arg expression contains a
        // `SUBQUERY` node; infer its output schema from the inner
        // SELECT statement using the call-site context (so any CTE /
        // model / source qualifiers in the SELECT's FROM clause
        // resolve).
        for node in arg_expr.syntax().descendants() {
            if node.kind() == Sk::SUBQUERY {
                if let Some(sub) = Subquery::cast(node) {
                    if let Some(inner) = sub.select_stmt() {
                        let cols = type_inference::infer_select_output_schema(&inner, ctx);
                        if !cols.is_empty() {
                            return Some(cols);
                        }
                    }
                }
            }
        }

        // Phase 46: CTE reference — the arg is a bare identifier
        // matching a CTE in scope. Walking the AST of a bare
        // identifier descends through EXPRESSION → COLUMN_REF → IDENT,
        // so we just compare the trimmed text against the CTE name
        // table on the call-site context.
        let trimmed = arg_expr.text().trim().to_string();
        if !trimmed.is_empty() && ctx.is_cte(&trimmed) {
            let cols: Vec<(String, TypedColumn)> = ctx
                .cte_columns(&trimmed)
                .into_iter()
                .map(|(name, tc)| (name.to_string(), tc.clone()))
                .collect();
            if !cols.is_empty() {
                return Some(cols);
            }
        }

        None
    };

    // Phase 45: resolve a `TableRef` inside a function body's FROM /
    // JOIN clauses to the joined schema, so the body's bare-column
    // resolver can see e.g. `dim_customer.col` from
    // `JOIN smelt.models.dim_customer AS dim_customer`.
    //
    // Supported shapes:
    //   - `smelt.models.model` — same path as `tableexpr_schema_lookup`.
    //   - `smelt.sources.src.tbl` — same path as `tableexpr_schema_lookup`.
    // Unsupported shapes (subqueries, CTE refs, derived tables) return
    // `None`. Phase 46 widens to those.
    let table_ref_schema_lookup =
        |table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
            // `smelt.<path>` value-form in JOIN position.
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segs = path_ref.segments();
                let model_name = segs.last().cloned().unwrap_or_default();
                let seed_key = segs.join("_");
                let provider = SalsaRefSchemaProvider::new(db, workspace);
                return provider
                    .resolved_columns(&model_name)
                    .or_else(|| provider.seed_columns(&seed_key));
            }

            None
        };

    // Path-call CTE wildcard expansion deferred to a follow-on phase.
    // Return `None` to fall back to the opaque-CTE marker.
    let smelt_path_schema_lookup =
        |_call: &smelt_parser::ast::SmeltPathCall| -> Option<Vec<(String, TypedColumn)>> { None };

    // TB-5: validates that a `smelt.<dir>.<name>(...)` call path's directory
    // segments equal the workspace-relative directory of the file declaring
    // a function with `name`. Returns `true` iff some workspace file at
    // exactly that directory declares the function.
    //
    // Spec rule (`functions.md` §"Function call syntax"): the file stem is
    // *not* a path component. So a function declared in `functions/status.sql`
    // is callable as `smelt.functions.is_shipped(...)`, not
    // `smelt.functions.status.is_shipped(...)`.
    //
    // Phase 2 (unified-paths): strip the matching scan-root prefix from the
    // file's directory so that a function in `models/fn.sql` under
    // `paths: ["models"]` is addressable as `smelt.fn_name()` (empty
    // dir_segments) rather than requiring `smelt.models.fn_name()`.
    let scan_roots_for_call: Arc<Vec<String>> = {
        let proj_root = file.project_root(db).clone();
        match find_project(db, workspace, &proj_root) {
            Some(p) => project_paths(db, p),
            None => Arc::new(Vec::new()),
        }
    };
    let path_prefix_validator = |dir_segments: &[String], name: &str| -> bool {
        for f in &files {
            let sigs = file_signature_inputs(db, *f);
            if !sigs.iter().any(|s| s.name == name) {
                continue;
            }
            let abs_path = f.path(db);
            let proj_root = f.project_root(db);
            let Ok(rel) = abs_path.strip_prefix(proj_root) else {
                continue;
            };
            let effective_dir = rel.parent().unwrap_or(std::path::Path::new(""));
            // Strip the first matching scan-root prefix.
            let stripped_dir = scan_roots_for_call
                .iter()
                .find_map(|sr| effective_dir.strip_prefix(sr.as_str()).ok())
                .unwrap_or(effective_dir);
            let file_dir_segments: Vec<String> = stripped_dir
                .components()
                .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
                .collect();
            if file_dir_segments == dir_segments {
                return true;
            }
        }
        false
    };

    // Phase 17: resolve a parameter's default-value expression by
    // re-parsing the declaring file and walking to the matching
    // PARAM node. Returns the inferred `DataType` of the default
    // expression (against an empty context — defaults are
    // self-contained per research §3).
    let default_type_lookup = |sig: &FunctionSig, param_name: &str| -> Option<DataType> {
        let decl_path = decl_lookup(sig)?;
        let f = files.iter().find(|f| f.path(db) == &decl_path)?;
        let f_parse = parse_file(db, *f);
        let ast = AstFile::cast(f_parse.syntax())?;
        for define in ast.defines() {
            if define.name().as_deref() != Some(&sig.name) {
                continue;
            }
            let Some(param_list) = define.param_list() else {
                continue;
            };
            for p in param_list.params() {
                if p.name().as_deref() != Some(param_name) {
                    continue;
                }
                let default_expr = p.default_value_expr()?;
                let empty_ctx = TypeContext::new();
                return type_inference::infer_expression_type(&default_expr, &empty_ctx)
                    .map(|t| t.data_type);
            }
        }
        None
    };

    let mut out = Vec::new();
    for call in &path_call_nodes {
        out.extend(function_body_check::check_smelt_path_call(
            call,
            &ctx,
            &clean_text,
            &sig_lookup,
            &builtin_lookup,
            &lub,
            &body_lookup,
            &decl_lookup,
            &tableexpr_schema_lookup,
            &default_type_lookup,
            &table_ref_schema_lookup,
            &smelt_path_schema_lookup,
            &path_prefix_validator,
        ));
    }
    out
}

/// Per-file diagnostics for malformed `smelt.define` parameter / return type
/// annotations (Phase 4). Iterates `functions_in_file(file)` and emits a
/// diagnostic for each [`ParamSpec::type_ref`] or
/// [`FunctionSig::return_type`] that carries a parse error.
///
/// Pure-function-rule note: the heavy lifting lives on
/// `smelt_types::signatures` — this helper is just a thin reader over the
/// signature query's cached output.
pub fn invalid_function_type_ref_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::SmeltTypeParseError;
    // Phase 13: the parser accepts `TableExpr`, `AggExpr`, `WindowExpr`, and
    // `SelectItems` sort heads in parameter / return positions so the Step 3
    // fixtures can land. `smelt-types::parse_smelt_type` still rejects those
    // sorts (full type-system wiring is Phases 14+), so we filter out known
    // Phase-13-deferred shapes here to keep `example_diagnostics` green:
    //   - `TableExpr`, `AggExpr`, `WindowExpr`, `SelectItems` in a tagged
    //     `UnsupportedSort` (forms with `<...>`),
    //   - the same heads with no angle brackets (surface as `Malformed`
    //     with leading identifier `TableExpr` / etc.). The bare form is
    //     legal per the plan (e.g. `-> TableExpr`), so a malformed whose
    //     source starts with one of these heads is deferred too.
    // Any *other* `UnsupportedSort` (e.g. `FooExpr<T>`) or any
    // `UnknownInner` / `NestedExpr` error still surfaces.
    let is_phase13_deferred_sort_name = |sort: &str| -> bool {
        matches!(sort, "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems")
    };
    let is_deferred_phase13_sort = |err: &SmeltTypeParseError| -> bool {
        match err {
            SmeltTypeParseError::UnsupportedSort { sort, .. } => {
                is_phase13_deferred_sort_name(sort)
            }
            SmeltTypeParseError::Malformed { span_text } => {
                // Leading identifier is the source head (bare `TableExpr`
                // with no `<...>` lands here). Extract the first word.
                let head = span_text
                    .trim()
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("");
                is_phase13_deferred_sort_name(head)
            }
            _ => false,
        }
    };
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        for param in &sig.params {
            if let (Some(Err(err)), Some(range)) = (&param.type_ref, param.type_ref_range) {
                if is_deferred_phase13_sort(err) {
                    continue;
                }
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!("Invalid type for parameter `{}`: {}", param.name, err),
                    range,
                    code: Some(DiagnosticCode::InvalidFunctionTypeRef),
                    data: None,
                });
            }
        }
        if let (Some(Err(err)), Some(range)) = (&sig.return_type, sig.return_type_range) {
            if is_deferred_phase13_sort(err) {
                continue;
            }
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid return type for function `{}`: {}", sig.name, err),
                range,
                code: Some(DiagnosticCode::InvalidFunctionTypeRef),
                data: None,
            });
        }
    }
    out
}

/// Phase 22 helper: collect the CTE names declared in the WITH clause of
/// `fn_name`'s body SELECT inside `ast`.
///
/// Returns an empty vec when the function has no SELECT body, no WITH
/// clause, or when `fn_name` isn't found.  Pure — only walks the CST.
fn cte_names_from_define(ast: &AstFile, fn_name: &str) -> Vec<String> {
    for define in ast.defines() {
        if define.name().as_deref() != Some(fn_name) {
            continue;
        }
        let Some(body) = define.body() else {
            return vec![];
        };
        let Some(select) = body.select_stmt() else {
            return vec![];
        };
        let Some(with_clause) = select.with_clause() else {
            return vec![];
        };
        return with_clause.ctes().filter_map(|c| c.name()).collect();
    }
    vec![]
}

/// Phase 19: emit `UnknownContext` when an `Expr<T, ctx>` or
/// `SelectItems<Kind, ctx>` parameter's context name doesn't match any other
/// parameter **or CTE name** in the same `smelt.define`.
///
/// Phase 22 extends this function to:
/// 1. Also validate the context embedded in `SelectItems<Kind, ctx>` type
///    annotations (previously only `param.context` / `Expr<T, ctx>` was
///    checked).
/// 2. Accept CTE names from the function body's WITH clause as valid
///    context names — so `metrics: SelectItems<Agg, sessionized>` does not
///    emit `UnknownContext` when `sessionized` is defined as a CTE in the
///    same function body.
///
/// Pure: re-uses the cached `file_signature_inputs` output.
pub fn unknown_context_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    use smelt_types::signatures::{ContextRef, SmeltType};

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };

    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for sig in sigs.iter() {
        let param_names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();
        let cte_names = cte_names_from_define(&ast, &sig.name);

        let is_valid_ctx = |ctx_name: &str| -> bool {
            param_names.contains(&ctx_name) || cte_names.iter().any(|n| n == ctx_name)
        };

        for param in &sig.params {
            // Case 1: `Expr<T, ctx>` / `AggExpr<T, ctx>` / `WindowExpr<T, ctx>` —
            // context is stored in `param.context`.
            if let Some(ctx_ref) = &param.context {
                let ctx_name = ctx_ref.name();
                if !is_valid_ctx(ctx_name) {
                    let Some(range) = param.type_ref_range else {
                        continue;
                    };
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Context `{ctx_name}` in parameter `{}` does not name a parameter \
                             or CTE in `{}`",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::UnknownContext),
                        data: None,
                    });
                }
            }

            // Case 2: `SelectItems<Kind, ctx>` — context is stored inside
            // `param.type_ref` as `SmeltType::SelectItems { context: Some(...) }`.
            if let Some(Ok(SmeltType::SelectItems {
                context: Some(ContextRef(ctx_name)),
                ..
            })) = &param.type_ref
            {
                if !is_valid_ctx(ctx_name) {
                    let Some(range) = param.type_ref_range else {
                        continue;
                    };
                    out.push(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Context `{ctx_name}` in parameter `{}` does not name a parameter \
                             or CTE in `{}`",
                            param.name, sig.name
                        ),
                        range,
                        code: Some(DiagnosticCode::UnknownContext),
                        data: None,
                    });
                }
            }
        }
    }
    out
}

/// Phase 20: emit [`DiagnosticCode::CteCycle`] for every `smelt.define` in
/// `file` whose SELECT body contains a cyclic CTE reference.
///
/// Uses [`function_body_check::extract_function_body_cte_schemas`] with an
/// empty seed context (cycle detection is purely structural).
///
/// Pure: reads `parse_file`.
pub fn cte_cycle_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw);
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };
    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(body) = define.body() else { continue };
        let Some(select) = body.select_stmt() else {
            continue;
        };
        let empty_ctx = type_inference::TypeContext::new();
        let (_ctx, cycle_diags) = function_body_check::extract_function_body_cte_schemas(
            &select,
            &empty_ctx,
            &clean_text,
        );
        out.extend(cycle_diags);
    }
    out
}

/// Phase 20: emit [`DiagnosticCode::ContextMismatch`] for every `smelt.define`
/// in `file` whose explicit `Expr<T, ctx>` annotation disagrees with the
/// context inferred from the parameter's splice point.
///
/// Uses [`function_body_check::context_mismatch_diagnostics_for_fn`].
///
/// Pure: reads `parse_file` and `file_signature_inputs`.
pub fn context_mismatch_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return vec![];
    };
    let sigs = file_signature_inputs(db, file);
    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else { continue };
        let Some(sig) = sigs.iter().find(|s| s.name == name) else {
            continue;
        };
        let Some(body) = define.body() else { continue };
        let Some(select) = body.select_stmt() else {
            continue;
        };
        out.extend(function_body_check::context_mismatch_diagnostics_for_fn(
            sig, &select,
        ));
    }
    out
}

// ============================================================================
// Semantic queries
// ============================================================================

/// Resolve a `smelt.ref(name)` call to the `SourceFile` that defines it.
#[salsa::tracked]
pub fn resolve_ref(
    db: &dyn salsa::Database,
    workspace: Workspace,
    model_name: String,
) -> Option<SourceFile> {
    for file in workspace.files(db).iter().copied() {
        if let Some(model) = parse_model(db, file) {
            if model.name == model_name {
                return Some(file);
            }
        }
    }
    None
}

/// Result of resolving a `smelt.<path>` ref against the workspace.
///
/// Phase 2a unifies model / seed / source / function / test resolution
/// behind a single entry point — [`resolve_ref_path`]. Callers dispatch
/// on `kind` to decide what to do; `source_file` is populated for
/// `Model`, `Function`, and `Test` kinds (the entity lives in a
/// `.sql` file tracked by Salsa).
#[derive(Clone)]
pub struct ResolvedRef {
    pub kind: RefKind,
    /// The Salsa-tracked file backing the entity. Populated for
    /// `Model` / `Function` / `Test`. `None` for seeds and sources
    /// (which live outside the SQL file index).
    pub source_file: Option<SourceFile>,
    /// The path tuple used to perform the lookup, for round-tripping
    /// into diagnostics.
    pub path: Vec<String>,
}

impl std::fmt::Debug for ResolvedRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRef")
            .field("kind", &self.kind)
            .field("source_file", &self.source_file.is_some())
            .field("path", &self.path)
            .finish()
    }
}

/// Resolve a path tuple (`["models", "users"]`,
/// `["seeds", "raw", "users"]`, …) against the workspace.
///
/// Per architecture Surface §"Resolution: smelt.<path> is the universal
/// addressing scheme":
/// - `.sql` file with a bare SELECT → `Model`
/// - `.sql` file declaring `smelt.define` → `Function`
/// - `.sql` file with `materialization: test` (Phase 2a stand-in for
///   the future `smelt.test` declaration kind) → `Test`
/// - `.csv` under a project's `paths` → `Seed`
/// - `.yml` declaring an external table → `Source`
///
/// The tuple is matched against each workspace `SourceFile`'s path,
/// falling back to seed/source registries for non-SQL kinds. Kind
/// dispatch is by file format/content, never by directory name.
pub fn resolve_ref_path(
    db: &dyn salsa::Database,
    workspace: Workspace,
    path: Vec<String>,
) -> Option<ResolvedRef> {
    if path.is_empty() {
        return None;
    }

    // Try every project root in the workspace; the first match wins.
    for project in workspace.projects(db).iter().copied() {
        let project_root = project.root(db).clone();
        // Fetch the project's scan-root list once (cached via Salsa) and pass
        // it into `file_path_tuple` for every workspace file. Without this
        // hoist, each iteration of the file loop below would re-parse
        // `smelt.yml` from disk inside `file_path_tuple`, which scaled the
        // resolver to O(workspace_files * config_load_cost) per call.
        let scan_roots = project_paths(db, project);

        // Seeds: match by address_segments (Phase 2 — no "seeds" prefix required).
        // address_segments is the scan-root-stripped path tuple, so
        // `smelt.data.users` matches a seed at `seeds/data/users.csv` under
        // `paths: ["seeds"]` with address_segments = ["data", "users"].
        for seed in project_seeds(db, project).iter() {
            if seed.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Seed,
                    source_file: None,
                    path,
                });
            }
        }

        // Sources: Phase 6 per-entity YAML files. Each source has an
        // `address_segments` tuple (scan-root-stripped path to stem).
        // `smelt.sources.raw.users` → path = ["sources", "raw", "users"]
        // which matches the `.yml` at `models/sources/raw/users.yml`.
        for source in project_sources(db, project).iter() {
            if source.address_segments == path.as_slice() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // Legacy sources: project-level aggregate `sources.yml`. Used as a
        // fallback for any projects not yet migrated to per-entity YAMLs.
        // Kept until Phase 6 migration is complete across all callers.
        if project_sources(db, project).is_empty() && path.len() >= 3 && path[0] == "sources" {
            let source_name = &path[path.len() - 2];
            let table_name = &path[path.len() - 1];
            if resolve_source(db, project, source_name.clone(), table_name.clone()).is_some() {
                return Some(ResolvedRef {
                    kind: RefKind::Source,
                    source_file: None,
                    path,
                });
            }
        }

        // SQL files: walk every workspace file, compute its
        // workspace-relative path tuple, and compare.
        for file in workspace.files(db).iter().copied() {
            let file_path = file.path(db);
            // Accept .sql files, .py files (Python models whose content is
            // generated SQL), and virtual `*.sql::model_name` paths created
            // by multi-model file splitting.
            let path_str = file_path.to_str().unwrap_or("");
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "sql" && ext != "py" && !path_str.contains(".sql::") {
                continue;
            }
            // Match if file_path lives under project_root.
            let file_tuple = match file_path_tuple(&project_root, file_path, file, db, &scan_roots)
            {
                Some(t) => t,
                None => continue,
            };
            if file_tuple == path {
                let kind = sql_file_kind(db, file);
                return Some(ResolvedRef {
                    kind,
                    source_file: Some(file),
                    path,
                });
            }
        }
    }

    None
}

/// Compute the path tuple for a SQL file relative to its project root,
/// stripping the matching `config.paths` scan-root prefix if one applies.
///
/// Algorithm:
/// 1. Strip `project_root` to get `rel`.
/// 2. Try each scan root from `config.paths`: if `rel` starts with the
///    scan root, use the remainder as the parent path.
/// 3. If no scan root matches (e.g. `functions/` with `paths: ["models"]`),
///    fall back to using `rel.parent()` (original behaviour).
/// 4. Build tuple from parent segments + leaf name (model name override as today).
///
/// Returns `None` if the file is not a descendant of the project root.
fn file_path_tuple(
    project_root: &Path,
    file_path: &Path,
    file: SourceFile,
    db: &dyn salsa::Database,
    scan_roots: &[String],
) -> Option<Vec<String>> {
    let rel = file_path.strip_prefix(project_root).ok()?;

    // Try each scan root. Use the first one that `rel` is under.
    let effective_rel = scan_roots
        .iter()
        .find_map(|sr| rel.strip_prefix(sr.as_str()).ok())
        .unwrap_or(rel);

    let parent = effective_rel.parent()?;
    let mut tuple: Vec<String> = parent
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    // Leaf segment: prefer the parsed model name (so multi-model files
    // expose their declared `name:` rather than the filename), falling
    // back to the file stem for non-model SQL files (functions, tests).
    let leaf = parse_model(db, file).map(|m| m.name.clone()).or_else(|| {
        file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })?;
    tuple.push(leaf);
    Some(tuple)
}

/// Determine the kind of a SQL file by its content. Model, function,
/// and test all live in `.sql` files; the dispatch is on
/// content/frontmatter, not filename.
fn sql_file_kind(db: &dyn salsa::Database, file: SourceFile) -> RefKind {
    let raw_text = file.text(db);
    // 1. `smelt.define` → Function. Inspect the parsed AST; this is
    //    cheap because the parse is already cached via Salsa.
    let parse = parse_file(db, file);
    if let Some(ast) = AstFile::cast(parse.syntax()) {
        if ast.defines().next().is_some() {
            return RefKind::Function;
        }
    }
    // 2. `materialization: test` frontmatter (Phase 2a stand-in for the
    //    forthcoming `smelt.test` declaration). Use `extract_file_metadata`
    //    so multi-model files with mixed materializations are handled.
    if let Ok(meta) = smelt_core::extract_file_metadata(raw_text) {
        match meta {
            smelt_core::FileMetadata::Single { metadata, .. } => {
                if metadata.materialization == Some(smelt_core::Materialization::Test) {
                    return RefKind::Test;
                }
            }
            smelt_core::FileMetadata::Multi { models } => {
                if models
                    .iter()
                    .all(|s| s.metadata.materialization == Some(smelt_core::Materialization::Test))
                    && !models.is_empty()
                {
                    return RefKind::Test;
                }
            }
            smelt_core::FileMetadata::Empty => {}
        }
    }
    // 3. Default: Model.
    RefKind::Model
}

#[salsa::tracked]
pub fn resolve_source(
    db: &dyn salsa::Database,
    project: ProjectInput,
    source_name: String,
    table_name: String,
) -> Option<SourceTableDef> {
    let config = sources_config(db, project);
    let source = config.sources.iter().find(|s| s.name == source_name)?;
    source.tables.iter().find(|t| t.name == table_name).cloned()
}

// ============================================================================
// Diagnostics (accumulator-based)
// ============================================================================

/// Top-level file diagnostics. Internally dispatches to the parse/type checkers,
/// which push into `DiagnosticAcc`. Returns the accumulated diagnostics.
pub fn file_diagnostics(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    check_file_diagnostics::accumulated::<DiagnosticAcc>(db, workspace, file)
        .into_iter()
        .map(|d| d.0.clone())
        .collect()
}

#[salsa::tracked]
pub fn check_file_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    let text = file.text(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    // Phase 7: seed CSV without a sibling sidecar YAML emits a workspace
    // warning. We check the file extension first so non-CSV files skip the
    // disk check entirely.
    if path.extension().is_some_and(|e| e == "csv") {
        let sidecar_path = path.with_extension("yml");
        if !sidecar_path.exists() {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "Seed schema is inferred and may drift if the CSV changes — pin it"
                    .to_string(),
                range: Range {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 0, column: 0 },
                },
                code: Some(DiagnosticCode::MissingSeedSidecar),
                data: Some(DiagnosticData::MissingSeedSidecar {
                    csv_path: path.clone(),
                    sidecar_path: sidecar_path.clone(),
                }),
            })
            .accumulate(db);
        }
        // CSV files have no SQL content — skip all SQL-level checks.
        return;
    }

    // Parse errors
    let parse = parse_file(db, file);
    for error in parse.errors.iter() {
        let range = smelt_parser::ast::text_range_to_range(text, error.range);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
            code: Some(DiagnosticCode::ParseError),
            data: None,
        })
        .accumulate(db);
    }

    // Duplicate-function diagnostics (Phase 3): emitted at the second
    // `smelt.define` declaration's name span; workspace-wide check.
    for diag in duplicate_function_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Invalid-type-ref diagnostics (Phase 4): emitted at each malformed
    // `Expr<T>` / unsupported-sort annotation on parameters or return types.
    for diag in invalid_function_type_ref_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Unknown-context diagnostics (Phase 19): emitted when `Expr<T, ctx>`
    // context name doesn't resolve to any parameter in the same function.
    for diag in unknown_context_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // CTE cycle diagnostics (Phase 20): emitted when a function body's WITH
    // clause contains a cyclic CTE reference.
    for diag in cte_cycle_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Context mismatch diagnostics (Phase 20): emitted when an explicit
    // Expr<T, ctx> annotation disagrees with the inferred splice-point context.
    for diag in context_mismatch_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Function body diagnostics (Phase 5): duplicate param names, unknown
    // identifiers inside a body, and body-level type mismatches. Emitted
    // regardless of whether the file contains a SELECT statement — pure
    // function files (functions/*.sql with no model) still surface body
    // diagnostics.
    for diag in function_body_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Call-site expansion diagnostics (Phase 6): unknown/missing/type-
    // mismatched args on `smelt.fn.*` calls, plus any body-cascaded errors
    // re-anchored to the call site. Runs before the `parse_model.is_none()`
    // early-return so call sites in non-model files also surface.
    for diag in smelt_fn_call_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 11 — backends widening / malformed frontmatter.
    for diag in backends_widening_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 43 — frontmatter parse-error / unknown-key diagnostics.
    // Fires unconditionally so workspaces with `unstable_schema: true` still
    // surface malformed YAML and unknown-key warnings on `smelt.define` /
    // `smelt.extern` declarations.
    for diag in frontmatter_parse_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 31 — provenance: unstable-schema gate.
    let unstable_schema = project
        .map(|p| project_unstable_schema(db, p))
        .unwrap_or(false);
    for diag in provenance_unstable_diagnostics_for_file(db, file, unstable_schema) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 51 — provenance/joins validator (only when unstable_schema: true).
    if unstable_schema {
        for diag in provenance_validator::provenance_validator_diagnostics_for_file(db, file) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase 52 — extern fragment-param rejection (fires unconditionally).
    for diag in extern_fragment_param_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 52 — missing-provenance pushdown advisory (Hint severity,
    // only when unstable_schema: true).
    if unstable_schema {
        for diag in missing_provenance_advisory_for_file(db, workspace, file) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase 38 / Phase 42 — smelt.as_struct() backend-capability gate.
    // Functions with explicit `backends:` are checked against that set;
    // functions without (default `BackendSet::All`) are checked against
    // the workspace's active backends from `smelt.yml`.
    let active_backends = project.and_then(|p| project_active_backends(db, p));
    for diag in as_struct_backend_diagnostics_for_file(db, file, active_backends.as_deref()) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase 41 — transparent-function call-graph cycle pre-pass.
    for diag in function_call_cycle_diagnostics_for_file(db, workspace, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // Phase B (meta-language) — smelt.config.var diagnostic wiring.
    //
    // Walk all SMELT_PATH_CALL nodes in the file for `smelt.config.var(...)` calls.
    // Emits: ConfigVarNameNotLiteral, ConfigVarNotFound, ConfigVarNullCoercion.
    // Requires the project-level vars map so this lives in check_file_diagnostics
    // (not check_type_diagnostics) where the project context is available.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        let vars_map = project
            .map(|p| smelt_yml_vars_query(db, p))
            .unwrap_or_default();
        for diag in type_inference::check_config_var_call_diagnostics(&syntax, &vars_map, text) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase B (meta-language) Phase 3: smelt.define name-shadowing.
    //
    // Check each smelt.define declaration for names that shadow built-in
    // HOFs (map, filter, reduce) or reducers (comma_sep, and_all, …).
    // Fires unconditionally so function-only files (functions/*.sql with no
    // SELECT statement) also surface these diagnostics before the early return.
    // Emits HofNameShadowed or ReducerNameShadowed at the name token.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        if let Some(ast) = AstFile::cast(syntax) {
            for define in ast.defines() {
                for diag in type_inference::check_define_name_shadowing(&define, text) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }
    }

    // Check if model is valid
    if parse_model(db, file).is_none() {
        let path_str = path.to_str().unwrap_or("");
        let is_virtual_submodel = path_str.contains("::");
        if !is_virtual_submodel && path_str.contains("models/") {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "File does not contain a valid SQL query".to_string(),
                range: Range {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 0, column: 0 },
                },
                code: Some(DiagnosticCode::InvalidModel),
                data: None,
            })
            .accumulate(db);
        }
        return;
    }

    // Undefined refs (legacy `smelt.models.name` form)
    let refs = model_refs(db, file);
    for ref_loc in refs.iter() {
        if resolve_ref(db, workspace, ref_loc.name.clone()).is_none()
            && !is_known_seed(db, workspace, &ref_loc.name)
        {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined model reference: '{}'", ref_loc.name),
                range: ref_loc.range,
                code: Some(DiagnosticCode::UndefinedModelRef),
                data: Some(DiagnosticData::UndefinedRef {
                    model_name: ref_loc.name.clone(),
                }),
            })
            .accumulate(db);
        }
    }

    // Phase 2a: unified path-form refs. Resolve through the path-tuple
    // resolver and either (a) flag undefined paths or (b) flag a
    // kind-mismatch when a `smelt.tests.*` path appears in a FROM
    // position (architecture Surface §"Resolution").
    let path_refs = model_path_refs(db, file);
    for path_ref_loc in path_refs.iter() {
        match resolve_ref_path(db, workspace, path_ref_loc.path.clone()) {
            Some(resolved) => {
                if resolved.kind == RefKind::Test && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference test '{leaf}' in a FROM position — \
                             smelt.tests.* paths are not valid as TableExpr values"
                        ),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::KindMismatch),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
            None => {
                let path_str = format!("smelt.{}", path_ref_loc.path.join("."));
                // Emit the right diagnostic code based on the path namespace so
                // code-action providers can offer the correct quickfix:
                //   smelt.sources.* → UndefinedSource (offer "Add table to YAML")
                //   smelt.models.*  → UndefinedModelRef (offer "Create model")
                //   anything else   → UndefinedModelRef (generic fallback)
                let is_source_path =
                    path_ref_loc.path.first().map(|s| s.as_str()) == Some("sources");
                if is_source_path && path_ref_loc.path.len() >= 3 {
                    let source_name = path_ref_loc.path[path_ref_loc.path.len() - 2].clone();
                    let table_name = path_ref_loc.path[path_ref_loc.path.len() - 1].clone();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined source: '{}.{}'", source_name, table_name),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedSource),
                        data: Some(DiagnosticData::UndefinedSource {
                            source_name,
                            table_name,
                        }),
                    })
                    .accumulate(db);
                } else {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined ref: {path_str}"),
                        range: path_ref_loc.range,
                        code: Some(DiagnosticCode::UndefinedModelRef),
                        data: Some(DiagnosticData::UndefinedRef {
                            model_name: path_ref_loc.path.last().cloned().unwrap_or_default(),
                        }),
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Undefined sources
    let sources = model_sources(db, file);
    for source_loc in sources.iter() {
        let resolved = if let Some(p) = project {
            resolve_source(
                db,
                p,
                source_loc.source_name.clone(),
                source_loc.table_name.clone(),
            )
        } else {
            None
        };
        if resolved.is_none() {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("Undefined source: '{}'", source_loc.qualified_name),
                range: source_loc.range,
                code: Some(DiagnosticCode::UndefinedSource),
                data: Some(DiagnosticData::UndefinedSource {
                    source_name: source_loc.source_name.clone(),
                    table_name: source_loc.table_name.clone(),
                }),
            })
            .accumulate(db);
        }
    }

    if !sources.is_empty() {
        if let Some(p) = project {
            if let Some(yaml_error) = sources_yaml_error(db, p) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!("sources.yml parse error: {}", yaml_error.message),
                    range: Range {
                        start: Position { line: 0, column: 0 },
                        end: Position { line: 0, column: 0 },
                    },
                    code: Some(DiagnosticCode::YamlParseError),
                    data: None,
                })
                .accumulate(db);
            }

            let type_errors = sources_type_errors(db, p);
            for error in type_errors.iter() {
                let source_qualified = format!("{}.{}", error.source_name, error.table_name);
                if sources.iter().any(|s| s.qualified_name == source_qualified) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                            "Unknown type '{}' for column '{}' in source '{}'. Type information unavailable.",
                            error.invalid_type, error.column_name, source_qualified
                        ),
                        range: Range {
                            start: Position { line: 0, column: 0 },
                            end: Position { line: 0, column: 0 },
                        },
                        code: Some(DiagnosticCode::SourceTypeError),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Unsupported constructs + malformed sources + CAST / unknown fn / ambiguous column
    check_unsupported_constructs(&parse.syntax(), text, db);

    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        // Phase 4: smelt.source() is a parse error so there are no SourceCall
        // nodes to validate. The malformed-source check is superseded by the
        // parser rejection.

        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        check_expression_types(&expr, db);
                    }
                }
            }

            // Phase 14 (§16 #24): reject window-kind expressions in WHERE
            // and GROUP BY positions. Kind synthesis is independent of any
            // column-schema lookups (column refs are always Scalar), so
            // the check runs on a fresh empty `TypeContext`.
            let kind_ctx = type_inference::TypeContext::new();
            for info in type_inference::check_window_in_scalar_contexts(&select_stmt, &kind_ctx) {
                let range = smelt_parser::ast::text_range_to_range(text, info.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Window function `{}` is not allowed in {} (only scalar / aggregate \
                         expressions are permitted here)",
                        info.expression_text, info.clause
                    ),
                    range,
                    code: Some(DiagnosticCode::WindowInScalarContext),
                    data: None,
                })
                .accumulate(db);
            }

            // Phase A (meta-language) Phase 3: list + spread diagnostics.
            //
            // 1. Walk LIST_SPREAD nodes in the SELECT list.
            //    Handles: MetaSpreadOnNonList, MetaListHeterogeneous (for inline
            //    spread-of-literal), MetaListEmptyTypeUnknown.
            //    GroupBy / OrderBy / function args / IN-list / VALUES remain
            //    deferred — the parser DOES emit LIST_SPREAD there, but the
            //    orchestrator does not yet walk those positions.
            //
            // 2. Walk SELECT_ITEM expressions for bare list literals
            //    (`SELECT [1, 'x'] FROM t`).
            //    Handles: MetaListHeterogeneous and MetaListEmptyTypeUnknown
            //    for non-spread list literals appearing directly in the
            //    SELECT list.
            //
            // 3. Detect spreads in forbidden positions (WHERE, etc.).
            //    Handles: MetaSpreadInForbiddenPosition.
            //
            // All three checks use an empty TypeContext (no column schema
            // available at this point) — consistent with the window-function
            // check above.
            let spread_result =
                type_inference::check_select_list_spreads(&select_stmt, &kind_ctx, text);
            for diag in spread_result.diagnostics {
                DiagnosticAcc(diag).accumulate(db);
            }

            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        if let Some(arr) = expr.as_array_literal() {
                            let elements = arr.elements();
                            // Use the expression's span for the diagnostic anchor.
                            let span = expr.syntax().text_range();
                            for diag in type_inference::list_literal_sentinels_to_diagnostics(
                                &elements, &kind_ctx, span, text,
                            ) {
                                DiagnosticAcc(diag).accumulate(db);
                            }
                        }
                    }
                }
            }

            let forbidden_diags =
                type_inference::check_forbidden_position_spreads(&select_stmt, &kind_ctx, text);
            for diag in forbidden_diags {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase B (meta-language) Phase 3: HOF + lambda + pipe diagnostics.
            //
            // Walks every LAMBDA, FUNCTION_CALL (HOF), and PIPE_EXPR descendant.
            // Covers: LambdaInForbiddenPosition, LambdaArityNotSupported,
            //   LambdaResultTypeMismatch, HofExpectsLambda, HofExpectsReducer,
            //   PipeRhsNotCall, PipeInDataPosition, ReducerInputTypeMismatch,
            //   ReducerEmptyNoIdentity.
            // Uses an empty TypeContext (consistent with spread/window checks above).
            let hof_diags =
                type_inference::check_hof_position_diagnostics(&select_stmt, &kind_ctx, text);
            for diag in hof_diags {
                DiagnosticAcc(diag).accumulate(db);
            }

            let from_sources = count_from_sources(&select_stmt);
            if from_sources > 1 {
                if let Some(select_list) = select_stmt.select_list() {
                    for item in select_list.items() {
                        if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                if col_ref.qualifier().is_none() {
                                    let col_name = col_ref.name();
                                    if col_name != "*" {
                                        DiagnosticAcc(Diagnostic {
                                            severity: DiagnosticSeverity::Warning,
                                            message: format!(
                                                "Column '{}' is ambiguous - multiple sources in FROM clause. Consider using a qualified name (e.g., table.{}).",
                                                col_name, col_name
                                            ),
                                            range: Range {
                                                start: Position { line: 0, column: 0 },
                                                end: Position { line: 0, column: 0 },
                                            },
                                            code: Some(DiagnosticCode::AmbiguousColumn),
                                            data: None,
                                        })
                                        .accumulate(db);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Resolve a project root path to a `ProjectInput` via the workspace.
fn find_project(
    db: &dyn salsa::Database,
    workspace: Workspace,
    root: &Path,
) -> Option<ProjectInput> {
    workspace
        .projects(db)
        .iter()
        .copied()
        .find(|p| p.root(db) == root)
}

/// True if `name` matches a discovered seed in any project in the workspace.
/// Seeds aren't part of the file-based `resolve_ref` graph; this helper lets
/// the "Undefined model reference" diagnostic exclude seed names.
fn is_known_seed(db: &dyn salsa::Database, workspace: Workspace, name: &str) -> bool {
    workspace
        .projects(db)
        .iter()
        .copied()
        .any(|p| project_seeds(db, p).iter().any(|s| s.name == name))
}

fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;
    if let Some(from_clause) = select_stmt.from_clause() {
        count += from_clause.table_refs().count();
        count += from_clause.joins().count();
    }
    count
}

/// Check an expression for invalid CAST types and unknown functions; push
/// diagnostics into the accumulator.
fn check_expression_types(expr: &smelt_parser::ast::Expr, db: &dyn salsa::Database) {
    let default_range = Range {
        start: Position { line: 0, column: 0 },
        end: Position { line: 0, column: 0 },
    };

    if let Some(cast_expr) = expr.as_cast() {
        if let Some(type_spec) = cast_expr.type_spec() {
            let type_text = type_spec.full_text();
            if parse_type(&type_text).is_err() {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' in CAST expression. Type inference unavailable.",
                        type_text
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnknownCastType),
                    data: None,
                })
                .accumulate(db);
            }
        }
        if let Some(inner) = cast_expr.expression() {
            check_expression_types(&inner, db);
        }
    }

    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            let upper_name = name.to_uppercase();
            // Phase B (meta-language): exempt HOF names (`map`, `filter`,
            // `reduce`) from the UnrecognizedFunction warning — they are
            // meta-language constructs, not SQL built-ins, and are validated
            // by `check_hof_position_diagnostics` instead.
            let is_hof = type_inference::HofKind::from_name(&name.to_lowercase()).is_some();
            if !is_hof
                && func.namespace().is_none()
                && smelt_types::SqlFunction::from_name(&upper_name).is_none()
            {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Function '{}' is not a recognized SQL function. Type inference unavailable.",
                        name
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnrecognizedFunction),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

fn check_unsupported_constructs(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    text: &str,
    db: &dyn salsa::Database,
) {
    use smelt_parser::SyntaxKind::{PIVOT_CLAUSE, UNPIVOT_CLAUSE};

    for node in syntax.descendants() {
        let (kind_name, node_range) = match node.kind() {
            PIVOT_CLAUSE => ("PIVOT", node.text_range()),
            UNPIVOT_CLAUSE => ("UNPIVOT", node.text_range()),
            _ => continue,
        };
        let range = smelt_parser::ast::text_range_to_range(text, node_range);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "{} is not supported \u{2014} output columns depend on data values and cannot be determined at compile time",
                kind_name
            ),
            range,
            code: Some(DiagnosticCode::UnsupportedConstruct),
            data: None,
        })
        .accumulate(db);
    }
}

// ============================================================================
// Schema queries
// ============================================================================

#[salsa::tracked]
pub fn model_schema(db: &dyn salsa::Database, file: SourceFile) -> Arc<ModelSchema> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(ModelSchema::empty()),
    };

    let select_list = match select_stmt.select_list() {
        Some(l) => l,
        None => return Arc::new(ModelSchema::empty()),
    };

    let from_refs: Vec<String> = if let Some(from_clause) = select_stmt.from_clause() {
        from_clause
            .table_refs()
            .filter_map(|table_ref| {
                // Path-form: smelt.models.foo → leaf segment "foo"
                table_ref
                    .smelt_path_ref()
                    .and_then(|pr| pr.segments().last().cloned())
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut columns = Vec::new();
    let mut row_extensions = Vec::new();

    for item in select_list.items() {
        if item.is_wildcard() {
            for ref_name in &from_refs {
                row_extensions.push(schema::RowExtension {
                    ref_name: ref_name.clone(),
                    excluded_columns: vec![],
                    range: item.range(),
                });
            }
            continue;
        }

        let name = match item.column_name() {
            Some(n) => n,
            None => continue,
        };

        let alias = item.alias();
        let expression = item.expression().map(|e| e.text()).unwrap_or_default();

        let source = if let Some(expr) = item.expression() {
            if expr.as_function_call().is_some() {
                ColumnSource::Computed
            } else if let Some(col_ref) = expr.as_column_ref() {
                let column_name = col_ref.name().to_string();
                if from_refs.len() == 1 {
                    ColumnSource::FromModel {
                        model_name: from_refs[0].clone(),
                        column_name,
                    }
                } else if from_refs.is_empty() {
                    ColumnSource::ExternalTable {
                        table_name: col_ref.qualifier().unwrap_or("unknown").to_string(),
                    }
                } else {
                    ColumnSource::Unknown
                }
            } else {
                ColumnSource::Computed
            }
        } else {
            ColumnSource::Unknown
        };

        columns.push(Column {
            name,
            alias,
            source,
            expression,
            range: item.range(),
            data_type: None,
        });
    }

    if !row_extensions.is_empty() {
        let explicit_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        for ext in &mut row_extensions {
            ext.excluded_columns = explicit_names.clone();
        }
    }

    Arc::new(ModelSchema {
        columns,
        row_extensions,
        input_constraints: vec![],
    })
}

#[salsa::tracked]
pub fn available_columns(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<Column>> {
    let schema = model_schema(db, file);
    let mut available = schema.columns.clone();

    // Walk FROM/JOIN clause SmeltPathRef nodes for smelt.models.* refs and
    // include upstream model columns (used by LSP column completion).
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(from_clause) = select_stmt.from_clause() {
                for table_ref in from_clause.table_refs() {
                    if let Some(path_ref) = table_ref.smelt_path_ref() {
                        let segs = path_ref.segments();
                        if segs.first().map(|s| s.as_str()) == Some("models") {
                            if let Some(model_name) = segs.last().cloned() {
                                if let Some(upstream) = resolve_ref(db, workspace, model_name) {
                                    let upstream_schema = model_schema(db, upstream);
                                    for col in upstream_schema.columns.iter() {
                                        available.push(col.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Arc::new(available)
}

// ============================================================================
// Type checking queries (with cycle recovery via cycle_initial)
// ============================================================================

fn typed_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ModelSchema> {
    Arc::new(ModelSchema::empty())
}

fn type_context_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<TypeContext> {
    Arc::new(TypeContext::new())
}

fn resolved_model_schema_initial(
    _db: &dyn salsa::Database,
    _id: salsa::Id,
    _workspace: Workspace,
    _file: SourceFile,
) -> Arc<ResolvedSchema> {
    Arc::new(ResolvedSchema {
        columns: vec![],
        is_fully_resolved: true,
        unresolved_extensions: vec![],
    })
}

/// Provider for upstream `smelt.ref()` schema lookups, used by the pure
/// `build_type_context` function.
///
/// The Salsa version uses [`SalsaRefSchemaProvider`] (delegates to the new
/// 0.26-API free functions `resolve_ref` + `resolved_model_schema`).
/// The CLI batch compiler uses [`StaticRefSchemaProvider`], which is fully
/// pure and takes pre-computed maps.
pub trait RefSchemaProvider {
    /// Returns the typed columns for the model named `model_name`, if known.
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>>;
    /// Returns the typed columns for the seed named `seed_name`, if known.
    /// Seeds and model refs are looked up separately because the type-context
    /// loop wants to distinguish them (CSV files don't participate in
    /// SELECT * schema resolution, etc.).
    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>>;
    /// Returns the typed columns for the `smelt.functions.<name>(...)` call in
    /// FROM position, by resolving the function's `TableExpr` return schema.
    /// The default implementation returns `None`; `SalsaRefSchemaProvider`
    /// overrides this with a full Salsa-backed resolution.
    fn smelt_path_call_columns(
        &self,
        _call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        None
    }
}

/// `RefSchemaProvider` impl that delegates to the Salsa database. Used by the
/// `type_context()` Salsa query so the LSP keeps benefiting from
/// incremental recomputation.
pub struct SalsaRefSchemaProvider<'a> {
    db: &'a dyn salsa::Database,
    workspace: Workspace,
}

impl<'a> SalsaRefSchemaProvider<'a> {
    pub fn new(db: &'a dyn salsa::Database, workspace: Workspace) -> Self {
        Self { db, workspace }
    }
}

impl SalsaRefSchemaProvider<'_> {
    /// Phase 45: resolve a `TableRef` (FROM/JOIN entry) to the columns it
    /// contributes. Used by [`function_body_check::register_join_alias_schemas`]
    /// from the `infer_tableexpr_return_schema` path so that `<alias>.*`
    /// projections from joined tables expand correctly.
    pub fn resolve_table_ref_schema(
        &self,
        table_ref: &smelt_parser::ast::TableRef,
    ) -> Option<Vec<(String, TypedColumn)>> {
        // smelt.functions.<name>(...) in FROM/JOIN position — resolve schema.
        if let Some(path_call) = table_ref.smelt_path_call() {
            return self.resolve_smelt_path_call_schema(&path_call);
        }

        // smelt.<path> value-form in FROM/JOIN position.
        if let Some(path_ref) = table_ref.smelt_path_ref() {
            let segs = path_ref.segments();
            let model_name = segs.last().cloned().unwrap_or_default();
            let seed_key = segs.join("_");
            return self
                .resolved_columns(&model_name)
                .or_else(|| self.seed_columns(&seed_key));
        }

        None
    }

    /// Resolve a `smelt.functions.<name>(...)` path-call in FROM position
    /// to its inferred output schema.
    ///
    /// Flow:
    ///   1. Resolve the path's tail segment to a workspace `FunctionSig`.
    ///   2. If the signature's return type isn't `TableExpr`, return `None`.
    ///   3. Re-parse the callee's file and find the body `SelectStmt`.
    ///   4. Build a body ctx by resolving each `TableExpr` argument to its
    ///      schema, seeding via `add_tableexpr_param`. Other `Expr<T>` params
    ///      are seeded to `Unknown`.
    ///   5. Call `infer_tableexpr_return_schema` on the body and return cols.
    fn resolve_smelt_path_call_schema(
        &self,
        call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        use smelt_parser::ast::Expr as AstExpr;
        use smelt_types::signatures::SmeltType;

        let segments = call.segments();
        let name = segments.last()?.clone();
        let sig_arc = resolve_function(self.db, self.workspace, name)?;
        let sig: &smelt_types::signatures::FunctionSig = sig_arc.as_ref();

        // Only `TableExpr`-returning functions contribute a FROM schema.
        match &sig.return_type {
            Some(Ok(SmeltType::TableExpr(_))) => {}
            _ => return None,
        }

        // Find the callee's file + body SelectStmt.
        let files: Vec<SourceFile> = self.workspace.files(self.db).to_vec();
        let mut body_select: Option<smelt_parser::ast::SelectStmt> = None;
        for f in &files {
            let sigs = file_signature_inputs(self.db, *f);
            if !sigs.iter().any(|s| s.name == sig.name) {
                continue;
            }
            let f_parse = parse_file(self.db, *f);
            let f_syntax = f_parse.syntax();
            if let Some(ast) = AstFile::cast(f_syntax) {
                for define in ast.defines() {
                    if define.name().as_deref() == Some(&sig.name) {
                        if let Some(body) = define.body() {
                            if let Some(stmt) = body.select_stmt() {
                                body_select = Some(stmt);
                                break;
                            }
                        }
                    }
                }
            }
            if body_select.is_some() {
                break;
            }
        }
        let body_select = body_select?;

        // Bind args to params by position / name.
        let arg_list = call.arg_list();
        let positional: Vec<AstExpr> = arg_list
            .as_ref()
            .map(|al| al.positional_args())
            .unwrap_or_default();
        let named: Vec<smelt_parser::ast::NamedParam> = arg_list
            .as_ref()
            .map(|al| al.named_params().collect())
            .unwrap_or_default();

        let mut bindings: std::collections::HashMap<String, AstExpr> =
            std::collections::HashMap::new();
        for (i, arg) in positional.iter().enumerate() {
            if let Some(p) = sig.params.get(i) {
                bindings.insert(p.name.clone(), arg.clone());
            }
        }
        for np in &named {
            if let (Some(nm), Some(value)) = (np.name(), np.value_expr()) {
                bindings.insert(nm, value);
            }
        }

        // Seed the body ctx with every parameter's caller schema / type.
        let mut body_ctx = TypeContext::new();
        for param in &sig.params {
            if param.name.is_empty() {
                continue;
            }
            if matches!(&param.type_ref, Some(Ok(SmeltType::TableExpr(_)))) {
                // Resolve the TableExpr argument to its column schema.
                if let Some(arg_expr) = bindings.get(&param.name) {
                    // If the argument is itself a smelt.functions.* call, resolve
                    // it recursively to get the inner call's output schema.
                    if let Some(nested) = arg_expr.as_smelt_path_call() {
                        if let Some(cols) = self.resolve_smelt_path_call_schema(&nested) {
                            body_ctx.add_tableexpr_param(&param.name, &cols);
                        }
                        continue;
                    }
                    // Walk the arg expression for a smelt.<path> ref.
                    for node in arg_expr.syntax().descendants() {
                        if node.kind() == smelt_parser::SyntaxKind::SMELT_PATH_REF {
                            if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
                                let segs = path_ref.segments();
                                let model_name = segs.last().cloned().unwrap_or_default();
                                let seed_key = segs.join("_");
                                if let Some(cols) = self
                                    .resolved_columns(&model_name)
                                    .or_else(|| self.seed_columns(&seed_key))
                                {
                                    body_ctx.add_tableexpr_param(&param.name, &cols);
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                // Expr<T> param — bind to Unknown for schema inference.
                let dt = match &param.type_ref {
                    Some(Ok(SmeltType::Expr(
                        smelt_types::signatures::TypeConstraint::Concrete(dt),
                    ))) => dt.clone(),
                    _ => DataType::Unknown,
                };
                body_ctx.add_function_param(&param.name, TypedColumn::nullable(dt));
            }
        }

        // Seed workspace signatures so nested function calls in the
        // body infer their return types.
        let mut wsp_files = files.clone();
        wsp_files.sort_by(|a, b| a.path(self.db).cmp(b.path(self.db)));
        for f in &wsp_files {
            let sigs = file_signature_inputs(self.db, *f);
            for s in sigs.iter() {
                body_ctx.add_function_signature(&s.name, s.clone());
            }
        }

        // Extract CTE schemas from the body's WITH clause so that
        // `infer_tableexpr_return_schema` can resolve bare column references
        // from CTE-derived rows. Cycle diagnostics are discarded — they're
        // surfaced separately by `cte_cycle_diagnostics_for_file`.
        let (body_ctx_with_ctes, _cycle_diags) =
            function_body_check::extract_function_body_cte_schemas(&body_select, &body_ctx, "");
        let mut body_ctx = body_ctx_with_ctes;

        // Seed JOIN-aliased schemas so that `infer_tableexpr_return_schema`
        // can expand `<alias>.*` projections from joined tables.
        let join_lookup =
            |table_ref: &smelt_parser::ast::TableRef| -> Option<Vec<(String, TypedColumn)>> {
                self.resolve_table_ref_schema(table_ref)
            };
        function_body_check::register_join_alias_schemas(
            &mut body_ctx,
            &body_select,
            sig,
            &join_lookup,
        );

        let schema = infer_tableexpr_return_schema(&body_select, &body_ctx)?;
        // Project to (name, TypedColumn) pairs.
        let cols: Vec<(String, TypedColumn)> = schema
            .columns
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    }),
                )
            })
            .collect();
        Some(cols)
    }
}

impl RefSchemaProvider for SalsaRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        let upstream = resolve_ref(self.db, self.workspace, model_name.to_string())?;
        let resolved = resolved_model_schema(self.db, self.workspace, upstream);
        Some(
            resolved
                .columns
                .iter()
                .map(|col| {
                    let typed_col = col.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    });
                    (col.name.clone(), typed_col)
                })
                .collect(),
        )
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        // Seeds aren't part of the file-based resolve_ref graph; walk all
        // projects in the workspace and look in each project's seeds.
        for project in self.workspace.projects(self.db).iter().copied() {
            for seed in project_seeds(self.db, project).iter() {
                if seed.address_segments.join("_") == seed_name {
                    return Some(
                        seed.columns
                            .iter()
                            .map(|(name, dt)| {
                                (
                                    name.clone(),
                                    TypedColumn {
                                        data_type: dt.clone(),
                                        nullable: true,
                                    },
                                )
                            })
                            .collect(),
                    );
                }
            }
        }
        None
    }

    fn smelt_path_call_columns(
        &self,
        call: &smelt_parser::ast::SmeltPathCall,
    ) -> Option<Vec<(String, TypedColumn)>> {
        self.resolve_smelt_path_call_schema(call)
    }
}

/// Fully pure `RefSchemaProvider` for batch compilation (CLI, planner). Holds
/// pre-computed maps of model and seed schemas so it can answer lookups
/// without touching Salsa.
pub struct StaticRefSchemaProvider<'a> {
    pub models: &'a HashMap<String, Vec<(String, TypedColumn)>>,
    pub seeds: &'a HashMap<String, Vec<(String, TypedColumn)>>,
}

impl RefSchemaProvider for StaticRefSchemaProvider<'_> {
    fn resolved_columns(&self, model_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.models.get(model_name).cloned()
    }

    fn seed_columns(&self, seed_name: &str) -> Option<Vec<(String, TypedColumn)>> {
        self.seeds.get(seed_name).cloned()
    }
}

/// Pure (Salsa-free) builder for a `TypeContext` from a parsed AST and the
/// surrounding source/seed/model schemas.
///
/// This is the canonical builder; the `type_context()` Salsa query is a thin
/// wrapper that gathers Salsa inputs (sources_config, parsed file, upstream
/// schemas) and delegates here. The CLI batch compiler uses
/// `StaticRefSchemaProvider` to call this directly without a `Database`.
///
/// See CLAUDE.md "Pure Function Rule" for why this matters.
pub fn build_type_context(
    file: &AstFile,
    sources_config: &SourcesConfig,
    refs: &dyn RefSchemaProvider,
) -> TypeContext {
    let mut ctx = TypeContext::new();

    // Source columns from sources.yml.
    for source in &sources_config.sources {
        for table in &source.tables {
            for col in &table.columns {
                let data_type = col.data_type.clone().unwrap_or(DataType::Unknown);
                ctx.add_source_column(
                    &source.name,
                    &table.name,
                    &col.name,
                    TypedColumn {
                        data_type,
                        nullable: true,
                    },
                );
            }
        }
    }

    if let Some(select_stmt) = file.select_stmt() {
        // Process WITH clause CTEs first (CTEs shadow outer scope).
        if let Some(with_clause) = select_stmt.with_clause() {
            for cte in with_clause.ctes() {
                if let Some(cte_name) = cte.name() {
                    // For recursive CTEs with explicit column list, bootstrap
                    // with Unknown types so the recursive reference can find
                    // the columns.
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

                    if let Some(cte_select) = cte.query().and_then(|q| q.select_stmt()) {
                        process_from_clause_pure(&cte_select, refs, &mut ctx);
                    }

                    let columns = infer_cte_columns(&cte, &ctx);
                    for (col_name, typed_col) in &columns {
                        ctx.add_cte_column(&cte_name, col_name, typed_col.clone());
                    }

                    ctx.add_alias(&cte_name, &cte_name);

                    // If the CTE body is a wildcard SELECT from a
                    // smelt.functions.* call that couldn't be resolved,
                    // mark the CTE opaque so outer column references
                    // don't cascade false-positive UndeclaredColumn
                    // diagnostics.
                    //
                    // Note: we cannot rely on `columns.is_empty()` here
                    // because `infer_cte_columns` always returns at least
                    // one entry for `SELECT *` (a synthetic "col1" with
                    // Unknown type). Instead, check directly whether the
                    // FROM source is a SMELT_PATH_CALL that didn't resolve.
                    {
                        let should_mark_opaque = cte
                            .query()
                            .and_then(|q| q.select_stmt())
                            .map(|s| {
                                let has_wildcard = s
                                    .select_list()
                                    .map(|sl| sl.items().any(|item| item.is_wildcard()))
                                    .unwrap_or(false);
                                if !has_wildcard {
                                    return false;
                                }
                                // The FROM clause has a smelt path call
                                // that couldn't be resolved.
                                s.from_clause()
                                    .and_then(|fc| {
                                        fc.table_refs().find_map(|tr| tr.smelt_path_call())
                                    })
                                    .map(|path_call| {
                                        refs.smelt_path_call_columns(&path_call).is_none()
                                    })
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false);
                        if should_mark_opaque {
                            ctx.mark_cte_opaque(&cte_name);
                        }
                    }
                }
            }
        }

        process_from_clause_pure(&select_stmt, refs, &mut ctx);
    }

    ctx
}

fn process_from_clause_pure(
    select_stmt: &smelt_parser::ast::SelectStmt,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            process_table_ref_pure(&table_ref, refs, ctx);
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                process_table_ref_pure(&table_ref, refs, ctx);
            }
        }
    }
}

fn process_table_ref_pure(
    table_ref: &TableRef,
    refs: &dyn RefSchemaProvider,
    ctx: &mut TypeContext,
) {
    // `smelt.<path>` call-form references (SMELT_PATH_CALL) in the FROM
    // clause. Ask the provider for the inferred TableExpr return schema.
    // If the provider resolves it, seed all columns under the alias binding
    // so the caller's SELECT can validate projections (e.g. UndeclaredColumn
    // for columns not in the function's return schema). If not resolved,
    // fall back to an opaque alias registration.
    if let Some(path_call) = table_ref.smelt_path_call() {
        let segments = path_call.segments();
        let fn_name = segments.last().cloned().unwrap_or_default();
        let bind_to = table_ref.alias().unwrap_or_else(|| fn_name.clone());
        if let Some(cols) = refs.smelt_path_call_columns(&path_call) {
            for (col_name, typed_col) in &cols {
                ctx.add_model_column(&bind_to, col_name, typed_col.clone());
            }
            if !bind_to.is_empty() {
                ctx.add_alias(&bind_to, &bind_to);
            }
        } else if !bind_to.is_empty() {
            // Opaque fallback: alias is registered but no column types.
            ctx.add_alias(&bind_to, &bind_to);
        }
        // Don't fall through to the generic identifier path.
        return;
    }

    // `smelt.<path>` value-form references (SMELT_PATH_REF) in the
    // FROM clause. The path tuple is used to determine the entity name for
    // schema lookup: the full segments joined with "_" are used as the seed
    // key, while the last segment is used for model lookup.
    if let Some(path_ref) = table_ref.smelt_path_ref() {
        let segments = path_ref.segments();
        let model_name = segments.last().cloned().unwrap_or_default();
        let seed_key = segments.join("_");
        // Try seed first, then model.
        if let Some((entity_name, cols)) = refs
            .seed_columns(&seed_key)
            .map(|c| (seed_key.clone(), c))
            .or_else(|| {
                refs.resolved_columns(&model_name)
                    .map(|c| (model_name.clone(), c))
            })
        {
            for (col_name, typed_col) in &cols {
                ctx.add_model_column(&entity_name, col_name, typed_col.clone());
            }
            let bind_to = table_ref.alias().unwrap_or_else(|| entity_name.clone());
            ctx.add_alias(&bind_to, &entity_name);
        } else if segments.first().map(|s| s.as_str()) == Some("sources") {
            // smelt.sources.<source_name>.<table_name> path refs. The source
            // columns were already seeded into `ctx` by `build_type_context`
            // (via `add_source_column`). We just need to register the alias
            // so that `lookup_identifier` can resolve `alias → entity_name`
            // and correctly validate qualified refs like `alias.col_name`
            // as well as bare alias references (e.g. `smelt.functions.f(e)`
            // where `e` is an alias for a source table).
            let bind_to = table_ref.alias().unwrap_or_else(|| model_name.clone());
            ctx.add_alias(&bind_to, &model_name);
        }
        return;
    }

    // CTE references with aliases (e.g. "FROM daily_totals dt")
    // OR bare upstream MODEL/seed references (e.g. "FROM main.stg_orders AS o"
    // — produced by the dialect printer after `smelt.models.stg_orders` is
    // resolved). Without this branch, the alias `o` is never bound and
    // `o.line_revenue` resolves to Unknown, which silently narrows
    // `SUM(o.line_revenue)` to BIGINT in `_smelt_typed`. See B8.
    if table_ref.function_call().is_none() && table_ref.subquery().is_none() {
        if let Some(raw_name) = table_ref.identifier() {
            // Strip an optional leading schema qualifier (`schema.table`).
            // The dialect printer emits `<schema>.<model_name>`; we want the
            // last segment to look up against the schema provider.
            let table_name = bare_table_name(table_ref).unwrap_or(raw_name.clone());

            if ctx.is_cte(&table_name) {
                if let Some(explicit_alias) = table_ref.alias() {
                    ctx.add_alias(&explicit_alias, &table_name);
                }
            } else if let Some(cols) = refs
                .resolved_columns(&table_name)
                .or_else(|| refs.seed_columns(&table_name))
            {
                for (col_name, typed_col) in cols {
                    ctx.add_model_column(&table_name, &col_name, typed_col);
                }
                // Bind the alias (or the table name itself, so qualified
                // refs like `stg_orders.col` also resolve).
                let bind_to = table_ref.alias().unwrap_or_else(|| table_name.clone());
                ctx.add_alias(&bind_to, &table_name);
            }
        }
    }

    // Subqueries / LATERAL subqueries
    if let Some(subquery) = table_ref.subquery() {
        if let Some(alias) = table_ref.alias() {
            if let Some(select_stmt) = subquery.select_stmt() {
                if let Some(select_list) = select_stmt.select_list() {
                    let mut subquery_ctx = ctx.clone();
                    process_from_clause_pure(&select_stmt, refs, &mut subquery_ctx);

                    let column_types = infer_select_column_types(&select_stmt, &subquery_ctx);

                    for (i, item) in select_list.items().enumerate() {
                        let col_name = if let Some(item_alias) = item.alias() {
                            item_alias
                        } else if let Some(expr) = item.expression() {
                            if let Some(col_ref) = expr.as_column_ref() {
                                col_ref.name().to_string()
                            } else {
                                format!("col{}", i + 1)
                            }
                        } else {
                            format!("col{}", i + 1)
                        };

                        let typed_col = column_types.get(i).cloned().unwrap_or(TypedColumn {
                            data_type: DataType::Unknown,
                            nullable: true,
                        });

                        ctx.add_cte_column(&alias, &col_name, typed_col);
                    }

                    ctx.add_alias(&alias, &alias);
                }
            }
        }
    }
}

/// Extract the table-name segment from a bare `TableRef` like `schema.table`,
/// stripping an optional schema qualifier. Used by `process_table_ref_pure`
/// to look up upstream MODEL/seed schemas after the dialect printer has
/// resolved `smelt.models.foo` to `<schema>.foo`.
///
/// Returns `None` for function calls and subqueries (those have their own
/// handling paths).
fn bare_table_name(table_ref: &TableRef) -> Option<String> {
    use smelt_parser::SyntaxKind::{AS_KW, DOT, IDENT};

    if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
        return None;
    }

    // Walk tokens and collect the IDENTs that come BEFORE any AS keyword.
    // The last such IDENT (after any DOT segments) is the table name.
    let mut idents: Vec<String> = Vec::new();
    let mut last_was_dot = false;
    let mut started = false;
    for tok in table_ref
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
    {
        match tok.kind() {
            AS_KW => break,
            IDENT => {
                if !started || last_was_dot {
                    idents.push(tok.text().to_string());
                } else {
                    // Implicit alias (no AS keyword): bail out, take what
                    // we have so far.
                    break;
                }
                started = true;
                last_was_dot = false;
            }
            DOT => {
                last_was_dot = true;
            }
            _ => {}
        }
    }

    idents.last().cloned()
}

/// Pure function: populate a `TypeContext` with column type information from
/// Phase 6 per-entity `SourceInfo` records.
///
/// The source's identity in the TypeContext is `(schema, table)` where:
///   schema = `address_segments[address_segments.len() - 2]` (e.g. "raw")
///   table  = `address_segments[address_segments.len() - 1]` (e.g. "users")
///
/// This mirrors how `smelt.sources.raw.users` is resolved: the last two
/// segments of the path are the schema and table.
pub fn add_source_info_to_type_context(sources: &[SourceInfo], ctx: &mut TypeContext) {
    for source in sources {
        let segs = &source.address_segments;
        if segs.len() < 2 {
            continue; // degenerate address — skip
        }
        let schema_name = &segs[segs.len() - 2];
        let table_name = &segs[segs.len() - 1];
        for col in &source.columns {
            ctx.add_source_column(
                schema_name,
                table_name,
                &col.name,
                TypedColumn {
                    data_type: col.data_type.clone(),
                    nullable: col.nullable,
                },
            );
        }
    }
}

#[salsa::tracked(cycle_initial = type_context_initial)]
pub fn type_context(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<TypeContext> {
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    // Phase 6: per-entity sources take precedence when present; fall back to
    // the legacy aggregate `sources.yml` for projects not yet migrated.
    let per_entity_sources: Arc<Vec<SourceInfo>> = project
        .map(|p| project_sources(db, p))
        .unwrap_or_else(|| Arc::new(Vec::new()));

    let legacy_sources: Arc<SourcesConfig> = if per_entity_sources.is_empty() {
        project
            .map(|p| sources_config(db, p))
            .unwrap_or_else(|| Arc::new(SourcesConfig::default()))
    } else {
        Arc::new(SourcesConfig::default())
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(TypeContext::new()),
    };

    let provider = SalsaRefSchemaProvider::new(db, workspace);
    let mut ctx = build_type_context(&ast, &legacy_sources, &provider);

    // Phase 6: add per-entity source columns to the TypeContext.
    // Source address_segments like ["sources", "raw", "users"] → schema="raw", table="users".
    add_source_info_to_type_context(&per_entity_sources, &mut ctx);

    // Seed the workspace's `smelt.define` signatures so path-call type
    // inference can resolve declared return types when a SELECT projects a
    // `smelt.functions.*` call. Kept pure — we only hand the signature data to
    // the `TypeContext`; analysis logic doesn't call back into Salsa.
    let mut wsp_files: Vec<SourceFile> = workspace.files(db).to_vec();
    wsp_files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in &wsp_files {
        let sigs = file_signature_inputs(db, *f);
        for sig in sigs.iter() {
            ctx.add_function_signature(&sig.name, sig.clone());
        }
    }

    Arc::new(ctx)
}

#[salsa::tracked(cycle_initial = typed_model_schema_initial)]
pub fn typed_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ModelSchema> {
    let base_schema = model_schema(db, file);
    let ctx = type_context(db, workspace, file);
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return base_schema,
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return base_schema,
    };

    let inferred_types = infer_select_column_types(&select_stmt, &ctx);

    let mut typed_columns = Vec::new();
    for (i, col) in base_schema.columns.iter().enumerate() {
        let mut col = col.clone();
        if let Some(typed_col) = inferred_types.get(i) {
            col.data_type = Some(typed_col.clone());
        }
        typed_columns.push(col);
    }

    Arc::new(ModelSchema {
        columns: typed_columns,
        row_extensions: base_schema.row_extensions.clone(),
        input_constraints: base_schema.input_constraints.clone(),
    })
}

#[salsa::tracked(cycle_initial = resolved_model_schema_initial)]
pub fn resolved_model_schema(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<ResolvedSchema> {
    let typed_schema = typed_model_schema(db, workspace, file);

    if typed_schema.row_extensions.is_empty() {
        return Arc::new(ResolvedSchema {
            columns: typed_schema.columns.clone(),
            is_fully_resolved: true,
            unresolved_extensions: vec![],
        });
    }

    let mut columns = Vec::new();
    let mut unresolved_extensions = Vec::new();
    let mut is_fully_resolved = true;

    for ext in &typed_schema.row_extensions {
        if let Some(upstream) = resolve_ref(db, workspace, ext.ref_name.clone()) {
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            for col in &upstream_resolved.columns {
                if !ext.excluded_columns.contains(&col.name) {
                    columns.push(col.clone());
                }
            }
            if !upstream_resolved.is_fully_resolved {
                is_fully_resolved = false;
                for upstream_ext in &upstream_resolved.unresolved_extensions {
                    unresolved_extensions.push(upstream_ext.clone());
                }
            }
        } else {
            is_fully_resolved = false;
            unresolved_extensions.push(ext.clone());
        }
    }

    for col in &typed_schema.columns {
        columns.push(col.clone());
    }

    Arc::new(ResolvedSchema {
        columns,
        is_fully_resolved,
        unresolved_extensions,
    })
}

/// Phase C (meta-language) — Salsa-cached query that materialises the concrete
/// `Vec<ColumnRefValue>` for a given model path at expansion time.
///
/// Takes a `model_name` (the leaf segment of a `smelt.<path>` reference, e.g.
/// `"orders"`) and resolves it via [`resolve_ref`] + [`resolved_model_schema`] to
/// produce a [`smelt_types::ColumnRefValue`] per column, preserving the source
/// schema's declared column order.
///
/// Returns `Ok(columns)` when the schema resolves to a concrete (non-empty)
/// column list, or `Err(())` when:
/// - the model is not found in the workspace (`resolve_ref` returns `None`), or
/// - the resolved schema has no columns and is not fully resolved (upstream
///   `Unknown`).
///
/// The error token is `()` — callers emit `ColumnsOfUnresolvableSchema` and
/// drop the surrounding splice on `Err`.
///
/// This query obeys the `smelt-db` pure-function rule: the analysis (building
/// `ColumnRefValue`s from `Column`s) is pure; the Salsa wrapper only wires up
/// the inputs.
#[salsa::tracked]
pub fn columns_of_for_table_expr(
    db: &dyn salsa::Database,
    workspace: Workspace,
    model_name: String,
) -> Result<Arc<Vec<smelt_types::ColumnRefValue>>, ()> {
    // Resolve the model name to a SourceFile via the existing resolution machinery.
    let file = match resolve_ref(db, workspace, model_name.clone()) {
        Some(f) => f,
        None => return Err(()),
    };

    // Use the typed + row-extended schema so callers see the full column list
    // (including wildcard-expanded columns from upstream models).
    let schema = resolved_model_schema(db, workspace, file);

    // If the schema is unresolvable (no columns and not fully resolved), signal
    // an unknown schema so the caller can emit ColumnsOfUnresolvableSchema.
    if schema.columns.is_empty() && !schema.is_fully_resolved {
        return Err(());
    }

    // Project each Column into a ColumnRefValue.
    let columns: Vec<smelt_types::ColumnRefValue> = columns_to_column_ref_values(&schema.columns);

    Ok(Arc::new(columns))
}

/// Pure helper: convert a slice of [`schema::Column`]s into
/// [`smelt_types::ColumnRefValue`]s in declaration order.
///
/// This function is deliberately separated from the Salsa query body so that
/// the conversion logic is independently testable without a database.
pub fn columns_to_column_ref_values(
    columns: &[schema::Column],
) -> Vec<smelt_types::ColumnRefValue> {
    columns
        .iter()
        .map(|col| {
            let data_type = col.data_type.as_ref().map(|tc| tc.data_type.clone());
            let is_numeric = data_type
                .as_ref()
                .map(|dt| dt.is_numeric())
                .unwrap_or(false);
            smelt_types::ColumnRefValue {
                name: col.name.clone(),
                data_type,
                is_numeric,
                source_span: Some(col.range),
            }
        })
        .collect()
}

#[salsa::tracked]
pub fn model_input_constraints(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<Vec<InputConstraint>> {
    use schema::{ColumnConstraint, InputConstraint};

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ctx = type_context(db, workspace, file);

    let ast = match AstFile::cast(syntax) {
        Some(f) => f,
        None => return Arc::new(vec![]),
    };

    let select_stmt = match ast.select_stmt() {
        Some(s) => s,
        None => return Arc::new(vec![]),
    };

    let mut alias_to_ref: HashMap<String, String> = HashMap::new();
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            // smelt.<path> table references in FROM position.
            if let Some(path_ref) = table_ref.smelt_path_ref() {
                let segs = path_ref.segments();
                if let Some(entity_name) = segs.last().cloned() {
                    if !entity_name.is_empty() {
                        alias_to_ref.insert(entity_name.clone(), entity_name.clone());
                        if let Some(alias) = table_ref.alias() {
                            alias_to_ref.insert(alias, entity_name);
                        }
                    }
                }
            }
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                // smelt.<path> table references in JOIN position.
                if let Some(path_ref) = table_ref.smelt_path_ref() {
                    let segs = path_ref.segments();
                    if let Some(entity_name) = segs.last().cloned() {
                        if !entity_name.is_empty() {
                            alias_to_ref.insert(entity_name.clone(), entity_name.clone());
                            if let Some(alias) = table_ref.alias() {
                                alias_to_ref.insert(alias, entity_name);
                            }
                        }
                    }
                }
            }
        }
    }

    if alias_to_ref.is_empty() {
        return Arc::new(vec![]);
    }

    let mut constraints_map: HashMap<String, HashMap<String, ColumnConstraint>> = HashMap::new();

    let mut record_constraint =
        |ref_name: &str, col_name: &str, expected_type: Option<TypedColumn>, range: TextRange| {
            let entry = constraints_map
                .entry(ref_name.to_string())
                .or_default()
                .entry(col_name.to_string())
                .or_insert_with(|| ColumnConstraint {
                    expected_type: None,
                    usage_sites: vec![],
                });
            if entry.expected_type.is_none() {
                entry.expected_type = expected_type;
            }
            entry.usage_sites.push(range);
        };

    {
        let mut visitor = |qualifier: Option<&str>,
                           col_name: &str,
                           type_hint: Option<&TypedColumn>,
                           range: TextRange| {
            if col_name == "*" {
                return;
            }
            let inferred_type = type_hint.cloned();
            if let Some(q) = qualifier {
                let resolved = ctx.resolve_alias(q).unwrap_or_else(|| q.to_string());
                if let Some(ref_name) = alias_to_ref.get(&resolved) {
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(Some(q), col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            } else {
                let unique_refs: std::collections::HashSet<&String> =
                    alias_to_ref.values().collect();
                if unique_refs.len() == 1 {
                    let ref_name = alias_to_ref
                        .values()
                        .next()
                        .expect("unique_refs.len() == 1 guarantees at least one value");
                    let final_type =
                        inferred_type.or_else(|| ctx.lookup_column(None, col_name).cloned());
                    record_constraint(ref_name, col_name, final_type, range);
                }
            }
        };

        type_inference::walk_select_columns_with_visitor(&select_stmt, &ctx, None, &mut visitor);
    }

    let constraints: Vec<InputConstraint> = constraints_map
        .into_iter()
        .map(|(ref_name, required_columns)| InputConstraint {
            ref_name,
            required_columns,
        })
        .collect();

    Arc::new(constraints)
}

#[salsa::tracked]
pub fn model_function_type(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<schema::ModelFunctionType> {
    use schema::{FunctionInput, FunctionOutput, TypedField};

    let path = file.path(db);
    let model_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let input_constraints = model_input_constraints(db, workspace, file);

    let mut inputs: Vec<FunctionInput> = input_constraints
        .iter()
        .map(|ic| {
            let mut columns: Vec<TypedField> = ic
                .required_columns
                .iter()
                .map(|(col_name, constraint)| TypedField {
                    name: col_name.clone(),
                    constraint: constraint.expected_type.clone(),
                })
                .collect();
            columns.sort_by(|a, b| a.name.cmp(&b.name));
            FunctionInput {
                ref_name: ic.ref_name.clone(),
                columns,
            }
        })
        .collect();

    inputs.sort_by(|a, b| a.ref_name.cmp(&b.ref_name));

    let typed_schema = typed_model_schema(db, workspace, file);

    let outputs: Vec<FunctionOutput> = typed_schema
        .columns
        .iter()
        .filter(|col| col.name != "*")
        .map(|col| FunctionOutput {
            name: col.name.clone(),
            data_type: col.data_type.clone(),
        })
        .collect();

    let has_wildcard_output = !typed_schema.row_extensions.is_empty();

    Arc::new(schema::ModelFunctionType {
        model_name,
        inputs,
        outputs,
        has_wildcard_output,
    })
}

fn types_compatible(expected: &DataType, actual: &DataType) -> bool {
    if matches!(expected, DataType::Unknown) || matches!(actual, DataType::Unknown) {
        return true;
    }
    if expected == actual {
        return true;
    }
    if expected.is_numeric() && actual.is_numeric() {
        return true;
    }
    if expected.is_string() && actual.is_string() {
        return true;
    }
    if expected.is_temporal() && actual.is_temporal() {
        return true;
    }
    if matches!(expected, DataType::Boolean) && matches!(actual, DataType::Boolean) {
        return true;
    }
    if expected.is_string() {
        return true;
    }
    false
}

#[salsa::tracked]
pub fn check_type_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    if !path
        .to_str()
        .map(|s| s.contains("models/"))
        .unwrap_or(false)
    {
        return;
    }

    if parse_model(db, file).is_none() {
        return;
    }

    let refs = model_refs(db, file);
    let sources = model_sources(db, file);
    // Phase 4: also include path-form refs that point to MODELS, SEEDS, or
    // SOURCES (smelt.models.* / smelt.seeds.* / smelt.sources.*) in the
    // "has any upstream dependency" guard. Without this, a model that uses
    // ONLY path-form refs (no legacy smelt.ref()/smelt.source()) would be
    // skipped by the early-return and never get column checks.
    //
    // Phase 4: path refs to models/seeds/sources bypass the early-return so
    // column checks run for models that use only path-form refs (no legacy
    // smelt.ref()/smelt.source()).
    //
    // Note: smelt.functions.* path refs in SELECT position (not FROM) are
    // excluded — they expand inline and don't bring FROM-scope columns.
    // However, smelt.functions.* in FROM/JOIN position (SMELT_PATH_CALL nodes
    // that appear directly in a TableRef) DO provide column scope and must
    // bypass the early-return so that UndeclaredColumn checks run against the
    // inferred return schema of the function call.
    let has_data_path_refs = model_path_refs(db, file).iter().any(|loc| {
        matches!(
            loc.path.first().map(|s| s.as_str()),
            Some("models") | Some("seeds") | Some("sources")
        )
    });
    // Check for smelt.functions.* (SMELT_PATH_CALL) nodes in FROM/JOIN position.
    let has_from_position_path_calls = {
        use smelt_parser::syntax_kind::SyntaxKind as Sk;
        let syntax = parse_file(db, file).syntax();
        syntax.descendants().any(|n| {
            n.kind() == Sk::SMELT_PATH_CALL
                && n.ancestors()
                    .any(|a| matches!(a.kind(), Sk::TABLE_REF | Sk::FROM_CLAUSE | Sk::JOIN_CLAUSE))
        })
    };
    // Files that have ONLY smelt.functions.* calls in FROM (no direct data refs)
    // skip the CannotInferType check since type inference for function return
    // schemas is intentionally limited. They still run check_undeclared_columns.
    let has_direct_data_refs = !refs.is_empty() || !sources.is_empty() || has_data_path_refs;
    if !has_direct_data_refs && !has_from_position_path_calls {
        return;
    }

    let text = file.text(db);

    if has_direct_data_refs {
        let typed_schema = typed_model_schema(db, workspace, file);

        for col in &typed_schema.columns {
            if col.name == "*" {
                continue;
            }

            match &col.data_type {
                Some(typed_col) if matches!(typed_col.data_type, DataType::Unknown) => {
                    let range = smelt_parser::ast::text_range_to_range(text, col.range);
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                        range,
                        code: Some(DiagnosticCode::CannotInferType),
                        data: Some(DiagnosticData::CannotInferType {
                            column_name: col.name.clone(),
                        }),
                    })
                    .accumulate(db);
                }
                None => {
                    let range = smelt_parser::ast::text_range_to_range(text, col.range);
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                        range,
                        code: Some(DiagnosticCode::CannotInferType),
                        data: Some(DiagnosticData::CannotInferType {
                            column_name: col.name.clone(),
                        }),
                    })
                    .accumulate(db);
                }
                _ => {}
            }
        }
    } // end if has_direct_data_refs

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            let ctx = type_context(db, workspace, file);
            let undeclared = type_inference::check_undeclared_columns(&select_stmt, &ctx);
            for info in undeclared {
                let range = smelt_parser::ast::text_range_to_range(text, info.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: info.message,
                    range,
                    code: Some(DiagnosticCode::UndeclaredColumn),
                    data: Some(DiagnosticData::UndeclaredColumn {
                        qualifier: info.qualifier,
                        column_name: info.column_name,
                    }),
                })
                .accumulate(db);
            }
        }
    }

    let input_constraints = model_input_constraints(db, workspace, file);
    for constraint in input_constraints.iter() {
        let upstream = match resolve_ref(db, workspace, constraint.ref_name.clone()) {
            Some(p) => p,
            None => continue,
        };
        let upstream_schema = typed_model_schema(db, workspace, upstream);

        for (col_name, col_constraint) in &constraint.required_columns {
            let expected = match &col_constraint.expected_type {
                Some(t) => t,
                None => continue,
            };
            let upstream_col = upstream_schema.columns.iter().find(|c| c.name == *col_name);
            if let Some(col) = upstream_col {
                if let Some(actual) = &col.data_type {
                    if !types_compatible(&expected.data_type, &actual.data_type) {
                        for site in &col_constraint.usage_sites {
                            let range = smelt_parser::ast::text_range_to_range(text, *site);
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!(
                                    "Column '{}' from '{}' has type {} but is used where {} is expected",
                                    col_name, constraint.ref_name, actual.data_type, expected.data_type
                                ),
                                range,
                                code: Some(DiagnosticCode::TypeMismatch),
                                data: Some(DiagnosticData::TypeMismatch {
                                    column_name: col_name.clone(),
                                    ref_name: constraint.ref_name.clone(),
                                    actual_type: actual.data_type.to_string(),
                                    expected_type: expected.data_type.to_string(),
                                }),
                            })
                            .accumulate(db);
                        }
                    }
                }
            }
        }
    }

    // Detect cycles: base schema has content but resolved is empty OR all
    // types are Unknown → cycle recovery fired.
    //
    // Phase 4: `model_refs` always returns empty (legacy syntax is a parse
    // error). Check path-form refs (smelt.models.*) instead.
    for path_loc in model_path_refs(db, file).iter() {
        // Only model refs can form cycles.
        if path_loc.path.first().map(|s| s.as_str()) != Some("models") {
            continue;
        }
        let model_name = match path_loc.path.last() {
            Some(n) => n.clone(),
            None => continue,
        };
        if let Some(upstream) = resolve_ref(db, workspace, model_name.clone()) {
            let upstream_base = model_schema(db, upstream);
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            let all_unknown = !upstream_resolved.columns.is_empty()
                && upstream_resolved.columns.iter().all(|c| {
                    c.data_type
                        .as_ref()
                        .map(|t| matches!(t.data_type, DataType::Unknown))
                        .unwrap_or(true)
                });
            let has_any_column =
                !upstream_base.columns.is_empty() || !upstream_base.row_extensions.is_empty();
            if has_any_column && (upstream_resolved.columns.is_empty() || all_unknown) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Circular dependency involving model '{}' — type information unavailable",
                        model_name
                    ),
                    range: path_loc.range,
                    code: Some(DiagnosticCode::CircularDependency),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

// ============================================================================
// Phase 30 — Logical plan construction
// ============================================================================

/// Pre-gathered inputs for one `smelt.fn.*` call site, collected by the Salsa
/// query before passing to the pure plan builder.
struct FnCallInput {
    fn_id: String,
    transparent: bool,
    properties: smelt_planner::logical::FunctionProperties,
    /// Resolved provenance: either the declared provenance (when the workspace
    /// opted in to `unstable_schema`) or `Unknown`.
    provenance: smelt_planner::logical::Provenance,
    /// Phase 41: the callee's body text, captured eagerly by the Salsa query.
    /// `None` for opaque calls, unresolved references, and calls suppressed by
    /// the cycle pre-pass.  When `Some`, the body is attached to the
    /// `FunctionCall` plan node as a `LogicalNode::Raw { sql_text }` subtree;
    /// the Phase 41 expansion rule clones it into the resulting `ExpandedCall`.
    body_text: Option<String>,
}

/// Build a [`smelt_planner::logical::Plan`] from a single source file.
///
/// This tracked query gathers all Salsa inputs — the parsed AST, resolved
/// signatures, and per-declaration frontmatter — then delegates to the pure
/// helper [`build_logical_plan_pure`] which takes no `db` reference.
///
/// Returns `None` when the file does not parse as a valid SQL model.
#[salsa::tracked]
pub fn logical_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_planner::logical::Plan> {
    use smelt_planner::logical::Provenance;

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;

    // Determine whether the workspace has opted in to unstable schema features.
    // Uses the Salsa-tracked ProjectInput so changes to smelt.yml invalidate
    // this query via Salsa's dependency graph (no raw filesystem I/O here).
    let project_root = file.project_root(db).clone();
    let unstable_schema = find_project(db, workspace, &project_root)
        .map(|p| project_unstable_schema(db, p))
        .unwrap_or(false);

    // Phase 41: workspace-wide body capture + cycle pre-pass.  The body map
    // lets the call-site loop attach `LogicalNode::Raw` subtrees without
    // re-walking the workspace per call; the cycle set tells us which
    // transparent calls must skip body attachment so the planner does not
    // attempt to inline a non-terminating expansion.
    let bodies = workspace_function_bodies(db, workspace);
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    // Walk the CST to collect all smelt.functions.* (path-form) call sites.
    let call_inputs: Vec<FnCallInput> = ast
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
        .map(|call| {
            let segments = call.segments();
            let fn_id = segments.last().cloned().unwrap_or_default();

            let sig_opt = if fn_id.is_empty() {
                None
            } else {
                resolve_function(db, workspace, fn_id.clone()).map(|arc| (*arc).clone())
            };

            let transparent = sig_opt
                .as_ref()
                .map(|sig| sig.origin == smelt_types::SigOrigin::Define)
                .unwrap_or(false);

            // Locate the declaring file and read its frontmatter via Salsa.
            let mut properties = sig_opt
                .as_ref()
                .and_then(|_| {
                    workspace
                        .files(db)
                        .iter()
                        .copied()
                        .find(|f| {
                            file_signature_inputs(db, *f)
                                .iter()
                                .any(|s| s.name == fn_id)
                        })
                        .and_then(|decl_file| {
                            let decl_parse = parse_file(db, decl_file);
                            let decl_syntax = decl_parse.syntax();
                            let decl_ast = AstFile::cast(decl_syntax)?;
                            let decl_raw = decl_file.text(db).clone();
                            let fm = decl_ast
                                .defines()
                                .find(|d| d.name().as_deref() == Some(fn_id.as_str()))
                                .and_then(|d| d.frontmatter(&decl_raw))
                                .or_else(|| {
                                    decl_ast
                                        .externs()
                                        .find(|e| e.name().as_deref() == Some(fn_id.as_str()))
                                        .and_then(|e| e.frontmatter(&decl_raw))
                                });
                            // Phase 43: ignore the frontmatter diagnostics here — they are
                            // surfaced via `provenance_unstable_diagnostics_for_file` (called
                            // from `check_file_diagnostics`), which has access to the
                            // declaration's name range for proper anchoring. The logical-plan
                            // path only needs the parsed properties.
                            fm.map(|text| {
                                smelt_planner::logical::parse_function_properties(&text).0
                            })
                        })
                })
                .unwrap_or_default();

            // Phase 31: enforce unstable_schema gate on `provenance:`.
            // If the function declared provenance but the workspace flag is
            // absent, silently return Unknown here. The diagnostic is emitted
            // by `provenance_unstable_diagnostics_for_file`, which is called
            // from `check_file_diagnostics` so it surfaces through
            // `file_diagnostics`.
            let resolved_provenance =
                if matches!(properties.provenance, Provenance::Declared(_)) && !unstable_schema {
                    Provenance::Unknown
                } else {
                    // Either the flag is set (use declared provenance) or
                    // provenance is already Unknown (pass through).
                    std::mem::replace(&mut properties.provenance, Provenance::Unknown)
                };

            // Phase 41: attach body text for transparent calls whose declaring
            // function is not in a cycle.  Opaque (`smelt.extern`) calls and
            // cycle participants leave `body_text: None` so the expansion
            // rule falls back to the marker-only behaviour from Phase 32.
            let body_text = if transparent && !cycle_set.contains(&fn_id) {
                bodies.get(&fn_id).cloned()
            } else {
                None
            };

            FnCallInput {
                fn_id,
                transparent,
                properties,
                provenance: resolved_provenance,
                body_text,
            }
        })
        .collect();

    Some(build_logical_plan_pure(call_inputs))
}

/// Pure plan builder — takes no `db` reference and calls no Salsa queries.
///
/// Constructs a minimal `Select` root with the first collected `FunctionCall`
/// as its `from` child. Phase 32+ replaces this with a full projection tree.
fn build_logical_plan_pure(call_inputs: Vec<FnCallInput>) -> smelt_planner::logical::Plan {
    use smelt_planner::logical::LogicalNode;
    use std::sync::Arc;

    let fn_call_nodes: Vec<Arc<LogicalNode>> = call_inputs
        .into_iter()
        .map(|input| {
            let body = input
                .body_text
                .map(|t| Arc::new(LogicalNode::Raw { sql_text: t }));
            Arc::new(LogicalNode::FunctionCall {
                fn_id: input.fn_id,
                args: Vec::new(), // Phase 30 stub — arg sub-plans deferred to Phase 32+
                transparent: input.transparent,
                provenance: input.provenance,
                properties: input.properties,
                pushed_filter: None,
                body,
            })
        })
        .collect();

    if fn_call_nodes.is_empty() {
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: None,
            filter: None,
        })
    } else {
        let first = fn_call_nodes.into_iter().next().unwrap();
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: Some(first),
            filter: None,
        })
    }
}

/// Phase 41: workspace-wide map from `fn_id` → body text for every
/// `smelt.define`. Opaque externs are not included (they have no body).
///
/// Salsa-tracked so the cycle pre-pass and body-attachment paths share one
/// cache entry per workspace.  The return is wrapped in `Arc` to satisfy
/// Salsa's interning / equality requirements (the same shape used by
/// `all_models`).
#[salsa::tracked]
fn workspace_function_bodies(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let body_text = body.syntax().text().to_string();
            // First wins on duplicates — duplicate-define diagnostics catch
            // the second occurrence elsewhere.
            out.entry(name).or_insert(body_text);
        }
    }
    Arc::new(out)
}

/// Workspace-wide call graph for `smelt.define` declarations.
///
/// Returns a map from caller `fn_id` → callees (set of `fn_id`s reached from
/// the body's `smelt.functions.*` call sites). Externs and unresolved references
/// are dropped — they are sinks in the graph.  Salsa-tracked so each
/// workspace pays the walk once per parse-graph epoch.
#[salsa::tracked]
fn workspace_function_call_graph(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, Vec<String>>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(caller) = define.name() else {
                continue;
            };
            let Some(body) = define.body() else { continue };
            let mut callees: Vec<String> = body
                .syntax()
                .descendants()
                .filter_map(smelt_parser::ast::SmeltPathCall::cast)
                .filter_map(|c| c.segments().last().cloned())
                .filter(|s| !s.is_empty())
                .collect();
            callees.sort();
            callees.dedup();
            out.entry(caller).or_insert(callees);
        }
    }
    Arc::new(out)
}

/// Phase 41 — pure DFS cycle detector over the workspace call graph.
/// Returns the set of `fn_id`s that participate in any cycle.
pub fn find_function_call_cycles(
    graph: &std::collections::HashMap<String, Vec<String>>,
) -> std::collections::HashSet<String> {
    use std::collections::{HashMap, HashSet};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Grey,
        Black,
    }
    let mut color: HashMap<&str, Color> = HashMap::new();
    let mut in_cycle: HashSet<String> = HashSet::new();

    for node in graph.keys() {
        color.insert(node.as_str(), Color::White);
    }

    fn dfs<'a>(
        node: &'a str,
        graph: &'a std::collections::HashMap<String, Vec<String>>,
        color: &mut HashMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
        in_cycle: &mut HashSet<String>,
    ) {
        color.insert(node, Color::Grey);
        stack.push(node);
        if let Some(callees) = graph.get(node) {
            for callee in callees {
                let key = callee.as_str();
                match color.get(key).copied().unwrap_or(Color::White) {
                    Color::White => {
                        if graph.contains_key(callee) {
                            dfs(key, graph, color, stack, in_cycle);
                        } else {
                            // sink: not in graph
                            color.insert(key, Color::Black);
                        }
                    }
                    Color::Grey => {
                        // Found a back-edge — every Grey node from `key` to
                        // the top of `stack` is on the cycle.
                        let mut on_cycle = false;
                        for &s in stack.iter() {
                            if s == key {
                                on_cycle = true;
                            }
                            if on_cycle {
                                in_cycle.insert(s.to_string());
                            }
                        }
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
    }

    let nodes: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    for node in nodes {
        if matches!(
            color.get(node).copied().unwrap_or(Color::White),
            Color::White
        ) {
            let mut stack: Vec<&str> = Vec::new();
            dfs(node, graph, &mut color, &mut stack, &mut in_cycle);
        }
    }

    in_cycle
}

/// Cached union of cycle-participant `fn_id`s for the current workspace.
#[salsa::tracked]
fn function_call_cycle_fn_ids(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashSet<String>> {
    let graph = workspace_function_call_graph(db, workspace);
    Arc::new(find_function_call_cycles(graph.as_ref()))
}

/// Phase 41 — emit [`DiagnosticCode::FunctionCallCycle`] for every
/// `smelt.define` in `file` whose `fn_id` is reachable inside a cycle in the
/// workspace call graph. Anchored at the declaration's name range.
pub fn function_call_cycle_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let cycle_set = function_call_cycle_fn_ids(db, workspace);
    if cycle_set.is_empty() {
        return Vec::new();
    }

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else { continue };
        if !cycle_set.contains(&name) {
            continue;
        }
        let range = sigs
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.name_range)
            .unwrap_or(Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 0 },
            });
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "function `{name}` participates in a cyclic call graph; \
                 transparent expansion is suppressed for this function and \
                 every other function on the cycle"
            ),
            range,
            code: Some(DiagnosticCode::FunctionCallCycle),
            data: None,
        });
    }
    out
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod test_harness;

#[cfg(test)]
mod tests;
