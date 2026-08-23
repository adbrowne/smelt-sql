//! The cross-engine emission audit.
//!
//! Enumerates every `BuiltinRegistry` entry against every dialect and proves
//! the enumeration is **total** — an entry with no probe is reported by name,
//! never dropped. Offline; the engine legs land alongside it.

mod fixture;
mod overrides;
mod probe;

use smelt_oracle_testkit::{DuckDbOracle, ValueOracle};
use smelt_types::{BuiltinRegistry, DialectId};

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
#[test]
fn every_override_names_a_real_registry_entry() {
    let unknown: Vec<&str> = overrides::overrides()
        .iter()
        .map(|o| o.name)
        .filter(|n| BuiltinRegistry::resolve(n).is_none())
        .collect();
    assert!(
        unknown.is_empty(),
        "these override rows name nothing in the registry — delete them rather \
         than leaving a dead exemption: {unknown:?}"
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
