//! Definition-delta detection — the single derivation `smelt run`'s
//! pending-migration gate, `smelt explain`, and `smelt migrate` all read
//! (`docs/specs/definition_deltas.md` §Detection). Moved out of
//! `smelt-cli::commands::migrate` so the run gate does not re-derive a
//! second copy of the same diff → classify → hash pipeline
//! (`docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)").

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use thiserror::Error;

use smelt_core::sources::SourcesConfig;
use smelt_core::ModelFile;
use smelt_logical::backbuild::{
    definition_diff, derive_migration_plan, plan_hash, BackbuildInputs, DefinitionDiff,
    MigrationPlan, MigrationVerdict, SourceRef,
};
use smelt_state::file_store::FileStore;
use smelt_state::schema_tracking::DeployedColumn;

/// `smelt run`'s (or `smelt build`'s) refusal to fold a data delta over a
/// pending, unapproved definition delta — `DefinitionDeltaPending`
/// (`docs/specs/definition_deltas.md` §"Detection"). A distinct type so
/// `commands::run::exit_code_for` can classify it to exit `3`
/// (`docs/specs/cli.md` §"Exit codes") without a string match.
#[derive(Debug, Error)]
#[error(
    "DefinitionDeltaPending: '{model}' has a pending, unapproved definition delta (verdict: \
     {verdict:?}, plan hash {plan_hash}). Review it with `smelt migrate {model}`, then \
     `--apply`, or run with `--full-refresh`."
)]
pub struct DefinitionDeltaPendingError {
    pub model: String,
    pub verdict: MigrationVerdict,
    pub plan_hash: String,
}

/// Where a model stands relative to its last-deployed definition.
#[derive(Debug, Clone)]
pub enum DefinitionDeltaStatus {
    /// No recorded deployed schema, or one that predates `model_sql`
    /// tracking — there is nothing to diff against, so this is never
    /// treated as a delta.
    Unknown,
    /// The recorded and current definitions are byte-identical (after
    /// frontmatter stripping) — no delta.
    None,
    /// A definition delta exists, but every affected column group folded to
    /// nothing (`MigrationPlan::groups` is empty) — nothing to migrate.
    Eclipsed,
    /// A non-eclipsed definition delta exists and either no approval is on
    /// record for it, or the recorded approval's hash does not match this
    /// freshly re-derived plan.
    Pending {
        verdict: MigrationVerdict,
        plan_hash: String,
        /// Whether the diff is a pure column addition
        /// (`DefinitionDiff::is_pure_column_addition`) — the shape the
        /// maintenance driver's own live `Trigger::ColumnAdded` dispatch
        /// already handles safely as part of an ordinary run. The run gate
        /// does not refuse this case; `smelt explain`/`smelt migrate`
        /// still report and offer it.
        pure_column_addition: bool,
    },
    /// An approved plan matching this hash is mid-`--apply` (interrupted or
    /// currently executing) — `docs/specs/definition_deltas.md` §"Mid-migration
    /// data folds" governs folding while this status holds.
    InProgress,
    /// An approval matching this exact plan hash is on record and not
    /// in-progress.
    Approved,
}

/// The result of diffing and classifying a model's current SQL against its
/// last-deployed definition — the plan `smelt migrate` prints and applies,
/// the run gate consults for an admission decision, and `smelt explain`
/// reports ahead of a run.
pub struct DerivedPlan {
    pub plan: MigrationPlan,
    pub inputs: BackbuildInputs,
    pub hash: String,
    pub diff: DefinitionDiff,
    /// Whether the recorded and current definitions differ at all after
    /// frontmatter stripping — `false` only for byte-identical text
    /// (distinct from a diff that exists but folds to a no-op, which the
    /// caller reads off `plan.groups.is_empty()` / `diff.is_noop()`).
    pub text_changed: bool,
}

/// Diff `model`'s current SQL against `before_sql_raw` (the recorded
/// last-deployed definition) and derive the migration plan + hash
/// (`docs/specs/definition_deltas.md` §"The migration plan"). `all_models`
/// and `sources` supply the same best-effort upstream facts `smelt migrate`
/// uses to build [`BackbuildInputs`] — a source or upstream model whose
/// facts cannot be found is left out, fail-closed, only ever costing
/// admitted techniques.
pub fn derive_plan(
    file_store: &FileStore,
    model: &ModelFile,
    all_models: &[ModelFile],
    sources: Option<&SourcesConfig>,
    db: &smelt_db::Database,
    before_sql_raw: &str,
    deployed_columns: &[DeployedColumn],
) -> Result<DerivedPlan> {
    let model_name = model.canonical_path();
    let db_name = model.db_name_owned();

    let before_clean = smelt_parser::strip_frontmatter(before_sql_raw);
    let before_parse = smelt_parser::parse(&before_clean);
    let before_file = smelt_parser::File::cast(before_parse.syntax()).ok_or_else(|| {
        anyhow::anyhow!("Failed to parse the recorded definition for '{model_name}'")
    })?;

    let after_clean = smelt_parser::strip_frontmatter(&model.content);
    let text_changed = before_clean != after_clean;

    let after_parse = smelt_parser::parse(&after_clean);
    let after_file = smelt_parser::File::cast(after_parse.syntax()).ok_or_else(|| {
        anyhow::anyhow!("Failed to parse the current definition for '{model_name}'")
    })?;

    let diff = definition_diff(&before_file, &after_file);

    let inferred = crate::schema_evolution::infer_deployed_columns(db, model);
    let row_identity = model.metadata.as_ref().and_then(|m| m.unique_key.clone());

    let not_null_columns: BTreeSet<String> = deployed_columns
        .iter()
        .filter(|c| !c.nullable)
        .map(|c| c.name.clone())
        .collect();

    let deployed_names: BTreeSet<&str> = deployed_columns.iter().map(|c| c.name.as_str()).collect();
    let added_column_types: BTreeMap<String, String> = inferred
        .iter()
        .filter(|c| !deployed_names.contains(c.name.as_str()))
        .map(|c| (c.name.clone(), c.data_type.clone()))
        .collect();

    let sources_map = build_sources_map(model, all_models, sources, file_store);

    let inputs = BackbuildInputs {
        table: db_name.clone(),
        after_sql: after_clean.clone(),
        row_identity,
        not_null_columns,
        added_column_types,
        sources: sources_map,
    };

    let plan = derive_migration_plan(&model_name, &diff, &inputs);
    let hash = plan_hash(&plan, &inputs);

    Ok(DerivedPlan {
        plan,
        inputs,
        hash,
        diff,
        text_changed,
    })
}

/// Derive `model`'s [`DefinitionDeltaStatus`] against its last-deployed
/// definition on `file_store`'s target — the single derivation the run
/// gate, `smelt explain`, and `smelt migrate` all read
/// (`docs/specs/definition_deltas.md` §Detection).
pub fn detect_definition_delta(
    file_store: &FileStore,
    model: &ModelFile,
    all_models: &[ModelFile],
    sources: Option<&SourcesConfig>,
    db: &smelt_db::Database,
) -> Result<DefinitionDeltaStatus> {
    let model_name = model.canonical_path();
    let db_name = model.db_name_owned();

    let deployed = file_store
        .load_schema(&db_name)
        .with_context(|| format!("Failed to load deployed schema for {model_name}"))?;

    let Some(deployed) = deployed else {
        return Ok(DefinitionDeltaStatus::Unknown);
    };

    let Some(before_sql_raw) = deployed.model_sql.clone() else {
        return Ok(DefinitionDeltaStatus::Unknown);
    };

    let derived = derive_plan(
        file_store,
        model,
        all_models,
        sources,
        db,
        &before_sql_raw,
        &deployed.columns,
    )?;

    // Byte-identical text is not merely eclipsed, it is no delta at all —
    // distinct from a diff that exists but folds to a no-op (`Eclipsed`).
    if !derived.text_changed {
        return Ok(DefinitionDeltaStatus::None);
    }

    if derived.plan.groups.is_empty() {
        return Ok(DefinitionDeltaStatus::Eclipsed);
    }

    let approvals = file_store
        .load_migration_approvals()
        .with_context(|| "Failed to load migration-approval store")?;

    match approvals.get(&model_name) {
        Some(a) if a.plan_hash == derived.hash && a.in_progress => {
            Ok(DefinitionDeltaStatus::InProgress)
        }
        Some(a) if a.plan_hash == derived.hash => Ok(DefinitionDeltaStatus::Approved),
        _ => Ok(DefinitionDeltaStatus::Pending {
            verdict: derived.plan.verdict(),
            plan_hash: derived.hash,
            pure_column_addition: derived.diff.is_pure_column_addition(),
        }),
    }
}

/// Best-effort upstream-facts map for `BackbuildInputs::sources`, keyed by
/// each ref's leaf name — not proven alias-exact against the FROM-tree
/// (a documented simplification, mirrored from `commands::migrate`). A
/// source or upstream model whose facts cannot be found is simply left
/// out — fail-closed, only ever costing admitted techniques, never a wrong
/// admission.
fn build_sources_map(
    model: &ModelFile,
    all_models: &[ModelFile],
    sources: Option<&SourcesConfig>,
    file_store: &FileStore,
) -> BTreeMap<String, SourceRef> {
    let mut sources_map: BTreeMap<String, SourceRef> = BTreeMap::new();
    for r in &model.refs {
        let path = r.smelt_ref.to_path();
        let leaf = r.smelt_ref.leaf_name();
        if leaf.is_empty() || sources_map.contains_key(&leaf) {
            continue;
        }

        if path.first().map(String::as_str) == Some("sources") && path.len() >= 3 {
            let source_name = &path[path.len() - 2];
            let table_name = &path[path.len() - 1];
            if let Some(source_def) = sources
                .as_ref()
                .and_then(|sc| sc.sources.iter().find(|s| &s.name == source_name))
            {
                if let Some(table_def) = source_def.tables.iter().find(|t| &t.name == table_name) {
                    sources_map.insert(
                        leaf.clone(),
                        SourceRef {
                            physical_name: table_def
                                .identifier
                                .clone()
                                .unwrap_or_else(|| table_def.name.clone()),
                            unique_key: None,
                            not_null_columns: BTreeSet::new(),
                        },
                    );
                    continue;
                }
            }
        }

        let canonical = path.join(".");
        if let Some(upstream) = all_models
            .iter()
            .find(|m| m.canonical_path() == canonical || m.name == leaf)
        {
            let upstream_unique_key = upstream
                .metadata
                .as_ref()
                .and_then(|m| m.unique_key.clone());
            let upstream_not_null: BTreeSet<String> = file_store
                .load_schema(&upstream.db_name_owned())
                .ok()
                .flatten()
                .map(|s| {
                    s.columns
                        .iter()
                        .filter(|c| !c.nullable)
                        .map(|c| c.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            sources_map.insert(
                leaf,
                SourceRef {
                    physical_name: upstream.db_name_owned(),
                    unique_key: upstream_unique_key,
                    not_null_columns: upstream_not_null,
                },
            );
        }
    }
    sources_map
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_state::migration_approvals::MigrationApprovalStore;
    use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};
    use tempfile::TempDir;

    fn model_file(project_dir: &std::path::Path, name: &str, sql: &str) -> ModelFile {
        let path = project_dir.join("models").join(format!("{name}.sql"));
        ModelFile {
            name: name.to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: sql.to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: smelt_core::discovery::ModelKind::Sql,
            address_segments: vec![name.to_string()],
        }
    }

    fn save_deployed(
        file_store: &FileStore,
        db_name: &str,
        sql: Option<&str>,
        columns: Vec<DeployedColumn>,
    ) {
        let schema = DeployedSchema {
            model: db_name.to_string(),
            version: 1,
            deployed_at: chrono::Utc::now(),
            model_hash: String::new(),
            model_sql: sql.map(|s| s.to_string()),
            columns,
        };
        file_store.save_schema(&schema).unwrap();
    }

    fn empty_db() -> smelt_db::Database {
        smelt_db::Database::default()
    }

    #[test]
    fn no_recorded_model_sql_is_not_a_delta() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let sql = "select 1 as a";
        let model = model_file(dir.path(), "m", sql);

        save_deployed(&file_store, &model.db_name_owned(), None, vec![]);

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(status, DefinitionDeltaStatus::Unknown));
    }

    #[test]
    fn identical_definition_is_no_delta() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let sql = "select 1 as a";
        let model = model_file(dir.path(), "m", sql);

        save_deployed(&file_store, &model.db_name_owned(), Some(sql), vec![]);

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(status, DefinitionDeltaStatus::None));
    }

    #[test]
    fn eclipsed_delta_does_not_gate() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        // Trivia-only change (whitespace) — eclipsed: no atoms at all, so
        // `derive_migration_plan` yields no groups.
        let before = "select 1 as a";
        let after = "select   1   as   a";
        let model = model_file(dir.path(), "m", after);

        save_deployed(&file_store, &model.db_name_owned(), Some(before), vec![]);

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(status, DefinitionDeltaStatus::Eclipsed));
    }

    #[test]
    fn non_eclipsed_unapproved_delta_is_pending() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let before = "select 1 as a";
        let after = "select 1 as a, 2 as b";
        let model = model_file(dir.path(), "m", after);

        save_deployed(
            &file_store,
            &model.db_name_owned(),
            Some(before),
            vec![DeployedColumn {
                name: "a".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
        );

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(
            status,
            DefinitionDeltaStatus::Pending {
                pure_column_addition: true,
                ..
            }
        ));
    }

    #[test]
    fn changed_column_is_not_a_pure_addition() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let before = "select 1 as a";
        let after = "select 2 as a";
        let model = model_file(dir.path(), "m", after);

        save_deployed(
            &file_store,
            &model.db_name_owned(),
            Some(before),
            vec![DeployedColumn {
                name: "a".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
        );

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(
            status,
            DefinitionDeltaStatus::Pending {
                pure_column_addition: false,
                ..
            }
        ));
    }

    #[test]
    fn matching_approval_is_approved() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");
        let before = "select 1 as a";
        let after = "select 1 as a, 2 as b";
        let model = model_file(dir.path(), "m", after);

        save_deployed(
            &file_store,
            &model.db_name_owned(),
            Some(before),
            vec![DeployedColumn {
                name: "a".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
        );

        let db = empty_db();
        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        let DefinitionDeltaStatus::Pending { plan_hash, .. } = status else {
            panic!("expected Pending before approval, got {status:?}");
        };

        let mut approvals = MigrationApprovalStore::default();
        approvals.record(&model.canonical_path(), plan_hash, false);
        file_store.save_migration_approvals(&approvals).unwrap();

        let status =
            detect_definition_delta(&file_store, &model, &[], None, &db).expect("should not error");
        assert!(matches!(status, DefinitionDeltaStatus::Approved));
    }
}
