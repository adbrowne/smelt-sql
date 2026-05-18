# Plan: Web Analytics Phase 6 — `identity_backward_fill` + extend `eventstream_with_identity`

**Date**: 2026-05-18
**Spec**: example phases do not anchor to a single feature spec; the oracle is the overall plan ([`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md) §Goal item 3 — three parallel identity algorithms surfaced side-by-side in one wide eventstream) and the meta-plan (`/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §3 row 6). Spec cross-references that ground specific decisions: [`docs/specs/testing.md`](../specs/testing.md) (`materialization: test` inline assertions, YAML coercion), [`docs/specs/incremental_models.md`](../specs/incremental_models.md) (only consulted if the new gold model is later promoted to `table`).
**Spec diff**: no spec change in this phase. Phase 6 introduces the second gold-layer identity model — `identity_backward_fill.sql` (per-device canonical user via `DISTINCT ON (device_id) ORDER BY event_count DESC, first_seen ASC` over `silver/device_user_edges`) — and adds a `backward_fill_user_id` column to the existing `gold/eventstream_with_identity.sql`. Phase 7 will widen the eventstream once more with `connected_components_user_id`.
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs (inline header comments inside the new and modified SQL files; no `docs-site/` touch — that lands in Phase 8 of the overall plan)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 6 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewer (`sql-expert`), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read the overall plan and the meta-plan for the sentinel emission contract and stop-the-line conditions. The Phase 3 plan (`docs/plans/20260517-web-analytics-3-scaffold.md`), Phase 4 plan (`docs/plans/20260517-web-analytics-4-sessionize.md`), and Phase 5 plan (`docs/plans/20260517-web-analytics-5-forward-only.md`) "Deferred during implementation" sections are required reading — they record concrete smelt constraints this phase must respect (call-syntax discipline, `to_seconds` not in inference, dead `smelt.ref` syntax, `paths:` discipline, struct-returning function calls in models, `IS DISTINCT FROM` not supported, `epoch_us` arithmetic over `INTERVAL` subtraction, `md5` not registered → use `CONCAT`, two-CTE structure needed for nested window functions, materialised model addresses include directory segment — e.g. `main.silver_sessions`, `main.gold_identity_forward_only` — and `tests` must appear in `smelt.yml` `paths:`; registering a function in `crates/smelt-types/src/signatures.rs` also requires lockstep edits in `crates/smelt-types/src/functions.rs` and `crates/smelt-db/src/type_inference/function_call.rs`). Do not re-open those decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 4 is the expert-reviewer dispatch loop** — after Phases 1–3 commit, dispatch the meta-plan §5 expert reviewer applicable to this phase (`sql-expert` only — Phase 6 of the overall plan has a single listed expert per meta-plan §5 row 6). Address material findings, re-dispatch until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 4. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 4's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user (emit `<<PAUSE_FOR_HUMAN>>`):**

- The reviewer surfaces the same material finding across two implementer passes on the same sub-phase.
- TDD tests cannot be made green without violating a Phase 3 / Phase 4 / Phase 5 deferred-item ground rule.
- The smelt parser or type-checker surfaces a defect that blocks both the canonical SQL form (`DISTINCT ON (device_id) ORDER BY ...`) AND the documented fallback (`ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY ...) = 1`). Escalating means: the resolution pattern needs a spec/registry change beyond Phase 6 scope.
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to this plan.
- Phase 4 (expert dispatch): `sql-expert` flags the same material finding on round 3 (per-expert bound), or escalates to a systemic concern requiring spec change.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation. The standing oracles are `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`) and the end-to-end integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` (extended per sub-phase — datagen → setup_sources → `smelt build` succeeds and the new model's row counts match invariants).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + the Phase 4 expert spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces *only* `identity_backward_fill` and the one-column extension of `eventstream_with_identity`. **No connected_components, no marts, no README polish** — those are Phases 7/8 of the overall plan.
- Honor architectural invariants from `CLAUDE.md` (no `crates/` edits unless extending the existing `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests). All SQL must parse without LSP diagnostics in `examples/web_analytics/`.
- **Timeless-oracle rule (CLAUDE.md).** This plan file uses phase vocabulary; SQL file header comments must read as feature descriptions with no `Phase N` labels.

---

## Context

The overall plan's Goal item 3 asks for three parallel identity-resolution algorithms surfaced side-by-side in one wide `eventstream_with_identity` row-per-event table. Phase 5 landed the first algorithm (`identity_forward_only`, within-session) and the wide-table chassis with one resolved column. Phase 6 adds the second algorithm — `identity_backward_fill`, the Amplitude-basic per-device canonical-user election — and extends the wide table with `backward_fill_user_id`. Phase 7 will add the third (`identity_connected_components`).

The backward-fill algorithm answers a strictly different question than forward-only:

- **Forward-only** resolves identity *within* a session — events before the signed-in observation get the user attribution from that observation, but only inside the session window.
- **Backward-fill** resolves identity *across all sessions on a device* — once a user has signed in on a device, *all* prior anonymous events on that device (including those in earlier sessions and on different days) are retroactively tagged with the canonical user. The canonical user is the one with the most signed-in observations on the device, ties broken by earliest `first_seen`.

The expected algorithmic relationship at the mart level is `count(distinct forward_only_user) ≤ count(distinct backward_fill_user) ≤ count(distinct connected_components_user)` on any day: backward-fill subsumes forward-only because every event a forward-only attribution sees is also visible to backward-fill (the device's canonical user includes that signed-in observation). Connected-components widens further by clustering across devices. The mart verification in Phase 8 of the overall plan asserts this monotonicity.

The input is `silver/device_user_edges`, which the Phase 4 scaffold already exposes with `(device_id, user_id, event_count, first_seen, last_seen)` — one row per `(device, user)` pair, aggregated from every signed-in event in `silver/events_parsed`. The reduction is `DISTINCT ON (device_id) ORDER BY event_count DESC, first_seen ASC` (the canonical Postgres-style form, which the smelt parser recognises — `crates/smelt-parser/src/syntax_kind.rs` defines `DISTINCT_ON_CLAUSE` and `crates/smelt-parser/src/parser/select.rs` has the parsing branch). If the diagnostics gate is clean on the canonical form, we use it; otherwise the documented `ROW_NUMBER()`-windowed fallback (already in use by `examples/retail_analytics/models/intermediate/int_customer_orders.sql`) is the back-up.

`eventstream_with_identity` widens by one LEFT JOIN to the new model on `device_id` (one row per device — Cartesian join is impossible because device_id is the primary key of `identity_backward_fill`). Existing columns and join shapes are preserved. The order of the new column in the SELECT list immediately follows `forward_only_user_id` so the row layout is `... session_id, forward_only_user_id, backward_fill_user_id` — Phase 7 will append `connected_components_user_id` after that.

## Scope

### In scope

- `examples/web_analytics/models/gold/identity_backward_fill.sql` — a `view` (default materialization) that selects from `smelt.silver.device_user_edges` and produces one row per device with `(device_id, backward_fill_user_id)`, where `backward_fill_user_id` is the `user_id` with the highest `event_count` on the device (ties broken by earliest `first_seen`). Output columns: `device_id: INTEGER`, `backward_fill_user_id: INTEGER` (non-nullable in the model output — every row of `device_user_edges` carries a non-null user_id by construction of the edges, so the reduction always yields a non-null value; devices that never had a signed-in event simply do not appear). One row per device.
- `examples/web_analytics/models/gold/eventstream_with_identity.sql` — edit (not replace): keep all existing columns and join shapes; add `LEFT JOIN smelt.gold.identity_backward_fill b ON e.device_id = b.device_id` and add `b.backward_fill_user_id` to the SELECT list immediately after `f.forward_only_user_id`. Update the header comment block to mention the new column. Devices that never had a signed-in event yield `NULL` via the LEFT JOIN — this is the row-by-row visible signature of the algorithm: an all-anonymous device's events stay NULL under backward-fill (the canonical-user-election has no candidate).
- `examples/web_analytics/tests/backward_fill_resolution_invariants.test.sql` — a `materialization: test` file (per `docs/specs/testing.md`) targeting `gold/identity_backward_fill` with a mocked `silver_device_user_edges` `inputs:` block. At minimum, exercises four cases:
  - **Device 1 — clear winner.** Two (device, user) edges: `(1, 100, event_count=5, first_seen='2026-04-01 10:00:00')` and `(1, 101, event_count=2, first_seen='2026-04-01 11:00:00')`. Canonical user is 100 (higher event_count).
  - **Device 2 — tie broken by first_seen.** Two edges: `(2, 200, event_count=3, first_seen='2026-04-01 12:00:00')` and `(2, 201, event_count=3, first_seen='2026-04-01 11:00:00')`. Both have `event_count=3`; canonical user is 201 (earlier `first_seen`).
  - **Device 3 — single user.** One edge: `(3, 300, event_count=1, first_seen='2026-04-01 13:00:00')`. Canonical user is 300.
  - **Device 4 — three-way scenario** to stress the secondary order: `(4, 400, event_count=10, first_seen='2026-04-01 09:00:00')`, `(4, 401, event_count=10, first_seen='2026-04-01 08:00:00')`, `(4, 402, event_count=5, first_seen='2026-04-01 07:00:00')`. Canonical is 401 (tied event_count with 400, but earlier first_seen; 402's earlier first_seen does not matter because its event_count loses on the primary sort).
  Because the target model produces one row per device, the test's `expect:` block has one row per device_id with the asserted `backward_fill_user_id`. Devices that never had a signed-in event are not in `silver/device_user_edges` and therefore not in the expected output (consistent with the overall scope: those device's events resolve to NULL in `eventstream_with_identity` via the LEFT JOIN, which is exercised by the end-to-end integration test below).
- `crates/smelt-datagen/tests/example_web_analytics.rs` extensions:
  - `test_identity_backward_fill_materializes` — runs `smelt-datagen ... --scale-factor 0.01 && setup_sources.sql && smelt build`, then asserts: (i) `count(*) FROM main.gold_identity_backward_fill > 0`; (ii) row count equals `count(DISTINCT device_id) FROM main.silver_device_user_edges` (one row per device that ever had a signed-in event); (iii) `backward_fill_user_id IS NOT NULL` for every row of `main.gold_identity_backward_fill` (the model itself never yields NULL — NULL only enters via the LEFT JOIN downstream); (iv) per-device determinism: for every device the chosen `backward_fill_user_id` is the user with the highest `event_count` (ties broken by `MIN(first_seen)`), verified by a self-join against a `MAX(event_count)` aggregate of `device_user_edges`.
  - `test_eventstream_with_identity_includes_backward_fill` — extends the existing eventstream end-to-end test: (i) `backward_fill_user_id` column exists in `main.gold_eventstream_with_identity` (probed by `SELECT backward_fill_user_id FROM ... LIMIT 1`); (ii) every event whose `device_id` appears in `gold_identity_backward_fill` has a non-null `backward_fill_user_id` in `eventstream_with_identity` — i.e. the LEFT JOIN on `device_id` populates correctly; (iii) within any single device, `backward_fill_user_id` is single-valued (the per-device election produces exactly one canonical user, propagated to every event on that device); (iv) the backward-fill subsumption invariant: every event with a non-null `forward_only_user_id` also has a non-null `backward_fill_user_id` (forward-only resolves only when a session sees a signed-in event; in that case the device by definition has at least one edge in `device_user_edges`, so backward-fill's election yields a non-null user too).

### Explicitly deferred (scope guardrails)

- **No connected_components.** No `gold/identity_connected_components.sql`, no `connected_components_user_id` column in `eventstream_with_identity`. Phase 7 of the overall plan.
- **No marts.** No `daily_active_users_by_method`, no `identity_method_comparison`. Phase 8.
- **No README polish.** The Phase 3 README stub stays as is. Phase 8 completes it.
- **No `paths:` change in `smelt.yml`.** The existing `paths: [models, tests]` already covers `models/gold/` and `tests/`.
- **No edits to `crates/` outside `crates/smelt-datagen/tests/example_web_analytics.rs`.** The smelt language surface is unchanged in this phase — both candidate resolution forms (`DISTINCT ON` and `ROW_NUMBER() OVER (...)`) use already-registered surface; no `signatures.rs` edits are required.
- **No `smelt.functions.<name>(...)` calls in model SQL bodies.** Per the Phase 3 / Phase 4 deferred precedent.
- **No `incremental:` frontmatter on the new gold model.** `silver/device_user_edges` is itself a view (not incremental); the upstream `silver/sessions` carries the only incremental boundary in the example. If a future phase needs to materialise `identity_backward_fill` as a `table` with `incremental:` (e.g., for query speed in downstream marts), it can be added then — the SELECT shape here is compatible with that future change.
- **No new joint-distribution edges in the canonical synthetic dataset.** The Phase 2 `linked_choice` generator's joint pool (60/25/10/5 weights, max 3 emits per draw) already produces enough multi-user devices for the backward-fill test to be non-trivial. We do not tweak `datagen.yaml`.

---

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |

---

### Phase 1: `gold/identity_backward_fill.sql` per-device canonical-user election

**Goal.** Land `examples/web_analytics/models/gold/identity_backward_fill.sql` — a view that produces one row per device with `(device_id, backward_fill_user_id)` from `silver/device_user_edges`, where `backward_fill_user_id` is the user_id with the highest `event_count` on the device, ties broken by earliest `first_seen`.

**Pre-conditions.** Phase 5 of the overall plan committed (`gold/identity_forward_only`, `gold/eventstream_with_identity` exist). Working tree clean on `worktree-web_analytics`.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the model lands.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_identity_backward_fill_materializes` — extend the end-to-end test:
  1. Run `smelt-datagen ... --scale-factor 0.01`.
  2. Execute `setup_sources.sql`.
  3. Invoke `smelt build --project-dir examples/web_analytics --target dev` and assert exit 0.
  4. Assert `SELECT count(*) FROM main.gold_identity_backward_fill > 0`.
  5. Assert one-row-per-device cardinality:
     ```sql
     SELECT count(*) FROM main.gold_identity_backward_fill
     = SELECT count(DISTINCT device_id) FROM main.silver_device_user_edges
     ```
  6. Assert non-null output: `SELECT count(*) FROM main.gold_identity_backward_fill WHERE backward_fill_user_id IS NULL = 0`.
  7. Assert per-device determinism (the algorithm picks the user with the highest event_count, ties broken by min first_seen):
     ```sql
     SELECT count(*) FROM main.gold_identity_backward_fill bf
     JOIN (
       SELECT device_id, MAX(event_count) AS max_count
       FROM main.silver_device_user_edges
       GROUP BY device_id
     ) m ON bf.device_id = m.device_id
     JOIN main.silver_device_user_edges e
       ON e.device_id = bf.device_id
      AND e.user_id = bf.backward_fill_user_id
     WHERE e.event_count != m.max_count
     ```
     must equal 0 — the chosen user's event_count must equal the max for the device.

**Implementation shape.**

Primary form (preferred — canonical Postgres-style DISTINCT ON, parsed by smelt-parser):

```sql
-- Per-device canonical-user election. From silver/device_user_edges (the
-- (device, user) co-occurrence evidence over all signed-in events), pick the
-- user_id with the highest event_count for each device; ties broken by
-- earliest first_seen. The chosen user is the device's "canonical" user under
-- the Amplitude-basic backward-fill model — once a user has signed in on a
-- device, every event on that device retroactively belongs to that user
-- (regardless of session or whether the event itself was signed-in).
--
-- Devices that never had a signed-in event do not appear in
-- silver/device_user_edges, and therefore do not appear in this table either.
-- Their events resolve to NULL in gold/eventstream_with_identity via the LEFT
-- JOIN downstream.
SELECT DISTINCT ON (device_id)
    device_id,
    user_id AS backward_fill_user_id
FROM smelt.silver.device_user_edges
ORDER BY device_id, event_count DESC, first_seen ASC
```

**Resolution-form decision (TDD discovery step).** Before writing the model, the implementer must verify whether `DISTINCT ON` parses without diagnostics. Order of preference:

1. **`DISTINCT ON (device_id) ORDER BY device_id, event_count DESC, first_seen ASC`** — canonical Postgres-style form. The smelt parser recognises `DISTINCT ON` (`syntax_kind.rs:191`, `select.rs:37`). Probe via the diagnostics gate; if clean, use it.
2. **Fallback: `ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY event_count DESC, first_seen ASC)` filtered to `= 1`.** Already in use by `examples/retail_analytics/models/intermediate/int_customer_orders.sql` and `examples/retail_analytics/models/marts/mart_customer_lifetime_value.sql`, so verified. Shape:

   ```sql
   WITH ranked AS (
       SELECT
           device_id,
           user_id,
           ROW_NUMBER() OVER (
               PARTITION BY device_id
               ORDER BY event_count DESC, first_seen ASC
           ) AS rn
       FROM smelt.silver.device_user_edges
   )
   SELECT
       device_id,
       user_id AS backward_fill_user_id
   FROM ranked
   WHERE rn = 1
   ```

   This is the registered-only path. `ROW_NUMBER` is in the registry (`crates/smelt-types/src/signatures.rs:3577`; `crates/smelt-types/src/functions.rs:64,228`).

The implementer picks form 1 if the diagnostics gate is clean on a one-line probe, else form 2. Record the chosen form in "Deferred during implementation".

Notes the implementer must verify against current smelt support during the discovery step:

- `DISTINCT ON (col)` clause — primary discovery item.
- `ORDER BY col1, col2 DESC, col3 ASC` mixing ASC/DESC on different columns — well-supported across DuckDB and smelt-parser; used in many existing examples.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/identity_backward_fill.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_identity_backward_fill_materializes` fn)
- `docs/plans/20260517-web-analytics-6-backward-fill.md` (this file — committed at the start of Phase 1 work)

**Docs touched.** None (header comment in the new SQL file is timeless per the Timeless-oracle rule).

**Review checklist** (material findings only):

- [ ] Model produces exactly the columns `(device_id, backward_fill_user_id)` — no extra columns that would constrain Phase 7 prematurely.
- [ ] Source reference uses path syntax (`smelt.silver.device_user_edges`) — no dead `smelt.ref()` syntax.
- [ ] Per-device reduction picks the user_id with the *highest* `event_count`, ties broken by *earliest* `first_seen` (the meta-plan's `DISTINCT ON (device_id) ORDER BY count DESC, first_seen ASC` semantic). Not the lowest event_count, not the latest first_seen.
- [ ] Devices with multiple (device, user) edges all yield exactly one row in the output; devices not in `device_user_edges` are absent from the output (handled downstream via LEFT JOIN).
- [ ] If form 2 (`ROW_NUMBER()`) was selected: the window frame defaults are fine (no `ROWS BETWEEN` needed because `ROW_NUMBER` is a ranking window function whose frame doesn't affect its value); the `WHERE rn = 1` filter is applied in the outer SELECT.
- [ ] Zero diagnostics for the file from `example_diagnostics`.
- [ ] Header comment is timeless — no `Phase N` references in the SQL file. Wording that describes the algorithm's relationship to forward-only ("Amplitude-basic" / "subsumes forward-only on a device") is acceptable (feature relationships, not phase history).

**Commit.** `feat(examples): web_analytics gold/identity_backward_fill model (web-analytics Phase 6)`

---

### Phase 2: Extend `gold/eventstream_with_identity.sql` with `backward_fill_user_id`

**Goal.** Edit `examples/web_analytics/models/gold/eventstream_with_identity.sql` to add `LEFT JOIN smelt.gold.identity_backward_fill b ON e.device_id = b.device_id` and project `b.backward_fill_user_id` immediately after `f.forward_only_user_id` in the SELECT list. Preserve every other join, filter, and column verbatim.

**Pre-conditions.** Phase 1 of this plan committed (`gold/identity_backward_fill` materialises).

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/` after the edit.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_eventstream_with_identity_includes_backward_fill` — a new end-to-end test (parallel to the existing `test_eventstream_with_identity_end_to_end`, but focused on the backward-fill column):
  1. Run datagen → setup_sources → `smelt build` as in Phase 1.
  2. Assert column shape: `SELECT backward_fill_user_id FROM main.gold_eventstream_with_identity LIMIT 1` does not error and returns a row.
  3. Assert event-preserving cardinality is unchanged: `SELECT count(*) FROM main.gold_eventstream_with_identity = SELECT count(*) FROM main.silver_events_parsed`. (The new LEFT JOIN is on device_id, which is one-to-one against `identity_backward_fill`; cardinality is preserved.)
  4. Assert LEFT-JOIN population: for every event whose `device_id` is in `gold_identity_backward_fill`, the `backward_fill_user_id` column is non-null:
     ```sql
     SELECT count(*) FROM main.gold_eventstream_with_identity es
     JOIN main.gold_identity_backward_fill bf USING (device_id)
     WHERE es.backward_fill_user_id IS NULL
     ```
     must equal 0.
  5. Assert single-valued `backward_fill_user_id` within device:
     ```sql
     SELECT count(*) FROM (
       SELECT device_id, count(DISTINCT backward_fill_user_id) AS k
       FROM main.gold_eventstream_with_identity
       GROUP BY device_id
       HAVING k > 1
     )
     ```
     must equal 0. (Allowed: `k = 0` for devices with no edges → LEFT JOIN yields NULL → `count(DISTINCT NULL) = 0`; `k = 1` for devices with at least one edge.)
  6. Assert subsumption: every event with a non-null `forward_only_user_id` also has a non-null `backward_fill_user_id`:
     ```sql
     SELECT count(*) FROM main.gold_eventstream_with_identity
     WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NULL
     ```
     must equal 0. (Forward-only resolves non-null only when a session sees a signed-in event; that signed-in event contributes to `device_user_edges`, so the device is in `identity_backward_fill` and the LEFT JOIN yields a non-null value.)

**Implementation shape.**

```sql
-- Per-event wide table that joins every silver/events_parsed row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved column. Today carries two identity columns (forward_only,
-- backward_fill); the wide shape is fixed so additional algorithms can be
-- added as LEFT JOIN + one column projection without restructuring the row.
--
-- Columns:
--   event_id              — opaque event identifier from raw ingestion
--   device_id             — the device that generated the event
--   event_user_id         — raw user_id observation on the event (nullable)
--   event_ts              — timestamp of the event
--   event_date            — calendar date of the event (partition key in raw)
--   event_name            — decoded event name from the JSON payload
--   platform              — decoded platform from the JSON payload
--   url                   — decoded url from the JSON payload
--   session_id            — the session this event belongs to
--   forward_only_user_id  — resolved identity via the within-session algorithm
--                           (NULL for sessions with zero signed-in events)
--   backward_fill_user_id — resolved identity via the per-device canonical-user
--                           election (NULL for devices that never had a
--                           signed-in event); see gold/identity_backward_fill
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
    f.forward_only_user_id,
    b.backward_fill_user_id
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
LEFT JOIN smelt.gold.identity_forward_only f
    ON s.session_id = f.session_id
LEFT JOIN smelt.gold.identity_backward_fill b
    ON e.device_id = b.device_id
```

Notes:

- The LEFT JOIN to `identity_backward_fill` is on `device_id`. Devices that never had a signed-in event do not appear in `device_user_edges` and therefore not in `identity_backward_fill`; their events get `NULL` for `backward_fill_user_id`. This is the algorithm's defining signature at the event level: all-anonymous devices stay anonymous under backward-fill (the canonical-user election has no candidate).
- LEFT (not INNER) is required so we don't silently drop all events from anonymous-only devices. Without this, the event count of `eventstream_with_identity` would change as the dataset evolves — anonymous-only devices would be culled from the join. The existing event-preserving cardinality test (step 3 above) catches the bug.
- Column order: `... session_id, forward_only_user_id, backward_fill_user_id`. Phase 7 appends `connected_components_user_id` to the end.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/eventstream_with_identity.sql` (edit — header comment block expanded, one new LEFT JOIN, one new column)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_eventstream_with_identity_includes_backward_fill` fn)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Existing columns and join shapes are preserved verbatim — only one new LEFT JOIN and one new column added.
- [ ] New JOIN to `identity_backward_fill` is LEFT (not INNER) — preserves events on anonymous-only devices.
- [ ] New JOIN condition is `e.device_id = b.device_id`. Not `s.device_id = b.device_id` (also correct since the events↔sessions join is by device_id, but the convention in this file is to drive joins off `e` for event-row-level attributes).
- [ ] Column order: `forward_only_user_id` precedes `backward_fill_user_id`. Phase 7 expects to append after `backward_fill_user_id`.
- [ ] Header comment block updated to list `backward_fill_user_id`. Timeless wording — no `Phase N` references.
- [ ] No reach into Phase 7 scope (no `connected_components_user_id`, no recursive-CTE references).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics eventstream backward_fill column (web-analytics Phase 6)`

---

### Phase 3: Inline `.test.sql` backward-fill resolution invariants

**Goal.** Land `examples/web_analytics/tests/backward_fill_resolution_invariants.test.sql` — a `materialization: test` file asserting the three defining invariants of the per-device canonical-user election (most-events wins; tie broken by earliest first_seen; primary sort dominates the tie-breaker) on hand-crafted mock data. The verification gate for Phase 6 ("inline test for retroactive tagging; diagnostics gate" — meta-plan §3 row 6) is met by this file.

**Pre-conditions.** Phases 1–2 of this plan committed. Phase 4's `test_compiler.rs` patches (WITH-clause merging, Timestamp coercion) are already in place — they support this test without further changes.

**TDD tests to write first.**

- The standing `example_diagnostics` gate continues to pass.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_backward_fill_invariants_inline_pass` — invoke `smelt test --project-dir examples/web_analytics --select backward_fill_resolution` (the test name selector from `docs/specs/testing.md` §"Selector behaviour"); assert exit 0 and that the named test reports PASS.

**Implementation shape.**

```sql
--- name: test_backward_fill_resolution_invariants ---
materialization: test
test:
  model: identity_backward_fill
  inputs:
    silver_device_user_edges:
      # Device 1 — clear winner on event_count (100 > 101)
      - {device_id: 1, user_id: 100, event_count: 5, first_seen: '2026-04-01 10:00:00', last_seen: '2026-04-01 10:30:00'}
      - {device_id: 1, user_id: 101, event_count: 2, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:10:00'}
      # Device 2 — tie on event_count, broken by earlier first_seen (user 201 wins; 11:00 < 12:00)
      - {device_id: 2, user_id: 200, event_count: 3, first_seen: '2026-04-01 12:00:00', last_seen: '2026-04-01 12:30:00'}
      - {device_id: 2, user_id: 201, event_count: 3, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:30:00'}
      # Device 3 — single user
      - {device_id: 3, user_id: 300, event_count: 1, first_seen: '2026-04-01 13:00:00', last_seen: '2026-04-01 13:01:00'}
      # Device 4 — three-way: 400 and 401 tie on event_count=10; 401 wins (earlier first_seen).
      # 402 has the earliest first_seen overall but loses on primary sort (event_count=5 < 10).
      - {device_id: 4, user_id: 400, event_count: 10, first_seen: '2026-04-01 09:00:00', last_seen: '2026-04-01 12:00:00'}
      - {device_id: 4, user_id: 401, event_count: 10, first_seen: '2026-04-01 08:00:00', last_seen: '2026-04-01 11:00:00'}
      - {device_id: 4, user_id: 402, event_count: 5,  first_seen: '2026-04-01 07:00:00', last_seen: '2026-04-01 07:30:00'}
  expect:
    - {device_id: 1, backward_fill_user_id: 100}  # higher event_count wins
    - {device_id: 2, backward_fill_user_id: 201}  # tie broken by earlier first_seen
    - {device_id: 3, backward_fill_user_id: 300}  # only candidate
    - {device_id: 4, backward_fill_user_id: 401}  # primary sort dominates: 401 wins the event_count tie
---
```

Notes:

- The `expect:` block tests the *defining* invariants of `identity_backward_fill` directly: (a) primary sort (`event_count DESC`) — Device 1; (b) secondary sort (`first_seen ASC`) when primary ties — Device 2; (c) primary sort dominates the secondary — Device 4 (user 402 has the earliest `first_seen` overall but loses because its `event_count` is lower than 400/401's).
- Device 3 establishes the single-candidate base case — useful for diagnosing failures (if the model returns zero rows for Device 3, the `DISTINCT ON` or `ROW_NUMBER()` filter is misapplied).
- Timestamp coercion uses the `YYYY-MM-DD HH:MM:SS` form fixed by the Phase 4 `crates/smelt-cli/src/test_compiler.rs` patch.
- `last_seen` is present for schema completeness (the mock must match the upstream view's column set) but is not exercised by the algorithm.
- The test works whether the model uses form 1 (`DISTINCT ON`, single SELECT) or form 2 (`WITH ranked AS ... SELECT ... WHERE rn = 1`, multi-CTE) — Phase 4's `test_compiler.rs` WITH-clause merging fix supports both.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/tests/backward_fill_resolution_invariants.test.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_backward_fill_invariants_inline_pass` fn)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Four devices exercise four distinct invariants (clear primary, tie broken by secondary, single candidate, primary dominates secondary). Each invariant attributable to one device_id.
- [ ] Device 2 has both users with the *same* event_count, distinguishable only by `first_seen`. Device 4 has 402 with the overall earliest `first_seen` but a lower `event_count`, so it loses; this distinguishes "earliest first_seen wins" (wrong) from "earliest first_seen among tied event_counts wins" (right).
- [ ] `inputs:` rows mock the full schema of `silver_device_user_edges` (`device_id, user_id, event_count, first_seen, last_seen`).
- [ ] Test runs under `smelt test` and reports PASS.
- [ ] No reach into Phase 7 scope (no `connected_components_user_id` columns in `inputs:` or `expect:`).

**Commit.** `feat(examples): web_analytics backward_fill resolution invariant test (web-analytics Phase 6)`

---

### Phase 4: Expert reviewer dispatch loop

For each expert listed below, dispatch via the Agent tool with `model: "sonnet"`,
brief prompt, the per-phase plan path, the spec path (if relevant), and the
in-repo plan path. The expert returns a list of findings classified as
"material" or "stylistic". Address material findings:

  - For each material finding, either edit directly (small) or dispatch a
    nested implementer subagent (larger).
  - Commit the fix with message `review(web-analytics-6): address {expert-name} feedback`.
  - Push.
  - Re-dispatch the same expert. Loop until the expert returns "no material findings".

Bounds:

  - Max 3 rounds per expert. If unresolved after 3 rounds → emit
    `<<PAUSE_FOR_HUMAN>>`.
  - If two different experts flag the same systemic concern in one round →
    emit `<<PAUSE_FOR_HUMAN>>`. (Phase 6 has only one expert in meta-plan §5, so the cross-expert clause is inert here. Retained for template consistency.)

Experts for this phase (from meta-plan §5 row 6):

  - `sql-expert` — focus per meta-plan §5: **`DISTINCT ON` tie-breaking determinism**. Specifically:
    - Per-device reduction correctness: the chosen form (whichever of the two resolution paths in Phase 1 was selected) must compute the `user_id` with the maximum `event_count` per `device_id`, with ties broken by earliest `first_seen`. Verify against the four invariants in Phase 3's test.
    - Determinism under further ties: if two rows in `device_user_edges` share both `device_id`, `event_count`, AND `first_seen`, the result is unspecified by the algorithm but should be stable across rebuilds. In practice this is exceedingly rare because `first_seen` is a timestamp drawn from event data with microsecond resolution; if the expert flags this as a concern, the resolution is to add a final tiebreaker on `user_id ASC` (cheap, deterministic). Note whether this is necessary in the round-1 review.
    - `DISTINCT ON` semantics under DuckDB: the canonical PG semantic is "for each unique value of the DISTINCT ON columns, keep the first row produced by ORDER BY". DuckDB supports this and the smelt parser handles it; verify the diagnostics gate confirms registry coverage.
    - `ORDER BY device_id, event_count DESC, first_seen ASC` — the first column (`device_id`) is required by Postgres semantics so that the DISTINCT ON columns appear first in the ORDER BY. Verify.
    - LEFT-vs-INNER JOIN choice in the eventstream extension: `LEFT JOIN identity_backward_fill` is required (events from anonymous-only devices must be preserved). The expert should flag if this is silently degraded to INNER or rendered LEFT in a way that doesn't actually preserve rows.
    - Subsumption invariant in the eventstream test (step 6 of Phase 2's TDD list): every event with non-null `forward_only_user_id` must have non-null `backward_fill_user_id`. The expert should verify the test query expresses this correctly.
    - No parser-unsupported constructs (verified by `example_diagnostics` already, but the expert may catch a construct that parses but is semantically wrong on DuckDB).

If a literal `sql-expert` agent type does not exist, dispatch `general-purpose`
with a prompt that frames it as such (read the plan + diff, flag plan/impl
drift, missing test cases, scope creep into later phases — material findings
only).

**Loop discipline.**

1. **Round 1.** Dispatch `sql-expert` with `model: "sonnet"`. The prompt MUST include:
   - This plan's path and the oracle paths (overall plan, meta-plan, Phase 3 plan's "Deferred during implementation" section, Phase 4 plan's "Deferred during implementation" section, Phase 5 plan's "Deferred during implementation" section, `testing.md`).
   - The exact file scope from the per-sub-phase tables above.
   - The diff range to review: commits since the start of Phase 6 of the overall plan (typically the three `feat(examples): web_analytics ... (web-analytics Phase 6)` commits — `git log --oneline {phase-6-base}..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, scope creep, missing test cases, plan/impl drift, parser limitations hit). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` if the expert's tool palette allows nested subagents.

2. **Address findings.** For each material finding:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics`, and the `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests after each fix batch.
   - Commit per round: `review(web-analytics-6): address sql-expert feedback`.
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch `sql-expert` with the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → expert is clean and exits.

4. **Repeat** until clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - `sql-expert` flags a material finding on round 3 (per-expert bound).
   - An expert's findings would force a spec change. Run `/smelt:spec` on the relevant slug first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 6.

**Critical files (allowed to touch in this phase).** Anything within the
expert's scope per the table above, plus
`docs/plans/20260517-web-analytics-6-backward-fill.md` (to record round counts
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

After acceptance gate: flip the overall-plan status row for Phase 6 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings:
`review(web-analytics-6): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-6): mark Phase 6 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the scope is satisfied at the end of Phase 4:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — no regression in the workspace.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `cargo test -p smelt-datagen --test example_web_analytics` passes — including the new `test_identity_backward_fill_materializes`, `test_eventstream_with_identity_includes_backward_fill`, and `test_backward_fill_invariants_inline_pass` sub-tests.
- Manual fresh-checkout dry run succeeds:
  ```bash
  smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01
  duckdb examples/web_analytics/target/dev.duckdb < examples/web_analytics/setup_sources.sql
  smelt build --project-dir examples/web_analytics --target dev
  smelt test --project-dir examples/web_analytics --select backward_fill_resolution
  ```
- Phase 4 acceptance gate met: `sql-expert` reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 6 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
