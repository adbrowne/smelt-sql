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

/// `G-05`: inner-join enrichment (fact × dim), batched per partition
/// (`unique_key: [d, user_id]`), where the fact source (`events`) is a
/// timeseries append-only source and the dim source (`users`) is a plain
/// lookup (no `timeseries:` block) declared `mutation_profile: mutable`
/// (`docs/research/20260705-property-discovery-loop.md` §4 `G-05`). A
/// dimension update broadcasts to every fact row referencing it (breaks
/// invariant A, keeps B; paper §10) — this cell asks empirically whether
/// smelt's batched recompute-region re-reads the CURRENT contents of the
/// (non-timeseries, unbounded) dimension source when a partition is
/// explicitly re-run, or whether some cached/derived bound on the dim source
/// clips it the way `SC-1`'s zero-margin fallback clipped `conversions`.
pub fn join_enrichment_mutable_dimension() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "events_enriched",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d, user_id]
---
SELECT e.d, e.user_id, e.val, u.tier
FROM smelt.sources.events e
JOIN smelt.sources.users u ON e.user_id = u.user_id
"#,
        sources: &[
            MultiSourceSpec {
                name: "events",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "val",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "users",
                columns: &[
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "tier",
                        ty: "VARCHAR",
                    },
                ],
                timeseries: None,
            },
        ],
    }
}

/// `G-06`: left-join null-preservation (fact `events` × right-side `refunds`),
/// batched per partition (`unique_key: [d, user_id]`), where BOTH sources are
/// timeseries **append-only** (`docs/research/20260705-property-discovery-loop.md`
/// §4 `G-06`). A left-joined row is first emitted with `refund_amt IS NULL`
/// because no matching `refunds` row has arrived yet; the `refunds` row then
/// arrives **late** — appended between runs into the SAME (already-processed)
/// `d` partition, never mutated in place (both sources stay append-only, so
/// this is `SC-1`'s "late row appended into an already-processed range" shape,
/// not `G-04`/`G-05`'s in-place mutation shape). This cell asks empirically
/// whether smelt's recompute-region (explicit backfill of the same window)
/// re-reads the CURRENT contents of `refunds` and recovers the non-NULL row —
/// or whether the NULL is stranded (paper §10: a left-join's unmatched-row
/// footprint is unbounded-forward until recompute-region re-derives it).
///
/// `refunds`'s own event-time column is named `refund_date`, deliberately
/// distinct from `events.d`: naming both partition columns `d` makes smelt's
/// derived bare (unqualified) `WHERE d >= ... AND d < ...` filter genuinely
/// ambiguous across the two FROM-clause sources (a DuckDB binder error, not
/// this cell's target) — a real but SEPARATE finding about the filter emitter
/// not qualifying its derived column reference, orthogonal to the late-arrival
/// hypothesis this cell tests. Distinct column names sidestep it so the cell
/// stays scoped to G-06's actual question.
pub fn left_join_late_right_side() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "events_with_refunds",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d, user_id]
---
SELECT e.d, e.user_id, e.val, r.refund_amt
FROM smelt.sources.events e
LEFT JOIN smelt.sources.refunds r ON e.user_id = r.user_id AND e.d = r.refund_date
"#,
        sources: &[
            MultiSourceSpec {
                name: "events",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "val",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "refunds",
                columns: &[
                    SourceColumn {
                        name: "refund_date",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "refund_amt",
                        ty: "DOUBLE",
                    },
                ],
                timeseries: Some(("refund_date", "refund_date")),
            },
        ],
    }
}

/// `G-07`: holistic `MEDIAN`/exact `COUNT(DISTINCT ...)` group-by, batched per
/// partition (`unique_key: [d]`), over an append-only source
/// (`docs/research/20260705-property-discovery-loop.md` §4 `G-07`). Same
/// `events(d, id, val)` source shape as `G-01`/`G-03`, but the combiners are
/// **holistic / non-monoid** (Link 0 table §2.0: no bounded combiner state —
/// `combiner_discriminants` classifies both `MEDIAN` and exact
/// `COUNT(DISTINCT ...)` into its fail-closed `holistic_or_unknown()` bucket,
/// `crates/smelt-logical/src/analysis/discriminants.rs`). Distinct model name
/// from `additive_agg_append_only`/`idempotent_agg_append_only` so all three
/// cells' staged projects never collide.
pub fn holistic_agg_append_only() -> ModelShape {
    ModelShape {
        name: "events_daily_holistic_append_only",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, MEDIAN(val) AS med_val, COUNT(DISTINCT id) AS distinct_ids FROM smelt.sources.events GROUP BY d
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

/// `G-08`: windowed running total via a **self-referential batched model**
/// (`docs/specs/batched_models.md` §"Window independence and self-referential
/// models" — the construct smelt actually built for a running balance; a
/// single-partition `ROWS UNBOUNDED PRECEDING` window frame has no
/// cross-partition trajectory to maintain, so it is not this construct).
/// `running_balance.d` self-joins its own PRIOR partition
/// (`bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d`, the exact backward-
/// bounded form `window_independence`'s own unit test proves `Ordered`) and
/// adds the current day's transaction total. `transactions` is the driving
/// append-only source; `running_balance` is the self-edge.
pub fn running_balance_self_ref() -> ModelShape {
    ModelShape {
        name: "running_balance",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d]
---
SELECT d, balance FROM (
  SELECT
    t.d AS d,
    COALESCE(bal.balance, 0) + SUM(t.amt) AS balance
  FROM smelt.sources.transactions t
  LEFT JOIN smelt.running_balance bal
    ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
  GROUP BY t.d, bal.balance
) inner_balance
"#,
        source: "transactions",
        source_columns: &[
            SourceColumn {
                name: "d",
                ty: "DATE",
            },
            SourceColumn {
                name: "amt",
                ty: "DOUBLE",
            },
        ],
    }
}

/// `G-09`: `UNION ALL` of two independent append-only timeseries sources
/// (`events_a`, `events_b`), batched per partition
/// (`unique_key: [d, id, src]` — `src` disambiguates the two arms so a
/// same-`(d, id)` row from each arm never collides under the union's shared
/// grain) (`docs/research/20260705-property-discovery-loop.md` §4 `G-09`).
/// Link 0: a multiset union is combiner-identity-agnostic by construction —
/// no combiner even runs, so this is really testing whether smelt's
/// **recompute-region** (batched DELETE+INSERT of the whole `[start,end)`
/// window; `G-03`/`G-07`'s established technique for `refresh: batched`) reads
/// BOTH union arms' CURRENT contents on an explicit backfill of an
/// already-processed partition — a per-arm reach that under-covers one arm
/// (e.g. `source_bounds` deriving a bound from only the first `FROM` clause
/// it text-scans, `SC-1`'s known failure shape in miniature) would silently
/// strand a late row appended into arm B while arm A recomputes correctly.
/// Each arm gets its own late-append between runs so the hazard schedule
/// exercises both arms independently, not just one.
pub fn union_all_two_append_only() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "events_union_all",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [d, id, src]
---
SELECT d, id, val, 'a' AS src FROM smelt.sources.events_a
UNION ALL
SELECT d, id, val, 'b' AS src FROM smelt.sources.events_b
"#,
        sources: &[
            MultiSourceSpec {
                name: "events_a",
                columns: &[
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
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "events_b",
                columns: &[
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
                timeseries: Some(("d", "d")),
            },
        ],
    }
}

/// `SC-1b`: `FIX-1`'s `lhs_column_is_partition_col` scopes a Form-B match to
/// the *name* of the source's own partition column, but `derive_bound_for_source`
/// is still called once per source with only that name — it has no notion of
/// *which table alias in the FROM/JOIN clause belongs to which source*. If two
/// sources declare partition columns with the SAME name (`d`), a Form-B
/// pattern that is textually scoped to one source's alias (`r.d BETWEEN ...`)
/// still satisfies the LHS check for the *other* source too, because the
/// check only compares the bare column name, not the alias's source identity.
/// `logins` has no temporal-lookback pattern of its own; `resets` is the only
/// source the correlated `EXISTS` predicate actually constrains. This cell
/// asks whether that spurious cross-source match ever *narrows* `logins`'s
/// derived bound (unsound — clamps away rows) or only ever *widens* it
/// (over-conservative but safe) via `BoundResult::merge`'s max-merge.
pub fn column_name_collision_across_sources() -> MultiSourceModelShape {
    MultiSourceModelShape {
        name: "logins_with_reset_flag",
        sql: r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: batched
batched:
  unique_key: [user_id, d]
---
SELECT
  l.d,
  l.user_id,
  EXISTS(
    SELECT 1 FROM smelt.sources.resets r
    WHERE r.user_id = l.user_id
      AND r.d BETWEEN l.d AND l.d + INTERVAL '3 days'
  ) AS reset_flag
FROM smelt.sources.logins l
"#,
        sources: &[
            MultiSourceSpec {
                name: "logins",
                columns: &[
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
            MultiSourceSpec {
                name: "resets",
                columns: &[
                    SourceColumn {
                        name: "user_id",
                        ty: "BIGINT",
                    },
                    SourceColumn {
                        name: "d",
                        ty: "DATE",
                    },
                ],
                timeseries: Some(("d", "d")),
            },
        ],
    }
}

// ── Cells below are stubs the loop fills in as it reaches them. Each returns a
//    ModelShape; keep them here so the tested scope stays in one file. ──
//
// G-10  join fan-out on a COMPOSITE unique key · append-only.
