# Plan: Web-Analytics Maintenance Demo (partition-grain reframe)

**Date**: 2026-07-10
**Spec**: [`docs/specs/cli.md`](../specs/cli.md), [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md), [`docs/specs/datagen.md`](../specs/datagen.md) — semantic oracles for the example models: [`docs/specs/timeseries.md`](../specs/timeseries.md), [`docs/specs/batched_models.md`](../specs/batched_models.md)
**Spec diff**: uncommitted working tree (2026-07-10): `cli.md` §"`--dry-run` prints the maintenance statements"; `maintenance_plan.md` §"Upstream model edges"; `datagen.md` `timestamp_offset` generator + §"Redelivery (duplicate emission)"
**Tracking PR / branch**: `worktree-incremental` (PR TBD)
**Docs**: code+docs

> Supersedes the DRAFT version of this file (brainstorming output + MP-series review findings;
> see git history of this path). Decisions carried forward: extend `examples/web_analytics/` in
> place; demo ships on the **partition-grain reframe** (dedup via `QUALIFY ROW_NUMBER()` under
> `grain: partition`), not keyed temporal locality; tutorial is a **generated** page whose SQL is
> the real emitter output.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the specs listed in the header — they are the correctness oracle. Do not re-open settled spec decisions.
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
- Honor architectural invariants from `CLAUDE.md` — in particular **maintenance-plan purity / statement single-owner** (`cargo test -p smelt-runtime --test statement_parity` stays green; dry-run rendering must consume the same emitters, never author SQL) and **run pipeline parity** (`execute_parity`).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to specs and `docs-site/docs/...` describe the feature as if it has always existed. If a phase ships an incomplete surface, the *spec* records the gap under **Known Divergences** in behavioural terms.

---

## Context

The maintenance-plan work (MP series + emit unification + `smelt explain --show-sql`) shipped the machinery; nothing in the repo yet *demonstrates* it end-to-end on a realistic pipeline, and two adjacent CLI/derivation gaps are recorded as Known Divergences (`cli.md`: dry-run prints SELECT only; model upstreams drop out of trigger derivation). This plan closes those two gaps and extends `examples/web_analytics/` with a lateness/dedup/attribution pipeline, culminating in a generated tutorial whose embedded SQL is the real emitter output — so the doc cannot drift.

## Scope

### In scope (spec coverage)
- `cli.md` §"`--dry-run` prints the maintenance statements": `smelt run`/`smelt backbuild --dry-run` render emitted maintenance statements, real window literals, per-chunk boundaries on backbuild.
- `maintenance_plan.md` §"Upstream model edges": creation-trigger cells for maintained-model upstreams, refusal (never silence) when the clock is underivable, `--source <model-address>` for forward propagation.
- `datagen.md` §Generator types `timestamp_offset` + §"Redelivery (duplicate emission)".
- `examples/web_analytics/` extension: `arrival_time` ingestion clock, event-id dedup over a 3-day late window (`grain: partition` on `event_date`), first-5-minutes `utm_campaign` session attribution with an explicit max-session-length cap, `events_enriched` at event grain.
- Generated tutorial page under `docs-site/docs/examples/` + drift gate.

### Explicitly deferred
- **Keyed temporal locality** (`keyed_models.md` §Known Divergences) — its own spec-first plan; this demo becomes its showcase once it lands.
- **Automatic watermark-diffed `--since-upstream`** (`maintenance_plan.md` §Future Extensions) — deltas stay explicit via `--landed`.
- **`smelt bakeoff`** (MP13, deliberate; ROADMAP §10).
- **Ephemeral-aggregate `BIGINT` cast fix** (`cli.md` §Known Divergences) — pre-existing shared-compiler ordering issue, unaffected by this plan.
- **Keyed multi-aggregate plan-derivation widening** (second half of the `cli.md` NewData divergence) — untouched here; only the model-upstream half is closed.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | d245da69 | 2026-07-10 |
| 2     | done     | d98af84b | 2026-07-10 |
| 3     | done     | d6effc7a | 2026-07-10 |
| 4     | done     | 12ea3508 | 2026-07-11 |
| 5     | done     |        | 2026-07-11 |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |

---

### Phase 1: `--dry-run` renders the emitted maintenance statements (run + backbuild, chunked)

**Goal.** `smelt run --dry-run` and `smelt backbuild --dry-run` print the single-owner emitters' statements with real window literals; backbuild additionally prints per-chunk boundary lines — per `cli.md` §"`--dry-run` prints the maintenance statements".

**Pre-conditions.** Emit unification landed (statements have a single owner in `smelt-logical`); `statement_parity` green at HEAD.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/dry_run_statements.rs::dry_run_reports_emitted_statements` — `execute_project` with `dry_run: true` on a partition-grain fixture reports, per model, the identical statement list the maintenance emitters produce for that window; literals are real (no `{{window_start}}` placeholders); asserts equality against the emitters' output, not a golden string.
- `crates/smelt-runtime/tests/dry_run_statements.rs::dry_run_executes_nothing` — after a dry-run over `examples/web_analytics/`, no target table exists/changes.
- `crates/smelt-cli/tests/backbuild_dry_run.rs::chunked_range_prints_per_chunk_boundaries` — a multi-day range on a model whose batch-safety classification forces chunking prints one statement block per chunk, each introduced by `-- chunk k/N: [start, end)`, in real execution order (real fixture: `examples/web_analytics/`).

**Implementation shape.** In `smelt-runtime`'s `execute_project` dry-run branch: instead of returning after `model_compiled`, run the same plan/emit path a real run takes up to (but not including) backend execution, and hand the emitted statements to the reporter (new reporter method, e.g. `maintenance_statements(model, chunk_info, &[Statement])`). Backbuild's chunk loop must be reached under dry-run so chunk windows are real. CLI reporter renders blocks + `BEGIN`/`COMMIT` brackets, matching `explain --show-sql`'s rendering (share the renderer in `smelt-cli/src/explain.rs` if practical). No SQL is authored anywhere in `smelt-runtime`/`smelt-cli` — statements come from the emitters only.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` — dry-run branch reaches statement emission/chunking
- `crates/smelt-runtime/src/reporter.rs` (or the reporter trait's home) — statement-reporting hook
- `crates/smelt-cli/src/reporter.rs`, `crates/smelt-cli/src/explain.rs` — rendering reuse
- `crates/smelt-cli/src/commands/backbuild.rs` — dry-run path reaches the chunk planner
- `docs/specs/cli.md` — remove the "`--dry-run` prints only the compiled SELECT body today" Known Divergence

**Docs touched.**
- `docs-site/docs/reference/cli.md` — `--dry-run` description under run/backbuild: emitted statements, real literals, chunk boundaries; division of labour vs `explain --show-sql`

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] `cli.md` §"`--dry-run` prints the maintenance statements" satisfied; statement single-owner invariant intact (`statement_parity` green, no authored SQL)
- [ ] Run pipeline parity: rendering lives behind `execute_project`'s reporter, not a CLI-side re-compile
- [ ] No scope creep into later phases
- [ ] User docs updated; spec + docs-site edits are timeless

**Commit.** `feat(cli+runtime): --dry-run renders emitted maintenance statements; backbuild prints per-chunk boundaries`

---

### Phase 2: Upstream-model trigger derivation

**Goal.** A maintained model's ref to another maintained model derives a creation-trigger cell clocked by the upstream's `timeseries:` declaration; an underivable clock is a recorded `MaintenanceReachNotDerivable` refusal — per `maintenance_plan.md` §"Upstream model edges".

**Pre-conditions.** None beyond HEAD (independent of Phase 1).

**TDD tests to write first.**
- `crates/smelt-db/tests/maintenance_model_upstream.rs::model_upstream_derives_creation_cell` — two-model chain (upstream `grain: partition` with `timeseries:`; downstream refs it): downstream's plan contains a creation cell for the model edge whose scan clamp uses the upstream's clock column + granularity.
- `crates/smelt-db/tests/maintenance_model_upstream.rs::model_upstream_without_clock_records_refusal` — upstream with no `timeseries:` → `MaintenanceReachNotDerivable` naming the edge; the cell is refused, not silently absent.
- `crates/smelt-db/tests/maintenance_model_upstream.rs::view_upstream_derives_no_creation_cell` — a view/`full` upstream contributes no creation cell and no refusal.
- Real fixture: `crates/smelt-cli/tests/explain_model.rs` (extend) — `smelt explain gold.eventstream_with_identity` in `examples/web_analytics/` shows a creation cell for its silver model upstream.

**Implementation shape.** `crates/smelt-db/src/queries/maintenance.rs`: when assembling the derivation inputs, resolve `smelt.<path>` refs against the project's maintained models as well as `sources.*`; a model upstream contributes an edge descriptor carrying the upstream's clock (from its own validated metadata). `crates/smelt-logical/src/maintenance/derive.rs`: accept model edges in trigger-list construction; refusal path for missing clock. Pure-function discipline: Salsa query stays a thin assembler (Salsa purity rule).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/maintenance.rs` — ref resolution over models
- `crates/smelt-logical/src/maintenance/derive.rs` (+ neighbours) — model-edge cells, refusal
- `docs/specs/cli.md`, `docs/specs/maintenance_plan.md` — trim the model-upstream Known Divergences to what remains (the `--source <model>` half until Phase 3)

**Docs touched.**
- `docs-site/docs/guide/incremental-models.md` — model-to-model chains derive creation cells; what `smelt explain` shows for them

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] `maintenance_plan.md` §"Upstream model edges" rules satisfied (clock from upstream `timeseries:`, refusal never silence, view/full upstreams excluded)
- [ ] Salsa purity + maintenance-plan purity invariants honored; `walk_coverage` green
- [ ] No scope creep into Phase 3 (CLI `--source` untouched)
- [ ] User docs updated; spec + docs-site edits are timeless

**Commit.** `feat(maintenance): derive creation-trigger cells for maintained-model upstreams; refuse underivable clocks`

---

### Phase 3: `--since-upstream --source <model-address>`

**Goal.** Forward propagation accepts an upstream maintained model as the delta origin: `smelt run --since-upstream --source <model> --landed <a>..<b>` propagates through the graph and runs exactly the affected downstream `(model, region)` pairs — per `maintenance_plan.md` §"Upstream model edges" (final paragraph).

**Pre-conditions.** Phase 2 (model edges exist in the derivation the propagation graph is built from).

**TDD tests to write first.**
- `crates/smelt-cli/tests/since_upstream.rs::model_address_landed_delta_propagates` (extend existing since-upstream suite if present) — in `examples/web_analytics/`, declaring a landed window on a silver model dirties only its downstreams; the printed dirty set and the executed pairs match.
- `crates/smelt-cli/tests/since_upstream.rs::model_address_unknown_is_error` — an address that is neither a declared source nor a maintained model exits non-zero with a diagnostic naming it.

**Implementation shape.** CLI arg resolution for `--source` goes through the same `resolve_ref_path` resolver as model SQL (cli.md invariant 11), accepting source or model addresses; `smelt-runtime/src/propagation.rs` seeds the walk from a model node the same way it seeds from a source node.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/main.rs` / `crates/smelt-cli/src/commands/run.rs` — `--source` resolution
- `crates/smelt-runtime/src/propagation.rs` — model-node delta seeding
- `docs/specs/maintenance_plan.md` — remove the remaining `--source <model-address>` clause from the Known Divergence

**Docs touched.**
- `docs-site/docs/reference/cli.md` — forward-propagation section: `--source` accepts model addresses; example

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Resolution uses the canonical resolver (no parallel leaf-only resolver — cli.md invariant 11)
- [ ] Dirty-set printout unchanged in shape; exit codes meaningful
- [ ] No scope creep into example phases
- [ ] User docs updated; spec + docs-site edits are timeless

**Commit.** `feat(cli): --since-upstream --source accepts maintained-model addresses as delta origins`

---

### Phase 4: Datagen lateness + redelivery + campaign columns

**Goal.** Implement `timestamp_offset` and `redelivery:` per `datagen.md`; extend `examples/web_analytics/datagen.yaml` with `event_time`/`arrival_time` (0–3-day lateness), ~2% redelivered duplicate `event_id`s, and a nullable `utm_campaign` payload field.

**Pre-conditions.** None (independent of Phases 1–3).

**TDD tests to write first.**
- `crates/smelt-datagen/tests/timestamp_offset.rs::offset_adds_seconds_to_base_column` — output equals base + drawn offset; ISO 8601.
- `crates/smelt-datagen/tests/timestamp_offset.rs::base_must_be_earlier_timestamp_column` — later or non-timestamp base is a config error.
- `crates/smelt-datagen/tests/redelivery.rs::duplicates_identical_except_arrival_column` — redelivered rows byte-equal to originals except the shifted arrival column; count = `round(fraction × num_rows)`.
- `crates/smelt-datagen/tests/redelivery.rs::redelivery_does_not_perturb_primary_rows` — toggling `redelivery:` leaves primary rows byte-identical (dedicated RNG stream, seed `+200`).
- `crates/smelt-datagen/tests/example_web_analytics.rs::web_analytics_has_lateness_duplicates_and_campaigns` (extend) — generated dataset: `arrival_time >= event_time` everywhere; some rows ≥1 day late, none >3 days; duplicate `event_id`s exist; `utm_campaign` non-null for a strict subset of rows.

**Implementation shape.** New generator variant + config parsing in `smelt-datagen` (mirroring `optional`'s inner-generator delegation for `offset_seconds`/`delay_seconds`); redelivery as a post-pass over the generated batch before Parquet write, drawing from the `+200` stream. `datagen.yaml`: add `event_time` (`timestamp`), `arrival_time` (`timestamp_offset` off `event_time`, weighted lateness mostly 0, tail to 3 days), `utm_campaign` (`optional` + `one_of`) inside the payload or as a column, `redelivery:` block on the events dataset.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-datagen/src/` — generator + config + redelivery pass
- `examples/web_analytics/datagen.yaml` — new columns + redelivery (models untouched; extra parquet columns are inert until Phase 5)
- `docs/specs/datagen.md` — remove the "`timestamp_offset` and `redelivery:` are not yet implemented" Known Divergence

**Docs touched.**
- `docs-site/docs/reference/` datagen page — new generator row + redelivery section

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] `datagen.md` determinism semantics honored (isolated RNG stream, prefix stability)
- [ ] Existing example datasets byte-identical where redelivery/offset unused
- [ ] No scope creep into model changes (Phase 5)
- [ ] User docs updated; spec + docs-site edits are timeless

**Commit.** `feat(datagen): timestamp_offset generator + redelivery block; web_analytics gains lateness, duplicates, utm_campaign`

---

### Phase 5: Silver dedup over a 3-day late window

**Goal.** `events_parsed` becomes the demo's load-bearing model: repartitions arrival-ordered input by `event_date`, dedups `event_id` via `QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1`, and accepts late events up to 3 days — the window expressed so the derived lookback / batch-safety bound is 3 days.

**Pre-conditions.** Phase 4 (data has lateness + duplicates).

**TDD tests to write first.**
- `crates/smelt-cli/tests/per_partition_equivalence.rs::web_analytics_dedup_matches_full_rebuild` (extend the existing suite) — day-by-day incremental build equals one full-window rebuild: zero duplicate `event_id`s in the result; every ≤3-day-late event present in its `event_date` partition.
- `examples/web_analytics/verify_incremental_equivalence.py` — add dedup/lateness assertion columns (duplicate count = 0; late-event presence count equal across pipelines) so the Python harness verifies the new behaviour too.

**Implementation shape.** `bronze/raw_events.sql` projects `event_time`/`arrival_time`; `silver/events_parsed.sql` gains the QUALIFY dedup and the 3-day window in its scan expression (Form-B filters per the existing example conventions), staying `grain: partition` on `event_date`. Downstream identity models keep working (dedup is upstream hygiene). `smelt explain silver.events_parsed` must show the derived 3-day clamp — that report is Phase 8's tutorial input.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/bronze/raw_events.sql`, `examples/web_analytics/models/silver/events_parsed.sql`
- `examples/web_analytics/verify_incremental_equivalence.py`, `examples/web_analytics/README.md`
- `crates/smelt-cli/tests/per_partition_equivalence.rs`

**Docs touched.**
- `examples/web_analytics/README.md` — lateness/dedup narrative (tutorial page itself is Phase 8)

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Derived lookback is 3 days (visible in `smelt explain`), not declared via YAML overrides (derive-don't-declare)
- [ ] `example_diagnostics` + equivalence harness green; identity pipeline unaffected
- [ ] No scope creep into sessions/enrichment
- [ ] Docs edits are timeless

**Commit.** `feat(examples): web_analytics silver dedup over 3-day late window (partition-grain QUALIFY reframe)`

---

### Phase 6: Session campaign attribution + explicit max-session-length cap

**Goal.** `sessions.sql` captures `utm_campaign` from an event in the first 5 minutes of the session and declares an explicit max-session-length cap (the bound that seals old partitions for safe partition-level maintenance).

**Pre-conditions.** Phase 5 (deduped events with `utm_campaign` available).

**TDD tests to write first.**
- `crates/smelt-cli/tests/per_partition_equivalence.rs::web_analytics_session_attribution_matches_full_rebuild` — incremental vs full rebuild equality on `(session_id, utm_campaign)`; attribution comes only from events within 5 minutes of session start; sessions never exceed the cap.
- `examples/web_analytics/verify_incremental_equivalence.py` — session/attribution assertion columns.

**Implementation shape.** Extend the existing sessionize showcase: campaign attribute via a bounded window (`MIN_BY`-style earliest campaign within `session_start + 5 min`, expressed with admissible constructs), cap expressed so the derived bound = cap (today's 1-day frame generalises; keep the frame-derived cap but make it an explicit, named interval in the SQL).

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/sessions.sql` (+ `examples/web_analytics/functions/` if the sessionize function needs the cap parameter)
- `examples/web_analytics/verify_incremental_equivalence.py`, `examples/web_analytics/README.md`
- `crates/smelt-cli/tests/per_partition_equivalence.rs`

**Docs touched.**
- `examples/web_analytics/README.md` — attribution + cap narrative

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Cap and lookback are derived from the SQL/function shape, not YAML declarations
- [ ] Equivalence harness green
- [ ] No scope creep into enrichment
- [ ] Docs edits are timeless

**Commit.** `feat(examples): web_analytics session utm_campaign attribution + explicit max-session-length cap`

---

### Phase 7: `events_enriched` — narrow update at event grain

**Goal.** New model joining `session_id` + `utm_campaign` back onto the event grain, demonstrating that maintenance targets only affected `event_date` partitions — and, via Phase 2, that its model upstreams derive real creation cells.

**Pre-conditions.** Phases 2, 5, 6.

**TDD tests to write first.**
- `crates/smelt-cli/tests/per_partition_equivalence.rs::web_analytics_events_enriched_matches_full_rebuild` — incremental equals full rebuild at event grain.
- `crates/smelt-cli/tests/per_partition_equivalence.rs::web_analytics_events_enriched_narrow_update` — snapshot partitions, run one additional arrival day, assert only `event_date` partitions within the derived window changed.
- `crates/smelt-cli/tests/explain_model.rs` (extend) — `smelt explain silver.events_enriched` shows creation cells for both model upstreams (`events_parsed`, `sessions`) with their derived clamps.

**Implementation shape.** `examples/web_analytics/models/silver/events_enriched.sql`: `grain: partition` on `event_date`, joins bounded by the session cap + late window so the clamp composes (per `maintenance_plan.md` §"Upstream model edges"). Wire into `verify_incremental_equivalence.py`.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/events_enriched.sql` (new)
- `examples/web_analytics/verify_incremental_equivalence.py`, `examples/web_analytics/README.md`
- `crates/smelt-cli/tests/per_partition_equivalence.rs`, `crates/smelt-cli/tests/explain_model.rs`

**Docs touched.**
- `examples/web_analytics/README.md` — enrichment narrative

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Model-upstream creation cells derived (Phase 2 surface exercised on a real fixture)
- [ ] Narrow-update assertion is on observed partitions, not implementation logs
- [ ] Equivalence harness green
- [ ] Docs edits are timeless

**Commit.** `feat(examples): web_analytics events_enriched — bounded event-grain enrichment with model-upstream creation cells`

---

### Phase 8: Generated tutorial page + drift gate

**Goal.** A generated MkDocs page `docs-site/docs/examples/web-analytics-maintenance.md` walking the pipeline, embedding the **real** emitted maintenance SQL (`smelt explain <model> --show-sql --json`) and a captured `smelt backbuild --dry-run` chunked backfill — plus a test that fails when the committed page drifts from regeneration.

**Pre-conditions.** All prior phases.

**TDD tests to write first.**
- `crates/smelt-cli/tests/tutorial_freshness.rs::web_analytics_maintenance_tutorial_sql_is_fresh` — re-derives the embedded SQL blocks (via the same in-process explain/`--show-sql` path, pinned `--period`) for each model the page names and asserts they byte-match the committed page's fenced blocks. Cheap: no datagen, no backend.
- Generator script self-check: `generate_tutorial.py --check` exits non-zero when the committed page differs from a fresh render.

**Implementation shape.** `examples/web_analytics/generate_tutorial.py`: renders the page from a template — prose sections + fenced SQL lifted from `smelt explain <model> --show-sql --json` (`statements` array; pinned `--period` so literals are stable) and a captured `smelt backbuild --dry-run` transcript for the backfill section. Output committed at `docs-site/docs/examples/web-analytics-maintenance.md`; nav entry added beside the existing identity-stitching page (which stays untouched). Follows the `docs/demos/generate-docs.ts` generate-and-copy precedent; no `pymdownx.snippets` wiring.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/generate_tutorial.py` (new), `examples/web_analytics/README.md`
- `docs-site/docs/examples/web-analytics-maintenance.md` (new, generated+committed)
- `docs-site/mkdocs.yml` — nav entry
- `crates/smelt-cli/tests/tutorial_freshness.rs` (new)

**Docs touched.**
- `docs-site/docs/examples/web-analytics-maintenance.md` — the deliverable (generated; timeless by construction — the generator template must contain no plan vocabulary)

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Embedded SQL is lifted from the machine output, never hand-pasted
- [ ] Existing identity-stitching page untouched; both pages in nav
- [ ] Freshness gate runs in plain `cargo test` (no datagen/backend dependency)
- [ ] Generated page is timeless — no phase vocabulary in the template

**Commit.** `docs(examples): generated web-analytics maintenance tutorial embedding real emitted SQL, with freshness gate`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- DuckDB 1.5.0 binder defect: `QUALIFY ROW_NUMBER() OVER (...)` mis-binds a
  window function's column type when the immediate FROM is a view whose own
  SELECT list carries type-conforming CASTs (not reproducible on the v1.4.4
  CLI). Worked around by materializing `bronze/raw_events` as a table
  instead of a view; revisit — and consider reverting to the default view
  materialization — once the pinned DuckDB library moves past 1.5.0.

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh` (includes `statement_parity`, `execute_parity`, `example_diagnostics`)
- `python examples/web_analytics/run_incremental.py` then `python examples/web_analytics/verify_incremental_equivalence.py` — full equivalence harness over the extended pipeline
- `cargo test -p smelt-cli --test per_partition_equivalence`
- `python examples/web_analytics/generate_tutorial.py --check` — tutorial page fresh
- `/smelt:validate cli`, `/smelt:validate maintenance_plan`, `/smelt:validate datagen` report zero drift
