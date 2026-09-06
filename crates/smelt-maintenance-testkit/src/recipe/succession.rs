//! The keyed-succession (SCD2) family's own typed recipe
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/07a-plan.md`),
//! split out of `recipe.rs` proper once the succession additions crossed
//! this crate's large-file baseline (plan task 8). `SourceRecipe`'s own two
//! new fields (`partition_column`, `delete_flag_column`) stay in the parent
//! module — a struct's fields cannot be declared across files — but every
//! succession-specific constructor, type, and unit test lives here.

use super::{KeyShape, SourcePosture, SourceRecipe};

impl SourceRecipe {
    /// The append-only, arrival-partitioned, delete-flagged succession
    /// driving source (`docs/outcomes/20260906-scd2-keyed-succession/
    /// phases/07a-plan.md`): `customer_id INTEGER` (key), `changed_at
    /// TIMESTAMP` (clock — the succession-classifier's `event_time_column`),
    /// `arrival_date DATE` (the declared `timeseries.partition_column`,
    /// deliberately distinct from `changed_at` so a late-arriving event's
    /// window membership is driven by arrival, not by the event-time value
    /// the succession patch orders on — the shape `crates/smelt-runtime/
    /// tests/fixtures/succession/` already pins), `tier VARCHAR` (payload),
    /// `is_deleted BOOLEAN NOT NULL` (the optional `QUALIFY NOT is_deleted`
    /// delete flag rule 6 requires be provably non-null).
    pub fn succession_events() -> Self {
        Self {
            name: "customer_changes".to_string(),
            clock_column: "changed_at".to_string(),
            key_column: "customer_id".to_string(),
            payload_column: "tier".to_string(),
            key_shape: KeyShape::Single,
            posture: SourcePosture::AppendOnly,
            key_recurrence: None,
            partition_column: Some("arrival_date".to_string()),
            delete_flag_column: Some("is_deleted".to_string()),
        }
    }

    /// [`Self::succession_events`]'s event-time-partitioned variant
    /// (`incremental_shapes.md` §"An event-time-partitioned source"):
    /// `partition_column: None` — a run's window scans `changed_at` itself
    /// rather than a separately declared arrival column, so there is no
    /// arrival column to insert into
    /// ([`crate::gate_succession::insert_row_succession_for`] derives its
    /// column list from this `Option`).
    pub fn succession_events_event_time_partitioned() -> Self {
        Self {
            partition_column: None,
            ..Self::succession_events()
        }
    }
}

/// A fully-typed keyed-succession (SCD2) model recipe (`docs/outcomes/
/// 20260906-scd2-keyed-succession/phases/07a-plan.md`): one arrival-partitioned,
/// delete-flagged [`SourceRecipe`] ([`SourceRecipe::succession_events`]), the
/// row-local `(alias, source expr)` projection, the `LEAD`/`LAG`-derived
/// column aliases, an optional pre-window lateness clamp, and whether the
/// model applies a `QUALIFY NOT <delete_flag>` filter. Named constructors
/// only (no proptest strategy yet — phase 7b adds the generated pool over
/// this shape).
#[derive(Debug, Clone)]
pub struct SuccessionRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    /// Every row-local column projected verbatim (key, clock, payload),
    /// as `(alias, source expr)` — mirrors [`smelt_logical::analysis::
    /// succession::SuccessionVerdict::Recognized`]'s own `row_local` field
    /// shape, so a renderer built from this recipe and the classifier's own
    /// verdict agree by construction.
    pub projection: Vec<(String, String)>,
    /// `LEAD(<clock>) OVER (PARTITION BY <key...> ORDER BY <clock>) AS
    /// <alias>` column aliases.
    pub lead_cols: Vec<String>,
    /// The `LAG` counterpart of `lead_cols`.
    pub lag_cols: Vec<String>,
    /// An optional pre-window lateness clamp: a row-local `WHERE` predicate
    /// text (e.g. `"changed_at >= DATE '2026-01-01'"`).
    pub clamp: Option<String>,
    /// Whether the model applies `QUALIFY NOT <source.delete_flag_column>`.
    pub delete_filter: bool,
}

impl SuccessionRecipe {
    /// The minimal `LEAD`-only shape (`crates/smelt-runtime/tests/fixtures/
    /// succession/models/customer_history.sql`'s own recipe): `customer_id,
    /// changed_at, tier, LEAD(changed_at) OVER (PARTITION BY customer_id
    /// ORDER BY changed_at) AS valid_to`.
    pub fn new_lead() -> Self {
        let source = SourceRecipe::succession_events();
        Self {
            model_name: "customer_history".to_string(),
            projection: vec![
                (source.key_column.clone(), source.key_column.clone()),
                (source.clock_column.clone(), source.clock_column.clone()),
                (source.payload_column.clone(), source.payload_column.clone()),
            ],
            lead_cols: vec!["valid_to".to_string()],
            lag_cols: vec![],
            clamp: None,
            delete_filter: false,
            source,
        }
    }

    /// [`Self::new_lead`]'s `LAG` counterpart — proves the renderer's `LAG`
    /// arm is exercised by the family quartet, not just `LEAD`
    /// (phase 7a test list item 7).
    pub fn new_lag() -> Self {
        let source = SourceRecipe::succession_events();
        Self {
            model_name: "customer_history_lag".to_string(),
            projection: vec![
                (source.key_column.clone(), source.key_column.clone()),
                (source.clock_column.clone(), source.clock_column.clone()),
                (source.payload_column.clone(), source.payload_column.clone()),
            ],
            lead_cols: vec![],
            lag_cols: vec!["valid_from".to_string()],
            clamp: None,
            delete_filter: false,
            source,
        }
    }

    /// Turn on `QUALIFY NOT <source.delete_flag_column>` — a builder
    /// combinator (phase 7b) rather than a new named constructor per leg;
    /// callers must still give the recipe a unique `model_name` so staged
    /// projects don't collide.
    pub fn with_delete_filter(mut self) -> Self {
        self.delete_filter = true;
        self
    }

    /// Attach a pre-window lateness clamp predicate (a row-local `WHERE`
    /// text, e.g. `"changed_at >= DATE '2026-01-01'"`).
    pub fn with_clamp(mut self, predicate: impl Into<String>) -> Self {
        self.clamp = Some(predicate.into());
        self
    }

    /// Swap the driving [`SourceRecipe`] (e.g. for
    /// [`SourceRecipe::succession_events_event_time_partitioned`]).
    pub fn with_source(mut self, source: SourceRecipe) -> Self {
        self.source = source;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `arrival_partitioned_source_has_distinct_partition_and_event_time_columns`
    /// (phase 7a test 1): [`SourceRecipe::succession_events`] declares a
    /// `partition_column` distinct from its `clock_column`, plus a `NOT
    /// NULL` delete-flag column.
    #[test]
    fn arrival_partitioned_source_has_distinct_partition_and_event_time_columns() {
        let source = SourceRecipe::succession_events();
        assert_ne!(
            source.partition_column.as_deref(),
            Some(source.clock_column.as_str()),
            "the succession source's declared partition_column must differ from its clock_column"
        );
        assert!(
            source.partition_column.is_some(),
            "the succession source must declare a partition_column"
        );
        assert_eq!(
            source.delete_flag_column.as_deref(),
            Some("is_deleted"),
            "the succession source must declare a NOT NULL is_deleted delete-flag column"
        );
    }
}
