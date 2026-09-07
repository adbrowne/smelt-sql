//! The LSP backend: `Backend` struct and `LanguageServer` trait impl.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use smelt_core::{
    find_config_file, find_project_root_by_walking_up, find_project_root_for_file,
    find_smelt_projects, is_sources_file,
    metadata::{extract_file_metadata, FileMetadata},
};
use smelt_db::{
    functions_in_file, project_address_collisions, project_emitted_name_collisions,
    project_source_diagnostics, Database, Diagnostic as DbDiagnostic, DiagnosticCode as DbCode,
    DiagnosticData as DbData, DiagnosticSeverity as DbSeverity, ProjectInput, SourceFile,
    Workspace,
};
use smelt_parser::ast::File as AstFile;
use smelt_parser::is_valid_sql_identifier;
use smelt_parser::symbol::{position_to_offset, symbol_at_cursor, SymbolAtCursor};
use smelt_types::{format_smelt_type_hover, TypedColumn};

use crate::column_resolution::{
    build_python_context, collect_from_model_names, find_definition_model_name, format_type,
    resolve_column_definitions, trace_upstream_column, ColumnDefLocation,
};
use crate::completion::{
    determine_completion_context, extract_from_aliases, AliasTarget, CompletionContext,
};
use crate::db_helpers::{
    all_file_paths, diagnostics_for, file_project_root, file_text, lookup_file, lookup_project,
    project_sources_yaml,
};
use crate::hover::{
    column_ref_field_completions,
    columns_of_arg_completions_for_sql,
    // Phase E2 goto-def + completion helpers
    completion_for_generates_value,
    completion_for_model_def_field_key,
    completion_item_for_if_snippet,
    completion_items_for_reduce_second_arg_with_snippets,
    find_smelt_fn_call_at_cursor,
    find_var_line_in_smelt_yml,
    goto_def_for_emitted_model_reference,
    hover_text_for_column_ref_field,
    hover_text_for_column_reference,
    hover_text_for_columns_of_call,
    hover_text_for_generates_frontmatter,
    hover_text_for_hof_meta_language,
    hover_text_for_list_literal_dual,
    hover_text_for_list_spread,
    hover_text_for_model_def_body_field_value,
    hover_text_for_model_def_literal_open_brace,
    hover_text_for_model_def_name_field_value,
    hover_text_for_model_def_optional_field_value,
    hover_text_for_model_ref_field,
    hover_text_for_models_all,
    hover_text_for_models_with_tag_call,
    hover_text_for_pipe_expr,
    hover_text_for_source_clamp,
    hover_text_for_source_ref_field,
    hover_text_for_sources_all,
    hover_text_for_sources_with_tag_call,
    is_column_ref_param_before_dot,
    is_model_ref_param_before_dot,
    is_source_ref_param_before_dot,
    lambda_param_binder_range,
    lambda_params_for_completion,
    model_ref_field_completions,
    passing_body_aggregate_labels,
    passing_body_completion_columns,
    render_expansion_frames,
    source_ref_field_completions,
    wide_reflection_accessor_completions,
};
use crate::python_scan::PythonModelCache;

mod completion_impl;
mod diagnostics_impl;
mod hover_impl;
mod navigation_impl;
mod rename_impl;

/// Tracks errors that occurred during workspace initialization
#[derive(Default)]
pub(crate) struct InitErrors {
    pub(crate) workspace_errors: Vec<String>,
    pub(crate) source_errors: Vec<String>,
    pub(crate) model_errors: Vec<String>,
}

impl InitErrors {
    pub(crate) fn has_errors(&self) -> bool {
        !self.workspace_errors.is_empty()
            || !self.source_errors.is_empty()
            || !self.model_errors.is_empty()
    }

    pub(crate) fn total_count(&self) -> usize {
        self.workspace_errors.len() + self.source_errors.len() + self.model_errors.len()
    }
}

/// `(virtual_path, sql_start_line, delimiter_line)` for each section in a
/// multi-model file.  `sql_start_line` is the offset of the SQL body (after
/// the closing `---`) used for diagnostic line adjustment.  `delimiter_line`
/// is the 0-based line of the `--- name: foo ---` header, used by the VSCode
/// TestController so the gutter icon lands on the declaration line.
pub(crate) type MultiModelEntry = Vec<(PathBuf, u32, u32)>;

pub struct Backend {
    client: Client,
    /// The salsa database. `Database: Clone` with internally-Arc'd storage, so
    /// reads snapshot via `self.snapshot()` (lock briefly, clone, drop lock).
    /// Writes lock the mutex for the duration of the input mutation.
    db: Arc<Mutex<Database>>,
    /// Tracked set of file paths registered in the DB (mirror of the
    /// Workspace singleton's `files`). Kept in sync whenever inputs change.
    tracked_files: Arc<Mutex<Vec<PathBuf>>>,
    /// Errors collected during initialization, reported after `initialized` notification
    init_errors: Arc<Mutex<Option<InitErrors>>>,
    /// Maps virtual .sql paths (used in Salsa) back to actual .py source paths + decorator line
    /// for goto-definition. The u32 is the 0-indexed line of the `@model` decorator.
    python_model_sources: Arc<Mutex<HashMap<PathBuf, (PathBuf, u32)>>>,
    /// Cache of Python model results (keyed by content hash)
    python_cache: Arc<Mutex<PythonModelCache>>,
    /// Diagnostics for Python files (separate from Salsa-managed SQL diagnostics)
    python_diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<lsp_types::Diagnostic>>>>,
    /// Project roots discovered during init (needed for file-change handling)
    project_roots: Arc<Mutex<Vec<PathBuf>>>,
    /// Maps real file paths to their virtual sub-paths for multi-model files.
    /// Each entry is (virtual_path, start_line_offset) where virtual_path uses
    /// the `file.sql::model_name` convention and start_line_offset is the
    /// 0-based line in the original file where the section's SQL begins.
    multi_model_files: Arc<Mutex<HashMap<PathBuf, MultiModelEntry>>>,
    /// The position encoding negotiated with the client during `initialize`.
    /// Defaults to UTF-16 (LSP default when the client advertises no preference).
    negotiated_encoding: Arc<Mutex<PositionEncodingKind>>,
    /// Test models discovered during workspace init and file-change scans.
    /// Populated by `publish_tests()`; read by the VSCode TestController via
    /// the `smelt/publishTests` notification.
    known_tests: Arc<Mutex<Vec<crate::notifications::TestInfo>>>,
    /// Property-diff editor state, one entry per project root
    /// (`docs/specs/property_diff.md` §Surface "Editor"; project isolation
    /// — each project resolves its own baseline). Populated by
    /// `refresh_property_diff`, which runs off the request path in
    /// `spawn_blocking` (`docs/outcomes/20260905-property-diff/phases/
    /// 07-plan.md` D7/R6) and is read-only from `code_lens` and
    /// `publish_diagnostics`.
    property_diff: Arc<Mutex<HashMap<PathBuf, crate::property_diff::ProjectDiffState>>>,
    /// Counts every time `refresh_property_diff` actually runs the
    /// pipeline (`crate::property_diff::refresh`) — not every call to
    /// `refresh_property_diff` itself, most of which coalesce. Exposed via
    /// `Backend::property_diff_derivation_count` so a test can assert a
    /// burst of `didChangeWatchedFiles` events collapses to one derivation
    /// per project root (`docs/outcomes/20260905-property-diff/phases/
    /// 07-plan.md` risk R3).
    property_diff_derivation_count: Arc<std::sync::atomic::AtomicUsize>,
}

/// Collect every `smelt.functions.<name>(...)` call-site path range across
/// the given files. Used by the references handler for both call-site and
/// declaration-site cursors. `files` is expected to already be project-scoped.
fn collect_function_call_sites(
    db: &smelt_db::Database,
    files: &[smelt_db::SourceFile],
    name: &str,
) -> Vec<(PathBuf, rowan::TextRange)> {
    let mut out = Vec::new();
    for f in files {
        let parse = smelt_db::parse_file(db, *f);
        let Some(ast) = AstFile::cast(parse.syntax()) else {
            continue;
        };
        for trange in smelt_db::references::find_function_call_sites_in_file(&ast, name) {
            out.push((f.path(db).clone(), trange));
        }
    }
    out
}

/// Derive file-system watcher patterns for a set of project roots.
///
/// Returns two `FileSystemWatcher` entries per project root:
/// - `<root>/**/*.sql` — covers every discoverable `.sql` (any non-excluded
///   `.sql` is a model or function per universal discovery D-01/D-05)
/// - `<root>/**/*.py` — covers Python model files
///
/// LSP glob patterns have no exclusion syntax, so the hidden-dir and `target/`
/// skip-list is enforced at the handler level: files in those paths are not
/// registered in the Salsa DB and are ignored when `did_change_watched_files`
/// fires for them.
pub(crate) fn derive_watch_globs(project_roots: &[PathBuf]) -> Vec<FileSystemWatcher> {
    let mut watchers = Vec::new();
    for root in project_roots {
        let root_str = root.to_string_lossy();
        watchers.push(FileSystemWatcher {
            glob_pattern: GlobPattern::String(format!("{root_str}/**/*.sql")),
            kind: Some(WatchKind::all()),
        });
        watchers.push(FileSystemWatcher {
            glob_pattern: GlobPattern::String(format!("{root_str}/**/*.py")),
            kind: Some(WatchKind::all()),
        });
    }
    watchers
}

/// Derive `.git` watcher patterns for the property diff's baseline refresh
/// trigger (`docs/specs/property_diff.md` §Surface "Editor";
/// `docs/outcomes/20260905-property-diff/phases/07-plan.md` D2). `repo_roots`
/// is deduplicated by the caller — several projects in one workspace may
/// share a repo root. This is a TRIGGER, not the correctness mechanism:
/// `refresh_property_diff` always re-resolves and compares the commit, so a
/// client that never reports `.git` changes only loses promptness, not
/// correctness.
pub(crate) fn derive_git_watch_globs(repo_roots: &[PathBuf]) -> Vec<FileSystemWatcher> {
    let mut watchers = Vec::new();
    for root in repo_roots {
        let root_str = root.to_string_lossy();
        for suffix in [".git/HEAD", ".git/refs/**", ".git/packed-refs"] {
            watchers.push(FileSystemWatcher {
                glob_pattern: GlobPattern::String(format!("{root_str}/{suffix}")),
                kind: Some(WatchKind::all()),
            });
        }
    }
    watchers
}

/// Map a `smelt-db` diagnostic code to its stable, wire-visible LSP
/// code string (kebab-case). Extracted as a pure function so the
/// mapping is directly unit-testable without constructing a `Backend`.
pub(crate) fn diagnostic_code_str(code: DbCode) -> &'static str {
    match code {
        DbCode::ParseError => "parse-error",
        DbCode::TrailingTopLevelContent => "trailing-top-level-content",
        DbCode::InvalidModel => "invalid-model",
        DbCode::UndefinedModelRef => "undefined-model-ref",
        DbCode::UndefinedSource => "undefined-source",
        DbCode::CannotInferType => "cannot-infer-type",
        DbCode::ColumnTypeUnresolved => "column-type-unresolved",
        DbCode::UndeclaredColumn => "undeclared-column",
        DbCode::TypeMismatch => "type-mismatch",
        DbCode::CircularDependency => "circular-dependency",
        DbCode::UnsupportedConstruct => "unsupported-construct",
        DbCode::YamlParseError => "yaml-parse-error",
        DbCode::SourceTypeError => "source-type-error",
        DbCode::MalformedSource => "malformed-source",
        DbCode::AmbiguousColumn => "ambiguous-column",
        DbCode::UnknownCastType => "unknown-cast-type",
        DbCode::UnrecognizedFunction => "unrecognized-function",
        DbCode::DuplicateFunctionDefinition => "duplicate-function-definition",
        DbCode::InvalidFunctionTypeRef => "invalid-function-type-ref",
        DbCode::FunctionBodyTypeMismatch => "function-body-type-mismatch",
        DbCode::UnknownIdentifier => "unknown-identifier",
        DbCode::DuplicateParameterName => "duplicate-parameter-name",
        DbCode::UnknownSmeltFn => "unknown-smelt-fn",
        DbCode::MissingArgument => "missing-argument",
        DbCode::ArgTypeMismatch => "arg-type-mismatch",
        DbCode::ExternCollidesWithBuiltin => "extern-collides-with-builtin",
        DbCode::BackendsWideningNotAllowed => "backends-widening-not-allowed",
        DbCode::WindowInScalarContext => "window-in-scalar-context",
        DbCode::ParameterShadowsColumn => "parameter-shadows-column",
        DbCode::RowRequirementUnsatisfied => "row-requirement-unsatisfied",
        DbCode::UnknownContext => "unknown-context",
        DbCode::CteCycle => "cte-cycle",
        DbCode::CteShadowsCallerCte => "cte-shadows-caller-cte",
        DbCode::ContextMismatch => "context-mismatch",
        DbCode::FragmentColumnMissing => "fragment-column-missing",
        DbCode::AnnotationTooWide => "annotation-too-wide",
        DbCode::FragmentKindMismatch => "fragment-kind-mismatch",
        DbCode::ReturnTypeMismatch => "return-type-mismatch",
        DbCode::UnknownPassingParameter => "unknown-passing-parameter",
        DbCode::UnstableSchemaRequired => "unstable-schema-required",
        DbCode::AsStructUnsupportedBackend => "as-struct-unsupported-backend",
        DbCode::FunctionCallCycle => "function-call-cycle",
        DbCode::FrontmatterParseError => "frontmatter-parse-error",
        DbCode::PythonModelNameMismatch => "python-model-name-mismatch",
        DbCode::ProvenanceMismatch => "provenance-mismatch",
        DbCode::JoinsMismatch => "joins-mismatch",
        DbCode::DeclaredCardinalityUnverifiable => "declared-cardinality-unverifiable",
        DbCode::MissingProvenancePushdownAdvisory => "missing-provenance-pushdown-advisory",
        DbCode::ExternFragmentParamUnsupported => "extern-fragment-param-unsupported",
        DbCode::KindMismatch => "kind-mismatch",
        DbCode::MissingSeedSidecar => "missing-seed-sidecar",
        // Phase A (meta-language) diagnostic codes.
        DbCode::MetaListEmptyTypeUnknown => "meta-list-empty-type-unknown",
        DbCode::MetaListHeterogeneous => "meta-list-heterogeneous",
        DbCode::MetaSpreadInForbiddenPosition => "meta-spread-in-forbidden-position",
        DbCode::MetaSpreadOnNonList => "meta-spread-on-non-list",
        DbCode::MetaListInScalarPosition => "meta-list-in-scalar-position",
        // Phase B (meta-language) diagnostic codes.
        DbCode::LambdaInForbiddenPosition => "lambda-in-forbidden-position",
        DbCode::LambdaArityMismatch => "lambda-arity-mismatch",
        DbCode::LambdaZeroParameters => "lambda-zero-parameters",
        DbCode::LambdaDuplicateParameter => "lambda-duplicate-parameter",
        DbCode::LambdaResultTypeMismatch => "lambda-result-type-mismatch",
        DbCode::HofExpectsLambda => "hof-expects-lambda",
        DbCode::HofExpectsReducer => "hof-expects-reducer",
        DbCode::HofNameShadowed => "hof-name-shadowed",
        DbCode::ReducerNameShadowed => "reducer-name-shadowed",
        DbCode::PipeRhsNotCall => "pipe-rhs-not-call",
        DbCode::PipeInDataPosition => "pipe-in-data-position",
        DbCode::ReducerInputTypeMismatch => "reducer-input-type-mismatch",
        DbCode::ReducerEmptyNoIdentity => "reducer-empty-no-identity",
        // Phase F (meta-language) parameterised reducer + ternary codes.
        DbCode::ReducerArityMismatch => "reducer-arity-mismatch",
        DbCode::ReducerArgTypeMismatch => "reducer-arg-type-mismatch",
        DbCode::ReducerArgNotCompileTime => "reducer-arg-not-compile-time",
        DbCode::ReducerNamedArgument => "reducer-named-argument",
        DbCode::HofNamedArgument => "hof-named-argument",
        DbCode::TernaryConditionNotBoolean => "ternary-condition-not-boolean",
        DbCode::TernaryBranchTypeMismatch => "ternary-branch-type-mismatch",
        DbCode::TernaryKeywordShadowed => "ternary-keyword-shadowed",
        DbCode::TernaryInDataPosition => "ternary-in-data-position",
        DbCode::TernaryDanglingThen => "ternary-dangling-then",
        DbCode::TernaryDanglingElse => "ternary-dangling-else",
        DbCode::ConfigVarNotFound => "config-var-not-found",
        DbCode::ConfigVarNameNotLiteral => "config-var-name-not-literal",
        DbCode::ConfigVarNullCoercion => "config-var-null-coercion",
        // Phase C (meta-language) diagnostic codes.
        DbCode::ColumnsOfRequiresTableExpr => "columns-of-requires-table-expr",
        DbCode::ColumnsOfNamedArgument => "columns-of-named-argument",
        DbCode::ColumnRefFieldUnknown => "column-ref-field-unknown",
        DbCode::ColumnsOfUnresolvableSchema => "columns-of-unresolvable-schema",
        // Phase D (meta-language) diagnostic codes.
        DbCode::WithTagRequiresText => "with-tag-requires-text",
        DbCode::WithTagNamedArgument => "with-tag-named-argument",
        DbCode::WideReflectionUnknownAccessor => "wide-reflection-unknown-accessor",
        DbCode::WideReflectionUnexpectedArgument => "wide-reflection-unexpected-argument",
        DbCode::ModelRefFieldUnknown => "model-ref-field-unknown",
        DbCode::SourceRefFieldUnknown => "source-ref-field-unknown",
        // Phase E1 (meta-language) record diagnostic codes.
        DbCode::SmeltRecordRedefinition => "smelt-record-redefinition",
        DbCode::RecordFieldUnknown => "record-field-unknown",
        DbCode::RecordFieldMissing => "record-field-missing",
        DbCode::RecordFieldDuplicate => "record-field-duplicate",
        DbCode::RecordFieldTypeMismatch => "record-field-type-mismatch",
        DbCode::RecordLiteralUnknownTarget => "record-literal-unknown-target",
        DbCode::RecordFieldNotProjectable => "record-field-not-projectable",
        DbCode::RecordFieldTypeForbidden => "record-field-type-forbidden",
        DbCode::RecordCyclicDeclaration => "record-cyclic-declaration",
        DbCode::RecordInDataWorld => "record-in-data-world",
        // Phase E1 (meta-language) map diagnostic codes.
        DbCode::MapKeyTypeNotText => "map-key-type-not-text",
        DbCode::MapApiUnknown => "map-api-unknown",
        DbCode::MapApiArityMismatch => "map-api-arity-mismatch",
        DbCode::MapApiNamedArgument => "map-api-named-argument",
        DbCode::MapApiUnexpectedArgument => "map-api-unexpected-argument",
        DbCode::MapGetMissingKey => "map-get-missing-key",
        DbCode::MapApiArgTypeMismatch => "map-api-arg-type-mismatch",
        // Phase E1 (meta-language) loader diagnostic codes.
        DbCode::ConfigLoaderPathNotLiteral => "config-loader-path-not-literal",
        DbCode::ConfigLoaderPathEscapesWorkspace => "config-loader-path-escapes-workspace",
        DbCode::ConfigLoaderPathBackslash => "config-loader-path-backslash",
        DbCode::ConfigLoaderFileNotFound => "config-loader-file-not-found",
        DbCode::ConfigLoaderSchemaForbidden => "config-loader-schema-forbidden",
        DbCode::ConfigLoaderTomlNotYetSupported => "config-loader-toml-not-yet-supported",
        DbCode::ConfigLoaderParseError => "config-loader-parse-error",
        DbCode::ConfigLoaderRequiredFieldMissing => "config-loader-required-field-missing",
        DbCode::ConfigLoaderUnknownField => "config-loader-unknown-field",
        DbCode::ConfigLoaderTypeMismatch => "config-loader-type-mismatch",
        DbCode::ConfigLoaderRootShapeMismatch => "config-loader-root-shape-mismatch",
        DbCode::ConfigLoaderDuplicateMapKey => "config-loader-duplicate-map-key",
        DbCode::ConfigLoaderNullCoercion => "config-loader-null-coercion",
        // Multi-model production diagnostic codes.
        DbCode::GeneratesUnknownValue => "generates-unknown-value",
        DbCode::GeneratesMixedWithBareModel => "generates-mixed-with-bare-model",
        DbCode::GenerateFileBareSelectForbidden => "generate-file-bare-select-forbidden",
        DbCode::GenerateFileBodyTypeError => "generate-file-body-type-error",
        DbCode::ModelDefOutsideGeneratorFile => "model-def-outside-generator-file",
        DbCode::ModelDefInvalidName => "model-def-invalid-name",
        DbCode::ModelDefInvalidMaterialization => "model-def-invalid-materialization",
        DbCode::ModelDefDuplicateName => "model-def-duplicate-name",
        DbCode::ModelDefHandAuthoredCollision => "model-def-hand-authored-collision",
        DbCode::GeneratorBodyForbidsModelReflection => "generator-body-forbids-model-reflection",
        DbCode::ModelDefOverrideRequiresIncremental => "model-def-override-requires-incremental",
        // Timeseries frontmatter validation diagnostic codes.
        DbCode::TimeseriesRequiredForPartitionGrain => "timeseries-required-for-batched",
        DbCode::MalformedTimeseries => "malformed-timeseries",
        DbCode::PlausibleContractOnSkeletonColumn => "plausible-contract-on-skeleton-column",
        DbCode::MalformedFunctionalDependency => "malformed-functional-dependency",
        DbCode::MalformedBoundedDomain => "malformed-bounded-domain",
        // VALUES / CTE alias-column-list diagnostic codes.
        DbCode::AliasColumnArityMismatch => "alias-column-arity-mismatch",
        DbCode::EmptyValuesClause => "empty-values-clause",
        // Planner-rule diagnostic codes (keyed classifier,
        // batched batch-safety) surfaced via the uniform rule →
        // diagnostics interface.
        DbCode::KeyedRequiresGroupBy => "keyed-requires-group-by",
        DbCode::KeyedUnknownCombiner => "keyed-unknown-combiner",
        DbCode::KeyedGroupByContainsPartitionColumn => "keyed-group-by-contains-partition-column",
        DbCode::KeyedForbidsWindowFunctions => "keyed-forbids-window-functions",
        DbCode::KeyedForbidsNondeterministic => "keyed-forbids-nondeterministic",
        DbCode::KeyedSnapshotPostureUnsupported => "keyed-snapshot-posture-unsupported",
        DbCode::KeyedSnapshotSourceUnsupportedColumn => "keyed-snapshot-source-unsupported-column",
        DbCode::KeyedMultipleDrivingSources => "keyed-multiple-driving-sources",
        DbCode::KeyedSqlNotParseable => "keyed-sql-not-parseable",
        DbCode::KeyedOnceWriteUnproven => "keyed-once-write-unproven",
        DbCode::KeyedStateColumnCollision => "keyed-state-column-collision",
        DbCode::KeyedForbidsTimeseries => "keyed-forbids-timeseries",
        DbCode::KeyedRecurrenceDeclarationMismatch => "keyed-recurrence-declaration-mismatch",
        DbCode::KeyedForbidsSafetyOverrides => "keyed-forbids-safety-overrides",
        DbCode::MaterializedViewForbidsTimeseries => "materialized-view-forbids-timeseries",
        DbCode::MaterializedViewForbidsPartitionGrain => "materialized-view-forbids-batched",
        DbCode::PartitionGrainNotSafe => "batched-not-safe",
        // Multi-model section structure diagnostic codes.
        DbCode::MalformedSectionDelimiter => "malformed-section-delimiter",
        DbCode::UnclosedFrontmatter => "unclosed-frontmatter",
        DbCode::DuplicateAddress => "duplicate-address",
        DbCode::DuplicateEmittedName => "duplicate-emitted-name",
        DbCode::DefaultReferencesParameter => "default-references-parameter",
        DbCode::UnknownStructFieldType => "unknown-struct-field-type",
        DbCode::DecimalPrecisionOverflow => "decimal-precision-overflow",
        DbCode::NonPortableCollation => "non-portable-collation",
        DbCode::EventTimeColumnNotVisibleAtOuterSelect => {
            "event-time-column-not-visible-at-outer-select"
        }
        DbCode::PartitionGrainForbidsMetrics => "partition-grain-forbids-metrics",
        DbCode::StateModeWidening => "state-mode-widening",
        DbCode::CteRefOutsideTest => "cte-ref-outside-test",
        DbCode::CheckHasTestClause => "check-has-test-clause",
        DbCode::UnknownTestInput => "unknown-test-input",
        DbCode::UnknownTestCte => "unknown-test-cte",
        // Pipe SQL (Data-World |> pipe query) diagnostic codes.
        DbCode::PipeUnknownOperator => "pipe-unknown-operator",
        DbCode::PipeOperatorUnsupported => "pipe-operator-unsupported",
        DbCode::PipeStageMalformed => "pipe-stage-malformed",
        DbCode::GrainRequiredForIncremental => "grain-required-for-incremental",
        DbCode::GrainRequiresIncremental => "grain-requires-incremental",
        DbCode::GrainAssertionMismatch => "grain-assertion-mismatch",
        DbCode::MaintenanceNoAdmissibleTechnique => "maintenance-no-admissible-technique",
        DbCode::MaintenanceScanUnbounded => "maintenance-scan-unbounded",
        DbCode::MaintenanceSkeletonChanged => "maintenance-skeleton-changed",
        DbCode::MaintenancePartitionColumnChanged => "maintenance-partition-column-changed",
        DbCode::MaintenanceColumnAddNotBackfillable => "maintenance-column-add-not-backfillable",
        DbCode::MaintenanceGranularityMismatch => "maintenance-granularity-mismatch",
        DbCode::MaintenanceUnsupportedGrain => "maintenance-unsupported-grain",
        DbCode::MaintenanceWritePatternUnavailable => "maintenance-write-pattern-unavailable",
        DbCode::MaintenanceWriteAddressingRefused => "maintenance-write-addressing-refused",
        DbCode::DefinitionDeltaPending => "definition-delta-pending",
        DbCode::UnknownColumnTestKind => "unknown-column-test-kind",
        DbCode::ColumnTestOnUnknownColumn => "column-test-on-unknown-column",
        DbCode::ContractFrozenHorizonInvalid => "contract-frozen-horizon-invalid",
        DbCode::ContractDeferralInvalid => "contract-deferral-invalid",
        DbCode::ContractRetainDepartedInvalid => "contract-retain-departed-invalid",
        DbCode::ReservedProjectionAliasPrefix => "reserved-projection-alias-prefix",
        DbCode::UnsupportedOnBackend => "unsupported-on-backend",
        DbCode::KeyedRetractableContribution => "keyed-retractable-contribution",
        DbCode::PropertyDowngrade => "property-downgrade",
        DbCode::PropertyDiffBaselineUnavailable => "property-diff-baseline-unavailable",
        DbCode::MaintenanceStateDowngraded => "maintenance-state-downgraded",
        DbCode::DeclaredContractRequiresState => "declared-contract-requires-state",
        DbCode::SuccessionWindowFunctionNotLead => "succession-window-function-not-lead",
        DbCode::SuccessionPartitionKeyMismatch => "succession-partition-key-mismatch",
        DbCode::SuccessionOrderNotMonotoneClock => "succession-order-not-monotone-clock",
        DbCode::SuccessionRowLocalColumnViolation => "succession-row-local-column-violation",
        DbCode::SuccessionIdentityNotProjected => "succession-identity-not-projected",
        DbCode::SuccessionSingleSourceOnly => "succession-single-source-only",
        DbCode::SuccessionDrivingSourceNotAppendOnly => "succession-driving-source-not-append-only",
        DbCode::SuccessionPreFilterNotRowLocal => "succession-pre-filter-not-row-local",
        DbCode::SuccessionDeleteFilterMisplaced => "succession-delete-filter-misplaced",
        DbCode::SuccessionPreFilterNegatesFlag => "succession-pre-filter-negates-flag",
        DbCode::SuccessionPatternUnrecognized => "succession-pattern-unrecognized",
    }
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            db: Arc::new(Mutex::new(Database::default())),
            tracked_files: Arc::new(Mutex::new(Vec::new())),
            init_errors: Arc::new(Mutex::new(None)),
            python_model_sources: Arc::new(Mutex::new(HashMap::new())),
            python_cache: Arc::new(Mutex::new(PythonModelCache::default())),
            python_diagnostics: Arc::new(Mutex::new(HashMap::new())),
            project_roots: Arc::new(Mutex::new(Vec::new())),
            multi_model_files: Arc::new(Mutex::new(HashMap::new())),
            // Default: UTF-16, as required by LSP spec §3.17 general capabilities.
            negotiated_encoding: Arc::new(Mutex::new(PositionEncodingKind::UTF16)),
            known_tests: Arc::new(Mutex::new(Vec::new())),
            property_diff: Arc::new(Mutex::new(HashMap::new())),
            property_diff_derivation_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Test-observable handle onto the property-diff derivation counter
    /// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` risk R3).
    /// `pub` so integration tests in `tests/` can grab this (via
    /// `LspService::inner()`, before the service is handed to `Server`)
    /// and assert a burst of change events collapses to one derivation per
    /// project root, without exposing anything about
    /// `refresh_property_diff`'s internal coalescing mechanism itself.
    pub fn property_diff_derivation_counter(&self) -> Arc<std::sync::atomic::AtomicUsize> {
        Arc::clone(&self.property_diff_derivation_count)
    }

    /// Convert URI to file path, logging a warning if conversion fails.
    /// Returns None for non-file URIs (e.g., untitled:, git:).
    async fn uri_to_path(&self, uri: &Url) -> Option<PathBuf> {
        match uri.to_file_path() {
            Ok(p) => Some(p),
            Err(_) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Cannot process non-file URI: {}", uri),
                    )
                    .await;
                None
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut init_errors = InitErrors::default();

        // Initialize inputs to empty first - ensures Salsa queries are always set
        // even if workspace folders aren't provided or models/ doesn't exist
        {
            let mut db = self.db.lock().await;
            Backend::sync_workspace(&mut db, &[], &[]);
        }

        // Get workspace folders if provided
        if let Some(workspace_folders) = params.workspace_folders {
            let mut db = self.db.lock().await;
            let mut all_files = Vec::new();
            let mut all_project_roots = Vec::new();

            for folder in &workspace_folders {
                let workspace_path = match folder.uri.to_file_path() {
                    Ok(p) => p,
                    Err(_) => {
                        init_errors.workspace_errors.push(format!(
                            "Cannot process workspace folder URI: {}",
                            folder.uri
                        ));
                        continue;
                    }
                };

                // Recursively discover smelt projects
                let project_roots = find_smelt_projects(&workspace_path);

                for project_root in project_roots {
                    // Check for ambiguous smelt config
                    if project_root.join("smelt.yml").exists()
                        && project_root.join("smelt.yaml").exists()
                    {
                        init_errors.workspace_errors.push(format!(
                            "Both smelt.yml and smelt.yaml exist in {}",
                            project_root.display()
                        ));
                        continue;
                    }

                    all_project_roots.push(project_root.clone());

                    // Canonical workspace loading — single source of truth for
                    // CLI and LSP. See docs/specs/architecture.md →
                    // "Workspace loading parity rule (CLI ↔ LSP)".
                    let loaded = smelt_core::load_workspace(&project_root);
                    init_errors
                        .workspace_errors
                        .extend(loaded.errors.workspace_errors.iter().cloned());
                    init_errors
                        .source_errors
                        .extend(loaded.errors.source_errors.iter().cloned());
                    // The "no models found" soft warning is noise for empty /
                    // functions-only workspaces; drop it from the LSP surface.
                    let model_errors: Vec<String> = loaded
                        .errors
                        .model_errors
                        .iter()
                        .filter(|e| !e.starts_with("No models found"))
                        .cloned()
                        .collect();
                    init_errors.model_errors.extend(model_errors);

                    // Sources input + loader files (the latter was previously
                    // missing in the LSP — smelt.config.load_yaml(...) in
                    // generator files didn't resolve).
                    db.set_project_input(project_root.clone(), loaded.sources_text.clone());
                    smelt_db::workspace_ingest::register_loader_files_from_disk(
                        &mut db,
                        &project_root,
                    );
                    // The deployed-schema snapshot world-fact input
                    // (symmetric with the CLI's `init_db` — Workspace
                    // Loading Parity rule): `docs/specs/definition_deltas.md`
                    // §"Detection".
                    let effective_target = loaded.config.target.as_deref().unwrap_or("dev");
                    smelt_db::workspace_ingest::register_deployed_schemas_from_disk(
                        &mut db,
                        &project_root,
                        effective_target,
                    );

                    // Register SQL files via register_sql_content so the LSP's
                    // multi-model line-offset tracking populates correctly.
                    // Dedup by real path — multi-model files appear once per
                    // section in loaded.sql_files but share one real file.
                    let mut seen_real_paths: std::collections::HashSet<PathBuf> =
                        std::collections::HashSet::new();
                    for model in &loaded.sql_files {
                        let real_path = model.model_id.source_path().to_path_buf();
                        if !seen_real_paths.insert(real_path.clone()) {
                            continue;
                        }
                        match std::fs::read_to_string(&real_path) {
                            Ok(content) => {
                                let paths = self
                                    .register_sql_content(
                                        &mut db,
                                        &real_path,
                                        &content,
                                        &project_root,
                                    )
                                    .await;
                                all_files.extend(paths);
                            }
                            Err(e) => {
                                init_errors.model_errors.push(format!(
                                    "Failed to read {}: {}",
                                    real_path.display(),
                                    e
                                ));
                            }
                        }
                    }

                    // Collect test models for the VSCode TestController (published
                    // after `initialized` once the client connection is ready).
                    self.collect_tests_into_cache(&loaded.sql_files).await;

                    // Python discovery — kept inline; runs python_scan with
                    // LSP-specific state (python_cache, python_model_sources)
                    // and emits LSP diagnostics for execution errors. Not yet
                    // shared with the CLI's run-Python pipeline.
                    let config = &loaded.config;
                    for model_path in &config.paths {
                        let models_path = project_root.join(model_path);
                        let context_json = build_python_context(&all_files, config, &project_root);
                        let mut cache = self.python_cache.lock().await;
                        *cache = PythonModelCache::load(&project_root);
                        let scan_result = crate::python_scan::discover_python_models(
                            &models_path,
                            &project_root,
                            &mut cache,
                            &context_json,
                        );
                        drop(cache);

                        if !scan_result.models.is_empty() {
                            let mut py_sources = self.python_model_sources.lock().await;
                            for py_model in &scan_result.models {
                                // Use <name>.sql so file_stem() yields the model name directly.
                                // Multi-model .py files each get their own virtual file.
                                let virtual_sql_path = py_model
                                    .source_path
                                    .parent()
                                    .unwrap_or_else(|| std::path::Path::new("."))
                                    .join(format!("{}.sql", py_model.name));

                                db.set_source_file(
                                    virtual_sql_path.clone(),
                                    py_model.sql.clone(),
                                    project_root.clone(),
                                );
                                // Map virtual path back to actual .py source for goto-definition
                                py_sources.insert(
                                    virtual_sql_path.clone(),
                                    (py_model.source_path.clone(), py_model.decorator_line),
                                );
                                all_files.push(virtual_sql_path);
                            }
                        }

                        // Collect Python model errors as diagnostics
                        if !scan_result.errors.is_empty() {
                            let mut py_diags = self.python_diagnostics.lock().await;
                            for error in &scan_result.errors {
                                let line = error.line.unwrap_or(1).saturating_sub(1);
                                let diag = lsp_types::Diagnostic {
                                    range: Range {
                                        start: Position::new(line, 0),
                                        end: Position::new(line, 0),
                                    },
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    message: error.message.clone(),
                                    source: Some("smelt-python".to_string()),
                                    ..Default::default()
                                };
                                py_diags
                                    .entry(error.source_path.clone())
                                    .or_default()
                                    .push(diag);
                                init_errors.model_errors.push(format!(
                                    "Python model error in {}: {}",
                                    error.source_path.display(),
                                    error.message,
                                ));
                            }
                        }
                    }
                }
            }

            Backend::sync_workspace(&mut db, &all_files, &all_project_roots);

            // Store project roots and file list for file-change handling
            *self.tracked_files.lock().await = all_files;
            *self.project_roots.lock().await = all_project_roots;
        }

        // Store errors for reporting after initialized notification
        *self.init_errors.lock().await = Some(init_errors);

        // Negotiate position encoding with the client.
        //
        // The server supports UTF-16 and UTF-8. Preference order: UTF-16 first
        // (LSP default), then UTF-8. If the client advertises a list, pick the
        // first mutually-supported encoding. Unknown/unsupported encodings are
        // skipped; if none match, fall back to UTF-16.
        let client_encodings: Vec<PositionEncodingKind> = params
            .capabilities
            .general
            .as_ref()
            .and_then(|g| g.position_encodings.clone())
            .unwrap_or_default();

        let negotiated = if client_encodings.is_empty() {
            // Client advertised no preference → LSP default is UTF-16.
            PositionEncodingKind::UTF16
        } else {
            // Pick the first encoding the client listed that we support.
            // Supported: UTF-8 and UTF-16.
            client_encodings
                .iter()
                .find(|e| **e == PositionEncodingKind::UTF8 || **e == PositionEncodingKind::UTF16)
                .cloned()
                .unwrap_or(PositionEncodingKind::UTF16)
        };

        *self.negotiated_encoding.lock().await = negotiated.clone();

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // Advertise the negotiated encoding back to the client.
                position_encoding: Some(negotiated),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "(".to_string(),
                        ".".to_string(),
                    ]),
                    ..Default::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "smelt language server initialized")
            .await;

        // Publish per-entity source YAML diagnostics (malformed sources) up
        // front — before the `register_capability` round-trip below, which
        // awaits a client response — so the source surface is populated even if
        // dynamic registration is slow or unsupported. Source discovery is
        // restart-scoped, so this startup publish is its lifecycle.
        self.publish_source_diagnostics().await;
        // Publish address-collision diagnostics alongside source diagnostics.
        // These are also project-scoped and restart-scoped.
        self.publish_address_collision_diagnostics().await;
        // Publish emitted-name collision diagnostics (DuplicateEmittedName).
        self.publish_emitted_name_collision_diagnostics().await;

        // Property-diff: derive each project's diff against its default
        // baseline on workspace load (`docs/specs/property_diff.md`
        // §Surface "Editor"). Placed BEFORE the `register_capability`
        // round-trip below for the same reason `publish_source_diagnostics`
        // is: that call awaits a client response, and a client slow (or
        // never) to answer it must not delay this. Off the request path —
        // `refresh_property_diff` hands the actual work to
        // `spawn_blocking` — but awaited here sequentially, same as the
        // diagnostics publishes above it.
        {
            let project_roots_for_diff = self.project_roots.lock().await.clone();
            for root in project_roots_for_diff {
                self.refresh_property_diff(root).await;
            }
        }

        // Register file watchers (dynamic registration). Watch every
        // discoverable `.sql` and `.py` under each project root, derived
        // from the loaded project roots rather than hardcoded directory names.
        // See `derive_watch_globs` for the rationale (D-48).
        let project_roots_snapshot = self.project_roots.lock().await.clone();
        let mut watchers = derive_watch_globs(&project_roots_snapshot);
        // `.git` watch (`docs/specs/property_diff.md` §Surface "Editor"): a
        // promptness trigger for the property-diff refresh, not the
        // correctness mechanism (D2) — a project with no resolvable git
        // repo root simply gets no `.git` watcher, same as any other
        // non-git workspace shows no lens.
        let git_roots: Vec<PathBuf> = {
            let mut roots: Vec<PathBuf> = project_roots_snapshot
                .iter()
                .filter_map(|root| smelt_core::baseline::discover_repo_root(root))
                .collect();
            roots.sort();
            roots.dedup();
            roots
        };
        watchers.extend(derive_git_watch_globs(&git_roots));
        let registration = Registration {
            id: "smelt-file-watcher".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions { watchers })
                    .unwrap(),
            ),
        };
        // intentionally ignored: LSP capability registration failure is non-fatal;
        // the server continues without file-watcher notifications.
        let _ = self.client.register_capability(vec![registration]).await;

        // Report any initialization errors
        if let Some(errors) = self.init_errors.lock().await.take() {
            if errors.has_errors() {
                // Log each error
                for err in &errors.workspace_errors {
                    self.client.log_message(MessageType::ERROR, err).await;
                }
                for err in &errors.source_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }
                for err in &errors.model_errors {
                    self.client.log_message(MessageType::WARNING, err).await;
                }

                // Show summary notification to user
                self.client
                    .show_message(
                        MessageType::WARNING,
                        format!(
                            "smelt: {} file(s) failed to load. Check Output for details.",
                            errors.total_count()
                        ),
                    )
                    .await;
            }
        }

        // Publish Python diagnostics collected during init
        let py_diags = self.python_diagnostics.lock().await;
        for (path, diagnostics) in py_diags.iter() {
            if let Ok(uri) = Url::from_file_path(path) {
                self.client
                    .publish_diagnostics(uri, diagnostics.clone(), None)
                    .await;
            }
        }
        drop(py_diags);

        // Publish test discovery — VSCode TestController subscribes to this.
        self.publish_known_tests().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Check if this is sources.yml/yaml - update sources config and refresh all diagnostics
        if is_sources_file(&path) {
            if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                let mut db = self.db.lock().await;
                db.set_project_input(project_root, params.text_document.text);
                drop(db);
                self.publish_all_diagnostics().await;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            let mut db = self.db.lock().await;
            // If this file wasn't seen during init, find its project root
            let project_roots = self.project_roots.lock().await.clone();
            let has_project_root = project_roots.iter().any(|root| path.starts_with(root));
            if !has_project_root {
                // Try to discover project root by walking up
                if let Some(project_root) = find_project_root_by_walking_up(&path) {
                    // Register this new project
                    let mut roots = self.project_roots.lock().await;
                    if !roots.contains(&project_root) {
                        roots.push(project_root.clone());
                        // Load sources for this project
                        let sources_content = find_config_file(&project_root, "sources")
                            .ok()
                            .flatten()
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .unwrap_or_default();
                        db.set_project_input(project_root.clone(), sources_content);
                    }
                    drop(roots);
                }
            }
            // Register file content (handles multi-model splitting)
            // register_sql_content skips mutations when content hasn't changed
            let project_root_for_reg = project_roots
                .iter()
                .find(|root| path.starts_with(root))
                .cloned()
                .unwrap_or_default();
            let registered_paths = self
                .register_sql_content(
                    &mut db,
                    &path,
                    &params.text_document.text,
                    &project_root_for_reg,
                )
                .await;
            // Only update tracked_files + workspace if new paths were registered
            let mut tracked = self.tracked_files.lock().await;
            let mut changed = false;
            for rp in &registered_paths {
                if !tracked.contains(rp) {
                    tracked.push(rp.clone());
                    changed = true;
                }
            }
            if changed {
                Backend::sync_workspace(&mut db, &tracked, &project_roots);
            }
            drop(tracked);
            drop(db);
            self.publish_diagnostics(uri).await;
        } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
            // Non-SQL, non-sources, non-Python file — register as a source file
            let mut db = self.db.lock().await;
            let project_roots = self.project_roots.lock().await.clone();
            let project_root = project_roots
                .iter()
                .find(|root| path.starts_with(root))
                .cloned()
                .unwrap_or_default();
            db.set_source_file(path, params.text_document.text, project_root);
            drop(db);
            self.publish_diagnostics(uri).await;
        }
        // Skip .py files - they are handled during init via subprocess execution,
        // and parsing them as SQL would produce spurious diagnostics
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = match self.uri_to_path(&uri).await {
            Some(p) => p,
            None => return,
        };

        // Get new text (we use FULL sync, so there's only one change)
        if let Some(change) = params.content_changes.into_iter().next() {
            if is_sources_file(&path) {
                if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                    let mut db = self.db.lock().await;
                    db.set_project_input(project_root, change.text);
                    drop(db);
                    self.publish_all_diagnostics().await;
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                let mut db = self.db.lock().await;
                let project_roots = self.project_roots.lock().await.clone();
                let project_root = project_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned()
                    .unwrap_or_default();
                let registered_paths = self
                    .register_sql_content(&mut db, &path, &change.text, &project_root)
                    .await;
                // Ensure all registered paths are in tracked_files + workspace
                let mut tracked = self.tracked_files.lock().await;
                let mut changed = false;
                for rp in &registered_paths {
                    if !tracked.contains(rp) {
                        tracked.push(rp.clone());
                        changed = true;
                    }
                }
                if changed {
                    Backend::sync_workspace(&mut db, &tracked, &project_roots);
                }
                drop(tracked);
                drop(db);
                self.publish_all_diagnostics().await;
            } else if path.extension().and_then(|s| s.to_str()) != Some("py") {
                let mut db = self.db.lock().await;
                let project_roots = self.project_roots.lock().await.clone();
                let project_root = project_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned()
                    .unwrap_or_default();
                db.set_source_file(path, change.text, project_root);
                drop(db);
                self.publish_diagnostics(uri).await;
            }
            // Skip .py files - parsing as SQL would produce spurious diagnostics
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Property-diff refresh trigger (`docs/specs/property_diff.md`
        // §Surface "Editor", Δ2): a model file being saved. `smelt.yml` and
        // source YAML overlays are deliberately not applied on `didChange`
        // (D4) — a save of either still lands here as an ordinary file
        // save and refreshes from the now-current disk content.
        if let Ok(path) = params.text_document.uri.to_file_path() {
            let project_roots = self.project_roots.lock().await.clone();
            if let Some(project_root) = project_roots.iter().find(|root| path.starts_with(root)) {
                self.refresh_property_diff(project_root.clone()).await;
            }
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // Property-diff refresh is deduped for the WHOLE notification
        // (`docs/outcomes/20260905-property-diff/phases/07-plan.md` risk
        // R3): a rebase or branch switch can carry hundreds of `.sql`/`.git`
        // change events in one `DidChangeWatchedFilesParams`, and each
        // project root must refresh at most once here, not once per event.
        // `refresh_property_diff` itself absorbs any trigger that arrives
        // WHILE a refresh for that root is already running (its own
        // `pending` flag schedules exactly one trailing re-run) — this set
        // only stops the same notification from queueing the same root
        // twice up front.
        let mut diff_roots: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

        for change in params.changes {
            let path = match change.uri.to_file_path() {
                Ok(p) => p,
                // intentionally ignored: non-file URIs (e.g. git:, untitled:)
                // are not tracked by the smelt workspace.
                Err(_) => continue,
            };

            if path.extension().and_then(|s| s.to_str()) == Some("py") {
                self.handle_python_file_change(&path).await;
            } else if is_sources_file(&path) {
                // Re-read sources.yml from disk when changed outside the editor
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(project_root) = path.parent().map(|p| p.to_path_buf()) {
                        let mut db = self.db.lock().await;
                        db.set_project_input(project_root, content);
                        drop(db);
                        self.publish_all_diagnostics().await;
                    }
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
                // External `.sql` change (currently only `functions/**/*.sql`
                // is watched). Re-read content into the DB and refresh all
                // diagnostics so dependents pick up the new signature.
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut db = self.db.lock().await;
                    let project_roots = self.project_roots.lock().await.clone();
                    let project_root = project_roots
                        .iter()
                        .find(|root| path.starts_with(root))
                        .cloned()
                        .unwrap_or_default();
                    let registered_paths = self
                        .register_sql_content(&mut db, &path, &content, &project_root)
                        .await;
                    let mut tracked = self.tracked_files.lock().await;
                    let mut changed = false;
                    for rp in &registered_paths {
                        if !tracked.contains(rp) {
                            tracked.push(rp.clone());
                            changed = true;
                        }
                    }
                    if changed {
                        Backend::sync_workspace(&mut db, &tracked, &project_roots);
                    }
                    drop(tracked);
                    drop(db);
                    self.publish_all_diagnostics().await;
                    // Re-scan tests in case a test file was added/modified.
                    self.refresh_and_publish_tests().await;
                    // Δ2: a model file changed outside the editor.
                    diff_roots.insert(project_root);
                }
            } else if path.components().any(|c| c.as_os_str() == ".git") {
                // `.git/HEAD`, `.git/refs/**`, or `.git/packed-refs` changed
                // (`derive_git_watch_globs`) — a commit, checkout, or merge
                // may have moved the resolved baseline. This is a
                // promptness trigger only (D2): `refresh_property_diff`
                // always re-resolves and compares the commit, so a client
                // that never reports `.git` changes only loses promptness,
                // not correctness.
                let project_roots = self.project_roots.lock().await.clone();
                diff_roots.extend(project_roots);
            }
        }

        for root in diff_roots {
            self.refresh_property_diff(root).await;
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.goto_definition_impl(params).await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.references_impl(params).await
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        // Read-only: `code_lens` never derives (R6) — it only reads
        // whatever `refresh_property_diff` last cached, per project.
        let path = match self.uri_to_path(&params.text_document.uri).await {
            Some(p) => p,
            None => return Ok(None),
        };

        let property_diff = self.property_diff.lock().await;
        for state in property_diff.values() {
            if let Some(title) = state.lenses.get(&path) {
                return Ok(Some(vec![CodeLens {
                    // §Surface "Editor": "one lens on the first line".
                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                    command: Some(Command {
                        title: title.clone(),
                        command: "smelt.showPropertyDiff".to_string(),
                        arguments: Some(vec![serde_json::json!({
                            "modelPath": path.to_string_lossy(),
                        })]),
                    }),
                    data: None,
                }]));
            }
        }
        Ok(None)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        self.code_action_impl(params).await
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        self.prepare_rename_impl(params).await
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.rename_impl(params).await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.hover_impl(params).await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.completion_impl(params).await
    }
}

#[cfg(test)]
mod watch_glob_tests {
    use super::*;
    use tempfile::TempDir;

    fn glob_string(w: &FileSystemWatcher) -> &str {
        match &w.glob_pattern {
            GlobPattern::String(s) => s.as_str(),
            GlobPattern::Relative(_) => panic!("expected string glob"),
        }
    }

    /// Verify `derive_watch_globs` produces two root-scoped watchers per root:
    /// one for `**/*.sql` (all discoverable SQL) and one for `**/*.py`.
    /// The patterns must not restrict to `models/` or `functions/`.
    #[test]
    fn derive_watch_globs_covers_all_sql_and_py() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();

        let watchers = derive_watch_globs(std::slice::from_ref(&root));
        assert_eq!(
            watchers.len(),
            2,
            "expected exactly one .sql and one .py watcher"
        );

        let root_str = root.to_string_lossy();
        let sql_pat = watchers
            .iter()
            .find(|w| glob_string(w).ends_with("**/*.sql"))
            .expect("must have a .sql watcher");
        let py_pat = watchers
            .iter()
            .find(|w| glob_string(w).ends_with("**/*.py"))
            .expect("must have a .py watcher");

        // Patterns are project-root-scoped
        assert!(
            glob_string(sql_pat).starts_with(root_str.as_ref()),
            ".sql pattern must be scoped to the project root"
        );
        assert!(
            glob_string(py_pat).starts_with(root_str.as_ref()),
            ".py pattern must be scoped to the project root"
        );

        // Patterns must NOT restrict to models/ or functions/ (universal coverage)
        assert!(
            !glob_string(sql_pat).contains("/models/"),
            ".sql pattern must not restrict to models/"
        );
        assert!(
            !glob_string(sql_pat).contains("/functions/"),
            ".sql pattern must not restrict to functions/"
        );
        assert!(
            !glob_string(py_pat).contains("/models/"),
            ".py pattern must not restrict to models/"
        );
    }

    /// Two project roots produce four watchers (2 per root), each root-scoped.
    #[test]
    fn derive_watch_globs_scales_with_multiple_roots() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        let roots = vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()];

        let watchers = derive_watch_globs(&roots);
        assert_eq!(watchers.len(), 4, "expected 2 watchers per project root");

        for (root, expected_count) in [(&dir1, 2usize), (&dir2, 2)] {
            let root_str = root.path().to_string_lossy();
            let root_watchers: Vec<_> = watchers
                .iter()
                .filter(|w| glob_string(w).starts_with(root_str.as_ref()))
                .collect();
            assert_eq!(
                root_watchers.len(),
                expected_count,
                "each root should have 2 watchers"
            );
        }
    }
}
