//! The cross-engine emission audit.
//!
//! Enumerates every `BuiltinRegistry` entry against every dialect and proves
//! the enumeration is **total** — an entry with no probe is reported by name,
//! never dropped. Offline; the engine legs land alongside it.

mod census;
mod fixture;
mod ledger;

/// The existing type-divergence registry, shared rather than duplicated.
///
/// `type_property_tests` already records every known smelt-versus-engine type
/// difference here, with a `// verified:` provenance line per row. Building a
/// second registry for the same facts is the two-sources-of-truth problem
/// single ownership exists to avoid, so the type leg consults this one — it
/// simply reaches many more entries than the property sweep's generators do.
// The audit reads only `id` and the type patterns; the property sweep that owns
// this file reads the rest. Compiling a shared module into a second binary that
// happens to use less of it is not dead code in any meaningful sense.
#[allow(dead_code)]
#[path = "../prop_helpers/divergences.rs"]
mod divergences;
mod overrides;
mod probe;
mod report;

mod bigquery;
mod coverage_table;
mod ledger_gates;
mod leg_tests;
mod legs;
mod registry_totality;
mod spark;
mod trino;

use smelt_types::DialectId;

/// Dialects this audit actually covers, as opposed to every `DialectId` that
/// exists.
///
/// `DialectId::ALL` is the exhaustive identity enumeration (criterion 2 of
/// `20260913-trino-target-spine`). This audit's fixtures, probes and gap
/// baseline are a separate, narrower claim, driving the offline totality
/// gates (the fixture gate and the print-for-every-dialect gate) — it does
/// **not** mean a member has both legs live. Trino joined this list in
/// `20260913-trino-emission` phase 5 with only its schema leg live; the
/// `census` module's `Verified` classification consults a separate,
/// narrower set for that reason (see `census::BOTH_LEGS_LIVE`).
const AUDITED_DIALECTS: &[DialectId] = &[
    DialectId::DuckDb,
    DialectId::SparkSql,
    DialectId::BigQuery,
    DialectId::Trino,
];
