//! Offline totality gates over the `BuiltinRegistry`, the derived probe set,
//! the override table and the fixture — no warehouse needed.

use crate::probe;
use crate::{fixture, overrides, AUDITED_DIALECTS};
use smelt_oracle_testkit::{DuckDbOracle, TrinoOracle, ValueOracle};
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
            // `probe_or_reason` never derives a conditional arm — that is
            // `conditional_arm_probes`'s own reason type, kept in the same
            // enum so an unreachable arm and an underivable entry share one
            // reporting vocabulary.
            Err(probe::NotProbed::UnreachableArm { .. }) => unreachable!(
                "probe_or_reason never returns UnreachableArm; that is conditional_arm_probes's own finding"
            ),
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
fn aggregates_are_probed_in_all_three_positions() {
    // MEDIAN proves the lowering differs per position; probing fewer would
    // have missed the BigQuery aggregate form, or the whole-partition-window
    // restructure, entirely.
    let probes = probe::derive_probes();
    let median: Vec<_> = probes.iter().filter(|p| p.name == "MEDIAN").collect();
    assert_eq!(median.len(), 3, "{median:#?}");
    assert!(median
        .iter()
        .any(|p| p.position == probe::Position::Aggregate));
    assert!(median
        .iter()
        .any(|p| p.position == probe::Position::WholePartitionWindow));
    assert!(median.iter().any(|p| p.position == probe::Position::Window));
}

/// The positions `sig.kind` implies a probe at, computed independently of
/// `probe::positions` — this test's whole point is to catch a regression to
/// *that* function, so it cannot call it. Mirrors its documented contract:
/// a scalar is probed once, a window-only entry is probed once, an
/// aggregate is probed in all three of its positions
/// (`docs/specs/multi_backend.md` §"Cross-engine emission audit").
fn expected_positions_for(kind: smelt_types::ExprKind) -> Vec<probe::Position> {
    use smelt_types::ExprKind;
    match kind {
        ExprKind::Scalar => vec![probe::Position::Scalar],
        ExprKind::Agg => vec![
            probe::Position::Aggregate,
            probe::Position::WholePartitionWindow,
            probe::Position::Window,
        ],
        ExprKind::Window => vec![probe::Position::Window],
    }
}

/// Genuine per-entry totality: every registry entry that yields a probe at
/// all is probed at **every** position its `ExprKind` implies — not just
/// `MEDIAN`. `aggregates_are_probed_in_all_three_positions` above pins one
/// named entry so a regression is easy to read, but it cannot catch a bug
/// that drops one position globally except when it happens to land on
/// `MEDIAN`: removing `Position::WholePartitionWindow` from
/// `probe::positions`'s `ExprKind::Agg` arm silently drops the
/// whole-partition-window probe for every OTHER aggregate — `ARG_MAX`,
/// `PERCENTILE_CONT`, `APPROX_COUNT_DISTINCT`, … — while that one test still
/// passes only because it happens to check `MEDIAN` by name. This test
/// checks every entry, and names the dropped one directly, closing that
/// gap for good.
#[test]
fn every_probeable_entry_is_probed_at_every_position_its_kind_implies() {
    let probes = probe::derive_probes();
    let mut missing = Vec::new();
    for name in BuiltinRegistry::names() {
        let sig = BuiltinRegistry::resolve(name).expect("names() resolves");
        // An entry with no derivable probe at all is
        // `every_registry_entry_yields_a_probe_or_a_recorded_reason`'s gap to
        // catch, not this one's — it has no probes to check positions on.
        let Ok(entry_probes) = probe::probe_or_reason(sig) else {
            continue;
        };
        let Some(canonical) = entry_probes.first().map(|p| p.name) else {
            continue;
        };
        for position in expected_positions_for(sig.kind) {
            let present = probes
                .iter()
                .any(|p| p.name == canonical && p.position == position);
            if !present {
                missing.push(format!("{canonical} at {position:?}"));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "{} (registry entry, position) pairs are missing from the probe \
         enumeration — the entry's `ExprKind` implies a probe there, but \
         `probe::derive_probes` produced none. This is a silently narrowed \
         audit, not a recorded gap: fix `probe::positions` rather than \
         adding an override.\n{}",
        missing.len(),
        missing.join("\n")
    );
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
    for d in AUDITED_DIALECTS {
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

/// Live. Selects named columns rather than `*` so `arr_int`/`bin_blob`/
/// `iv_interval` — none of which `trino_type_to_arrow` decodes yet — are not
/// in scope; this test proves the fixture executes and yields the right row
/// count, not that every column decodes.
#[test]
fn the_trino_fixture_executes_and_yields_eight_rows() {
    let Some(oracle) = TrinoOracle::from_env() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping the_trino_fixture_executes_and_yields_eight_rows"
        );
        return;
    };
    let sql = format!(
        "{}SELECT rid, g, n_int, n_bigint, n_double, n_dec, s_text, b_bool, d_date, ts_ts \
         FROM fixture",
        fixture::fixture_cte(DialectId::Trino)
    );
    let count = oracle
        .row_count(&sql)
        .unwrap_or_else(|e| panic!("fixture must execute: {e}\n{sql}"));
    assert_eq!(count, fixture::ROW_COUNT);
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
    for d in AUDITED_DIALECTS {
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
