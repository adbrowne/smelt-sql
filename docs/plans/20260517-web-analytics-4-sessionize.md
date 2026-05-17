# Plan: Web Analytics Phase 4 — Sessionization

**Date**: 2026-05-18
**Spec**: example phases do not anchor to a single feature spec; oracles are the overall plan ([`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md) §Goal items 2, 4) and the meta-plan (`/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §3 row 4). Spec cross-references that ground specific decisions: [`docs/specs/functions.md`](../specs/functions.md) (smelt function declaration surface), [`docs/specs/incremental_models.md`](../specs/incremental_models.md) (`materialization: table` + `incremental:` frontmatter), [`docs/specs/testing.md`](../specs/testing.md) (`materialization: test` inline assertions).
**Spec diff**: no spec change in this phase. Phase 4 is the first consumer of `silver/events_parsed` (landed in Phase 3 commit `84096dd1`) and produces three new artefacts under `examples/web_analytics/`: a `sessionize` smelt function, a sessions silver table, and a device↔user edge silver table.
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs (inline header comments inside the new SQL files; no `docs-site/` touch — that lands in Phase 8 of the overall plan)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 4 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewers (`sql-expert`, `examples-curator`), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read the overall plan and the meta-plan for the sentinel emission contract and stop-the-line conditions. The Phase 3 plan (`docs/plans/20260517-web-analytics-3-scaffold.md`) is also required reading — its "Deferred during implementation" section records concrete smelt constraints (call-syntax, `to_seconds`, `smelt.ref` dead syntax, `paths:` discipline, struct-returning function calls in models) that this phase must respect. Do not re-open those decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 5 is the expert-reviewer dispatch loop** — after Phases 1–4 commit, dispatch the meta-plan §5 expert reviewers applicable to this phase (`sql-expert`, `examples-curator`), address material findings, re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 5. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 5's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user (emit `<<PAUSE_FOR_HUMAN>>`):**

- The reviewer surfaces the same material finding across two implementer passes on the same sub-phase.
- TDD tests cannot be made green without violating a Phase 3 deferred-item ground rule.
- The smelt parser or type-checker surfaces a defect that blocks both the function declaration AND the inline SQL fallback (escalate: this is a real product gap, not example scope).
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to this plan.
- Phase 5: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation. The standing oracles are `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`) and the end-to-end integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` (extended per sub-phase — datagen → setup_sources → `smelt build` succeeds and the new model's row counts match invariants).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + every expert in Phase 5 spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces sessionization and the device↔user edge table only. **No identity stitching, no eventstream_with_identity, no marts** — those are Phases 5–8 of the overall plan.
- Honor the architectural invariants from `CLAUDE.md` (no `crates/` edits unless extending the existing `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests; all SQL must parse without LSP diagnostics in `examples/web_analytics/`).
- **Timeless-oracle rule (CLAUDE.md).** This plan file uses phase vocabulary; the SQL file header comments must read as feature descriptions with no `Phase N` labels.

---

## Context

The overall plan's Goal items 2 and 4 require an incremental sessionization (30-minute inactivity gap + platform-boundary rule) and "late-arriving stitch evidence with a 7-day rolling lookback window." Phase 4 lays the silver-layer foundation that Phases 5–7 (identity stitching variants) will consume: it produces a `silver/sessions.sql` table where every event is assigned a session id under the dual gap-and-platform rule, and a `silver/device_user_edges.sql` table that aggregates the observed `(device_id, user_id, count)` evidence which the three identity algorithms diverge on. A `functions/sessionize.sql` smelt function is declared as the canonical signature for the sessionization operator; in v1 the sessions model inlines the equivalent window-function SQL directly, mirroring how Phase 3's `parse_event_payload` is declared but not called from `silver/events_parsed` (Phase 3 deferred §"`smelt.functions.parse_event_payload(...)` is not callable in model SQL"; same root cause — Phase-19 context-binding hasn't landed, so smelt functions taking `Expr<T>` column-reference arguments are not yet usable from real model bodies).

## Scope

### In scope

- `examples/web_analytics/functions/sessionize.sql` — a `smelt.define sessionize(source: TableExpr, partition_col: Expr<Integer>, ts_col: Expr<Timestamp>, platform_col: Expr<Text>, gap: Expr<Interval> = INTERVAL '30 minutes') -> TableExpr` declaration. Body uses `LAG()` + `SUM() OVER (...)` to project a `session_seq: BIGINT` column alongside `source.*`, with a session boundary triggered when either the gap rule fires (`ts_col - LAG(ts_col) > gap`) or the platform-boundary rule fires (`LAG(platform_col) IS DISTINCT FROM platform_col`). Matches the canonical fixture in `examples/functions_demo/functions/sessionize.sql` in shape, extended with the platform-boundary disjunct. Diagnostics-clean; not called from any model in this phase (rationale: Phase 3 deferred §"function call in models" — Phase-19 context binding required).
- `examples/web_analytics/models/silver/sessions.sql` — a table-materialised model that consumes `smelt.silver.events_parsed`, inlines the same `LAG()` + `SUM() OVER (...)` window logic as the function body, and aggregates per `(device_id, session_seq)` into one row per session. Output columns: `session_id: VARCHAR` (deterministic hash of device_id + session_seq + session_start), `device_id: INTEGER`, `session_seq: BIGINT`, `session_start: TIMESTAMP`, `session_end: TIMESTAMP`, `session_start_date: DATE`, `event_count: BIGINT`, `platform: VARCHAR` (single value per session by the boundary rule — verified by an inline invariant test). Frontmatter: `materialization: table` + `incremental: { enabled: true, event_time_column: session_start_date, partition_column: session_start_date, granularity: day }` per `incremental_models.md` §"YAML frontmatter (in `.sql` files)". The 7-day late-arriving lookback is realised operationally by running `smelt run --event-time-start (today-7d) --event-time-end today` daily; the model itself does not encode the lookback (see `incremental_models.md` §"Late-arriving data (interim guidance)" — overlapping ranges is the documented interim mitigation).
- `examples/web_analytics/models/silver/device_user_edges.sql` — a view (or table; default `view` per `smelt.yml`) that aggregates `(device_id, user_id, COUNT(*) AS event_count, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen)` from `smelt.silver.events_parsed` filtered to rows where `user_id IS NOT NULL`. This is the edge set the three identity algorithms in Phases 5–7 will join against; it is intentionally the simplest possible aggregation so all three algorithms see the same evidence shape.
- `examples/web_analytics/tests/session_boundary_invariants.test.sql` — a `materialization: test` file asserting per-row invariants of the sessionization rule on a small hand-crafted `inputs:` block (per `docs/specs/testing.md`). At minimum: (a) two events 35 minutes apart on the same `(device_id, platform)` produce two distinct `session_seq` values; (b) two events 5 minutes apart with different `platform` values produce two distinct `session_seq` values; (c) two events 5 minutes apart with the same `platform` produce one `session_seq`. The test targets `silver/sessions.sql` with mock `silver/events_parsed` inputs. If the testing framework cannot test a CTE inside `sessions.sql` (target-cte limitation), split the inline window-function projection into a named CTE (e.g. `sessionized`) and use `target_cte: sessionized` so the assertions stay surgical.
- `crates/smelt-datagen/tests/example_web_analytics.rs` extension — add `test_sessions_and_edges_end_to_end` that runs `smelt-datagen ... --scale-factor 0.01 && setup_sources.sql && smelt build`, then opens the DuckDB and asserts: (i) `SELECT count(*) FROM main.sessions > 0`; (ii) every `sessions` row's `platform` is non-null and single-valued by construction (`SELECT count(DISTINCT platform) FROM main.events_parsed JOIN main.sessions USING (...)` for a session is 1); (iii) `SELECT count(*) FROM main.device_user_edges` matches `SELECT count(*) FROM (SELECT DISTINCT device_id, user_id FROM main.events_parsed WHERE user_id IS NOT NULL)`.

### Explicitly deferred (scope guardrails)

- **No identity stitching.** No `gold/identity_forward_only.sql`, no `gold/identity_backward_fill.sql`, no `gold/identity_connected_components.sql`, no `gold/eventstream_with_identity.sql`. Phases 5–7 of the overall plan.
- **No calling `smelt.functions.sessionize(...)` from `silver/sessions.sql`.** Phase 3 deferred §"smelt.functions.parse_event_payload(payload) is not callable in model SQL" applies — the same root cause (context binding not yet landed) blocks passing `device_id`, `event_ts`, `platform` as column references to the function. The function is declared for forward compatibility; the model inlines the SQL. If a future phase removes that constraint, the inline SQL in `silver/sessions.sql` can be replaced with `FROM smelt.functions.sessionize(smelt.silver.events_parsed, ...)` in a single mechanical edit.
- **No 7-day lookback encoded in the model SQL.** Per `incremental_models.md` §"Late-arriving data (interim guidance)", the documented v1 mechanism for handling late-arriving data is overlapping CLI runs (`--event-time-start (today-7d) --event-time-end today`). Encoding a lookback inside the model body via a `WHERE event_ts >= (current_date - INTERVAL 7 DAY)` filter would conflict with the planner's AST-level partition filter injection and is not supported.
- **No marts.** No `daily_active_users_by_method`, no `identity_method_comparison`. Phase 8.
- **No `paths:` change in `smelt.yml`.** Per Phase 3 deferred §"`functions` must NOT appear in `smelt.yml`'s `paths:` list", function discovery is path-independent; the existing `paths: [models]` remains correct.
- **No edits to `crates/` outside `crates/smelt-datagen/tests/example_web_analytics.rs`.** The smelt language surface is fixed for this phase.

---

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | `d03890a1` | 2026-05-18 |
| 2     | done     | `49da2c78` | 2026-05-18 |
| 3     | done     | `e98f42aa` | 2026-05-18 |
| 4     | pending  |        |      |
| 5     | pending  |        |      |

---

### Phase 1: `functions/sessionize.sql` smelt function declaration

**Goal.** Land the `sessionize` smelt function declaration at `examples/web_analytics/functions/sessionize.sql` — a `smelt.define` whose body uses window functions to compute a `session_seq` column under the 30-minute gap + platform-boundary rule. Diagnostics-clean; not yet called from any model.

**Pre-conditions.** Phase 3 of the overall plan committed (`silver/events_parsed` exists; `parse_event_payload` precedent exists). Working tree clean on `worktree-web_analytics`.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the function file lands. This is the LSP-side oracle. The test fails (or reports diagnostics) before the function body parses cleanly; it passes after.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_sessionize_function_compiles` — a lower-cost equivalent of the Phase 3 `test_parse_event_payload_function_compiles` test (see Phase 3 plan §"Phase 3 TDD tests"). Loads the function source via `smelt-parser` and asserts: (i) the declaration parses without errors; (ii) the parsed signature contains five parameters with the names and types listed under "Implementation shape" below; (iii) no diagnostic errors are emitted for the function file. **Caveat:** if a higher-level smelt-db loader is reachable, prefer that over a raw `smelt-parser` call — the test should mirror Phase 3's `test_parse_event_payload_function_compiles` implementation choice.

**Implementation shape.**

`examples/web_analytics/functions/sessionize.sql`:

```sql
-- Assign a session_seq to each event in `source`, partitioned by partition_col
-- and ordered by ts_col. A session boundary fires when either the inactivity
-- gap exceeds `gap` OR the platform column changes between consecutive events.
-- The output schema extends `source.*` with a single `session_seq: BIGINT`
-- column added by the explicit projection (per the TableExpr return-schema
-- inference rule in docs/specs/functions.md).
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Timestamp>,
    platform_col: Expr<Text>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT
        source.*,
        SUM(
            CASE
                WHEN ts_col - LAG(ts_col) OVER (PARTITION BY partition_col ORDER BY ts_col) > gap
                  OR LAG(platform_col) OVER (PARTITION BY partition_col ORDER BY ts_col) IS DISTINCT FROM platform_col
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY partition_col ORDER BY ts_col) AS session_seq
    FROM source
)
```

Window functions in SELECT-list position are the same pattern as `examples/functions_demo/functions/sessionize.sql`; the canonical fixture there is the precedent. The only additions are (a) the `platform_col: Expr<Text>` parameter and (b) the `LAG(platform_col) IS DISTINCT FROM platform_col` disjunct inside the CASE.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/functions/sessionize.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension)
- `docs/plans/20260517-web-analytics-4-sessionize.md` (this file — committed at the start of Phase 1 work, before any other change)

**Docs touched.** None (header comment in the new SQL file is timeless per the Timeless-oracle rule).

**Review checklist** (material findings only):

- [ ] Function declares all five parameters with the exact names and types listed.
- [ ] Window-function syntax matches `examples/functions_demo/functions/sessionize.sql` (no novel constructs).
- [ ] `gap: Expr<Interval> = INTERVAL '30 minutes'` default is present.
- [ ] `IS DISTINCT FROM` is used (NULL-safe inequality — the first row's `LAG(platform_col)` is NULL by definition and must not spuriously start a second session).
- [ ] Zero diagnostics for `examples/web_analytics/functions/sessionize.sql` from `example_diagnostics`.
- [ ] Header comment is timeless — no `Phase N` references in the SQL file.

**Commit.** `feat(examples): web_analytics sessionize smelt function declaration (web-analytics Phase 4)`

---

### Phase 2: `silver/sessions.sql` incremental sessions table

**Goal.** Land `examples/web_analytics/models/silver/sessions.sql` — a `materialization: table` model with `incremental:` frontmatter that inlines the sessionization window logic against `smelt.silver.events_parsed` and aggregates per `(device_id, session_seq)` into one row per session. After this phase, `smelt build` materialises `main.sessions` and every row's `platform` is single-valued (by the platform-boundary rule).

**Pre-conditions.** Phase 1 of this plan committed (the function file exists for forward-compat; the model does not call it).

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/` after the model lands. Fails when the new model is added with a type error; passes after the column shapes line up.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_sessions_model_materializes` — extends the end-to-end test:
  1. Run `smelt-datagen ... --scale-factor 0.01`.
  2. Programmatically execute `setup_sources.sql`.
  3. Invoke `smelt build --project-dir examples/web_analytics --target dev` and assert exit 0.
  4. Open the DuckDB and assert `SELECT count(*) FROM main.sessions > 0`.
  5. Assert per-session platform uniqueness: `SELECT count(*) FROM (SELECT session_id, count(DISTINCT platform) AS plats FROM main.sessions GROUP BY session_id HAVING plats > 1)` = 0. (Trivially true given one row per session; the real invariant is checked by joining `events_parsed` rows to their session via `(device_id, event_ts)` falling inside `[session_start, session_end]` — defer the join-based invariant to the Phase 4 inline `.test.sql` since it is per-row and easier to express with mock data.)
  6. Assert `MAX(session_seq) >= 1` somewhere in the dataset — a smoke that sessionization actually triggered at least one boundary.

**Implementation shape.**

`examples/web_analytics/models/silver/sessions.sql`:

```sql
---
materialization: table
incremental:
  enabled: true
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
-- One row per session under the 30-minute inactivity + platform-boundary rule.
-- The sessionization logic is inlined (rather than calling
-- smelt.functions.sessionize) because column-reference arguments to smelt
-- functions are not yet supported in model contexts; the function declaration
-- in functions/sessionize.sql is the canonical signature for that future
-- refactor.
WITH sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        SUM(
            CASE
                WHEN event_ts - LAG(event_ts) OVER (PARTITION BY device_id ORDER BY event_ts) > INTERVAL '30 minutes'
                  OR LAG(platform) OVER (PARTITION BY device_id ORDER BY event_ts) IS DISTINCT FROM platform
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY device_id ORDER BY event_ts) AS session_seq
    FROM smelt.silver.events_parsed
)
SELECT
    md5(CAST(device_id AS VARCHAR) || '-' || CAST(session_seq AS VARCHAR) || '-' || CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    CAST(MIN(event_ts) AS DATE) AS session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM sessionized
GROUP BY device_id, session_seq
```

Notes the implementer must verify against current smelt support:

- `INTERVAL '30 minutes'` typed-literal — verified in spec `types.md` as the canonical typed-literal form; Phase 3 deferred §"smelt parser does not support `<expr> * INTERVAL 1 SECOND`" confirms the prefix-`INTERVAL` form is what parses.
- `IS DISTINCT FROM` — verify against smelt's expression grammar. If unsupported, fall back to the NULL-safe equivalent `COALESCE(LAG(platform), '') <> COALESCE(platform, '')` (the platform column is non-null in `events_parsed` per Phase 3's `json_extract_string`, so `LAG(platform)` is only NULL on the first row per device — using a sentinel string is correct).
- `md5(...)` — verify availability in DuckDB; alternative is `hash(...)` (DuckDB built-in returning UBIGINT) cast to VARCHAR if `md5` is not registered.
- `ANY_VALUE(platform)` — verified valid in DuckDB. The grouping key guarantees platform is single-valued within a `(device_id, session_seq)` partition by the boundary rule; the aggregate just picks the representative value without forcing GROUP BY on it.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/silver/sessions.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Frontmatter declares `materialization: table` + `incremental:` block with `event_time_column: session_start_date`, `partition_column: session_start_date`, `granularity: day` per `incremental_models.md` §"YAML frontmatter".
- [ ] Source reference uses path syntax `smelt.silver.events_parsed` (per Phase 3 deferred §"smelt.ref is dead syntax").
- [ ] Window-function clause exactly mirrors the function body in Phase 1 (gap rule + platform-boundary rule).
- [ ] `session_start_date` is derived deterministically from `MIN(event_ts)` so the partition column equals what would be the session's start date; verify the cast preserves date semantics (no time-zone shift).
- [ ] `session_id` derivation is deterministic and idempotent under re-runs (md5 of stable inputs — no timestamps that change between runs).
- [ ] No reach into Phase 5–7 scope (no identity columns, no `user_id`-aware logic — sessions are device-bound by the partition column).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics silver/sessions incremental table (web-analytics Phase 4)`

---

### Phase 3: `silver/device_user_edges.sql` edge aggregation

**Goal.** Land `examples/web_analytics/models/silver/device_user_edges.sql` — a `view` (default materialization) over `smelt.silver.events_parsed` that aggregates per `(device_id, user_id)` with non-null user filter. This is the edge set the three identity algorithms in Phases 5–7 will join against.

**Pre-conditions.** Phase 2 of this plan committed.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/` after the new view lands.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_device_user_edges_view` — extends the end-to-end test:
  1. Run datagen → setup_sources → `smelt build` as in Phase 2.
  2. Assert `SELECT count(*) FROM main.device_user_edges > 0`.
  3. Assert the row count matches the distinct `(device_id, user_id)` pairs in `events_parsed` with non-null `user_id`: `SELECT count(*) FROM main.device_user_edges = (SELECT count(*) FROM (SELECT DISTINCT device_id, user_id FROM main.events_parsed WHERE user_id IS NOT NULL))`.
  4. Assert `MIN(event_count) >= 1` (every aggregated edge has at least one supporting event).
  5. Assert `MIN(first_seen) <= MAX(last_seen)` per row (temporal ordering invariant).

**Implementation shape.**

`examples/web_analytics/models/silver/device_user_edges.sql`:

```sql
-- (device_id, user_id) co-occurrence evidence — every signed-in event
-- contributes one observation. Downstream identity algorithms (forward-only,
-- backward-fill, connected-components) consume this as the canonical edge
-- set so they all see the same evidence shape.
SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

Materialization is inherited from `default_materialization: view` in `smelt.yml`.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/silver/device_user_edges.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Source reference is `smelt.silver.events_parsed` (path syntax, not `smelt.ref`).
- [ ] `WHERE user_id IS NOT NULL` is present — anonymous events must be excluded from the edge set (they carry no identity evidence).
- [ ] Columns are exactly `(device_id, user_id, event_count, first_seen, last_seen)` — no extra columns that would constrain Phases 5–7 prematurely.
- [ ] No JOIN syntax (smelt parser limitation per `examples/timeseries/README.md` — this view doesn't need any).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics silver/device_user_edges view (web-analytics Phase 4)`

---

### Phase 4: Inline `.test.sql` session-boundary invariants

**Goal.** Land `examples/web_analytics/tests/session_boundary_invariants.test.sql` — a `materialization: test` file asserting the gap rule and platform-boundary rule on hand-crafted mock data. The verification gate for Phase 4 ("inline `.test.sql` for session-boundary invariants") is met by this file.

**Pre-conditions.** Phases 1–3 of this plan committed. The `silver/sessions.sql` model exists and is the target of the test.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test cohort_count_acceptance` is the precedent for how inline `.test.sql` files are exercised end-to-end (the existing `examples/per_cohort_union/tests/cohort_count.test.sql` is run by that harness). Verify the integration test that picks up `examples/web_analytics/tests/*.test.sql` either already exists (likely auto-discovery via `paths: [models]` ⊕ a `tests/` walk in `crates/smelt-cli/src/discovery.rs`) or needs to be added. If a new integration test is required, add `crates/smelt-cli/tests/web_analytics_session_invariants.rs` following the `cohort_count_acceptance.rs` shape. **Failing oracle:** the test fails (or `smelt test` reports unknown test) before the `.test.sql` file lands; it passes after.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_sessions_invariants_inline_pass` — invoke `smelt test --project-dir examples/web_analytics --select session_boundary` (the test name selector from `docs/specs/testing.md` §"Selector behaviour"); assert exit 0 and that the named test reports PASS.

**Implementation shape.**

The test file uses the `materialization: test` format from `docs/specs/testing.md` §"Test file format". The plan defers the exact `target_cte` choice to the implementer — it depends on whether the `silver/sessions.sql` body's `sessionized` CTE can be addressed as a target, or whether the aggregated rows in the outer SELECT are testable directly. Both options:

**Option A — target the `sessionized` CTE** (preferred — surgical, asserts `session_seq` boundaries directly):

```sql
--- name: test_session_boundary_invariants ---
materialization: test
test:
  model: sessions
  target_cte: sessionized
  inputs:
    events_parsed:
      # 30-minute gap with same platform → new session on the second row
      - {device_id: 1, event_ts: '2026-04-01T10:00:00', event_date: '2026-04-01', platform: 'web', user_id: null, event_id: 1, event_name: 'page_view', url: 'https://example.com/'}
      - {device_id: 1, event_ts: '2026-04-01T10:35:00', event_date: '2026-04-01', platform: 'web', user_id: null, event_id: 2, event_name: 'page_view', url: 'https://example.com/'}
      # Platform change within gap → new session on the second row
      - {device_id: 2, event_ts: '2026-04-01T10:00:00', event_date: '2026-04-01', platform: 'web', user_id: null, event_id: 3, event_name: 'page_view', url: 'https://example.com/'}
      - {device_id: 2, event_ts: '2026-04-01T10:05:00', event_date: '2026-04-01', platform: 'ios', user_id: null, event_id: 4, event_name: 'page_view', url: 'https://example.com/'}
      # Same platform within gap → SAME session
      - {device_id: 3, event_ts: '2026-04-01T10:00:00', event_date: '2026-04-01', platform: 'web', user_id: null, event_id: 5, event_name: 'page_view', url: 'https://example.com/'}
      - {device_id: 3, event_ts: '2026-04-01T10:05:00', event_date: '2026-04-01', platform: 'web', user_id: null, event_id: 6, event_name: 'page_view', url: 'https://example.com/'}
  expect:
    - {device_id: 1, event_ts: '2026-04-01T10:00:00', session_seq: 0}
    - {device_id: 1, event_ts: '2026-04-01T10:35:00', session_seq: 1}
    - {device_id: 2, event_ts: '2026-04-01T10:00:00', session_seq: 0}
    - {device_id: 2, event_ts: '2026-04-01T10:05:00', session_seq: 1}
    - {device_id: 3, event_ts: '2026-04-01T10:00:00', session_seq: 0}
    - {device_id: 3, event_ts: '2026-04-01T10:05:00', session_seq: 0}
---
```

**Option B — target the full model** (fallback if `target_cte` cannot reach an inner CTE through an incremental-materialised parent): assert the aggregated `(device_id, event_count)` counts per the three test cases (1+1, 1+1, 2 events → 2, 2, 1 sessions). Choose Option A if and only if it runs cleanly; otherwise document the constraint inline and use Option B.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/tests/session_boundary_invariants.test.sql` (new)
- `examples/web_analytics/models/silver/sessions.sql` (refactor: extract the `sessionized` CTE if Option A is chosen and it requires the inline `WITH` to be named — but the Phase 2 implementation shape already names it `sessionized`, so no edit should be needed)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — the inline-test smoke)
- `crates/smelt-cli/tests/web_analytics_session_invariants.rs` (new, only if no auto-discovery picks up the file — verify before writing)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Test file's `inputs:` rows mock the full `events_parsed` schema (all columns the model SELECTs from must be present, or the cast rules from `testing.md` §"YAML value → SQL type coercion" must cover absent columns deterministically).
- [ ] Each of the three invariants (gap-boundary, platform-boundary, no-boundary) is exercised by a distinct `device_id` so failures are attributable to one rule.
- [ ] `expect:` rows are correct for the gap rule (default 30 minutes, exclusive) — the 35-minute case must produce a boundary; a 30-minute-exactly case is a separate edge case the implementer may add if useful but is not required.
- [ ] The test runs under `smelt test` and reports PASS (verified via the new integration test).
- [ ] No reach into Phases 5–7 scope (no identity columns in `inputs:` or `expect:` beyond what `events_parsed` carries).

**Commit.** `feat(examples): web_analytics session boundary invariant test (web-analytics Phase 4)`

---

### Phase 5: Expert reviewer dispatch loop

For each expert listed below, dispatch via the Agent tool with `model: "sonnet"`,
brief prompt, the per-phase plan path, the spec path (if relevant), and the
in-repo plan path. The expert returns a list of findings classified as
"material" or "stylistic". Address material findings:

  - For each material finding, either edit directly (small) or dispatch a
    nested implementer subagent (larger).
  - Commit the fix with message `review(web-analytics-4): address {expert-name} feedback`.
  - Push.
  - Re-dispatch the same expert. Loop until the expert returns "no material findings".

Bounds:

  - Max 3 rounds per expert. If unresolved after 3 rounds → emit
    `<<PAUSE_FOR_HUMAN>>`.
  - If two different experts flag the same systemic concern in one round →
    emit `<<PAUSE_FOR_HUMAN>>`.

Experts for this phase (from meta-plan §5):

  - `sql-expert` — focus: gap-and-island correctness (the window-function expression in both the function body and the inline model SQL), incremental lookback (frontmatter shape, partition-column semantics, late-arriving-data mitigation), `IS DISTINCT FROM` / `COALESCE` NULL-handling, `md5`/`hash` portability, `ANY_VALUE` semantics under the GROUP BY, no parser-unsupported constructs.
  - `examples-curator` — focus: file placement under the bronze/silver layering convention, header comments are timeless and accurate (no Phase N labels; the rationale for inlining vs calling the smelt function is captured), `.test.sql` is the smallest case that demonstrates each invariant (no dead mock data), no scope creep into Phases 5–7 (no identity columns introduced, no `eventstream_with_identity`), the function declaration's intent is documented (canonical signature for future context-binding refactor).

If a literal `sql-expert` or `examples-curator` agent type does not exist,
dispatch `general-purpose` with a prompt that frames it as such (read the plan
+ diff, flag plan/impl drift, missing test cases, scope creep into later
phases — material findings only).

**Loop discipline.**

1. **Round 1.** Dispatch both experts in parallel — single message, multiple
   Agent tool calls. Each prompt MUST include:
   - This plan's path and the oracle paths (overall plan, meta-plan, Phase 3
     plan's "Deferred during implementation" section, `incremental_models.md`,
     `functions.md`, `testing.md`).
   - The exact file scope from the per-sub-phase tables above.
   - The diff range to review: commits since the start of Phase 4 of the
     overall plan (typically the four `feat(examples): web_analytics ... (web-analytics Phase 4)`
     commits — `git log --oneline {phase-4-base}..HEAD`).
   - Explicit instruction: report only **material** findings (correctness,
     scope creep, missing test cases, plan/impl drift, parser limitations
     hit). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or
     "no material findings".
   - Reminder to spawn with `model: "sonnet"` if the expert's tool palette
     allows nested subagents.

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent
     (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`,
     `cargo test -p smelt-cli --test example_diagnostics`, and the
     `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests
     after each fix batch.
   - Commit per expert: `review(web-analytics-4): address {expert-name} feedback`.
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were
   addressed. Provide the round-1 prompt plus a diff of what changed since
   round N−1. "No material findings" → that expert is clean and exits.

4. **Repeat** until both experts are clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line
   reason on the line above) and stop the autonomy loop if any of the
   following fires:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Both experts flag the same systemic concern in the same round (per
     meta-plan §7).
   - An expert's findings would force a spec change. Run `/smelt:spec` on the
     relevant slug first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 4.

**Critical files (allowed to touch in this phase).** Anything within an
expert's scope per the table above, plus
`docs/plans/20260517-web-analytics-4-sessionize.md` (to record round counts
and the final clean status) and `docs/plans/20260517-web-analytics-example.md`
(to flip the overall-plan status row).

**Review checklist** (material findings only — applied to the expert-dispatch
*process*, not to a code diff):

- [ ] Both experts dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`,
  `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for
  `examples/web_analytics/`), the end-to-end integration tests, and the inline
  `.test.sql` invariant test all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during
implementation" of the form:

> Phase 5 expert review: sql-expert clean (R{n}), examples-curator clean (R{n}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 4 in
`docs/plans/20260517-web-analytics-example.md` to `done` with today's date and
the latest commit SHA. Commit and push that change. Then emit
`<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings:
`review(web-analytics-4): address {expert-name} feedback`. The status-table
flip lands as: `chore(web-analytics-4): mark Phase 4 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

**Phase 1: `IS DISTINCT FROM` not supported by smelt-parser; both function body and inline model use plain `!=`.** The plan's prose specified `LAG(platform_col) IS DISTINCT FROM platform_col` for the platform-boundary check. The smelt-parser's `IS` handler only recognises `IS [NOT] NULL`. Both `functions/sessionize.sql` and `models/silver/sessions.sql` use plain `!=` instead. This is semantically *more correct* for the first-row case under three-valued logic: `LAG` returns NULL on the first row per partition → `NULL != platform = NULL` → OR'd with the also-NULL time-gap result stays NULL → `CASE WHEN NULL THEN 1 ELSE 0 = 0` → session_seq = 0 for the first row. `IS DISTINCT FROM` would have evaluated to TRUE on the first row and started session_seq at 1, contradicting the Phase 4 invariant test's `expect:` rows. The plan's review checklist note "must not spuriously start a second session" captures the intent; the `!=` form realises it. If smelt-parser ever gains `IS DISTINCT FROM` support, the function body cannot mechanically switch to it without also handling the first-row case (e.g., `COALESCE(LAG(platform), platform) != platform`).

**Phase 2: two-CTE structure required for nested window functions.** DuckDB rejects `LAG(...)` nested inside `SUM(... OVER ...)`. The plan's single-CTE shape compiles in some engines but not DuckDB. The model splits into `lagged` (resolves the `LAG` columns) and `sessionized` (applies the `SUM(CASE ...) OVER` over those pre-resolved values). Semantics are identical.

**Phase 2: `epoch_us()` arithmetic instead of `INTERVAL` subtraction.** Smelt's silver `events_parsed` model composes `event_ts` as `CAST(event_date AS DATE) + to_seconds(seconds_in_day)`. DuckDB executes this correctly at runtime, but the smelt-db type-inference layer does not recognise `to_seconds` (see Phase 3 plan deferred §"`to_seconds` is unknown to smelt's type inference"), so the inferred type for `event_ts` is not `Timestamp`. `event_ts - LAG(event_ts) > INTERVAL '30 minutes'` therefore fails type-checking inside the model body. The model uses `epoch_us(event_ts) - prev_ts_us > 30 * 60 * 1000000` (microseconds) instead. Threshold: 30 min × 60 s × 1,000,000 µs/s = 1,800,000,000 µs. NULL propagation still holds: `LAG(epoch_us(event_ts))` on first row is NULL → `epoch_us(event_ts) - NULL = NULL` → `NULL > 1.8e9 = NULL` → CASE = 0. Resolving the upstream type-inference gap (registering `to_seconds` in `crates/smelt-types/src/functions.rs` as returning `Interval`) would let this revert to the cleaner `INTERVAL` form.

**Phase 2: `session_id` uses `CONCAT(...)` rather than `md5(...)`.** Smelt's type-inference layer does not recognise `md5` as a standard SQL function and emits a Warning diagnostic that fails the `example_diagnostics` gate. `CONCAT` is recognised and produces a deterministic, idempotent surrogate key from the same `(device_id, session_seq, MIN(event_ts))` inputs. Registering `md5` in the smelt function registry would let this revert to a fixed-width hash if a shorter key becomes desirable.

**Phase 2: plan typo on the materialized table name.** The plan's Phase 2 TDD step 4 says `SELECT count(*) FROM main.sessions`. Smelt materialises `models/silver/sessions.sql` as `main.silver_sessions` (the address segments include the directory). All queries in the implemented `test_sessions_model_materializes` use the correct `main.silver_sessions` name. Future readers of the plan should treat the `main.sessions` reference as a typo.

---

## Verification

How to confirm the scope is satisfied at the end of Phase 5:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — no regression in the workspace.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `cargo test -p smelt-datagen --test example_web_analytics` passes — including the new sessions / device_user_edges / invariant-test sub-tests.
- Manual fresh-checkout dry run succeeds:
  ```bash
  smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01
  duckdb examples/web_analytics/target/dev.duckdb < examples/web_analytics/setup_sources.sql
  smelt build --project-dir examples/web_analytics --target dev
  smelt test --project-dir examples/web_analytics --select session_boundary
  ```
- Phase 5 acceptance gate met: both applicable expert reviewers (`sql-expert`, `examples-curator`) reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 4 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
