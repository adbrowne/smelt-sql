use super::*;

/// Phase 28c: `source_shape` maps the plan-layer `MutationProfile::
/// ChangeFeed` onto the analysis-layer `DeltaMutationProfile::
/// ChangeFeed` 1:1, the same mapping `AppendOnly`/`MutableSnapshot`
/// already get — closing the "no `ChangeFeed` in the plan layer" gap
/// this function's own doc comment used to record.
#[test]
fn change_feed_source_shape_maps_to_change_feed_delta_profile() {
    let facts = SourceFacts {
        name: "feed".to_string(),
        mutation: MutationProfile::ChangeFeed,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: false,
    };
    let shape = source_shape(&facts);
    assert_eq!(
        shape.mutation_profile,
        Some(DeltaMutationProfile::ChangeFeed)
    );
}
