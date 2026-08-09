//! Pinned discriminating test for the P1 skeleton-source-closure proof
//! (`docs/specs/model_properties.md` §"Skeleton-source closure") over a real
//! fixture, not a synthetic SQL string.
//!
//! `examples/timeseries/models/daily_events_enriched.sql` joins
//! `smelt.sources.raw.users` via a bare inner `JOIN`
//! (`e.user_id = u.user_id`) — the SAME SQL text serves both tests below.
//! `examples/timeseries/models/sources/raw/users.yml` now declares both
//! `unique_key: [user_id]` and `referential_integrity: [user_id]` (T3 over
//! external sources, `docs/plans/20260715-composed-axes-conditional-
//! maintenance.md` Phase F5), but [`skeleton_source_closure`] takes those
//! facts as explicit arguments rather than reading the YAML itself — this
//! file constructs both the declared-facts case (below) AND the
//! absent-facts case ([`bare_inner_join_enrichment_stays_open`]) directly,
//! so the discriminating pair is asserted side by side regardless of what
//! the fixture's source YAML currently says.

use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::skeleton_closure::{
    skeleton_source_closure, RowPreservation, SkeletonSourceClosure,
};

fn daily_events_enriched_sql() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/timeseries/models/daily_events_enriched.sql"
    ))
    .expect("fixture must exist")
}

/// With NO declared `unique_key`/`referential_integrity` facts fed to the
/// proof, row preservation (conjunct 4) cannot be proven for a bare inner
/// join — the closure must stay `Open`.
///
/// **Pinned — load-bearing.** This is the negative half of the
/// discriminating pair with [`inner_join_enrichment_closes_with_declared_
/// referential_integrity`] below: the SAME fixture SQL, only the declared
/// facts differ. Do not remove or weaken this test when extending the
/// positive case — the proof must keep discriminating between "declared"
/// and "undeclared", not just always close now that the real fixture's YAML
/// declares the facts.
#[test]
fn bare_inner_join_enrichment_stays_open() {
    let sql = daily_events_enriched_sql();

    // No JoinContext key facts and no referential_integrity — this fixture
    // cannot pass conjunct 3 or conjunct 4, which is exactly the
    // discriminating shape this pin exists to hold open.
    let ctx = JoinContext::new();
    let verdict = skeleton_source_closure(&sql, "raw.users", None, &ctx);

    assert!(
        !verdict.is_closed(),
        "daily_events_enriched.sql's bare inner join against raw.users (no declared \
         referential_integrity) must stay Open, got: {verdict:?}"
    );
}

/// The positive half of the discriminating pair: fed the SAME facts
/// `examples/timeseries/models/sources/raw/users.yml` now actually declares
/// (`unique_key: [user_id]` licenses conjunct 3's one-to-one join
/// contribution; `referential_integrity: [user_id]` licenses conjunct 4's
/// row preservation for the bare inner join in place of a `LEFT JOIN`), the
/// SAME `daily_events_enriched.sql` text closes — this is the fixture T3
/// over external sources (Phase F5) restricts the enrichment recompute
/// under.
#[test]
fn inner_join_enrichment_closes_with_declared_referential_integrity() {
    let sql = daily_events_enriched_sql();

    let ctx = JoinContext::new().with_unique_key("u", "user_id");
    let referential_integrity = vec!["user_id".to_string()];
    let verdict = skeleton_source_closure(&sql, "raw.users", Some(&referential_integrity), &ctx);

    assert_eq!(
        verdict,
        SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity {
                source: "raw.users".to_string()
            }
        },
        "daily_events_enriched.sql's inner join against raw.users must close once fed the \
         dimension's declared unique_key + referential_integrity facts: {verdict:?}"
    );
}
