# Plan: Web Analytics Phase 5 — `identity_forward_only` + initial `eventstream_with_identity`

**Date**: 2026-05-18
**Spec**: example phases do not anchor to a single feature spec; the oracle is the overall plan ([`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md) §Goal item 3 — three parallel identity algorithms surfaced side-by-side in one wide eventstream) and the meta-plan (`/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §3 row 5). Spec cross-references that ground specific decisions: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) (frontmatter shape, partition_column rules), [`docs/specs/testing.md`](../specs/testing.md) (`materialization: test` inline assertions, YAML coercion).
**Spec diff**: no spec change in this phase. Phase 5 introduces the first two `gold/` models in `examples/web_analytics/` — `identity_forward_only.sql` (per-session within-session resolution) and the initial `eventstream_with_identity.sql` (per-event wide table carrying one identity column today; Phases 6–7 widen it).
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs (inline header comments inside the new SQL files; no `docs-site/` touch — that lands in Phase 8 of the overall plan)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 5 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewer (`sql-expert`), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read the overall plan and the meta-plan for the sentinel emission contract and stop-the-line conditions. The Phase 3 plan (`docs/plans/20260517-web-analytics-3-scaffold.md`) and Phase 4 plan (`docs/plans/20260517-web-analytics-4-sessionize.md`) "Deferred during implementation" sections are required reading — they record concrete smelt constraints that this phase must respect (call-syntax, `to_seconds` not in inference, dead `smelt.ref` syntax, `paths:` discipline, struct-returning function calls in models, `IS DISTINCT FROM` not supported, `epoch_us` arithmetic over `INTERVAL` subtraction, `md5` not registered → use `CONCAT`, two-CTE structure needed for nested window functions, materialised model addresses include directory segment — e.g. `main.silver_sessions` not `main.sessions`, `tests` must appear in `smelt.yml` `paths:`). Do not re-open those decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 4 is the expert-reviewer dispatch loop** — after Phases 1–3 commit, dispatch the meta-plan §5 expert reviewer applicable to this phase (`sql-expert` only — Phase 5 of the overall plan has a single listed expert per meta-plan §5 row 5). Address material findings, re-dispatch until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 4. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 4's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user (emit `<<PAUSE_FOR_HUMAN>>`):**

- The reviewer surfaces the same material finding across two implementer passes on the same sub-phase.
- TDD tests cannot be made green without violating a Phase 3 / Phase 4 deferred-item ground rule.
- The smelt parser or type-checker surfaces a defect that blocks both the canonical SQL form AND the documented fallback for the within-session resolution (`arg_max(... FILTER WHERE ...)` → `LAST_VALUE(... IGNORE NULLS) OVER (...)` → correlated-subquery; see Phase 1 below). Escalating means: the resolution pattern needs a spec/registry change beyond Phase 5 scope.
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to this plan.
- Phase 4 (expert dispatch): `sql-expert` flags the same material finding on round 3 (per-expert bound), or escalates to a systemic concern requiring spec change.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation. The standing oracles are `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`) and the end-to-end integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` (extended per sub-phase — datagen → setup_sources → `smelt build` succeeds and the new model's row counts match invariants).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + the Phase 4 expert spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces *only* `identity_forward_only` and the *initial* shape of `eventstream_with_identity` (one identity column). **No backward_fill, no connected_components, no marts** — those are Phases 6/7/8 of the overall plan.
- Honor architectural invariants from `CLAUDE.md` (no `crates/` edits unless extending the existing `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests, or — per the Phase 4 precedent — registering a missing built-in function in `crates/smelt-types/src/signatures.rs` if the within-session resolution requires it; see Phase 1 below). All SQL must parse without LSP diagnostics in `examples/web_analytics/`.
- **Timeless-oracle rule (CLAUDE.md).** This plan file uses phase vocabulary; the SQL file header comments must read as feature descriptions with no `Phase N` labels.

---

## Context

The overall plan's Goal item 3 asks for three parallel identity-resolution algorithms surfaced side-by-side in one wide `eventstream_with_identity` row-per-event table, so the row-by-row tradeoff between them is directly observable. Phase 5 lands the first algorithm and the wide-table chassis. `identity_forward_only` resolves user identity only *within* a session — the partition is `session_id`, the reduction picks the user_id observed in the session at the event with the latest `event_ts` among non-null observations. Events in a session with zero signed-in observations resolve to NULL. The algorithm is the simplest of the three: it does no cross-session propagation, no per-device canonical-user election, no edge-graph clustering — those are Phases 6 and 7.

`eventstream_with_identity` is the wide event-level table that joins `silver/events_parsed` to its session (via `silver/sessions`) and then attaches each available identity algorithm's resolved column. In Phase 5 it carries one such column (`forward_only_user_id`); Phases 6 and 7 extend the SELECT list with `backward_fill_user_id` and `connected_components_user_id`. The join shape is fixed in this phase so subsequent phases are pure column-addition edits: events_parsed JOIN sessions on (device_id, event_ts ∈ [session_start, session_end]) LEFT JOIN identity_forward_only on session_id.

The within-session reduction is described in the meta-plan context as `arg_max(user_id, event_ts) FILTER (WHERE user_id IS NOT NULL)` grouped by `session_id`. This is a DuckDB built-in. The smelt function registry (`crates/smelt-types/src/signatures.rs`) may or may not list `arg_max` today; that is a discovery item for Phase 1's TDD step. If unregistered, the resolution surfaces two paths consistent with the Phase 4 deferred precedent: (a) register `arg_max` in the registry (small scope creep, mirrors the Phase 4 `crates/smelt-cli/src/test_compiler.rs` fixes), or (b) substitute an equivalent form using already-registered functions (`LAST_VALUE(user_id) IGNORE NULLS OVER (PARTITION BY session_id ORDER BY event_ts ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)` projected into the rows, then `ANY_VALUE` per session_id GROUP BY; or a self-correlated subquery). The implementer picks the lowest-friction path and records the rationale in "Deferred during implementation".

## Scope

### In scope

- `examples/web_analytics/models/gold/identity_forward_only.sql` — a `view` (or `table` if `default_materialization: view` is overridden in Phase 6/7; for now a view is fine since the model is downstream of the incremental `sessions` model and rebuilds are cheap) that joins `smelt.silver.events_parsed` to `smelt.silver.sessions` on `(device_id, event_ts ∈ [session_start, session_end])`, groups by `session_id`, and projects a single derived column `forward_only_user_id: INTEGER` computed as the within-session reduction described above. Output columns: `session_id: VARCHAR`, `forward_only_user_id: INTEGER` (nullable — sessions with zero signed-in observations resolve to NULL). One row per session.
- `examples/web_analytics/models/gold/eventstream_with_identity.sql` — a `view` (default) that exposes every row of `silver/events_parsed` augmented with `session_id` (via JOIN to `silver/sessions`) and `forward_only_user_id` (via LEFT JOIN to `gold/identity_forward_only` on `session_id`). Output columns: `event_id`, `device_id`, `event_user_id` (renamed projection of `events_parsed.user_id` so the raw observation is distinguishable from the resolved columns added in later phases), `event_ts`, `event_date`, `event_name`, `platform`, `url`, `session_id`, `forward_only_user_id`. One row per event.
- `examples/web_analytics/tests/forward_only_resolution_invariants.test.sql` — a `materialization: test` file (per `docs/specs/testing.md`) targeting `gold/identity_forward_only` with mocked `silver_events_parsed` and `silver_sessions` `inputs:` blocks. At minimum, exercises three sessions:
  - **Session A** with one signed-in event at the end → all forward_only resolutions for the session = that user_id.
  - **Session B** with two signed-in events at different times → forward_only resolves to the user_id of the *later* event (the `arg_max(... event_ts)` semantic).
  - **Session C** with zero signed-in events → forward_only_user_id is NULL.
  Because the target model `gold/identity_forward_only` produces one row per session, the test's `expect:` block has one row per session_id with the asserted `forward_only_user_id`. If `target_cte` selection runs cleanly inside the model body, it can pin the intermediate reduction directly; otherwise the whole-model fallback (Option B in Phase 4's precedent) is acceptable.
- `crates/smelt-datagen/tests/example_web_analytics.rs` extension — add `test_eventstream_with_identity_end_to_end` that runs `smelt-datagen ... --scale-factor 0.01 && setup_sources.sql && smelt build`, then opens the DuckDB and asserts: (i) `SELECT count(*) FROM main.gold_eventstream_with_identity = SELECT count(*) FROM main.silver_events_parsed` (event-preserving join — no event is dropped or duplicated); (ii) `SELECT count(*) FROM main.gold_identity_forward_only = SELECT count(*) FROM main.silver_sessions` (one row per session); (iii) for any event row with non-null `event_user_id`, the `forward_only_user_id` for its session is non-null (the session has at least one signed-in observation, so the reduction is non-null); (iv) within any single session, `forward_only_user_id` is single-valued (the per-session reduction propagates to every event in the session through the LEFT JOIN on session_id).

### Explicitly deferred (scope guardrails)

- **No backward_fill.** No `gold/identity_backward_fill.sql`, no `backward_fill_user_id` column in `eventstream_with_identity`. Phase 6 of the overall plan.
- **No connected_components.** No `gold/identity_connected_components.sql`, no `connected_components_user_id` column. Phase 7.
- **No marts.** No `daily_active_users_by_method`, no `identity_method_comparison`. Phase 8.
- **No `paths:` change in `smelt.yml`.** The existing `paths: [models, tests]` already covers `models/gold/` and `tests/`.
- **No edits to `crates/` outside `crates/smelt-datagen/tests/example_web_analytics.rs` and (conditionally, per Phase 1 TDD discovery) `crates/smelt-types/src/signatures.rs` to register `arg_max` if and only if that function is the chosen resolution path AND it is not currently in the registry.** The smelt language surface beyond function registration is fixed for this phase. If a third-path workaround using already-registered functions is feasible, prefer that over registering `arg_max`.
- **No `smelt.functions.<name>(...)` calls in model SQL bodies.** Per the Phase 3 deferred precedent (`parse_event_payload` is declared but not called from `silver/events_parsed`; same root cause — Phase-19 context binding not landed). This phase declares no new smelt functions.
- **No incremental frontmatter on `gold/` models in this phase.** The two new gold models are simple views over the silver layer; `silver/sessions` carries the incremental boundary. If a future phase needs to materialise either gold model as a `table` with `incremental:` (e.g., for performance), it can be added then — the SELECT shapes here are compatible with that future change.

---

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | `7669786c` | 2026-05-18 |
| 2     | done     | `f6ee37ef` | 2026-05-18 |
| 3     | done     | `9421a636` | 2026-05-18 |
| 4     | done     | *(this commit)* | 2026-05-18 |

---

### Phase 1: `gold/identity_forward_only.sql` per-session within-session resolution

**Goal.** Land `examples/web_analytics/models/gold/identity_forward_only.sql` — a view that produces one row per session with `(session_id, forward_only_user_id)`, where `forward_only_user_id` is the user_id observed in the session at the event with the latest `event_ts` among non-null observations. NULL when the session has zero signed-in events.

**Pre-conditions.** Phase 4 of the overall plan committed (`silver/events_parsed`, `silver/sessions`, `silver/device_user_edges` exist; `tests` is in `smelt.yml` `paths:`). Working tree clean on `worktree-web_analytics`.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the model lands. This is the LSP-side oracle. The test fails (reports diagnostics, e.g., "unknown function `arg_max`") before the function body parses cleanly under the chosen resolution path; it passes after.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_identity_forward_only_materializes` — extend the end-to-end test:
  1. Run `smelt-datagen ... --scale-factor 0.01`.
  2. Programmatically execute `setup_sources.sql`.
  3. Invoke `smelt build --project-dir examples/web_analytics --target dev` and assert exit 0.
  4. Open the DuckDB and assert `SELECT count(*) FROM main.gold_identity_forward_only > 0`.
  5. Assert one-row-per-session cardinality: `SELECT count(*) FROM main.gold_identity_forward_only = SELECT count(*) FROM main.silver_sessions`.
  6. Assert the population invariant: every session that contains at least one event with non-null `user_id` in `silver_events_parsed` (joined on device_id + event_ts range) has a non-null `forward_only_user_id` in `gold_identity_forward_only`. Concretely:

     ```sql
     SELECT count(*) FROM (
       SELECT s.session_id
       FROM main.silver_sessions s
       JOIN main.silver_events_parsed e
         ON e.device_id = s.device_id
        AND e.event_ts BETWEEN s.session_start AND s.session_end
       WHERE e.user_id IS NOT NULL
       GROUP BY s.session_id
     ) sessions_with_user
     JOIN main.gold_identity_forward_only f USING (session_id)
     WHERE f.forward_only_user_id IS NULL
     ```
     must equal 0.

**Implementation shape.**

Primary form (preferred if `arg_max` is registered or can be registered):

```sql
-- Per-session resolution: within each session, take the user_id observed at the
-- event with the latest event_ts among non-null observations. Events in
-- sessions with zero signed-in observations resolve to NULL. This is the
-- simplest of the three identity algorithms — no cross-session propagation, no
-- per-device canonical-user election, no edge clustering.
SELECT
    s.session_id,
    arg_max(e.user_id, e.event_ts) FILTER (WHERE e.user_id IS NOT NULL) AS forward_only_user_id
FROM smelt.silver.sessions s
JOIN smelt.silver.events_parsed e
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
GROUP BY s.session_id
```

**Resolution-form decision (TDD discovery step).** Before writing the model, the implementer must verify whether `arg_max` parses without diagnostics. The Phase 4 precedent for unregistered functions (`md5` → `CONCAT`, `IS DISTINCT FROM` → `!=`) is: prefer the canonical DuckDB form if recognised; otherwise pick the lowest-friction equivalent. Order of preference:

1. **`arg_max(value, key) FILTER (WHERE value IS NOT NULL)` aggregate** — canonical form. Check via a one-line probe model first; if the diagnostics gate is clean, use it.
2. **Register `arg_max` in `crates/smelt-types/src/signatures.rs`** — if the function is unregistered, registering it is small scope creep (one row in the signatures table, type signature `arg_max(any, any) -> any` or per the registry's own convention). Mirrors the Phase 4 precedent (test_compiler.rs fixes accepted as scope creep). Record the rationale in "Deferred during implementation".
3. **Fallback: project the resolution onto rows via `LAST_VALUE(user_id) IGNORE NULLS OVER (PARTITION BY session_id ORDER BY event_ts ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)`, then `ANY_VALUE` per session_id.** Two-CTE structure:

   ```sql
   WITH joined AS (
       SELECT
           s.session_id,
           e.event_ts,
           LAST_VALUE(e.user_id) IGNORE NULLS OVER (
               PARTITION BY s.session_id
               ORDER BY e.event_ts
               ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
           ) AS resolved_user_id
       FROM smelt.silver.sessions s
       JOIN smelt.silver.events_parsed e
           ON e.device_id = s.device_id
          AND e.event_ts >= s.session_start
          AND e.event_ts <= s.session_end
   )
   SELECT
       session_id,
       ANY_VALUE(resolved_user_id) AS forward_only_user_id
   FROM joined
   GROUP BY session_id
   ```

   This is the registered-only path. `LAST_VALUE` and `ANY_VALUE` are both in the registry per `crates/smelt-types/src/signatures.rs`. `IGNORE NULLS` modifier on window functions is verified via the diagnostics gate before adoption — if unsupported, escalate via Phase 1's "When to pause" trigger.
4. **Fallback (correlated subquery)** — last resort if neither (1)–(3) parses cleanly:

   ```sql
   SELECT
       s.session_id,
       (SELECT e2.user_id
        FROM smelt.silver.events_parsed e2
        WHERE e2.device_id = s.device_id
          AND e2.event_ts >= s.session_start
          AND e2.event_ts <= s.session_end
          AND e2.user_id IS NOT NULL
        ORDER BY e2.event_ts DESC
        LIMIT 1) AS forward_only_user_id
   FROM smelt.silver.sessions s
   ```

   Subquery support in smelt-parser: verified by retail_analytics models using subqueries in SELECT lists.

Notes the implementer must verify against current smelt support during the discovery step:

- `arg_max` registry presence — primary discovery item.
- `... FILTER (WHERE ...)` clause on aggregates — used in `examples/retail_analytics/models/intermediate/int_return_analysis.sql`, so verified.
- `JOIN ... ON e.event_ts BETWEEN s.session_start AND s.session_end` — verify; if `BETWEEN` is not in the JOIN-`ON` grammar, the equivalent `e.event_ts >= s.session_start AND e.event_ts <= s.session_end` is the documented form (used in implementation shape above).
- `IGNORE NULLS` modifier on `LAST_VALUE` — only checked if fallback (3) is selected.
- `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` window frame — only checked if fallback (3) is selected.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/identity_forward_only.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension)
- `crates/smelt-types/src/signatures.rs` (only if resolution form 2 is selected; one-line registry addition for `arg_max`)
- `crates/smelt-types/tests/registry_coverage.rs` (only if form 2 is selected; one-line coverage assertion mirroring existing `ANY_VALUE` / `FIRST_VALUE` entries)
- `docs/plans/20260517-web-analytics-5-forward-only.md` (this file — committed at the start of Phase 1 work, before any other change)

**Docs touched.** None (header comments in the new SQL file are timeless per the Timeless-oracle rule). If form 2 is selected, the new entry in `signatures.rs` is the canonical registration site — no separate user-doc edit needed since smelt's function registry is not a user-facing surface today (that lands later in the meta-language / functions work).

**Review checklist** (material findings only):

- [ ] Model produces exactly the columns `(session_id, forward_only_user_id)` — no extra columns that would constrain Phase 6/7 prematurely.
- [ ] Source references use path syntax (`smelt.silver.sessions`, `smelt.silver.events_parsed`) — no dead `smelt.ref()` syntax.
- [ ] Within-session reduction picks the user_id at the event with the *latest* `event_ts` among non-null observations (the meta-plan's `arg_max(... event_ts) FILTER (WHERE ... IS NOT NULL)` semantic), not the first or an arbitrary one.
- [ ] Sessions with zero signed-in events resolve to `NULL` (verified by the end-to-end test's invariant query).
- [ ] If form 2 (register `arg_max`) was selected: the registry addition is minimal (one row in `signatures.rs` + one coverage assertion), no other product change.
- [ ] If form 3 (`LAST_VALUE IGNORE NULLS` + `ANY_VALUE`) was selected: the window frame is `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` (frame-defaulted `LAST_VALUE` returns the current row's value, which would be wrong for events before the signed-in observation).
- [ ] Zero diagnostics for the file from `example_diagnostics`.
- [ ] Header comment is timeless — no `Phase N` references in the SQL file. The "simplest of the three identity algorithms" wording is acceptable (it describes feature relationships, not phase history).

**Commit.** `feat(examples): web_analytics gold/identity_forward_only model (web-analytics Phase 5)`

---

### Phase 2: `gold/eventstream_with_identity.sql` per-event wide table (single identity column)

**Goal.** Land `examples/web_analytics/models/gold/eventstream_with_identity.sql` — a per-event view that exposes every row of `silver/events_parsed` augmented with `session_id` (via JOIN to `silver/sessions`) and `forward_only_user_id` (via LEFT JOIN to `gold/identity_forward_only`). One identity-resolved column today; Phases 6 and 7 add two more.

**Pre-conditions.** Phase 1 of this plan committed (`gold/identity_forward_only` materialises).

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/` after the model lands.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_eventstream_with_identity_end_to_end` — extends the end-to-end test:
  1. Run datagen → setup_sources → `smelt build` as in Phase 1.
  2. Assert event-preserving cardinality: `SELECT count(*) FROM main.gold_eventstream_with_identity = SELECT count(*) FROM main.silver_events_parsed`. (The JOIN to sessions is one-to-one on `(device_id, event_ts ∈ [session_start, session_end])`; no event is dropped or duplicated.)
  3. Assert single-valued `forward_only_user_id` within session: `SELECT count(*) FROM (SELECT session_id, count(DISTINCT forward_only_user_id) AS k FROM main.gold_eventstream_with_identity GROUP BY session_id HAVING k > 1) = 0`. (Allowed: `k = 0` for sessions with NULL resolution and `k = 1` for sessions with non-NULL resolution. `DISTINCT` ignores NULL by default, so the HAVING clause correctly catches only "two distinct non-NULL values in one session", which the algorithm forbids.)
  4. Assert non-null resolution for signed-in events: `SELECT count(*) FROM main.gold_eventstream_with_identity WHERE event_user_id IS NOT NULL AND forward_only_user_id IS NULL = 0`. (If the event itself observed a non-null user_id, the session containing it must resolve to non-null — the reduction sees at least one non-null input.)
  5. Assert column shape matches the SELECT list under "Implementation shape".

**Implementation shape.**

```sql
-- Per-event wide table that joins every silver/events_parsed row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved column. Today carries one identity column (forward_only); the wide
-- shape is fixed here so additional algorithms can be added as LEFT JOIN + one
-- column projection without restructuring the row.
SELECT
    e.event_id,
    e.device_id,
    e.user_id AS event_user_id,
    e.event_ts,
    e.event_date,
    e.event_name,
    e.platform,
    e.url,
    s.session_id,
    f.forward_only_user_id
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
LEFT JOIN smelt.gold.identity_forward_only f
    ON s.session_id = f.session_id
```

Notes:

- `event_user_id` (a renamed projection of `events_parsed.user_id`) is the *raw observation* on the event row. The resolved-column equivalents added in later phases (`forward_only_user_id`, plus `backward_fill_user_id` and `connected_components_user_id` in Phases 6–7) are deliberately suffixed with the algorithm name. This naming is the row-by-row comparison surface the overall plan §Goal item 3 calls for.
- The JOIN to `sessions` uses inclusive range bounds matching the session's `[session_start, session_end]` derivation in Phase 4 (`MIN(event_ts) AS session_start`, `MAX(event_ts) AS session_end`). Every event is in exactly one session: the gap-and-platform rule produces non-overlapping `(device_id, session_seq)` partitions, and each partition's range covers every event in the partition. The end-to-end test step 2 catches the event-preserving invariant.
- The LEFT JOIN to `identity_forward_only` is on `session_id`. Every session_id present in `sessions` is present in `identity_forward_only` (one-row-per-session by Phase 1's cardinality invariant), so the LEFT could be an INNER, but LEFT is the safer shape: it preserves event rows even under transient build ordering or model-output mismatch (e.g., when `identity_forward_only` is being rebuilt).
- Materialization is inherited from `default_materialization: view` in `smelt.yml`. If a future phase needs materialisation as a table (for query speed in downstream marts), that's a single frontmatter edit; the SELECT shape is compatible.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/eventstream_with_identity.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Source references use path syntax (`smelt.silver.events_parsed`, `smelt.silver.sessions`, `smelt.gold.identity_forward_only`).
- [ ] JOIN to sessions is INNER (or unqualified) — every event is in exactly one session, so the JOIN preserves event count. LEFT JOIN here would be a bug (sessions table is non-skewed and complete by construction).
- [ ] JOIN to `identity_forward_only` is LEFT — preserves event rows under transient model-output mismatch.
- [ ] `event_user_id` is the renamed projection of `events_parsed.user_id` (raw observation). Not `user_id` unqualified — the latter would conflict with the resolved columns named `<algorithm>_user_id`.
- [ ] Column order: identifying columns (event_id, device_id, event_user_id), temporal (event_ts, event_date), event payload (event_name, platform, url), session attribution (session_id), then resolved columns. This is the shape Phase 6/7 will widen.
- [ ] No reach into Phase 6/7 scope (no `backward_fill_user_id`, no `connected_components_user_id`).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics gold/eventstream_with_identity initial column (web-analytics Phase 5)`

---

### Phase 3: Inline `.test.sql` forward-only resolution invariants

**Goal.** Land `examples/web_analytics/tests/forward_only_resolution_invariants.test.sql` — a `materialization: test` file asserting the three within-session resolution invariants (signed-in event present → resolves to that user; multiple signed-in events → resolves to the latest; zero signed-in events → NULL) on hand-crafted mock data. The verification gate for Phase 5 ("inline test for within-session resolution; diagnostics gate" — meta-plan §3 row 5) is met by this file.

**Pre-conditions.** Phases 1–2 of this plan committed. The Phase 4 plan's "Phase 4: two product fixes in `crates/smelt-cli/src/test_compiler.rs`" precedent lets multi-CTE models with `YYYY-MM-DD HH:MM:SS` timestamps be tested with mock inputs; this phase can use the same machinery without changes.

**TDD tests to write first.**

- The standing `example_diagnostics` gate continues to pass.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_forward_only_invariants_inline_pass` — invoke `smelt test --project-dir examples/web_analytics --select forward_only_resolution` (the test name selector from `docs/specs/testing.md` §"Selector behaviour"); assert exit 0 and that the named test reports PASS.

**Implementation shape.**

The test uses the `materialization: test` format from `docs/specs/testing.md` §"Test file format". The target model is `gold/identity_forward_only`. The model's immediate inputs are `silver/sessions` and `silver/events_parsed`; both must be mocked. Phase 4's `session_boundary_invariants.test.sql` (Option B — whole-model, no `target_cte`) is the precedent for mocking multiple inputs at once.

```sql
--- name: test_forward_only_resolution_invariants ---
materialization: test
test:
  model: identity_forward_only
  inputs:
    silver_sessions:
      # Session A: one signed-in event at the end (event id 2 carries user_id 100)
      - {session_id: 'sa', device_id: 1, session_seq: 0, session_start: '2026-04-01 10:00:00', session_end: '2026-04-01 10:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'}
      # Session B: two signed-in events; the LATER one (event id 5 at 11:08, user_id 201) wins
      - {session_id: 'sb', device_id: 2, session_seq: 0, session_start: '2026-04-01 11:00:00', session_end: '2026-04-01 11:10:00', session_start_date: '2026-04-01', event_count: 3, platform: 'web'}
      # Session C: zero signed-in events
      - {session_id: 'sc', device_id: 3, session_seq: 0, session_start: '2026-04-01 12:00:00', session_end: '2026-04-01 12:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'}
    silver_events_parsed:
      # Session A events
      - {event_id: 1, device_id: 1, user_id: null, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 2, device_id: 1, user_id: 100,  event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      # Session B events — two signed-in observations
      - {event_id: 3, device_id: 2, user_id: 200, event_ts: '2026-04-01 11:02:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      - {event_id: 4, device_id: 2, user_id: null, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 5, device_id: 2, user_id: 201, event_ts: '2026-04-01 11:08:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      # Session C events — all anonymous
      - {event_id: 6, device_id: 3, user_id: null, event_ts: '2026-04-01 12:01:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 7, device_id: 3, user_id: null, event_ts: '2026-04-01 12:09:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
  expect:
    - {session_id: 'sa', forward_only_user_id: 100}
    - {session_id: 'sb', forward_only_user_id: 201}  # the LATER signed-in user wins, not the earlier (user_id 200)
    - {session_id: 'sc', forward_only_user_id: null}
---
```

Notes:

- The `expect:` block tests the *defining* invariants of `identity_forward_only` directly: (a) signed-in user propagates within session; (b) latest-non-null wins when there are multiple signed-in observations; (c) NULL resolution for all-anonymous sessions. Each invariant has its own session_id so a failure is attributable to one rule.
- Timestamp coercion uses the `YYYY-MM-DD HH:MM:SS` form fixed by the Phase 4 `crates/smelt-cli/src/test_compiler.rs` Timestamp coercion patch.
- The mock `silver_sessions` rows must include `session_start_date` because the model joins on `event_ts ∈ [session_start, session_end]` (not on `session_start_date`); the column is present for schema completeness but is not exercised.
- `session_id` is a hand-supplied VARCHAR (`'sa'`, `'sb'`, `'sc'`) rather than the production CONCAT-derived form, so the assertions are surgical.
- The test_compiler "WITH-clause merging" fix from Phase 4 means this works whether `gold/identity_forward_only.sql` uses a single SELECT (form 1) or a multi-CTE structure (form 3).

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/tests/forward_only_resolution_invariants.test.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — the inline-test smoke)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Three sessions exercise three distinct invariants (within-session propagation, latest-non-null tie-break, all-anonymous → NULL). Each invariant attributable to one session_id.
- [ ] Session B has *two* signed-in observations at *different* timestamps so the `arg_max(..., event_ts)` semantic is tested distinguishably from `min` / `any_value`. Specifically: `user_id = 200` at the earlier timestamp, `user_id = 201` at the later — expect `201`.
- [ ] `inputs:` rows mock the full schemas the model SELECTs from (`silver_sessions`: `session_id, device_id, session_seq, session_start, session_end, session_start_date, event_count, platform`; `silver_events_parsed`: `event_id, device_id, user_id, event_ts, event_date, event_name, platform, url`), or coverage gaps documented inline.
- [ ] Test runs under `smelt test` and reports PASS.
- [ ] No reach into Phase 6/7 scope (no `backward_fill_user_id` / `connected_components_user_id` columns in `inputs:` or `expect:`).

**Commit.** `feat(examples): web_analytics forward_only resolution invariant test (web-analytics Phase 5)`

---

### Phase 4: Expert reviewer dispatch loop

For each expert listed below, dispatch via the Agent tool with `model: "sonnet"`,
brief prompt, the per-phase plan path, the spec path (if relevant), and the
in-repo plan path. The expert returns a list of findings classified as
"material" or "stylistic". Address material findings:

  - For each material finding, either edit directly (small) or dispatch a
    nested implementer subagent (larger).
  - Commit the fix with message `review(web-analytics-5): address {expert-name} feedback`.
  - Push.
  - Re-dispatch the same expert. Loop until the expert returns "no material findings".

Bounds:

  - Max 3 rounds per expert. If unresolved after 3 rounds → emit
    `<<PAUSE_FOR_HUMAN>>`.
  - If two different experts flag the same systemic concern in one round →
    emit `<<PAUSE_FOR_HUMAN>>`. (Phase 5 has only one expert in meta-plan §5, so the cross-expert clause is inert here. Retained for template consistency.)

Experts for this phase (from meta-plan §5 row 5):

  - `sql-expert` — focus per meta-plan §5: **`arg_max ... FILTER` correctness, NULL handling**. Specifically:
    - Within-session reduction correctness: the chosen form (whichever of the four resolution paths in Phase 1 was selected) must compute `user_id` at the row with the latest `event_ts` among non-null observations, partitioned by `session_id`. Verify against the three invariants in Phase 3's test.
    - NULL handling: sessions with zero signed-in events resolve to NULL — not to 0, not to an arbitrary string, not to the most recent non-null value across sessions.
    - `FILTER (WHERE ...)` semantics: if form 1 was chosen, verify the FILTER clause is on the aggregate (not a row-level WHERE) and that `arg_max(value, key) FILTER (WHERE value IS NOT NULL)` is semantically the same as `arg_max(value, key) WHERE key matches the row where value IS NOT NULL` — DuckDB documentation may clarify the exact semantic.
    - JOIN range correctness: `e.event_ts >= s.session_start AND e.event_ts <= s.session_end` (inclusive both sides) — verify against Phase 4's session_start / session_end derivation (`MIN(event_ts)`, `MAX(event_ts)`); both bounds are inclusive there, so the JOIN range is correct and covers every event.
    - Event-preserving cardinality of `eventstream_with_identity`: every event is in exactly one session given the gap-and-platform rule. The JOIN therefore yields exactly `count(events_parsed)` rows.
    - LEFT-vs-INNER JOIN choices: `JOIN sessions` (INNER) is justified because the sessions table is complete by construction; `LEFT JOIN identity_forward_only` is justified by safety under transient builds.
    - No parser-unsupported constructs (verified by `example_diagnostics` already, but the expert may catch a construct that parses but is semantically wrong on DuckDB).
    - If form 2 (`arg_max` registration in `signatures.rs`) was chosen: the registry signature reads correctly against DuckDB's actual `arg_max` signature (two positional arguments, optional FILTER clause, returns the type of the first argument).
    - If form 3 (`LAST_VALUE IGNORE NULLS` + `ANY_VALUE`) was chosen: the window frame is `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING`. The default frame (`RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`) would make `LAST_VALUE` return the current row's value, which is wrong for events before the signed-in observation.

If a literal `sql-expert` agent type does not exist, dispatch `general-purpose`
with a prompt that frames it as such (read the plan + diff, flag plan/impl
drift, missing test cases, scope creep into later phases — material findings
only).

**Loop discipline.**

1. **Round 1.** Dispatch `sql-expert` with `model: "sonnet"`. The prompt MUST include:
   - This plan's path and the oracle paths (overall plan, meta-plan, Phase 3 plan's "Deferred during implementation" section, Phase 4 plan's "Deferred during implementation" section, `incremental_models.md`, `testing.md`).
   - The exact file scope from the per-sub-phase tables above.
   - The diff range to review: commits since the start of Phase 5 of the overall plan (typically the three `feat(examples): web_analytics ... (web-analytics Phase 5)` commits — `git log --oneline {phase-5-base}..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, scope creep, missing test cases, plan/impl drift, parser limitations hit). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` if the expert's tool palette allows nested subagents.

2. **Address findings.** For each material finding:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics`, and the `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests after each fix batch.
   - Commit per round: `review(web-analytics-5): address sql-expert feedback`.
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch `sql-expert` with the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → expert is clean and exits.

4. **Repeat** until clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - `sql-expert` flags a material finding on round 3 (per-expert bound).
   - An expert's findings would force a spec change. Run `/smelt:spec` on the relevant slug first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 5.

**Critical files (allowed to touch in this phase).** Anything within the
expert's scope per the table above, plus
`docs/plans/20260517-web-analytics-5-forward-only.md` (to record round counts
and the final clean status) and `docs/plans/20260517-web-analytics-example.md`
(to flip the overall-plan status row).

**Review checklist** (applied to the expert-dispatch *process*, not to a code diff):

- [ ] `sql-expert` dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`,
  `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`), the end-to-end integration tests, and the inline `.test.sql` invariant test all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 4 expert review: sql-expert clean (R{n}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 5 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings:
`review(web-analytics-5): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-5): mark Phase 5 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 1: registering `arg_max` required edits beyond `signatures.rs`.** Form 2 was selected. Adding a function to the registry meant extending three files in lockstep so the registries stayed in sync: `crates/smelt-types/src/signatures.rs` (signature entry), `crates/smelt-types/src/functions.rs` (`SqlFunction::ArgMax` variant + `ALL_FUNCTIONS` + `name()` + `is_agg()` category match), and `crates/smelt-db/src/type_inference/function_call.rs` (inference arm returning the first-arg type). The plan's "one-line registry addition" wording underestimated this; the structural complement is unavoidable because `check_types.rs` uses `SqlFunction::from_name` to surface the unrecognized-function diagnostic, and `infer_function_type` must produce a type when the function appears. `arg_min` was *not* registered (initially added as a symmetric pair, removed after sql-expert-style review flagged it as out-of-scope given no current call site).

- **Phase 4 expert review: sql-expert clean (R1). No stop-the-line fired.**

## Verification

How to confirm the scope is satisfied at the end of Phase 4:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — no regression in the workspace.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `cargo test -p smelt-datagen --test example_web_analytics` passes — including the new `gold/identity_forward_only`, `gold/eventstream_with_identity`, and inline-test sub-tests.
- Manual fresh-checkout dry run succeeds:
  ```bash
  smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01
  duckdb examples/web_analytics/target/dev.duckdb < examples/web_analytics/setup_sources.sql
  smelt build --project-dir examples/web_analytics --target dev
  smelt test --project-dir examples/web_analytics --select forward_only_resolution
  ```
- Phase 4 acceptance gate met: `sql-expert` reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 5 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
