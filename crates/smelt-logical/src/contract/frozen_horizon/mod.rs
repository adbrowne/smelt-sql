//! `frozen_horizon` — the contract-lattice point declaring that partitions
//! older than `H` are never revisited by maintenance
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)"). This module single-owns the complete triple: the
//! grain-admissibility validator, the pure write-range clamp transform, and
//! the late-arrival probe emitter (baseline snapshot + comparison) that
//! disproves the frozen-band oracle at runtime.

use crate::contract::GrainLabel;
use crate::maintenance::emit::{
    probe_dialect_string_type, MaintenanceDialect, MaintenanceStatement,
};

/// Validates that `frozen_horizon:` is declared only on a partition-grain
/// model. Format validity (a parseable interval) is checked earlier, at
/// frontmatter-parse time, by `smelt_core::metadata`'s strict `contract:`
/// pre-validation (`MetadataError::ContractFrozenHorizonInvalid`) — this
/// function only makes the grain-admissibility check, which needs the
/// model's derived `grain`, unavailable to that pure-parse validator.
///
/// Returns `Err` naming the offending grain when `grain` is not
/// [`GrainLabel::Partition`].
pub fn validate_frozen_horizon(grain: GrainLabel) -> Result<(), String> {
    if grain != GrainLabel::Partition {
        return Err(format!(
            "contract.frozen_horizon is admitted only on a partition-grain model; found grain {grain}"
        ));
    }
    Ok(())
}

/// Validates that `frozen_horizon:`'s driving source has a posture the
/// late-arrival probe can actually observe: the probe is a baseline-vs-
/// current row-count comparison ([`late_arrivals`]), which is sound only
/// when the driving source's row count is non-decreasing —
/// `append_only` (`incremental_models.md` §"The contract lattice": "declaring
/// `frozen_horizon` on a model whose driving source has any other declared
/// mutation profile is refused at declaration time").
///
/// An **undeclared** mutation profile (`profile: None`) is admitted: the
/// spec refuses on "any other *declared* mutation profile", and an
/// undeclared profile declares nothing to contradict the probe — the
/// undeclared case is already policed at run time by
/// `SourceMutationProfileViolated`.
///
/// Returns `Err` naming the offending source and its declared posture when
/// `profile` is `Some` and not `AppendOnly`.
pub fn validate_frozen_horizon_posture(
    driving_source: &str,
    profile: Option<smelt_core::sources::MutationProfile>,
) -> Result<(), String> {
    match profile {
        Some(smelt_core::sources::MutationProfile::AppendOnly) | None => Ok(()),
        Some(other) => Err(format!(
            "contract.frozen_horizon is admitted only when the driving source is append_only; \
             driving source '{driving_source}' declares mutation_profile {other:?}"
        )),
    }
}

/// The frozen-band floor `end - h`: partitions strictly before this point
/// are frozen — a run's write-eligibility clamp never touches them
/// ([`clamp_write_range`]), and the late-arrival probe treats them as
/// closed ([`late_arrivals`]). Both legs derive from this one function so
/// the write clamp and the probe's frozen/unfrozen boundary can never drift
/// apart (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md` —
/// "the band boundary shares one derivation with `clamp_write_range`").
/// Unit-agnostic: the caller supplies `end`/`h` in the same unit (days, in
/// the current `smelt-runtime` call site).
pub fn frozen_band_before(end: i64, h: i64) -> i64 {
    end - h
}

/// Narrows the write-eligible start of a run range to the frozen-horizon
/// floor `end - h`, never widening: `start' = max(start, end - h)`
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/02-plan.md` — "the
/// clamp only ever narrows"). Unit-agnostic: the caller supplies
/// `start`/`end`/`h` in the same unit (days, in the current
/// `smelt-runtime` call site).
pub fn clamp_write_range(start: i64, end: i64, h: i64) -> i64 {
    start.max(frozen_band_before(end, h))
}

/// One partition's row count, the shared shape [`late_arrivals`] compares a
/// recorded baseline against a source's current state with, and the row
/// shape [`emit_frozen_band_snapshot`]'s `SELECT` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionCount {
    /// The partition column's value, as text (ISO date, for the current
    /// day-granularity call sites).
    pub partition_value: String,
    pub row_count: i64,
}

/// A genuine late arrival: a frozen partition (`partition_value <
/// frozen_before`) whose row count increased since the last recorded
/// baseline, or a frozen partition present now with no recorded baseline at
/// all — smelt cannot fail loud on a row it never scans, so this is only
/// ever observed baseline-comparatively, never by scanning
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md`
/// §"Design constraint").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LateArrival {
    pub partition: String,
    /// The row-count increase since the recorded baseline (or the whole
    /// current count, for a frozen partition absent from the baseline —
    /// nothing was recorded to subtract).
    pub added_rows: i64,
}

/// Compares `current` against `baseline`, restricted to partitions strictly
/// before `frozen_before` (string-ISO compare, matching how partition
/// values are already compared elsewhere, e.g. `landed_deltas.rs`'s
/// `Interval` start/end) — a partition at or after `frozen_before` is
/// ordinary maintenance, not a late arrival, even if its count changed.
/// Within the frozen band: a count increase, or a partition present in
/// `current` but absent from `baseline`, is a late arrival; a count
/// decrease or an unchanged count is not (a shrink is out of this point's
/// scope, never silently upgraded to a violation). Pure — no I/O, no
/// baseline persistence; the caller owns reading/writing the recorded
/// baseline.
pub fn late_arrivals(
    baseline: &[PartitionCount],
    current: &[PartitionCount],
    frozen_before: &str,
) -> Vec<LateArrival> {
    use std::collections::HashMap;

    let recorded: HashMap<&str, i64> = baseline
        .iter()
        .map(|p| (p.partition_value.as_str(), p.row_count))
        .collect();

    let mut frozen_current: Vec<&PartitionCount> = current
        .iter()
        .filter(|p| p.partition_value.as_str() < frozen_before)
        .collect();
    frozen_current.sort_by(|a, b| a.partition_value.cmp(&b.partition_value));

    frozen_current
        .into_iter()
        .filter_map(|p| match recorded.get(p.partition_value.as_str()) {
            Some(&recorded_count) if p.row_count > recorded_count => Some(LateArrival {
                partition: p.partition_value.clone(),
                added_rows: p.row_count - recorded_count,
            }),
            Some(_) => None,
            None => Some(LateArrival {
                partition: p.partition_value.clone(),
                added_rows: p.row_count,
            }),
        })
        .collect()
}

/// The per-partition row-count `SELECT` over `source_table`'s frozen band —
/// every partition strictly before `frozen_before` — that both establishes
/// and verifies the frozen-band baseline [`late_arrivals`] compares
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md`: "a
/// partition-filtered scan never reads a row whose partition is already
/// frozen", so this is the only way lateness is observed). `source_table`
/// is already fully qualified (`schema.table`); `frozen_before` is an
/// ISO-formatted partition-column literal.
pub fn emit_frozen_band_snapshot(
    source_table: &str,
    partition_column: &str,
    frozen_before: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let cast_type = probe_dialect_string_type(dialect);
    let escaped = frozen_before.replace('\'', "''");
    let sql = format!(
        "SELECT CAST({partition_column} AS {cast_type}) AS partition_value, \
         COUNT(*) AS row_count \
         FROM {source_table} \
         WHERE CAST({partition_column} AS {cast_type}) < '{escaped}' \
         GROUP BY {partition_column}"
    );
    MaintenanceStatement::new(sql)
}

#[cfg(test)]
mod tests;
