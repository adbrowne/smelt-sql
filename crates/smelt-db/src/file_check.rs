//! The file-diagnostics orchestrator.
//!
//! `file_diagnostics` / `check_file_diagnostics` are the thin Salsa wrappers
//! named as acceptable orchestration exceptions in `architecture.md` §"Salsa
//! purity rule (analysis)": they gather inputs from other queries and call the
//! pure checks (`metadata_errors`, `meta_lists`, `diagnostic_mapping`, the
//! `queries::*` checkers) rather than embedding analysis logic themselves.

use std::collections::HashMap;

use line_index::{LineCol as LILineCol, LineIndex as LI};
use salsa::Accumulator;
use smelt_core::metadata::{extract_file_metadata, FileMetadata, MetadataError, MixedKind};
use smelt_parser::File as AstFile;

use crate::*;

// ============================================================================
// Diagnostics (accumulator-based orchestrator)
// ============================================================================

/// The grain label a contract-lattice validator names in a refusal message
/// (`smelt_logical::contract::GrainLabel`): the declared `grain:` when
/// written, else the fact-derived label (`resolved_grain()`, `timeseries:`/
/// `unique_key:`), else — for an undeclared-grain `refresh: incremental`
/// model — the keyed-succession leaf classifier's own verdict over the same
/// `(ref, SourceInfo)` pairs `smelt-db`'s own maintenance-plan derivation
/// consults (`queries::maintenance::build_succession_context`). Falls back
/// to `GrainLabel::Key` when none of those apply (unchanged from this
/// check's pre-succession `metadata.grain.unwrap_or(Grain::Key)` default —
/// a model with a `contract:` block but no incremental grain at all is an
/// existing, unrelated malformed-declaration shape, not this phase's
/// concern).
fn model_grain_label(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    metadata: &smelt_core::ModelMetadata,
    sql_body: &str,
) -> smelt_logical::contract::GrainLabel {
    if let Some(g) = metadata.grain {
        return g.into();
    }
    if let Some(g) = metadata.resolved_grain() {
        return g.into();
    }
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped
                .strip_prefix("sources.")
                .unwrap_or(stripped)
                .to_string();
            Some((bare, ref_source_info(db, workspace, project, r)))
        })
        .collect();
    let ctx = crate::queries::maintenance::build_succession_context(sql_body, &source_refs);
    let verdict = match smelt_logical::analysis::walk::QueryTree::from_sql(sql_body) {
        Some(tree) => smelt_logical::analysis::walk::model_keyed_succession(&tree, &ctx),
        None => {
            return smelt_logical::contract::GrainLabel::Key;
        }
    };
    match verdict {
        smelt_logical::analysis::succession::SuccessionVerdict::Recognized { .. } => {
            smelt_logical::contract::GrainLabel::Succession
        }
        smelt_logical::analysis::succession::SuccessionVerdict::NotSuccession { .. } => {
            smelt_logical::contract::GrainLabel::Key
        }
    }
}

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
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
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

    // Generator-file frontmatter diagnostics: bridge MetadataError variants
    // that arise from `generates:` key validation into standard diagnostics.
    // These must run before parse errors so that callers see the frontmatter
    // error rather than a confusing "Expected SELECT statement" parse error.
    match extract_file_metadata(text) {
        Err(MetadataError::GeneratesUnknownValue { value, value_span }) => {
            // Anchor at the YAML value token (1-based line/col → 0-based).
            let diag_line = value_span.line.saturating_sub(1) as u32;
            let diag_col = value_span.column.saturating_sub(1) as u32;
            let li = LI::new(text);
            let start_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col,
                })
                .unwrap_or_default();
            let end_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col + value.len() as u32,
                })
                .unwrap_or(start_ts);
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!("generates must be `models`; found {}", value),
                range: rowan::TextRange::new(start_ts, end_ts),
                code: Some(DiagnosticCode::GeneratesUnknownValue),
                data: None,
            })
            .accumulate(db);
            // File does not parse as SQL; no further checks make sense.
            return;
        }
        Err(MetadataError::GeneratesMixedWithBareModel { offending, span }) => {
            // Anchor at the offending key / delimiter (1-based → 0-based).
            let diag_line = span.line.saturating_sub(1) as u32;
            let diag_col = span.column.saturating_sub(1) as u32;
            let key_len = match &offending {
                MixedKind::NameField => "name:".len() as u32,
                MixedKind::SectionDelimiter => "--- name:".len() as u32,
            };
            let li = LI::new(text);
            let start_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col,
                })
                .unwrap_or_default();
            let end_ts = li
                .offset(LILineCol {
                    line: diag_line,
                    col: diag_col + key_len,
                })
                .unwrap_or(start_ts);
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: "generates: models cannot coexist with bare-model identity (name field or section delimiter)".to_string(),
                range: rowan::TextRange::new(start_ts, end_ts),
                code: Some(DiagnosticCode::GeneratesMixedWithBareModel),
                data: None,
            })
            .accumulate(db);
            // File cannot be used further; bail.
            return;
        }
        Err(e) => {
            // All MetadataError variants not handled by the Generates* arms above
            // go through the exhaustive mapper. The compiler enforces that every
            // new MetadataError variant is explicitly listed there.
            if let Some(diag) = map_metadata_error_to_diagnostic(&e) {
                DiagnosticAcc(diag).accumulate(db);
            }
            // A structural metadata error means the file's model shape is unknown;
            // skip all subsequent semantic checks (refs, types, timeseries, etc.)
            // to avoid cascading noise from a file the parser couldn't classify.
            return;
        }
        Ok(FileMetadata::Generator { .. }) => {
            // Check whether the parsed generator body starts with a bare SQL
            // statement. The parse_file query routes generator files through the
            // meta-expression parser which produces a SELECT_STMT node when it
            // encounters SELECT/WITH/VALUES as the first body token.
            let parse = parse_file(db, file);
            let syntax = parse.syntax();
            // A bare SELECT is only a problem when it is a *direct* child of the
            // FILE root — that is, when the generator body itself is a top-level
            // SELECT/WITH/VALUES statement (the hand-authored model shape).
            // SELECT_STMT nodes nested inside record-literal field values (e.g.
            // `ModelDef { body: SELECT * FROM t }`) are valid TableExpr values and
            // must NOT trigger this diagnostic.
            let has_bare_sql = syntax
                .children()
                .any(|n| n.kind() == smelt_parser::SyntaxKind::SELECT_STMT);
            if has_bare_sql {
                // Find the SELECT_STMT direct child to anchor the diagnostic.
                let select_node = syntax
                    .children()
                    .find(|n| n.kind() == smelt_parser::SyntaxKind::SELECT_STMT);
                let bare_range = select_node
                    .and_then(|n| n.first_token())
                    .map(|t| t.text_range())
                    .unwrap_or(rowan::TextRange::empty(rowan::TextSize::from(0)));
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: "generator file body must produce List<ModelDef>; bare SELECT is the hand-authored model shape".to_string(),
                    range: bare_range,
                    code: Some(DiagnosticCode::GenerateFileBareSelectForbidden),
                    data: None,
                })
                .accumulate(db);
            }

            // Surface diagnostics from the W2 (evaluate_generator) and W3
            // (emitted_models) pipeline for this generator file.
            //
            // W2 diagnostics include: GenerateFileBodyTypeError,
            // ModelDefDuplicateName, ModelDefInvalidName,
            // ModelDefInvalidMaterialization, GeneratorBodyForbidsModelReflection.
            //
            // W3 diagnostics include: ModelDefHandAuthoredCollision and
            // cross-generator collisions anchored at this file.
            let gen_file_path = file.path(db).to_path_buf();
            let evaluated = evaluate_generator(db, workspace, file);
            for diag in &evaluated.diagnostics {
                DiagnosticAcc(diag.clone()).accumulate(db);
            }
            // W3 collision diagnostics: each `DiscardedEmission` pairs the
            // dropped emission with its collision diagnostic in a single
            // struct, so there is no risk of the two drifting out of step
            // (`DiscardedEmission` in `crates/smelt-db/src/queries/project.rs`).
            // We emit only those where the discarded emission's
            // `generator_file` matches the current file.
            let emitted_result = emitted_models(db, workspace);
            for item in emitted_result.discarded.iter() {
                if item.emission.generator_file == gen_file_path {
                    DiagnosticAcc(item.diagnostic.clone()).accumulate(db);
                }
            }

            // W4 body diagnostics: for each surviving emission from this
            // generator file, run `emitted_model_body_analysis` and surface
            // any SQL-level diagnostics (UndeclaredColumn, ParseError,
            // CteCycle, etc.) anchored inside the generator file body.
            // Discarded emissions are naturally skipped because they are not
            // in `survivors` — their bodies are never analysed.
            for survivor in emitted_result.survivors.iter() {
                if survivor.generator_file != gen_file_path {
                    continue;
                }
                let analysis =
                    emitted_model_body_analysis(db, workspace, file, survivor.name.clone());
                for diag in analysis.diagnostics.iter() {
                    DiagnosticAcc(diag.clone()).accumulate(db);
                }
            }

            // Loader call diagnostics for generator files: smelt.config.load_yaml /
            // load_json calls in the generator body must also be validated (path
            // literals, schema arguments, content, and per-target overlay validation).
            // These diagnostics are emitted here (before the early return) so that
            // generator files surface the same loader-call diagnostics as regular
            // model files.  BUG-014 P4: this is the seam that surfaces overlay
            // validation errors (`ConfigLoaderUnknownField` etc.) for generator files.
            for diag in
                crate::queries::loader::loader_call_diagnostics_for_file(db, workspace, file)
            {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Generator files are not SQL models; skip the model-validity check
            // and all SQL-only diagnostics.
            return;
        }
        _ => {
            // Non-generator file: continue with the standard parse-error pipeline.
        }
    }

    // Model frontmatter diagnostics via the unified catalogue (U3).
    // Skips smelt.define / smelt.extern function files — their frontmatter is
    // handled (with the correct DeclarationKind) by
    // frontmatter_parse_diagnostics_for_file. Only pure SQL model files reach
    // this block. Calls parse_frontmatter(text, Model/Check) to surface unknown-key
    // errors and inapplicable-key warnings. Also tries to deserialize
    // ModelMetadata from the validated map to catch nested sub-field failures
    // (e.g. a bad timeseries.granularity value) that would previously be swallowed.
    let (is_function_file, is_check_file) = {
        let p = parse_file(db, file);
        let ast_opt = AstFile::cast(p.syntax());
        let is_fn = ast_opt
            .as_ref()
            .map(|ast| ast.defines().next().is_some() || ast.externs().next().is_some())
            .unwrap_or(false);
        let is_chk = ast_opt
            .as_ref()
            .map(|ast| ast.checks().next().is_some())
            .unwrap_or(false);
        (is_fn, is_chk)
    };
    if !is_function_file {
        if let Some(yaml_text) = smelt_core::frontmatter_yaml_text(text) {
            use smelt_core::{FrontmatterSeverity, ModelMetadata};
            let decl_kind = if is_check_file {
                smelt_core::DeclarationKind::Check
            } else {
                smelt_core::DeclarationKind::Model
            };
            let (validated_map, fm_diags) = smelt_core::parse_frontmatter(&yaml_text, decl_kind);

            // Emit catalogue diagnostics (unknown key → Error, inapplicable → Warning).
            for fm_diag in &fm_diags {
                let severity = match fm_diag.severity {
                    FrontmatterSeverity::Error => DiagnosticSeverity::Error,
                    FrontmatterSeverity::Warning => DiagnosticSeverity::Warning,
                };
                DiagnosticAcc(Diagnostic {
                    severity,
                    message: fm_diag.message.clone(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::FrontmatterParseError),
                    data: None,
                })
                .accumulate(db);
            }

            // Try to deserialize ModelMetadata from the validated map to catch
            // nested sub-field failures (e.g. timeseries.granularity: fortnight).
            // A failure here means a nested field is malformed — surface as
            // MalformedTimeseries.
            if !validated_map.is_empty() {
                if let Err(serde_err) = serde_yaml::from_value::<ModelMetadata>(
                    serde_yaml::Value::Mapping(validated_map),
                ) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("MalformedTimeseries: {serde_err}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::MalformedTimeseries),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Timeseries / incremental frontmatter validation.
    // Runs on every non-CSV, non-generator file that has Single frontmatter.
    // Calls the pure `validate_timeseries` function from smelt-core and maps
    // its errors into DiagnosticAcc entries so they surface through
    // `file_diagnostics`.
    if let Ok(FileMetadata::Single {
        ref metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    {
        let sql_body = &text[sql_offset..];
        if let Err(ts_err) = smelt_core::metadata::validate_timeseries(metadata, sql_body) {
            let maybe_diag = match &ts_err {
                smelt_core::metadata::MetadataError::TimeseriesRequiredForPartitionGrain => Some((
                    ts_err.to_string(),
                    DiagnosticCode::TimeseriesRequiredForPartitionGrain,
                )),
                smelt_core::metadata::MetadataError::MalformedTimeseries { .. } => {
                    Some((ts_err.to_string(), DiagnosticCode::MalformedTimeseries))
                }
                smelt_core::metadata::MetadataError::PlausibleContractOnSkeletonColumn {
                    ..
                } => Some((
                    ts_err.to_string(),
                    DiagnosticCode::PlausibleContractOnSkeletonColumn,
                )),
                // `validate_timeseries` no longer raises this — whether
                // keyed+timeseries: is admitted is decided by the locality
                // gate in plan derivation
                // (`smelt_logical::maintenance::locality::establish_locality`),
                // which surfaces its own `KeyedForbidsTimeseries` diagnostic
                // from the maintenance-plan fold-in below. The arm is kept
                // (rather than folded into the `_ => None` wildcard) so the
                // `MetadataError` variant's diagnostic mapping stays
                // documented at its point of historical use.
                smelt_core::metadata::MetadataError::KeyedForbidsTimeseries => None,
                // `batched:` without `refresh: batched` maps to the generic
                // YamlParseError code — no dedicated code exists yet. This
                // is also the only remaining way a `grain: key` model can
                // still carry an internally-folded `batched` block — the
                // literal sub-block is refused before a `ModelMetadata`
                // exists, so the dedicated `KeyedForbidsPartitionGrain` code was
                // retired outright (`docs/specs/diagnostics.md` §"Keyed
                // refresh mode").
                smelt_core::metadata::MetadataError::PartitionGrainRequiresRefreshIncremental => {
                    Some((ts_err.to_string(), DiagnosticCode::YamlParseError))
                }
                smelt_core::metadata::MetadataError::KeyedForbidsSafetyOverrides => Some((
                    ts_err.to_string(),
                    DiagnosticCode::KeyedForbidsSafetyOverrides,
                )),
                smelt_core::metadata::MetadataError::MaterializedViewForbidsTimeseries => Some((
                    ts_err.to_string(),
                    DiagnosticCode::MaterializedViewForbidsTimeseries,
                )),
                smelt_core::metadata::MetadataError::MaterializedViewForbidsPartitionGrain => {
                    Some((
                        ts_err.to_string(),
                        DiagnosticCode::MaterializedViewForbidsPartitionGrain,
                    ))
                }
                smelt_core::metadata::MetadataError::GrainRequiredForIncremental => Some((
                    ts_err.to_string(),
                    DiagnosticCode::GrainRequiredForIncremental,
                )),
                smelt_core::metadata::MetadataError::GrainRequiresIncremental => {
                    Some((ts_err.to_string(), DiagnosticCode::GrainRequiresIncremental))
                }
                smelt_core::metadata::MetadataError::GrainAssertionMismatch { .. } => {
                    Some((ts_err.to_string(), DiagnosticCode::GrainAssertionMismatch))
                }
                // Other MetadataError variants are already handled by the generates-key
                // block above or by serde_yaml at parse time; skip them here.
                _ => None,
            };
            if let Some((message, code)) = maybe_diag {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message,
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(code),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Functional-dependency (`key -> determines`) declaration structural
        // validation (DC2, `model_properties.md` §"Model-scoped declarations").
        if let Err(fd_err) =
            smelt_core::metadata::validate_functional_dependencies(metadata, sql_body)
        {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: fd_err.to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::MalformedFunctionalDependency),
                data: None,
            })
            .accumulate(db);
        }

        // Bounded-domain / space-budget declaration structural validation
        // (DC3, `model_properties.md` §"Model-scoped declarations").
        if let Err(bd_err) = smelt_core::metadata::validate_bounded_domains(metadata, sql_body) {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: bd_err.to_string(),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::MalformedBoundedDomain),
                data: None,
            })
            .accumulate(db);
        }

        // The grain label the contract-lattice validators name in a refusal
        // message (`smelt_logical::contract::GrainLabel`) — declared, else
        // fact-derived (`resolved_grain()`), else the keyed-succession leaf
        // classifier's own verdict for an undeclared-grain model, falling
        // back to `GrainLabel::Key` (matching this check's pre-succession
        // default) when the model is neither. Computed once and reused by
        // the `frozen_horizon`/`retain_departed`/`deferral` checks below,
        // never re-derived per check.
        let grain_label = model_grain_label(db, workspace, project, metadata, sql_body);

        // Contract-lattice `frozen_horizon` grain-admissibility check
        // (`docs/specs/incremental_models.md` §"Contract relaxations
        // (`contract:`)"). Format validity was already checked at
        // frontmatter-parse time (`MetadataError::ContractFrozenHorizonInvalid`,
        // handled above); this pure `smelt-logical` validator only checks that
        // the declaration sits on a partition-grain model, sharing the same
        // diagnostic code (single-owner rule: the oracle/validator, not this
        // Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.frozen_horizon.is_some() {
                if let Err(why) = smelt_logical::validate_frozen_horizon(grain_label) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractFrozenHorizonInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }

        // Contract-lattice `frozen_horizon` driving-source posture check
        // (`docs/specs/incremental_models.md` §"The contract lattice":
        // declaring `frozen_horizon` on a model whose driving source has any
        // other *declared* mutation profile is refused, since the late-
        // arrival probe's row-count comparison is blind under any posture
        // other than `append_only`). Resolves the model's driving relation
        // from the FROM clause's first entry, the same parse pattern
        // `smelt_logical::maintenance::locality::resolve_driving_source`
        // uses, and shares the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.frozen_horizon.is_some() {
                let driving_source =
                    smelt_parser::File::cast(smelt_parser::parse(sql_body).syntax())
                        .and_then(|f| f.select_stmt())
                        .and_then(|s| s.from_clause())
                        .map(|fc| {
                            smelt_logical::analysis::source_bounds::from_clause_alias_sources(&fc)
                        })
                        .and_then(|sources| sources.into_iter().next());
                if let Some((_, source_name)) = driving_source {
                    let profile =
                        ref_source_info(db, workspace, project, &format!("smelt.{source_name}"))
                            .and_then(|info| info.mutation_profile.map(|m| m.kind));
                    if let Err(why) =
                        smelt_logical::validate_frozen_horizon_posture(&source_name, profile)
                    {
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!("ContractFrozenHorizonInvalid: {why}"),
                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                            code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }

        // Contract-lattice `deferral` clock-admissibility check
        // (`docs/specs/incremental_models.md` §"Contract relaxations
        // (`contract:`)"). Format validity was already checked at
        // frontmatter-parse time (`MetadataError::ContractDeferralInvalid`,
        // handled above); this check resolves whether the declaration has an
        // interval-representable clock to measure lag against — the model's
        // own `timeseries:` clock for a model-level `deferral`, or the
        // resolved source behind a `cells[].on` trigger for a cell-level
        // one — and shares the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if contract.deferral.is_some() {
                let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
                // A succession model always carries a clock — the
                // classifier-derived `clock_col`, never a declared
                // `timeseries:` block — so `deferral` is admitted with
                // unchanged frontier-lag semantics (2026-09-06 decision,
                // `docs/outcomes/20260906-scd2-keyed-succession/outcome.md`).
                let has_clock = metadata.timeseries.is_some()
                    || grain_label == smelt_logical::contract::GrainLabel::Succession;
                if let Err(why) =
                    smelt_logical::validate_deferral(has_clock, &format!("model '{model_name}'"))
                {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractDeferralInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractDeferralInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
            if contract.cells.iter().any(|c| c.deferral.is_some()) {
                let refs = smelt_logical::collect_path_refs(sql_body);
                for cell in contract.cells.iter().filter(|c| c.deferral.is_some()) {
                    let has_clock = cell.on != "backfill"
                        && refs
                            .iter()
                            .filter_map(|r| {
                                let stripped = r.strip_prefix("smelt.")?;
                                let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
                                if bare != cell.on {
                                    return None;
                                }
                                ref_source_info(db, workspace, project, r)
                            })
                            .next()
                            .is_some_and(|info| {
                                info.timeseries.is_some()
                                    && info.mutation_profile.as_ref().is_some_and(|m| {
                                        m.kind == smelt_core::sources::MutationProfile::AppendOnly
                                    })
                            });
                    if let Err(why) = smelt_logical::validate_deferral(
                        has_clock,
                        &format!("cell on '{}'", cell.on),
                    ) {
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!("ContractDeferralInvalid: {why}"),
                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                            code: Some(DiagnosticCode::ContractDeferralInvalid),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }

        // Contract-lattice `retain_departed` posture-admissibility +
        // tombstone-column check (`docs/specs/incremental_models.md`
        // §"Contract relaxations (`contract:`)"). Format validity was
        // already checked at frontmatter-parse time
        // (`MetadataError::ContractRetainDepartedInvalid`, handled above);
        // this pure `smelt-logical` validator checks that the declaration
        // sits on a keyed shape consuming a mutable snapshot, and that a
        // declared tombstone column exists in the model's inferred output —
        // sharing the same diagnostic code (single-owner rule: the
        // oracle/validator, not this Salsa wrapper, decides admissibility).
        if let Some(contract) = &metadata.contract {
            if let Some(retain_departed) = &contract.retain_departed {
                let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
                let refs = smelt_logical::collect_path_refs(sql_body);
                let consumes_mutable_snapshot = refs.iter().any(|r| {
                    ref_source_info(db, workspace, project, r).is_some_and(|info| {
                        info.mutation_profile.as_ref().is_some_and(|m| {
                            m.kind == smelt_core::sources::MutationProfile::Mutable
                        })
                    })
                });
                let tombstone_column = match retain_departed {
                    smelt_core::config::RetainDeparted::Bool(_) => None,
                    smelt_core::config::RetainDeparted::Tombstone { tombstone } => {
                        Some(tombstone.as_str())
                    }
                };
                let typed_schema = typed_model_schema(db, workspace, file);
                let output_columns: Vec<String> = typed_schema
                    .columns
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                if let Err(why) = smelt_logical::validate_retain_departed(
                    grain_label,
                    consumes_mutable_snapshot,
                    tombstone_column,
                    &output_columns,
                    model_name,
                ) {
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("ContractRetainDepartedInvalid: {why}"),
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::ContractRetainDepartedInvalid),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }

        // Declarative column test validation (`docs/specs/data_tests.md`
        // §"Fail-loud validation"). Two checks, run only when at least one
        // column declares a non-empty `tests` list:
        //   1. `UnknownColumnTestKind` — pure, from `validate_column_tests`.
        //   2. `ColumnTestOnUnknownColumn` — needs the inferred output
        //      schema, so it is made here (not in `smelt-core`) via
        //      `typed_model_schema`.
        if metadata.columns.values().any(|c| !c.tests.is_empty()) {
            if let Err(kind_err) = smelt_core::metadata::validate_column_tests(metadata) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: kind_err.to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::UnknownColumnTestKind),
                    data: None,
                })
                .accumulate(db);
            }

            let model_name = metadata.name.as_deref().unwrap_or("<unnamed>");
            let typed_schema = typed_model_schema(db, workspace, file);
            let schema_columns: Vec<String> = typed_schema
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect();
            if let Err(col_err) = smelt_core::metadata::validate_column_tests_against_schema(
                metadata,
                model_name,
                &schema_columns,
            ) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: col_err.to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::ColumnTestOnUnknownColumn),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Timeseries schema invariants (D-52 rules 7 and 8).
        if let Some(ts) = metadata.timeseries.as_ref() {
            let typed_schema = typed_model_schema(db, workspace, file);
            // Rule 7: partition_column and event_time_column must be NOT NULL.
            for diag in queries::check_types::check_timeseries_nullability(ts, &typed_schema) {
                DiagnosticAcc(diag).accumulate(db);
            }
            // Rule 8: sub-day granularity (hour) requires a timestamp-resolution
            // partition_column type (not DATE).
            for diag in queries::check_types::check_timeseries_granularity_type(ts, &typed_schema) {
                DiagnosticAcc(diag).accumulate(db);
            }
        }

        // State posture widening check (D-47): a model may narrow the project's
        // state.mode but not widen it.
        if let Some(model_state) = metadata.state.as_ref() {
            let project_mode = project
                .map(|p| crate::queries::project::project_state_mode(db, p))
                .unwrap_or_default();
            if !project_mode.can_narrow_to(&model_state.mode) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "model declares state.mode {} but project posture is {}; \
                         models may narrow but not widen the project posture",
                        model_state.mode.as_str(),
                        project_mode.as_str(),
                    ),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::StateModeWidening),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Built-in planner-rule diagnostics (keyed classifier, incremental
        // batch-safety) surfaced through the uniform rule → diagnostics
        // interface. The checks live in `smelt-planner` (analysis-pure); this
        // query only gathers inputs and aggregates, so the editor and the build
        // reach an identical verdict (architecture.md §"Diagnostic parity rule"
        // + §"Planner scope"). Anchored at the model SQL body start.
        // Route keyed detection through is_keyed() (`refresh: incremental` +
        // `grain: key`) and partition-grain detection through
        // is_partition_grain() (`refresh: incremental` + `grain: partition`
        // — the opt-in, independent of whether the optional `batched:` block
        // is present) so both reach the classifier. The strings below are the
        // classifier's internal keys for each rule, not user surface values.
        let materialization = if metadata.is_keyed() {
            "cumulative_aggregate"
        } else if metadata.is_partition_grain() {
            "incremental"
        } else {
            ""
        };
        if !materialization.is_empty() {
            let stripped = smelt_parser::strip_frontmatter(text);
            let refs = smelt_logical::collect_path_refs(&stripped);
            // The keyed classifier resolves its driving source by looking
            // each ref up in this map. The incremental rule's UNION-ALL
            // injectability check (`rule_diagnostics::check_union_all_injectable`)
            // also needs it — it builds the same per-ref `BoundContext` the
            // pushdown-scoping walk (`rules::incremental::derive_model_source_bounds`)
            // builds from `RuleContext.refs`/`source_timeseries`, so both rules
            // populate this map for every ref regardless of materialization.
            let mut source_timeseries: smelt_logical::SourceTimeseriesMap = HashMap::new();
            for r in &refs {
                if let Some(ts) = ref_timeseries_config(db, workspace, project, r) {
                    source_timeseries.insert(r.clone(), ts);
                }
            }
            let model_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // The opt-in is `refresh: batched`, not the presence of the optional
            // `batched:` block — default to an empty config when the block is
            // absent so a bare `refresh: batched` model still reaches the rule.
            let default_batched_config = smelt_core::config::PartitionGrainConfig::default();
            let plausible_columns: std::collections::BTreeSet<String> = metadata
                .columns
                .iter()
                .filter(|(_, c)| c.contract == Some(smelt_core::metadata::Contract::Plausible))
                .map(|(name, _)| name.clone())
                .collect();
            let ctx = smelt_logical::RuleContext {
                model_name: &model_name,
                materialization,
                sql: &stripped,
                refs: &refs,
                source_timeseries: &source_timeseries,
                timeseries_config: metadata.timeseries.as_ref(),
                incremental_config: if materialization == "incremental" {
                    Some(metadata.batched.as_ref().unwrap_or(&default_batched_config))
                } else {
                    None
                },
                declared_functional_dependencies: &metadata.functional_dependencies,
                plausible_columns: &plausible_columns,
            };
            let body_start = rowan::TextSize::from(sql_offset as u32);
            for rd in smelt_logical::detect_builtin_rules(&ctx) {
                DiagnosticAcc(Diagnostic {
                    severity: match rd.severity {
                        smelt_logical::RuleSeverity::Error => DiagnosticSeverity::Error,
                        smelt_logical::RuleSeverity::Warning => DiagnosticSeverity::Warning,
                    },
                    message: rd.message,
                    range: rowan::TextRange::empty(body_start),
                    code: Some(rule_diagnostic_code(rd.code)),
                    data: None,
                })
                .accumulate(db);
            }
        }

        // Maintenance-plan diagnostics (`incremental_models.md` §Diagnostics):
        // fold the derived plan's admission refusals and the
        // `maintenance.cells[]` column-group-span check onto the
        // `Maintenance*` codes. `maintenance_plan` is the thin Salsa query —
        // this block only maps its (already-derived) result onto
        // diagnostics, never re-derives the plan itself.
        let plan_diags = maintenance_plan(db, workspace, file);
        let body_start = rowan::TextSize::from(sql_offset as u32);
        for refusal in &plan_diags.refusals {
            // Single owner of the refusal → diagnostic mapping (ruling R2,
            // F2): `crate::queries::maintenance::diagnostic_for_refusal` is
            // also what the `refusal_codes` agreement gate drives its
            // assertions from, so the two can never drift.
            let Some((severity, code, message)) =
                crate::queries::maintenance::diagnostic_for_refusal(refusal)
            else {
                continue;
            };
            DiagnosticAcc(Diagnostic {
                severity,
                message,
                range: rowan::TextRange::empty(body_start),
                code: Some(code),
                data: None,
            })
            .accumulate(db);
        }
        for violation in &plan_diags.cell_column_group_violations {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: violation.clone(),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique),
                data: None,
            })
            .accumulate(db);
        }
        for source in &plan_diags.scan_bounds_warnings {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "maintenance scan over '{source}' cannot be partition-bounded — admitted \
                     under scan_bounds.on_violation: warn"
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceScanUnbounded),
                data: None,
            })
            .accumulate(db);
        }
        for write_refusal in &plan_diags.write_pin_refusals {
            let (code, message) = match write_refusal {
                crate::queries::maintenance::WritePinDiagnostic::PatternUnavailable {
                    pattern,
                    backend,
                } => (
                    DiagnosticCode::MaintenanceWritePatternUnavailable,
                    format!(
                        "MaintenanceWritePatternUnavailable: write pattern '{pattern}' is \
                         unrecognised, or backend '{backend}' cannot provide it"
                    ),
                ),
                crate::queries::maintenance::WritePinDiagnostic::AddressingRefused {
                    cell,
                    pattern,
                    why,
                } => (
                    DiagnosticCode::MaintenanceWriteAddressingRefused,
                    format!(
                        "MaintenanceWriteAddressingRefused: write pattern '{pattern}' cannot \
                         uphold the equivalence invariant for cell {cell} — {why}"
                    ),
                ),
            };
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message,
                range: rowan::TextRange::empty(body_start),
                code: Some(code),
                data: None,
            })
            .accumulate(db);
        }
        for downgrade in &plan_diags.state_downgrades {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "MaintenanceStateDowngraded: cell {} downgraded from {} to its \
                     recompute-family equivalent — {}",
                    downgrade.cell, downgrade.original_technique, downgrade.reason
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceStateDowngraded),
                data: None,
            })
            .accumulate(db);
        }
        for refusal in &plan_diags.contract_state_refusals {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "DeclaredContractRequiresState: {} requires the {}, which is unavailable \
                     on backend '{}'",
                    refusal.declaration, refusal.missing_structure, refusal.backend
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::DeclaredContractRequiresState),
                data: None,
            })
            .accumulate(db);
        }
        if let Some(mismatch) = &plan_diags.granularity_mismatch {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "declared timeseries.granularity ({}) is contradicted by the model's own \
                     partition-column grouping, which derives to {}",
                    granularity_lower(mismatch.declared),
                    granularity_lower(mismatch.actual),
                ),
                range: rowan::TextRange::empty(body_start),
                code: Some(DiagnosticCode::MaintenanceGranularityMismatch),
                data: None,
            })
            .accumulate(db);
        }
    }

    // Parse errors
    let parse = parse_file(db, file);
    for error in parse.errors.iter() {
        let range = error.range;
        // Remap pipe-operator parse errors to their proper diagnostic codes so
        // consumers can distinguish them from generic syntax errors.
        let code = remap_pipe_parse_error_code(&error.message);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.message.clone(),
            range,
            code: Some(code),
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

    // Struct-field-unknown diagnostics (hardening Phase 3): emitted at each
    // struct field whose type text is not a recognised concrete DataType.
    for diag in struct_field_type_unknown_diagnostics_for_file(db, file) {
        DiagnosticAcc(diag).accumulate(db);
    }

    // BUG-003: Semantics #9 — default must not reference sibling parameters.
    for diag in default_references_parameter_diagnostics_for_file(db, file) {
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

    // BUG-007: CTE-collision diagnostics — emitted when a model's top-level
    // CTE name collides with a CTE declared in the body of a directly-called
    // transparent function (CteShadowsCallerCte, Error).
    for diag in cte_shadow_caller_cte_diagnostics_for_file(db, workspace, file) {
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
        for diag in type_inference::check_config_var_call_diagnostics(&syntax, &vars_map) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // Phase E1 Phase 5: smelt.config.load_yaml / load_json / load_toml call diagnostics.
    //
    // Validates path literals, schema arguments, and file existence.
    // Runs unconditionally (before the early return on parse failure) so that
    // config-var files also surface loader diagnostics.
    {
        for diag in loader_call_diagnostics_for_file(db, workspace, file) {
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
                for diag in type_inference::check_define_name_shadowing(&define) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }
    }

    // Phase E2 — ModelDefOutsideGeneratorFile: scan for ModelDef record literals
    // in non-generator files. A `ModelDef { name: '…', body: … }` construct is
    // only valid inside a generator file (generates: models); using it in a
    // regular SQL model file is an error.
    {
        use smelt_parser::ast::RecordLiteral;
        use smelt_parser::SyntaxKind::{IDENT, RECORD_LITERAL};
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        let mut ctx = type_inference::TypeContext::new();
        ctx.is_inside_generator_file = false; // non-generator file
        for node in syntax.descendants().filter(|n| n.kind() == RECORD_LITERAL) {
            if let Some(lit) = RecordLiteral::cast(node) {
                // Only check record literals whose leading token is the identifier
                // "ModelDef". In the CST, a named record literal `TypeName { … }`
                // has the type-name IDENT as its first token.
                let leading_name = lit
                    .syntax()
                    .children_with_tokens()
                    .find_map(|e| {
                        let tok = e.into_token()?;
                        if tok.kind() == IDENT {
                            Some(tok.text().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                if leading_name != "ModelDef" {
                    continue;
                }
                let result = type_inference::infer_model_def_literal(&lit, &ctx);
                for sentinel in result.sentinels {
                    if sentinel.code == DiagnosticCode::ModelDefOutsideGeneratorFile {
                        let range = sentinel.span;
                        DiagnosticAcc(Diagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: sentinel.message,
                            range,
                            code: Some(sentinel.code),
                            data: None,
                        })
                        .accumulate(db);
                    }
                }
            }
        }
    }

    // VALUES / CTE alias-column arity checks.
    //
    // Walk all TABLE_REF and CTE nodes in the file's CST and emit:
    //   - `AliasColumnArityMismatch` when the alias column list length does not
    //     match the underlying relation's column count.
    //   - `EmptyValuesClause` when a VALUES derived table has zero rows.
    //
    // These are pure structural checks that do not require schema resolution.
    // They run unconditionally (before the `parse_model.is_none()` early-return)
    // so that function files also surface them.
    {
        use smelt_parser::ast::{Cte, TableRef};
        use smelt_parser::SyntaxKind::{CTE, TABLE_REF};
        let parse = parse_file(db, file);
        let syntax = parse.syntax();

        // VALUES derived-table checks: scan all TABLE_REF nodes.
        for node in syntax.descendants().filter(|n| n.kind() == TABLE_REF) {
            if let Some(tr) = TableRef::cast(node) {
                for diag in type_inference::check_table_ref_values_arity(&tr) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }

        // CTE alias-list checks: scan all CTE nodes.
        for node in syntax.descendants().filter(|n| n.kind() == CTE) {
            if let Some(cte) = Cte::cast(node) {
                for diag in type_inference::check_cte_alias_arity(&cte) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }
        }
    }

    // Phase 4 (testing): `#` CTE-reference outside smelt.test.
    //
    // A `smelt.<path>#<cte>` reference is only valid inside a `smelt.test`
    // body.  Walk all SMELT_PATH_REF nodes in the CST and emit
    // `CteRefOutsideTest` for any that carry a `#` suffix but are not
    // inside a SMELT_TEST ancestor.  This is a pure structural check that
    // runs unconditionally (no early-return) so model files, function files,
    // check files, and test files all surface it correctly.
    {
        let parse = parse_file(db, file);
        let syntax = parse.syntax();
        for diag in cte_ref_outside_test_diagnostics(&syntax) {
            DiagnosticAcc(diag).accumulate(db);
        }
    }

    // `smelt.check` structural validation: PASSING and EXPECT clauses are
    // test-only surface. A check body is a failing-rows query against real
    // built data; it has no mock tables and no expected output rows. Emit
    // `CheckHasTestClause` anchored at the offending clause keyword range.
    {
        let parse = parse_file(db, file);
        if let Some(ast) = AstFile::cast(parse.syntax()) {
            for check in ast.checks() {
                for passing in check.passing_clauses() {
                    let range = passing.syntax().text_range();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "PASSING clauses are not valid on smelt.check — \
                                  only smelt.test declarations accept mock table data"
                            .to_string(),
                        range,
                        code: Some(DiagnosticCode::CheckHasTestClause),
                        data: None,
                    })
                    .accumulate(db);
                }
                if let Some(expect) = check.expect_clause() {
                    let range = expect.syntax().text_range();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "EXPECT clause is not valid on smelt.check — \
                                  only smelt.test declarations assert against expected output rows"
                            .to_string(),
                        range,
                        code: Some(DiagnosticCode::CheckHasTestClause),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Check if model is valid
    if parse_model(db, file).is_none() {
        let path_str = path.to_str().unwrap_or("");
        let is_virtual_submodel = path_str.contains("::");
        if !is_virtual_submodel && path_str.contains("models/") {
            // Files that contain only `smelt.test` or `smelt.check` declarations are
            // valid — they have no SELECT body but they are not broken models.
            // Suppress the "does not contain a valid SQL query" warning for such files.
            let parse = parse_file(db, file);
            let has_smelt_tests_or_checks = AstFile::cast(parse.syntax())
                .map(|ast| ast.tests().next().is_some() || ast.checks().next().is_some())
                .unwrap_or(false);

            if !has_smelt_tests_or_checks {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: "File does not contain a valid SQL query".to_string(),
                    range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                    code: Some(DiagnosticCode::InvalidModel),
                    data: None,
                })
                .accumulate(db);
            }
        }
        return;
    }

    // Unified path-form refs. Resolve through the path-tuple
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
                if resolved.kind == RefKind::Check && path_ref_loc.in_table_expr_position {
                    let leaf = path_ref_loc.path.last().cloned().unwrap_or_default();
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Cannot reference check '{leaf}' in a FROM position — \
                             smelt.check files produce no DB object and cannot be used as TableExpr values"
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
                    // Compute a "did you mean" hint by scanning for models
                    // whose leaf segment matches the last segment of the
                    // unresolved path. This helps users find the full canonical
                    // address when they used only a leaf or partial path.
                    let leaf = path_ref_loc.path.last().map(|s| s.as_str()).unwrap_or("");
                    let hint = if !leaf.is_empty() {
                        let candidates = leaf_did_you_mean(db, workspace, project, leaf);
                        match candidates.as_slice() {
                            [] => String::new(),
                            [single] => format!(" did you mean '{single}'?"),
                            many => {
                                let list = many
                                    .iter()
                                    .map(|s| format!("'{s}'"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(" did you mean one of {list}?")
                            }
                        }
                    } else {
                        String::new()
                    };
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: format!("Undefined ref: {path_str}{hint}"),
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

    // BUG-078: checked whenever the project carries aggregate `sources.yml`
    // text — NOT gated on `sources` (legacy `smelt.source()` call sites, which
    // are always empty since the per-entity migration made `smelt.source()` a
    // parse error). Gating here made a YAML-broken aggregate file silently
    // fall back to `SourcesConfig::default()` with no diagnostic.
    if let Some(p) = project {
        if let Some(yaml_error) = sources_yaml_error(db, p) {
            DiagnosticAcc(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("sources.yml parse error: {}", yaml_error.message),
                range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                code: Some(DiagnosticCode::YamlParseError),
                data: None,
            })
            .accumulate(db);
        }
    }

    if !sources.is_empty() {
        if let Some(p) = project {
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
                        range: rowan::TextRange::empty(rowan::TextSize::from(0)),
                        code: Some(DiagnosticCode::SourceTypeError),
                        data: None,
                    })
                    .accumulate(db);
                }
            }
        }
    }

    // Unsupported constructs + malformed sources + CAST / unknown fn / ambiguous column
    queries::check_types::check_unsupported_constructs(&parse.syntax(), db);

    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        // Phase 4: smelt.source() is a parse error so there are no SourceCall
        // nodes to validate. The malformed-source check is superseded by the
        // parser rejection.

        if let Some(select_stmt) = ast.select_stmt() {
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    if let Some(expr) = item.expression() {
                        queries::check_types::check_expression_types(&expr, db);
                    }
                    // The `_smelt_` alias prefix is reserved for smelt's own
                    // generated identifiers (`multi_backend.md` §"Output-schema
                    // type conformance") — most visibly the synthesized
                    // `_smelt_col{n}` alias bound to a nameless projection
                    // item. Emitted here (the analyzer) rather than only at
                    // build time so the LSP and the CLI build path agree
                    // (`architecture.md` §"Diagnostic parity rule").
                    if let Some(alias) = item.alias() {
                        if alias.starts_with("_smelt_") {
                            let range = item.alias_range().unwrap_or_else(|| item.range());
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: format!(
                                    "column alias `{alias}` uses the reserved `_smelt_` prefix; \
                                     smelt uses this prefix for its own generated identifiers \
                                     (e.g. the synthesized name for an unaliased expression \
                                     column) — choose a different alias"
                                ),
                                range,
                                code: Some(DiagnosticCode::ReservedProjectionAliasPrefix),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase 14 (§16 #24): reject window-kind expressions in WHERE
            // and GROUP BY positions. Kind synthesis is independent of any
            // column-schema lookups (column refs are always Scalar), so
            // the check runs on a fresh empty `TypeContext`.
            let kind_ctx = type_inference::TypeContext::new();
            for info in type_inference::check_window_in_scalar_contexts(&select_stmt, &kind_ctx) {
                let range = info.range;
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
            // Ranges of meta diagnostics already emitted for this select
            // statement. A `List<T>`-in-scalar-position check (below) is
            // suppressed for any select item that already carries another meta
            // error (drop-on-error: a single malformed item does not avalanche).
            let mut flagged_meta_ranges: Vec<rowan::TextRange> = Vec::new();

            let spread_result = type_inference::check_select_list_spreads(&select_stmt, &kind_ctx);
            for diag in spread_result.diagnostics {
                flagged_meta_ranges.push(diag.range);
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
                                &elements, &kind_ctx, span,
                            ) {
                                flagged_meta_ranges.push(diag.range);
                                DiagnosticAcc(diag).accumulate(db);
                            }
                        }
                    }
                }
            }

            let forbidden_diags =
                type_inference::check_forbidden_position_spreads(&select_stmt, &kind_ctx);
            for diag in forbidden_diags {
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase B (meta-language) Phase 3: HOF + lambda + pipe diagnostics.
            //
            // Walks every LAMBDA, FUNCTION_CALL (HOF), and PIPE_EXPR descendant.
            // Covers: LambdaInForbiddenPosition, LambdaArityMismatch, LambdaZeroParameters,
            //   LambdaDuplicateParameter, LambdaResultTypeMismatch, HofExpectsLambda,
            //   HofExpectsReducer, PipeRhsNotCall, PipeInDataPosition,
            //   ReducerInputTypeMismatch, ReducerEmptyNoIdentity.
            // Also covers Phase F REDUCER_CALL nodes (parameterised reducers):
            //   ReducerArityMismatch, ReducerArgTypeMismatch, ReducerArgNotCompileTime,
            //   ReducerNamedArgument.
            // Uses an empty TypeContext (consistent with spread/window checks above).
            let hof_diags =
                type_inference::check_hof_position_diagnostics(&select_stmt, &kind_ctx, text);
            for diag in hof_diags {
                flagged_meta_ranges.push(diag.range);
                DiagnosticAcc(diag).accumulate(db);
            }

            // Phase F (meta-language) — Ternary expression diagnostics.
            //
            // Walks every TERNARY_EXPR descendant and bare THEN_KW tokens.
            // Covers: TernaryConditionNotBoolean, TernaryBranchTypeMismatch,
            //   TernaryDanglingElse, TernaryDanglingThen.
            // Uses an empty TypeContext (consistent with HOF checks above).
            {
                let ternary_diags =
                    type_inference::check_ternary_expr_diagnostics(&select_stmt, &kind_ctx);
                for diag in ternary_diags {
                    flagged_meta_ranges.push(diag.range);
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // BUG-017: cross-family binary arithmetic → TypeMismatch.
            //
            // Walks every BINARY_EXPR and emits exactly one TypeMismatch Error
            // at the operator span when a numeric/string/boolean/temporal
            // cross-family pair is detected (spec §1 and §14).
            // Uses an empty TypeContext — literal operands (`42 + '3'`)
            // resolve without column context; column-typed operands resolve
            // if a full ctx is available later in check_type_diagnostics.
            {
                let xfamily_diags = type_inference::check_crossfamily_arithmetic_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in xfamily_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — decimal precision overflow → `DecimalPrecisionOverflow`.
            //
            // Walks every `+`, `-`, `*`, `%` BINARY_EXPR and emits exactly one
            // `DecimalPrecisionOverflow` Error at the operator span when the
            // Spark-style growth formula yields `p' > 38`. Division is excluded
            // (handled below). The result type in such expressions is already
            // `DataType::Unknown` as computed by `promote_numeric_operands_for_op`.
            {
                let overflow_diags = type_inference::check_decimal_precision_overflow_diagnostics(
                    &select_stmt,
                    &kind_ctx,
                );
                for diag in overflow_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §15 — division rejection → `TypeMismatch`.
            //
            // `Decimal / T` for any numeric `T` is not in the portable surface.
            // Emits one `TypeMismatch` Error at the `/` operator span directing
            // the user to cast to Double. The inferred result type is already
            // `DataType::Unknown` (set by `promote_numeric_operands_for_op`).
            {
                let div_diags =
                    type_inference::check_decimal_division_diagnostics(&select_stmt, &kind_ctx);
                for diag in div_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §17 — non-portable collation → `NonPortableCollation`.
            //
            // Walks every COLLATE_EXPR in the SELECT statement. For any
            // non-binary collation name the diagnostic fires at the COLLATE
            // clause span and the expression type degrades to Unknown
            // (handled in `infer_expression_type` via
            // `infer_collate_expr_type`). Binary collations (COLLATE "C",
            // COLLATE BINARY, COLLATE UTF8_BINARY, COLLATE POSIX) are
            // silent no-ops.
            {
                let collation_diags =
                    type_inference::check_collation_diagnostics(&select_stmt, &kind_ctx);
                for diag in collation_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Spec §16 — mixed naive/tz-aware Timestamp in set operations, CASE
            // branches, and arithmetic → TypeMismatch.
            //
            // These three checks need the full per-file TypeContext (column types
            // from upstream models) so that column references such as `ts_col` and
            // `tstz_col` resolve to their inferred DataType. They cannot run on the
            // empty `kind_ctx` used for shape checks above. `type_context` is a
            // Salsa query that builds the column-schema context for this file; it
            // is safe to call from within a Salsa tracked function.
            //
            // Only run for model files that have at least one data reference
            // (the model_path filter is already satisfied by the outer `if let
            // Some(select_stmt)` guard and the `models/` path check earlier).
            {
                let tz_ctx = type_context(db, workspace, file);

                // Set-operations (UNION/INTERSECT/EXCEPT)
                let setop_diags =
                    type_inference::check_mixed_tz_setop_diagnostics(&select_stmt, &tz_ctx);
                for diag in setop_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // CASE branches
                let case_diags =
                    type_inference::check_mixed_tz_case_diagnostics(&select_stmt, &tz_ctx);
                for diag in case_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // Arithmetic operators (-, +, *, /, %)
                let mixed_tz_arith_diags =
                    type_inference::check_mixed_tz_arithmetic_diagnostics(&select_stmt, &tz_ctx);
                for diag in mixed_tz_arith_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }

                // VALUES-clause columns (§16 strict temporal mixing rule)
                let values_temporal_diags =
                    type_inference::check_mixed_temporal_values_diagnostics(&select_stmt, &tz_ctx);
                for diag in values_temporal_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Meta-language (P6) — `MetaListInScalarPosition`.
            //
            // A `List<T>`-typed expression that reaches a Data-World scalar /
            // SELECT-item position without being consumed (by a spread, a HOF,
            // a reducer, a record, a map, or a generator) cannot materialise as
            // a scalar value — there is no implicit auto-spread
            // (`meta_language.md` §Semantics "Lists and spread" rule 10). A bare
            // list literal (`SELECT [1, 2, 3]`), or a bare `map`/`filter` /
            // pipe-to-`map`/`filter` result (`SELECT xs |> map(fn c => …)`),
            // left in a select item is unconsumed. `reduce` collapses a list to
            // a scalar, so a `reduce(...)` select item is consumed and clean.
            //
            // This is a select-shape check that runs for every model, including
            // a model with no FROM clause — `check_type_diagnostics`
            // early-returns when a model has no data refs, so the check lives
            // here (the meta walk runs regardless of FROM). Suppressed for any
            // item already carrying another meta diagnostic (drop-on-error).
            if let Some(select_list) = select_stmt.select_list() {
                for item in select_list.items() {
                    let Some(expr) = item.expression() else {
                        continue;
                    };
                    if !select_item_yields_bare_list(&expr) {
                        continue;
                    }
                    let span = expr.syntax().text_range();
                    if flagged_meta_ranges
                        .iter()
                        .any(|r| r.intersect(span).is_some())
                    {
                        continue;
                    }
                    DiagnosticAcc(Diagnostic {
                        severity: DiagnosticSeverity::Error,
                        message: "a List<T> cannot be used as a scalar value here; consume it \
                                  with a spread (`...xs`), a reducer (`reduce(xs, …)`), or a HOF \
                                  before splicing"
                            .to_string(),
                        range: span,
                        code: Some(DiagnosticCode::MetaListInScalarPosition),
                        data: None,
                    })
                    .accumulate(db);
                }
            }

            // Phase C (meta-language) — smelt.columns_of diagnostic wiring.
            //
            // Walks every SMELT_PATH_CALL for `smelt.columns_of(...)` in the
            // select statement. Emits:
            //   - ColumnsOfNamedArgument: named argument passed to columns_of
            //   - ColumnsOfRequiresTableExpr: non-TableExpr positional arg
            // Uses the same empty TypeContext as HOF checks (no column schema
            // available at this stage in the orchestrator).
            {
                let cols_of_diags =
                    type_inference::check_columns_of_diagnostics(&select_stmt, &kind_ctx);
                for diag in cols_of_diags {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase C (meta-language) — ColumnsOfUnresolvableSchema wiring.
            //
            // For each `smelt.columns_of(smelt.models.<name>)` (or
            // `smelt.columns_of(<name>)` where `<name>` is a bare identifier that
            // resolves via the workspace) call in the select statement, attempt to
            // resolve the model schema via `columns_of_for_table_expr`. When the
            // schema cannot be resolved (the model does not exist or has an unknown
            // schema), emit exactly one `ColumnsOfUnresolvableSchema` diagnostic
            // anchored at the full `smelt.columns_of(...)` call span.
            //
            // This implements the drop-on-error recovery policy (same as
            // `MetaSpreadInForbiddenPosition`): the call-site gets one diagnostic
            // and no cascading errors from the surrounding expression.
            {
                use smelt_parser::ast::SmeltPathCall;
                use smelt_parser::SyntaxKind::SMELT_PATH_CALL;
                for node in select_stmt.syntax().descendants() {
                    if node.kind() != SMELT_PATH_CALL {
                        continue;
                    }
                    let call = match SmeltPathCall::cast(node.clone()) {
                        Some(c) => c,
                        None => continue,
                    };
                    let segs = call.segments();
                    if segs.len() != 1 || segs[0].to_lowercase() != "columns_of" {
                        continue;
                    }
                    let arg_list = match call.arg_list() {
                        Some(al) => al,
                        None => continue,
                    };
                    // Only check positional args (named args are caught by
                    // ColumnsOfNamedArgument above).
                    for pos_arg in arg_list.positional_args() {
                        // Extract the model name from the positional argument:
                        // - smelt path ref: e.g. `smelt.models.orders` → last segment
                        // - bare identifier: e.g. `orders`
                        let model_name: Option<String> = {
                            // Try smelt path ref child.
                            let path_ref_name = pos_arg
                                .syntax()
                                .children()
                                .find_map(smelt_parser::ast::SmeltPathRef::cast)
                                .and_then(|r| r.segments().last().cloned());
                            if let Some(n) = path_ref_name {
                                Some(n)
                            } else {
                                // Try direct SmeltPathRef cast.
                                smelt_parser::ast::SmeltPathRef::cast(pos_arg.syntax().clone())
                                    .and_then(|r| r.segments().last().cloned())
                                    .or_else(|| {
                                        // Bare identifier: must start with a letter or
                                        // underscore (not a numeric literal like `42`).
                                        let arg_text = pos_arg.text().trim().to_string();
                                        let is_bare = !arg_text.is_empty()
                                            && arg_text
                                                .chars()
                                                .next()
                                                .is_some_and(|c| c.is_alphabetic() || c == '_')
                                            && arg_text
                                                .chars()
                                                .all(|c| c.is_alphanumeric() || c == '_');
                                        if is_bare {
                                            Some(arg_text)
                                        } else {
                                            None
                                        }
                                    })
                            }
                        };
                        let model_name = match model_name {
                            Some(n) => n,
                            None => continue,
                        };
                        let resolves = project
                            .map(|p| {
                                columns_of_for_table_expr(db, workspace, p, model_name.clone())
                                    .is_ok()
                            })
                            .unwrap_or(false);
                        if !resolves {
                            let call_range = node.text_range();
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Error,
                                message: meta_reflection_diagnostic_message_with_table_expr(
                                    DiagnosticCode::ColumnsOfUnresolvableSchema,
                                    None,
                                    None,
                                    Some(&model_name),
                                ),
                                range: call_range,
                                code: Some(DiagnosticCode::ColumnsOfUnresolvableSchema),
                                data: None,
                            })
                            .accumulate(db);
                        }
                    }
                }
            }

            // Phase C (meta-language) — ColumnRefFieldUnknown HOF dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is
            // `smelt.columns_of(…)`, walk the lambda body and emit
            // `ColumnRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed ColumnRef field set
            // `{name, type, is_numeric}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file`.
            {
                for diag in
                    function_body_check::check_hof_column_ref_field_diagnostics(&select_stmt)
                {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — wide-reflection accessor diagnostics.
            //
            // Walks every SMELT_PATH_CALL for `smelt.models.*` / `smelt.sources.*`
            // in the model SELECT statement.  Emits:
            //   - WideReflectionUnknownAccessor: unknown accessor name
            //   - WideReflectionUnexpectedArgument: argument to `all`
            //   - WithTagRequiresText: non-compile-time-Text argument to `with_tag`
            //   - WithTagNamedArgument: named argument to `with_tag`
            //
            // Uses an empty TypeContext (no ModelRef/SourceRef bindings exist at
            // the top-level model SELECT scope).
            {
                let phase_d_ctx = type_inference::TypeContext::new();
                for diag in type_inference::check_wide_reflection_diagnostics(
                    &select_stmt,
                    &phase_d_ctx,
                    text,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
                }
            }

            // Phase D (meta-language) — ModelRef / SourceRef HOF field dispatcher.
            //
            // For each `map`/`filter` HOF call whose first argument is a
            // `smelt.models.*` / `smelt.sources.*` wide-reflection call, walk
            // the lambda body and emit `ModelRefFieldUnknown` /
            // `SourceRefFieldUnknown` for any `<param>.<field>` access where
            // `<field>` is not in the closed field set `{path, name, tags, columns}`.
            //
            // This runs on MODEL select statements (the outer `select_stmt`).
            // Function-file SELECT bodies are handled separately in
            // `function_body_diagnostics_for_file` via `check_function_select_body`.
            {
                for diag in function_body_check::check_hof_model_ref_source_ref_field_diagnostics(
                    &select_stmt,
                ) {
                    DiagnosticAcc(diag).accumulate(db);
                }
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
                                            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
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

        // Phase F (meta-language) — File-level dangling THEN_KW detection.
        //
        // The parser's error recovery may eject a bare `then` keyword to the
        // top-level FILE node when it appears in an unexpected expression
        // position (e.g. `SELECT then x FROM t`).  `check_ternary_expr_diagnostics`
        // walks only the SelectStmt subtree and cannot reach FILE-level tokens.
        // This block walks the FULL file syntax so dangling THEN_KW tokens are
        // always caught, regardless of where error recovery placed them.
        //
        // Emits: TernaryDanglingThen.
        {
            let file_syntax = ast.syntax().clone();
            for diag in type_inference::check_dangling_ternary_keywords(&file_syntax) {
                DiagnosticAcc(diag).accumulate(db);
            }
        }
    }
}

fn count_from_sources(select_stmt: &smelt_parser::ast::SelectStmt) -> usize {
    let mut count = 0;
    if let Some(from_clause) = select_stmt.from_clause() {
        count += from_clause.table_refs().count();
        count += from_clause.joins().count();
    }
    count
}
