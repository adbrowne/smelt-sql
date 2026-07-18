//! Pinned discriminating test for the P1 skeleton-source-closure proof
//! (`docs/specs/model_properties.md` §"Skeleton-source closure") over a real
//! fixture, not a synthetic SQL string.

use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::skeleton_closure::skeleton_source_closure;

/// `examples/timeseries/models/daily_events_enriched.sql` joins
/// `raw.users` via a bare inner `JOIN` (`e.user_id = u.user_id`), and
/// `examples/timeseries/models/sources/raw/users.yml` currently declares
/// `mutation_profile: mutable_snapshot` but no `unique_key`/
/// `referential_integrity`. Absent that declaration, row preservation
/// (conjunct 4) cannot be proven for an inner join — the closure must stay
/// `Open`.
///
/// **Pinned — load-bearing.** This test survives until a later phase (F5)
/// flips only the `referential_integrity`-declared variant of this fixture
/// (or an equivalent new fixture) to `Closed`; do not silently update
/// `raw.users.yml` to add `referential_integrity` without adding a
/// corresponding new positive test alongside it.
#[test]
fn bare_inner_join_enrichment_stays_open() {
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/timeseries/models/daily_events_enriched.sql"
    ))
    .expect("fixture must exist");

    // `raw.users` declares no unique_key today, so no JoinContext key facts
    // exist to license the join's cardinality either — this fixture cannot
    // pass conjunct 3 or conjunct 4, which is exactly the discriminating
    // shape this pin exists to hold open.
    let ctx = JoinContext::new();
    let verdict = skeleton_source_closure(&sql, "raw.users", None, &ctx);

    assert!(
        !verdict.is_closed(),
        "daily_events_enriched.sql's bare inner join against raw.users (no declared \
         referential_integrity) must stay Open, got: {verdict:?}"
    );
}
