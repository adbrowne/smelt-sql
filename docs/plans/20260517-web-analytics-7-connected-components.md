# Plan: Web Analytics Phase 7 — `identity_connected_components` + extend `eventstream_with_identity`

**Date**: 2026-05-18
**Spec**: example phases do not anchor to a single feature spec; the oracle is the overall plan ([`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md) §Goal item 3 — three parallel identity algorithms surfaced side-by-side in one wide eventstream) and the meta-plan (`/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §3 row 7). Spec cross-references that ground specific decisions: [`docs/specs/testing.md`](../specs/testing.md) (`materialization: test` inline assertions, YAML coercion). DuckDB's recursive-CTE semantics are the runtime oracle; smelt-parser already accepts `WITH RECURSIVE` (`crates/smelt-parser/src/parser/select.rs:799`, `crates/smelt-parser/src/lexer.rs:458`) and smelt-db bootstraps recursive-CTE column types with `Unknown` before inferring the recursive body (`crates/smelt-db/src/type_inference/subquery.rs:66`).
**Spec diff**: no spec change in this phase. Phase 7 introduces the third gold-layer identity model — `identity_connected_components.sql` (label-propagation union-find over `(device, user)` edges via an iter-capped recursive CTE) — and adds two columns to the existing `gold/eventstream_with_identity.sql`: `connected_components_user_id` (the device's resolved canonical user under the connected-components algorithm) and `connected_components_cluster_id` (the cluster label, the smallest user_id in the cluster). Phase 8 completes the example with marts that quantify the algorithmic differences.
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs (inline header comments inside the new and modified SQL files; no `docs-site/` touch — that lands in Phase 8 of the overall plan).

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 7 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewers (`sql-expert` and `examples-curator`), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read the overall plan and the meta-plan for the sentinel emission contract and stop-the-line conditions. The Phase 3 (`docs/plans/20260517-web-analytics-3-scaffold.md`), Phase 4 (`docs/plans/20260517-web-analytics-4-sessionize.md`), Phase 5 (`docs/plans/20260517-web-analytics-5-forward-only.md`), and Phase 6 (`docs/plans/20260517-web-analytics-6-backward-fill.md`) "Deferred during implementation" sections are required reading — they record concrete smelt constraints this phase must respect:
   - call-syntax discipline (path syntax `smelt.<dir>.<stem>`; `smelt.ref()` is dead syntax);
   - `to_seconds` not in inference (use `epoch_us` arithmetic) — not directly relevant here because Phase 7 does not consume `event_ts`, but a downstream cost-modeling concern;
   - `IS DISTINCT FROM` not supported by smelt-parser — use `!=` or `OR ... IS NULL`;
   - `md5` not registered → use `CONCAT` for surrogate-key formation if needed (Phase 7's `cluster_id` is just `MIN(user_id)`, no hashing);
   - two-CTE structure required for nested window functions;
   - materialised model addresses include the directory segment (`main.gold_identity_connected_components`, `main.gold_eventstream_with_identity`);
   - `tests` must already appear in `smelt.yml` `paths:` (it does — `examples/web_analytics/smelt.yml:5`);
   - registering a function in `crates/smelt-types/src/signatures.rs` also requires lockstep edits in `crates/smelt-types/src/functions.rs` and `crates/smelt-db/src/type_inference/function_call.rs` — Phase 7 should **not** need any registry additions, but if a finding surfaces one, scope-check before doing it;
   - `WITH`-clause-merging fix in `crates/smelt-cli/src/test_compiler.rs` (Phase 4 deferred) supports models that begin with `WITH RECURSIVE` — the recursive CTE form is compatible with inline tests;
   - Timestamp coercion in `yaml_value_to_sql` (Phase 4 deferred) supports `YYYY-MM-DD HH:MM:SS` — not needed for this phase's inline test because the `silver_device_user_edges` mock does not require timestamps to drive the algorithm (only `(device_id, user_id, event_count, first_seen)` rows; first_seen is unused by connected-components and may be supplied as any valid TIMESTAMP literal for schema completeness).
   - Do not re-open those decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 4 is the expert-reviewer dispatch loop** — after Phases 1–3 commit, dispatch the meta-plan §5 expert reviewers applicable to this phase (`sql-expert` *and* `examples-curator` — Phase 7 of the overall plan has two listed experts per meta-plan §5 row 7). Address material findings, re-dispatch until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 4. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 4's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user (emit `<<PAUSE_FOR_HUMAN>>`):**

- The reviewer surfaces the same material finding across two implementer passes on the same sub-phase.
- TDD tests cannot be made green without violating a Phase 3 / Phase 4 / Phase 5 / Phase 6 deferred-item ground rule.
- The recursive CTE fails to terminate within the iter cap on the synthetic dataset (the iter cap is intentionally generous — 8 iterations cover at most a 256-fold growth in the largest component, which is far beyond the synthetic graph's expected diameter of 3–5). If termination fails, the cap or the algorithm needs review.
- The smelt parser or type-checker surfaces a defect that blocks the recursive-CTE form. Escalating means: the resolution pattern needs a parser/type-inference change beyond Phase 7 scope.
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to this plan.
- Phase 4 (expert dispatch): any single expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in one round (cross-expert bound).

**Conventions every phase:**

- Red-green TDD: failing test before any implementation. The standing oracles are `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`) and the end-to-end integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` (extended per sub-phase — datagen → setup_sources → `smelt build` succeeds and the new model's row counts match invariants).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + each Phase 4 expert spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces *only* `identity_connected_components` and the two-column extension of `eventstream_with_identity`. **No marts, no README polish, no `daily_active_users_by_method` mart** — those are Phase 8 of the overall plan.
- Honor architectural invariants from `CLAUDE.md` (no `crates/` edits unless extending the existing `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests). All SQL must parse without LSP diagnostics in `examples/web_analytics/`.
- **Timeless-oracle rule (CLAUDE.md).** This plan file uses phase vocabulary; SQL file header comments must read as feature descriptions with no `Phase N` labels.

---

## Context

The overall plan's Goal item 3 asks for three parallel identity-resolution algorithms surfaced side-by-side in one wide `eventstream_with_identity` row-per-event table. Phase 5 landed the first (`identity_forward_only`, within-session, single column) and the wide-table chassis. Phase 6 landed the second (`identity_backward_fill`, per-device canonical-user election, one new column). Phase 7 lands the third — `identity_connected_components`, the Amplitude-full algorithm — and extends the eventstream with the resolved-user column *and* the cluster-id column. The cluster id is the algorithm's distinguishing artefact (it identifies the union-find component, which is the unit that the algorithm groups across devices), and surfacing it makes the algorithm's behaviour observable row-by-row in the eventstream.

The connected-components algorithm answers a strictly wider question than backward-fill:

- **Forward-only** resolves identity *within* a session.
- **Backward-fill** resolves identity *across all sessions on a device* — but each device is independent. If users A and B both sign in on device 1, the device's canonical user is whichever has more events; the *other* user retains their own identity on every other device.
- **Connected-components** resolves identity *across devices*. If user A signs in on devices 1 and 2, and user B signs in on devices 2 and 3, then the algorithm clusters {A, B} together because edges `(d1, A)`, `(d2, A)`, `(d2, B)`, `(d3, B)` form a connected graph component through `d2`. Every event on `d1`, `d2`, or `d3` resolves to the same cluster representative (the smallest `user_id` in the cluster, by convention).

The expected algorithmic relationship at the mart level (Phase 8's verification gate) is:

```
count(distinct forward_only_user) ≤ count(distinct backward_fill_user) ≤ count(distinct connected_components_user)
```

on any day, because each algorithm subsumes the next: every connected-components cluster contains every device that backward-fill would have considered (and more), so any user resolved by backward-fill is also represented in the cluster. Connected-components further widens by transitive closure across devices.

The input is `silver/device_user_edges`, which the Phase 4 scaffold already exposes with `(device_id, user_id, event_count, first_seen, last_seen)` — one row per `(device, user)` pair. The natural graph for connected-components is bipartite: `device_id` and `user_id` are two disjoint node sets, with each edge `(d, u)` in `device_user_edges` connecting one device node to one user node. Cluster id can be defined as the smallest user_id in the cluster — equivalent under union-find.

The reduction proceeds in three logical passes:

1. **Seed labels.** Each user_id is initially its own cluster: `label = user_id`.
2. **Propagate.** A device's label is the MIN of its users' current labels; a user's label is the MIN of its devices' current labels. Iterate to a fixed point.
3. **Resolve.** After convergence, each device has a stable label that equals the smallest user_id in its connected component.

In a recursive CTE this is expressed as a fixed-point iteration with an iter cap. We pre-aggregate the edge set to `(device_id, user_id)` pairs (drop weights — connected-components only uses the bipartite graph). The recursive base case seeds device → MIN(user_id seen on that device). The recursive step joins the device→label table against the original edges, finds for each device the MIN over (its own current label, the labels of every user that has co-occurred with it on any device, computed transitively via a JOIN through the edges). The cap is 8 iterations: a 256-fold expansion of cluster diameter, which dominates the synthetic dataset's longest expected chain (3–5 hops at the 60/25/10/5 weights from Phase 2). Beyond the cap the model emits whatever the iter-cap convergence has reached — this is the documented v1 limitation, and Phase 9 (deferred / optional) replaces the cap with a true fixed-point.

The eventstream extension widens by one LEFT JOIN to the new model on `device_id` (one row per device — Cartesian join is impossible because `device_id` is the primary key of `identity_connected_components`). Existing columns and join shapes are preserved. The order of the new columns in the SELECT list immediately follows `backward_fill_user_id` so the row layout is `... session_id, forward_only_user_id, backward_fill_user_id, connected_components_user_id, connected_components_cluster_id`.

The model surfaces *two* columns rather than one because the cluster id is the algorithm's defining artefact:

- `connected_components_user_id` is a *resolved-user* column comparable to the other two algorithms' resolved-user columns: under connected-components, the device's canonical user is the smallest user_id in its cluster. This is what the Phase 8 mart will use for DAU.
- `connected_components_cluster_id` is the cluster label itself — by convention the smallest user_id in the cluster, so numerically identical to `connected_components_user_id`. They are surfaced as separate columns to make the algorithm's behaviour observable at the row level: a future Phase 9 fixed-point implementation will have `cluster_id` and `connected_components_user_id` continue to coincide *by definition*, while a probabilistic-stitching alternative (out of scope per the overall plan) might compute cluster id differently from chosen-user. Keeping them as two columns from the start avoids reshuffling the eventstream when those follow-ons land.

## Scope

### In scope

- `examples/web_analytics/models/gold/identity_connected_components.sql` — a `view` (default materialization) that selects from `smelt.silver.device_user_edges` and produces one row per device with `(device_id, connected_components_user_id, connected_components_cluster_id)`. The two output identity columns are numerically equal in v1 (both = smallest user_id in the cluster) but surfaced separately for the reasons above. Output columns: `device_id: INTEGER`, `connected_components_user_id: INTEGER` (non-nullable in the model output — every row of `device_user_edges` carries a non-null user_id by construction, so the propagation always yields a non-null cluster representative; devices that never had a signed-in event simply do not appear), `connected_components_cluster_id: INTEGER` (non-nullable). One row per device that appears in `silver/device_user_edges`.

- `examples/web_analytics/models/gold/eventstream_with_identity.sql` — edit (not replace): keep all existing columns and join shapes; add `LEFT JOIN smelt.gold.identity_connected_components c ON e.device_id = c.device_id` and add `c.connected_components_user_id` and `c.connected_components_cluster_id` to the SELECT list immediately after `b.backward_fill_user_id`. Update the header comment block to describe the new columns. Devices that never had a signed-in event yield `NULL` for both new columns via the LEFT JOIN — this is the row-by-row visible signature of the algorithm: an all-anonymous device's events stay NULL under connected-components (no graph node, no cluster).

- `examples/web_analytics/tests/connected_components_resolution_invariants.test.sql` — a `materialization: test` file (per `docs/specs/testing.md`) targeting `gold/identity_connected_components` with a mocked `silver_device_user_edges` `inputs:` block. At minimum, exercises **four** cluster-shape cases:
  - **Cluster 1 — single device, single user (degenerate).** One edge `(1, 100, ...)`. Device 1's cluster is `{100}`; resolved user is 100; cluster_id is 100.
  - **Cluster 2 — single device, two users (within-device co-occurrence).** Two edges `(2, 200, ...)` and `(2, 201, ...)`. Device 2 is connected to both users; the cluster is `{200, 201}` (joined through device 2). Resolved user = cluster_id = `MIN(200, 201) = 200`.
  - **Cluster 3 — two devices, one user each, joined through co-occurrence.** Edges `(3, 300, ...)`, `(3, 301, ...)`, `(4, 301, ...)`. Devices 3 and 4 are connected: user 301 appears on both devices, so devices 3 and 4 share the cluster `{300, 301}`. Cluster_id = `MIN(300, 301) = 300`. Both device 3 and device 4 resolve to user 300.
  - **Cluster 4 — three-device chain (transitive closure).** Edges `(5, 500, ...)`, `(5, 501, ...)`, `(6, 501, ...)`, `(6, 502, ...)`, `(7, 502, ...)`, `(7, 503, ...)`. Devices 5, 6, 7 are connected via users 501 (links 5↔6) and 502 (links 6↔7). Cluster = `{500, 501, 502, 503}`. Cluster_id = `MIN(500, 501, 502, 503) = 500`. All three devices resolve to user 500. This case stress-tests the recursive-propagation step — the cluster forms only after 2 propagation rounds, so a faulty iter cap of 1 would fail this test.
  - **Cluster 5 — isolated user retains identity (negative test).** Edge `(8, 600, ...)`. No overlap with any other device or user. Cluster = `{600}`. Resolved user = cluster_id = 600. This case ensures the algorithm does not spuriously merge isolated components.

  Because the target model produces one row per device, the test's `expect:` block has one row per device_id with the asserted `(connected_components_user_id, connected_components_cluster_id)`. Devices that never had a signed-in event are not in `silver/device_user_edges` and therefore not in the expected output (consistent with the overall scope: those devices' events resolve to NULL in `eventstream_with_identity` via the LEFT JOIN, exercised by the end-to-end integration test).

- `crates/smelt-datagen/tests/example_web_analytics.rs` extensions:
  - `test_identity_connected_components_materializes` — runs `smelt-datagen ... --scale-factor 0.01 && setup_sources.sql && smelt build`, then asserts: (i) `count(*) FROM main.gold_identity_connected_components > 0`; (ii) row count equals `count(DISTINCT device_id) FROM main.silver_device_user_edges` (one row per device that ever had a signed-in event); (iii) both `connected_components_user_id IS NOT NULL` and `connected_components_cluster_id IS NOT NULL` for every row (the model itself never yields NULL — NULL only enters via the LEFT JOIN downstream); (iv) cluster-id equality invariant: `connected_components_user_id = connected_components_cluster_id` on every row (the v1 algorithm sets both to the smallest user_id in the cluster — Phase 9 would relax this); (v) **transitive closure invariant**: for every pair of devices that share at least one user in `silver/device_user_edges` (i.e., `EXISTS (SELECT 1 FROM e1, e2 WHERE e1.device_id = d1 AND e2.device_id = d2 AND e1.user_id = e2.user_id)` for any user), the two devices have the same `connected_components_cluster_id`; (vi) **cluster-id is the MIN(user_id) over the cluster**: for every device d with cluster c, `c.connected_components_cluster_id = MIN(user_id over all edges whose device is in the cluster)`.
  - `test_eventstream_with_identity_includes_connected_components` — extends the existing eventstream end-to-end test: (i) both `connected_components_user_id` and `connected_components_cluster_id` columns exist in `main.gold_eventstream_with_identity` (probed by `SELECT connected_components_user_id, connected_components_cluster_id FROM ... LIMIT 1`); (ii) event-preserving cardinality unchanged (one row per `silver_events_parsed` row); (iii) every event whose `device_id` appears in `gold_identity_connected_components` has non-null `connected_components_user_id` and `connected_components_cluster_id` in `eventstream_with_identity` (LEFT-JOIN-on-`device_id` population correctness); (iv) within any single device, `connected_components_user_id` is single-valued AND `connected_components_cluster_id` is single-valued (the per-device output is propagated to every event on that device); (v) the connected-components subsumption invariant: every event with a non-null `backward_fill_user_id` also has a non-null `connected_components_user_id` (the backward-fill device-set is a subset of the connected-components device-set, because every device with at least one edge appears in both).

### Explicitly deferred (scope guardrails)

- **No true fixed-point.** The recursive CTE uses an iter cap of 8. Phase 9 of the overall plan (optional) replaces the cap with a true fixed-point. Until then, the algorithm is exact on graphs whose longest path is ≤ 8 nodes — covers the synthetic dataset with multi-order-of-magnitude headroom.
- **No marts.** No `daily_active_users_by_method`, no `identity_method_comparison`. Phase 8 of the overall plan.
- **No README polish.** The Phase 3 README stub stays as is. Phase 8 completes it.
- **No `paths:` change in `smelt.yml`.** The existing `paths: [models, tests]` already covers `models/gold/` and `tests/`.
- **No edits to `crates/` outside `crates/smelt-datagen/tests/example_web_analytics.rs`.** The smelt language surface is unchanged in this phase — `WITH RECURSIVE` is already parsed and recursive-CTE column types already bootstrap to `Unknown` correctly (`crates/smelt-db/src/type_inference/subquery.rs:66`). No `signatures.rs` edits are required.
- **No `smelt.functions.<name>(...)` calls in model SQL bodies.** Per the Phase 3 / Phase 4 deferred precedent.
- **No `incremental:` frontmatter on the new gold model.** `silver/device_user_edges` is a view; the upstream `silver/sessions` carries the only incremental boundary. If a future phase needs to materialise `identity_connected_components` as a `table` with `incremental:` for downstream query speed, it can be added then — the SELECT shape here is compatible with that future change.
- **No probabilistic stitching, no ML-based linking.** Out of scope per the overall plan's "Out of scope" §.
- **No new joint-distribution edges in the canonical synthetic dataset.** The Phase 2 `linked_choice` generator's joint pool (60/25/10/5 weights, max 3 emits per draw) already produces enough multi-user devices and multi-device users to form non-trivial clusters for connected-components.

---

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | e55b30a6 | 2026-05-18 |
| 2     | done     | (see below) | 2026-05-18 |
| 3     | done     | 442e74aa | 2026-05-18 |
| 4     | pending  |        |      |

---

### Phase 1: `gold/identity_connected_components.sql` recursive-CTE label propagation

**Goal.** Land `examples/web_analytics/models/gold/identity_connected_components.sql` — a view that produces one row per device with `(device_id, connected_components_user_id, connected_components_cluster_id)` from `silver/device_user_edges`, where both identity columns equal the smallest user_id in the device's connected component under the bipartite-graph union-find via iter-capped recursive label propagation.

**Pre-conditions.** Phase 6 of the overall plan committed (`gold/identity_backward_fill`, `gold/eventstream_with_identity` with `backward_fill_user_id` column exist). Working tree clean on `worktree-web_analytics`.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the model lands.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_identity_connected_components_materializes` — extend the end-to-end test:
  1. Run `smelt-datagen ... --scale-factor 0.01`.
  2. Execute `setup_sources.sql`.
  3. Invoke `smelt build --project-dir examples/web_analytics --target dev` and assert exit 0.
  4. Assert `SELECT count(*) FROM main.gold_identity_connected_components > 0`.
  5. Assert one-row-per-device cardinality:
     ```sql
     SELECT count(*) FROM main.gold_identity_connected_components
     = SELECT count(DISTINCT device_id) FROM main.silver_device_user_edges
     ```
  6. Assert non-null output for both identity columns:
     ```sql
     SELECT count(*) FROM main.gold_identity_connected_components
     WHERE connected_components_user_id IS NULL
        OR connected_components_cluster_id IS NULL
     ```
     must equal 0.
  7. Assert the v1 cluster-id-equals-user-id invariant:
     ```sql
     SELECT count(*) FROM main.gold_identity_connected_components
     WHERE connected_components_user_id != connected_components_cluster_id
     ```
     must equal 0.
  8. Assert the transitive-closure invariant: any two devices sharing at least one user end up in the same cluster:
     ```sql
     SELECT count(*) FROM main.silver_device_user_edges e1
     JOIN main.silver_device_user_edges e2 ON e1.user_id = e2.user_id
     JOIN main.gold_identity_connected_components c1 ON c1.device_id = e1.device_id
     JOIN main.gold_identity_connected_components c2 ON c2.device_id = e2.device_id
     WHERE c1.connected_components_cluster_id != c2.connected_components_cluster_id
     ```
     must equal 0.
  9. Assert the cluster-id-is-MIN invariant: the cluster_id assigned to a device equals the smallest user_id in the cluster (closure of the bipartite-graph component containing the device). We can compute this from one round of cluster-aware aggregation: cluster ↔ MIN(user_id over every edge attached to a device with that cluster_id):
     ```sql
     SELECT count(*) FROM main.gold_identity_connected_components c
     JOIN main.silver_device_user_edges e ON e.device_id = c.device_id
     GROUP BY c.connected_components_cluster_id
     HAVING MIN(e.user_id) != c.connected_components_cluster_id
     ```
     must return zero rows. (Subtle: this checks the *local* closure — the MIN over edges directly attached to devices in the cluster. By the transitive-closure invariant above, that local closure equals the global MIN, but reviewers may want both invariants stated separately for diagnostic clarity.)

**Implementation shape.**

```sql
-- Per-device connected-components identity resolution via bipartite-graph
-- union-find. From silver/device_user_edges (the (device, user) co-occurrence
-- evidence over all signed-in events), build the bipartite graph where each
-- edge connects one device node to one user node. Two devices are in the same
-- component if a path of edges connects them through one or more shared users.
-- The resolved identity for every device in a component is the smallest
-- user_id in the component, which doubles as the component's cluster_id under
-- this v1 convention.
--
-- This is the Amplitude-full identity model — it propagates identity across
-- devices via user co-occurrence. Subsumes backward-fill on every device:
-- a device's backward-fill canonical user appears in its connected component
-- (because the backward-fill election only considers users who signed in on
-- the device itself, and the connected component includes all such users by
-- definition).
--
-- Devices that never had a signed-in event do not appear in
-- silver/device_user_edges and therefore not in this table; their events
-- resolve to NULL in gold/eventstream_with_identity via the LEFT JOIN.
--
-- The propagation runs as an iter-capped recursive CTE. The cap (8 iterations)
-- is generous: a 256-fold expansion of cluster diameter, far above the
-- synthetic dataset's expected graph diameter (3–5 hops at the 60/25/10/5
-- co-occurrence weights). Beyond the cap the algorithm emits whatever the cap
-- has reached — Phase 9 of the example's overall plan replaces the cap with a
-- true fixed-point if cap-bounded approximation becomes a constraint.
WITH RECURSIVE edges AS (
    SELECT device_id, user_id
    FROM smelt.silver.device_user_edges
),
device_seed AS (
    -- Base case: each device's initial label is the MIN user_id seen on it.
    SELECT device_id, MIN(user_id) AS label
    FROM edges
    GROUP BY device_id
),
device_label(device_id, label, iter) AS (
    SELECT device_id, label, 0 AS iter
    FROM device_seed
    UNION ALL
    -- Recursive step: a device's new label is the MIN over its own current
    -- label and the labels of every device that shares a user with it.
    -- Stop after 8 iterations.
    SELECT
        d.device_id,
        LEAST(d.label, MIN(d2.label)) AS label,
        d.iter + 1 AS iter
    FROM device_label d
    JOIN edges e1 ON e1.device_id = d.device_id
    JOIN edges e2 ON e2.user_id = e1.user_id
    JOIN device_label d2 ON d2.device_id = e2.device_id AND d2.iter = d.iter
    WHERE d.iter < 8
    GROUP BY d.device_id, d.label, d.iter
)
SELECT
    device_id,
    label AS connected_components_user_id,
    label AS connected_components_cluster_id
FROM (
    -- Take the label at the largest iter (i.e., after convergence within the cap).
    SELECT DISTINCT ON (device_id)
        device_id,
        label
    FROM device_label
    ORDER BY device_id, iter DESC
)
```

**Implementation-form notes (the implementer must verify against current smelt support during a discovery step).**

The recursive CTE above is one defensible shape. Two issues are worth probing before committing to it, because DuckDB's recursive-CTE engine has known limitations around aggregates in the recursive term:

1. **DuckDB recursive-CTE aggregate restriction.** DuckDB requires the recursive term to be a pure SELECT with no aggregation; the canonical pattern is to do the per-iteration aggregation *outside* the recursive table, then take the final-iteration result. If the discovery step fails on aggregate-in-recursive-term, the working shape is:
   ```sql
   WITH RECURSIVE edges AS (
       SELECT device_id, user_id FROM smelt.silver.device_user_edges
   ),
   device_seed AS (
       SELECT device_id, MIN(user_id) AS label FROM edges GROUP BY device_id
   ),
   -- Iterate label propagation: each iter recomputes the device label as the MIN
   -- label of every device that shares any user with it, via a JOIN through edges.
   propagation(device_id, label, iter) AS (
       SELECT device_id, label, 0 FROM device_seed
       UNION ALL
       SELECT d.device_id, d.label, d.iter + 1
       FROM propagation d
       WHERE d.iter < 8
   ),
   -- ... (refined shape per DuckDB's accepted recursion patterns)
   ```
   The implementer's discovery step is allowed to refactor toward whichever form DuckDB accepts, **provided** the algorithmic invariants (Steps 5–9 in the TDD list above) all pass. Record the chosen form in "Deferred during implementation" with a one-line rationale.

2. **Alternative: non-recursive iter-unrolled form.** If the recursive CTE cannot be coerced into a working DuckDB shape, an iter-unrolled form with 8 explicit CTEs (`iter0`, `iter1`, ..., `iter8`) is an acceptable fallback. It is more verbose but equivalent in semantics and avoids the recursive-CTE-restrictions pitfall. Keep it as the last-resort form; document the reason for falling back in "Deferred during implementation".

3. **`LEAST` registration check.** `LEAST` is the standard SQL function for n-ary min in DuckDB and is broadly available. Verify against `crates/smelt-types/src/functions.rs` before relying on it: if not registered, either register it (with the same lockstep update across signatures/functions/function_call.rs as Phase 5 had to do for `arg_max`) or rewrite with `CASE WHEN d.label < MIN(d2.label) THEN d.label ELSE MIN(d2.label) END`. Prefer registering `LEAST` because it is genuinely useful for future plans — but only if the lockstep update is mechanical; if it surfaces type-inference complexity, fall back to the `CASE` form and add `LEAST` registration as a separate deferred follow-up. Scope discipline: do not let this phase grow into a function-registry expansion.

Notes the implementer must verify against current smelt support:

- `WITH RECURSIVE` syntax — verified at the parser level (`crates/smelt-parser/src/parser/select.rs:799`).
- Recursive-CTE column type bootstrapping — handled (`crates/smelt-db/src/type_inference/subquery.rs:66` bootstraps recursive-CTE columns to `Unknown`).
- The choice between `UNION ALL` (typical for recursive CTEs) and `UNION` (deduplicating) — `UNION ALL` is required by DuckDB for recursive CTEs.
- `DISTINCT ON` to pick the largest-iter row per device — already verified at Phase 6 to be cleanly supported.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/identity_connected_components.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_identity_connected_components_materializes` fn)
- `docs/plans/20260517-web-analytics-7-connected-components.md` (this file — committed at the start of Phase 1 work)
- *Conditional only if `LEAST` discovery requires registration:* `crates/smelt-types/src/signatures.rs`, `crates/smelt-types/src/functions.rs`, `crates/smelt-db/src/type_inference/function_call.rs`. Implementer should default to the `CASE` form first to avoid registry-expansion scope; only register `LEAST` if the inline test is unable to make the `CASE` form work cleanly.

**Docs touched.** None (header comment in the new SQL file is timeless per the Timeless-oracle rule).

**Review checklist** (material findings only):

- [ ] Model produces exactly the columns `(device_id, connected_components_user_id, connected_components_cluster_id)` — no extra columns that would constrain Phase 8 prematurely.
- [ ] Source reference uses path syntax (`smelt.silver.device_user_edges`) — no dead `smelt.ref()` syntax.
- [ ] Per-device output is the smallest user_id in the device's bipartite-graph connected component. The propagation is correct: each iteration broadens a device's label to the MIN over its current label and the labels of every device that shares a user with it.
- [ ] Iter cap is documented (8 in v1; Phase 9 replaces with a fixed-point). The cap appears in a header-comment block with the rationale.
- [ ] The TDD transitive-closure invariant (Step 8) and cluster-id-is-MIN invariant (Step 9) both pass on the synthetic dataset.
- [ ] Devices not in `silver/device_user_edges` are absent from the output (handled downstream via LEFT JOIN).
- [ ] Header comment is timeless — no `Phase N` references in the SQL file. Wording that describes the algorithm's relationship to backward-fill ("Amplitude-full" / "subsumes backward-fill") is acceptable (feature relationships, not phase history).
- [ ] Recursive CTE termination: the synthetic dataset's longest expected component diameter (3–5 hops) is well below the iter cap of 8 — convergence is achieved before the cap fires. If the implementer observes the cap firing on the synthetic data, the algorithm is wrong (or the synthetic distribution has shifted) — flag immediately.
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics gold/identity_connected_components model (web-analytics Phase 7)`

---

### Phase 2: Extend `gold/eventstream_with_identity.sql` with `connected_components_user_id` and `connected_components_cluster_id`

**Goal.** Edit `examples/web_analytics/models/gold/eventstream_with_identity.sql` to add `LEFT JOIN smelt.gold.identity_connected_components c ON e.device_id = c.device_id` and project both `c.connected_components_user_id` and `c.connected_components_cluster_id` immediately after `b.backward_fill_user_id` in the SELECT list. Preserve every other join, filter, and column verbatim.

**Pre-conditions.** Phase 1 of this plan committed (`gold/identity_connected_components` materialises).

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/` after the edit.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_eventstream_with_identity_includes_connected_components` — a new end-to-end test (parallel to the existing `test_eventstream_with_identity_includes_backward_fill`, but focused on the connected-components columns):
  1. Run datagen → setup_sources → `smelt build` as in Phase 1.
  2. Assert column shape: `SELECT connected_components_user_id, connected_components_cluster_id FROM main.gold_eventstream_with_identity LIMIT 1` does not error and returns a row.
  3. Assert event-preserving cardinality is unchanged: `SELECT count(*) FROM main.gold_eventstream_with_identity = SELECT count(*) FROM main.silver_events_parsed`.
  4. Assert LEFT-JOIN population: for every event whose `device_id` is in `gold_identity_connected_components`, both `connected_components_user_id` and `connected_components_cluster_id` are non-null:
     ```sql
     SELECT count(*) FROM main.gold_eventstream_with_identity es
     JOIN main.gold_identity_connected_components cc USING (device_id)
     WHERE es.connected_components_user_id IS NULL
        OR es.connected_components_cluster_id IS NULL
     ```
     must equal 0.
  5. Assert single-valued `connected_components_user_id` within device:
     ```sql
     SELECT count(*) FROM (
       SELECT device_id, count(DISTINCT connected_components_user_id) AS k
       FROM main.gold_eventstream_with_identity
       GROUP BY device_id
       HAVING k > 1
     )
     ```
     must equal 0. Same shape for `connected_components_cluster_id`.
  6. Assert subsumption (backward-fill ⊆ connected-components on the per-event level): every event with a non-null `backward_fill_user_id` also has a non-null `connected_components_user_id`:
     ```sql
     SELECT count(*) FROM main.gold_eventstream_with_identity
     WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NULL
     ```
     must equal 0. (A device has a backward-fill canonical user iff it has at least one signed-in event iff it appears in `silver/device_user_edges` iff it appears in `gold_identity_connected_components`. The LEFT JOINs on `device_id` are isomorphic between the two algorithms.)
  7. Assert column ordering is preserved relative to Phase 6 (regression test): `SELECT event_id, device_id, event_user_id, event_ts, event_date, event_name, platform, url, session_id, forward_only_user_id, backward_fill_user_id, connected_components_user_id, connected_components_cluster_id FROM main.gold_eventstream_with_identity LIMIT 1` must succeed without error — this implicitly verifies column ordering and existence.

**Implementation shape.**

```sql
-- Per-event wide table that joins every silver/events_parsed row to its
-- session (silver/sessions) and attaches each available identity algorithm's
-- resolved column. Carries three identity algorithms today (forward_only,
-- backward_fill, connected_components); the wide shape is fixed so additional
-- algorithms can be added as LEFT JOIN + column projections without
-- restructuring the row.
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
--   connected_components_user_id  — resolved identity via the cross-device
--                           bipartite-graph union-find (NULL for devices that
--                           never had a signed-in event); see
--                           gold/identity_connected_components
--   connected_components_cluster_id — cluster label from the connected-components
--                           union-find (NULL on the same condition as the
--                           user_id column). Numerically equal to
--                           connected_components_user_id in the v1 algorithm
--                           (both are the smallest user_id in the cluster);
--                           surfaced separately so a future probabilistic-
--                           stitching alternative could decouple them without
--                           reshuffling the eventstream.
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
    b.backward_fill_user_id,
    c.connected_components_user_id,
    c.connected_components_cluster_id
FROM smelt.silver.events_parsed e
JOIN smelt.silver.sessions s
    ON e.device_id = s.device_id
   AND e.event_ts >= s.session_start
   AND e.event_ts <= s.session_end
LEFT JOIN smelt.gold.identity_forward_only f
    ON s.session_id = f.session_id
LEFT JOIN smelt.gold.identity_backward_fill b
    ON e.device_id = b.device_id
LEFT JOIN smelt.gold.identity_connected_components c
    ON e.device_id = c.device_id
```

Notes:

- The LEFT JOIN to `identity_connected_components` is on `device_id`. Devices that never had a signed-in event do not appear in `device_user_edges` and therefore not in `identity_connected_components`; their events get `NULL` for both new columns. This is the algorithm's defining signature at the event level: all-anonymous devices stay anonymous under connected-components (no graph node, no cluster).
- LEFT (not INNER) is required so we don't silently drop all events from anonymous-only devices. Without this, the event count of `eventstream_with_identity` would change as the dataset evolves — anonymous-only devices would be culled from the join. The existing event-preserving cardinality test (step 3 above) catches the bug.
- Column order: `... session_id, forward_only_user_id, backward_fill_user_id, connected_components_user_id, connected_components_cluster_id`. Phase 8 marts will read these by name, so the column-order regression test in step 7 above guards against future reshuffling.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/gold/eventstream_with_identity.sql` (edit — header comment block expanded, one new LEFT JOIN, two new columns)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_eventstream_with_identity_includes_connected_components` fn)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Existing columns and join shapes are preserved verbatim — only one new LEFT JOIN and two new columns added.
- [ ] New JOIN to `identity_connected_components` is LEFT (not INNER) — preserves events on anonymous-only devices.
- [ ] New JOIN condition is `e.device_id = c.device_id`. Not `s.device_id = c.device_id`.
- [ ] Column order: `forward_only_user_id` precedes `backward_fill_user_id` precedes `connected_components_user_id` precedes `connected_components_cluster_id`. Phase 8 marts depend on this ordering for `SELECT *` ergonomics; the explicit-column-list query in TDD step 7 above is a regression test against unintended re-orderings.
- [ ] Header comment block updated to list both new columns and to explain the cluster_id ↔ user_id v1 coincidence (with the framing that they are surfaced separately for future-proofing, not because they currently differ).
- [ ] No reach into Phase 8 scope (no mart references, no aggregate queries).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics eventstream connected_components columns (web-analytics Phase 7)`

---

### Phase 3: Inline `.test.sql` connected-components resolution invariants

**Goal.** Land `examples/web_analytics/tests/connected_components_resolution_invariants.test.sql` — a `materialization: test` file asserting the five defining cluster-shape invariants of the bipartite-graph union-find (single device + single user; single device + two users; two devices joined through co-occurrence; three-device chain via transitive closure; isolated user as a negative test) on hand-crafted mock data. The verification gate for Phase 7 ("inline test for shared-device cluster; diagnostics gate" — meta-plan §3 row 7) is met by this file.

**Pre-conditions.** Phases 1–2 of this plan committed. Phase 4's `test_compiler.rs` WITH-clause-merging patch is already in place — it supports a `WITH RECURSIVE`-leading model without further changes (the patch's `find_leading_with()` helper looks for a top-level `WITH` keyword and the merge is mechanical regardless of the `RECURSIVE` modifier).

**TDD tests to write first.**

- The standing `example_diagnostics` gate continues to pass.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_connected_components_invariants_inline_pass` — invoke `smelt test --project-dir examples/web_analytics --select connected_components_resolution`; assert exit 0 and that the named test reports PASS.

**Implementation shape.**

```sql
--- name: test_connected_components_resolution_invariants ---
materialization: test
test:
  model: identity_connected_components
  inputs:
    silver_device_user_edges:
      # Cluster 1: single device, single user (degenerate base case)
      - {device_id: 1, user_id: 100, event_count: 1, first_seen: '2026-04-01 09:00:00', last_seen: '2026-04-01 09:01:00'}

      # Cluster 2: single device, two users → joined through device 2
      # Cluster = {200, 201}; cluster_id = MIN(200, 201) = 200
      - {device_id: 2, user_id: 200, event_count: 1, first_seen: '2026-04-01 10:00:00', last_seen: '2026-04-01 10:01:00'}
      - {device_id: 2, user_id: 201, event_count: 1, first_seen: '2026-04-01 10:10:00', last_seen: '2026-04-01 10:11:00'}

      # Cluster 3: two devices joined through user 301
      # Cluster = {300, 301}; both device 3 and device 4 resolve to user 300
      - {device_id: 3, user_id: 300, event_count: 1, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:01:00'}
      - {device_id: 3, user_id: 301, event_count: 1, first_seen: '2026-04-01 11:10:00', last_seen: '2026-04-01 11:11:00'}
      - {device_id: 4, user_id: 301, event_count: 1, first_seen: '2026-04-01 11:20:00', last_seen: '2026-04-01 11:21:00'}

      # Cluster 4: three-device chain (transitive closure)
      # Device 5 ↔ user 501 ↔ Device 6 ↔ user 502 ↔ Device 7
      # Cluster = {500, 501, 502, 503}; cluster_id = 500
      # All three devices resolve to user 500. This case forces propagation
      # to converge over ≥ 2 iterations; a single-iter implementation would
      # fail because device 7 cannot reach device 5 in one hop.
      - {device_id: 5, user_id: 500, event_count: 1, first_seen: '2026-04-01 12:00:00', last_seen: '2026-04-01 12:01:00'}
      - {device_id: 5, user_id: 501, event_count: 1, first_seen: '2026-04-01 12:10:00', last_seen: '2026-04-01 12:11:00'}
      - {device_id: 6, user_id: 501, event_count: 1, first_seen: '2026-04-01 12:20:00', last_seen: '2026-04-01 12:21:00'}
      - {device_id: 6, user_id: 502, event_count: 1, first_seen: '2026-04-01 12:30:00', last_seen: '2026-04-01 12:31:00'}
      - {device_id: 7, user_id: 502, event_count: 1, first_seen: '2026-04-01 12:40:00', last_seen: '2026-04-01 12:41:00'}
      - {device_id: 7, user_id: 503, event_count: 1, first_seen: '2026-04-01 12:50:00', last_seen: '2026-04-01 12:51:00'}

      # Cluster 5: isolated user retains identity (negative test — no spurious merging)
      - {device_id: 8, user_id: 600, event_count: 1, first_seen: '2026-04-01 13:00:00', last_seen: '2026-04-01 13:01:00'}
  expect:
    - {device_id: 1, connected_components_user_id: 100, connected_components_cluster_id: 100}  # degenerate
    - {device_id: 2, connected_components_user_id: 200, connected_components_cluster_id: 200}  # MIN(200, 201)
    - {device_id: 3, connected_components_user_id: 300, connected_components_cluster_id: 300}  # MIN(300, 301)
    - {device_id: 4, connected_components_user_id: 300, connected_components_cluster_id: 300}  # via user 301 ↔ device 3
    - {device_id: 5, connected_components_user_id: 500, connected_components_cluster_id: 500}  # chain head
    - {device_id: 6, connected_components_user_id: 500, connected_components_cluster_id: 500}  # transitive
    - {device_id: 7, connected_components_user_id: 500, connected_components_cluster_id: 500}  # 2-hop transitive
    - {device_id: 8, connected_components_user_id: 600, connected_components_cluster_id: 600}  # isolated
---
```

Notes:

- The `expect:` block tests the *defining* invariants of `identity_connected_components` directly: (a) degenerate single-edge cluster — Device 1; (b) within-device union via a single device with two users — Device 2 (the user-set is the union of users on the device); (c) cross-device union via a shared user — Devices 3 and 4 (note that both must resolve to the *same* cluster_id = 300, not their own MIN); (d) transitive closure across multiple hops — Devices 5/6/7 (this is the key correctness test for the recursive propagation: a single-iter or fixed-step implementation would resolve Device 7 to cluster_id = 502 instead of 500); (e) negative case — Device 8 (no spurious merging when there is no shared user).
- The `event_count` and `first_seen`/`last_seen` columns are present for schema completeness (the mock must match the upstream view's column set) but are *not* used by the connected-components algorithm; any valid TIMESTAMP literal works.
- The test works whether the model uses the recursive-CTE form or an iter-unrolled fallback (per Phase 1's discovery step) — both should produce the same row-by-row output for cluster sizes ≤ 8 hops.
- All eight `expect:` rows exercise the algorithm's full row-shape: every device_id from the input appears in the expected output (the model produces one row per device that has at least one edge).

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/tests/connected_components_resolution_invariants.test.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_connected_components_invariants_inline_pass` fn)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Five clusters exercise five distinct invariants (degenerate, within-device union, cross-device via shared user, transitive closure across a 3-device chain, isolated negative test). Each invariant attributable to one or more device_ids.
- [ ] Cluster 4 (three-device chain) is essential — it is the only test that would fail under a faulty single-iter implementation. Removing it would silently accept a broken algorithm.
- [ ] Cluster 5 (isolated user) is essential — it is the only negative test against spurious merging. Removing it would silently accept an algorithm that lumps every device into a single cluster.
- [ ] `inputs:` rows mock the full schema of `silver_device_user_edges` (`device_id, user_id, event_count, first_seen, last_seen`).
- [ ] `expect:` rows have *both* identity columns asserted (`connected_components_user_id` AND `connected_components_cluster_id`), with values that are equal in v1 — this protects against an implementation that emits only one of the two columns or that lets them diverge spuriously.
- [ ] Test runs under `smelt test` and reports PASS.
- [ ] No reach into Phase 8 scope (no mart references, no DAU comparisons in the test body).

**Commit.** `feat(examples): web_analytics connected_components resolution invariant test (web-analytics Phase 7)`

---

### Phase 4: Expert reviewer dispatch loop

For each expert listed below, dispatch via the Agent tool with `model: "sonnet"`,
brief prompt, the per-phase plan path, the spec path (if relevant), and the
in-repo plan path. The expert returns a list of findings classified as
"material" or "stylistic". Address material findings:

  - For each material finding, either edit directly (small) or dispatch a
    nested implementer subagent (larger).
  - Commit the fix with message `review(web-analytics-7): address {expert-name} feedback`.
  - Push.
  - Re-dispatch the same expert. Loop until the expert returns "no material findings".

Bounds:

  - Max 3 rounds per expert. If unresolved after 3 rounds → emit
    `<<PAUSE_FOR_HUMAN>>`.
  - If two different experts flag the same systemic concern in one round →
    emit `<<PAUSE_FOR_HUMAN>>`.

Experts for this phase (from meta-plan §5 row 7):

  - `sql-expert` — focus per meta-plan §5: **recursive-CTE termination, iter-cap rationale, label-propagation correctness**. Specifically:
    - Recursive-CTE correctness: the propagation must compute the smallest user_id in the bipartite-graph connected component containing each device. Verify against the five cluster shapes in Phase 3's test. The three-device chain (Cluster 4) is the critical test — it only passes if propagation runs for ≥ 2 iterations.
    - Iter-cap rationale: the cap (8) must be justified. A 256-fold expansion in cluster diameter at 8 iterations is more than enough for the synthetic dataset's expected graph diameter (3–5 hops at 60/25/10/5 weights). The expert should verify the cap is documented and that the cap-firing case (cluster diameter > 8) is correctly identified as the v1 limitation, with Phase 9 of the overall plan tracking the follow-up.
    - DuckDB recursive-CTE limitations: the recursive term cannot contain aggregations in some DuckDB versions. The expert should verify the chosen form is DuckDB-compatible (and, if the implementer chose the iter-unrolled fallback per Phase 1's discovery note, that the iter-unrolled form is semantically equivalent and the rationale is documented in "Deferred during implementation").
    - Cluster-id definition: in v1, `connected_components_cluster_id = connected_components_user_id = MIN(user_id over cluster)`. Both columns are required to be emitted, both are required to be non-null, and both are required to be numerically equal. The test in Phase 3 must enforce all three. The expert should verify the test asserts both columns on every expected row.
    - LEFT-vs-INNER JOIN choice in the eventstream extension: `LEFT JOIN identity_connected_components` is required (events from anonymous-only devices must be preserved). The expert should flag if this is silently degraded to INNER or rendered LEFT in a way that doesn't actually preserve rows.
    - Subsumption invariant in the eventstream test (step 6 of Phase 2's TDD list): every event with non-null `backward_fill_user_id` must have non-null `connected_components_user_id`. The expert should verify the test query expresses this correctly.
    - Transitive-closure invariant (Phase 1 TDD step 8): the query joins `silver_device_user_edges` to itself on `user_id` to find every device pair sharing a user, then asserts both devices have the same `connected_components_cluster_id`. The expert should verify the query is correct and doesn't accidentally exclude self-pairs (`d1 = d2`) or duplicate-counted bidirectional pairs in a way that masks a real failure.
    - No parser-unsupported constructs (verified by `example_diagnostics` already, but the expert may catch a construct that parses but is semantically wrong on DuckDB).

  - `examples-curator` — focus on example-pipeline quality:
    - The new model fits the bronze→silver→gold pattern of the example. `identity_connected_components` is materialised at the gold layer alongside the two existing identity models, with the same input shape (`silver/device_user_edges`) and the same output shape (`(device_id, ...)`).
    - The header comments are timeless, descriptive, and educational. They frame the algorithm as "Amplitude-full" and explain the cross-device subsumption invariant. They document the iter cap with its rationale (a 256-fold cluster-diameter expansion is well above the synthetic data's expected diameter).
    - The two-identity-column shape (`connected_components_user_id` + `connected_components_cluster_id`) is justified in the header comment — the rationale that they are surfaced separately for future-proofing against probabilistic-stitching alternatives is preserved.
    - The inline `.test.sql` is well-structured and reads as documentation: each cluster's purpose is named in YAML comments; the three-device chain is identified as the transitive-closure stress test; the isolated user is identified as the negative test.
    - The integration tests in `example_web_analytics.rs` are consistent with the existing tests' structure (TempDir + copy_dir_all + rewrite_outputs + run_datagen + setup_sources + smelt build + assertions). The expert should flag any drift from that established pattern.
    - No README mentions of the new algorithm (Phase 8 of the overall plan completes the README — Phase 7 must not leak into Phase 8's scope).
    - The new model's name (`identity_connected_components`) matches the meta-plan's naming convention (matches `identity_forward_only`, `identity_backward_fill`).

If a literal `sql-expert` agent type does not exist, dispatch `general-purpose`
with a prompt that frames it as such (read the plan + diff, flag plan/impl
drift, missing test cases, scope creep into later phases, recursive-CTE
correctness — material findings only). Same for `examples-curator`.

**Loop discipline.**

1. **Round 1.** Dispatch `sql-expert` and `examples-curator` (in parallel if possible) with `model: "sonnet"`. The prompt MUST include:
   - This plan's path and the oracle paths (overall plan, meta-plan, Phase 3 / Phase 4 / Phase 5 / Phase 6 plan "Deferred during implementation" sections, `testing.md`).
   - The exact file scope from the per-sub-phase tables above.
   - The diff range to review: commits since the start of Phase 7 of the overall plan (typically the three `feat(examples): web_analytics ... (web-analytics Phase 7)` commits — `git log --oneline {phase-7-base}..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, scope creep, missing test cases, plan/impl drift, parser limitations hit). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` if the expert's tool palette allows nested subagents.

2. **Address findings.** For each material finding:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics`, and the `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests after each fix batch.
   - Commit per round: `review(web-analytics-7): address {expert-name} feedback`.
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch the same expert with the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → expert is clean and exits.

4. **Repeat** until all experts return clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - Any single expert flags a material finding on round 3 (per-expert bound).
   - Two different experts flag the same systemic concern in one round (cross-expert bound).
   - An expert's findings would force a spec change. Run `/smelt:spec` on the relevant slug first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 7.
   - The recursive CTE fails to terminate (cap fires on the synthetic data, indicating an algorithm bug).

**Critical files (allowed to touch in this phase).** Anything within the
experts' scope per the table above, plus
`docs/plans/20260517-web-analytics-7-connected-components.md` (to record round counts
and the final clean status) and `docs/plans/20260517-web-analytics-example.md`
(to flip the overall-plan status row).

**Review checklist** (applied to the expert-dispatch *process*, not to a code diff):

- [ ] `sql-expert` dispatched at least once.
- [ ] `examples-curator` dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`,
  `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`), the end-to-end integration tests, and the inline `.test.sql` invariant test all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 4 expert review: sql-expert clean (R{n}), examples-curator clean (R{m}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 7 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings:
`review(web-analytics-7): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-7): mark Phase 7 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 1 — iter-unrolled form chosen over WITH RECURSIVE.** DuckDB's recursive-CTE engine prohibits aggregates (`MIN`, `GROUP BY`) inside the recursive term; the plan's primary form used both in the recursive step. Fallback: 8 explicit CTEs (`iter0`–`iter8`) each recomputing every device's label as `MIN(CASE WHEN ...)` over the one-hop neighbourhood. This is semantically equivalent and verified correct by all 6 TDD assertions (count, cardinality, non-null, cluster-id=user-id, transitive-closure, cluster-id-is-MIN). `LEAST` registration was confirmed present (`crates/smelt-types/src/signatures.rs:3635`) but `CASE WHEN` was used instead per plan instructions to avoid registry expansion.

- **Phase 1 — Progress tracking update.** Phase 1 commit: `e55b30a6`.

## Verification

How to confirm the scope is satisfied at the end of Phase 4:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — no regression in the workspace.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `cargo test -p smelt-datagen --test example_web_analytics` passes — including the new `test_identity_connected_components_materializes`, `test_eventstream_with_identity_includes_connected_components`, and `test_connected_components_invariants_inline_pass` sub-tests.
- Manual fresh-checkout dry run succeeds:
  ```bash
  smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01
  duckdb examples/web_analytics/target/dev.duckdb < examples/web_analytics/setup_sources.sql
  smelt build --project-dir examples/web_analytics --target dev
  smelt test --project-dir examples/web_analytics --select connected_components_resolution
  ```
- Phase 4 acceptance gate met: `sql-expert` and `examples-curator` both reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 7 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
