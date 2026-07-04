# Plan: Model updates — Group B (Batched eligibility relaxations)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — Group B (phases B0–B8)
**Specs (oracles)**:
- [`docs/specs/batched_models.md`](../specs/batched_models.md) — PRIMARY. §"Per-source bound derivation", §"Batch safety classification", §"Source-filter pushdown", §"Event-time monotonicity trace", §"Observing the per-source clamp", §"Window independence and self-referential models", §"Non-determinism and the equivalence contract", §"Per-partition equivalence", §"Run window vs partition granularity", §Surface, §Known Divergences.
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis", §"Constraint violations".
- [`docs/specs/timeseries.md`](../specs/timeseries.md) — §"Granularity values", §"Granularity arithmetic", §"Constraints & Invariants" (partition-column projection & monotonicity).
- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — whitelist as the *intersection* of per-backend monotonicity.
**Research (the "why")**: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Parts 3, 5–11, and §18.3 (monotone-integer keys).
**Spec diff**: none new — every surface Group B lands was made normative by the 2026-07-04 spec reshape (committed `f056ac35`). Each phase *removes* a `batched_models.md` §Known-Divergence note as its behaviour ships; no phase authors a spec.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Dependency on Group A (do not start Group B until A1/A2 have landed).** Group B builds on the
`refresh: batched` selector + `batched:` block (A1) and the `Batched`-spelled diagnostic codes +
config type (A2). This plan is written against the **post-A** names: the config type is
`BatchedConfig` (A2 renames `IncrementalConfig → BatchedConfig`), diagnostics read
`TimeseriesRequiredForBatched` / `CumulativeForbidsBatched` / `BatchedNotBatchSafe`, and the mode is
selected by `refresh: batched` (not an `incremental:` block). A2 also *optionally* renames the rule
module file `crates/smelt-logical/src/rules/incremental.rs` and the `IncrementalStrategy` enum —
these may or may not have moved. **First action of every phase: `rg` for the identifier you are about
to touch and confirm its current spelling**; the file:line anchors below were taken against the
pre-A tree (`IncrementalConfig`, `rules/incremental.rs`) and A1/A2 will have shifted the exact
strings even where the structure is unchanged.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (added when
Group B is scaffolded into the registry — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant oracle for every phase is the **per-partition (batched ≡ full refresh) contract**
   (`batched_models.md` §"Per-partition equivalence", Constraint 6). Every relaxation only *widens*
   what is admitted and must **fail closed** (`batched_models.md` Constraint 10 §"No silent downgrade",
   Constraint 12 one-directional soundness).
2. Confirm you are on branch `worktree-incremental` and that Group A (A1, A2) is `done` in the master.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry
   Status to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase
needs a *full-refresh-equivalence* test AND a *fail-closed* (stays-rejected) unit test) → reviewer
subagent (material findings only) → iterate → set the row `done` → commit + push with the phase's
`Commit.` line.

**The shipped generative soundness oracle is the regression net.** The monotonicity primitive
(`crates/smelt-logical/src/analysis/monotonicity.rs`, `trace_event_time`) and its smelt-sql soundness
oracle already exist and are exhaustively tested (predecessor W1 work,
[`docs/plans/20260702-monotonicity-primitive-tested.md`](20260702-monotonicity-primitive-tested.md)).
**Do not re-implement the primitive** — B0/B1 *wire it into consumers*. Keep the oracle green; extend
it only where a phase adds a genuinely new admitted form.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/`
edits describe the feature as if it always existed; as each phase lands, **remove** the matching
`batched_models.md` §Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (the per-phase "Open decision"
callouts flag the known ones), or a pre-flight red unrelated to this phase's target: set the row
`blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit
`<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape made a large batched-eligibility surface normative: a single downward
filter-placement walk (research Part 3), the event-time monotonicity trace and its three
below-the-outer-SELECT consumers (Parts 5–7), bounded-`RANGE` / `LAG`/`LEAD` windows (Part 8), scoped
non-determinism (Part 9), group-aligned `HAVING`/`DISTINCT` (Parts 9.4–9.5), run-window↔partition
granularity alignment (Part 10), self-referential ordered execution (Part 11), monotone-integer
partition keys (§18.3), and the run-relative clamp observability surfaces. The implementation lags:
the batch-safety classifier (`crates/smelt-logical/src/rules/incremental.rs`) still gates on
whole-string text scans (`find_inadmissible_over`, the `allow_*` override checks), bound derivation
(`derive_model_source_bounds`) reads only the outer stripped SQL, the monotonicity primitive is
**built and tested but unwired** (no consumer calls it), and the injected time filter sits only at the
outer clamp. Group B closes this gap one relaxation at a time, each pinned to the per-partition
equivalence contract and each fail-closed.

Group B **re-homes** the queued waves W2–W7 of the predecessor master
[`docs/plans/20260702-incremental-eligibility-expansion.md`](20260702-incremental-eligibility-expansion.md)
under the `batched` spec names. Do not run both masters against this branch. Its W1 (the primitive) is
done and consumed here, not redone.

## Scope

### In scope
- **B0** — Filter-placement classifier (pushdown depth) + one unified per-source bound derivation; drop
  the redundant outer clamp on the transparent (no-lookback) slice. Foundation of Group B.
- **B1** — Wire the monotonicity primitive into its three consumers: `UNION`-branch partitionability,
  subquery/CTE-body pushdown, join driving-fact resolution (adds alias-scoped leaf resolution).
- **B2** — Cross-partition window functions with a bounded `RANGE BETWEEN INTERVAL … PRECEDING` frame
  and the `LAG`/`LEAD` two-layer widened-scan/exact-clamp move; bare `LAG`/`ROWS`/`GROUPS`/`UNBOUNDED`
  stay rejected or per-partition.
- **B3** — Non-determinism split: compile-time pinning of run-deterministic clocks (`NOW`/`CURRENT_*`)
  + payload opt-in `nondeterministic_columns` with the flow/taint check and three hard exclusions.
- **B4** — Group-aligned `HAVING` / `DISTINCT` relaxations; relocate the partition-alignment check to
  per-scope and expose its verdict as the shared alignment signal; keep `LIMIT` rejected.
- **B5** — Run-window ↔ partition-granularity alignment (`g_run ≥ g_part`, aligned boundaries).
- **B6** — Self-referential batched models: derive the *ordered* property from the DAG self-edge and
  enforce strictly-sequential temporal backfill; refuse a non-converging self-reference.
- **B7** — Monotone-integer partition keys (non-temporal `partition_column`: sequence id / offset /
  watermark).
- **B8** — Per-source clamp observability: run-relative `explain --json` window + LSP hover readout.

### Explicitly deferred
- The keyed-mode maintenance rungs (Group C) and the new keyed modes (Group D). Group B touches only
  the `batched` (partitioned-output) member of the refresh axis.
- The batched Open Questions research §18.2 lists as *not yet settled* — scalar subqueries over
  bounded sources, `GROUPING SETS`/`ROLLUP`/`CUBE`, `FOLLOWING`-frame forward reach,
  membership/grouping non-determinism, aggregating-branch unions. Each stays **rejected (fail-closed)**;
  none is a phase here.
- Exact-clamp *migration* (the wholesale switch from widened outer clamps to exact output clamps that
  changes the late-data re-write use case, research §3.2/§8.6). B0 and B2 read the exact margin but the
  B0 decision (below) fixes what carries late-data re-writes; a full migration is out of scope.
- The `-- @materialize:` annotation form (never implemented; frontmatter is the only surface).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| B0    | done    | 18a0f0cd | 2026-07-04 |
| B1    | pending |        |      |
| B2    | pending |        |      |
| B3    | pending |        |      |
| B4    | pending |        |      |
| B5    | pending |        |      |
| B6    | pending |        |      |
| B7    | pending |        |      |
| B8    | pending |        |      |

---

### Phase B0: Filter-placement classifier (pushdown depth) + unified bound derivation

**Goal.** Turn the eligibility check into a **downward walk** that returns the *deepest safe injection
point* for `event_time` (source scan / below-aggregate / above-window-with-lookback), per research
Part 3. Derive the output-clamp write window **and** each per-source scan window from **one**
per-source walk (fixing the §3.2 independent-derivation under-read, where the margin-rewrite path
clipped frames because two derivations disagreed). Drop the redundant outer clamp on the *transparent*
(no-lookback) slice: a transparent single-source subquery emits a single source-level filter, no outer
wrap.

**Pre-conditions.** A1 (`refresh: batched` selector + `batched:` block) and A2 (`BatchedConfig`,
`Batched` codes) landed. Consumes the W1 primitive (`trace_event_time` — do not modify it).

**Depends on.** A1.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — the confirmed §3.2 under-read harness: a model
  whose margin rewrite previously clipped its frame now derives one consistent write-window + source
  window from a single walk; assert the derived `(before, after)` matches the frame's `INTERVAL`.
- `crates/smelt-logical/src/rules/incremental.rs` unit — a transparent single-source subquery model
  emits exactly one source-level filter and **no** outer wrap (assert the injected-SQL shape / the
  injection-point returned by the walk).
- `crates/smelt-logical/src/rules/incremental.rs` unit (fail-closed) — a `NotDerivable` source still
  refuses at planning time (`derive_model_source_bounds` returns `Err`, naming the construct).
- `crates/smelt-cli/tests/incremental/backfill.rs` + `.../lookback.rs` (real fixtures under
  `examples/timeseries/`) — existing batched integration tests stay green (no equivalence regression),
  and the single-source pushdown case matches full refresh.

**Implementation shape.**
- Replace the current parallel derivations (the batch-safety text scans in `analyze_batch_safety`
  at `incremental.rs:56` and the outer-clamp injection) with **one** per-source downward walk that
  returns a `(injection_point, BoundResult)` per source. Reuse the existing
  `derive_model_source_bounds` (`incremental.rs:553`) / `BoundContext` /
  `analysis/source_bounds.rs` machinery as the walk's spine.
- The walk returns the *deepest* safe injection point; a transparent (`Bounded(_,0,0)`, single-column)
  slice pushes to the source and carries no outer clamp.
- Keep the fail-closed refusal (`NotDerivable → Err`) exactly as today.

**Critical files.**
- `crates/smelt-logical/src/rules/incremental.rs` — `analyze_batch_safety` (`:56`),
  `derive_model_source_bounds` (`:553`), `BatchSafety` (`:30`).
- `crates/smelt-logical/src/analysis/source_bounds.rs` — `BoundResult` (`:76`), `BoundContext`
  (`:132`), `derive_model_bounds` (walk entry).
- `crates/smelt-runtime/src/transformer.rs` — `inject_time_filter` (`:272`), `inject_source_filters`
  (`:65`) — the injection consumers of the walk's result.
- (Read-only) `crates/smelt-logical/src/analysis/monotonicity.rs` — the primitive B1 wires; B0 leaves
  it untouched.

**Open decision (for the implementer).** *Exact-clamp vs widened-clamp for the transparent slice.*
Dropping the outer clamp entirely on the transparent slice adopts an exact source clamp; research
§3.2/§8.6 note this changes the late-data re-write use case (a widened DELETE window that mops up
late rows). **State in the phase commit what carries late-data re-writes after B0** — either the
write-window widening in `temporal.rs` (`filter_range`, `crates/smelt-cli/src/temporal.rs:28`) is
unchanged (DELETE still covers the widened window; only the *output clamp* narrows) or the phase is
blocked pending the master's exact-clamp-migration open question. Prefer the former (narrow the output
clamp, keep the DELETE write-window widening) so the idempotence contract in
`batched_models.md` §"Execution model" step 1 is untouched.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — no note is fully removed by B0 alone; add nothing
  and remove nothing except adjusting the §3.2 wording only if a Known-Divergence note references the
  independent-derivation under-read (it does not today — leave the section unchanged).
- `docs-site/docs/guide/incremental-models.md` — no user-facing surface change (internal analysis
  refactor); verify prose still matches.

**Review checklist.**
- [ ] One walk produces both the write window and the per-source scan windows (no second derivation).
- [ ] Transparent slice emits a single source filter, no outer wrap; the equivalence tests pass.
- [ ] `NotDerivable` still refuses (fail-closed) with the construct-naming diagnostic.
- [ ] Late-data re-write path documented in the commit; idempotence contract intact.
- [ ] Existing `incremental_*` integration tests green.

**Commit.** `refactor(logical): unify batched bound derivation into one pushdown-depth walk; drop redundant outer clamp on the transparent slice`

---

### Phase B1: Wire the monotonicity primitive into consumers (UNION / subquery-CTE / joins)

**Goal.** Call `trace_event_time` (via the `smelt-db` nullability-gated wrapper where schema is
available) from the three below-the-outer-SELECT consumers, so the injected filter relocates below the
outermost SELECT when — and only when — the projection is a monotone image of a source clock
(`batched_models.md` §"Event-time monotonicity trace"):
- **UNION branches** (research Part 5): trace each branch independently; an all-`Traceable` set unlocks
  single-stream `UNION ALL` pushdown; a `StaticSeed` branch is the NULL/constant hazard — **named and
  rejected**.
- **Subquery/CTE bodies** (Part 6): one parse-based body classifier replacing *both* the current
  subquery gate and the CTE bypass, applied to derived tables **and** `WITH` CTEs; `Traceable → push`,
  else stay at the outer clamp.
- **Joins** (Part 7): resolve the driving fact (exactly-one-`Traceable` input); window only the
  driving fact, full-scan every other input, so the §7.2/J3 misfilter is 0 by construction. Requires
  **alias-scoped leaf resolution** on top of the primitive (closes the name-based-only gap,
  `batched_models.md` §Known Divergences "Leaf-column resolution is name-based"; research §4.8, §7.4).

**Pre-conditions.** B0 landed (unified walk + injection points).

**Depends on.** B0. Re-homes ex-waves W2–W4.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — the P3 (UNION `StaticSeed`), Q5
  (subquery/CTE body), and J3–J5 (join driving-fact) harnesses already reproduced in W1: each
  `Traceable` case pushes to the named source column; each reject case names the offending construct.
- `crates/smelt-logical/src/analysis/*` unit — alias-scoped leaf resolution: two joined inputs sharing
  a partition-column *name* now resolve the driving side via FROM/alias scope (previously
  ambiguous-`NotTraceable`).
- `crates/smelt-db/…` integration — the nullability gate still downgrades a `Traceable` push on a
  nullable leaf to `NotTraceable` (fail-closed; the wrapper `queries/monotonicity.rs::n` /
  `gate_nullable_leaf`).
- `crates/smelt-cli/tests/incremental_parity.rs` (real fixtures) — a UNION-ALL model and a
  CTE-bodied model match full refresh; example incremental models unaffected.

**Implementation shape.**
- In the classifier, for each of the three constructs, call the primitive at the branch/body/input
  and act on the verdict: `Traceable` annotates the tree node with the source-level filter (per
  `batched_models.md` §"Injection target is semantic, not textual"); `StaticSeed`/`NotTraceable` keeps
  the outer clamp or refuses per the construct's rule.
- Add alias-scoped leaf resolution (map FROM aliases → sources) so the join consumer disambiguates a
  shared partition-column name; feed it into `BoundContext.source_partition_cols`
  (`source_bounds.rs:135`).
- Replace the whole-string subquery gate (`incremental.rs:273` `allow_subqueries` branch) and the CTE
  bypass with the single parse-based body classifier.

**Critical files.**
- `crates/smelt-logical/src/rules/incremental.rs` — the three construct gates (`:230` window, `:273`
  subqueries), driving-fact resolution.
- `crates/smelt-logical/src/analysis/monotonicity.rs` — **call site only** (do not modify the
  primitive); `crates/smelt-db/src/queries/monotonicity.rs` — the nullability-gated `n` wrapper.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — alias-scoped resolution feeding
  `BoundContext`.
- `crates/smelt-runtime/src/transformer.rs` — relocate the filter to the annotated node.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — **remove** the "Event-time monotonicity trace:
  structural primitive emitted; consumers pending" note (consumers now wired) and the "Leaf-column
  resolution is name-based; no alias/FROM resolution" note's *join-consumer prerequisite* clause (now
  added). Leave the AT-TIME-ZONE-parser and per-backend-DuckDB-only notes (unchanged by B1).
- `docs-site/docs/guide/incremental-models.md` — note that `UNION ALL`, subqueries/CTEs, and
  fact⋈lookup joins are now batch-eligible when the event-time projection is monotone.

**Review checklist.**
- [ ] All three consumers call the primitive; none re-derives monotonicity privately.
- [ ] `StaticSeed` UNION branch and non-driving-fact join input are named-and-rejected (fail-closed).
- [ ] Alias-scoped resolution disambiguates shared partition-column names.
- [ ] Nullable-leaf downgrade still holds; soundness oracle green.
- [ ] The two §Known-Divergence notes removed; edits timeless.

**Commit.** `feat(logical): wire event-time monotonicity trace into UNION / subquery-CTE / join consumers`

---

### Phase B2: Window functions — bounded-`RANGE` cross-partition frames + `LAG`/`LEAD`

**Goal.** Admit a cross-partition window function whose `OVER` carries a bounded
`RANGE BETWEEN INTERVAL '…' PRECEDING [AND …]` frame (no `UNBOUNDED` bound) via the primitive + a
derived lookback margin — the two-layer move: widen the *source scan* by the margin, keep an *exact*
output clamp (research Part 8). Keep `ROWS`/`GROUPS`/bare `LAG`/`LEAD`/`UNBOUNDED` rejected or
per-partition (`batched_models.md` §"Batch safety classification" → "Window functions and the
partition_column"; §"Partition-aligned window functions").

**Pre-conditions.** B0 landed. Complements B1's primitive wiring.

**Depends on.** B0.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — a `LAG(...) OVER (PARTITION BY device_id
  ORDER BY event_ts RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW)` model classifies
  `BoundedSafe(n)` with `n = 30 min` (Form A), partition-by-`device_id` admitted despite the model
  being partitioned by a derived `session_start_date`.
- `crates/smelt-logical/src/rules/incremental.rs` unit (fail-closed) — a bare `LAG(event_ts) OVER
  (PARTITION BY device_id ORDER BY event_ts)` (no RANGE) stays `NotDerivable`; a `ROWS`/`GROUPS` frame
  and an `UNBOUNDED PRECEDING` frame are rejected / forced `PerPartitionOnly`.
- `crates/smelt-cli/tests/incremental_parity.rs` (real fixture, e.g. a sessionization model under
  `examples/timeseries/`) — the bounded-`RANGE` `LAG` model matches full refresh across partitions.

**Implementation shape.**
- Extend `has_bounded_range_interval_frame` (`incremental.rs:419`) + `find_inadmissible_over`
  (`incremental.rs:343`) to feed the frame `INTERVAL` into the source-bound deriver as a Form-A
  lookback (so the source read widens), and admit the cross-partition window on that basis.
- Apply the two-layer move: widened scan (source filter `+ margin`) vs exact output clamp — wire
  through B0's injection points.
- Preserve the existing rejects: no RANGE, `ROWS`/`GROUPS`, `UNBOUNDED` → refuse or `PerPartitionOnly`;
  `safety_overrides.allow_window_functions: true` remains the escape hatch.
- **Forward-reach opportunity (`after_secs` mirror).** B2 derives the *backward* (`before_secs`)
  margin from a `RANGE … PRECEDING` frame. The symmetric *forward* (`after_secs`) reach — from a
  `RANGE … FOLLOWING` frame or a `BETWEEN event.ts AND event.ts + INTERVAL` predicate — is the
  currently-unworked mirror (`batched_models.md` §8.3 forward reach) and is the sole new engine
  dependency of the accumulating-snapshot peer (**research
  [`20260703-model-updates.md`](../research/20260703-model-updates.md) Part 20**, §20.5). It is the
  same walk with the opposite sign of margin; landing it here while this code is open is cheaper than
  deferring it to that peer. Optional for B2's own goal, but note in the commit whether it was wired.

**Critical files.**
- `crates/smelt-logical/src/rules/incremental.rs` — `find_inadmissible_over` (`:343`),
  `has_bounded_range_interval_frame` (`:419`), `extract_balanced_parens` (`:445`),
  `find_partition_by_in_over` (`:478`).
- `crates/smelt-logical/src/analysis/source_bounds.rs` — Form-A margin → `BoundResult::Bounded`.
- `crates/smelt-runtime/src/transformer.rs` — widened-scan vs exact-clamp injection.

**Open decision (for the implementer).** *Whether `find_inadmissible_over`'s text scan is replaced by
a parse-based frame reader in this phase* — the §Known-Divergence note "Window-function batch-safety
check also runs on unexpanded outer SQL" (function-body `OVER` invisibility) is a **separate** gap
tracked in `20260530-thread-fn-registry-classification.md`; B2 need not close it. If B2 keeps the
text scan, do **not** remove that note; if it happens to parse the frame properly, note the reduction.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — no full note removed by B2 (the window
  §Known-Divergence is the function-body-invisibility one, out of B2 scope). Verify §"Batch safety
  classification" → "Window functions and the partition_column" prose matches the shipped admission.
- `docs-site/docs/guide/incremental-models.md` — document the bounded-`RANGE` window admission and the
  rejected frame shapes.

**Review checklist.**
- [ ] Bounded-`RANGE` `LAG`/`LEAD` admitted cross-partition and equivalence-tested.
- [ ] Bare `LAG`, `ROWS`/`GROUPS`, `UNBOUNDED` still rejected/per-partition (fail-closed).
- [ ] Two-layer move: source widened, output clamp exact.
- [ ] Function-body-`OVER` §Known-Divergence note left intact (out of scope) unless genuinely closed.

**Commit.** `feat(logical): admit bounded-RANGE cross-partition window functions via derived lookback margin`

---

### Phase B3: Non-determinism — run-pinning + payload opt-in (`nondeterministic_columns`)

**Goal.** Split the current single non-determinism reject (`batched_models.md` §"Safety checks"):
(1) **pin** run-deterministic clocks (`NOW`/`CURRENT_*`) at compile time — admissible, the value is
frozen once per run; (2) admit **row-nondeterministic** (`RANDOM`/`UUID`) only when confined to a
column listed in the `batched:` block's `nondeterministic_columns`, gated by a flow/taint check with
the three hard exclusions (event-time/partition/unique-key; membership/grouping position). Per research
Parts 9.1–9.2 and `batched_models.md` §"Non-determinism and the equivalence contract" + Constraint 13.

**Pre-conditions.** A1 landed (`batched:` block exists to carry `nondeterministic_columns` —
this field is a **B3 addition**, per the A1 scope note; it does not exist before B3).

**Depends on.** A1.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` / `metadata.rs` unit — `nondeterministic_columns` parses into
  `BatchedConfig`; listing `event_time_column` / `partition_column` / a `unique_key` column is a
  **configuration error** (Constraint 13; `batched_models.md` §Surface).
- `crates/smelt-logical/src/rules/incremental.rs` unit — `inserted_at = NOW()` flowing only into a
  listed column **builds**; `RANDOM()` in a `WHERE` / `GROUP BY` / `PARTITION BY` still **rejects**
  (fail-closed), naming the offending position; `NOW()` pinned at compile time is admitted even without
  a listed column (run-deterministic).
- `crates/smelt-logical/…` unit — the flow/taint check: a non-deterministic value reaching a listed
  payload column is admitted, reaching a non-listed column or an excluded role is rejected.
- `crates/smelt-cli/tests/incremental_*.rs` (real fixture) — a `nondeterministic_columns` example
  workspace builds and its deterministic skeleton matches full refresh.

**Implementation shape.**
- Add `nondeterministic_columns: Vec<String>` to the `batched:` block struct (`config.rs`;
  post-A2 `BatchedConfig`).
- Split the current `allow_nondeterministic` branch (`incremental.rs:288`) into: a run-clock pinner
  (compile-time freeze of `NOW`/`CURRENT_*`) and a payload flow check. The blunt
  `safety_overrides.allow_nondeterministic` remains but discouraged.
- Flow/taint: from each non-deterministic call, follow its value; admit iff every sink is a listed
  payload column; reject with the three hard exclusions naming the role.

**Critical files.**
- `crates/smelt-core/src/config.rs`, `metadata.rs` — `BatchedConfig` field + the excluded-column
  config-error validation.
- `crates/smelt-logical/src/rules/incremental.rs` — non-determinism branch (`:288`), new flow check.
- `crates/smelt-logical/src/types.rs` — safety-override types (post-A2 `BatchedSafetyOverrides`).
- `examples/` — add a `nondeterministic_columns` fixture (audit-stamp column).

**Open decision (for the implementer).** *Depth of the flow/taint check.* Research §9.2 specifies a
value-flow/taint analysis; a minimal version tracks only direct projection (`col = NOW()`), a fuller
one follows through CTEs/subqueries. Ship the **direct-projection** analysis first (fail-closed on any
indirection: an unresolvable flow rejects), and record any deferred indirection depth under §Deferred.
The **membership/grouping** non-determinism case (a non-deterministic `WHERE`/`GROUP BY`) is
**out of scope even with the opt-in** (research §9.1a) — keep it rejected.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — **remove** the "Opt-in non-deterministic columns
  not yet implemented" note (its payload-opt-in clause; keep the membership/grouping-out-of-scope
  clause, re-worded as a plain out-of-scope statement, not a phase reference).
- `docs-site/docs/guide/incremental-models.md` — document `nondeterministic_columns` and the three
  hard exclusions.

**Review checklist.**
- [ ] Run-clock pinning and payload opt-in are distinct paths.
- [ ] Excluded-column listing is a config error; membership/grouping non-determinism stays rejected.
- [ ] Deterministic skeleton equivalence-tested; flow check fail-closed on indirection.
- [ ] §Known-Divergence payload-opt-in note removed; edits timeless.

**Commit.** `feat(batched): pin run-deterministic clocks + admit payload non-determinism via nondeterministic_columns`

---

### Phase B4: Group-aligned `HAVING` / `DISTINCT`; relocate the partition-alignment check per-scope

**Goal.** Admit `HAVING` when the `GROUP BY` key ⊇ `partition_column`; admit `DISTINCT` when its key
⊇ `partition_column`; keep `LIMIT` rejected (never commutes with the partition filter). Relocate the
partition-in-`GROUP BY` check to per-branch / per-body scopes and expose its verdict as the **shared
partition-alignment signal** other B phases (B1 UNION/subquery, B2 windows) consume. Per research
Parts 9.4–9.5.

**Pre-conditions.** B0 landed (per-scope walk); shares the alignment signal with B1.

**Depends on.** B0.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/incremental.rs` unit — a group-aligned `HAVING` (GROUP BY ⊇
  `partition_column`) **builds**; a non-aligned `HAVING` / `DISTINCT` **refuses** (fail-closed);
  `LIMIT` always refuses.
- `crates/smelt-logical/src/rules/incremental.rs` unit — the partition-alignment verdict is computed
  per-scope (a subquery body's own `GROUP BY`, not just the outer one) and is reused by the UNION /
  window admission (shared signal).
- `crates/smelt-cli/tests/incremental_parity.rs` (real fixture) — a group-aligned `HAVING` model
  matches full refresh across partitions.

**Implementation shape.**
- Replace the whole-model `allow_having` text gate (`incremental.rs:248`) and the `DISTINCT` check with
  a superset test (`GROUP BY`/`DISTINCT` key ⊇ `partition_column`) run at each scope B0 walks.
- Extract the existing partition-in-`GROUP BY` alignment check into a reusable per-scope function
  returning an `Aligned`/`NotAligned` verdict; feed it to B1/B2 consumers as the shared signal.
- `LIMIT` stays unconditionally rejected.

**Critical files.**
- `crates/smelt-logical/src/rules/incremental.rs` — `allow_having` branch (`:248`), `DISTINCT` check,
  the alignment-check extraction; `analyze_select` scope walk.
- `crates/smelt-logical/src/analysis/mod.rs` — `SelectAnalysis` (per-scope items) feeding the check.

**Docs touched.**
- `docs/specs/batched_models.md` §"Safety checks" — verify the `HAVING`/`DISTINCT`/`LIMIT` prose
  matches the shipped superset rule (no §Known-Divergence note is dedicated to B4 today; none removed).
- `docs-site/docs/guide/incremental-models.md` — document the group-aligned `HAVING`/`DISTINCT`
  admission and that `LIMIT` is never admitted.

**Review checklist.**
- [ ] Group-aligned `HAVING`/`DISTINCT` admitted and equivalence-tested; non-aligned refused.
- [ ] `LIMIT` unconditionally rejected.
- [ ] Alignment check is per-scope and exposed as the shared signal B1/B2 consume.

**Commit.** `feat(logical): admit group-aligned HAVING/DISTINCT; expose per-scope partition-alignment signal`

---

### Phase B5: Run-window ↔ partition-granularity alignment (`g_run ≥ g_part`)

**Goal.** Enforce `g_run ≥ g_part` with aligned boundaries (research Part 10;
`batched_models.md` §"Run window vs partition granularity", §CLI). Derive `g_part` from the
partition-column transform unit via the primitive; validate the CLI run window against it.

**Pre-conditions.** A1 landed (run-window plumbing exists in the CLI).

**Depends on.** A1.

**TDD tests to write first.**
- `crates/smelt-cli/src/temporal.rs` unit — a run window finer than `g_part` (e.g. an hourly window on
  a daily-partitioned model) is rejected (or auto-coarsened — see open decision) with a clear message;
  a `g_run ≥ g_part` aligned window passes.
- `crates/smelt-cli/tests/incremental/backfill.rs` (real fixture) — an incomplete final partition is
  handled per §10.3 (the run window's last partial partition).
- `crates/smelt-cli/src/temporal.rs` unit — `g_part` derived from the partition-column transform unit
  (`DATE_TRUNC('day', …) → day`) matches the model's declared `timeseries.granularity`.

**Implementation shape.**
- In run-window validation (`temporal.rs`, `filter_range` at `:28` and the alignment check the CLI
  already does for whole-granularity multiples), add the `g_run ≥ g_part` comparison with
  boundary-alignment, using the granularity arithmetic in `timeseries.md`.
- Derive `g_part` via the monotonicity trace's `is_strict=false` many-to-one form (`DATE_TRUNC` unit)
  rather than re-parsing.

**Critical files.**
- `crates/smelt-cli/src/temporal.rs` — `filter_range` (`:28`), run-window validation.
- `crates/smelt-core/src/config.rs` — `Granularity` arithmetic helpers.
- `crates/smelt-logical/src/analysis/monotonicity.rs` — read `g_part` from the trace (call site only).

**Open decision (for the implementer, flagged by the master).** *Hard-validate vs auto-coarsen the run
window* — research Part 10 leaves this open. **Ship hard-validation first** (reject a sub-`g_part`
window with a message telling the user the minimum window), the conservative fail-closed choice;
record auto-coarsen as a deferred enhancement under §Deferred. Do not silently coarsen.

**Docs touched.**
- `docs/specs/batched_models.md` §"Run window vs partition granularity" — verify prose matches the
  shipped hard-validation; no §Known-Divergence note removed (this is new enforcement of an
  already-specified invariant).
- `docs-site/docs/guide/incremental-models.md` — document the `g_run ≥ g_part` requirement and the
  error on a too-fine run window.

**Review checklist.**
- [ ] Sub-`g_part` run window rejected (hard-validation) with a clear minimum-window message.
- [ ] `g_part` derived from the partition-column transform, matching declared granularity.
- [ ] Incomplete final partition handled per §10.3.

**Commit.** `feat(cli): enforce run-window ≥ partition-granularity alignment for batched runs`

---

### Phase B6: Self-referential batched models — ordered execution

**Goal.** Detect a batched model's self-edge in the DAG, mark it **ordered**, and make the backfill
chunker build its windows strictly sequentially in temporal order (no parallel / out-of-order
backfill). A self-reference the planner cannot prove converges partition-by-partition (reads *forward*
or across whole history) is refused at planning time. Per research Part 11 and
`batched_models.md` §"Window independence and self-referential models".

**Pre-conditions.** A1 landed.

**Depends on.** A1.

**TDD tests to write first.**
- `crates/smelt-planner/…` unit — a batched model whose SQL reads `smelt.<self>` prior partitions is
  detected as self-referential and marked `ordered`; a batched model with no self-edge is
  window-independent (parallelisable).
- `crates/smelt-planner/…` unit (fail-closed) — a self-reference reading *forward* / whole-history is
  refused at planning time with a diagnostic naming the non-convergent self-edge.
- `crates/smelt-cli/tests/incremental/backfill.rs` (real fixture) — a running-balance model reading
  yesterday's close backfills correctly in temporal order and is **never** parallelised or reordered;
  the end state matches a strictly-sequential reference build.

**Implementation shape.**
- Detect the self-edge in the model DAG (`crates/smelt-logical/src/graph.rs`, `ModelGraph`); mark the
  model `ordered`.
- Thread the `ordered` flag into the backfill chunker so `ordered` models chunk one window at a time in
  temporal order (no parallel dispatch, no out-of-order sub-ranges); window-independent models keep the
  existing auto-chunking.
- Refuse a non-converging self-reference (forward/whole-history) at planning time.

**Critical files.**
- `crates/smelt-logical/src/graph.rs` — `ModelGraph` (`:17` carries `incremental_config`/
  post-A2 `batched_config`); self-edge detection.
- `crates/smelt-planner/src/rules/incremental.rs` (re-export) / the chunker — ordered execution.
- `crates/smelt-cli/src/executor.rs` — `execute_*_incremental` (`:195`+) backfill dispatch honours
  `ordered`.

**Open decision (for the implementer).** *How "converges partition-by-partition" is proven.* The spec
requires refusing a self-reference the planner cannot prove reads only immediately-prior partitions.
A minimal proof: admit only self-references whose `smelt.<self>` read carries a backward-bounded
Form-B filter (prior partitions), refuse everything else (fail-closed). Record the exact admitted
shape in the phase; broader convergence proofs are deferred.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — **remove** the "Self-referential (ordered)
  batched models are specified but not yet enforced" note.
- `docs-site/docs/guide/incremental-models.md` — document that a self-referential batched model runs
  ordered and cannot be parallelised, and that a forward self-reference is refused.

**Review checklist.**
- [ ] Self-edge detected; model marked `ordered`; enforced in the chunker (never parallelised).
- [ ] Forward / whole-history self-reference refused at planning time (fail-closed).
- [ ] Running-balance fixture backfills to the strictly-sequential reference state.
- [ ] §Known-Divergence self-referential note removed; edits timeless.

**Commit.** `feat(planner): derive ordered execution from a batched model's DAG self-edge; enforce sequential backfill`

---

### Phase B7: Monotone-integer partition keys

**Goal.** Generalise the time-typed batched machinery to a non-temporal **monotone** `partition_column`
(sequence id / offset / watermark): integer offsets/bands in the monotonicity whitelist, `g_part` and
lookback margins for integer keys, `Offset` generalised past `Seconds`. Per research §18.3;
`batched_models.md` §Surface ("`partition_column` must be monotone — a timestamp *or* an ever-increasing
integer").

**Pre-conditions.** B0 landed (unified walk carries the bound + offset).

**Depends on.** B0.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/monotonicity.rs` unit — the whitelist admits monotone integer
  offset arithmetic (`batch_id + <const>`, integer bands) as `Traceable`; a non-monotone integer
  transform (`batch_id % N`, `batch_id * -1`) is `NotTraceable` (fail-closed).
- `crates/smelt-logical/src/analysis/source_bounds.rs` unit — `Offset` carries an integer magnitude
  (not only `Seconds`); a `Bounded(c, k, 0)` integer lookback derives.
- `crates/smelt-cli/tests/incremental_parity.rs` (real fixture) — a model partitioned by a monotone
  `batch_id` integer builds and backfills, matching full refresh.

**Implementation shape.**
- Generalise `Offset` (`monotonicity.rs:30`, currently `Seconds(Seconds)`) to also carry an integer
  offset; extend the whitelist classifier to admit integer monotone forms and reject non-monotone
  integer transforms.
- Thread integer `g_part` / lookback margins through `derive_model_source_bounds` and the source-filter
  injection (integer `WHERE c >= run_start - k AND c < run_end + k`).

**Critical files.**
- `crates/smelt-logical/src/analysis/monotonicity.rs` — `Offset` (`:30`), the classifier whitelist
  (`classify_binary` `:370`, `parse_interval_literal` `:427`) extended for integer keys.
- `crates/smelt-logical/src/analysis/source_bounds.rs` — integer `BoundResult`/`Offset`.
- `crates/smelt-runtime/src/transformer.rs` — integer source-filter injection.

**Open decision (for the implementer).** *Whether integer keys reuse the ISO-8601 duration rendering
of the clamp readout (B8) or get an integer rendering.* An integer `Offset` has no ISO-8601 form;
decide the readout string for integer bounds (a bare integer count) and keep B8's rendering polymorphic
over `Offset`. Flag if this forces a B8 change.

**Docs touched.**
- `docs/specs/batched_models.md` §Surface / §"Per-source bound derivation" — verify the monotone-integer
  prose matches; no dedicated §Known-Divergence note today (the "monotone timestamp *or* integer"
  wording is already in §Surface) — remove none unless one is added by an earlier phase.
- `docs-site/docs/guide/incremental-models.md` — document integer `partition_column` support.

**Review checklist.**
- [ ] Integer monotone forms admitted; non-monotone integer transforms rejected (fail-closed).
- [ ] `Offset` carries integer magnitudes; integer source filters inject correctly.
- [ ] Integer-partitioned fixture backfills to the full-refresh state.

**Commit.** `feat(logical): support monotone-integer partition columns in batched eligibility and bound derivation`

---

### Phase B8: Per-source clamp observability

**Goal.** Finish the two observability surfaces already specified in
`batched_models.md` §"Observing the per-source clamp": `smelt explain --json` resolves the run-relative
scan window `[run_start − before, run_end + after)` when a run window is supplied; LSP hover on a
`smelt.<path>` reference inside a batched model shows its derived clamp alongside the existing
schema/column readout. Both render via the ISO-8601 duration rendering of the bound machinery
(`Seconds::to_iso8601`).

**Pre-conditions.** None strictly (reads B0's bound map); sequence last so it renders the final
bound shapes (including B7's integer bounds).

**Depends on.** — (master lists no dependency; sequence after B7 so the readout covers integer bounds).

**TDD tests to write first.**
- `crates/smelt-cli/src/explain.rs` unit / `crates/smelt-cli/tests/…` — `explain --json
  --event-time-start/--event-time-end` reports the concrete `[run_start − before, run_end + after)`
  window per source; without a run window it reports the symbolic offsets only (as today).
- `crates/smelt-cli/src/explain.rs` unit — the four bound outcomes render distinctly
  (`Bounded(c,0,0)` / `Bounded(c,before,after)` / `Unbounded` / lookup) per the §"Observing the
  per-source clamp" table.
- `crates/smelt-lsp/tests/…` (hover integration) — hovering a `smelt.<path>` source reference inside a
  batched model shows its derived clamp + window shape alongside the schema/column readout; a
  `NotDerivable` model shows the refusal, not a window.

**Implementation shape.**
- In `build_explain_output` / `ExplainIncremental` (`explain.rs:44`, `source_bounds` at `:56`,
  `SourceBoundJson` at `:62`, `map_source_bounds` at `:273`), when a run window is present, resolve the
  concrete window and add it to `SourceBoundJson::Bounded`; keep the symbolic-only path when absent.
- Extend LSP hover (`crates/smelt-lsp/src/hover.rs`) with a batched-clamp readout for `smelt.<path>`
  references: reuse the `smelt-logical` bound map + the `Seconds::to_iso8601` rendering; append to the
  existing schema/column hover (do not replace it).

**Critical files.**
- `crates/smelt-cli/src/explain.rs` — `ExplainIncremental` (`:44`), `SourceBoundJson` (`:62`),
  `map_source_bounds` (`:273`), `compute_batch_safety_label` (`:314`).
- `crates/smelt-lsp/src/hover.rs` — new batched-clamp hover branch (alongside `hover_text_for_*`).
- `crates/smelt-logical/src/analysis/source_bounds.rs` — the bound map + `Seconds::to_iso8601`.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — **remove** the "Per-source clamp observability
  partly emitted" note (both surfaces now ship). Leave the parenthetical about no clamp *warning*
  (deliberately out of scope) as a plain statement.
- `docs-site/docs/guide/incremental-models.md` — document `explain --json` run-relative windows and the
  editor-hover clamp readout.

**Review checklist.**
- [ ] `explain --json` with a run window reports the concrete `[run_start − before, run_end + after)`;
      without one, symbolic offsets only.
- [ ] Four bound outcomes render distinctly; hover shows the clamp alongside schema.
- [ ] Integer bounds (B7) render sensibly.
- [ ] §Known-Divergence observability note removed; edits timeless.

**Commit.** `feat(cli,lsp): resolve run-relative source clamp in explain --json + surface it in LSP hover`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only.)

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- The generative monotonicity soundness oracle
  ([`docs/plans/20260702-monotonicity-primitive-tested.md`](20260702-monotonicity-primitive-tested.md))
  stays green throughout — no relaxation admits a `Traceable` verdict that breaks the pushdown
  commutation identity.
- **Per-partition equivalence, per phase.** Each of B1–B7 has a full-refresh-equivalence real-fixture
  test (`crates/smelt-cli/tests/incremental_parity.rs` / `.../incremental/backfill.rs` under
  `examples/timeseries/`) **and** a fail-closed unit test (an unsound form stays rejected, naming the
  construct), per the master §"Post-implementation verification (per group)" B row.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test
  example_workspaces` — example workspaces build with zero diagnostics.
- `/smelt:validate batched_models` reports zero drift for the surfaces this group touches; every
  §Known-Divergence note this plan lists as removed (B1 monotonicity-consumers + leaf-resolution-join
  clause; B3 payload-opt-in; B6 self-referential; B8 observability) is gone from
  `docs/specs/batched_models.md`.
</content>
</invoke>
