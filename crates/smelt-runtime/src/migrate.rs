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

use smelt_backend::Backend;
use smelt_logical::backbuild::{
    definition_diff, derive_backbuild_options, derive_migration_plan,
    statement_group_for_candidate, BackbuildInputs, CostClass, MigrationPlan, SourceRef, Technique,
    Verdict,
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

    // Both sides are the raw, frontmatter-bearing model text
    // (`ModelMigrationFacts::after_sql`'s own doc comment: "the same form
    // `DeployedSchema::definition_sql` records"). `definition_diff` walks
    // `File::select_stmt()`, which resolves only when the SQL body is the
    // file's own top-level statement — a leading `---\n...\n---\n`
    // frontmatter block (every model declaring `refresh: incremental`
    // carries one) makes that `None`, collapsing every diff to `Opaque`
    // ("not a plain SELECT statement") regardless of what actually changed.
    // Strip it from both sides here, once, right before parsing — the
    // stored/passed-in raw form stays frontmatter-bearing for whichever
    // other consumer needs it (`facts.after_sql` unchanged; only these local
    // copies feed the diff and `BackbuildInputs`).
    let before_sql = smelt_parser::strip_frontmatter(&deployed.definition_sql);
    let after_sql = smelt_parser::strip_frontmatter(&facts.after_sql);

    let before_file = parse_select_file(&before_sql).map_err(|reason| {
        MigrateError::UnparsableRecordedDefinition {
            model: model_name.to_string(),
            reason,
        }
    })?;
    let after_file =
        parse_select_file(&after_sql).map_err(|reason| MigrateError::UnparsableCurrentSql {
            model: model_name.to_string(),
            reason,
        })?;

    let diff = definition_diff(&before_file, &after_file);

    let inputs = BackbuildInputs {
        table: facts.table.clone(),
        after_sql,
        row_identity: facts.row_identity.clone(),
        not_null_columns: not_null_columns(&facts.current_columns),
        added_column_types: added_column_types(&facts.current_columns, &deployed.columns),
        sources: facts.sources.clone(),
    };

    let options = derive_backbuild_options(&diff, &inputs);
    let plan = derive_migration_plan(&options);
    Ok((inputs, plan))
}

/// A named, fail-closed reason [`apply_migration_plan`] refuses to execute
/// `label`'s group — checked over **every** group before anything runs, so a
/// refusal on any one group means the whole plan executes nothing
/// (`docs/specs/definition_deltas.md` §Surface "`smelt migrate`" "Approve
/// and apply").
#[derive(Debug, thiserror::Error)]
pub enum MigrationApplyRefusal {
    #[error(
        "group '{label}' is a skeleton change — no targeted technique is ever admissible; the \
         honest route is `smelt build --full-refresh` or `smelt rebuild`"
    )]
    SkeletonChange { label: String },
    #[error("group '{label}' has no admissible candidate technique")]
    NoAdmissibleCandidate { label: String },
    #[error(
        "group '{label}' first candidate ({technique:?}) is destructive — its verification \
         probes are not emitted yet, so it is refused rather than executed unverified"
    )]
    DestructiveCandidate { label: String, technique: Technique },
}

/// Either [`apply_migration_plan`] refused to execute (nothing ran) or a
/// group's statements failed against the backend partway through (earlier
/// groups in this call may already have committed — `on_group_applied` was
/// invoked for each of them before the failure).
#[derive(Debug, thiserror::Error)]
pub enum MigrationApplyError {
    #[error(transparent)]
    Refused(#[from] MigrationApplyRefusal),
    #[error("group '{label}' failed to execute: {source}")]
    Backend {
        label: String,
        #[source]
        source: smelt_backend::BackendError,
    },
}

/// Every group's admission reason to refuse execution, checked before any
/// statement runs — `None` when every group is either already applied or has
/// an admissible, non-destructive first candidate.
fn first_refusal(plan: &MigrationPlan) -> Option<MigrationApplyRefusal> {
    for group in &plan.groups {
        if group.verdict == Verdict::SkeletonChange {
            return Some(MigrationApplyRefusal::SkeletonChange {
                label: group.label.clone(),
            });
        }
        let Some(candidate) = group.candidates.first() else {
            return Some(MigrationApplyRefusal::NoAdmissibleCandidate {
                label: group.label.clone(),
            });
        };
        if candidate.cost_class == CostClass::Destructive {
            return Some(MigrationApplyRefusal::DestructiveCandidate {
                label: group.label.clone(),
                technique: candidate.technique,
            });
        }
    }
    None
}

/// Execute an approved [`MigrationPlan`]'s statements against `backend`: each
/// group's first presented candidate, one transactional statement group per
/// column group, in plan order (`docs/specs/definition_deltas.md` §"The
/// atomicity rule") — approving the plan approves that selection, since the
/// plan is deterministic and its hash covers every candidate. Admission is
/// checked over **every** group first; a refusal on any one group executes
/// nothing. A group whose label is already in `already_applied` is skipped
/// (resume — §Surface "`smelt migrate`" "Resume"); `on_group_applied` is
/// invoked with a group's label immediately after its statements commit, so
/// a caller can persist resume progress before this call returns (including
/// on a later group's failure).
pub async fn apply_migration_plan(
    backend: &dyn Backend,
    plan: &MigrationPlan,
    already_applied: &BTreeSet<String>,
    mut on_group_applied: impl FnMut(&str),
) -> Result<(), MigrationApplyError> {
    if let Some(refusal) = first_refusal(plan) {
        return Err(refusal.into());
    }

    for group in &plan.groups {
        if already_applied.contains(&group.label) {
            continue;
        }
        // `first_refusal` already proved every group has a non-destructive
        // first candidate.
        let candidate = &group.candidates[0];
        let statement_group = statement_group_for_candidate(candidate);
        backend
            .execute_statement_group(&statement_group)
            .await
            .map_err(|source| MigrationApplyError::Backend {
                label: group.label.clone(),
                source,
            })?;
        on_group_applied(&group.label);
    }

    Ok(())
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

    // ===== `apply_migration_plan` =====

    use smelt_backend::{BackendCapabilities, BackendError, PartitionRange, SqlDialect};
    use smelt_logical::backbuild::{BackbuildOption, ColumnGroupPlan, TechniqueCandidate};
    use std::sync::Mutex;

    /// Records every [`StatementGroup`] handed to `execute_statement_group`
    /// (the sole point `apply_migration_plan` executes through) and every
    /// other required `Backend` method is unreachable from this module's
    /// code path and left `unimplemented!()`.
    struct RecordingBackend {
        groups: Mutex<Vec<smelt_backend::StatementGroup>>,
        /// When `Some(n)`, the `n`th call to `execute_statement_group`
        /// (0-indexed, counting only calls that reach it) fails instead of
        /// recording.
        fail_on_call: Option<usize>,
        calls: Mutex<usize>,
    }

    impl RecordingBackend {
        fn new() -> Self {
            RecordingBackend {
                groups: Mutex::new(Vec::new()),
                fail_on_call: None,
                calls: Mutex::new(0),
            }
        }

        fn failing_on_call(n: usize) -> Self {
            RecordingBackend {
                groups: Mutex::new(Vec::new()),
                fail_on_call: Some(n),
                calls: Mutex::new(0),
            }
        }

        fn recorded_sql(&self) -> Vec<String> {
            self.groups
                .lock()
                .unwrap()
                .iter()
                .flat_map(|g| g.statements.iter().map(|s| s.sql.clone()))
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl Backend for RecordingBackend {
        async fn execute_sql(
            &self,
            _sql: &str,
        ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
            unimplemented!("apply_migration_plan only calls execute_statement_group")
        }

        async fn create_table_as(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn create_view_as(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn drop_table_if_exists(&self, _: &str, _: &str) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn drop_view_if_exists(&self, _: &str, _: &str) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn get_row_count(&self, _: &str, _: &str) -> Result<usize, BackendError> {
            unimplemented!()
        }

        async fn get_preview(
            &self,
            _: &str,
            _: &str,
            _: usize,
        ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
            unimplemented!()
        }

        async fn table_exists(&self, _: &str, _: &str) -> Result<bool, BackendError> {
            unimplemented!()
        }

        async fn ensure_schema(&self, _: &str) -> Result<(), BackendError> {
            unimplemented!()
        }

        fn dialect(&self) -> SqlDialect {
            SqlDialect::DuckDB
        }

        fn capabilities(&self) -> BackendCapabilities {
            unimplemented!()
        }

        async fn load_table(
            &self,
            _: &str,
            _: &str,
            _: arrow::datatypes::SchemaRef,
            _: Vec<arrow::array::RecordBatch>,
        ) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn delete_partitions(
            &self,
            _: &str,
            _: &str,
            _: &PartitionRange,
        ) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn insert_into_from_query(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn insert_overwrite(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &PartitionRange,
        ) -> Result<(), BackendError> {
            unimplemented!()
        }

        async fn execute_statement_group(
            &self,
            group: &smelt_backend::StatementGroup,
        ) -> Result<(), BackendError> {
            let mut calls = self.calls.lock().unwrap();
            let this_call = *calls;
            *calls += 1;
            if self.fail_on_call == Some(this_call) {
                return Err(BackendError::ExecutionFailed {
                    model: "test".to_string(),
                    message: "simulated failure".to_string(),
                });
            }
            self.groups.lock().unwrap().push(group.clone());
            Ok(())
        }
    }

    fn full_refresh_option() -> BackbuildOption {
        BackbuildOption {
            technique: Technique::FullRefresh,
            slot: None,
            statements: vec!["CREATE OR REPLACE TABLE t AS SELECT 1".to_string()],
            write_scope: smelt_logical::backbuild::WriteScope::FullWrite,
            reads_upstream: true,
            rerun_safe: true,
        }
    }

    fn candidate(technique: Technique, cost_class: CostClass, sql: &str) -> TechniqueCandidate {
        TechniqueCandidate {
            technique,
            cost_class,
            statements: vec![sql.to_string()],
            reads_upstream: false,
            rerun_safe: true,
        }
    }

    fn backfill_group(label: &str, sql: &str) -> ColumnGroupPlan {
        ColumnGroupPlan {
            label: label.to_string(),
            verdict: Verdict::BackfillInPlace,
            candidates: vec![candidate(
                Technique::SelfDerivedColumnAdd,
                CostClass::LocalColumnUpdate,
                sql,
            )],
            refusals: Vec::new(),
        }
    }

    fn skeleton_group(label: &str) -> ColumnGroupPlan {
        ColumnGroupPlan {
            label: label.to_string(),
            verdict: Verdict::SkeletonChange,
            candidates: Vec::new(),
            refusals: Vec::new(),
        }
    }

    fn no_candidate_group(label: &str) -> ColumnGroupPlan {
        ColumnGroupPlan {
            label: label.to_string(),
            verdict: Verdict::ReDerive,
            candidates: Vec::new(),
            refusals: Vec::new(),
        }
    }

    fn destructive_group(label: &str) -> ColumnGroupPlan {
        ColumnGroupPlan {
            label: label.to_string(),
            verdict: Verdict::ReDerive,
            candidates: vec![candidate(
                Technique::ColumnDrop,
                CostClass::Destructive,
                "ALTER TABLE t DROP COLUMN d",
            )],
            refusals: Vec::new(),
        }
    }

    fn plan_with(groups: Vec<ColumnGroupPlan>) -> MigrationPlan {
        MigrationPlan {
            eclipsed: false,
            groups,
            full_refresh: full_refresh_option(),
        }
    }

    #[tokio::test]
    async fn apply_executes_first_candidate_per_group_in_plan_order() {
        let plan = plan_with(vec![
            backfill_group("added column 'a'", "ALTER TABLE t ADD COLUMN a INT"),
            backfill_group("added column 'b'", "ALTER TABLE t ADD COLUMN b INT"),
        ]);
        let backend = RecordingBackend::new();
        let mut applied = Vec::new();

        apply_migration_plan(&backend, &plan, &BTreeSet::new(), |label| {
            applied.push(label.to_string())
        })
        .await
        .expect("plan should apply");

        assert_eq!(
            backend.recorded_sql(),
            vec![
                "ALTER TABLE t ADD COLUMN a INT".to_string(),
                "ALTER TABLE t ADD COLUMN b INT".to_string(),
            ]
        );
        assert_eq!(applied, vec!["added column 'a'", "added column 'b'"]);
    }

    #[tokio::test]
    async fn apply_refuses_skeleton_change_group_without_executing() {
        let plan = plan_with(vec![
            backfill_group("added column 'a'", "ALTER TABLE t ADD COLUMN a INT"),
            skeleton_group("skeleton"),
        ]);
        let backend = RecordingBackend::new();

        let err = apply_migration_plan(&backend, &plan, &BTreeSet::new(), |_| {})
            .await
            .expect_err("a skeleton-change group refuses the whole plan");

        assert!(matches!(
            err,
            MigrationApplyError::Refused(MigrationApplyRefusal::SkeletonChange { .. })
        ));
        assert!(backend.recorded_sql().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_group_with_no_admissible_candidate() {
        let plan = plan_with(vec![no_candidate_group("changed column 'c'")]);
        let backend = RecordingBackend::new();

        let err = apply_migration_plan(&backend, &plan, &BTreeSet::new(), |_| {})
            .await
            .expect_err("a candidate-less group refuses the whole plan");

        assert!(matches!(
            err,
            MigrationApplyError::Refused(MigrationApplyRefusal::NoAdmissibleCandidate { .. })
        ));
        assert!(backend.recorded_sql().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_destructive_candidate() {
        let plan = plan_with(vec![destructive_group("dropped column 'd'")]);
        let backend = RecordingBackend::new();

        let err = apply_migration_plan(&backend, &plan, &BTreeSet::new(), |_| {})
            .await
            .expect_err("a destructive first candidate refuses the whole plan");

        assert!(matches!(
            err,
            MigrationApplyError::Refused(MigrationApplyRefusal::DestructiveCandidate { .. })
        ));
        assert!(backend.recorded_sql().is_empty());
    }

    #[tokio::test]
    async fn apply_skips_groups_already_recorded_applied() {
        let plan = plan_with(vec![
            backfill_group("added column 'a'", "ALTER TABLE t ADD COLUMN a INT"),
            backfill_group("added column 'b'", "ALTER TABLE t ADD COLUMN b INT"),
        ]);
        let backend = RecordingBackend::new();
        let already_applied: BTreeSet<String> = ["added column 'a'".to_string()].into();
        let mut applied = Vec::new();

        apply_migration_plan(&backend, &plan, &already_applied, |label| {
            applied.push(label.to_string())
        })
        .await
        .expect("plan should apply the remainder");

        assert_eq!(
            backend.recorded_sql(),
            vec!["ALTER TABLE t ADD COLUMN b INT".to_string()]
        );
        assert_eq!(applied, vec!["added column 'b'"]);
    }

    #[tokio::test]
    async fn apply_reports_already_applied_when_every_group_is_recorded() {
        let plan = plan_with(vec![backfill_group(
            "added column 'a'",
            "ALTER TABLE t ADD COLUMN a INT",
        )]);
        let backend = RecordingBackend::new();
        let already_applied: BTreeSet<String> = ["added column 'a'".to_string()].into();

        apply_migration_plan(&backend, &plan, &already_applied, |_| {
            panic!("nothing should apply when every group is already recorded")
        })
        .await
        .expect("already-applied plan reports Ok with nothing to do");

        assert!(backend.recorded_sql().is_empty());
    }

    #[tokio::test]
    async fn apply_stops_after_a_backend_failure_but_reports_earlier_progress() {
        let plan = plan_with(vec![
            backfill_group("added column 'a'", "ALTER TABLE t ADD COLUMN a INT"),
            backfill_group("added column 'b'", "ALTER TABLE t ADD COLUMN b INT"),
        ]);
        let backend = RecordingBackend::failing_on_call(1);
        let mut applied = Vec::new();

        let err = apply_migration_plan(&backend, &plan, &BTreeSet::new(), |label| {
            applied.push(label.to_string())
        })
        .await
        .expect_err("the second group's backend failure should propagate");

        assert!(matches!(err, MigrationApplyError::Backend { .. }));
        assert_eq!(applied, vec!["added column 'a'"]);
    }
}
