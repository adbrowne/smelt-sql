//! `retain_departed` — the contract-lattice point declaring that keys a
//! keyed model's mutable-snapshot source no longer carries are kept (or
//! tombstoned) at reconcile instead of deleted
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)", §"Retention (`retain_departed`)"). This module
//! single-owns the complete triple: the posture-admissibility +
//! tombstone-column validator, the pure departed-key quotient oracle, the
//! reconcile-anti-join probe emitter, and — via [`reconcile_disposition`] —
//! the write-path seam that resolves a declaration to what the reconcile
//! write must do: run the default point's anti-join delete
//! (`crate::maintenance::emit::emit_departed_key_delete`), or suppress it and
//! dispatch the probe instead (`docs/outcomes/
//! 20260815-definition-delta-migrate/phases/32b-plan.md`).
//!
//! Unlike `frozen_horizon`/`deferral`, this point has no write-eligibility
//! clamp and no scheduling license of its own — declaring it changes what
//! the default reconcile write is required to prove, not when a cell runs
//! (`incremental_models.md` §"The equivalence invariant", key departure).

use smelt_core::config::RetainDeparted;

use crate::contract::GrainLabel;
use crate::maintenance::emit::MaintenanceStatement;

/// The runtime write-path disposition a reconcile write must apply for
/// departed keys — the seam `execute_snapshot_reconcile` consults instead of
/// inspecting `RetainDeparted` itself, so this module stays the single owner
/// of what a declaration *means* at write time
/// (`incremental_models.md` §"Contract-lattice point single ownership").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepartedKeyDisposition {
    /// The default point: run `emit_departed_key_delete` as part of the
    /// same transactional write as the reconcile merge.
    Delete,
    /// The declared `retain_departed` point: suppress the delete leg
    /// entirely. `tombstone` names the column the probe should read to spot
    /// an unmarked departure, when one was declared.
    Retain { tombstone: Option<String> },
}

/// Resolve a model's declared `contract.retain_departed` (or its absence)
/// to the write-path disposition. Pure: no default point exists that is not
/// exactly "absent declaration".
pub fn reconcile_disposition(declared: Option<&RetainDeparted>) -> DepartedKeyDisposition {
    match declared {
        None => DepartedKeyDisposition::Delete,
        Some(RetainDeparted::Bool(true)) => DepartedKeyDisposition::Retain { tombstone: None },
        Some(RetainDeparted::Bool(false)) => DepartedKeyDisposition::Delete,
        Some(RetainDeparted::Tombstone { tombstone }) => DepartedKeyDisposition::Retain {
            tombstone: Some(tombstone.clone()),
        },
    }
}

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
    grain: GrainLabel,
    consumes_mutable_snapshot: bool,
    tombstone_column: Option<&str>,
    output_columns: &[String],
    model_name: &str,
) -> Result<(), String> {
    if grain != GrainLabel::Key {
        return Err(format!(
            "contract.retain_departed is admitted only on a keyed shape consuming a mutable \
             snapshot; model '{model_name}' has grain {grain}"
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
mod tests;
