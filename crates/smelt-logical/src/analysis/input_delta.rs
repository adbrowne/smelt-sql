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
//! world-fact (`sources.md`) and the re-scan/probe transform, which is wired
//! per consuming mode (L4), not here.

/// The source mutation profile — the one non-derivable world-fact on the
/// input-consumption axis (`models.md` §"Input-consumption axis";
/// `model_properties.md` §"Catalogued inputs"). `sources.md` has no
/// first-class declaration for this yet (`models.md` §Known Divergences
/// "Source mutation profile is inferred, not yet a first-class source
/// declaration"); callers pass `None` on [`SourceShape`] until that
/// declaration surface exists.
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

/// The shape facts [`input_delta_discovery`] reads: whether the driving
/// source carries a `timeseries:` clock (`timeseries.md`), and its mutation
/// profile if declared/derivable. `mutation_profile: None` is the
/// undeclared/unknown case — the fail-closed default until `sources.md`
/// grows a first-class declaration for it.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceShape {
    pub has_clock: bool,
    pub mutation_profile: Option<MutationProfile>,
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
}
