//! `retain_departed` — the contract-lattice point declaring that keys a
//! keyed model's mutable-snapshot source no longer carries are kept (or
//! tombstoned) at reconcile instead of deleted
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)", §"Retention (`retain_departed`)"). This module
//! single-owns the complete declaration half of the triple: the
//! posture-admissibility + tombstone-column validator, the pure
//! departed-key quotient oracle, and the reconcile-anti-join probe emitter
//! that would recover the retained-departed (and, where declared,
//! unmarked-tombstone) key count at runtime. The runtime half — dispatching
//! this probe and suppressing the default point's anti-join delete leg on a
//! declared point — has not landed (`docs/outcomes/
//! 20260815-definition-delta-migrate/phases/32-plan.md`).
//!
//! Unlike `frozen_horizon`/`deferral`, this point has no write-eligibility
//! clamp and no scheduling license of its own — declaring it changes what
//! the default reconcile write is required to prove, not when a cell runs
//! (`incremental_models.md` §"The equivalence invariant", key departure).

use smelt_core::config::Grain;

use crate::maintenance::emit::MaintenanceStatement;

/// Validates that `contract.retain_departed` is declared only on a keyed
/// shape consuming a mutable snapshot — the one posture where key departure
/// is observable and deletion is the default — and, when a tombstone column
/// is named, that it is present in the model's output columns.
/// `consumes_mutable_snapshot` and `output_columns` are caller-resolved (the
/// model's derived grain plus resolved source facts and the inferred output
/// schema, unavailable to this pure function).
///
/// Returns `Err` naming the offending posture or the missing tombstone
/// column.
pub fn validate(
    grain: Grain,
    consumes_mutable_snapshot: bool,
    tombstone_column: Option<&str>,
    output_columns: &[String],
    model_name: &str,
) -> Result<(), String> {
    if grain != Grain::Key {
        return Err(format!(
            "contract.retain_departed is admitted only on a keyed shape consuming a mutable \
             snapshot; model '{model_name}' has grain {grain:?}"
        ));
    }
    if !consumes_mutable_snapshot {
        return Err(format!(
            "contract.retain_departed is admitted only on a keyed shape consuming a mutable \
             snapshot; model '{model_name}' consumes no mutable_snapshot source"
        ));
    }
    if let Some(col) = tombstone_column {
        if !output_columns.iter().any(|c| c == col) {
            return Err(format!(
                "contract.retain_departed's tombstone column '{col}' is absent from model \
                 '{model_name}''s output columns"
            ));
        }
    }
    Ok(())
}

/// The per-key comparison verdict [`classify_key`] returns — the quotient
/// `incremental_models.md` §"Retention (`retain_departed`)" describes: a key
/// present in the current snapshot compares strictly; a departed key is
/// exempt from comparison unless a tombstone is declared and the key was not
/// marked, which is a violation, not an exemption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyVerdict {
    Strict,
    Exempt,
    UnmarkedDeparture,
}

/// The pure oracle transform: classifies one stored key against the current
/// snapshot and, for a departed key, whether it was marked in the declared
/// tombstone column. `in_current`/`tombstone_marked` are caller-resolved
/// facts (a row lookup); this function makes no comparison of its own
/// beyond the three-way classification.
pub fn classify_key(
    in_current: bool,
    tombstone_declared: bool,
    tombstone_marked: bool,
) -> KeyVerdict {
    if in_current {
        return KeyVerdict::Strict;
    }
    if tombstone_declared && !tombstone_marked {
        return KeyVerdict::UnmarkedDeparture;
    }
    KeyVerdict::Exempt
}

/// The reconcile scan's own anti-join — the probe that recovers the
/// retained-departed key count (and, where a tombstone column is declared,
/// the unmarked-tombstone count among them) without re-running the delete
/// the default point would have performed (`incremental_models.md`
/// §"Retention (`retain_departed`)": "The probe is the reconcile scan's own
/// anti-join"). `stored_table`/`current_table` are already fully qualified;
/// `key_columns` is the model's keyed-shape key.
pub fn emit_departed_key_probe(
    stored_table: &str,
    current_table: &str,
    key_columns: &[&str],
    tombstone_column: Option<&str>,
) -> MaintenanceStatement {
    let join_predicate = key_columns
        .iter()
        .map(|k| format!("s.{k} = c.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let null_check = key_columns
        .first()
        .map(|k| format!("c.{k} IS NULL"))
        .unwrap_or_else(|| "1=1".to_string());
    let unmarked_expr = match tombstone_column {
        Some(col) => format!(
            ", SUM(CASE WHEN s.{col} IS NULL OR s.{col} = FALSE THEN 1 ELSE 0 END) AS \
             unmarked_departed_count"
        ),
        None => String::new(),
    };
    let sql = format!(
        "SELECT COUNT(*) AS retained_departed_count{unmarked_expr} \
         FROM {stored_table} s \
         LEFT JOIN {current_table} c ON {join_predicate} \
         WHERE {null_check}"
    );
    MaintenanceStatement::new(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn admitted_only_on_keyed_mutable_snapshot() {
        assert!(validate(Grain::Key, true, None, &cols(&["id"]), "m").is_ok());

        let err = validate(Grain::Partition, true, None, &cols(&["id"]), "m").unwrap_err();
        assert!(err.contains("Partition"), "got: {err}");

        let err = validate(Grain::Key, false, None, &cols(&["id"]), "m").unwrap_err();
        assert!(err.contains("mutable_snapshot"), "got: {err}");
    }

    #[test]
    fn tombstone_column_must_exist_in_output() {
        let err = validate(Grain::Key, true, Some("is_departed"), &cols(&["id"]), "m").unwrap_err();
        assert!(err.contains("is_departed"), "got: {err}");

        assert!(validate(
            Grain::Key,
            true,
            Some("is_departed"),
            &cols(&["id", "is_departed"]),
            "m"
        )
        .is_ok());
    }

    #[test]
    fn oracle_exempts_departed_keys() {
        assert_eq!(classify_key(true, false, false), KeyVerdict::Strict);
        assert_eq!(classify_key(true, true, false), KeyVerdict::Strict);
        assert_eq!(classify_key(false, false, false), KeyVerdict::Exempt);
        assert_eq!(classify_key(false, true, true), KeyVerdict::Exempt);
        assert_eq!(
            classify_key(false, true, false),
            KeyVerdict::UnmarkedDeparture
        );
    }

    #[test]
    fn probe_emits_antijoin_over_stored_minus_current() {
        let stmt = emit_departed_key_probe("main.stored", "main.current", &["id"], None);
        assert!(
            stmt.sql.contains("LEFT JOIN main.current c ON s.id = c.id"),
            "{}",
            stmt.sql
        );
        assert!(stmt.sql.contains("WHERE c.id IS NULL"), "{}", stmt.sql);
        assert!(
            stmt.sql.contains("COUNT(*) AS retained_departed_count"),
            "{}",
            stmt.sql
        );
        assert!(!stmt.sql.contains("unmarked_departed_count"));

        let stmt2 =
            emit_departed_key_probe("main.stored", "main.current", &["id"], Some("is_departed"));
        assert!(
            stmt2.sql.contains("unmarked_departed_count"),
            "{}",
            stmt2.sql
        );
    }
}
