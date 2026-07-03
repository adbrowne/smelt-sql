# Master plan: Model updates — batched refresh, keyed maintenance, and the refresh surface

**Date**: 2026-07-04
**Design / research**: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md)
(consolidates and replaces `20260701-expanding-incremental-eligibility.md` and
`20260703-maintained-refresh-and-hidden-state.md`).
**Specs (authoritative oracles for every phase)**:
- [`docs/specs/models.md`](../specs/models.md) — the three axes; the refresh enum
  (`full | batched | cumulative | versioned | latest_value | materialized_view`); storage minus
  `materialized_view`.
- [`docs/specs/batched_models.md`](../specs/batched_models.md) — the `refresh: batched` mode, the
  `batched:` block, the eligibility relaxations, self-referential ordering, the monotonicity trace.
- [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — the `refresh: cumulative`
  mode and the algebraic maintenance ladder (rungs 1–4).
- [`docs/specs/versioned_models.md`](../specs/versioned_models.md),
  [`docs/specs/latest_value_models.md`](../specs/latest_value_models.md),
  [`docs/specs/materialized_view.md`](../specs/materialized_view.md) — the three not-yet-built keyed modes.
- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — `supports_native_ivm` /
  `supports_retraction` capability flags.

**Spec diff (this plan's charter)**: the 2026-07-04 spec edits that (a) renamed the window-forward
mode `incremental → batched` (surface + `batched:` block), (b) reworked `models.md` §"Refresh axis"
into a peer enum split by output shape (partitioned vs keyed), (c) removed `materialized_view` from
the storage axis and re-homed it to `refresh: materialized_view`, (d) added the Part-14 algebraic
ladder to `cumulative_aggregate.md`, (e) added the three keyed-mode specs, (f) added the two IVM
capability flags, and (g) the Part-19 follow-through on the keyed-mode specs: input consumption is
**derived from the source's shape** (window-forward over a `timeseries:` source, exactly as
`cumulative` consumes its driving source, vs snapshot-diff for a mutable snapshot source), the
`timeseries:` forbid is scoped to the model itself (output partitioning, not event-time-aware
consumption), and `latest_value`'s "definition of latest" carries the ordering-column preferred
direction (research §19.4). The implementation currently lags all of these; this plan closes the gap.

**Tracking branch**: `worktree-incremental`.

**Predecessor master (history, not edited)**:
[`docs/plans/20260702-incremental-eligibility-expansion.md`](20260702-incremental-eligibility-expansion.md)
began the batched relaxations as autonomy-loop waves against the old spec/research names. Its **W1**
(the event-time monotonicity trace primitive) is **done and tested** — this plan consumes it, does
not re-do it. Its queued waves W2–W7 are re-homed here as Group B under the `batched` spec names; do
not run both masters against the same branch.

## Execution model

This is a **master plan** in the autonomy-loop style: a backlog registry (below), not a single linear
script. Each group's phases are scaffolded into their own detailed sub-plan (via `/smelt:plan` against
the cited spec section) before execution, then driven by `/smelt:implement` (implementer + reviewer,
spec as oracle, red-green TDD) — optionally headlessly by the autonomy loop. **No phase authors a new
spec autonomously**; every spec increment a phase needs is already landed (2026-07-04) or is called out
in that phase's row and pre-authorised here.

**Ordering rule.** Group A (rename + ontology) lands first — it is the foundation every other group's
surface references. Group B (batched relaxations) and Group C (keyed rungs) are independent of each
other and may proceed in parallel after A. Group D (new keyed modes) depends on C1 (the presentation-view
mechanism) for the smelt-driven modes and on A4 for `materialized_view`.

## Progress tracking

| # | Phase | Group | Depends on | Spec anchor | Status |
|---|-------|-------|-----------|-------------|--------|
| A1 | `refresh: batched` selector + `batched:` block; retire the `incremental:` block | A | — | `batched_models.md` §Surface; `models.md` §"Refresh axis" | pending |
| A2 | Diagnostic-code + config-field rename (`*Incremental*` → `*Batched*`); downstream spec + user-doc sweep | A | A1 | `batched_models.md` §Known Divergences; `diagnostics.md` | pending |
| A3 | Remove `materialized_view` from the storage/`Materialization` axis (enum → `View\|Table\|Ephemeral`); catalog + `smelt.yml` sweep | A | — | `models.md` §"Materialization modes"; `smelt_yml.md`; `data_catalog.md` | pending |
| A4 | Capability flags: `supports_materialized_views → supports_native_ivm`, add `supports_retraction`; wire `refresh: materialized_view` hard error | A | A3 | `multi_backend.md` §"IVM capabilities"; `materialized_view.md` | pending |
| B0 | Filter-placement classifier (pushdown depth); unify the two bound derivations; retire the outer clamp on the transparent slice | B | A1 | `batched_models.md` §"Event-time monotonicity trace"; research Part 3 | pending |
| B1 | Wire the monotonicity primitive into consumers: `UNION`-branch partitionability, subquery/CTE pushdown, join driving-fact | B | B0 | research Parts 5–7; ex-W2–W4 | pending |
| B2 | Window functions: bounded-`RANGE` cross-partition frames + `LAG`/`LEAD` two-layer lookback | B | B0 | `batched_models.md` §"Batch safety"; research Part 8 | pending |
| B3 | Non-determinism: run-deterministic pinning (`NOW`/`CURRENT_*`) + payload opt-in `nondeterministic_columns` | B | A1 | `batched_models.md` §"Non-determinism"; research Part 9.1–9.2 | pending |
| B4 | `HAVING` / `DISTINCT` group-aligned relaxations; relocate the A4 partition-alignment check per-scope | B | B0 | research Parts 9.4–9.5 | pending |
| B5 | Run-window ↔ partition granularity alignment (`g_run ≥ g_part`) | B | A1 | `batched_models.md` §"Run window vs partition granularity"; research Part 10 | pending |
| B6 | Self-referential batched models: derive the ordered property from the DAG self-edge; enforce sequential backfill | B | A1 | `batched_models.md` §"Window independence and self-referential models"; research Part 11 | pending |
| B7 | Monotone-integer partition keys (non-temporal `partition_column`) | B | B0 | `batched_models.md` §Surface; research Part 18.3 | pending |
| B8 | Per-source clamp observability: run-relative `explain` window + LSP hover readout | B | — | `batched_models.md` §"Observing the per-source clamp" | pending |
| C1 | Decomposed-monoid rung: `AVG → (sum,count)` state table + presentation view | C | A1 | `cumulative_aggregate.md` §"The maintenance boundary" (rung 2) | pending |
| C2 | Decomposed-monoid rung: variance/stddev (Welford triple) + approximate-distinct sketch | C | C1 | `cumulative_aggregate.md` §"The maintenance boundary" (rung 2) | pending |
| C3 | Group rung: retraction via per-partition delta history (`SUM`/`COUNT`/`BIT_XOR`); `--auto` staleness fidelity | C | A1 | `cumulative_aggregate.md` §"Reprocessing semantics", §"The maintenance boundary" (rung 3) | pending |
| C4 | Opt-in bounded-domain multiset rung for exact holistic aggregates (`MEDIAN`/`MODE`/quantiles/exact-distinct) | C | C1 | `cumulative_aggregate.md` §"The maintenance boundary" (rung 4) | pending |
| D1 | `refresh: latest_value` (SCD Type 1): classifier + upsert-overwrite execution | D | C1 | `latest_value_models.md` | pending |
| D2 | `refresh: versioned` (SCD Type 2): classifier + version-maintenance (close-old/open-new) + validity columns | D | C1 | `versioned_models.md` | pending |
| D3 | `refresh: materialized_view`: emit native maintained object; surface engine errors; hard-error when `supports_native_ivm = false` | D | A4 | `materialized_view.md` | pending |

## Scope

### In scope
Everything in `docs/research/20260703-model-updates.md` that the 2026-07-04 spec edits made
normative: the batched rename, the refresh-axis reshape, the batched eligibility relaxations
(Parts 3, 5–11), self-referential batched models (Part 11 — **confirmed in scope**, reinforced by
research §19.6: the composition alternative — cumulative upstream + batched snapshot downstream — is
blocked by the non-replayable-input cell, so the ordered self-referential shape is the only correct
realization of "maintained trajectory" short of a new peer mode), the algebraic
maintenance ladder rungs 2–4 (Part 14), the two new smelt-driven keyed modes (`versioned`,
`latest_value`), and a **minimal** `materialized_view` mode.

### Explicitly minimal / deferred
- **`materialized_view` (D3)** is deliberately thin per the design decision recorded in
  `materialized_view.md`: smelt does **no** native-IVM eligibility analysis of its own. It emits the
  backend's maintained object and relays the engine's accept/reject verbatim; when the backend has no
  native IVM it is a hard error. Rich pre-flight prediction and a per-engine physical-strategy modifier
  are out of scope (research §17.8, §18.2).
- **Batched Open Questions not yet settled** (research §18.2): scalar subqueries over bounded sources,
  `GROUPING SETS`/`ROLLUP`/`CUBE`, `FOLLOWING`-frame forward reach, membership/grouping
  non-determinism, aggregating-branch unions. Each stays rejected (fail-closed) until its own research
  increment settles it; not a phase here.
- **Mode-migration mechanism** (research §18.3): detecting a changed `refresh:` against existing
  physical state and refusing/offering a migration. Recorded as an Open Question below; not a phase
  until the enum is fully built out.
- **The two §19.6 hybrid cells** (research Part 19): no new refresh values and no combo modes — the
  litmus rule (§19.7) resolves every surveyed combination to an existing cell, a derived behaviour,
  or DAG composition. The two residual hybrids stay out of scope: a *maintained-trajectory* peer
  (cumulative-with-history) is demand-gated, and the *observation-series* shape ("snapshot X daily",
  a non-replayable input under a partitioned output) should eventually get a **named rejection** —
  but that rejection needs the source mutation-profile declaration (§17.6), which does not exist
  yet, so it is deferred with it rather than phased here.

## Phase detail

Each phase is scaffolded into its own sub-plan before execution; the blocks below fix the goal, the
spec oracle, the key edits, the red-green test, and the acceptance gate.

### Group A — Rename & ontology landing

#### A1 — `refresh: batched` selector + `batched:` block
- **Goal.** Select the mode with `refresh: batched` (implying `table`); move the config fields
  (`unique_key`, `nondeterministic_columns`, `safety_overrides`) into an optional `batched:` block;
  make `refresh: batched` require `timeseries:` and forbid a `batched:` block without it. Retire the
  `incremental:`-block-with-`enabled` surface.
- **Key edits.** `smelt-core` metadata/config: add the `Batched` refresh variant and the `batched:`
  block struct; map the old `incremental:` block onto it (accept both during a deprecation window, or
  hard-cut — decide in the sub-plan); frontmatter validation per `models.md` §"Constraint violations".
- **Test (red-green).** A model with `refresh: batched` + `timeseries:` builds; a `batched:` block
  without `refresh: batched` errors; `refresh: batched` without `timeseries:` errors
  (`TimeseriesRequiredForBatched`, landed in A2).
- **Acceptance.** `cargo test -p smelt-core`; example workspaces migrated to the new surface build with
  no diagnostics (`cargo test -p smelt-cli --test example_diagnostics`).

#### A2 — Diagnostic-code + config-field rename; downstream sweep
- **Goal.** Rename `Incremental`-spelled user-facing identifiers to `Batched`:
  `TimeseriesRequiredForIncremental → TimeseriesRequiredForBatched`,
  `CumulativeForbidsIncremental → CumulativeForbidsBatched`, and internal config types where they
  surface. Sweep the downstream specs and user docs that still say "incremental" in prose
  (`timeseries.md`, `sources.md`, `run_state.md`, `planner_integration.md`, `types.md`,
  `diagnostics.md`, `cli.md`, `docs-site/`).
- **Spec increment (pre-authorised).** Update `diagnostics.md`'s catalogue rows to the `Batched`
  names in the same commit as the code rename (they must agree — `diagnostics.md` mirrors the enum).
- **Test.** Diagnostic snapshot tests updated; `cargo test -p smelt-db` diagnostics tests green.
- **Acceptance.** No production identifier or user doc emits the old spelling except the historical
  `docs/plans/` and `docs/research/`; `batched_models.md`/`cumulative_aggregate.md` Known-Divergence
  rename notes removed.

#### A3 — Remove `materialized_view` from the storage axis
- **Goal.** `Materialization` enum → `View | Table | Ephemeral`. A `materialization: materialized_view`
  in frontmatter/`smelt.yml` becomes an unknown-value error suggesting `refresh: materialized_view`.
- **Key edits.** `smelt-core` `Materialization`; `smelt.yml` `default_materialization` validation;
  `data_catalog` serialization enum; any printer/DDL branch that emitted a materialized view from the
  storage value.
- **Test.** `materialization: materialized_view` errors with the migration hint; catalog output no
  longer lists it.
- **Acceptance.** `cargo test`; `smelt_yml.md` / `data_catalog.md` Known-Divergence notes removed.

#### A4 — IVM capability flags + `materialized_view` hard error
- **Goal.** Rename `supports_materialized_views → supports_native_ivm`; add `supports_retraction`
  (both `false` on every current backend); make `refresh: materialized_view` a hard error naming the
  reason when `supports_native_ivm = false`.
- **Test.** On DuckDB, `refresh: materialized_view` errors: *"requires native IVM; this engine has
  none — use `refresh: cumulative`"*; the capability-conformance suite asserts the new flags.
- **Acceptance.** `cargo test -p smelt-dialect`; `multi_backend.md` Known-Divergence note removed.

### Group B — Batched eligibility relaxations

> All of Group B upholds the per-partition (incremental ≡ full) contract as its oracle; the shipped
> generative soundness oracle from W1 is the regression net. Each relaxation only *widens* what is
> admitted and must never admit an unsound push (fail-closed).

#### B0 — Filter-placement classifier (pushdown depth) + unified bound derivation
- **Goal.** Turn the eligibility check into a **downward walk** that returns the *deepest safe injection
  point* for `event_time` (source scan / below-aggregate / above-window-with-lookback), per research
  Part 3. Derive the output-clamp window and each per-source scan window from **one** per-source walk
  (fixing the §3.2 independent-derivation under-read); drop the redundant outer clamp on the transparent
  slice (no lookback).
- **Depends on.** A1; consumes the W1 primitive.
- **Test.** The confirmed §3.2 under-read harness (margin rewrite with clipped frames) goes green; a
  transparent subquery model emits a single source-level filter, no outer wrap.
- **Acceptance.** `cargo test -p smelt-logical`; no regression in existing incremental integration tests.

#### B1 — Monotonicity-primitive consumers (UNION / subquery-CTE / joins)
- **Goal.** Wire the three below-outer-SELECT consumers that block on the primitive:
  - **UNION branches** (Part 5): per-branch trace; a `StaticSeed` branch is the P3 NULL/constant
    hazard — named and rejected; all-`Traceable` branches unlock single-stream `UNION ALL`.
  - **Subquery/CTE bodies** (Part 6): one parse-based body classifier replacing the B4 gate *and* the
    E2 diagnostic, applied to derived tables **and** CTEs (closing the CTE bypass); `Traceable → push`,
    else stay at the outer clamp.
  - **Joins** (Part 7): resolve the driving fact (exactly-one-`Traceable`-input); window only the
    driving fact, full-scan every other input, so the §7.2/J3 misfilter goes to 0 by construction.
    Needs alias-scoped leaf resolution on top of the primitive (research §4.8 leaf-resolution gap).
- **Depends on.** B0. Re-homes ex-waves W2–W4.
- **Test.** The P3, Q5, and J3–J5 harnesses (already reproduced in W1) go green; each consumer's
  reject path names the offending construct.
- **Acceptance.** `cargo test -p smelt-logical -p smelt-db`; example incremental models unaffected.

#### B2 — Window functions: bounded-`RANGE` + `LAG`/`LEAD`
- **Goal.** Admit cross-partition window functions whose `OVER` carries a bounded
  `RANGE BETWEEN INTERVAL '…' PRECEDING` frame via the primitive + a derived lookback margin (the
  two-layer widened-scan/exact-clamp move); keep `ROWS`/`GROUPS`/bare `LAG`/`LEAD`/`UNBOUNDED` rejected
  or per-partition, per Part 8.
- **Depends on.** B0.
- **Test.** A sessionization model with a bounded-`RANGE` `LAG` builds cross-partition and matches full
  refresh; a bare `LAG` still refuses (`NotDerivable`).
- **Acceptance.** `cargo test -p smelt-logical`; equivalence harness green.

#### B3 — Non-determinism: run-pinning + payload opt-in
- **Goal.** Split B5 non-determinism: pin run-deterministic clocks (`NOW`/`CURRENT_*`) at compile time
  (admissible); admit row-nondeterministic (`RANDOM`/`UUID`) only when confined to a column listed in
  the `batched:` block's `nondeterministic_columns`, gated by the flow/taint check with the three hard
  exclusions (event-time/partition/unique-key/membership). Per Parts 9.1–9.2.
- **Test.** `inserted_at = NOW()` in a listed column builds; `RANDOM()` in a `WHERE`/`GROUP BY` still
  rejects; a listed excluded column is a config error.
- **Acceptance.** `cargo test -p smelt-logical`; the `nondeterministic_columns` example builds.

#### B4 — `HAVING` / `DISTINCT` group-aligned + A4 relocation
- **Goal.** Admit `HAVING` when the `GROUP BY` key ⊇ `partition_column`; admit `DISTINCT` when its key
  ⊇ `partition_column`; keep `LIMIT` rejected (never commutes). Relocate the A4 partition-in-GROUP-BY
  check to per-branch/per-body scopes and expose its verdict as the shared partition-alignment signal.
  Per Parts 9.4–9.5.
- **Depends on.** B0 (shares the alignment signal).
- **Test.** A group-aligned `HAVING` builds; a non-aligned `DISTINCT` refuses; `LIMIT` refuses.
- **Acceptance.** `cargo test -p smelt-logical`.

#### B5 — Run-window ↔ partition-granularity alignment
- **Goal.** Enforce `g_run ≥ g_part` with aligned boundaries (research Part 10). Decide hard-validation
  vs auto-coarsen the run window; derive `g_part` from the partition-column transform unit via the
  primitive.
- **Test.** A sub-partition-granularity run window is rejected (or auto-coarsened) with a clear message;
  an incomplete final partition is handled per §10.3.
- **Acceptance.** `cargo test -p smelt-cli` incremental run tests.

#### B6 — Self-referential batched ordered execution
- **Goal.** Detect a batched model's self-edge in the DAG, mark it **ordered**, and make the backfill
  chunker build its windows strictly sequentially in temporal order (no parallel/out-of-order backfill).
  A self-reference the planner cannot prove converges partition-by-partition is refused. Per Part 11 and
  `batched_models.md` §"Window independence and self-referential models".
- **Test.** A running-balance model reading `smelt.<self>` prior partitions backfills correctly and is
  never parallelised; a forward/whole-history self-reference is refused at planning time.
- **Acceptance.** `cargo test -p smelt-planner -p smelt-cli`; ordered-execution integration test.

#### B7 — Monotone-integer partition keys
- **Goal.** Generalise the time-typed batched machinery to a non-temporal monotone `partition_column`
  (sequence id / offset / watermark): integer offsets/bands in the monotonicity whitelist, `g_part` and
  lookback margins for integer keys, `Offset` generalised past `Seconds`. Per research §18.3.
- **Depends on.** B0.
- **Test.** A model partitioned by a monotone `batch_id` integer builds and backfills; the whitelist
  admits integer offset arithmetic and rejects non-monotone integer transforms.
- **Acceptance.** `cargo test -p smelt-logical`.

#### B8 — Per-source clamp observability
- **Goal.** Finish the two observability surfaces already specified: `smelt explain --json` resolves the
  run-relative scan window `[run_start − before, run_end + after)` when a run window is supplied; LSP
  hover on a `smelt.<path>` reference shows its derived clamp alongside schema.
- **Test.** `explain --json --event-time-start/-end` reports the concrete window; a hover integration
  test shows the clamp.
- **Acceptance.** `cargo test -p smelt-cli -p smelt-lsp`; `batched_models.md` Known-Divergence updated.

### Group C — Keyed-mode maintenance rungs (cumulative extensions)

#### C1 — Decomposed-monoid rung: `AVG` via `(sum,count)` + presentation view
- **Goal.** Admit `AVG` by storing `(sum, count)` in the state table under componentwise `+` and
  exposing `sum/count` through a presentation view; treat `(state table, view)` as one atomically-swapped
  unit. This is the enabling mechanism for the whole rung-2 unlock. Per `cumulative_aggregate.md`
  §"The maintenance boundary" and research §13.1/§14.1/§15.
- **Test.** An `AVG` cumulative model maintains correctly across partitions (end-state equals full
  refresh); the view is well-defined under partial merge.
- **Acceptance.** `cargo test`; cumulative equivalence harness extended to `AVG`.

#### C2 — variance/stddev + approximate distinct
- **Goal.** Add the Welford-triple state for `VAR`/`STDDEV` and an HLL/sketch register-vector state for
  approximate `COUNT(DISTINCT)`, each with its presentation map. Decide closed-table vs registry (research
  §18.2 — default closed table).
- **Depends on.** C1 (presentation-view machinery).
- **Test.** variance and approx-distinct maintain within tolerance vs full refresh.
- **Acceptance.** `cargo test`.

#### C3 — Group rung: retraction + delta history + `--auto` fidelity
- **Goal.** For the group aggregators (`SUM`/`COUNT`/`BIT_XOR`) store per-partition deltas so a changed
  partition is reprocessed by subtract-then-add; derive reversibility from the projection; upgrade
  `--auto` staleness to "exactly the changed partitions" for fully-reversible models. Per
  `cumulative_aggregate.md` §"Reprocessing semantics" and research §14.2.
- **Test.** Reprocessing a changed partition converges without a full refresh for a `SUM` model; a `MIN`
  model still refuses (non-group).
- **Acceptance.** `cargo test`; reprocessing equivalence harness.

#### C4 — Opt-in bounded-domain multiset (exact holistic aggregates)
- **Goal.** Admit exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles/exact-`COUNT(DISTINCT)`/`DISTINCT`-aggs
  by storing the per-key value→count multiset, **only** behind a bounded-domain space-budget opt-in with a
  runtime cap that falls back to full refresh; default-refuse unbounded state with a fail-loud message
  suggesting the approximate form or `refresh: full`. Per research §14.4.
- **Depends on.** C1.
- **Spec increment (pre-authorised).** Add the bounded-domain opt-in surface to `cumulative_aggregate.md`
  §Surface once its shape is chosen (research §18.2 open question — the opt-in must be a space assertion,
  not a contract-changing knob).
- **Test.** Exact `MEDIAN` with a declared bounded domain maintains correctly; an unbounded column
  refuses with the fail-loud message; exceeding the cap falls back to full refresh.
- **Acceptance.** `cargo test`.

### Group D — New keyed modes

#### D1 — `refresh: latest_value` (SCD Type 1)
- **Goal.** Classifier (natural key + attributes, no partition column on the model itself) +
  upsert-overwrite execution via `merge_into`. Two Part-19 requirements:
  - **"Latest" prefers an ordering column derived from the SQL** (research §19.4): with an
    ordering column the combiner is max-by-ordering-key — a commutative monoid — so merges are
    order-independent (out-of-order/parallel backfill is licensed). Last-processed is the fallback
    and *derives ordered execution* (strictly sequential windows), never a declaration.
  - **Input consumption is derived from the source** (`latest_value_models.md` §Semantics): a
    `timeseries:` source is consumed window-forward via the same `--event-time` driving-source
    machinery as cumulative; a mutable snapshot source is re-scanned and upserted whole. Whether the
    windowed path *shares* cumulative's executor or keeps a per-rule copy is decided in the sub-plan
    (research §19.8 open question).
- **Depends on.** C1 (keyed-mode `merge_into` + view plumbing).
- **Test.** One row per key, always the most-recent value; changing an attribute overwrites in place.
  With an ordering column, replaying an old run window does **not** clobber newer values (the §19.4
  footgun test); a windowed source reads only the covered partitions.
- **Acceptance.** `cargo test`; end-state equivalence harness for `latest_value`, including an
  out-of-order-merge case for the ordering-column form.

#### D2 — `refresh: versioned` (SCD Type 2)
- **Goal.** Classifier + version maintenance (compare incoming to stored current per key; close the prior
  version and open a new one on a tracked-attribute change) + smelt-managed validity columns
  (`valid_from`/`valid_to`/`is_current`). Input consumption is derived from the source
  (`versioned_models.md` §Semantics, research Part 19): a `timeseries:` source (update-events / CDC
  feed) is consumed window-forward with windows applied in temporal order (close/open is inherently
  ordered) and validity intervals stamped from the **source's event time**, not the run clock, so
  end-state equivalence survives replays; a mutable snapshot source is re-scanned and compared. Settle
  the Open Questions in `versioned_models.md` (validity column shape, tracked-attribute selection,
  deletions) in the sub-plan.
- **Depends on.** C1.
- **Spec increment (pre-authorised).** Promote the settled validity-column + change-tracking surface from
  `versioned_models.md` Open Questions into §Surface as it is decided.
- **Test.** A key with three successive states yields two closed intervals + one open; non-overlapping
  snapshots merge order-independently to the same history.
- **Acceptance.** `cargo test`; interval-keyed equivalence harness.

#### D3 — `refresh: materialized_view` (engine IVM, minimal)
- **Goal.** Emit the backend's native maintained object for the model's SQL; relay the engine's
  accept/reject verbatim; hard-error when `supports_native_ivm = false` (the common case today). No
  smelt-side eligibility analysis. Per `materialized_view.md` and the design decision to keep it thin.
- **Depends on.** A4.
- **Test.** On DuckDB, `refresh: materialized_view` produces the A4 hard error; the emit path is exercised
  against a mock/`supports_native_ivm = true` backend fixture (no real Databricks backend exists yet).
- **Acceptance.** `cargo test -p smelt-cli`; `materialized_view.md` Known-Divergence updated when a real
  IVM backend lands.

## Post-implementation verification (per group)

- **A**: example workspaces migrated to the `batched`/refresh surface build clean
  (`cargo test -p smelt-cli --test example_diagnostics`, `-p smelt-lsp --test example_workspaces`); no
  production identifier emits the old `incremental`/storage-`materialized_view` spelling; every
  2026-07-04 Known-Divergence rename note is removed as its phase lands.
- **B**: each relaxation has a full-refresh-equivalence test and passes the generative soundness oracle;
  the fail-closed direction is unit-tested (an unsound form stays rejected).
- **C**: each rung has an end-state-equivalence harness vs full refresh; the fail-loud path (unbounded
  state, non-group retraction) is tested.
- **D**: each new mode has an end-state-equivalence harness; `materialized_view` has the hard-error path.

## Open questions during execution

- **Deprecation window for the old `incremental:` surface (A1).** Hard-cut to `refresh: batched`, or
  accept both for one release with a warning? The sub-plan decides; the example migration in A1/A2 is the
  forcing function.
- **Hard-validate vs auto-coarsen the run window (B5).** research Part 10 leaves this open.
- **Exact-clamp migration (B0/B2).** Adopting exact output clamps wholesale changes the late-data
  re-write use case (research §3.2/§8.6); the B0 sub-plan must state what carries late-data re-writes.
- **Closed rewrite table vs registry for decomposition (C2).** Default closed table (matches cumulative's
  allowlist stance); revisit only on a concrete sketch motivator.
- **Bounded-domain opt-in surface (C4).** A per-model annotation vs a domain-size hint vs a runtime cap —
  must stay a space assertion, not a strategy knob (research §18.2).
- **Mode-migration mechanism (§18.3).** What `smelt build` does when a model's `refresh:` changes against
  existing physical state (refuse until `--full-refresh`, or offer a migration). Not a phase here; needs
  its own research increment once the enum is fully built.
- **Shared executor vs per-rule copies for windowed keyed consumption (D1/D2, research §19.8).** The
  windowed input path (driving source + per-partition step) now has three members (`cumulative`,
  `latest_value`, `versioned`). One shared executor under the umbrella, or per-rule copies per the
  narrow-composable-rules posture? The D1 sub-plan decides and D2 follows it.
- **Snapshot-diff mechanics for keyed modes (D1/D2, research §19.8).** What the `--event-time` flags
  mean for a snapshot-diff run (there is no window), and how `--auto` staleness fires for a source
  with no monotone clock. The sub-plans may ship snapshot-diff as always-full-rescan first and defer
  the staleness question.

## References

- Research: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md)
- Predecessor master (history): [`docs/plans/20260702-incremental-eligibility-expansion.md`](20260702-incremental-eligibility-expansion.md)
- W1 primitive (done): [`docs/plans/20260702-monotonicity-primitive-tested.md`](20260702-monotonicity-primitive-tested.md)
- Cumulative implementation (history): [`docs/plans/20260523-cumulative-aggregate.md`](20260523-cumulative-aggregate.md)
- Specs: `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `versioned_models.md`,
  `latest_value_models.md`, `materialized_view.md`, `multi_backend.md`
