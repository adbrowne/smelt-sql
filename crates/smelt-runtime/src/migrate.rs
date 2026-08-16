//! Plan-only definition-delta migration: assemble real facts about a
//! deployed model into `smelt_logical::backbuild::BackbuildInputs`, run the
//! diff → classify → plan pipeline, and hand back the printable
//! [`smelt_logical::backbuild::MigrationPlan`]. `smelt migrate` is a
//! renderer only — this is the one place fact assembly happens
//! (`docs/outcomes/20260816-definition-delta-migrate-v2/outcome.md`
//! "Decision log": fact assembly lives in `smelt-runtime` so the UI can
//! consume it too, plan derivation stays pure in `smelt-logical`).
//!
//! No backend writes: this module never touches a `Backend`, only
//! `smelt_state::FileStore` reads and pure classification.

use std::collections::{BTreeMap, BTreeSet};

use smelt_logical::backbuild::{
    definition_diff, derive_backbuild_options, derive_migration_plan, BackbuildInputs,
    MigrationPlan, SourceRef,
};
use smelt_state::schema_tracking::{DeployedColumn, DeployedSchema};

/// Real facts about a deployed model and its upstreams, gathered by the
/// caller (the CLI command assembles this from `Config`, `DependencyGraph`,
/// `smelt_db::Database`, and `FileStore` — see `commands/migrate.rs`).
/// Missing facts stay absent (`None`/empty) rather than guessed — fail
/// closed, mirroring `BackbuildInputs`'s own posture.
#[derive(Debug, Clone)]
pub struct ModelMigrationFacts {
    /// Physical name of the deployed table.
    pub table: String,
    /// The model's current raw SQL text — the same form
    /// `DeployedSchema::definition_sql` records (`model.content`, never a
    /// compiled/type-cast form) — the "after" side of the diff.
    pub after_sql: String,
    /// The model's declared row identity (`unique_key:` or `GROUP BY`).
    pub row_identity: Option<Vec<String>>,
    /// The model's current output columns (name, SQL type, nullability),
    /// as inferred from `after_sql` — used both for `not_null_columns` and
    /// to detect which output columns are newly added relative to the
    /// deployed schema.
    pub current_columns: Vec<DeployedColumn>,
    /// The deployed-schema snapshot last saved for this model — carries the
    /// "before" definition SQL (`DeployedSchema::definition_sql`) and the
    /// previously-deployed column set. `None` when the model has never been
    /// deployed.
    pub deployed: Option<DeployedSchema>,
    /// Upstream name (as referenced in the model's FROM/JOIN tree) → source
    /// facts.
    pub sources: BTreeMap<String, SourceRef>,
}

/// A named, fail-closed reason [`derive_migration_plan_for_model`] could not
/// produce a plan — never a silently empty or default plan.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(
        "model '{model}' has no recorded definition to diff against — it has never been \
         deployed, or its deployed-schema snapshot predates definition-SQL tracking; run \
         `smelt build` (or `smelt run`) at least once before `smelt migrate`"
    )]
    NoRecordedDefinition { model: String },
    #[error("model '{model}': could not parse the recorded definition SQL: {reason}")]
    UnparsableRecordedDefinition { model: String, reason: String },
    #[error("model '{model}': could not parse the current SQL: {reason}")]
    UnparsableCurrentSql { model: String, reason: String },
}

fn parse_select_file(sql: &str) -> Result<smelt_parser::File, String> {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).ok_or_else(|| "not a valid smelt file".to_string())
}

/// Output columns present in `current` but absent (by name) from `deployed`
/// — the added-column-types map `BackbuildInputs` needs.
fn added_column_types(
    current: &[DeployedColumn],
    deployed: &[DeployedColumn],
) -> BTreeMap<String, String> {
    let deployed_names: BTreeSet<&str> = deployed.iter().map(|c| c.name.as_str()).collect();
    current
        .iter()
        .filter(|c| !deployed_names.contains(c.name.as_str()))
        .map(|c| (c.name.clone(), c.data_type.clone()))
        .collect()
}

fn not_null_columns(columns: &[DeployedColumn]) -> BTreeSet<String> {
    columns
        .iter()
        .filter(|c| !c.nullable)
        .map(|c| c.name.clone())
        .collect()
}

/// Derive the [`MigrationPlan`] for one model from its gathered facts —
/// pure aside from the two SQL parses. Executes nothing. Returns the
/// [`BackbuildInputs`] alongside the plan so a caller (`commands/migrate.rs`)
/// can hash exactly the facts the plan was derived from
/// (`smelt_logical::backbuild::plan_hash` — `docs/specs/definition_deltas.md`
/// §Design "The plan hash covers the plan data structure, not only rendered
/// SQL") without reconstructing them.
pub fn derive_migration_plan_for_model(
    model_name: &str,
    facts: &ModelMigrationFacts,
) -> Result<(BackbuildInputs, MigrationPlan), MigrateError> {
    let deployed = facts
        .deployed
        .as_ref()
        .filter(|d| !d.definition_sql.is_empty())
        .ok_or_else(|| MigrateError::NoRecordedDefinition {
            model: model_name.to_string(),
        })?;

    let before_file = parse_select_file(&deployed.definition_sql).map_err(|reason| {
        MigrateError::UnparsableRecordedDefinition {
            model: model_name.to_string(),
            reason,
        }
    })?;
    let after_file = parse_select_file(&facts.after_sql).map_err(|reason| {
        MigrateError::UnparsableCurrentSql {
            model: model_name.to_string(),
            reason,
        }
    })?;

    let diff = definition_diff(&before_file, &after_file);

    let inputs = BackbuildInputs {
        table: facts.table.clone(),
        after_sql: facts.after_sql.clone(),
        row_identity: facts.row_identity.clone(),
        not_null_columns: not_null_columns(&facts.current_columns),
        added_column_types: added_column_types(&facts.current_columns, &deployed.columns),
        sources: facts.sources.clone(),
    };

    let options = derive_backbuild_options(&diff, &inputs);
    let plan = derive_migration_plan(&options);
    Ok((inputs, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn column(name: &str, ty: &str, nullable: bool) -> DeployedColumn {
        DeployedColumn {
            name: name.to_string(),
            data_type: ty.to_string(),
            nullable,
        }
    }

    fn deployed(definition_sql: &str, columns: Vec<DeployedColumn>) -> DeployedSchema {
        DeployedSchema {
            model: "orders_summary".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:old".to_string(),
            columns,
            definition_sql: definition_sql.to_string(),
        }
    }

    #[test]
    fn derive_migration_plan_reads_recorded_definition() {
        let facts = ModelMigrationFacts {
            table: "orders_summary".to_string(),
            after_sql: "SELECT id, amount, amount * 0.9 AS net_amount FROM orders".to_string(),
            row_identity: None,
            current_columns: vec![
                column("id", "BIGINT", false),
                column("amount", "DECIMAL(10,2)", false),
                column("net_amount", "DECIMAL(10,2)", true),
            ],
            deployed: Some(deployed(
                "SELECT id, amount FROM orders",
                vec![
                    column("id", "BIGINT", false),
                    column("amount", "DECIMAL(10,2)", false),
                ],
            )),
            sources: BTreeMap::new(),
        };

        let (_inputs, plan) =
            derive_migration_plan_for_model("orders_summary", &facts).expect("plan should derive");

        assert!(!plan.eclipsed);
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(
            plan.groups[0].verdict,
            smelt_logical::backbuild::Verdict::BackfillInPlace
        );
        assert!(plan.groups[0]
            .candidates
            .iter()
            .any(|c| c.technique == smelt_logical::backbuild::Technique::SelfDerivedColumnAdd));
    }

    #[test]
    fn derive_migration_plan_without_recorded_definition_errors() {
        let facts = ModelMigrationFacts {
            table: "orders_summary".to_string(),
            after_sql: "SELECT id FROM orders".to_string(),
            row_identity: None,
            current_columns: vec![column("id", "BIGINT", false)],
            deployed: None,
            sources: BTreeMap::new(),
        };

        let err = derive_migration_plan_for_model("orders_summary", &facts)
            .expect_err("no deployed schema should error, never yield an empty plan");

        assert!(matches!(err, MigrateError::NoRecordedDefinition { .. }));
    }
}
