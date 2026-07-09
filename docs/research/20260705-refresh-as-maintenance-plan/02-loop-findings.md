# Loop findings: what the property-discovery loop learned about smelt

- **Date**: 2026-07-06
- **Status**: research
- **Author**: Andrew (with Claude)
- Part 2 of `docs/research/20260705-refresh-as-maintenance-plan/` (see `README.md` for the
  directory's structure). Part 1 is `01-framework.md` — referred to below as "the paper".
- **Sources**: `docs/research/property-discovery/ledger.md` (per-cell verdict blocks, cited by
  cell id below), `docs/research/property-discovery/unsupported.md` (negative catalogue),
  `docs/research/property-discovery/catalog.md` (cell backlog/index), and
  `docs/research/20260705-property-discovery-loop.md` (the loop's Link 0/A/B/C design).

---

## 0. How to read this document

The ledger is organized cell-by-cell (`SC-1`, `G-01`, …), in the order the loop happened to reach
them. That is the wrong axis for a spec author: the same mechanism fact gets re-discovered by four
different cells, and the interesting findings (a production fix, a dormant classifier, a design
fork) are scattered between cells that are individually unremarkable. This document re-cuts the
same evidence by *finding*. Every claim below cites the ledger cell(s) it is drawn from and, where
the ledger gives one, a `file:line` for the underlying code fact — copied from the ledger, not
re-derived.

---

## 1. What smelt's execution layer actually is today

The single most load-bearing discovery of the loop is negative: **`refresh: batched` has no
fold-delta corner in production.** Every cell that touched a batched model — `G-01`, `G-02`,
`G-03`, `G-04`, `G-07`, `SC-2`, `G-05`, `G-06`, `G-09` — independently re-confirmed the same
mechanism, first made the *headline* finding in `G-02` and `G-07`:

- `resolve_strategy` (`crates/smelt-backend/src/lib.rs`) always returns
  `IncrementalStrategy::DeleteInsert` for a batched model. `unique_key` on `BatchedConfig` is
  accepted but never consulted for the *strategy* decision — it is diagnostics-only: `let _ =
  unique_key;` at `lib.rs:195` (G-02). MERGE/`merge_into` is dead code on this path; it exists but
  backs `cumulative_aggregate` instead, a different feature entirely (G-02, G-07).
- The strategy that actually runs is `delete_and_insert_transactional`
  (`crates/smelt-backend-duckdb/src/lib.rs:618-659`): one DuckDB transaction, `DELETE FROM table
  WHERE col >= start AND col < end` followed by `INSERT INTO table {sql}`, where the DELETE range
  is *exactly* the run's write window (`crates/smelt-runtime/src/execute.rs:970-973, 1028-1042` —
  "the DELETE range must equal exactly what the INSERT writes", per G-02).
- Consequently every batched cell's "fold" is in fact a **full partition replace**: the INSERT's
  `SELECT` re-derives the aggregate/join/union fresh from the source's *current* contents for
  exactly the requested `[start, end)` window, every run, unconditionally. There is no remembered
  partial state anywhere in the maintained table for a technique to fold a delta onto.

Mapped onto the paper's 2×2 (§3 read scope × write scope):

| corner | production reality |
|---|---|
| **targeted write, delta+state read** (fold-delta) | **absent** for `refresh: batched`. No code path folds a delta onto remembered aggregate state for this mode; `unique_key` is inert for strategy selection (G-02, G-04, G-07). |
| **region-overwrite write, full-input read** (recompute-region) | **the only corner batched actually implements.** DELETE+INSERT-by-window-range, full re-derivation from current source contents, every cell (G-01 through G-09, SC-1, SC-2). |
| **targeted write, full-input read** | not exercised by any cell in this catalog — `cumulative_aggregate`'s MERGE path is the closest candidate and was explicitly out of scope for every cell here (G-02's "Note on generality", G-07). |
| **region-overwrite, delta+state read** | not observed; would require a bounded-delta read feeding a region write, which nothing in the batched path does. |

The re-delivery-safety result (`G-02`: re-running an identical window never double-counts `SUM`)
is therefore **a mechanism fact, not an algebra fact about the ledger.** It holds for `SUM` (a
non-idempotent combiner that in principle *needs* a dedup ledger under a fold-delta technique) for
the same reason it holds for `MAX`/`BOOL_OR`/`MIN`/holistic combiners (`G-03`, `G-04`, `G-07`): none
of them are ever folded onto remembered state in the first place. `G-04`'s hypothesis ("does the
`MIN` fold get stuck at the lowest value ever observed, since `MIN` is non-invertible under
mutation") could not even be posed against real smelt behaviour, because there is no fold to get
stuck — every backfill is a fresh recompute. The dormant `cumulative_aggregate` MERGE path is the
one place in the codebase where this corner *could* exist; no cell in this catalog probed it (see
§8, Coverage honesty).

---

## 2. The universal traded guarantee

Four cells with completely different SQL constructs — `SC-2` (pass-through + `SUM`, mutated
already-processed partition), `G-04` (`MIN`, non-invertible lower-then-raise mutation), `G-05`
(inner-join enrichment, mutated dimension), `G-06` (left-join, late-arriving right side), `G-09`
(`UNION ALL`, late row into each arm) — converged on **the same CONDITIONAL**, restated verbatim or
near-verbatim in each cell's "condition" field. Stated once, as the general theorem the cells
instantiate:

> **An explicit backfill of a window recovers the source's current contents for that window, for
> any construct recompute-region touches** (aggregation, join enrichment, left-join
> null-preservation, union, non-invertible combiners). **A forward-only advance never revisits an
> already-processed partition** — nothing in smelt tracks a dependency from "source row mutated" or
> "late row appended into partition p" back to "partition p needs re-derivation." The guarantee
> traded is: *recompute-region only gives you current-correctness for partitions you choose to
> re-run; staleness relative to source mutation is permanent and silent otherwise.*

Instances of this theorem in the ledger:

- **SC-2**: an in-place `UPDATE` of an already-materialized partition's source row is invisible
  until that exact `[start,end)` window is explicitly re-run (backfilled); a plain forward advance
  to the next window leaves the mutated partition stale, "as expected (not a bug)."
- **G-04**: same shape for `MIN`, chosen specifically because `MIN` is *non-invertible* under
  mutation for a hypothetical fold-delta technique (lower-then-raise the row holding the minimum) —
  the non-invertibility hazard has no purchase because there is no remembered fold state (§1) to get
  stuck; an explicit backfill recomputes `MIN` fresh and gets the right (raised) answer.
- **G-05**: mutating a joined dimension row between runs. The already-processed fact partition's
  enrichment column stays stale until that fact window is explicitly re-run; on re-run, the
  dimension table is read fully and fresh (dimension sources carry no `timeseries:` block and never
  enter `source_bounds`'s domain — see §4), so the backfill recovers the current dimension value.
- **G-06**: a late-arriving right-side row (a refund) landing into an already-processed left-join
  partition. Forward advance leaves the row's effect (`refund_amt` stays `NULL`) stale; explicit
  backfill re-reads the right side's current contents and recovers the match.
- **G-09**: a late row appended into *each* arm of a `UNION ALL` independently, into an
  already-processed partition. Same shape: forward advance stale, explicit backfill recovers both
  arms because the recompute-region technique re-executes the *entire* `SELECT` (both arms) against
  current source contents — there is no per-arm bound that could under-cover one arm while covering
  the other.

The theorem generalizes across combiner identity (additive `SUM`, idempotent-but-non-invertible
`MIN`, holistic `MEDIAN`/`COUNT DISTINCT` in `G-07`), across join shape (inner enrichment, left
join, union), and across which side mutates (fact, dimension, either union arm). It is, in effect,
the paper's traded-guarantee vocabulary (§6) made concrete: recompute-region trades *automatic
staleness detection* for *simplicity and always-correct-on-demand*. `G-08` (§6 below) shows the
theorem has a sharper edge once the construct itself has a stored, order-dependent trajectory.

---

## 3. Dormant classifiers inventory

The loop repeatedly found analysis machinery that computes exactly the facts the paper's
maintenance-plan derivation would need — and that nothing in production consumes. These are
distinct from bugs: the classifiers are *correct* in isolation, they are simply unwired.

- **`input_delta_discovery`** (`crates/smelt-logical/src/analysis/input_delta.rs:88`). Classifies a
  clocked `Mutable` source as `InputDeltaKind::WindowForward` — the same branch a clocked
  `AppendOnly`/undeclared source takes, because the match arm order is `Some(ChangeFeed) =>
  ChangeFeed`, then `_ if has_clock => WindowForward` (SC-2). A repo-wide `rg -n
  "input_delta_discovery|InputDeltaKind"` (excluding its own module and `#[cfg(test)]`) found **zero
  production call sites** — not `smelt-runtime::maintenance_driver`, not anywhere else (SC-2, FIX-2).
  `FIX-2` added a permanent tripwire test
  (`crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs`) asserting the call-site
  set stays empty outside the function's own definition file, verified to actually trip (a scratch
  call was added and observed to fail the test, then removed). Wiring this classifier's verdict to
  any consuming maintenance mode is flagged as a **behaviour-defining design decision** (new
  semantics: "read only the next window forward" as a licensed technique for mutable sources), not a
  mechanical fix — BLOCKED for human review (`FIX-2`).
- **`join_shape::fan_out` / `JoinContext` / `dimension_horizon_merge`**
  (`crates/smelt-logical/src/analysis/join_shape.rs`, `crates/smelt-runtime/src/
  dimension_horizon_merge.rs`). `JoinContext::with_unique_key` can only declare a *single* column as
  unique (`join_shape.rs:29-35`); a genuine composite-key equi-join (proven one-to-one by a 100+-case
  ground-truth proptest in `G-10`) has no way to be declared unique, so `fan_out` conservatively
  returns `OneToMany` — a false negative, not a false positive (over-conservative, never unsound;
  `G-10`). `rg -n "JoinContext|dimension_horizon_merge\("` outside tests found **zero production
  call sites** for either — same dormant class as `input_delta_discovery` (`G-10`). Extending
  `JoinContext` to accept composite keys, or wiring either function to a live consumer, is again a
  design fork BLOCKED for human review, same policy precedent as `FIX-2` (`G-10`).
- **`combiner_discriminants`** (`crates/smelt-logical/src/analysis/discriminants.rs:77-134`).
  Correctly classifies `MEDIAN`/`MODE`/`PERCENTILE_CONT`/`PERCENTILE_DISC` and exact
  `COUNT(DISTINCT …)` as holistic (`holistic_or_unknown()`, lines 80-84 and the fail-closed `_` arm
  at 130-132) — but this classification is consumed *only* by the cumulative/running-total rule
  (`crates/smelt-logical/src/rules/cumulative.rs:92`, refusing `COUNT(DISTINCT)` at lines 294-300)
  and by `join_shape`'s fan-out analysis. `rules/incremental.rs` — the rule that actually governs
  `refresh: batched` GROUP BY models, i.e. every cell in this catalog — **never imports or consults
  `discriminants`/`Discriminants` at all**; its only refusals are keyed on time-bound derivability
  (`NotDerivable`) and window/ordering shape, not combiner algebra (`G-07`). This is *why* `G-07`'s
  holistic-aggregate hazard (re-delivering a delta into partial holistic state, which has no
  well-defined semantics) never has a chance to manifest: the technique that runs
  (recompute-region) is combiner-identity-agnostic by construction, so it never asks whether `MEDIAN`
  is a monoid.

All three are exactly the analysis facts the paper's plan-derivation (§5, "maintenance is a
per-(column-group × input) plan") needs to pick a per-cell technique — combiner algebra, delta
classification, and join cardinality. They exist, are individually correct (modulo `join_shape`'s
expressiveness gap), and are structurally disconnected from the one technique smelt actually emits.

---

## 4. Analyzer findings

- **FIX-1 (production fix, landed)** — Form-B reach derivation made column-aware.
  `extract_form_b_bounds` originally scanned the *whole model SQL as one text blob* for any `…
  BETWEEN <expr> AND <expr> + INTERVAL '…'` pattern and attributed the match to a source regardless
  of which column it actually constrained; `_partition_col_upper` was accepted but unused
  (`crates/smelt-logical/src/analysis/source_bounds.rs:589`, pre-fix — first surfaced as an aside in
  `SC-1`). For `SC-1`'s model, the correlated-`EXISTS` predicate `c.conversion_date BETWEEN
  e.event_date AND e.event_date + INTERVAL '7 days'` was correctly attributed to `conversions` but
  *also* spuriously attributed to `events` (compiled `events` read widened to `< 2024-01-09` with no
  textual justification). The fix added a `lhs_column_is_partition_col` helper: a match only
  contributes to a source's bound if the identifier immediately left of the matched
  `BETWEEN`/`>=`/`<` operator is that source's own partition column (bare or table-qualified).
  Cross-column rebase (`WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL … AND
  m.event_date_local + INTERVAL …`) is preserved — only the LHS column is checked, not the RHS
  anchor. Verified red→green (`test_form_b_does_not_leak_bound_to_unrelated_source`, failing
  pre-fix with the spurious 7-day leak, passing post-fix) and no-regression gated across
  `smelt-logical` (296 passed), `smelt-planner` (38 passed), `smelt-runtime`, and `smelt-cli`'s
  property-discovery suite (15 passed) (`FIX-1`).
- **SC-1b** — the residual cross-source leak `FIX-1` cannot close: two sources whose partition
  *columns happen to share a name* (`d`), only one of which has an actual Form-B pattern. The
  column-name check has no notion of *which FROM/JOIN alias belongs to which source*, so the
  column-name match spuriously widens the *other* source's read too. Proven **widen-only, never
  unsound**, by construction: `BoundResult::merge` takes `before.max`/`after.max` when folding
  multiple matches into one source's bound, so a spurious cross-source match can only ever add
  margin, never remove it — a same-named-column collision wastes read work but cannot clamp away a
  row full-refresh would include (`SC-1b`). Not actioned (no observable-behaviour bug to red→green
  fix against); recorded as a legitimate future efficiency improvement (making `derive_bound_for_source`
  alias/source-scoped, not just column-name-scoped).
- **G-05 / G-06** — non-timeseries sources are structurally outside `source_bounds`' domain.
  `derive_model_bounds` only iterates `ctx.source_partition_cols`, populated from
  `crates/smelt-runtime/src/compile.rs::build_source_bound_map`'s walk over `dep_timeseries`; a
  source with no `timeseries:` block (G-05's `users` dimension) never enters `BoundContext` at all —
  its reach is *absent*, not a computed `Unbounded` value, so the bound-emission loop simply has
  nothing to skip past for it. This is sound for a plain inner-join enrichment (no filter is needed
  or emitted on an always-fully-read dimension) but is a structural absence, not a reasoned
  derivation (G-05). `G-06`'s `refunds` source *is* a genuine timeseries source but its join
  predicate (`e.d = r.refund_date`, an equality across two different source columns) is not a
  same-source temporal filter smelt's Form-A/B derivation recognizes, so no bound clips it either —
  same observed effect (unbounded/no-op), different mechanism (present-but-unmatched vs
  absent-from-domain).
- **P0-6** — derived bounds shown sound *and tight* for Form-B. An independent DuckDB clamp-probe
  (restrict the read to a candidate margin, then apply the model's own filter on top) against the
  fixed `SUM(s.payload)` Form-B model confirmed the analyzer's derived 1-day margin is both
  **sufficient** (every generated dataset: margin=1d clamp always equals the true output) and
  **necessary** (margin=0d diverges on a row exactly at `partition_date - 1 day`) — not merely a
  safe over-approximation.

---

## 5. Execution-layer bugs found

- **G-11 (BLOCKED — design fork, root cause confirmed)** — a hard SQL binder error on the spec's
  own documented self-referential direct-join pattern. `crates/smelt-runtime/src/
  transformer.rs::inject_time_filter` injects the outer output clamp as a **bare, unqualified**
  `{event_time_column} >= .. AND {event_time_column} < ..` whenever `is_transparent_single_source`
  is false — true for any self-referential batched model, since the self-edge counts as a second
  bounded source. `docs/specs/batched_models.md`'s documented pattern and
  `window_independence.rs`'s own unit tests (lines ~113-119) use a *direct* join (`bal.partition_date`
  / `t.partition_date`, no subquery wrap) where both the driving source and the self-reference expose
  the output column under its own bare name — DuckDB rejects the compiled SQL with `Binder Error:
  Ambiguous reference to column name "d"`. `G-08`'s own test could only proceed by wrapping the
  self-join in a subquery, a workaround the documented pattern does not itself describe. Reproduced
  red→green as a test (`g_11_self_ref_ambiguous_column`, asserting `execute_project` returns an
  `Err` whose message contains the DuckDB binder error) but the *fix* is a genuine design fork
  between two non-equivalent repair strategies (qualify the clamp to the resolved driving-fact alias,
  vs. always wrap the query in an outer subquery before clamping) — each with its own contract
  implications (alias resolution requires `smelt-runtime` to gain knowledge `smelt-logical` already
  encodes elsewhere, or an outer-wrap changes what `inject_time_filter`'s existing
  already-qualified-column calling convention — `test_with_join` — must do). BLOCKED for human
  review; see `03-design-forks.md` for the two candidate resolutions and their trade-offs.
- **G-06 aside** — giving two FROM-clause sources the *same* partition column name (`d`) makes
  smelt's derived bare `WHERE d >= … AND d < …` filter genuinely ambiguous across both sources: a
  DuckDB `Binder Error: Ambiguous reference to column name "d"` at execution time (not a silent
  wrong answer, but a real filter-emission gap — the derived predicate is not qualified by
  source/alias). Sidestepped in `G-06`'s own fixture by naming `refunds`' column `refund_date`
  instead; not filed as its own catalog cell (adjacent-append cap discipline) but flagged for
  whoever next touches `source_bounds`/filter emission for multi-timeseries-source models.
- **G-01 aside** — a general (non-schedule-dependent) correctness bug, out of this framework's
  scope but worth flagging: `add_source_info_to_type_context`
  (`crates/smelt-db/src/queries/schema.rs`, ~line 1356) derives `(schema, table)` from a source's
  `address_segments` and requires `segs.len() >= 2`, silently `continue`-ing (dropping *all* of that
  source's declared columns from the `TypeContext`) for a source file at scan-root with a
  single-segment address (e.g. `sources/events.yml` → `["events"]`). With the column's `DOUBLE` type
  unresolved, `SqlFunction::Sum` falls through to its historical `BigInt` default, and
  `wrap_with_type_casts` faithfully emits `CAST(total AS BIGINT)` — silently truncating any
  fractional aggregate. Reproduces identically on a single non-incremental run with no schedule at
  all, so it is not a fold-delta/schedule-safety bug; flagged as an **uncovered variant** of an
  already-partially-documented failure class (`crates/smelt-db/tests/proptests/aggregate_widening.rs`'s
  header covers the *empty*-`TypeContext` trigger; this is a *populated-but-arity-mismatched*
  `TypeContext` trigger). Worked around in `G-01`'s own generator (whole-number values only), not
  fixed — a human should triage against `docs/research/20260417-0.3-regression-triage.md` bug #3.

---

## 6. The trajectory/cascade finding (G-08)

`G-08` targets a self-referential running-total model (`ROWS UNBOUNDED PRECEDING`-shaped, one-day
self-edge, forced `Ordered` by `window_independence`). Sequence: build a 3-day trajectory
sequentially (day1→day2→day3), then append a late transaction into the *already-processed* day1
partition, then backfill **only** day1.

Result: day1 self-corrects (110, matching full-refresh) but **day2 and day3 remain stale** (15/16,
diverging from the true 115/116) until *they too* are explicitly re-run, in temporal order,
downstream of the mutation. Once that cascade is performed by hand, all three partitions match
full-refresh exactly.

The analyzer is not unsound here: `window_independence`'s `Ordered` verdict only claims "this
self-edge converges partition-by-partition under strictly sequential execution within one run" — it
never claims that backfilling one stale partition repairs the downstream partitions of a *separate*
run. Nothing in smelt detects that day1's stored value changed, and nothing schedules day2/day3 for
re-derivation. This is recorded as a **CONDITIONAL**: the maintained trajectory equals full-refresh
only when every backfill of partition `p` is followed by a backfill of every partition `> p`, in
strict temporal order — a real, silent staleness trap for an operator who backfills a single day of
a running-balance model expecting the trajectory to "just be correct" downstream.

This confirms, empirically, the paper's §7 claim that a stored trajectory has an **unbounded
forward footprint** (a change to one partition's value invalidates every later partition's own
stored value, not just its own) and its §12 framing that recompute-region has **no cross-partition
cascade** built in — recompute-region does exactly what it is asked to do for the partition it is
asked to rebuild, and nothing more.

---

## 7. What the loop validates about the paper's framework

| paper claim | confirming / refuting cells |
|---|---|
| §3 technique space is a 2×2 (read scope × write scope), not a dichotomy | **Confirmed but lopsided in practice**: batched production code occupies exactly one corner (region-overwrite × full-input-read = recompute-region); the fold-delta corner is architecturally absent, not merely unused (§1 above; G-01–G-04, G-07, SC-2). |
| §6 skeleton/payload column scoping is the right invariant for an oracle | **Confirmed operationally**: Link B's skeleton-column set (e.g. `{d}` for a GROUP BY, `{d, id, src}` for a discriminated union) was usable as the diff floor in every Link-C cell without needing revision; Link C's "diff all columns by default, skeleton as floor not ceiling" design (§2.2 of the loop doc) meant no cell was ever silently green on an under-identified skeleton. |
| Link A abstract predictions (faithful-fold over append-only for idempotent monoids; MIN-under-retraction REFUTED) generalize to real smelt | **P0-5** made these predictions abstractly; **G-01/G-03** (idempotent) and **G-04** (MIN non-invertibility) replayed the concrete constructs through real smelt — but the *concrete* verdict diverged from the *abstract* one precisely because production smelt never folds (§1), so the abstract REFUTED-under-fold prediction for MIN could not even be exercised; the concrete cell HOLDS for an orthogonal reason (recompute, not a correct fold). This is itself informative: Link A predicts what a fold-delta *would* do; Link C reveals smelt doesn't do that. |
| Observer-semantics prediction for MIN under mutation (G-04) | **G-04 came out HOLDS *because* the technique is recompute**, exactly as the general theorem (§2 above) predicts: there is no fold state for non-invertibility to poison. This is the theorem doing real explanatory work, not a coincidence per cell. |
| §7/§12 stored-trajectory unbounded forward footprint / no automatic cascade | **Confirmed empirically** by G-08 (§6 above) — a local backfill repairs only the backfilled partition; downstream partitions require an explicit, temporally-ordered cascade smelt does not perform or detect the need for. |
| §5 maintenance is a per-(column-group × input) *plan*, requiring combiner algebra / delta classification / join cardinality as inputs | **Confirmed the inputs exist** (`combiner_discriminants`, `input_delta_discovery`, `join_shape::fan_out`) **but are structurally disconnected** from the one technique that runs (§3 above) — the plan-derivation machinery the paper needs is partially built and entirely unwired for `refresh: batched`. |

---

## 8. Coverage honesty

What this loop has **not** probed, and which the design-fork / proof-obligation follow-on work
should not assume is covered:

- **Keyed/cumulative paths.** Every cell in this catalog exercises `refresh: batched`. The MERGE/
  `merge_into` path backing `cumulative_aggregate` — the one place a targeted-write, fold-delta-like
  corner plausibly exists in production — was named as out-of-scope in `G-02`'s "Note on
  generality" and never itself became a cell. Nothing here says whether the cumulative path's
  MERGE/upsert is sound, over-conservative, or unsound for re-delivery, mutation, or any other
  hazard.
- **Proptest depth.** Only `G-01`, `G-02`, `G-03`, and `G-07` are proptest-generated (8 cases each,
  small window/row-count ranges). `SC-1`, `SC-1b`, `SC-2`, `G-04`, `G-05`, `G-06`, `G-08`, `G-09`,
  `G-11` are each a single hand-authored deterministic schedule, chosen to target one seeded hazard
  — appropriate for the specific question asked (per the loop's own N4 anti-vacuity note) but not a
  sweep over the schedule space. Every one of these cells' own "Coverage caveat" says so explicitly
  and should not be read as a general clearance for the construct.
- **Change-feed / CDF sources.** `input_delta_discovery`'s `Some(ChangeFeed) => ChangeFeed` arm
  (ranked above the `WindowForward` fallback SC-2 exercises) was never itself exercised by any cell
  — no cell staged a source with a genuine change-feed/CDF declaration.
- **Beyond-horizon lateness through the real path.** `P0-5`'s abstract Link-A scaffold demonstrated
  a late arrival *beyond* a technique's derived horizon silently diverges from batch (an abstract
  finding); no Link-C cell replayed a beyond-horizon late arrival through real `execute_project` —
  every seeded hazard in `SC-1`, `SC-2`, `G-05`, `G-06`, `G-09` lands *within* the derived
  margin/window.
- **Multi-arm mutable unions.** `G-09` covers two append-only arms. A third arm, or an arm declared
  `mutation_profile: mutable`, or a `UNION ALL` feeding a downstream `GROUP BY`, are all named as
  not-yet-catalogued in `G-09`'s own coverage caveat.
- **Holistic combiner over a mutable-snapshot source.** `G-07` covers holistic aggregates only over
  append-only; `G-04`'s non-invertibility concern and `G-07`'s no-bounded-state concern are
  orthogonal and were never combined into one cell.
- **Composite keys ≥3 columns, and misuse of `JoinContext`.** `G-10`'s ground-truth proptest only
  generated 2-column composite keys with small cardinalities; a 3+-column composite key, and the
  case where a caller wrongly declares one column of a composite key as individually unique, were
  both named as untested in `G-10`'s coverage caveat.

This feeds `06-proof-obligations.md` — every bullet above is a gap in the empirical record, not a
claim that the untested shape is unsafe.

---

## References

- `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` — the paper this document
  is part 2 of.
- `docs/research/property-discovery/ledger.md` — full per-cell verdict blocks (cited throughout).
- `docs/research/property-discovery/unsupported.md` — the negative catalogue (REFUTED/CONDITIONAL
  index).
- `docs/research/property-discovery/catalog.md` — the cell backlog/index.
- `docs/research/20260705-property-discovery-loop.md` — the loop's design (Link 0/A/B/C).
