//! Key temporal locality gate
//! (`docs/specs/incremental_models.md` §"Key temporal locality (the
//! time-partitioned output)").
//!
//! A keyed model (`grain: key`) may time-partition its output with a
//! `timeseries:` block; admission requires **key temporal locality** — a
//! guarantee that every stored row a run's deltas can touch lies within a
//! computable slice of the output's time axis. Three routes can establish
//! it (key-embedded, key-determined, recurrence-bounded); a model that
//! satisfies none of them is refused.
//!
//! [`establish_locality`] is the **single entry point** for this decision
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
//! A1's review checklist: "no second place decides keyed+timeseries
//! admissibility"). Structural preconditions are checked first (window-forward
//! run shape, a provably NOT NULL partition column, matching granularity);
//! then routes are tried in order. This phase (A2) implements **route 1**
//! (key-embedded — `partition_column` is itself a `unique_key` column) only;
//! routes 2 (key-determined) and 3 (recurrence-bounded) land in later phases
//! behind this same seam — later phases widen the body of
//! `establish_locality`, not its signature or its callers.
//!
//! Pure module: no I/O, no Salsa. `smelt-db`'s `maintenance_plan` query
//! (`crates/smelt-db/src/queries/maintenance.rs`) and `smelt-runtime`'s
//! keyed execution loop (`crates/smelt-runtime/src/cumulative.rs`) are the
//! two callers — both pure-function call sites, so calling
//! `establish_locality` from each is not "two places deciding admissibility"
//! (the review checklist's concern): it is one deterministic pure function
//! whose result is identical wherever it is invoked with the same facts.

use crate::analysis::source_bounds::{
    derive_model_bounds, from_clause_alias_sources, resolve_single_anchor, AnchorAmbiguity,
    BoundContext, BoundResult, Seconds,
};
use crate::maintenance::SourceFacts;
use smelt_core::config::Granularity;

/// Inputs the locality gate consults.
///
/// `sql` is the expanded model SQL (function bodies inlined), reused for
/// margin derivation via [`crate::analysis::source_bounds::derive_model_bounds`]
/// — the walk-composed derivation already used elsewhere for scan-bound
/// margins (`CLAUDE.md` §"Property composition walk (`smelt-logical`)"):
/// this module never re-scans the SQL text itself.
#[derive(Debug, Clone)]
pub struct LocalityInputs<'a> {
    /// The model name, folded into the refusal message.
    pub model_name: String,
    /// The model's own derived `unique_key` (the keyed classifier's GROUP BY
    /// columns, `rules::cumulative::group_by_unique_key`).
    pub unique_key: Vec<String>,
    /// The model's own `timeseries.partition_column`.
    pub partition_column: String,
    /// The model's own `timeseries.granularity`.
    pub granularity: Granularity,
    /// Whether `partition_column` is provably NOT NULL from a key's first
    /// stored row (`incremental_models.md` §"Key temporal locality" —
    /// structural precondition 2). Supplied by the caller today rather than
    /// derived by a walk-based prover; a caller admitting route 1 derives it
    /// conservatively as "`partition_column` is itself a `unique_key`
    /// column" (a key component is never meaningfully NULL). Routes 2/3
    /// (later phases) will need a real non-key nullability proof here.
    pub partition_column_not_null: bool,
    /// The driving source's name (its `smelt.<path>` — used for margin
    /// derivation and refusal messages).
    pub driving_source_name: String,
    /// Whether the driving source carries a `timeseries:` clock at all —
    /// the window-forward structural precondition. `false` means the run
    /// shape is snapshot-reconcile, which establishes no locality.
    pub driving_source_has_clock: bool,
    /// The driving source's own declared granularity, when known. `None`
    /// when the caller cannot determine it (fails the granularity-equality
    /// precondition closed, since an unproven match is never admitted).
    pub driving_source_granularity: Option<Granularity>,
    /// The driving source's own partition column, when known — needed to
    /// derive read margins via [`derive_model_bounds`].
    pub driving_source_partition_column: Option<String>,
    /// Expanded model SQL, for margin derivation (route 1).
    pub sql: &'a str,
}

/// The established locality slice a `merge_into` target scan may be pruned
/// to (`incremental_models.md` §"Key temporal locality"): the derived read
/// margins around a run step's own partition value. Concrete lower/upper
/// bounds for one step are computed by the caller (`smelt-runtime`, which
/// already owns date arithmetic in `transformer.rs`) — this module stays
/// date-arithmetic-free, matching `analysis::source_bounds`'s own
/// `Seconds`-only vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalitySlice {
    /// The output's partition column the slice predicate ranges over.
    pub partition_column: String,
    /// How far back of a step's own partition value the slice must widen
    /// (lateness/skew margin, route 1).
    pub margin_before: Seconds,
    /// How far forward of a step's own partition value the slice must
    /// widen.
    pub margin_after: Seconds,
}

/// Why key temporal locality could not be established for a model's
/// `timeseries:` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalityRefusal {
    /// None of the three routes (key-embedded, key-determined,
    /// recurrence-bounded) applies, or a structural precondition failed.
    /// `nearest_missing_fact` names the single fact closest to being
    /// satisfied, to focus the fix — the spec's diagnostic contract: "The
    /// message names the three routes and the nearest missing fact"
    /// (`incremental_models.md` §Diagnostics, `KeyedForbidsTimeseries`).
    NoRouteEstablished { nearest_missing_fact: String },
}

impl LocalityRefusal {
    /// Render the refusal as the `KeyedForbidsTimeseries` diagnostic
    /// message: names all three routes and the nearest missing fact.
    pub fn message(&self, model_name: &str) -> String {
        match self {
            LocalityRefusal::NoRouteEstablished {
                nearest_missing_fact,
            } => format!(
                "KeyedForbidsTimeseries: model '{model_name}' declares a `timeseries:` block \
                 but key temporal locality could not be established — no route applies. \
                 The three routes: \
                 (1) key-embedded — `partition_column` is itself a `unique_key` column; \
                 (2) key-determined — the partition projection is a per-key constant, proven by \
                 once-write provenance (a key-derived expression or a declared functional \
                 dependency over a provably non-null column); \
                 (3) recurrence-bounded — a key-recurrence bound `r` holds (statically derived, \
                 or declared on the driving source via `key_recurrence`), so every pair of rows \
                 sharing a key lies within `r` of each other on the event-time axis. \
                 Nearest missing fact: {nearest_missing_fact}."
            ),
        }
    }
}

impl std::fmt::Display for LocalityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message("<model>"))
    }
}

fn refuse(nearest_missing_fact: impl Into<String>) -> LocalityRefusal {
    LocalityRefusal::NoRouteEstablished {
        nearest_missing_fact: nearest_missing_fact.into(),
    }
}

/// Resolve the model's **driving source** — the single alias-scoped
/// FROM/JOIN input that is both a referenced source and clocked
/// (`partition_col.is_some()`) — the same anchor-resolution primitive
/// (`from_clause_alias_sources` + `resolve_single_anchor`) that
/// `rules::cumulative::classify_cumulative` uses to resolve its own
/// `DrivingSource` at runtime.
///
/// This is the **single shared resolution** both of this gate's callers must
/// use: `smelt-db`'s static plan-derivation query
/// (`crates/smelt-db/src/queries/maintenance.rs`) and `smelt-runtime`'s keyed
/// execution loop (`crates/smelt-runtime/src/cumulative.rs`, via
/// `classify_cumulative`). Before this helper existed, `smelt-db` resolved
/// the driving source over *every* referenced source (ignoring FROM/JOIN
/// structure) while the runtime resolved it only over the outermost
/// FROM/JOIN's alias-scoped sources — a multi-source model could disagree
/// between `smelt explain`'s admission and what `smelt build` actually
/// executes. Factoring the resolution here (rather than duplicating
/// `resolve_single_anchor`'s candidate test in `smelt-db`) keeps it one
/// algorithm, not two independently-evolving ones.
///
/// `sql` is the model's own outermost `SELECT`. `sources` is every
/// already-resolved [`SourceFacts`] the model references (unordered — this
/// function is what recovers FROM/JOIN structural order from `sql`).
/// `SourceFacts::name` is the *bare* source name (the `smelt.<path>` address
/// with its leading `sources.` breadcrumb stripped — the same convention
/// `crate::maintenance::grouping`'s own FROM-alias resolution uses to match
/// against `SourceFacts.name`), so this function strips `sources.` off each
/// alias-scoped FROM/JOIN candidate's resolved path the same way before
/// matching, rather than assuming `classify_cumulative`'s different
/// `SourceTimeseriesMap` convention (keyed by the full `smelt.<path>`).
///
/// Returns `Ok(None)` when no alias-scoped candidate is clocked at all
/// (snapshot-reconcile — not an error here; the caller's own structural
/// precondition check reports that). Returns `Err` when more than one
/// alias-scoped candidate is clocked (ambiguous driving source — fails
/// closed, matching `classify_cumulative`'s own `KeyedMultipleDrivingSources`
/// posture).
pub fn resolve_driving_source<'a>(
    sql: &str,
    sources: &'a [SourceFacts],
) -> Result<Option<&'a SourceFacts>, AnchorAmbiguity> {
    let alias_sources: Vec<(String, String)> =
        smelt_parser::File::cast(smelt_parser::parse(sql).syntax())
            .and_then(|file| file.select_stmt())
            .and_then(|select| select.from_clause())
            .map(|from_clause| from_clause_alias_sources(&from_clause))
            .unwrap_or_default();

    match resolve_single_anchor(&alias_sources, |source_name| {
        let bare = source_name.strip_prefix("sources.").unwrap_or(source_name);
        sources
            .iter()
            .find(|s| s.name == bare && s.partition_col.is_some())
    }) {
        Ok(source) => Ok(Some(source)),
        Err(AnchorAmbiguity::NoCandidate) => Ok(None),
        Err(other @ AnchorAmbiguity::Multiple(_)) => Err(other),
    }
}

/// Establish key temporal locality for a keyed model's `timeseries:`
/// block.
///
/// Checks the structural preconditions first (`incremental_models.md`
/// §"Key temporal locality" — "Structural preconditions, checked before the
/// routes"), in the order the spec lists them:
///
/// 1. The run shape is window-forward (the driving source carries a
///    clock) — snapshot-reconcile establishes no locality.
/// 2. `partition_column` is provably NOT NULL.
/// 3. The block's granularity equals the driving source's granularity.
///
/// Then tries **route 1** (key-embedded: `partition_column ∈ unique_key`).
/// Routes 2 and 3 are not yet implemented (`docs/plans/20260715-composed-
/// axes-conditional-maintenance.md` phases A3/A4) — every model that clears
/// the preconditions but does not satisfy route 1 refuses with a message
/// naming that.
pub fn establish_locality(inputs: &LocalityInputs) -> Result<LocalitySlice, LocalityRefusal> {
    // Structural precondition 1: window-forward run shape.
    if !inputs.driving_source_has_clock {
        return Err(refuse(format!(
            "the run shape must be window-forward — driving source '{}' carries no \
             `timeseries:` clock, so snapshot-reconcile establishes no locality",
            inputs.driving_source_name
        )));
    }

    // Structural precondition 2: partition_column provably NOT NULL.
    if !inputs.partition_column_not_null {
        return Err(refuse(format!(
            "`{}` must be provably NOT NULL from a key's first stored row",
            inputs.partition_column
        )));
    }

    // Structural precondition 3: granularity equality.
    match inputs.driving_source_granularity {
        Some(g) if g == inputs.granularity => {}
        Some(g) => {
            return Err(refuse(format!(
                "the block's granularity ({:?}) must equal driving source '{}''s granularity \
                 ({:?})",
                inputs.granularity, inputs.driving_source_name, g
            )))
        }
        None => {
            return Err(refuse(format!(
                "driving source '{}''s granularity is not known, so it cannot be checked \
                 against the block's granularity ({:?})",
                inputs.driving_source_name, inputs.granularity
            )))
        }
    }

    // Route 1: key-embedded — partition_column is itself a unique_key column.
    if inputs
        .unique_key
        .iter()
        .any(|k| k == &inputs.partition_column)
    {
        let Some(driving_partition_col) = inputs.driving_source_partition_column.as_deref() else {
            return Err(refuse(format!(
                "driving source '{}' has no declared partition column to derive read margins \
                 against",
                inputs.driving_source_name
            )));
        };
        let ctx =
            BoundContext::new().with_source(&inputs.driving_source_name, driving_partition_col);
        let bounds = derive_model_bounds(inputs.sql, &ctx);
        let (margin_before, margin_after) = match bounds.get(&inputs.driving_source_name) {
            Some(BoundResult::Bounded { before, after, .. }) => (*before, *after),
            // Fail-closed: `Unbounded`/`NotDerivable`/absent means no
            // computable slice exists, so no pruning can be proven safe
            // (`incremental_models.md` §"Windowed maintenance and the
            // horizon": "only proofs prune").
            Some(BoundResult::Unbounded) | Some(BoundResult::NotDerivable) | None => {
                return Err(refuse(format!(
                    "the read margin for driving source '{}' could not be derived from the \
                     model's SQL — key-embedded locality requires a computable slice",
                    inputs.driving_source_name
                )));
            }
        };
        return Ok(LocalitySlice {
            partition_column: inputs.partition_column.clone(),
            margin_before,
            margin_after,
        });
    }

    Err(refuse(format!(
        "`{}` is not a `unique_key` column (route 1 fails); route 2 (key-determined) and route \
         3 (recurrence-bounded) are not yet implemented \
         (docs/plans/20260715-composed-axes-conditional-maintenance.md, phases A3-A4)",
        inputs.partition_column
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::MutationProfile;

    fn clocked_source(name: &str) -> SourceFacts {
        SourceFacts {
            name: name.to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }
    }

    /// [`resolve_driving_source`] scopes to the outer `SELECT`'s FROM/JOIN
    /// alias sources, not every referenced source — a clocked source
    /// referenced only inside a CTE (never joined into the outer FROM/JOIN)
    /// must not count as a second driving-source candidate. This is the
    /// same scoping `rules::cumulative::classify_cumulative` applies via
    /// `from_clause_alias_sources`/`resolve_single_anchor`; a resolver that
    /// instead scanned the whole (unordered) source list would see two
    /// clocked sources here and wrongly report ambiguity.
    #[test]
    fn resolve_driving_source_ignores_cte_only_clocked_source() {
        let sql = "WITH other AS ( \
                       SELECT device_id, event_date FROM smelt.sources.other_stream \
                   ) \
                   SELECT device_id, event_date, COUNT(*) AS n \
                   FROM smelt.sources.events \
                   GROUP BY device_id, event_date";
        let sources = vec![clocked_source("events"), clocked_source("other_stream")];
        let resolved = resolve_driving_source(sql, &sources)
            .expect("the CTE-only source is out of alias scope — no ambiguity")
            .expect("`sources.events` must resolve as the sole alias-scoped driving source");
        assert_eq!(resolved.name, "events");
    }

    /// Two genuinely joined, both-clocked sources in the outer FROM/JOIN
    /// are ambiguous — fails closed, matching `classify_cumulative`'s own
    /// `KeyedMultipleDrivingSources` posture.
    #[test]
    fn resolve_driving_source_reports_ambiguity_for_two_joined_clocked_sources() {
        let sql = "SELECT e.device_id, e.event_date, COUNT(*) AS n \
                   FROM smelt.sources.events e \
                   JOIN smelt.sources.other_stream o ON e.device_id = o.device_id \
                   GROUP BY e.device_id, e.event_date";
        let sources = vec![clocked_source("events"), clocked_source("other_stream")];
        let err = resolve_driving_source(sql, &sources).expect_err("both are alias-scoped");
        assert!(matches!(err, AnchorAmbiguity::Multiple(_)));
    }

    /// A minimal route-1-shaped input: `event_date` is both the partition
    /// column and part of the unique key, the driving source is clocked at
    /// the same (day) granularity, and the model SQL has no lookback
    /// construct (zero margins).
    fn route1_inputs(sql: &'_ str) -> LocalityInputs<'_> {
        LocalityInputs {
            model_name: "device_daily".to_string(),
            unique_key: vec!["device_id".to_string(), "event_date".to_string()],
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            partition_column_not_null: true,
            driving_source_name: "smelt.sources.raw.events".to_string(),
            driving_source_has_clock: true,
            driving_source_granularity: Some(Granularity::Day),
            driving_source_partition_column: Some("event_date".to_string()),
            sql,
        }
    }

    const SIMPLE_SQL: &str = "SELECT device_id, event_date, COUNT(*) AS event_count \
         FROM smelt.sources.raw.events GROUP BY device_id, event_date";

    // ---- Structural preconditions ----------------------------------------

    /// Snapshot-reconcile (no clock on the driving source) establishes no
    /// locality, even though every other fact would otherwise admit route 1.
    #[test]
    fn snapshot_reconcile_driving_source_refused() {
        let mut inputs = route1_inputs(SIMPLE_SQL);
        inputs.driving_source_has_clock = false;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("device_daily");
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
        assert!(
            message.to_lowercase().contains("window-forward")
                || message.to_lowercase().contains("snapshot-reconcile"),
            "message must explain the run-shape precondition: {message}"
        );
    }

    /// A nullable partition projection is refused — the caller's supplied
    /// `partition_column_not_null: false` must gate admission even when
    /// route 1's key-membership fact alone would otherwise be satisfied.
    #[test]
    fn nullable_partition_projection_refused() {
        let mut inputs = route1_inputs(SIMPLE_SQL);
        inputs.partition_column_not_null = false;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("device_daily");
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
        assert!(
            message.contains("NOT NULL"),
            "message must name the NOT NULL precondition: {message}"
        );
    }

    /// A driver-granularity/output-granularity mismatch is refused.
    #[test]
    fn granularity_mismatch_refused() {
        let mut inputs = route1_inputs(SIMPLE_SQL);
        inputs.driving_source_granularity = Some(Granularity::Week);
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("device_daily");
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
        assert!(
            message.to_lowercase().contains("granularity"),
            "message must name the granularity precondition: {message}"
        );
    }

    // ---- Route 1 -----------------------------------------------------------

    /// Route 1 admits when `partition_column ∈ unique_key`; the slice's
    /// margins come from `derive_model_bounds` over the model's own SQL —
    /// zero here, since `SIMPLE_SQL` has no lookback construct.
    #[test]
    fn route1_admits_key_embedded_partition_column() {
        let inputs = route1_inputs(SIMPLE_SQL);
        let slice = establish_locality(&inputs).expect("route 1 must admit");
        assert_eq!(slice.partition_column, "event_date");
        assert_eq!(slice.margin_before, Seconds::ZERO);
        assert_eq!(slice.margin_after, Seconds::ZERO);
    }

    /// A genuine lookback construct in the model SQL (Form B: a WHERE
    /// filter with a literal `INTERVAL` offset against the driving source's
    /// own partition column) widens the derived margin — the slice must
    /// reflect it, not silently stay zero.
    #[test]
    fn route1_slice_widens_by_derived_margin() {
        let sql = "SELECT device_id, event_date, COUNT(*) AS event_count \
                   FROM smelt.sources.raw.events \
                   WHERE event_date >= CAST(event_date AS DATE) - INTERVAL '2 days' \
                   GROUP BY device_id, event_date";
        let inputs = route1_inputs(sql);
        let slice = establish_locality(&inputs).expect("route 1 must admit");
        assert_eq!(slice.margin_before, Seconds::days(2));
    }

    /// When `partition_column` is not a `unique_key` column, route 1 fails
    /// and (with routes 2/3 unimplemented) the model refuses.
    #[test]
    fn non_key_partition_column_refused_no_route() {
        let mut inputs = route1_inputs(SIMPLE_SQL);
        inputs.partition_column = "event_hour".to_string();
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("device_daily");
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
        assert!(
            message.contains("event_hour"),
            "message must name the missing column: {message}"
        );
    }

    /// The rendered message names all three routes and the nearest missing
    /// fact — the spec's diagnostic contract for `KeyedForbidsTimeseries`.
    #[test]
    fn refusal_message_names_all_three_routes_and_nearest_missing_fact() {
        let mut inputs = route1_inputs(SIMPLE_SQL);
        inputs.partition_column = "event_hour".to_string();
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("device_daily");

        assert!(
            message.contains("KeyedForbidsTimeseries"),
            "message must carry the diagnostic code: {message}"
        );
        assert!(
            message.to_lowercase().contains("key-embedded"),
            "message must name route 1 (key-embedded): {message}"
        );
        assert!(
            message.to_lowercase().contains("key-determined"),
            "message must name route 2 (key-determined): {message}"
        );
        assert!(
            message.to_lowercase().contains("recurrence-bounded"),
            "message must name route 3 (recurrence-bounded): {message}"
        );
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
    }
}
