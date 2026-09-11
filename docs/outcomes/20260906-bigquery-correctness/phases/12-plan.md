# Phase 12 — Observed deltas on BigQuery

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md`
**Row:** 12 — "Observed deltas on BigQuery: `ddl_bigquery` sidecar emitters plus a
`record_observed_delta_with_write` override on a BigQuery multi-statement transaction; flip the
row back on, which the phase-11 gate then forces the driver guard to be deleted for"
**Criteria served:** 8, 9, 3, 7
**Spec anchors:** `docs/specs/incremental_models.md` §"The graph layer" (Observed deltas on
model edges; "Empty and absent are distinct"); `docs/specs/state.md` §"The state-structure
inventory", §"Which dialects realise which structure", §"The degradation contract"

**Sequencing note.** This row was written before phases 13 and 14 ran, and it is being taken
*after* them. Two of its three clauses are already satisfied — read §"What is already done"
before planning any work, and do not rebuild them.

## What is already done (phases 11, 13, 14)

- **The transactional half.** The row names "a `record_observed_delta_with_write` override".
  That method is `Backend::execute_conditional_write_and_record_observed_delta`, and the trait
  documents it as a *thin delegation* to `execute_write_with_bookkeeping` — "the one seam a
  backend needs to override to get a real transaction here; this method itself is never
  overridden directly". Phase 13 overrode that seam on BigQuery
  (`crates/smelt-backend-bigquery/src/lib.rs`, built from the pure
  `sql::write_with_bookkeeping_plan`: `ensure_sqls` first and outside, then one
  `BEGIN … BEGIN TRANSACTION; … COMMIT TRANSACTION; EXCEPTION WHEN ERROR THEN ROLLBACK
  TRANSACTION; RAISE …; END` script). **Do not add a second override.** Verify the delegation
  and say so; if it needs nothing, that is the finding.
- **The guard is already gone.** The row expects "the phase-11 gate then forces the driver
  guard to be deleted". Phase 11 already retired all three T5 sites in favour of the derived
  predicate `maintenance_driver::observed_delta::records_observed_deltas(dialect)`, which reads
  `realisable_state_structures`. The three write sites (`driver.rs:699`, `column_scoped.rs:363`,
  `membership/execute.rs:63`) and the read site (`observed_delta.rs:103`) are `if
  !records_observed_deltas(…)` skip-paths, not `!= DuckDB` comparisons, so **flipping the row on
  is the switch** — there is no guard to delete and `state_guard_census` will not change.
- **The dialect-dispatch pattern.** Phase 13 established `crates/smelt-state/src/ledger.rs` —
  one `SqlDialect`-keyed, exhaustive dispatch, Spark erroring by name. Follow it exactly for the
  observed-delta builders; do not scatter `match dialect` at call sites.
- **String escaping.** Phase 13 found DuckDB's `''`-doubling is not GoogleSQL-portable
  (backslash is an escape character there) and added `escape_string_literal`. Use it.

So this phase's real content is the **emitters, the read-side decode, and the row flip** — plus
one genuine semantic question the DuckDB spelling hides (below).

## Scope: `ObservedOutputDeltas` only

The row says "sidecar emitters", which is loose — `_smelt_observed_delta` is the observed-delta
table, not `StateStructure::FingerprintSidecar`. **`FingerprintSidecar` stays off for BigQuery
and is out of scope here.** It has a second, independent source of truth agreeing it is
DuckDB-only — `BackendCapabilities::supports_fingerprint_sidecar`, gated on by every consumer
in `maintenance_driver/sidecar.rs` — and realising it would mean flipping that flag too, which
is a different piece of work with a different blast radius. Phase 11's decision-log entry
records that contradiction as its own test; leave it intact. If you disagree, say so in the
summary rather than widening the phase.

## Work

### 1. BigQuery observed-delta SQL (`crates/smelt-state/src/ddl_bigquery.rs` or its split)

Port the three builders from `ddl_duckdb.rs:579-740`:

- `generate_observed_delta_table_ddl` — `STRING NOT NULL` scalars, `ARRAY<STRING>` for
  `changed_keys`/`partitions`, `PRIMARY KEY (model_name, window_start, window_end) NOT
  ENFORCED`. Two-part backticked name via the same `qualified()` helper phase 13 used.
- `generate_observed_delta_upsert_sql` — **the load-bearing translation.** The DuckDB form is
  `INSERT … SELECT … ON CONFLICT (…) DO UPDATE SET …`, which GoogleSQL has not got. Re-express
  as `MERGE … WHEN MATCHED THEN UPDATE SET changed_keys = …, partitions = … WHEN NOT MATCHED
  THEN INSERT …`, the same shape phase 13 used for the ledger upsert. Three sub-translations
  inside it, each of which must be checked rather than assumed:
  - `ARRAY_AGG(DISTINCT x) FILTER (WHERE x IS NOT NULL)` — GoogleSQL has no `FILTER` clause
    (phase 12's earlier work refused `FILTER` at compile time as a dialect fact, see
    `docs/specs/multi_backend.md` §"Clause-level dialect refusals"). Use
    `ARRAY_AGG(DISTINCT x IGNORE NULLS)`, GoogleSQL's own form. **This matters beyond syntax:
    `ARRAY_AGG` raises an error on a NULL element in GoogleSQL rather than producing a NULL
    array**, so `IGNORE NULLS` is load-bearing, not cosmetic.
  - `COALESCE(…, []::VARCHAR[])` — the empty-array fallback that makes a fully-suppressed run
    record *present-and-empty* rather than absent. GoogleSQL spells the empty typed array
    `[]` / `ARRAY<STRING>[]`; confirm which `COALESCE` accepts here.
  - `ARRAY_AGG` over zero rows: DuckDB gives `NULL`, which the `COALESCE` folds to `[]`.
    **Verify GoogleSQL's behaviour** — it may already return an empty array, in which case say
    so and keep the `COALESCE` as belt-and-braces with a comment, or drop it deliberately.
- `generate_observed_delta_select_sql` — same shape.

### 2. The semantic trap to decide explicitly

BigQuery **does not distinguish a NULL `ARRAY` from an empty one** — a NULL array written to a
table reads back as empty, and this is documented behaviour, not a quirk. The spec's rule here
is "Empty and absent are distinct" (`incremental_models.md` §"The graph layer"), where *absent*
means **no row for the window**, not a NULL column. Establish and state in a doc comment that
the distinction this system depends on is row-presence, so BigQuery's array flattening cannot
break it — or, if you find a path where it can, fix that path. Either way it gets a test, and
the finding goes in the summary. Do not leave this to be discovered live.

### 3. Dispatch and read side

- Add the observed-delta builders to the same dialect dispatch phase 13 built
  (`crates/smelt-state/src/ledger.rs`, or a sibling module if that file is getting long —
  match the existing naming rather than inventing a third convention). Spark errors by name.
- `maintenance_driver/observed_delta.rs` calls `ddl_duckdb::generate_observed_delta_*` directly
  and decodes into `ddl_duckdb::ObservedDelta`. Route the SQL through the dispatch. The decode
  is Arrow `ListArray` → `StringArray`; BigQuery returns Arrow too, so it should hold — but
  **check the Arrow list type BigQuery's adapter actually produces** (`LargeList`, or a
  differently-named child field, would silently decode to an empty vector through those
  `downcast_ref` early-returns). That silent-empty failure mode is exactly the kind this
  outcome exists to catch; if you cannot settle it offline, say so plainly and hand it to
  phase 16 rather than claiming it works.
- `ObservedDelta` living in `ddl_duckdb` is now a misnomer. Moving it is optional; if you leave
  it, note why.

### 4. Flip the row

`realisable_state_structures(SqlDialect::BigQuery)` gains `StateStructure::ObservedOutputDeltas`
(alongside whatever phases 13/14 left there — **rebase on their state, do not restate it**).
Update the function's doc comment. `records_observed_deltas(BigQuery)` then returns `true` and
the four sites start recording and reading.

Expect fallout in tests that encode the old claim — phases 11 and 13 both hit this. Likely:
`crates/smelt-runtime/tests/observed_delta/{main.rs,degradation.rs}` (especially
`keyed_fold_suppressed_recording_degrades_on_a_non_duckdb_backend`, which phase 11 renamed for
exactly this reason and which now has to change again),
`crates/smelt-logical/tests/maintenance_availability/*`, `availability_seam`,
`crates/smelt-cli/tests/example_diagnostics*`. **Correct them; never delete them.**

## Tests (red-green, written before the fix)

1. **Unit, `smelt-state`:** each BigQuery observed-delta builder's emitted text asserted
   verbatim — no `ON CONFLICT`, no `FILTER (WHERE`, no `"`-quoted identifier, no `::VARCHAR[]`;
   `IGNORE NULLS` present; `MERGE … WHEN MATCHED … WHEN NOT MATCHED` shape.
2. **Empty-and-absent:** a test pinning that a fully-suppressed run records a
   present-and-empty row and that absence is row-absence, per §2's finding.
3. **Seam:** BigQuery's conditional-write-plus-record puts the record and the write in one
   transaction with the ensure DDL outside — asserted against the recorded statement plan, the
   way phase 13's `write_with_bookkeeping_plan` tests are written. If this is fully covered by
   phase 13's tests, assert the *delegation* instead and say so.
4. **Degradation is now realisation:** the BigQuery leg of
   `observed_delta/degradation.rs` flips from "skips the record" to "records it".
5. **Availability:** BigQuery records no `ObservedOutputDeltas` downgrade; Spark still does.
6. Extend `crates/smelt-state/tests/ledger_dialect.rs` rather than starting a parallel file.

**Do not reach a live BigQuery warehouse.** Offline proof plus `cargo check -p smelt-cli
--features bigquery`. Phase 16 owns live verification and already inherits three checks; add
the Arrow list-decode question from §3 if it cannot be settled offline.

## Spec delta (spec-first)

`docs/specs/state.md` §"Which dialects realise which structure": BigQuery's observed-delta row
moves to realised, naming the `MERGE` upsert and `IGNORE NULLS` as the realisation's mechanism,
and stating the row-presence basis of empty-vs-absent from §2 — that is a property of the
guarantee, not an implementation detail, and BigQuery's array flattening is exactly the reason
it needs saying. Spark's row is untouched (`vec![]`, permanent, reason already recorded).

## Gates

```
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-runtime --test observed_delta
cargo test -p smelt-runtime --test state_guard_census
cargo test -p smelt-runtime --test availability_seam
cargo test -p smelt-logical --test maintenance_availability
cargo test -p smelt-logical --test maintenance_dialect_blindness
cargo test -p smelt-state --test ledger_dialect
cargo check -p smelt-cli --features bigquery
bash .claude/scripts/large-file-check.sh
```

`ddl_bigquery.rs` was 967 lines after phase 13 and phase 14 may have grown or split it. Check
the baseline first; if this phase pushes it over, split by structure rather than raising the
ratchet — and note that `observed_delta.rs` itself was already split once (phase 11) for the
same reason.

## Deliverables

- The code + tests above.
- `docs/outcomes/20260906-bigquery-correctness/phases/12-summary.md`.
- A decision-log entry at the **top** of the outcome's `## Decision log`, in the voice of the
  existing entries: dense, file:line specific, explicit about what is not proven offline.
- Row 12 flipped to `done`.
- One commit on `bigquery-prod` with the session's trailers.
