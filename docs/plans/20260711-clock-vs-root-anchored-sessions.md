# Plan: Clock-anchored vs root-anchored session tables in the web_analytics example

**Date**: 2026-07-11
**Spec**: [`docs/research/20260711-clock-vs-root-anchored-sessions.md`](../research/20260711-clock-vs-root-anchored-sessions.md) (design doc — this is example/docs work; the one framework behaviour change lands in [`docs/specs/batched_models.md`](../specs/batched_models.md) §"Window-independence at the partition grain", Phase 1)
**Spec diff**: new design doc (committed `004200c0`)
**Tracking PR / branch**: `worktree-incremental`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the design doc at `docs/research/20260711-clock-vs-root-anchored-sessions.md` — it is the correctness oracle for the two session rules. For Phase 1, `docs/specs/batched_models.md` is the oracle. Do not re-open settled design decisions (the cut rules' closed forms are proven there).
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a design/spec rule.
- A design assumption turns out to be wrong (update the research doc / spec first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (maintenance-plan purity, property-composition-walk rule, fail-loud discipline).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/*.md` and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

The current `silver.sessions` enforces its length cap in `sessionize`'s window frames with a `COALESCE` fallback; under a never-idle device it degenerates into single-event-session confetti (design doc §Problem). The design replaces it with two tables that differ only in where the cap's *phase* comes from — the clock (window-independent, parallel) vs the session's own root (self-referential, ordered) — and enriches both back onto events. One framework gap blocks the chained table: `compute_incremental_windows_ordered` disables Form B output-window derivation for `Ordered` models (`crates/smelt-runtime/src/windowing.rs:386`), but `sessions_chained` needs both (self-read for root state; Form B rebase so a day-D run rewrites partitions D−2..D).

## Scope

### In scope (design coverage)
- Design §"`silver.sessions` — clock-anchored cut": rewritten `sessionize`, same Form B relation, model + `.test.sql` + e2e updates.
- Design §"`silver.sessions_chained` — root-anchored cut": new self-referential model, ordered execution, never-idle fixture.
- Design §"Enrichment": `events_enriched` carries both id/campaign pairs; gold models unchanged.
- Design §"The lesson": `generate_tutorial.py` + docs-site rewrite, anti-example narration, freshness-gate pin updates.
- Framework: ordered × derived-output-window composition (`batched_models.md` spec edit + `windowing.rs`).

### Explicitly deferred
- Automatic downstream cascade for ordered self-ref backfills (documented gap in `g_08_running_total_self_ref.rs`) — pre-existing, orthogonal.
- Any keyed-grain session variant (design §Rejected alternatives).
- `smelt.yml`/datagen changes to ship a never-idle device in the default seed — the fixture is test-injected only, keeping the tutorial's seeded numbers stable except where the rule change moves them.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | (see date) | 2026-07-11 |
| 2     | done     | (see date) | 2026-07-12 |
| 3a    | done     | (see date) | 2026-07-12 |
| 3     | done     | (see date) | 2026-07-12 |
| 4     | done     | (see date) | 2026-07-12 |
| 5     | done     | (see date) | 2026-07-12 |

---

### Phase 1: Ordered execution composes with the derived output window

**Goal.** An `Ordered` (convergent self-edge) model whose partition column skews earlier than its event date gets the same Form B write-window rebase as a window-independent model — sequential single-partition batches over the *rebased* range.

**Pre-conditions.** None (framework-first; the example consumer arrives in Phase 3).

**TDD tests to write first.**
- `crates/smelt-cli/tests/property_discovery/g_12_self_ref_derived_output_window.rs::ordered_self_ref_run_rewrites_skewed_prior_partitions` — stages a minimal self-ref model with a derived partition column (`session_start_date`-style, Form B relation declaring 1-day skew) plus a self-read of the prior partition; seeds 3 days; runs day 3 alone; asserts partitions D−1 and D are rewritten (rebased window), batches executed strictly in temporal order, and results equal a from-scratch build.
- `crates/smelt-runtime/tests/windowing_ordered.rs::ordered_verdict_still_ignores_self_edge_as_skew_anchor` — the false-positive guard the current `!Ordered` gate protects: a self-ref model whose *only* bounding relation is the self-edge's own column derives **no** output-window skew (the self-reference is not a skew anchor), while a genuine source-anchored Form B relation on the same model does.

**Implementation shape.** In `windowing.rs`, replace the blanket `apply_output_window_derivation = !Ordered` with anchor-aware derivation: run output-window derivation for `Ordered` models but exclude the self-referenced table's relations from skew-anchor candidates (the walk already knows which ref is the self-edge). Sequential batch enumeration then runs over the rebased range. Spec edit: `batched_models.md` §"Window-independence at the partition grain" gains the composition statement (an ordered model's write window is rebased by the same derived-output-window rule; ordering applies to the rebased partitions) — timeless wording.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/windowing.rs` — anchor-aware derivation for `Ordered`
- `crates/smelt-logical/src/analysis/walk.rs` (and the `model_partition_skew` entry point it exposes) — the self-source exclusion lives in the skew walk as a structural, per-scope exclusion (property-composition-walk rule: composition-relevant bounds are produced by the walk, never by a downstream text scan)
- `crates/smelt-logical/tests/` — unit coverage for the exclusion (alias scoping, UNION branches, comma joins)
- `crates/smelt-cli/tests/property_discovery/g_13_self_ref_derived_output_window.rs` — new (g_12 was already taken; plan updated to match)
- `crates/smelt-runtime/tests/windowing_ordered.rs` — new/extended unit coverage
- `docs/specs/batched_models.md` — composition statement

**Docs touched.**
- `docs/specs/batched_models.md` — §"Window-independence at the partition grain": ordered execution composes with the derived output window; the self-edge is never a skew anchor.
- `docs-site/docs/guide/incremental-models.md` — one paragraph in the ordered-execution section stating the same, as a feature description.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Self-edge excluded as skew anchor (guard test green), genuine anchors admitted
- [ ] Ordered batches enumerate over the rebased range in strict temporal order
- [ ] Spec + docs-site edits are timeless
- [ ] No scope creep into example phases

**Commit.** `feat(runtime): ordered self-ref execution composes with the derived output window`

---

### Phase 2: Clock-anchored sessionize replaces the frame-cap rule

**Goal.** `silver.sessions` implements the clock-anchored cut (cut at day D's end iff the session has an event in `[D 00:00, D 00:30)`), eliminating the confetti degeneration while staying window-independent with the same Form B relation.

**Pre-conditions.** None (independent of Phase 1).

**TDD tests to write first.**
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::never_idle_device_yields_one_session_per_day` — new: seeds an 8-day 29-minute-gap chain rooted at 14:00; asserts session count = 9 (one ~34h session, then one per day), **zero** single-event sessions, every event counted exactly once, and day-by-day replay ≡ full rebuild.
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::two_boundary_session_truncated_at_declared_bound` — updated pins: the 60-event chain rooted `23:50` now yields a 59-event session (`session_end = <chain's 23:55 day-2 event>`) + 1 forced-root single on day 3; conservation 59+1=60; replay ≡ rebuild.
- `examples/web_analytics/tests/session_boundary_invariants.test.sql` — updated: existing four device fixtures keep their expected outcomes (none trips the new cut); add device 5: events at `00:10` and `23:50 + next-day 00:05`-style fixtures pinning (a) an early-root session cut at its day's end, (b) a late-root session crossing one midnight and cut at the second.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs::web_analytics_session_attribution_matches_full_rebuild` — updated pins for the new rule.

**Implementation shape.** Rewrite `examples/web_analytics/functions/sessionize.sql` around the closed form: natural boundaries (gap > 30 min / platform change / no predecessor) as today; per event, candidate root = most recent natural boundary in a trailing 2-day frame; deadline = `end_of_day(date(root))` when `time(root) < 00:30` else `end_of_day(date(root) + 1 day)`; events past the deadline re-root at the first chain event of their own day (forced roots always land before 00:30, so the cascade is one cut per day — derivable within the 2-day frame; design doc §clock-anchored, closed form). `models/silver/sessions.sql`: Form B relation stays `event_date BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`; `HAVING` cap asserts `session_end < deadline` in checkable form. Comments rewritten to describe the clock rule.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/functions/sessionize.sql` — the new rule
- `examples/web_analytics/models/silver/sessions.sql` — relation/cap/comments
- `examples/web_analytics/tests/session_boundary_invariants.test.sql` — updated + new fixtures
- `crates/smelt-cli/tests/e2e/{cross_midnight_rebase,per_partition_equivalence}.rs` — new/updated pins

**Docs touched.**
- None yet — the generated tutorial page is rewritten wholesale in Phase 5; `tutorial_freshness` pin updates land there. If `tutorial_freshness` fails in the interim, mark the affected assertion `#[ignore]` with a `// re-enabled in the docs phase` note *only if* the suite must stay green mid-plan; prefer landing Phases 2→5 in quick succession.

**Review checklist** (material findings only):
- [ ] Never-idle fixture: no single-event confetti, deterministic 1/day cadence
- [ ] Crossing session always dies at the second midnight (fixture pins it)
- [ ] Replay ≡ rebuild for every fixture (window-independence preserved)
- [ ] Form B relation still sound: every emitted row satisfies it
- [ ] No scope creep into `sessions_chained`

**Commit.** `feat(examples): clock-anchored session cut replaces the frame-cap rule in web_analytics`

---

### Phase 3a: Self-referential first-run bootstrap (framework)

*(Inserted during implementation, 2026-07-12, with user sign-off. Phase 3 surfaced that a self-referential model cannot be built from scratch — `CREATE TABLE ... AS SELECT` cannot resolve the self-reference when the target table does not yet exist. This was a documented Known Divergence in `docs/specs/batched_models.md`; the example (and Phase 5's tutorial) needs a clean `smelt run` from scratch, so the gap is closed here rather than worked around with seeded fixtures.)*

**Goal.** A self-referential partition-grain model builds from scratch with no manual seed: when the target table does not exist, the runtime first materialises an empty table with the model's output schema, then executes the run's batches as normal inserts (the self-read over the empty table correctly yields no prior state for the first partition).

**Pre-conditions.** Phase 1 (ordered × Form B). Phase 3's uncommitted example work may sit in the tree but this phase must not touch it.

**TDD tests to write first.**
- Extend `crates/smelt-cli/tests/property_discovery/g_08_running_total_self_ref.rs` (or a sibling test in the same file/family): a self-referential model with **no** pre-seeded target table, built from scratch over N partitions, equals the expected sequential result; second run is idempotent.
- A statement-parity leg if the bootstrap DDL is a new executed-statement family (`cargo test -p smelt-runtime --test statement_parity` must stay green and cover it).

**Implementation shape.** Spec-first: rewrite the Known Divergence bullet in `docs/specs/batched_models.md` ("A self-referential model's very first partition cannot be created via `CREATE TABLE ... AS SELECT ...`") into normative Semantics: first-run bootstrap creates the empty target from the model's inferred output schema, then batches insert. Honour the **maintenance-plan purity** invariant: the bootstrap DDL is authored by a pure emitter in `smelt-logical`'s maintenance layer (taking the schema as data); the backend only executes it. Wire into the first-run path in `smelt-runtime`/`smelt-backend` where `CREATE TABLE AS SELECT` is chosen today, gated on the model being self-referential (non-self-ref models keep the existing CTAS path).

**Critical files (allowed to touch in this phase).**
- `docs/specs/batched_models.md` — Known Divergence → normative first-run bootstrap semantics (timeless)
- `crates/smelt-logical/src/maintenance/` — pure bootstrap-DDL emitter
- `crates/smelt-runtime/` and `crates/smelt-backend*/` — first-run dispatch
- `crates/smelt-cli/tests/property_discovery/g_08_running_total_self_ref.rs` (or sibling) — from-scratch bootstrap test
- `crates/smelt-runtime/tests/statement_parity.rs` — parity leg if a new statement family is added
- `docs-site/docs/guide/incremental-models.md` — remove/adjust any "must pre-seed" caveat if one exists

**Review checklist** (material findings only):
- [ ] From-scratch self-ref build works with no seed; result equals sequential expectation; idempotent
- [ ] Bootstrap DDL emitted by a pure `smelt-logical` emitter (backends execute, never author); statement_parity green
- [ ] Non-self-referential models keep the existing CTAS first-run path (no behaviour change)
- [ ] Spec edit is timeless and the Known Divergence bullet is gone
- [ ] Phase 3's uncommitted example files untouched

**Commit.** `feat(runtime): self-referential models bootstrap an empty target on first run`

---

### Phase 3: Root-anchored `silver.sessions_chained` (self-referential, ordered)

**Goal.** New one-row-per-session model where a day-D event continues an open session only if that session rooted < 2 days ago; the model reads its own prior partitions and the planner proves `Ordered`.

**Pre-conditions.** Phase 1 (ordered × Form B) merged.

**TDD tests to write first.**
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::chained_never_idle_device_yields_one_session_per_two_days` — same 8-day chain; asserts ~1 session per 2 days phase-locked to the 14:00 root, every event counted once, and **in-order** day-by-day replay ≡ full sequential build.
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::chained_run_is_refused_or_ordered_never_parallel` — asserts the planner verdict for the model is `Ordered` (not `WindowIndependent`, not `Refused`) and a multi-day run executes as sequential single-partition batches over the rebased range.
- `examples/web_analytics/tests/session_boundary_chained_invariants.test.sql` — new: mirrors the four base fixtures (same outcomes — gap/platform rules identical) plus a fixture where a session rooted 2 days earlier is force-cut mid-chain while the clock table would have cut it at a midnight — pinning the divergence.

**Implementation shape.** `examples/web_analytics/models/silver/sessions_chained.sql`: partition grain on `session_start_date`; inline SQL (no function — self-reference + state inheritance); reads `smelt.silver.sessions_chained` with a bounding relation reaching back 2 days (open sessions: root within 2 days, `session_end` within 30 min of the read window's start-of-day), source events with the Form B relation `event_date BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`; same 5-minute first-touch attribution as `sessions`. Frontmatter mirrors `sessions.sql`.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/sessions_chained.sql` — new
- `examples/web_analytics/tests/session_boundary_chained_invariants.test.sql` — new
- `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs` — chained fixtures
- `crates/smelt-datagen/tests/example_web_analytics.rs` — gate running the new `.test.sql`

**Docs touched.**
- None yet (Phase 5 rewrites the tutorial page around both tables).

**Review checklist** (material findings only):
- [ ] Planner verdict is `Ordered` via the real proof (no override/declaration)
- [ ] Self-read is backward-only, 2 days — matches design §root-anchored
- [ ] Out-of-order replay of the chained table is impossible by construction (runtime sequences it), and the test demonstrates in-order replay ≡ rebuild
- [ ] Divergence fixture pins where the two rules disagree
- [ ] Gap/platform semantics identical to `sessions` (shared fixtures agree)

**Commit.** `feat(examples): root-anchored self-referential sessions_chained model in web_analytics`

---

### Phase 4: Enrichment carries both session identities

**Goal.** `silver.events_enriched` joins both tables: `session_id`/`utm_campaign` (primary, from `sessions`) plus `session_id_chained`/`utm_campaign_chained`; gold models unchanged.

**Pre-conditions.** Phases 2 and 3.

**TDD tests to write first.**
- `examples/web_analytics/tests/enrichment_dual_session_invariants.test.sql` — new: mocks both session tables + events; asserts every event carries both ids, ids agree on a fixture where neither cap fires, and diverge only on the cap-divergence fixture from Phase 3.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs::events_enriched_dual_ids_replay_matches_rebuild` — full-pipeline assertion that per-partition maintenance of the enriched table (now downstream of an ordered model) matches full rebuild.

**Implementation shape.** Add the second join in `events_enriched.sql` (same bounded join-window pattern as the existing one, cap now 2 days to cover the chained table's reach); update the model's comments describing the two upstream clocks. Gold identity models keep consuming `session_id` — verify no diagnostics drift (`example_diagnostics`).

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/events_enriched.sql` — second join + columns
- `examples/web_analytics/tests/enrichment_dual_session_invariants.test.sql` — new
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` — dual-id assertion
- `crates/smelt-datagen/tests/example_web_analytics.rs` — gate for the new test

**Docs touched.**
- None yet (Phase 5).

**Review checklist** (material findings only):
- [ ] Both id/campaign pairs present; gold models untouched and diagnostic-clean
- [ ] Enriched table's derived read bounds compose with the chained upstream (2-day cap)
- [ ] Divergence is queryable and pinned by test

**Commit.** `feat(examples): events_enriched carries clock- and root-anchored session identities`

---

### Phase 5: Docs — the two-table teaching arc

**Goal.** The docs-site walkthrough teaches the lesson: any bounded sessionizer must cut; the cut's *phase source* (clock vs root) decides parallel vs ordered execution; the old frame-cap design is a narrated anti-example.

**Pre-conditions.** Phases 1–4 (generated blocks must reflect final SQL).

**TDD tests to write first.**
- `crates/smelt-cli/tests/tutorial_freshness.rs::web_analytics_maintenance_tutorial_sql_is_fresh` — updated pins: new prose-value assertions for the never-idle comparison (chained ≈ 1/2 days, clock ≈ 1/day) and the updated two-boundary values (59 + 1); require at least one `explain` block for **each** session table; drop the old `event_count=58` pins.
- Doc-build gate: `cd docs-site && uv run mkdocs build` succeeds (broken-link check for the new cross-references).

**Implementation shape.** Rewrite the sessions/enrichment sections of `examples/web_analytics/generate_tutorial.py`: the arc (naive unbounded rule → must cut → root-anchored/ordered vs clock-anchored/parallel → the frame-cap anti-example with the confetti numbers, narrated only), an `explain --show-sql` block per table, the never-idle comparison table from the design doc, and the dual-id enrichment section. Regenerate `docs-site/docs/examples/web-analytics-maintenance.md`. Cross-link `docs-site/docs/guide/incremental-models.md` §ordered execution (paragraph landed in Phase 1). Update `examples/web_analytics/README.md` model list.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/generate_tutorial.py` — narrative + blocks
- `docs-site/docs/examples/web-analytics-maintenance.md` — regenerated
- `docs-site/docs/guide/incremental-models.md` — cross-link
- `crates/smelt-cli/tests/tutorial_freshness.rs` — updated pins
- `examples/web_analytics/README.md` — model list

**Docs touched.** (This phase *is* the docs phase — all timeless: the page describes the two tables as the example's design, the anti-example as a cautionary pattern, never as "what this example used to do last plan".)

**Review checklist** (material findings only):
- [ ] Freshness gate green against regenerated blocks (byte-equal)
- [ ] Comparison table's numbers come from generated/tested output, not hand-arithmetic
- [ ] Anti-example narrated without shipping a broken model
- [ ] Timeless wording throughout; no plan/phase vocabulary
- [ ] mkdocs build clean

**Commit.** `docs(examples): two-table sessionization teaching arc — clock-anchored vs root-anchored cuts`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the design is satisfied at the end:
- `cargo test -p smelt-cli --test e2e -- cross_midnight` and `-- per_partition_equivalence` — all session fixtures green, including never-idle and in-order chained replay.
- `cargo test -p smelt-datagen --test example_web_analytics` — all `.test.sql` gates pass via real `smelt test`.
- `cargo test -p smelt-cli --test tutorial_freshness` — generated page byte-fresh.
- `bash .claude/scripts/verify-phase.sh`
- Manual: `python3 examples/web_analytics/generate_tutorial.py` produces no diff.
