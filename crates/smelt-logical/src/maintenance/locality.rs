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
//! then routes are tried in order. **Route 1** (key-embedded —
//! `partition_column` is itself a `unique_key` column) and **route 2**
//! (key-determined — the partition projection is a per-key constant under
//! the once-write provenance proof, admitted only via a declared
//! `functional_dependencies:` entry; an extremal-fold combiner such as
//! `MIN`/`MAX` is a distinct, value-mutating family and is refused even
//! when declared) are implemented; route 3 (recurrence-bounded) lands in a
//! later phase behind this same seam.
//!
//! Pure module: no I/O, no Salsa. `smelt-db`'s `maintenance_plan` query
//! (`crates/smelt-db/src/queries/maintenance.rs`) and `smelt-runtime`'s
//! keyed execution loop (`crates/smelt-runtime/src/cumulative.rs`) are the
//! two callers — both pure-function call sites, so calling
//! `establish_locality` from each is not "two places deciding admissibility"
//! (the review checklist's concern): it is one deterministic pure function
//! whose result is identical wherever it is invoked with the same facts.

use crate::analysis::discriminants::Monotone;
use crate::analysis::functional_dependency::{
    functional_dependency_verdict_over_vector, FunctionalDependencyVerdict,
};
use crate::analysis::join_shape::JoinContext;
use crate::analysis::source_bounds::{
    derive_model_bounds, from_clause_alias_sources, resolve_single_anchor, AnchorAmbiguity,
    BoundContext, BoundResult, Seconds,
};
use crate::analysis::walk::model_property_vector;
use crate::maintenance::SourceFacts;
use smelt_core::config::{FunctionalDependency, Granularity};

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
    /// Expanded model SQL, for margin derivation (route 1) and for the
    /// walk-composed property vector route 2 consumes
    /// (`analysis::walk::model_property_vector`) — never re-scanned as raw
    /// text by this module.
    pub sql: &'a str,
    /// Model-scoped `functional_dependencies:` declarations
    /// (`model_properties.md` §"Model-scoped declarations"), consulted by
    /// route 2 against the model's walk-derived [`crate::analysis::walk::
    /// PropertyVector`] via
    /// [`functional_dependency_verdict_over_vector`]. Route 2 admits only
    /// via a declaration naming `partition_column` (the modeller's own
    /// once-write assertion) — there is currently no walk-provable
    /// "no declaration needed" shape distinct from route 1's literal
    /// key-column case, and a grain-subset-key FD proof alone cannot
    /// distinguish a genuinely once-write column from an extremal-fold
    /// (`MIN`/`MAX`) one, so it is never used to auto-admit route 2. Empty
    /// when the model declares none — route 2 then always refuses.
    pub declared_functional_dependencies: &'a [FunctionalDependency],
}

/// The established locality slice a `merge_into` target scan may be pruned
/// to (`incremental_models.md` §"Key temporal locality"). The two routes
/// this module implements license two structurally different slices:
///
/// - Route 1 (key-embedded) derives a **window**: margins around a run
///   step's own partition value, resolved to concrete bounds by the caller
///   (`smelt-runtime`, which already owns date arithmetic in
///   `transformer.rs`) — this module stays date-arithmetic-free, matching
///   `analysis::source_bounds`'s own `Seconds`-only vocabulary.
/// - Route 2 (key-determined) derives **delta values**: the slice is
///   exactly the partition-column values the step's own delta carries — no
///   widening, and independent of the run step's own date, since a
///   once-write-provenance-proven column is a per-key constant regardless
///   of which row (or which run) reveals it (`incremental_models.md`
///   §"Key temporal locality (the time-partitioned output)": "the slice is
///   the delta's own partition values — exact regardless of key age").
///   The caller resolves this to a `target.<col> IN (SELECT DISTINCT <col>
///   FROM (<delta>))` predicate against the step's own already-compiled
///   delta relation — no new value discovery, no extra query round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalitySlice {
    /// Route 1: a step-relative window.
    Window {
        /// The output's partition column the slice predicate ranges over.
        partition_column: String,
        /// How far back of a step's own partition value the slice must
        /// widen (lateness/skew margin).
        margin_before: Seconds,
        /// How far forward of a step's own partition value the slice must
        /// widen.
        margin_after: Seconds,
    },
    /// Route 2: the exact partition values the step's own delta carries.
    DeltaValues {
        /// The output's partition column the slice predicate ranges over.
        partition_column: String,
    },
}

impl LocalitySlice {
    /// The output's partition column the slice predicate ranges over,
    /// regardless of route.
    pub fn partition_column(&self) -> &str {
        match self {
            LocalitySlice::Window {
                partition_column, ..
            }
            | LocalitySlice::DeltaValues { partition_column } => partition_column,
        }
    }
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

/// The single shared "provably NOT NULL" derivation both of
/// [`establish_locality`]'s callers must use (`smelt-db`'s static
/// plan-derivation query and `smelt-runtime`'s keyed execution loop) — moved
/// to `analysis::not_null` (`crate::analysis::not_null`) because it is a
/// **leaf classifier** per `docs/specs/architecture.md` §"Property
/// composition walk rule", and that is the directory the walk-coverage gate
/// (`cargo test -p smelt-logical --test walk_coverage`) scans. Re-exported
/// here so callers that already import it from this module (`smelt-db`,
/// `smelt-runtime`) are unaffected.
pub use crate::analysis::not_null::partition_column_provably_not_null;

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
/// Then tries **route 1** (key-embedded: `partition_column ∈ unique_key`),
/// then **route 2** (key-determined: `partition_column` is a per-key
/// constant, admitted only via a declared `functional_dependencies:` entry
/// naming it — and refused outright, even when declared, when the walk's
/// own discriminants prove the column is an extremal-fold (`MIN`/`MAX`)
/// combiner, a distinct value-mutating family, not once-write). Route 3 is
/// not yet implemented (`docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` phase A4) — every model that clears the preconditions
/// but does not satisfy routes 1 or 2 refuses with a message naming that.
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
        return Ok(LocalitySlice::Window {
            partition_column: inputs.partition_column.clone(),
            margin_before,
            margin_after,
        });
    }

    // Route 2: key-determined — the partition projection is a per-key
    // constant under the once-write provenance proof
    // (`incremental_models.md` §"Key temporal locality", route 2): a
    // declared functional dependency over a column present non-null on
    // every input row (the "present non-null" half of that obligation is
    // the structural precondition already checked above —
    // `partition_column_not_null` gates every route, not just this one).
    // This route consumes the existing walk-composed property vector
    // (`analysis::walk::model_property_vector`) and the existing
    // functional-dependency verdict
    // (`analysis::functional_dependency::functional_dependency_verdict_over_vector`)
    // — no raw-text FD reimplementation.
    //
    // Once-write provenance is a **family** distinction
    // (`incremental_models.md` §"The algebraic maintenance ladder"), not
    // merely "deterministic given the current computation": an
    // extremal-fold combiner (`MIN`/`MAX`) is a genuinely value-mutating
    // combiner across merges — a later, out-of-order redelivery with an
    // earlier true event value changes the folded value on re-merge
    // (`analysis::discriminants::Monotone::Value`). The plain grain-
    // subset-key FD proof (`functional_dependency_verdict_over_vector`'s
    // "no declaration needed" branch) only establishes that the column is
    // a deterministic function of the key *within one fixed computation*;
    // it says nothing about invariance across merges over time, so it is
    // never consulted here — no currently-implemented walk proof
    // distinguishes a genuinely once-write shape (e.g. a literal `GROUP
    // BY` key passthrough) from an extremal-fold aggregate over the same
    // grain. Route 2 therefore consults only an explicit
    // `functional_dependencies:` declaration (the modeller's own
    // once-write assertion) — and even then refuses outright when the
    // walk's own discriminants prove the column is an extremal-fold
    // combiner: a declaration can widen only a genuinely undecidable
    // origin, never override a proven value-mutating combiner.
    if let Some(vector) = model_property_vector(inputs.sql, &JoinContext::new()) {
        if let Some(discriminant) = vector
            .discriminants
            .iter()
            .find(|d| d.output.eq_ignore_ascii_case(&inputs.partition_column))
        {
            if discriminant.discriminants.monotone == Monotone::Value {
                return Err(refuse(format!(
                    "route 2 (key-determined) refused: `{}` is derived from an extremal-fold \
                     combiner (MIN/MAX) — a later, out-of-order row can still change its value on \
                     re-merge, so it belongs to the extremal-fold family, distinct from once-write \
                     provenance (`incremental_models.md` §\"Key temporal locality\")",
                    inputs.partition_column
                )));
            }
        }

        let verdict = match inputs
            .declared_functional_dependencies
            .iter()
            .find(|fd| fd.determines.eq_ignore_ascii_case(&inputs.partition_column))
        {
            Some(fd) => {
                functional_dependency_verdict_over_vector(&fd.key, &fd.determines, &vector, true)
            }
            None => FunctionalDependencyVerdict::NotProven,
        };
        match verdict {
            FunctionalDependencyVerdict::Constant => {
                return Ok(LocalitySlice::DeltaValues {
                    partition_column: inputs.partition_column.clone(),
                });
            }
            FunctionalDependencyVerdict::Refused(reason) => {
                return Err(refuse(format!(
                    "route 2 (key-determined) refused: `{}` is not a provable per-key constant \
                     — {reason}",
                    inputs.partition_column
                )));
            }
            FunctionalDependencyVerdict::NotProven => {
                // Fall through to the final refusal below — route 3 is not
                // yet implemented either.
            }
        }
    }

    Err(refuse(format!(
        "`{}` is not a `unique_key` column (route 1 fails) and is not proven a per-key constant \
         by a declared `functional_dependencies:` entry (route 2 fails); route 3 \
         (recurrence-bounded) is not yet implemented \
         (docs/plans/20260715-composed-axes-conditional-maintenance.md, phase A4)",
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
            declared_functional_dependencies: &[],
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
        assert_eq!(slice.partition_column(), "event_date");
        match slice {
            LocalitySlice::Window {
                margin_before,
                margin_after,
                ..
            } => {
                assert_eq!(margin_before, Seconds::ZERO);
                assert_eq!(margin_after, Seconds::ZERO);
            }
            other => panic!("route 1 must derive a Window slice, got {other:?}"),
        }
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
        match slice {
            LocalitySlice::Window { margin_before, .. } => {
                assert_eq!(margin_before, Seconds::days(2));
            }
            other => panic!("route 1 must derive a Window slice, got {other:?}"),
        }
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

    // ---- Route 2 -------------------------------------------------------

    /// A minimal route-2-shaped input: `unique_key` is `event_id` alone
    /// (not `event_date`), and `first_seen_date` is a non-key aggregate
    /// projection (`MIN(event_date)`), so route 1 fails
    /// (`partition_column ∉ unique_key`).
    fn route2_inputs(sql: &'_ str) -> LocalityInputs<'_> {
        LocalityInputs {
            model_name: "events_deduped".to_string(),
            unique_key: vec!["event_id".to_string()],
            partition_column: "first_seen_date".to_string(),
            granularity: Granularity::Day,
            partition_column_not_null: true,
            driving_source_name: "smelt.sources.raw.events".to_string(),
            driving_source_has_clock: true,
            driving_source_granularity: Some(Granularity::Day),
            driving_source_partition_column: Some("event_date".to_string()),
            declared_functional_dependencies: &[],
            sql,
        }
    }

    const ROUTE2_SQL: &str = "SELECT event_id, MIN(event_date) AS first_seen_date, \
         COUNT(*) AS event_count FROM smelt.sources.raw.events GROUP BY event_id";

    /// Regression test (reviewer-found correctness bug): a `MIN`/`MAX`-
    /// derived partition column must NOT be admitted by route 2, declared
    /// FD or not. `MIN`/`MAX` is a genuinely value-mutating combiner across
    /// merges — a late/out-of-order redelivery with an earlier true event
    /// date changes the folded value on re-merge — so it belongs to the
    /// extremal-fold family, which is distinct from once-write provenance
    /// (`incremental_models.md` §"Key temporal locality", "Row movement":
    /// only route 3, not route 2, may see a partition value move). Proving
    /// that the model's own `GROUP BY` key is a subset of its `unique_key`
    /// only establishes that `first_seen_date` is a deterministic function
    /// of the key *within one fixed computation* — it says nothing about
    /// invariance across merges over time, so it must never be treated as
    /// a once-write proof.
    #[test]
    fn route2_refuses_min_derived_partition_column_not_once_write() {
        let inputs = route2_inputs(ROUTE2_SQL);
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("events_deduped");
        assert!(
            message.to_lowercase().contains("extremal-fold"),
            "message must name the extremal-fold family as the refusal reason: {message}"
        );
    }

    /// The same refusal holds even when a `functional_dependencies:` entry
    /// explicitly declares `event_id -> first_seen_date`: a declaration is
    /// the modeller's once-write assertion, but it can never override the
    /// walk's own proof that the column is an extremal-fold combiner.
    #[test]
    fn route2_refuses_min_derived_partition_column_even_when_declared() {
        let mut inputs = route2_inputs(ROUTE2_SQL);
        let fd = smelt_core::config::FunctionalDependency {
            key: vec!["event_id".to_string()],
            determines: "first_seen_date".to_string(),
        };
        let declared = [fd];
        inputs.declared_functional_dependencies = &declared;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("events_deduped");
        assert!(
            message.to_lowercase().contains("extremal-fold"),
            "message must name the extremal-fold family as the refusal reason: {message}"
        );
    }

    /// A declared `functional_dependencies:` entry widens an otherwise
    /// undecidable (no traceable join, no `GROUP BY` subset-key) origin to
    /// `Constant` — the same widening `functional_dependency_verdict_over_
    /// vector` already proves at the unit level, exercised here through the
    /// locality gate's own consumption of it.
    #[test]
    fn route2_admits_declared_functional_dependency() {
        let sql = "SELECT customer_id, region FROM smelt.sources.raw.crm";
        let mut inputs = route2_inputs(sql);
        inputs.unique_key = vec!["customer_id".to_string()];
        inputs.partition_column = "region".to_string();
        let fd = smelt_core::config::FunctionalDependency {
            key: vec!["customer_id".to_string()],
            determines: "region".to_string(),
        };
        let declared = [fd];
        inputs.declared_functional_dependencies = &declared;
        let slice = establish_locality(&inputs).expect("declared FD must admit route 2");
        assert!(matches!(slice, LocalitySlice::DeltaValues { .. }));
    }

    /// A declared FD over a column the model does not even project is not
    /// widened (`functional_dependency_verdict_over_vector`'s consultation
    /// rule) — route 2 refuses fail-closed rather than trust a declaration
    /// disconnected from the model's own columns.
    #[test]
    fn route2_refuses_declared_fd_over_column_not_projected() {
        let sql = "SELECT customer_id, region FROM smelt.sources.raw.crm";
        let mut inputs = route2_inputs(sql);
        inputs.unique_key = vec!["customer_id".to_string()];
        inputs.partition_column = "region".to_string();
        let fd = smelt_core::config::FunctionalDependency {
            key: vec!["not_a_real_column".to_string()],
            determines: "region".to_string(),
        };
        let declared = [fd];
        inputs.declared_functional_dependencies = &declared;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("events_deduped");
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
    }

    /// A `determines` column sourced from a join F6 proves fans out
    /// (`OneToMany`) is a structural disproof no declaration can override —
    /// route 2 refuses fail-closed and names the reason.
    #[test]
    fn route2_refuses_when_column_crosses_undiscriminated_union() {
        let sql = "SELECT customer_id, region FROM smelt.sources.raw.crm_a \
                   UNION ALL \
                   SELECT customer_id, region FROM smelt.sources.raw.crm_b";
        let mut inputs = route2_inputs(sql);
        inputs.unique_key = vec!["customer_id".to_string()];
        inputs.partition_column = "region".to_string();
        let fd = smelt_core::config::FunctionalDependency {
            key: vec!["customer_id".to_string()],
            determines: "region".to_string(),
        };
        let declared = [fd];
        inputs.declared_functional_dependencies = &declared;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("events_deduped");
        assert!(
            message.to_lowercase().contains("union"),
            "message must name the structural disproof reason: {message}"
        );
    }

    /// The structural NOT NULL precondition (checked before any route) gates
    /// route 2 exactly as it gates route 1 — a non-key partition column the
    /// caller cannot prove NOT NULL is refused before the functional-
    /// dependency proof is ever consulted.
    #[test]
    fn route2_refuses_when_partition_column_not_provably_not_null() {
        let mut inputs = route2_inputs(ROUTE2_SQL);
        inputs.partition_column_not_null = false;
        let err = establish_locality(&inputs).unwrap_err();
        let message = err.message("events_deduped");
        assert!(
            message.contains("NOT NULL"),
            "message must name the NOT NULL precondition: {message}"
        );
    }
}
