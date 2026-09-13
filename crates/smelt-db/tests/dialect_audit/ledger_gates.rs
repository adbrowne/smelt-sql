//! Ledger gates. Static data, so these need no warehouse and run per-PR.

use crate::legs::settles_unsupported;
use crate::probe::{self, Position};
use crate::{ledger, AUDITED_DIALECTS};
use smelt_types::{BuiltinRegistry, CallFacts, DialectId, SettledEmission};
use std::collections::HashSet;

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

const BASELINE_SRC: &str = include_str!("../../../../.claude/dialect-gaps-baseline.txt");

/// Two-sided: every `dialect_gaps_*` metric in the baseline corresponds to a
/// `DialectId::ALL` slug, and every slug has a metric. Catches the retired-
/// dialect class directly — a stale `dialect_gaps_postgres` line would
/// otherwise sit unread forever, since `baseline()` only looks up metrics by
/// name and never notices an entry nobody asks for.
#[test]
fn baseline_names_exactly_the_audited_dialects() {
    let baseline_metrics: HashSet<String> = BASELINE_SRC
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| l.trim().split_once(' ').map(|(k, _)| k.to_string()))
        .collect();
    let expected: HashSet<String> = AUDITED_DIALECTS
        .iter()
        .map(|d| format!("dialect_gaps_{}", d.slug()))
        .collect();
    let stale: Vec<_> = baseline_metrics.difference(&expected).collect();
    assert!(
        stale.is_empty(),
        "baseline names a metric for no audited dialect: {stale:?}"
    );
    let missing: Vec<_> = expected.difference(&baseline_metrics).collect();
    assert!(
        missing.is_empty(),
        "baseline is missing a metric for an audited dialect: {missing:?}"
    );
}

#[test]
fn gap_count_ratchet() {
    for d in AUDITED_DIALECTS {
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
    // longer has, a pair the harness never probes, or (test 9) an arm the
    // entry does not have, can never fire — and reads as coverage while
    // covering nothing.
    let probes = probe::derive_probes();
    let probed: HashSet<&str> = probes.iter().map(|p| p.name).collect();
    let probed_arms: HashSet<(&str, usize)> = probes
        .iter()
        .filter_map(|p| p.arm.map(|a| (p.name, a)))
        .collect();
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
        } else if let Some(arm) = row.arm {
            if !probed_arms.contains(&(row.name, arm)) {
                orphans.push(format!(
                    "  {} ({}) arm {}: entry has no such probed arm",
                    row.name,
                    row.dialect.slug(),
                    arm
                ));
            }
        }
    }
    assert!(
        orphans.is_empty(),
        "ORPHANED LEDGER ROWS — registered but unreachable. Delete them:\n{}",
        orphans.join("\n")
    );
}

/// One row per `(entry, dialect, position, arm, leg)`. The key includes the
/// leg because a pair can legitimately be registered on more than one:
/// `DATE_ADD` on BigQuery both infers the wrong type and returns a different
/// value. It includes the arm because two arm-scoped rows for the same
/// entry are legitimately different rows.
#[test]
fn a_pair_has_at_most_one_ledger_row() {
    let mut seen = HashSet::new();
    for row in ledger::dialect_divergences() {
        assert!(
            seen.insert((row.name, row.dialect, row.position, row.arm, row.leg)),
            "duplicate ledger row for {} on {} ({:?}, arm {:?}, {:?})",
            row.name,
            row.dialect.slug(),
            row.position,
            row.arm,
            row.leg
        );
    }
}

/// Test 6: every arm of every `Emission::Conditional` entry in the real
/// registry is covered by a derived probe. No production entry is
/// `Conditional` yet (phase 7 populates the first ones), so this is
/// green-but-vacuous by construction until then — the gate must exist
/// before the rows, or an arm could land unprobed the day one is added.
#[test]
fn every_conditional_arm_is_covered_by_a_probe() {
    let mut missing = Vec::new();
    for name in BuiltinRegistry::names() {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        let (_, unreachable) = probe::conditional_arm_probes(name, sig);
        for reason in unreachable {
            if let probe::NotProbed::UnreachableArm { index, detail } = reason {
                missing.push(format!("{name} arm {index}: {detail}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "conditional arms with no covering probe:\n{}",
        missing.join("\n")
    );
}

/// Test 10: the declared-unsupported exemption settles per the probe's own
/// facts — an arm-specific `Unsupported` verdict exempts that arm and no
/// other.
#[test]
fn the_declared_unsupported_exemption_settles_the_probes_arm() {
    use smelt_types::{
        ConditionalArm, OperandClass, SigParam, Signature, TypeConstraint, TypeExpr,
    };

    const ARMS: &[ConditionalArm] = &[
        ConditionalArm {
            arity: None,
            classes: &[(0, OperandClass::Integral)],
            verdict: SettledEmission::Native,
        },
        ConditionalArm {
            arity: None,
            classes: &[],
            verdict: SettledEmission::Unsupported {
                reason: "otherwise",
            },
        },
    ];
    let sig = Signature::new(
        "TEST_DECLARED_UNSUPPORTED",
        vec![],
        vec![SigParam::Concrete(TypeConstraint::Concrete(
            smelt_types::DataType::Integer,
        ))],
        TypeExpr::Concrete(TypeConstraint::Concrete(smelt_types::DataType::Integer)),
    )
    .with_emission(&[(
        DialectId::DuckDb,
        Position::Scalar,
        smelt_types::Emission::Conditional(ARMS),
    )]);

    let native_facts = CallFacts::new(vec![OperandClass::Integral]);
    assert!(!settles_unsupported(
        &sig,
        DialectId::DuckDb,
        Position::Scalar,
        &native_facts
    ));

    let otherwise_facts = CallFacts::new(vec![OperandClass::String]);
    assert!(settles_unsupported(
        &sig,
        DialectId::DuckDb,
        Position::Scalar,
        &otherwise_facts
    ));
}
