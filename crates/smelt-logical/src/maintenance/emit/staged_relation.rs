//! The staged relation a change-suppressed write group creates, populates,
//! reads and cleans up — its residence, name and lifecycle are backend
//! **capability data**
//! (`docs/specs/multi_backend.md` §"Column-scoped merge and conditional-write
//! capabilities"), not a hardcoded `CREATE TEMP TABLE`. DuckDB's session temp
//! namespace gives an atomic, implicitly-dropped relation; Trino has none, so
//! its staged relation is a real, explicitly-named, explicitly-dropped table
//! in the target's own schema, and the group that uses it cannot run as one
//! atomic transaction (`docs/outcomes/20260913-trino-ledger/phases/
//! 01-summary.md` — Trino/Iceberg has no transactional write capability at
//! all).

pub use smelt_dialect::StagedRelationResidence;

/// A staged relation's derived name, where it lives, and whether the group
/// that uses it can run as one atomic transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRelation {
    pub name: String,
    pub residence: StagedRelationResidence,
    pub atomic: bool,
}

impl StagedRelation {
    /// Derive a staged relation's name once, for every purpose
    /// (`__smelt_staged_`, `__smelt_diff_patch_`, `__smelt_repair_`, …),
    /// from the capability-declared residence and atomicity.
    ///
    /// `purpose` is the caller's existing name prefix (kept verbatim so
    /// DuckDB's names stay byte-unchanged); `qualified_table` may carry a
    /// schema qualifier (`main.dim_users`), flattened to a single
    /// non-colliding identifier — the same flattening every purpose already
    /// applied ad hoc before this deriver existed.
    pub fn derive(
        purpose: &str,
        qualified_table: &str,
        residence: StagedRelationResidence,
        atomic: bool,
    ) -> Self {
        let flattened = qualified_table.replace('.', "_");
        Self {
            name: format!("{purpose}{flattened}"),
            residence,
            atomic,
        }
    }

    /// A session-temporary, atomic staged relation with an already-known
    /// name — the shape every production caller uses today (DuckDB, Spark,
    /// BigQuery all have a session temp namespace and run the group inside
    /// one native transaction).
    pub fn session_temporary(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            residence: StagedRelationResidence::SessionTemporary,
            atomic: true,
        }
    }

    /// The `CREATE ... TABLE` prefix for this relation's residence —
    /// `CREATE TEMP TABLE` for a session-temporary relation, `CREATE TABLE`
    /// for a real target-schema relation.
    pub fn create_prefix(&self) -> &'static str {
        match self.residence {
            StagedRelationResidence::SessionTemporary => "CREATE TEMP TABLE",
            StagedRelationResidence::TargetSchema => "CREATE TABLE",
        }
    }

    /// The reclaim statement a non-atomic group must run **before** its own
    /// `CREATE`, so an orphan relation left by a run interrupted between the
    /// stage and the apply is never adopted as live data by a later run.
    /// `None` for an atomic group — DuckDB rolls the whole group back on
    /// failure, so no orphan can exist to reclaim.
    pub fn reclaim_statement(&self) -> Option<String> {
        if self.atomic {
            None
        } else {
            Some(format!("DROP TABLE IF EXISTS {}", self.name))
        }
    }

    /// The trailing cleanup statement every group ends with.
    pub fn drop_statement(&self) -> String {
        format!("DROP TABLE {}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_derived_once_for_every_purpose() {
        let staged = StagedRelation::derive(
            "__smelt_staged_",
            "main.dim_users",
            StagedRelationResidence::SessionTemporary,
            true,
        );
        assert_eq!(staged.name, "__smelt_staged_main_dim_users");

        let diff_patch = StagedRelation::derive(
            "__smelt_diff_patch_",
            "main.dim_users",
            StagedRelationResidence::SessionTemporary,
            true,
        );
        assert_eq!(diff_patch.name, "__smelt_diff_patch_main_dim_users");

        let repair = StagedRelation::derive(
            "__smelt_repair_",
            "main.dim_users",
            StagedRelationResidence::SessionTemporary,
            true,
        );
        assert_eq!(repair.name, "__smelt_repair_main_dim_users");
    }

    #[test]
    fn session_temporary_residence_emits_create_temp_table() {
        let staged = StagedRelation::session_temporary("__smelt_staged_dim_users");
        assert_eq!(staged.create_prefix(), "CREATE TEMP TABLE");
        assert_eq!(staged.reclaim_statement(), None);
        assert_eq!(
            staged.drop_statement(),
            "DROP TABLE __smelt_staged_dim_users"
        );
    }

    #[test]
    fn target_schema_residence_emits_a_schema_qualified_real_table() {
        let staged = StagedRelation::derive(
            "__smelt_staged_",
            "iceberg.smelt_dev.dim_users",
            StagedRelationResidence::TargetSchema,
            false,
        );
        assert_eq!(staged.name, "__smelt_staged_iceberg_smelt_dev_dim_users");
        assert_eq!(staged.create_prefix(), "CREATE TABLE");
        assert!(!staged.name.contains("TEMP"));
    }

    #[test]
    fn non_atomic_residence_prepends_a_reclaim_drop_and_flags_the_group_non_atomic() {
        let staged = StagedRelation::derive(
            "__smelt_staged_",
            "iceberg.smelt_dev.dim_users",
            StagedRelationResidence::TargetSchema,
            false,
        );
        assert_eq!(
            staged.reclaim_statement(),
            Some("DROP TABLE IF EXISTS __smelt_staged_iceberg_smelt_dev_dim_users".to_string())
        );
        assert!(!staged.atomic);
    }
}
