//! The legs. Shared machinery consumed by the DuckDB, Spark and BigQuery leg
//! tests, and by the leg-level unit tests in `leg_tests.rs`.

use crate::probe::{self, Position, Probe};
use crate::{divergences, ledger};
use smelt_oracle_testkit::{
    classify_oracle_error, compare_cells, compare_types, Cell, OracleErrorKind, TypeMatch,
    TypeOracle, ValueMatch, ValueOracle,
};
use smelt_types::{BuiltinRegistry, CallFacts, DialectId, SettledEmission};
use std::collections::HashMap;
use std::sync::LazyLock;

/// What one leg actually verified.
///
/// `probes_compared` exists so a leg that "ran" every probe but compared none
/// — because every one was refused — cannot report green. It is the same
/// anti-silent-skip guard `type_property_tests.rs` uses for its BigQuery leg.
#[derive(Debug, Default)]
pub(crate) struct LegOutcome {
    pub(crate) probes_compared: usize,
    /// Probes whose smelt-inferred output type was checked against the
    /// engine's reported one.
    pub(crate) types_compared: usize,
    /// Probes the engine refused that the ledger accounts for.
    pub(crate) registered: Vec<String>,
    /// Probes skipped because the entry is nondeterministic.
    pub(crate) schema_only: Vec<String>,
    /// Unregistered problems. Non-empty means the leg fails.
    pub(crate) failures: Vec<String>,
}

impl LegOutcome {
    pub(crate) fn report(&self) -> String {
        format!(
            "compared={} types={} registered={} schema_only={} failures={}\n{}",
            self.probes_compared,
            self.types_compared,
            self.registered.len(),
            self.schema_only.len(),
            self.failures.len(),
            self.failures.join("\n")
        )
    }
}

/// A probe the ledger already accounts for on this dialect, position, arm and
/// leg. `arm` narrows a row scoped to one `Emission::Conditional` arm; a row
/// with no `arm` of its own still exempts every arm (`ledger::find`'s own
/// None-means-every convention).
fn is_registered(
    name: &str,
    dialect: DialectId,
    position: Position,
    arm: Option<usize>,
    leg: ledger::Leg,
) -> bool {
    ledger::find(name, dialect, position, arm, leg).is_some()
}

/// Whether `sig` settles to `Unsupported` for `facts` at `dialect`/`position` —
/// the pure decision [`is_declared_unsupported`] wraps with a registry
/// lookup, pulled out so it can be exercised against a synthetic
/// `Emission::Conditional` signature directly, the same separation
/// [`classify_accepted`] uses.
///
/// Settled through [`smelt_types::Signature::settle_at`], not looked up via
/// `emission_at`: an arm-specific `Unsupported` verdict must exempt only the
/// arm whose facts resolve to it, never every call to the entry
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
pub(crate) fn settles_unsupported(
    sig: &smelt_types::Signature,
    dialect: DialectId,
    position: Position,
    facts: &CallFacts,
) -> bool {
    matches!(
        sig.settle_at(dialect, position, facts),
        SettledEmission::Unsupported { .. }
    )
}

/// Whether the registry itself declares this entry unsupported on `dialect`
/// at `position`, for `facts` (a conditional entry's declared-unsupported
/// verdict may hold only for some arms — see [`settles_unsupported`]).
///
/// The printer emits such a construct verbatim by design — the compile path
/// refuses the model before printing (`UnsupportedOnBackend`) — so the engine
/// rejecting it here is the *declared* outcome, not a finding. Acceptance would
/// be: it would mean the verdict is wrong.
///
/// Position-scoped, not position-blind: `settle_at` is looked up at the
/// probe's own position (falling back to `Position::Any` internally, never
/// between two concrete positions — `docs/specs/multi_backend.md` §"Emission
/// is scoped to call position"). A helper that always asked about
/// `Position::Any` would never see a verdict declared only at
/// `Position::Window`, and would silently stop exempting — or start wrongly
/// exempting — the exact pairs the position axis exists to distinguish.
fn is_declared_unsupported(
    name: &str,
    dialect: DialectId,
    position: Position,
    facts: &CallFacts,
) -> bool {
    BuiltinRegistry::resolve(name)
        .is_some_and(|sig| settles_unsupported(sig, dialect, position, facts))
}

/// A pair the legs must not treat as a plain pass/fail: either the registry
/// declares it unsupported at this position (for these facts), or the ledger
/// accepts it (at this arm, if any).
fn is_exempt(
    name: &str,
    dialect: DialectId,
    position: Position,
    arm: Option<usize>,
    facts: &CallFacts,
    leg: ledger::Leg,
) -> bool {
    is_declared_unsupported(name, dialect, position, facts)
        || is_registered(name, dialect, position, arm, leg)
}

/// One batched query per (position, group of probes), so a sweep is a few
/// dozen round trips rather than a few hundred. Probes the ledger already
/// accounts for are left out of the batch: one refused probe would otherwise
/// fail the whole group and force the per-probe fallback every time.
fn batch_statement(position: Position, probes: &[&Probe]) -> String {
    let items: Vec<String> = probes
        .iter()
        .map(|p| match position {
            Position::Window => format!(
                "{} OVER (PARTITION BY g ORDER BY rid) AS {}",
                p.expr, p.alias
            ),
            Position::WholePartitionWindow => {
                format!("{} OVER (PARTITION BY g) AS {}", p.expr, p.alias)
            }
            _ => format!("{} AS {}", p.expr, p.alias),
        })
        .collect();
    match position {
        Position::Any => unreachable!("Position::Any is a lookup wildcard, never probed"),
        Position::Scalar => format!("SELECT {} FROM fixture ORDER BY rid", items.join(", ")),
        Position::Aggregate => format!(
            "SELECT g, {} FROM fixture GROUP BY g ORDER BY g",
            items.join(", ")
        ),
        Position::WholePartitionWindow | Position::Window => {
            format!("SELECT {} FROM fixture ORDER BY rid", items.join(", "))
        }
    }
}

/// Group probes by position, preserving derivation order within a group.
fn by_position(probes: &[Probe]) -> Vec<(Position, Vec<&Probe>)> {
    let mut out: Vec<(Position, Vec<&Probe>)> = Vec::new();
    for position in [
        Position::Scalar,
        Position::Aggregate,
        Position::WholePartitionWindow,
        Position::Window,
    ] {
        let group: Vec<&Probe> = probes.iter().filter(|p| p.position == position).collect();
        if !group.is_empty() {
            out.push((position, group));
        }
    }
    out
}

/// Print each probe for the dialect and ask the oracle for its output schema.
///
/// Acceptance is what this leg proves, and it is most of the audit's value: it
/// catches every missing lowering and every construct the target rejects.
/// Comparing the *reported type* against smelt's inference is not repeated
/// here — `type_property_tests` owns that comparison, with its own divergence
/// registry.
pub(crate) fn run_schema_leg(dialect: DialectId, oracle: &dyn TypeOracle) -> LegOutcome {
    let probes = probe::derive_probes();
    let mut outcome = LegOutcome::default();

    for (position, group) in by_position(&probes) {
        let (expected_pass, known): (Vec<&Probe>, Vec<&Probe>) = group.iter().partition(|p| {
            !is_exempt(
                p.name,
                dialect,
                position,
                p.arm,
                &p.facts,
                ledger::Leg::Schema,
            )
        });

        // Fast path: one query for everything expected to work.
        if !expected_pass.is_empty() {
            let sql = probe::print_for(dialect, &batch_statement(position, &expected_pass));
            match oracle.query_types(&sql) {
                Ok(cols) => {
                    let reported: HashMap<String, smelt_types::DataType> = cols
                        .iter()
                        .map(|(n, t)| (n.to_ascii_lowercase(), t.clone()))
                        .collect();
                    for p in &expected_pass {
                        match reported.get(&p.alias) {
                            Some(engine_type) => {
                                outcome.probes_compared += 1;
                                check_inferred_type(
                                    dialect,
                                    p,
                                    engine_type,
                                    outcome_mut(&mut outcome),
                                );
                            }
                            None => outcome.failures.push(format!(
                                "  {} [{:?}] on {}: the batch succeeded but produced no column \
                                 named {}",
                                p.name,
                                p.position,
                                dialect.slug(),
                                p.alias
                            )),
                        }
                    }
                }
                // The batch failed. Re-run one probe per query so the error
                // names the function rather than the group.
                Err(_) => {
                    for p in &expected_pass {
                        probe_schema_once(dialect, oracle, p, &mut outcome);
                    }
                }
            }
        }

        // Ledger-accounted probes always run individually: the point is to
        // confirm the row is still live.
        for p in &known {
            probe_schema_once(dialect, oracle, p, &mut outcome);
        }
    }
    outcome
}

/// What an accepted probe means, decided purely from the two lookups —
/// separated from `probe_schema_once` so the decision (see
/// `a_ledger_row_the_engine_now_accepts_is_reported_stale`) can be proven
/// without an oracle and without depending on which real ledger rows happen
/// to be live at the time: both go to zero for DuckDB once every DuckDB
/// Schema-leg gap is closed, which phase 4 of
/// `docs/outcomes/20260904-dialect-emission-vocabulary` did.
pub(crate) enum AcceptedVerdict {
    /// Compare the engine's reported type against smelt's inference.
    CheckType,
    /// The registry declares this refused, but the engine just accepted it.
    UnsupportedButAccepted,
    /// The ledger still lists this as a gap, but the engine just accepted it.
    StaleLedgerRow,
}

pub(crate) fn classify_accepted(
    is_declared_unsupported: bool,
    is_known_gap: bool,
) -> AcceptedVerdict {
    if is_declared_unsupported {
        AcceptedVerdict::UnsupportedButAccepted
    } else if is_known_gap {
        AcceptedVerdict::StaleLedgerRow
    } else {
        AcceptedVerdict::CheckType
    }
}

fn probe_schema_once(
    dialect: DialectId,
    oracle: &dyn TypeOracle,
    p: &Probe,
    outcome: &mut LegOutcome,
) {
    let sql = probe::print_for(dialect, &p.statement());
    match oracle.query_types(&sql) {
        Ok(cols) => {
            let verdict = classify_accepted(
                is_declared_unsupported(p.name, dialect, p.position, &p.facts),
                is_registered(p.name, dialect, p.position, p.arm, ledger::Leg::Schema),
            );
            match verdict {
                AcceptedVerdict::UnsupportedButAccepted => {
                    outcome.failures.push(format!(
                        "  {} [{:?}] on {}: the registry declares this Unsupported, but the \
                         engine accepts it. Either the verdict is wrong or the printer is \
                         lowering it after all.",
                        p.name,
                        p.position,
                        dialect.slug()
                    ));
                }
                AcceptedVerdict::StaleLedgerRow => {
                    outcome.failures.push(format!(
                        "  {} [{:?}] on {}: STALE LEDGER ROW — the engine now accepts this. \
                         Delete the row and tighten .claude/dialect-gaps-baseline.txt.",
                        p.name,
                        p.position,
                        dialect.slug()
                    ));
                }
                AcceptedVerdict::CheckType => {
                    outcome.probes_compared += 1;
                    if let Some((_, engine_type)) =
                        cols.iter().find(|(n, _)| n.eq_ignore_ascii_case(&p.alias))
                    {
                        check_inferred_type(dialect, p, engine_type, outcome);
                    }
                }
            }
        }
        Err(e) => match classify_oracle_error(&e) {
            OracleErrorKind::QueryRefusal
                if is_exempt(
                    p.name,
                    dialect,
                    p.position,
                    p.arm,
                    &p.facts,
                    ledger::Leg::Schema,
                ) =>
            {
                outcome
                    .registered
                    .push(format!("{} [{:?}]", p.name, p.position));
            }
            OracleErrorKind::QueryRefusal => outcome.failures.push(format!(
                "  {} [{:?}] on {}: refused with `{}`. Either give the entry an \
                 `Emission` verdict in `signatures.rs`, or register the pair in \
                 `ledger.rs` with a reason.\n    probe: {}",
                p.name,
                p.position,
                dialect.slug(),
                e.lines().next().unwrap_or("").trim(),
                p.statement()
            )),
            // The oracle itself is unusable — never "skip" this, or the leg
            // reports green while verifying nothing.
            OracleErrorKind::Fatal => outcome.failures.push(format!(
                "  FATAL oracle error on {} while probing {} [{:?}]: {e}",
                dialect.slug(),
                p.name,
                p.position
            )),
        },
    }
}

/// Identity helper so the batch arm can pass `&mut outcome` while the closure
/// above still borrows it immutably for the failure push.
fn outcome_mut(outcome: &mut LegOutcome) -> &mut LegOutcome {
    outcome
}

/// Compare smelt's inferred output type for `p` against what the engine
/// reported.
///
/// This is the leg the type property tests do **not** cover: they generate from
/// `core_functions()`, a hand-maintained registry-blind table, so most of the
/// registry is never type-checked against any engine at all. Here every entry
/// the enumeration reaches is.
///
/// `compare_types`' `Compatible` verdict (the named string-family leniency and
/// decimal-precision tolerance) counts as agreement, matching
/// `type_property_tests`' convention rather than inventing a second one.
fn check_inferred_type(
    dialect: DialectId,
    p: &Probe,
    engine_type: &smelt_types::DataType,
    outcome: &mut LegOutcome,
) {
    let inferred = probe::infer_types(&p.statement());
    let Some((_, smelt_type)) = inferred
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(&p.alias))
    else {
        // Inference produced no column for this alias at all. That is an
        // inference finding like any other — it goes through the same ledger,
        // rather than being an unregistrable hard failure.
        if is_registered(p.name, dialect, p.position, p.arm, ledger::Leg::Type) {
            outcome.registered.push(format!(
                "{} [{:?}] (no inferred column)",
                p.name, p.position
            ));
        } else {
            outcome.failures.push(format!(
                "  {} [{:?}] on {}: smelt inferred NO COLUMN named {} for its own probe — the \
                 select item did not even yield an alias. Register the pair in `ledger.rs` \
                 with `Leg::Type` and a reason.\n    probe: {}",
                p.name,
                p.position,
                dialect.slug(),
                p.alias,
                p.statement()
            ));
        }
        return;
    };

    outcome.types_compared += 1;
    match compare_types(smelt_type, engine_type) {
        TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
        TypeMatch::Mismatch => {
            // Built once per process: `known_divergences()` rebuilds the whole
            // table on every call, and the type leg asks ~150 times per dialect.
            static KNOWN: LazyLock<Vec<divergences::TypeDivergence>> =
                LazyLock::new(divergences::known_divergences);
            let known =
                divergences::find_divergence(smelt_type, engine_type, dialect.slug(), &KNOWN);
            if let Some(d) = known {
                outcome
                    .registered
                    .push(format!("{} [{:?}] (type: {})", p.name, p.position, d.id));
            } else if is_registered(p.name, dialect, p.position, p.arm, ledger::Leg::Type) {
                outcome
                    .registered
                    .push(format!("{} [{:?}] (type)", p.name, p.position));
            } else {
                outcome.failures.push(format!(
                    "  {} [{:?}] on {}: TYPE MISMATCH — smelt inferred {:?}, {} reported {:?}. \
                     Either fix the inference, register the type pattern in \
                     `prop_helpers/divergences.rs` (preferred — that registry is shared \
                     with `type_property_tests`), or register this one pair in \
                     `ledger.rs` with `Leg::Type`.\n    probe: {}",
                    p.name,
                    p.position,
                    dialect.slug(),
                    smelt_type,
                    dialect.slug(),
                    engine_type,
                    p.statement()
                ));
            }
        }
    }
}

/// Execute each probe on the target and on DuckDB and compare row-wise.
///
/// DuckDB is the reference, matching the repo's oracle convention. This is the
/// leg that catches `^`: a bitwise-XOR reading and a power reading are the same
/// type, so no schema comparison can tell them apart.
pub(crate) fn run_value_leg(
    dialect: DialectId,
    target: &dyn ValueOracle,
    reference: &smelt_oracle_testkit::DuckDbOracle,
) -> LegOutcome {
    let probes = probe::derive_probes();
    let mut outcome = LegOutcome::default();

    for p in &probes {
        if let Some(reason) = p.schema_only {
            outcome
                .schema_only
                .push(format!("{} [{:?}]: {reason}", p.name, p.position));
            continue;
        }
        if is_exempt(
            p.name,
            dialect,
            p.position,
            p.arm,
            &p.facts,
            ledger::Leg::Value,
        ) || is_exempt(
            p.name,
            DialectId::DuckDb,
            p.position,
            p.arm,
            &p.facts,
            ledger::Leg::Value,
        ) {
            outcome
                .registered
                .push(format!("{} [{:?}]", p.name, p.position));
            continue;
        }

        let reference_rows =
            match reference.execute_rows(&probe::print_for(DialectId::DuckDb, &p.statement())) {
                Ok(rows) => rows,
                // The reference cannot answer, so there is nothing to compare
                // against. That is a harness gap, not a dialect finding.
                Err(e) => {
                    outcome.failures.push(format!(
                        "  {} [{:?}]: the DuckDB reference refused its own probe (`{}`), so \
                         nothing on {} can be compared against it",
                        p.name,
                        p.position,
                        e.lines().next().unwrap_or("").trim(),
                        dialect.slug()
                    ));
                    continue;
                }
            };

        match target.execute_rows(&probe::print_for(dialect, &p.statement())) {
            Ok(actual) => {
                outcome.probes_compared += 1;
                if let Some(detail) = first_row_difference(&reference_rows, &actual) {
                    outcome.failures.push(format!(
                        "  {} [{:?}] on {}: VALUE DIVERGENCE {detail}",
                        p.name,
                        p.position,
                        dialect.slug()
                    ));
                }
            }
            Err(e) => match classify_oracle_error(&e) {
                OracleErrorKind::QueryRefusal => outcome.failures.push(format!(
                    "  {} [{:?}] on {}: refused during execution with `{}`",
                    p.name,
                    p.position,
                    dialect.slug(),
                    e.lines().next().unwrap_or("").trim()
                )),
                OracleErrorKind::Fatal => outcome.failures.push(format!(
                    "  FATAL oracle error on {} while executing {} [{:?}]: {e}",
                    dialect.slug(),
                    p.name,
                    p.position
                )),
            },
        }
    }
    outcome
}

/// The first cell that differs between the reference and the target, or `None`
/// when every cell agrees.
pub(crate) fn first_row_difference(
    reference: &[Vec<Cell>],
    actual: &[Vec<Cell>],
) -> Option<String> {
    if reference.len() != actual.len() {
        return Some(format!("row count {} vs {}", reference.len(), actual.len()));
    }
    for (r, (rrow, arow)) in reference.iter().zip(actual).enumerate() {
        if rrow.len() != arow.len() {
            return Some(format!(
                "row {r}: column count {} vs {}",
                rrow.len(),
                arow.len()
            ));
        }
        for (c, (rc, ac)) in rrow.iter().zip(arow).enumerate() {
            if let ValueMatch::Divergent { detail } = compare_cells(rc, ac) {
                return Some(format!("row {r} column {c}: {detail}"));
            }
        }
    }
    None
}

/// The floor below which a leg is not proving anything, whatever it reports.
pub(crate) const PROBE_COVERAGE_FLOOR: usize = 100;
