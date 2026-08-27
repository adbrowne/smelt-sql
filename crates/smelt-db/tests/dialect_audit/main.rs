//! The cross-engine emission audit.
//!
//! Enumerates every `BuiltinRegistry` entry against every dialect and proves
//! the enumeration is **total** — an entry with no probe is reported by name,
//! never dropped. Offline; the engine legs land alongside it.

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

use smelt_oracle_testkit::{
    classify_oracle_error, compare_cells, compare_types, BigQueryOracle, Cell, DuckDbOracle,
    OracleErrorKind, SparkOracle, TypeMatch, TypeOracle, ValueMatch, ValueOracle,
};
use smelt_types::{BuiltinRegistry, DialectId};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use probe::{Position, Probe};

#[test]
fn every_registry_entry_yields_a_probe_or_a_recorded_reason() {
    let mut underivable = Vec::new();
    for name in BuiltinRegistry::names() {
        let sig = BuiltinRegistry::resolve(name).expect("names() resolves");
        match probe::probe_or_reason(sig) {
            Ok(probes) => assert!(!probes.is_empty(), "{name} yielded an empty probe set"),
            Err(probe::NotProbed::Underivable { detail }) => {
                underivable.push(format!("  {name}: {detail}"));
            }
        }
    }
    underivable.sort();
    assert!(
        underivable.is_empty(),
        "{} registry entries have no derivable probe and no override. Add a row \
         to `overrides.rs` — with `schema_only` and a reason if the entry is \
         nondeterministic — rather than narrowing the enumeration:\n{}",
        underivable.len(),
        underivable.join("\n")
    );
}

#[test]
fn aggregates_are_probed_in_both_positions() {
    // MEDIAN proves the lowering differs per position; probing one position
    // would have missed the BigQuery aggregate form entirely.
    let probes = probe::derive_probes();
    let median: Vec<_> = probes.iter().filter(|p| p.name == "MEDIAN").collect();
    assert_eq!(median.len(), 2, "{median:#?}");
    assert!(median
        .iter()
        .any(|p| p.position == probe::Position::Aggregate));
    assert!(median.iter().any(|p| p.position == probe::Position::Window));
}

#[test]
fn every_special_form_entry_has_a_spelling_override() {
    for name in BuiltinRegistry::names() {
        let sig = BuiltinRegistry::resolve(name).expect("resolves");
        if sig.syntax_form != smelt_types::SyntaxForm::Special {
            continue;
        }
        assert!(
            overrides::overrides()
                .iter()
                .any(|o| o.name == name && o.spelling.is_some()),
            "{name} is SyntaxForm::Special and has no spelling override; a Special \
             entry has no uniform shape the harness can derive"
        );
    }
}

#[test]
fn probe_aliases_are_unique() {
    // Probes are batched into one SELECT per (dialect, shape); a duplicate
    // alias would silently drop a probe from the batch.
    let probes = probe::derive_probes();
    let mut aliases: Vec<&str> = probes.iter().map(|p| p.alias.as_str()).collect();
    let total = aliases.len();
    aliases.sort_unstable();
    aliases.dedup();
    assert_eq!(aliases.len(), total, "duplicate probe alias");
}

#[test]
fn the_fixture_has_a_column_for_every_type_constraint_family() {
    for d in DialectId::ALL {
        let cte = fixture::fixture_cte(*d);
        for (col, _) in fixture::COLUMNS {
            assert!(cte.contains(col), "{} fixture lacks {col}", d.slug());
        }
        assert!(
            !cte.contains("'NULL'"),
            "{} fixture contains the literal string NULL, which Spark's text \
             rendering cannot distinguish from a real NULL",
            d.slug()
        );
    }
}

#[test]
fn the_duckdb_fixture_executes_and_yields_eight_rows() {
    let oracle = DuckDbOracle::new();
    let sql = format!(
        "{}SELECT * FROM fixture",
        fixture::fixture_cte(DialectId::DuckDb)
    );
    let rows = oracle
        .execute_rows(&sql)
        .unwrap_or_else(|e| panic!("fixture must execute: {e}\n{sql}"));
    assert_eq!(rows.len(), fixture::ROW_COUNT);
}

/// An override naming an entry the registry does not have is dead weight that
/// reads as coverage. Two-sided, like every other ledger in this repo.
///
/// The row must also name the **canonical** entry, not one of its aliases:
/// `overrides::find` looks rows up by canonical name, so an alias-named row
/// resolves in the registry and still never applies. That is worse than a
/// missing row, because it reads as covered. Caught for real —
/// `JSON_EXTRACT_STRING` was such a row until the live sweep tripped over it.
#[test]
fn every_override_names_a_real_canonical_registry_entry() {
    let mut bad = Vec::new();
    for o in overrides::overrides() {
        match BuiltinRegistry::canonical_name(o.name) {
            None => bad.push(format!("  {}: names nothing in the registry", o.name)),
            Some(canonical) if canonical != o.name => bad.push(format!(
                "  {}: is an alias of `{canonical}`, so this row never applies — \
                 rename it to the canonical entry",
                o.name
            )),
            Some(_) => {}
        }
    }
    assert!(
        bad.is_empty(),
        "dead override rows — each reads as coverage while covering nothing:\n{}",
        bad.join("\n")
    );
}

/// Two rows for one name means the second is dead: `overrides::find` returns
/// the first match, so a later correction would silently never apply.
#[test]
fn override_names_are_unique() {
    let mut names: Vec<&str> = overrides::overrides().iter().map(|o| o.name).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        total,
        "duplicate override row; only the first would ever apply"
    );
}

/// Every probe prints for every dialect without panicking, and the printed SQL
/// still carries the probe's alias.
///
/// The offline half of the audit: it needs no warehouse, and it is what would
/// catch a lowering that drops or renames the projection alias the value leg
/// keys on.
#[test]
fn every_probe_prints_for_every_dialect() {
    let probes = probe::derive_probes();
    assert!(!probes.is_empty());
    for d in DialectId::ALL {
        for p in &probes {
            let sql = probe::print_for(*d, &p.statement());
            assert!(
                sql.contains(&p.alias),
                "{} lost the alias {} while printing {}: {sql}",
                d.slug(),
                p.alias,
                p.name
            );
            assert!(
                sql.contains("WITH fixture"),
                "{} lost the fixture CTE",
                d.slug()
            );
        }
    }
}

/// A nondeterministic entry is probed, but only on the schema leg, and the
/// reason is recorded rather than implied.
#[test]
fn nondeterministic_entries_are_schema_only_with_a_reason() {
    let probes = probe::derive_probes();
    for name in ["RANDOM", "NOW", "CURRENT_DATE", "CURRENT_TIMESTAMP"] {
        let found: Vec<_> = probes.iter().filter(|p| p.name == name).collect();
        assert!(!found.is_empty(), "{name} is not probed at all");
        for p in found {
            assert!(
                p.schema_only.is_some(),
                "{name} executes at a different instant per engine; its value leg \
                 must be skipped with a recorded reason, not compared"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Ledger gates. Static data, so these need no warehouse and run per-PR.
// ─────────────────────────────────────────────────────────────────────────

fn baseline(metric: &str) -> usize {
    include_str!("../../../../.claude/dialect-gaps-baseline.txt")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .find_map(|l| {
            let (k, v) = l.trim().split_once(' ')?;
            if k != metric {
                return None;
            }
            v.trim().parse::<usize>().ok()
        })
        .unwrap_or_else(|| panic!("`{metric}` not found in .claude/dialect-gaps-baseline.txt"))
}

#[test]
fn gap_count_ratchet() {
    for d in DialectId::ALL {
        let metric = format!("dialect_gaps_{}", d.slug());
        let current = ledger::dialect_divergences()
            .iter()
            .filter(|r| r.dialect == *d && matches!(r.verdict, ledger::Verdict::Gap { .. }))
            .count();
        let base = baseline(&metric);
        assert!(
            current <= base,
            "Registered dialect-gap count REGRESSED for {}: current={current} > baseline={base}.\n\
             A new gap must be justified by editing .claude/dialect-gaps-baseline.txt \
             (reviewer-visible), never absorbed silently.",
            d.slug()
        );
        assert!(
            current >= base,
            "STALE baseline for {}: current={current} < baseline={base}.\n\
             A lowering closed a gap — tighten .claude/dialect-gaps-baseline.txt to {current}.",
            d.slug()
        );
    }
}

#[test]
fn every_ledger_row_names_a_real_registry_entry_and_a_probed_pair() {
    // The unreachable-row direction: a row naming an entry the registry no
    // longer has, or a pair the harness never probes, can never fire — and
    // reads as coverage while covering nothing.
    let probed: HashSet<&str> = probe::derive_probes().iter().map(|p| p.name).collect();
    let mut orphans = Vec::new();
    for row in ledger::dialect_divergences() {
        if BuiltinRegistry::resolve(row.name).is_none() {
            orphans.push(format!(
                "  {} ({}): no such registry entry",
                row.name,
                row.dialect.slug()
            ));
        } else if !probed.contains(row.name) {
            orphans.push(format!(
                "  {} ({}): entry is never probed, so this row can never fire",
                row.name,
                row.dialect.slug()
            ));
        }
    }
    assert!(
        orphans.is_empty(),
        "ORPHANED LEDGER ROWS — registered but unreachable. Delete them:\n{}",
        orphans.join("\n")
    );
}

/// One row per `(entry, dialect, position, leg)`. The key includes the leg
/// because a pair can legitimately be registered on more than one: `DATE_ADD`
/// on BigQuery both infers the wrong type and returns a different value.
#[test]
fn a_pair_has_at_most_one_ledger_row() {
    let mut seen = HashSet::new();
    for row in ledger::dialect_divergences() {
        assert!(
            seen.insert((row.name, row.dialect, row.position, row.leg)),
            "duplicate ledger row for {} on {} ({:?}, {:?})",
            row.name,
            row.dialect.slug(),
            row.position,
            row.leg
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The legs.
// ─────────────────────────────────────────────────────────────────────────

/// What one leg actually verified.
///
/// `probes_compared` exists so a leg that "ran" every probe but compared none
/// — because every one was refused — cannot report green. It is the same
/// anti-silent-skip guard `type_property_tests.rs` uses for its BigQuery leg.
#[derive(Debug, Default)]
struct LegOutcome {
    probes_compared: usize,
    /// Probes whose smelt-inferred output type was checked against the
    /// engine's reported one.
    types_compared: usize,
    /// Probes the engine refused that the ledger accounts for.
    registered: Vec<String>,
    /// Probes skipped because the entry is nondeterministic.
    schema_only: Vec<String>,
    /// Unregistered problems. Non-empty means the leg fails.
    failures: Vec<String>,
}

impl LegOutcome {
    fn report(&self) -> String {
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

/// A probe the ledger already accounts for on this dialect, position and leg.
fn is_registered(name: &str, dialect: DialectId, position: Position, leg: ledger::Leg) -> bool {
    ledger::find(name, dialect, position, leg).is_some()
}

/// Whether the registry itself declares this entry unsupported on `dialect`.
///
/// The printer emits such a construct verbatim by design — the compile path
/// refuses the model before printing (`UnsupportedOnBackend`) — so the engine
/// rejecting it here is the *declared* outcome, not a finding. Acceptance would
/// be: it would mean the verdict is wrong.
fn is_declared_unsupported(name: &str, dialect: DialectId) -> bool {
    BuiltinRegistry::resolve(name).is_some_and(|sig| {
        matches!(
            sig.emission_at(dialect, smelt_types::signatures::Position::Any),
            smelt_types::Emission::Unsupported { .. }
        )
    })
}

/// A pair the legs must not treat as a plain pass/fail: either the registry
/// declares it unsupported, or the ledger accepts it.
fn is_exempt(name: &str, dialect: DialectId, position: Position, leg: ledger::Leg) -> bool {
    is_declared_unsupported(name, dialect) || is_registered(name, dialect, position, leg)
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
            _ => format!("{} AS {}", p.expr, p.alias),
        })
        .collect();
    match position {
        Position::Scalar => format!("SELECT {} FROM fixture ORDER BY rid", items.join(", ")),
        Position::Aggregate => format!(
            "SELECT g, {} FROM fixture GROUP BY g ORDER BY g",
            items.join(", ")
        ),
        Position::Window => format!("SELECT {} FROM fixture ORDER BY rid", items.join(", ")),
    }
}

/// Group probes by position, preserving derivation order within a group.
fn by_position(probes: &[Probe]) -> Vec<(Position, Vec<&Probe>)> {
    let mut out: Vec<(Position, Vec<&Probe>)> = Vec::new();
    for position in [Position::Scalar, Position::Aggregate, Position::Window] {
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
fn run_schema_leg(dialect: DialectId, oracle: &dyn TypeOracle) -> LegOutcome {
    let probes = probe::derive_probes();
    let mut outcome = LegOutcome::default();

    for (position, group) in by_position(&probes) {
        let (expected_pass, known): (Vec<&Probe>, Vec<&Probe>) = group
            .iter()
            .partition(|p| !is_exempt(p.name, dialect, position, ledger::Leg::Schema));

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

fn probe_schema_once(
    dialect: DialectId,
    oracle: &dyn TypeOracle,
    p: &Probe,
    outcome: &mut LegOutcome,
) {
    let sql = probe::print_for(dialect, &p.statement());
    match oracle.query_types(&sql) {
        Ok(cols) => {
            if is_declared_unsupported(p.name, dialect) {
                outcome.failures.push(format!(
                    "  {} [{:?}] on {}: the registry declares this Unsupported, but the \
                     engine accepts it. Either the verdict is wrong or the printer is \
                     lowering it after all.",
                    p.name,
                    p.position,
                    dialect.slug()
                ));
            } else if is_registered(p.name, dialect, p.position, ledger::Leg::Schema) {
                outcome.failures.push(format!(
                    "  {} [{:?}] on {}: STALE LEDGER ROW — the engine now accepts this. \
                     Delete the row and tighten .claude/dialect-gaps-baseline.txt.",
                    p.name,
                    p.position,
                    dialect.slug()
                ));
            } else {
                outcome.probes_compared += 1;
                if let Some((_, engine_type)) =
                    cols.iter().find(|(n, _)| n.eq_ignore_ascii_case(&p.alias))
                {
                    check_inferred_type(dialect, p, engine_type, outcome);
                }
            }
        }
        Err(e) => match classify_oracle_error(&e) {
            OracleErrorKind::QueryRefusal
                if is_exempt(p.name, dialect, p.position, ledger::Leg::Schema) =>
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
        if is_registered(p.name, dialect, p.position, ledger::Leg::Type) {
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
            } else if is_registered(p.name, dialect, p.position, ledger::Leg::Type) {
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
fn run_value_leg(
    dialect: DialectId,
    target: &dyn ValueOracle,
    reference: &DuckDbOracle,
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
        if is_exempt(p.name, dialect, p.position, ledger::Leg::Value)
            || is_exempt(p.name, DialectId::DuckDb, p.position, ledger::Leg::Value)
        {
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
fn first_row_difference(reference: &[Vec<Cell>], actual: &[Vec<Cell>]) -> Option<String> {
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
const PROBE_COVERAGE_FLOOR: usize = 100;

#[test]
fn schema_leg_duckdb() {
    let oracle = DuckDbOracle::new();
    let outcome = run_schema_leg(DialectId::DuckDb, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= PROBE_COVERAGE_FLOOR,
        "schema leg compared only {} probes — the enumeration collapsed",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[duckdb schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_duckdb_is_self_consistent() {
    // DuckDB against itself: proves the harness, the fixture and the comparator
    // agree before any cross-engine claim is made.
    let oracle = DuckDbOracle::new();
    let outcome = run_value_leg(DialectId::DuckDb, &oracle, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= PROBE_COVERAGE_FLOOR,
        "value leg compared only {} probes — the enumeration collapsed",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[duckdb value] probes_compared={} schema_only={}",
        outcome.probes_compared,
        outcome.schema_only.len()
    );
}

/// The leg's comparator actually reports a difference — a self-consistent
/// green run proves the plumbing, not the detection.
///
/// The planted values are the real case: DuckDB's `2 ^ 3` is 8, and a dialect
/// reading `^` as bitwise XOR answers 1. Both are the same type, so no schema
/// comparison can tell them apart.
#[test]
fn the_value_leg_reports_a_planted_divergence() {
    let reference = vec![vec![Cell::Int(8)]];
    let xor_reading = vec![vec![Cell::Int(1)]];
    let detail =
        first_row_difference(&reference, &xor_reading).expect("8 and 1 are not the same number");
    assert!(detail.contains("row 0 column 0"), "{detail}");

    // …and a shorter result set is a difference too, not a silently truncated
    // comparison.
    assert!(first_row_difference(&reference, &[]).is_some());
    assert!(first_row_difference(&reference, &reference).is_none());
}

/// A ledger row that the engine has started accepting is reported as stale
/// rather than left standing — the same two-sidedness the hardening baseline
/// has. Proven by pointing the check at a pair with a live row and a query the
/// engine does accept.
#[test]
fn a_ledger_row_the_engine_now_accepts_is_reported_stale() {
    let oracle = DuckDbOracle::new();
    let mut outcome = LegOutcome::default();
    let accepted = Probe {
        // A registered DuckDB gap…
        name: "INITCAP",
        position: Position::Scalar,
        // …but an expression DuckDB accepts, standing in for the day the
        // lowering lands.
        expr: "n_bigint".to_string(),
        alias: "p_initcap_scalar".to_string(),
        schema_only: None,
    };
    probe_schema_once(DialectId::DuckDb, &oracle, &accepted, &mut outcome);
    assert!(
        outcome
            .failures
            .iter()
            .any(|f| f.contains("STALE LEDGER ROW")),
        "{}",
        outcome.report()
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Spark. Skips green when `SPARK_CONTAINER_ID` is unset; a live Spark Connect
// container is a labelled-PR / nightly cost, not a per-PR one.
// ─────────────────────────────────────────────────────────────────────────

static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| SparkOracle::new(&id))
});

#[test]
fn schema_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping schema_leg_spark");
        return;
    };
    let outcome = run_schema_leg(DialectId::SparkSql, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[spark schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping value_leg_spark");
        return;
    };
    let outcome = run_value_leg(DialectId::SparkSql, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[spark value] probes_compared={}",
        outcome.probes_compared
    );
}

/// The regression test for the finding that motivated this work: Spark's infix
/// `^` is bitwise XOR, not exponentiation. Before the emission row that lowers
/// it to `POWER(a, b)`, `SELECT 2 ^ 3` returned 1 on Spark and 8 on DuckDB — a
/// silently wrong number, not an error, and the same type on both engines, so
/// no schema comparison could see it.
#[test]
fn spark_caret_agrees_with_duckdb_power() {
    let Some(spark) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping spark_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY rid";
    let spark_rows = spark
        .execute_rows(&probe::print_for(DialectId::SparkSql, smelt_expr))
        .expect("spark");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_expr))
        .expect("duckdb");
    assert_eq!(spark_rows.len(), duck_rows.len());
    for (s, d) in spark_rows.iter().zip(&duck_rows) {
        assert_eq!(
            compare_cells(&d[0], &s[0]),
            ValueMatch::Equal,
            "`^` diverges on Spark: it is bitwise XOR there, and must be lowered to POWER"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// BigQuery. Manual sweep only, per `multi_backend.md` §"BigQuery has no CI
// tier, by decision, not by omission" — and more so here than anywhere else,
// because the value leg *executes* rather than dry-runs, so it bills.
//
// Every test below skips green without a credential, which is why
// `scripts/bigquery-dialect-audit.sh` refuses to start without one rather
// than letting a green skip read as a passing sweep.
// ─────────────────────────────────────────────────────────────────────────

static BIGQUERY: LazyLock<Option<BigQueryOracle>> = LazyLock::new(BigQueryOracle::from_env);

#[test]
fn schema_leg_bigquery() {
    let Some(oracle) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping schema_leg_bigquery");
        return;
    };
    let outcome = run_schema_leg(DialectId::BigQuery, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[bigquery schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_bigquery() {
    let Some(oracle) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping value_leg_bigquery");
        return;
    };
    let outcome = run_value_leg(DialectId::BigQuery, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[bigquery value] probes_compared={}",
        outcome.probes_compared
    );
}

/// GoogleSQL defines infix `^` as bitwise XOR, exactly as Spark does. The same
/// silent-wrong-number hazard, on the backend where a wrong number is most
/// expensive to discover.
#[test]
fn bigquery_caret_agrees_with_duckdb_power() {
    let Some(bq) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping bigquery_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY rid";
    let bq_rows = bq
        .execute_rows(&probe::print_for(DialectId::BigQuery, smelt_expr))
        .expect("bigquery");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_expr))
        .expect("duckdb");
    assert_eq!(bq_rows.len(), duck_rows.len());
    for (b, d) in bq_rows.iter().zip(&duck_rows) {
        assert_eq!(
            compare_cells(&d[0], &b[0]),
            ValueMatch::Equal,
            "`^` diverges on BigQuery: GoogleSQL reads it as bitwise XOR"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The published table.
// ─────────────────────────────────────────────────────────────────────────

const COVERAGE_DOC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/reference/dialect-coverage.md"
);

#[test]
fn the_coverage_table_matches_the_registry() {
    let rendered = report::render();
    let on_disk = std::fs::read_to_string(COVERAGE_DOC).unwrap_or_default();
    if std::env::var("SMELT_REGEN_DOCS").as_deref() == Ok("1") {
        if on_disk != rendered {
            std::fs::write(COVERAGE_DOC, &rendered).expect("write coverage doc");
        }
        return;
    }
    assert_eq!(
        on_disk, rendered,
        "docs/reference/dialect-coverage.md is stale. Regenerate with:\n  \
         SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \
         the_coverage_table_matches_the_registry"
    );
}

/// Totality on the published side. The table is the deliverable, and a gate
/// that only checked freshness would let an entry vanish from it silently.
#[test]
fn every_entry_and_dialect_appears_in_the_table() {
    let rendered = report::render();
    for name in BuiltinRegistry::names() {
        assert!(
            rendered.contains(&format!("| `{name}` |")),
            "{name} missing from the table"
        );
    }
    // Every dialect has a verification-tier row: the table's honesty depends
    // on saying which cells a live leg actually visits.
    for label in ["DuckDB", "Spark SQL", "PostgreSQL", "BigQuery"] {
        assert!(
            rendered.contains(&format!("| {label} |")),
            "{label} has no verification-tier row"
        );
    }
}
