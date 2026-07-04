//! Input-delta discovery — the proof stage of the input-consumption axis.
//!
//! See `docs/specs/model_properties.md` §"Derived proofs" → "Input-delta
//! discovery" and `docs/specs/models.md` §"Input-consumption axis". Derives
//! *which* rows of a driving source are new from the source's shape —
//! **window-forward** (the source carries a `timeseries:` clock),
//! **snapshot-diff** (a mutable, clockless source — the conservative
//! default), or **change-feed** (the source itself reports what changed).
//! This never changes what the stored relation means (`models.md`
//! §"Input-consumption axis"); it pairs with the source mutation-profile
//! world-fact (`sources.md`, read via [`SourceShape::from_source_info`]) and
//! the re-scan/probe transform, which is wired per consuming mode (L4), not
//! here.

/// The source mutation profile — the one non-derivable world-fact on the
/// input-consumption axis (`models.md` §"Input-consumption axis";
/// `model_properties.md` §"Catalogued inputs"). Declared on the source via
/// `sources.md`'s `mutation_profile:` key (`smelt_core::sources::SourceInfo`);
/// `mutation_profile: None` on [`SourceShape`] is the undeclared/unknown case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationProfile {
    /// Rows are only ever appended, never updated or deleted in place.
    AppendOnly,
    /// Rows may be updated or deleted in place — only a full re-scan sees
    /// every change.
    Mutable,
    /// The source itself exposes a change-data feed (CDC/CDF): a run can
    /// read only the rows that changed since the last run.
    ChangeFeed,
}

impl From<smelt_core::sources::MutationProfile> for MutationProfile {
    fn from(profile: smelt_core::sources::MutationProfile) -> Self {
        match profile {
            smelt_core::sources::MutationProfile::AppendOnly => MutationProfile::AppendOnly,
            smelt_core::sources::MutationProfile::Mutable => MutationProfile::Mutable,
            smelt_core::sources::MutationProfile::ChangeFeed => MutationProfile::ChangeFeed,
        }
    }
}

/// The shape facts [`input_delta_discovery`] reads: whether the driving
/// source carries a `timeseries:` clock (`timeseries.md`), and its mutation
/// profile if declared/derivable. `mutation_profile: None` is the
/// undeclared/unknown case — the fail-closed default when a source declares
/// no `mutation_profile:`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceShape {
    pub has_clock: bool,
    pub mutation_profile: Option<MutationProfile>,
}

impl SourceShape {
    /// Build the shape [`input_delta_discovery`] reads from a source's
    /// catalogued `SourceInfo` (`sources.md`): `has_clock` from `timeseries:`
    /// presence, `mutation_profile` from the declared `mutation_profile:` key
    /// (`None` when undeclared — the fail-closed default is unchanged).
    pub fn from_source_info(info: &smelt_core::sources::SourceInfo) -> Self {
        SourceShape {
            has_clock: info.timeseries.is_some(),
            mutation_profile: info.mutation_profile.map(Into::into),
        }
    }
}

/// Verdict: how a consuming mode may discover which input rows are new
/// since the last run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDeltaKind {
    /// The source carries a monotone clock; read the next window(s) forward.
    WindowForward,
    /// Re-scan the source whole and diff against stored state. The
    /// fail-closed default whenever the shape is not provably window-forward
    /// or change-feed — never an unsound optimistic delta.
    SnapshotDiff,
    /// The source itself reports what changed.
    ChangeFeed,
}

/// Prove which input-delta kind a consuming mode may use for a source of the
/// given `shape`. Proof only — the re-scan/probe *transform* this verdict
/// licenses is wired per consuming mode (L4).
///
/// Fail-closed (`model_properties.md` §Constraints): an undeclared/unknown
/// mutation profile on a clockless source never yields an optimistic
/// `WindowForward` — it falls back to `SnapshotDiff`, the whole-relation
/// re-scan that cannot silently drop rows.
pub fn input_delta_discovery(shape: SourceShape) -> InputDeltaKind {
    match shape.mutation_profile {
        Some(MutationProfile::ChangeFeed) => InputDeltaKind::ChangeFeed,
        _ if shape.has_clock => InputDeltaKind::WindowForward,
        _ => InputDeltaKind::SnapshotDiff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocked_source_is_window_forward() {
        let shape = SourceShape {
            has_clock: true,
            mutation_profile: None,
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::WindowForward);
    }

    #[test]
    fn mutable_snapshot_source_is_snapshot_diff() {
        let shape = SourceShape {
            has_clock: false,
            mutation_profile: Some(MutationProfile::Mutable),
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::SnapshotDiff);
    }

    #[test]
    fn change_feed_source_is_change_feed() {
        let shape = SourceShape {
            has_clock: false,
            mutation_profile: Some(MutationProfile::ChangeFeed),
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::ChangeFeed);
    }

    #[test]
    fn change_feed_takes_precedence_over_clock() {
        let shape = SourceShape {
            has_clock: true,
            mutation_profile: Some(MutationProfile::ChangeFeed),
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::ChangeFeed);
    }

    #[test]
    fn unknown_mutation_profile_without_clock_fails_closed_to_snapshot_diff() {
        // No clock, no declared mutation profile: the undecidable case must
        // never yield an optimistic WindowForward that could silently drop
        // rows the profile turns out not to support.
        let shape = SourceShape {
            has_clock: false,
            mutation_profile: None,
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::SnapshotDiff);
    }

    #[test]
    fn append_only_without_clock_still_falls_back_to_snapshot_diff() {
        // AppendOnly alone (no clock) is not yet a wired discovery path —
        // window-forward requires the clock itself, so this is also
        // fail-closed to a full re-scan rather than an optimistic delta.
        let shape = SourceShape {
            has_clock: false,
            mutation_profile: Some(MutationProfile::AppendOnly),
        };
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::SnapshotDiff);
    }

    // -----------------------------------------------------------------
    // DC5: SourceShape::from_source_info reads the declared profile
    // (docs/plans/20260704-model-updates-l3-declarations.md, Phase DC5)
    // -----------------------------------------------------------------

    fn make_source_info(
        has_timeseries: bool,
        mutation_profile: Option<smelt_core::sources::MutationProfile>,
    ) -> smelt_core::sources::SourceInfo {
        smelt_core::sources::SourceInfo {
            path: std::path::PathBuf::from("/tmp/fake.yml"),
            address_segments: vec!["fake".to_string()],
            columns: vec![],
            description: None,
            name_override: None,
            tags: vec![],
            timeseries: has_timeseries.then(|| smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            mutation_profile,
            source_lateness: None,
        }
    }

    #[test]
    fn declared_change_feed_source_is_admitted_for_change_feed_instead_of_snapshot_diff() {
        // Undeclared (the pre-DC5 default every caller passed) would fail-closed
        // to SnapshotDiff. A source that *declares* change_feed widens the
        // verdict to ChangeFeed instead of the conservative whole-relation re-scan.
        let info = make_source_info(
            false,
            Some(smelt_core::sources::MutationProfile::ChangeFeed),
        );
        let shape = SourceShape::from_source_info(&info);
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::ChangeFeed);
    }

    #[test]
    fn undeclared_profile_from_source_info_still_fails_closed_to_snapshot_diff() {
        let info = make_source_info(false, None);
        let shape = SourceShape::from_source_info(&info);
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::SnapshotDiff);
    }

    #[test]
    fn timeseries_presence_on_source_info_sets_has_clock() {
        let info = make_source_info(true, None);
        let shape = SourceShape::from_source_info(&info);
        assert!(shape.has_clock);
        assert_eq!(input_delta_discovery(shape), InputDeltaKind::WindowForward);
    }
}
