# Plan: Derived Output Window for Partition-Grain Runs

**Date**: 2026-07-11
**Spec**: [`docs/specs/model_transforms.md`](../specs/model_transforms.md) §Semantics "The output window is derived, never assumed", §Design "Rejected: auto-widening the write window to the scan margin" / "Derived output window composes with chunking", §Constraints "Write window = output window"; [`docs/specs/batched_models.md`](../specs/batched_models.md) §"Execution model (DuckDB, current)" items 1–2
**Spec diff**: commit `b7c2d270`
**Tracking PR / branch**: `worktree-incremental`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec sections named above — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` — in particular **maintenance-plan purity** (window/skew derivation is a pure function in `smelt-logical`; the runtime consumes, never re-derives) and the **property composition walk rule** (a new SQL-text scan is admissible only as a leaf classifier the walk invokes).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to specs and `docs-site/docs/...` describe the feature as if it has always existed.
- Build/test env: `export DUCKDB_LIB_DIR=~/.local/lib/duckdb; export LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH` inline on every cargo command.

---

## Context

A run's DELETE range and output clamp are built from the batch's run window verbatim, so a model whose `partition_column` is derived and skews away from the driving date column (a Form B relation) silently under-writes: the run that receives skew-reaching data computes the correct neighbour-partition row and clamps it away, and no later run's window contains that partition (`model_transforms.md` §Known Divergences "Output-window derivation is unbuilt"; deterministic repro in `docs/plans/20260710-web-analytics-maintenance-demo.md` §"Deferred during implementation"). The spec now requires the output window to be **derived** — identity in the common case, skew-inverted `[start − after, end + before)` under a Form B relation — with every written partition's scan sized from the derived window's reach, composing with backfill chunking.

## Scope

### In scope (spec coverage)
- `model_transforms.md` §Semantics "The output window is derived, never assumed" — pure skew-bound derivation + window inversion.
- `batched_models.md` §"Execution model" items 1–2 — DELETE range, output clamp, and per-batch scan all keyed to the derived output window; transparent-slice fast path restricted to zero-skew.
- `batched_models.md` §Known Divergences (harness gap) — strengthen the Rust session assertion with `(session_end, event_count)`.
- Example truth: `examples/web_analytics` comments/README describe the mechanism as it exists.
- `smelt explain --show-sql` statement emission for a model whose FROM is a function call (prerequisite for embedding real sessions SQL in the tutorial).
- Tutorial: cross-midnight prior-day rewrite section in `docs-site/docs/examples/web-analytics-maintenance.md`, freshness-gated.
- `model_transforms.md` §Semantics "semantic cap" paragraph — two-boundary truncation at the declared bound, asserted equivalent between day-by-day and full builds, explained in the tutorial (Phase 6; spec edit committed with the plan extension).

### Explicitly deferred
- Key-grain (`grain: key`) interaction — the skew inversion applies to the partition grain's recompute corner only; keyed folds keep their ledger semantics.
- Integer (non-date) partition-column skew — the windowing machinery is date-typed throughout (`batched_models.md` §Known Divergences, monotone-integer entry).
- Sub-granularity skew rounding beyond outward alignment to granularity boundaries.
- `smelt explain --json` surfacing of the derived skew bound as a first-class field (the propagation edges already carry before/after; a dedicated `output_window` readout is future observability work).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | f5144f69 | 2026-07-11 |
| 2     | done     | fe7f13c5 | 2026-07-11 |
| 3     | done     | 98813033 | 2026-07-11 |
| 4     | done     | 23d5c35b | 2026-07-11 |
| 5     | done     |        | 2026-07-11 |
| 6     | pending  |        |      |

---

### Phase 1: Pure partition-skew derivation in `smelt-logical`

**Goal.** A pure function derives the model's partition-column skew bound `(before, after)` from its expanded SQL: the Form B relation whose *anchor* is the model's own `partition_column` and whose LHS is a different date column (`driving_date BETWEEN partition_column − before AND partition_column + after`). Identity models (no such relation, or partition column = event-time column) derive `(0, 0)`.

**Pre-conditions.** None (first phase).

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/source_bounds.rs::tests::partition_skew_form_b_symmetric` — sessions-shaped SQL (`WHERE event_date BETWEEN session_start_date - INTERVAL '1 day' AND session_start_date + INTERVAL '1 day'`) with `partition_column = session_start_date` derives `(1 day, 1 day)`.
- `…::tests::partition_skew_identity_zero` — a model whose partition column is the event-time column (no Form B on it) derives `(0, 0)`.
- `…::tests::partition_skew_ignores_source_form_b` — a Form B filter anchored on an *upstream source's* partition column (the existing per-source bound pattern) does **not** contribute to the model's own skew.
- `crates/smelt-cli/tests/cli_unit/web_analytics_source_bounds.rs::sessions_skew_bound_derived` — real fixture: `examples/web_analytics/models/silver/sessions.sql` (expanded) derives skew `(1 day, 1 day)`.

**Implementation shape.** New leaf classifier in `analysis/source_bounds.rs` (e.g. `extract_partition_skew_bounds(upper_sql, partition_col) -> Vec<(Seconds, Seconds)>`) — mirrors `extract_form_b_bounds` but matches the anchor side rather than the LHS; a pure wrapper `derive_partition_skew(sql, partition_column) -> Skew { before, after }` takes the max over matches. Invoked from the shared walk as a leaf per the property-composition rule; doc-comment classifies it as a leaf classifier. The skew value joins the maintenance-plan data (pure, derived once).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/source_bounds.rs` — classifier + derivation + unit tests
- `crates/smelt-logical/src/analysis/walk.rs` — walk invocation, if the walk owns the composition point
- `crates/smelt-cli/tests/cli_unit/web_analytics_source_bounds.rs` — fixture test

**Docs touched.**
- None beyond code (spec already states the rule; nothing user-visible until Phase 2 wires it).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Derivation is a pure function; no Salsa/runtime dependency (maintenance-plan purity)
- [ ] New scan is a leaf classifier invoked from the walk, classified in a doc comment (walk rule; `walk_coverage` gate green)
- [ ] Direction of inversion correct: the `+ after` side of the relation extends the window *earlier*, `− before` extends it *later*
- [ ] No scope creep into the runtime (Phase 2)

**Commit.** `feat(logical): derive partition-column skew bound from Form B relations anchored on the model's own partition column`

---

### Phase 2: Runtime output-window derivation (DELETE + clamp + scan)

**Goal.** The windowing seam derives the output window from the run window using Phase 1's skew — `[start − after, end + before)`, rounded outward to granularity boundaries — and batches over it. DELETE range, output clamp, and per-batch scan widening then all key off output-window batches with no further changes at the execute loop. The transparent-slice fast path (outer clamp dropped) is restricted to zero-skew models.

**Pre-conditions.** Phase 1 merged.

**TDD tests to write first.**
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::single_day_replay_rewrites_prior_day_partition` — small dedicated fixture (sessions-shaped model over a two-event source straddling midnight, gap < 30 min): replay day 1 then day 2 as single-day windows; assert the day-1 partition's session row ends up with `event_count = 2` and `session_end` on day 2 (red today — this is the minimal repro of the web_analytics divergence).
- `…::identity_model_windows_unchanged` — a zero-skew model's emitted DELETE range and clamp equal the run window verbatim (guards against regressions in the common case).
- `crates/smelt-runtime/tests/source_pushdown_unit.rs::skewed_batch_scan_sized_from_output_window` — for a skewed batch `[D−1, D+2)` with source lookback 1 day, the injected source filter covers `[D−2, D+3)`.
- Existing gates stay green: `cargo test -p smelt-runtime --test statement_parity`, `crates/smelt-cli/tests/e2e/two_layer_clamp.rs`.

**Implementation shape.** In `crates/smelt-runtime/src/windowing.rs::compute_incremental_windows`: compute `output_range = [full_range.start − after, full_range.end + before)` (skew from Phase 1's pure fn over the expanded SQL, aligned outward per `timeseries.granularity`), then chunk **that** range into `IncrementalBatch`es exactly as today — `filter_start`/`filter_end` widening is already batch-relative, so scan-from-derived-window falls out. `execute.rs` keeps building `run_range`/`PartitionRange` from `batch.partition_start/end` (now output-window batches) unchanged. In `transformer.rs`, `is_transparent_single_source` (or its call site) additionally requires zero skew so the outer clamp is kept for write-rebasing models. Wide-batch warning and alignment validation apply to the run window as declared, not the derived window.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/windowing.rs` — output-window derivation + chunking
- `crates/smelt-runtime/src/execute.rs` — comment updates where "run window" becomes "derived output window"; no new window math here
- `crates/smelt-runtime/src/transformer.rs` — zero-skew condition on the transparent fast path
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs` (+ fixture under `crates/smelt-cli/tests/fixtures/` or a minimal `examples/`-style temp workspace, matching `two_layer_clamp.rs` conventions)
- `crates/smelt-runtime/tests/source_pushdown_unit.rs`

**Docs touched.**
- `docs-site/docs/guide/incremental-models.md` — short section: the output window is derived from the run window; a derived, skewing partition column (Form B) makes a run rewrite the neighbour partitions its new data reaches; chunking still bounds per-statement write size.
- `docs/specs/model_transforms.md` — flip the catalogue row "Output-window derivation" to **built**; drop the corresponding Known Divergences entry.
- `docs/specs/batched_models.md` — drop the "Output-window derivation is unbuilt" divergence half (keep the harness-gap half until Phase 3).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Write window = output window exactly (DELETE range ≡ clamp range) for both skewed and identity models
- [ ] Scan sized from output window's reach, not run window's (`skewed_batch_scan_sized_from_output_window`)
- [ ] Transparent fast path never fires for a skewed model
- [ ] `statement_parity` and `two_layer_clamp` green; no authoring outside `smelt-logical` emitters
- [ ] Docs/spec edits timeless

**Commit.** `feat(runtime): derive the output window from the run window via partition-column skew inversion`

---

### Phase 3: Harness strengthening + example truth

**Goal.** The equivalence harness can see this failure class, and the web_analytics example describes the real mechanism.

**Pre-conditions.** Phase 2 merged (otherwise the strengthened assertion is red for the wrong reason).

**TDD tests to write first.**
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` — extend `SessionRow` equality to assert `(session_id, session_start, session_end, event_count, utm_campaign)` set equality between day-by-day and full-window builds (currently `(session_id, utm_campaign)` only). Verify by reverting the Phase 2 windowing change locally: the strengthened assertion must fail on the pre-fix runtime for a cross-midnight fixture (add one to the harness's datagen window if its current 7-day data has no cross-midnight pair — force one via a fixture event pair rather than scale).

**Implementation shape.** Assertion-only change in the Rust harness plus a guaranteed cross-midnight event pair in its fixture data. Example edits are comment/docs-only: `sessions.sql` / `sessionize.sql` Form B comments now describe behaviour that exists (write-window rebase to the skew-inverted window); README's "Why sessions spans midnight" section aligned. Mark the open-investigation note in `docs/plans/20260710-web-analytics-maintenance-demo.md` resolved with a pointer to this plan (append-only, per that section's convention).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`
- `examples/web_analytics/models/silver/sessions.sql`, `examples/web_analytics/functions/sessionize.sql`, `examples/web_analytics/README.md` — comments only
- `docs/plans/20260710-web-analytics-maintenance-demo.md` — resolution note
- `docs/specs/batched_models.md` — drop the harness-gap sentence from Known Divergences

**Docs touched.**
- Covered above (README + spec divergence cleanup).

**Review checklist** (material findings only):
- [ ] Strengthened assertion demonstrably red on the pre-fix runtime (recorded in the phase notes), green on HEAD
- [ ] Example comments match emitted behaviour (spot-check with `smelt run --select silver.sessions -v`)
- [ ] No model SQL semantics changed — comments/README only
- [ ] Spec + docs edits timeless

**Commit.** `test(e2e): assert session end/count equivalence; align web_analytics comments with derived output window`

---

### Phase 4: `explain --show-sql` clamp injection through function-at-FROM

**Goal.** `smelt explain <model> --show-sql` emits statements for a model whose outermost FROM is a transparent function call (today: "failed to inject the output clamp: No FROM clause found"), by injecting over the **expanded** SQL exactly as the live run does — the single-owner emission path the tutorial embeds.

**Pre-conditions.** Phase 2 merged (emitted windows must be the derived ones).

**TDD tests to write first.**
- `crates/smelt-cli/tests/explain_model.rs::sessions_show_sql_emits_statements` — real fixture: `smelt explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11` in `examples/web_analytics` emits a non-empty DELETE+INSERT group whose DELETE range and clamp are the skew-inverted window `['2026-04-09', '2026-04-12')`, and whose scan filter covers `['2026-04-08', '2026-04-13')`.
- A unit test in the emission path asserting clamp injection succeeds on `SELECT ... FROM smelt.functions.f(...)`-shaped SQL after expansion.

**Implementation shape.** Route the explain/dry-run statement-emission path through function expansion before `derive_batch_filtered_sql` (the live run already compiles expanded SQL) — likely a call-ordering fix where the emission branch passes unexpanded `model.content`. Statement parity's per-family executed-vs-emitted leg is the oracle: emitted text must equal what a live run executes.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` (dry-run/emission branch) and/or `crates/smelt-cli/src/` explain plumbing — whichever passes unexpanded SQL today
- `crates/smelt-cli/tests/explain_model.rs`

**Docs touched.**
- `docs-site/docs/examples/web-analytics-maintenance.md` — remove the paragraph + placeholder block documenting the "no statements print for this model" limitation (regenerated properly in Phase 5).

**Review checklist** (material findings only):
- [ ] Emitted statements for `silver.sessions` match a live run's executed statements (statement-parity leg)
- [ ] No second emission path introduced — single owner preserved
- [ ] Docs edit timeless

**Commit.** `fix(runtime): expand function calls before explain --show-sql clamp injection`

---

### Phase 5: Tutorial — the cross-midnight prior-day rewrite

**Goal.** The web-analytics maintenance tutorial demonstrates the derived output window on `silver.sessions` with real emitted SQL: a single-day run whose DELETE+INSERT covers `[D−1, D+2)`, rewriting the prior-day partition when a session crosses midnight.

**Pre-conditions.** Phase 4 merged.

**TDD tests to write first.**
- `crates/smelt-cli/tests/tutorial_freshness.rs` — extend the freshness gate to cover the new `smelt-generate` block(s): the embedded sessions `explain --show-sql` output must match a live regeneration (red first: add the gate entry pointing at the not-yet-written section).

**Implementation shape.** Extend `examples/web_analytics/generate_tutorial.py` with a sessions section: narrative (derived partition column, skew inversion, write-size control via chunking — spec vocabulary, no plan vocabulary) + a `smelt-generate: explain silver.sessions --show-sql --json --period …` block; regenerate `docs-site/docs/examples/web-analytics-maintenance.md`. Walk the reader through the day-46 shape: event at `00:03` extending a session rooted `23:47` the previous day, and the emitted DELETE range that now covers the prior day.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/generate_tutorial.py`
- `docs-site/docs/examples/web-analytics-maintenance.md` (generated)
- `crates/smelt-cli/tests/tutorial_freshness.rs`

**Docs touched.**
- The tutorial page itself (generated; timeless narrative).

**Review checklist** (material findings only):
- [ ] Embedded SQL is real emitted output, freshness-gated
- [ ] Narrative uses spec vocabulary (derived output window, skew inversion); no phase/plan vocabulary
- [ ] `mkdocs` nav unchanged (section within the existing page)

**Commit.** `docs(examples): tutorial section — cross-midnight prior-day rewrite via the derived output window`

---

### Phase 6: Two-boundary truncation — the declared relation is a semantic cap

**Goal.** A session whose events chain across **two** date boundaries is truncated at the declared ±1-day Form B bound — identically in a day-by-day replay and a full rebuild — and the tutorial explains why: the declared relation is part of the model's own SQL (a semantic cap, not a heuristic), so truncation is never an incremental artifact; widening the declaration is the remedy and widens the derived output window with it. Spec anchor: `model_transforms.md` §Semantics "The output window is derived, never assumed" (the "semantic cap" paragraph).

**Pre-conditions.** Phase 5 merged (extends the tutorial section it creates).

**TDD tests to write first.**
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::two_boundary_session_truncated_at_declared_bound` — fixture with an event chain spanning two midnights, every gap < 30 min (e.g. day 1 23:50 → day 2 00:10 → … → day 2 23:55 → day 3 00:15): replay day-by-day as single-day windows; assert the session rooted on day 1 is truncated at the declared bound (its `session_end`/`event_count` exclude the events outside `[root − 1 day, root + 1 day]`), pin what happens to the excess events (the test documents the model's actual behaviour for them — e.g. they root a new session), and assert the day-by-day result set-equals a from-scratch full-window build of the same source data (truncation identical in both shapes).
- `crates/smelt-cli/tests/tutorial_freshness.rs` — gate entry for the new tutorial block(s), red before the section exists.

**Implementation shape.** Test + docs only; no runtime or model-SQL semantics changes expected. If the equivalence assertion fails (day-by-day ≠ full build for the two-boundary shape), STOP — that is a real bug, pause and report rather than adjusting the test. Tutorial: extend `examples/web_analytics/generate_tutorial.py` with a follow-on subsection after the cross-midnight rewrite — "What about a session that spans two midnights?" — narrating the truncation with real emitted/queried output (`smelt-generate` block), and stating the cap/widen trade-off in spec vocabulary. Regenerate `docs-site/docs/examples/web-analytics-maintenance.md`. If the web_analytics fixture data cannot exhibit a two-boundary chain deterministically, demonstrate with the e2e fixture's numbers in prose instead of a generated block, and say so in the phase notes.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs`
- `crates/smelt-cli/tests/tutorial_freshness.rs`
- `examples/web_analytics/generate_tutorial.py`
- `docs-site/docs/examples/web-analytics-maintenance.md` (generated)

**Docs touched.**
- The tutorial page subsection (generated; timeless narrative).

**Review checklist** (material findings only):
- [ ] Truncation asserted at the exact declared bound, and day-by-day ≡ full-build equivalence asserted for the two-boundary shape
- [ ] Excess-event behaviour pinned explicitly, not left implicit
- [ ] Tutorial subsection freshness-gated (or prose-only fallback recorded in phase notes); spec vocabulary; no plan/phase vocabulary
- [ ] No model SQL or runtime changes

**Commit.** `docs(examples): two-boundary session truncation — the declared relation is a semantic cap`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **(Phase 2) Skew-anchor matching is name-only** — a table-qualified Form B anchor on a foreign table (`b.d`) matches a model partition column named `d`, deriving a spurious over-wide (correctness-safe, never under-wide) output window. Documented in `docs/specs/model_transforms.md` §Known Divergences; a precise fix needs the anchor proven to be the model's own output column. Evidence: `crates/smelt-cli/tests/since_upstream.rs` fixture rename.
- **(Phase 4) `SqlCompiler::apply_type_casts` is silently inert on every clamped incremental statement** — the output clamp always makes the outermost query a bare `SELECT *`, which `apply_type_casts` (`crates/smelt-runtime/src/compile.rs`) never wraps, so the static `CAST(col AS T)` machinery never applies to a real incremental run's executed statement. Pre-existing, confirmed independent of this plan (live run and explain now agree). Needs its own investigation/plan.

## Verification

How to confirm the spec is satisfied at the end:
- `python3 examples/web_analytics/verify_incremental_equivalence.py` (default 60-day scale) passes with zero local-column divergence — the day-46 `event_id=7647` case closes.
- `cargo test -p smelt-cli --test per_partition_equivalence` green with the strengthened `(session_id, session_start, session_end, event_count, utm_campaign)` assertion.
- `cargo test -p smelt-runtime --test statement_parity` green (executed ≡ emitted, including the skew-inverted windows).
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate model_transforms` and `/smelt:validate batched_models` report zero drift on the output-window sections.
