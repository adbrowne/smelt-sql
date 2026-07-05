//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! **The single readable catalogue of every model shape the property-discovery
//! loop tests.** One function per catalog construct, returning the model's
//! frontmatter + SQL as a string. This is the answer to "where is the generator
//! for models?" — the *data* is proptest-generated (Link-C run schedule +
//! `smelt-db`'s `prop_helpers::generators`), but the *model SQL* per construct
//! lives here, in one file, so the scope of what is tested is legible in one
//! place. Each `G-*` / `SC-*` catalog cell adds its construct here; the cell's
//! test only references `model_shapes::<construct>()`.
//!
//! Convention: models carry NO `WHERE start/end` clause — smelt derives the
//! incremental filter (design §2.3). Sources are referenced as
//! `smelt.sources.<name>`; the harness stages the matching `sources/<name>.yml`
//! and seeds the DuckDB table `main.sources_<name>`.

/// Column declaration for a staged source (`name`, DuckDB type).
pub struct SourceColumn {
    pub name: &'static str,
    pub ty: &'static str,
}

/// A model shape: the model file SQL + the source(s) it needs staged.
pub struct ModelShape {
    /// Model name (becomes `models/<name>.sql` and output table `main.<name>`).
    pub name: &'static str,
    /// Full model file contents (frontmatter + SQL), no `WHERE start/end`.
    pub sql: &'static str,
    /// Source name (becomes `sources/<src>.yml` + seed table `main.sources_<src>`).
    pub source: &'static str,
    /// Source columns.
    pub source_columns: &'static [SourceColumn],
}

/// `P0-1` / control: a `refresh: batched` pass-through over an append-only
/// source. No `WHERE` — the smoke test asserts the framework derives one.
pub fn batched_passthrough() -> ModelShape {
    ModelShape {
        name: "events_batched",
        // Raw string: YAML nesting (2-space indents under `timeseries:` /
        // `batched:`) must be preserved. A `\`-newline continuation would strip
        // the leading whitespace and flatten the block.
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [id]
---
SELECT d, id, val FROM smelt.sources.events
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// A declared upstream source for a [`MultiSourceModelShape`]: name, columns,
/// and (for timeseries sources) its `event_time_column`/`partition_column`
/// pair. `None` stages the source as a plain lookup (no `timeseries:` block) —
/// absent from `source_bounds`'s `BoundContext` entirely.
pub struct MultiSourceSpec {
    pub name: &'static str,
    pub columns: &'static [SourceColumn],
    /// `Some((event_time_column, partition_column))` for a timeseries source.
    pub timeseries: Option<(&'static str, &'static str)>,
}

/// Like [`ModelShape`], but for cells whose model correlates more than one
/// staged source (join enrichment, correlated `EXISTS`, ...). `ModelShape`'s
/// single `source`/`source_columns` fields can't express a second source, so
/// multi-source cells add a shape here instead of forcing a single-source
/// struct to grow optional fields every other cell ignores.
pub struct MultiSourceModelShape {
    pub name: &'static str,
    pub sql: &'static str,
    pub sources: &'static [MultiSourceSpec],
}

/// `SC-1`: correlated `EXISTS` 7-day attribution over two append-only
/// timeseries sources (the design §2 worked example). `events` is the
/// model's own driving source; `conversions` is read only inside the
/// correlated subquery's `WHERE` — never joined or unioned at the top level.
/// `source_bounds::derive_bound_for_source` is a per-source, text-scanning
/// walk over the *whole* model SQL (Form A/B), not scoped to which FROM
/// clause a column reference actually belongs to (design §2.2's skeleton-
/// column-set concern in miniature) — this cell asks empirically whether
/// that walk still derives a correct forward margin for `conversions`, or
/// falls through to the zero-margin "no temporal dependency" default
/// (`docs/research/20260705-property-discovery-loop.md` §4 `SC-1`).
pub fn correlated_exists_attribution() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "event_conversions",
        sql: r#"---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: batched
batched:
  unique_key: [user_id, event_date]
---
SELECT
  e.event_date,
  e.user_id,
  EXISTS(
    SELECT 1 FROM smelt.sources.conversions c
    WHERE c.user_id = e.user_id
      AND c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL '7 days'
  ) AS converted
FROM smelt.sources.events e
"#,
        sources: &[
            MultiSourceSpec {
                name: "events",
                columns: &[
                    SourceColumn {
                        name: "event_date",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                ],
                timeseries: Some(("event_date", "event_date")),
            },
            MultiSourceSpec {
                name: "conversions",
                columns: &[
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "conversion_date",
                        ty: "DATE",
                    },
                ],
                timeseries: Some(("conversion_date", "conversion_date")),
            },
        ],
    }
}

/// `SC-2`: additive `SUM` group-by, batched per partition (`unique_key: [d]`),
/// over a clocked source declared `mutation_profile: mutable`
/// (`docs/research/20260705-property-discovery-loop.md` §4 `SC-2`). Reuses the
/// `events(d, id, val)` shape every other cell's harness already stages, so
/// the run-schedule driver's `arb_mutable_schedule`/`InPlaceUpdate` (keyed on
/// `id`) applies unmodified. `input_delta.rs:88-93` classifies a clocked
/// source as `WindowForward` **regardless of its declared `MutationProfile`**
/// (the `Some(MutationProfile::ChangeFeed) => ChangeFeed` arm is the only
/// profile-conditioned branch; `Mutable` falls through to the `has_clock`
/// guard identically to `AppendOnly`/`None`) — this cell asks empirically
/// whether smelt's actual emitted batched maintenance ever revisits an
/// already-processed partition's source rows after they are mutated in place
/// between runs, with no hand-injected `WHERE` deciding the answer for it.
pub fn additive_agg_mutable_source() -> ModelShape {
    ModelShape {
        name: "events_daily_total",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, SUM(val) AS total FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-01`: additive `SUM` group-by, batched per partition (`unique_key: [d]`),
/// over an append-only source — the control cell establishing the happy path
/// for the fold-delta technique (`docs/research/20260705-property-discovery-loop.md`
/// §4 `G-01`): disjoint append-only deltas, each partition's rows fully
/// present before its window is ever run, no between-run mutation of an
/// already-processed partition. Same `SUM(val) GROUP BY d` shape as
/// `additive_agg_mutable_source` (`SC-2`), but a distinct model name so both
/// cells' staged projects never collide and the declared source-shape (this
/// cell: `append_only`) stays legible per-cell in each test's own
/// `stage_project`.
pub fn additive_agg_append_only() -> ModelShape {
    ModelShape {
        name: "events_daily_total_append_only",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, SUM(val) AS total FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-03`: idempotent `MAX` group-by, batched per partition (`unique_key:
/// [d]`), over an append-only source
/// (`docs/research/20260705-property-discovery-loop.md` §4 `G-03`). Same
/// `events(d, id, val)` source shape as `G-01`/`G-02`, but the combiner is
/// `MAX` — an idempotent monoid (Link 0 table §2.0) rather than `SUM`'s
/// additive one. Distinct model name from `additive_agg_append_only` so both
/// cells' staged projects never collide.
pub fn idempotent_agg_append_only() -> ModelShape {
    ModelShape {
        name: "events_daily_max_append_only",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, MAX(val) AS max_val FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-04`: idempotent `MIN` group-by, batched per partition (`unique_key:
/// [d]`), over a source declared `mutation_profile: mutable`
/// (`docs/research/20260705-property-discovery-loop.md` §4 `G-04`). Same
/// `events(d, id, val)` shape as `SC-2`/`additive_agg_mutable_source`, but the
/// combiner is the idempotent-but-**non-invertible** `MIN` (Link 0 table
/// §2.0: idempotent monoids are unsound to *fold* over a mutable source,
/// because a fold can only ever lower the running minimum, never recover
/// after the row holding the minimum is mutated upward). Distinct model name
/// from `additive_agg_mutable_source` so both cells' staged projects never
/// collide.
pub fn idempotent_agg_mutable_source() -> ModelShape {
    ModelShape {
        name: "events_daily_min_mutable",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, MIN(val) AS min_val FROM smelt.sources.events GROUP BY d
"#,
        source: "events",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "id",
                ty: "INTEGER",
            },
            SourceColumn {
                name: "val",
                ty: "DOUBLE",
            },
        ],
    }
}

// ── Cells below are stubs the loop fills in as it reaches them. Each returns a
//    ModelShape; keep them here so the tested scope stays in one file. ──
//
// G-05  inner-join enrichment (fact × dim) · mutable dimension.
// G-06  left-join null-preservation · append-only + late right side.
// G-07  holistic MEDIAN / COUNT DISTINCT · append-only.
// G-08  windowed running total (ROWS UNBOUNDED PRECEDING) · append-only.
// G-09  UNION ALL of two append-only arms.
// G-10  join fan-out on a COMPOSITE unique key · append-only.
