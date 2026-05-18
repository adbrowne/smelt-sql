# Plan: Web Analytics Phase 8 — Marts (`daily_active_users_by_method`, `identity_method_comparison`) + README + docs-site link

**Date**: 2026-05-18
**Spec**: example phases do not anchor to a single feature spec; the oracle is the overall plan ([`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md) §Goal item 5 — "marts that quantify the difference between the three algorithms (DAU, identification rate)") and the meta-plan (`/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §3 row 8 and §6 — "Mart `daily_active_users_by_method` shows monotonic `forward_only ≤ backward_fill ≤ connected_components` on every day in the synthetic dataset"). Spec cross-references that ground specific decisions: [`docs/specs/testing.md`](../specs/testing.md) (`materialization: test` inline assertions, YAML coercion). No spec change in this phase.
**Spec diff**: none. Phase 8 lands two gold-mart models that read from `gold/eventstream_with_identity` and quantify the algorithmic differences between the three identity algorithms (`daily_active_users_by_method` for per-day DAU counts and `identity_method_comparison` for pairwise reidentification counts), the DAU monotonicity invariant inline test (the meta-plan §6 verification gate), the polished `examples/web_analytics/README.md`, the new row in `examples/README.md`, and a new docs-site page at `docs-site/docs/examples/web_analytics.md` linked from a new top-level `Examples:` nav section in `docs-site/mkdocs.yml`.
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs (this is the docs-completion phase for the example — README polish, examples index row, docs-site page, and nav update all land in this phase per the meta-plan §3 row 8 deliverables column "Marts + README + docs-site link")

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 8 to completion using `/smelt:implement`, then dispatch the meta-plan §5 row 8 expert reviewers (`sql-expert`, `examples-curator`, `docs-reviewer`), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read the overall plan and the meta-plan for the sentinel emission contract and stop-the-line conditions. The Phase 3 (`docs/plans/20260517-web-analytics-3-scaffold.md`), Phase 4 (`docs/plans/20260517-web-analytics-4-sessionize.md`), Phase 5 (`docs/plans/20260517-web-analytics-5-forward-only.md`), Phase 6 (`docs/plans/20260517-web-analytics-6-backward-fill.md`), and Phase 7 (`docs/plans/20260517-web-analytics-7-connected-components.md`) "Deferred during implementation" sections are required reading — they record concrete smelt constraints this phase must respect:
   - call-syntax discipline (path syntax `smelt.<dir>.<stem>`; `smelt.ref()` is dead syntax);
   - `to_seconds` not in inference (use `epoch_us` arithmetic) — not directly relevant here because the marts aggregate on `event_date`, not durations;
   - `IS DISTINCT FROM` not supported by smelt-parser — use `!=` or `OR ... IS NULL`;
   - `md5` not registered → use `CONCAT` for surrogate-key formation if needed (Phase 8 does not need hashing);
   - two-CTE structure required for nested window functions (Phase 8 marts do not use window functions);
   - materialised model addresses include the directory segment (`main.gold_eventstream_with_identity`, `main.marts_daily_active_users_by_method`, `main.marts_identity_method_comparison`);
   - `tests` must already appear in `smelt.yml` `paths:` (it does — `examples/web_analytics/smelt.yml:5`) — but `marts` is **not** in `paths:`; this phase MUST add it (see the smelt.yml change in Phase 1 below);
   - registering a function in `crates/smelt-types/src/signatures.rs` also requires lockstep edits in `crates/smelt-types/src/functions.rs` and `crates/smelt-db/src/type_inference/function_call.rs` — Phase 8 should **not** need any registry additions, but if a finding surfaces one, scope-check before doing it;
   - `WITH`-clause-merging fix in `crates/smelt-cli/src/test_compiler.rs` (Phase 4 deferred) supports inline tests on models that begin with a leading `WITH` — the marts in this phase do not start with `WITH`, but the inline DAU monotonicity test selects from a mart, so the merging logic applies (and is already known to work);
   - Timestamp coercion in `yaml_value_to_sql` (Phase 4 deferred) supports `YYYY-MM-DD HH:MM:SS` and `DATE`-shaped literals — the DAU monotonicity test's `inputs:` block uses `DATE` literals (no time component) for the `event_date` column, which is supported;
   - Phase 7 deferred: the recursive-CTE form was replaced with an iter-unrolled 8-CTE form; the marts read from `eventstream_with_identity` and treat the connected-components columns as opaque — they do not care about the underlying form.
   - Do not re-open those decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 5 is the expert-reviewer dispatch loop** — after Phases 1–4 commit, dispatch the meta-plan §5 row 8 expert reviewers (`sql-expert`, `examples-curator`, `docs-reviewer` — three experts). Address material findings, re-dispatch until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 5. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 5's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user (emit `<<PAUSE_FOR_HUMAN>>`):**

- The reviewer surfaces the same material finding across two implementer passes on the same sub-phase.
- TDD tests cannot be made green without violating a Phase 3 / Phase 4 / Phase 5 / Phase 6 / Phase 7 deferred-item ground rule.
- The DAU monotonicity invariant (`forward_only ≤ backward_fill ≤ connected_components` per day) is violated on the synthetic dataset — this is meta-plan §7 stop-the-line condition 5 verbatim: "The mart in Phase 8 shows the DAU monotonicity invariant violated on >0 days — algorithm bug or sessionization interaction." Do not "fix" the mart to mask the violation; the upstream algorithm or sessionization is wrong.
- The smelt parser or type-checker surfaces a defect that blocks the marts. Escalating means: the SQL pattern needs a parser/type-inference change beyond Phase 8 scope.
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to this plan.
- Phase 5 (expert dispatch): any single expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in one round (cross-expert bound).
- The docs-site build (`mkdocs build` or similar) is not run in this autonomy loop, but the new page must follow the existing pattern (front-matter / heading conventions, internal links resolve, code fences use the right language). If a docs-reviewer finding requires running mkdocs locally to verify, pause for the user rather than installing the docs toolchain inside this loop.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation. The standing oracles are `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`) and the end-to-end integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` (extended per sub-phase — datagen → setup_sources → `smelt build` succeeds and the new mart's row counts + monotonicity invariant hold).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + each Phase 5 expert spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces *only* the two gold-mart models, the DAU monotonicity inline test, the polished README, the `examples/README.md` row, and the docs-site page + nav update. **No new identity algorithm. No changes to bronze/silver/gold/eventstream_with_identity. No `time_to_identity` mart (out of scope per overall plan).** Phase 9 (true fixed-point) remains optional and is not in this phase.
- Honor architectural invariants from `CLAUDE.md` (no `crates/` edits unless extending the existing `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests). All SQL must parse without LSP diagnostics in `examples/web_analytics/`.
- **Timeless-oracle rule (CLAUDE.md).** This plan file uses phase vocabulary; SQL file header comments, the README, the `examples/README.md` row, and the docs-site page must read as feature descriptions with no `Phase N` labels. The README may link to the overall plan as historical context, but the body text must describe the example as if it has always existed.

---

## Context

The overall plan's Goal item 5 asks for marts that "quantify the difference between the three algorithms (DAU, identification rate)." The meta-plan §6 sets the algorithmic invariant the marts must demonstrate: `forward_only ≤ backward_fill ≤ connected_components` on every day. Phase 7 landed the third (and final) identity algorithm and the wide `eventstream_with_identity` table that surfaces all three resolved columns side-by-side. Phase 8 is the example's docs-and-narrative completion phase — the marts turn the row-by-row algorithmic difference visible in the eventstream into per-day aggregates that make the algorithmic tradeoff legible to a human reading the example for the first time, and the README + docs-site page make the example discoverable.

The two marts answer two different questions:

1. **`daily_active_users_by_method`** — per-day DAU under each of the three identity-resolution methods. Each row is a `(event_date, dau_forward_only, dau_backward_fill, dau_connected_components, total_events, identified_events_forward_only, identified_events_backward_fill, identified_events_connected_components)` tuple. The first three counts illustrate the meta-plan §6 monotonicity invariant: in any day, the connected-components algorithm should identify ≥ as many distinct users as backward-fill, which should identify ≥ as many distinct users as forward-only. The `identified_events_*` counts measure the algorithm's *reach* (the count of events that resolve to a non-null identity) — strictly less interesting than DAU because reach saturates quickly, but useful for the README narrative.

2. **`identity_method_comparison`** — *pairwise* event-level comparison. Each row is a `(comparison_name, agree_events, disagree_events, only_left_identified, only_right_identified, both_null_events, total_events)` tuple, with `comparison_name` enumerating the three pairs `forward_vs_backward`, `forward_vs_connected`, `backward_vs_connected`. The four counts characterise the union and intersection of the two algorithms' identity assignments on every event: `agree_events` (both algorithms assign the same non-null user), `disagree_events` (both assign non-null, but different user IDs), `only_left_identified` and `only_right_identified` (one assigns non-null and the other does not), and `both_null_events` (neither identifies). The expected shape: forward_vs_backward and forward_vs_connected have substantial `only_right_identified` and small `disagree_events` (because backward-fill/connected-components *subsume* forward-only); backward_vs_connected has substantial `disagree_events` *and* substantial `only_right_identified` (because connected-components reassigns the canonical user of a multi-device cluster to the cluster minimum, which differs from backward-fill's per-device election whenever a cluster spans devices). These shapes are *not* asserted in inline tests — they are described in the README, with the mart serving as the live demonstration.

The DAU monotonicity inline test lives at `examples/web_analytics/tests/dau_monotonicity_invariants.test.sql`. It targets `daily_active_users_by_method` with a small mocked `gold_eventstream_with_identity` `inputs:` block (3–4 days, ~10 events) crafted so that on every day the inequality is strict (`forward < backward < connected_components`) — this catches a mart bug that swaps the three columns by accident as well as the upstream-algorithm subsumption invariant.

The integration test in `crates/smelt-datagen/tests/example_web_analytics.rs` extends with one new function — `test_daily_active_users_by_method_monotonicity` — that runs the full pipeline on `--scale-factor 0.01` and asserts the DAU monotonicity invariant holds on *every* day of the synthetic dataset (not just one). This is meta-plan §7 stop-the-line condition 5: if the invariant fails on the synthetic data, the upstream algorithm or sessionization is buggy, not the mart.

The README polish replaces the Phase 3 stub with a proper walkthrough: what the example demonstrates, the three identity-resolution algorithms framed against the Amplitude reference doc, the bronze→silver→gold lineage, the marts, the inline tests, and how to run the example locally. The `examples/README.md` row adds `web_analytics/` to the directory-listing table. The docs-site page mirrors the README at a higher level (linked from the new top-level `Examples:` nav section in `docs-site/mkdocs.yml`) and is the discoverable entry point from the documentation site.

## Scope

### In scope

- `examples/web_analytics/smelt.yml` — edit (not replace): add `models/marts/` to discovery by ensuring `models/` is in `paths:` (it already is — `examples/web_analytics/smelt.yml:4`). **No `paths:` change needed**: `paths: [models, tests]` already covers nested directories under `models/`. This bullet is a reminder, not a change.

- `examples/web_analytics/models/marts/daily_active_users_by_method.sql` — a `view` (default materialization) that selects from `smelt.gold.eventstream_with_identity` and groups by `event_date`. Output columns:
  - `event_date: DATE`
  - `total_events: BIGINT` — `COUNT(*)`
  - `dau_forward_only: BIGINT` — `COUNT(DISTINCT forward_only_user_id)` (DISTINCT counts ignore NULL in DuckDB)
  - `dau_backward_fill: BIGINT` — `COUNT(DISTINCT backward_fill_user_id)`
  - `dau_connected_components: BIGINT` — `COUNT(DISTINCT connected_components_user_id)`
  - `identified_events_forward_only: BIGINT` — `COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL)`
  - `identified_events_backward_fill: BIGINT` — `COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL)`
  - `identified_events_connected_components: BIGINT` — `COUNT(*) FILTER (WHERE connected_components_user_id IS NOT NULL)`
  
  One row per `event_date`. ORDER BY `event_date` ASC. Header comment: timeless feature description of the mart, with a paragraph stating the expected algorithmic invariant `dau_forward_only ≤ dau_backward_fill ≤ dau_connected_components` and explaining why (subsumption — connected-components includes every device that backward-fill considers, which includes every session that forward-only resolves).

- `examples/web_analytics/models/marts/identity_method_comparison.sql` — a `view` that produces three rows of pairwise comparison (`forward_vs_backward`, `forward_vs_connected`, `backward_vs_connected`). Output columns:
  - `comparison_name: TEXT`
  - `total_events: BIGINT`
  - `agree_events: BIGINT` — both non-null AND equal
  - `disagree_events: BIGINT` — both non-null AND not equal
  - `only_left_identified: BIGINT` — left non-null, right null
  - `only_right_identified: BIGINT` — left null, right non-null
  - `both_null_events: BIGINT` — both null
  
  Implementation shape: `UNION ALL` of three SELECTs, each grouping over all events of `gold_eventstream_with_identity` and computing the five counts via `COUNT(*) FILTER (WHERE ...)`. (No GROUP BY column — the SELECT is a single scalar row per comparison; the literal `comparison_name` column distinguishes the three rows.) Header comment explains each comparison's expected shape: forward_vs_backward (and forward_vs_connected) should show large `only_right_identified` and ~0 `disagree_events` (subsumption); backward_vs_connected should show non-zero `disagree_events` and `only_right_identified` (connected-components reassigns cluster representatives across devices).

- `examples/web_analytics/tests/dau_monotonicity_invariants.test.sql` — a `materialization: test` file (per `docs/specs/testing.md`) targeting `daily_active_users_by_method` with a mocked `gold_eventstream_with_identity` `inputs:` block. The mock crafts ~3 distinct dates of ~3 events each, so that on every day all three columns are non-zero and the strict inequality `dau_forward_only < dau_backward_fill < dau_connected_components` holds. The `expect:` block asserts the three DAU values plus `total_events` on each day. At minimum:
  - **Day 1 (2026-04-01).** 4 events on 2 devices in 2 sessions. Device 1 / session 'sa': two anonymous events, one signed-in event for user 100; `forward_only_user_id = 100` on the signed-in event (others depend on the implementation but the test only asserts mart-level aggregates). Device 2 / session 'sb': one anonymous event, `forward_only_user_id = null` everywhere. Backward-fill on device 1 elects user 100; on device 2 elects nothing → `backward_fill_user_id = 100` on three rows, null on one. Connected-components forms two singleton clusters {100} on device 1 and {} on device 2 → `connected_components_user_id = 100` on three rows, null on one (same as backward-fill — Day 1 has no cross-device cluster).
  - **Day 2 (2026-04-02).** 4 events on 2 devices. Device 3: two events with `forward_only_user_id = 200`, `backward_fill_user_id = 200`, `connected_components_user_id = 200`. Device 4: two events with `forward_only_user_id = 201`, `backward_fill_user_id = 201`, `connected_components_user_id = 200` (i.e. the mock simulates devices 3 and 4 being in the same connected-components cluster {200, 201} with cluster id 200, while backward-fill keeps them as 200 and 201 respectively). DAU on Day 2: forward_only = 2 (users 200, 201), backward_fill = 2 (users 200, 201), connected_components = 1 (only user 200 after cluster collapse). Wait — this violates the *direction* of the monotonicity: connected_components has *fewer* distinct users than backward_fill, not more. **This is correct.** The meta-plan §6 invariant is `count(distinct forward_only_user) ≤ count(distinct backward_fill_user) ≤ count(distinct connected_components_user)` — but the *event-level* subsumption goes the other way at the user level: connected-components *clusters* users together, so the DAU under connected-components is ≤ the DAU under backward-fill, not ≥.

  **Read the invariant carefully.** The meta-plan §6 verification gate is stated as `dau_forward_only ≤ dau_backward_fill ≤ dau_connected_components`. This is correct *if* DAU is defined as the count of distinct *identified* users (rows where the identity column is non-null), and the *user attributed to an event* is the resolved column's value — under connected-components, that value is the cluster representative (the smallest user_id in the cluster), so multiple distinct users in a cluster all map to the same representative, which would *reduce* DAU.

  Resolve this carefully by clarifying the convention in the mart header. The Phase 8 mart adopts the **events-count-based subsumption** definition: a "day-active user" under method M is a user_id u such that at least one event on that day has `M_user_id = u`. Under this definition, the meta-plan's stated monotonicity does NOT hold in the multi-device-cluster case (Day 2 above) — connected-components collapses two distinct users to one cluster representative, reducing DAU.

  **Decision.** The plan reinterprets the meta-plan §6 verification gate as written ("monotonic forward_only ≤ backward_fill ≤ connected_components" on the *identified-events* counts, NOT on the distinct-user counts). The mart surfaces both `dau_*` and `identified_events_*` columns. The inline test and the integration-test gate assert the monotonicity on **`identified_events_*`** (which IS monotonic by subsumption — every event that has a non-null forward_only also has a non-null backward_fill, which also has a non-null connected_components, per the Phase 5 → 6 → 7 subsumption invariants). The `dau_*` columns are exposed for human inspection but are NOT asserted as monotonic; the README explains that `dau_connected_components ≤ dau_backward_fill ≤ dau_forward_only` *can* hold (cluster collapse), and the README narrative is built around this richer behaviour.

  **TODO carry-over.** Update the meta-plan and the overall plan's "Verification" section to reflect the corrected invariant. Phase 8's Phase 5 (expert dispatch) will surface this — the sql-expert and the docs-reviewer both have visibility into the gate definition. Address the meta-plan/overall-plan edit as a separate small commit ahead of the status-table flip (commit prefix `chore(web-analytics-8): clarify DAU subsumption invariant`).

  **Inline test rows** (rewritten given the corrected invariant — the test asserts `identified_events_*` monotonicity, not `dau_*` monotonicity):
  - Day 1 (2026-04-01): 4 events. 2 events have `forward_only_user_id` non-null; 3 events have `backward_fill_user_id` non-null; 3 events have `connected_components_user_id` non-null. `dau_forward_only = 1` (user 100 only — the forward-only column resolves only inside the session containing the signed-in observation); `dau_backward_fill = 1` (device 1's canonical user 100, retroactively applied to all 3 events on device 1; device 2's events remain unidentified); `dau_connected_components = 1` (same as backward-fill on Day 1, no cross-device cluster).
  - Day 2 (2026-04-02): 4 events on 2 devices both signed-in. All 4 events have non-null in all three columns (every event has a signed-in observation in its session). `identified_events_*` = 4 on all three methods. `dau_forward_only = 2` (users 200, 201). `dau_backward_fill = 2` (each device elects its respective user). `dau_connected_components = 1` (cluster collapses devices 3 and 4 to user 200).
  - Day 3 (2026-04-03): 2 events on 1 device. 1 event signed-in for user 300, 1 anonymous. `identified_events_forward_only = 1` (only the signed-in event); `identified_events_backward_fill = 2` (both events on device 5, after retroactive election); `identified_events_connected_components = 2` (same as backward-fill, singleton cluster). `dau_*` = 1 on all three methods.

  The `expect:` block contains 3 rows, one per day, with the asserted `(event_date, total_events, dau_forward_only, dau_backward_fill, dau_connected_components, identified_events_forward_only, identified_events_backward_fill, identified_events_connected_components)` tuple.

- `crates/smelt-datagen/tests/example_web_analytics.rs` extension — one new function `test_daily_active_users_by_method_monotonicity`:
  1. Run `smelt-datagen ... --scale-factor 0.01` → `setup_sources.sql` → `smelt build` (same scaffolding as the existing Phase-6 / Phase-7 tests).
  2. Assert `SELECT count(*) FROM main.marts_daily_active_users_by_method > 0`.
  3. Assert one row per `event_date`: `count(*) FROM main.marts_daily_active_users_by_method = count(DISTINCT event_date) FROM main.silver_events_parsed`.
  4. Assert the `identified_events_*` monotonicity invariant on every day: `SELECT count(*) FROM main.marts_daily_active_users_by_method WHERE identified_events_forward_only > identified_events_backward_fill OR identified_events_backward_fill > identified_events_connected_components` must equal 0. This is the meta-plan §6 verification gate verbatim (re-anchored to `identified_events_*` per the decision above).
  5. Assert the `identity_method_comparison` mart materialises with exactly 3 rows and the three expected `comparison_name` values present: `SELECT comparison_name FROM main.marts_identity_method_comparison ORDER BY comparison_name` returns `['backward_vs_connected', 'forward_vs_backward', 'forward_vs_connected']` in order.
  6. Assert pairwise comparison shape: for `forward_vs_backward` and `forward_vs_connected`, `disagree_events = 0` (subsumption: when both are non-null, they agree because backward-fill / connected-components only refines forward-only's resolution by *adding* identifications, never by *changing* them). The narrative bullet in the README depends on this shape.

- `examples/web_analytics/README.md` — replace the Phase 3 stub with a polished walkthrough. Structure (markdown headings shown):
  - `# Web analytics — three-way user stitching`
  - One-paragraph framing: what the example demonstrates (bronze → silver → gold, three identity-resolution algorithms compared side-by-side, marts that quantify the difference).
  - `## Reference` — link to the Amplitude reference doc and a one-sentence explanation that the three algorithms here are *inspired* by (not faithful re-implementations of) the Amplitude track-unique-users methods. Three sub-headers covering each algorithm (Forward-only, Backward-fill, Connected-components) with 2–3 sentences each.
  - `## Pipeline` — bullet list / tree of the bronze → silver → gold → marts lineage. Mention each model with one-line summary; link the SQL files via relative paths.
  - `## Inline tests` — list the four `.test.sql` files and one-line summaries.
  - `## Run locally` — three-step block (`smelt-datagen … --scale-factor 0.01`, `duckdb target/dev.duckdb < setup_sources.sql`, `smelt build`). Add `smelt test` as a fourth step.
  - `## Inspect the marts` — three or four `duckdb`-prompt example queries that surface the algorithmic difference (e.g., "show DAU under each method for the first week", "show the pairwise comparison shape"). Each query is followed by a one-paragraph explanation of what to look for.
  - Link to the overall plan at the bottom under "How this example was built" as a single relative link — this is the only acceptable reference to plan history per the Timeless-oracle rule.
  - **No `Phase N` references** in the README body (the link at the bottom may carry the plan filename, which contains a date prefix but no phase number).

- `examples/README.md` — add a row to the directory table: `| `web_analytics/` | Bronze→silver→gold pipeline over JSON events with three parallel identity-resolution algorithms compared side-by-side | 9 SQL + 4 tests |`. Position alphabetically among the existing rows (after `timeseries/` — `web_` follows `time` alphabetically). The row's "Models" cell is computed from the current file count (`models/bronze/raw_events.sql` + `models/silver/events_parsed.sql`, `models/silver/sessions.sql`, `models/silver/device_user_edges.sql` + `models/gold/identity_forward_only.sql`, `models/gold/identity_backward_fill.sql`, `models/gold/identity_connected_components.sql`, `models/gold/eventstream_with_identity.sql` + `models/marts/daily_active_users_by_method.sql`, `models/marts/identity_method_comparison.sql` = 10 SQL; with the inline tests (forward_only, backward_fill, connected_components, dau_monotonicity, session_boundary) = 5 tests). **Verify the counts at write-time** rather than copying these numbers, in case the implementer adds or merges a model.

- `docs-site/docs/examples/web_analytics.md` — a new docs-site page mirroring the README at a higher level. The page is the discoverable entry from the documentation site. Front matter / structure is consistent with other docs-site pages (no YAML front-matter — the existing docs-site pages use plain markdown with a top-level `# Title`). Content:
  - `# Web analytics example: three-way user stitching`
  - Same one-paragraph framing as the README.
  - `## What this example demonstrates` — bulleted list mirroring the README's Pipeline + Inline tests sections.
  - `## The three identity-resolution algorithms` — same content as the README's Reference section, with the three sub-headers.
  - `## Why three?` — a paragraph explaining the algorithmic tradeoff: forward-only is the simplest (within-session, no retroactive tagging); backward-fill broadens to per-device but does not cross devices; connected-components is the most aggressive (clusters across devices via shared users, can over-cluster if a device is shared between unrelated users). Phase 8's inline test and the integration-test gate are mentioned as the correctness guards.
  - `## Where to find the code` — three or four relative links to the GitHub view of the example directory (`https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics`), the three identity models, and the two marts.
  - **No `Phase N` references**, no plan-vocabulary leak.

- `docs-site/mkdocs.yml` — add a new top-level nav section:
  ```yaml
    - Examples:
      - Web Analytics: examples/web_analytics.md
  ```
  Position immediately after `- Guide:` and before `- Reference:` (matches the conceptual flow: concepts → meta-language → guide → examples → reference → developing). Keep alphabetical ordering within the section (currently only one entry; future examples will be added as new lines).

### Explicitly deferred (scope guardrails)

- **No `time_to_identity` mart.** Per the overall plan's Out of scope: "interesting but adds a fourth concept and isn't needed for the side-by-side comparison."
- **No `daily_unique_devices_by_method` or other per-device DAU variant.** The DAU mart focuses on per-day distinct *users* under each method; per-device counts would be redundant with the existing `silver/device_user_edges` view.
- **No mart-level incremental materialization.** Both marts are views; their upstream `gold/eventstream_with_identity` is itself a view, and the only incremental boundary in the example is `silver/sessions`. If a future phase needs the marts as `table` with `incremental:` (e.g., for dashboard latency), it can be added then — the SELECT shape here is compatible.
- **No CSV / parquet export of the marts.** The marts are queryable in-place from DuckDB; the README's `## Inspect the marts` section shows `duckdb`-prompt queries.
- **No mkdocs build verification inside this autonomy loop.** Running `mkdocs build` requires the docs-site Python toolchain, which is not part of the standard test gate. If the docs-reviewer flags a syntax issue that would break the build, that is the time to escalate (per the "When to pause" list above).
- **No README or docs-site translation / i18n.** Single English-language entry.
- **No connected-components Phase 9 fixed-point.** That is the optional Phase 9 of the overall plan, separately tracked.
- **No edits to `crates/` outside `crates/smelt-datagen/tests/example_web_analytics.rs`.** The smelt language surface is unchanged in this phase — `COUNT(*) FILTER (WHERE ...)` and `COUNT(DISTINCT ...)` are standard SQL and already supported.
- **No `smelt.functions.<name>(...)` calls in model SQL bodies.** Per Phase 3 precedent.
- **No new joint-distribution edges in `datagen.yaml`.** Untouched.

---

## Progress tracking

| Phase | Status | Commit     | Date       |
|-------|--------|------------|------------|
| 1     | done   | `819e4ae9` | 2026-05-18 |
| 2     | done   | `3e9ddd4c` | 2026-05-18 |
| 3     | done   | `99286a7f` | 2026-05-18 |
| 4     | done   | `070c85ee` | 2026-05-18 |
| 5     | done   | `4f577206` | 2026-05-18 |

---

### Phase 1: `marts/daily_active_users_by_method.sql` + DAU monotonicity inline test + integration-test gate

**Goal.** Land `examples/web_analytics/models/marts/daily_active_users_by_method.sql` (the per-day DAU + identified-events mart over `gold/eventstream_with_identity`), the inline `.test.sql` asserting the per-day `identified_events_*` monotonicity invariant on a small mocked dataset, and the integration-test function in `crates/smelt-datagen/tests/example_web_analytics.rs` that runs the full pipeline at `--scale-factor 0.01` and asserts the monotonicity invariant holds on every day of the synthetic data.

**Pre-conditions.** Phase 7 of the overall plan committed (all three identity models exist; `gold/eventstream_with_identity` carries all four identity columns). Working tree clean on `worktree-web_analytics`.

**Required pre-edit (meta-plan / overall-plan invariant correction).** Before writing the mart, edit:
- `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` §6 verification gate bullet 8 to read: "Mart `daily_active_users_by_method` shows monotonic `identified_events_forward_only ≤ identified_events_backward_fill ≤ identified_events_connected_components` on every day in the synthetic dataset. (Note: distinct-user DAU is NOT monotonic in the same direction — connected-components collapses cross-device clusters and can have *fewer* distinct users than backward-fill.)"
- `docs/plans/20260517-web-analytics-example.md` Verification bullet that currently reads "The mart `daily_active_users_by_method` shows the expected monotonic relationship `forward_only ≤ backward_fill ≤ connected_components` on every day in the synthetic dataset." — update to the same `identified_events_*` phrasing.

Commit both edits in a single `chore(web-analytics-8): clarify DAU subsumption invariant on identified_events_*` commit before starting on the mart.

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the mart and inline test land.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_daily_active_users_by_method_monotonicity` — extend the integration suite:
  1. Run `smelt-datagen --scale-factor 0.01` against the temp-dir copy of the example.
  2. Execute `setup_sources.sql`.
  3. Invoke `smelt build --project-dir <tmp> --target dev` and assert exit 0.
  4. Assert `SELECT count(*) FROM main.marts_daily_active_users_by_method > 0`.
  5. Assert one-row-per-day cardinality:
     ```sql
     SELECT count(*) FROM main.marts_daily_active_users_by_method
     = SELECT count(DISTINCT event_date) FROM main.silver_events_parsed
     ```
  6. Assert the monotonicity invariant on every day:
     ```sql
     SELECT count(*) FROM main.marts_daily_active_users_by_method
     WHERE identified_events_forward_only > identified_events_backward_fill
        OR identified_events_backward_fill > identified_events_connected_components
     ```
     must equal 0.
  7. Assert the column-existence regression: a probing SELECT of all eight output columns succeeds without error.
- `examples/web_analytics/tests/dau_monotonicity_invariants.test.sql` — a `materialization: test` file targeting `daily_active_users_by_method`. `inputs:` mocks `gold_eventstream_with_identity` with 10 events across 3 dates (per the Scope §In scope inline-test rows above). `expect:` asserts the three per-day rows with the correct DAU and identified-events counts. The test is meant to catch:
  - A mart that swaps two columns (e.g., applies `COUNT(DISTINCT forward_only_user_id)` to the `dau_backward_fill` column) — caught by the strict-inequality cases on Day 1.
  - A mart that aggregates without `event_date` partitioning — caught by the row-count assertion (3 rows expected, not 1).
  - A mart that drops anonymous events from `total_events` — caught by the `total_events` column assertion.

**Implementation shape (mart).**

```sql
-- Per-day distinct-user and identified-event counts under each of the three
-- identity-resolution algorithms surfaced in gold/eventstream_with_identity.
-- One row per event_date. The identified_events_* columns count events whose
-- corresponding identity column resolved to a non-null user; the dau_*
-- columns count distinct non-null users.
--
-- Invariants the upstream algorithms guarantee:
--   identified_events_forward_only
--     ≤ identified_events_backward_fill
--     ≤ identified_events_connected_components
-- on every day, by subsumption: every event that forward-only identifies is
-- also identified by backward-fill (backward-fill considers the same signed-in
-- observation across all sessions on the device, never fewer); every event
-- that backward-fill identifies is also identified by connected-components
-- (connected-components clusters across devices; a device with a backward-fill
-- canonical user is a non-empty graph node, so it has a cluster).
--
-- DAU is NOT monotonic in the same direction. Connected-components clusters
-- distinct users together (the cluster representative is the smallest user_id),
-- so dau_connected_components can be ≤ dau_backward_fill when a cluster spans
-- distinct users. This is why the mart surfaces both counts: identified_events
-- measures reach (monotonic), dau measures cardinality after collapse
-- (non-monotonic in the cluster-collapse case).
SELECT
    event_date,
    COUNT(*) AS total_events,
    COUNT(DISTINCT forward_only_user_id) AS dau_forward_only,
    COUNT(DISTINCT backward_fill_user_id) AS dau_backward_fill,
    COUNT(DISTINCT connected_components_user_id) AS dau_connected_components,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL) AS identified_events_forward_only,
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL) AS identified_events_backward_fill,
    COUNT(*) FILTER (WHERE connected_components_user_id IS NOT NULL) AS identified_events_connected_components
FROM smelt.gold.eventstream_with_identity
GROUP BY event_date
ORDER BY event_date
```

**Implementation shape (inline test — partial; see Scope for the full `inputs:` and `expect:` shape).**

```sql
--- name: test_dau_monotonicity_invariants ---
materialization: test
test:
  model: daily_active_users_by_method
  inputs:
    gold_eventstream_with_identity:
      # Day 1: 4 events on 2 devices. Only device 1 has a signed-in event.
      - {event_id: 1, device_id: 1, event_user_id: null, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sa', forward_only_user_id: 100, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 2, device_id: 1, event_user_id: 100,  event_ts: '2026-04-01 10:05:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sa', forward_only_user_id: 100, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 3, device_id: 1, event_user_id: null, event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sa', forward_only_user_id: null, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 4, device_id: 2, event_user_id: null, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sb', forward_only_user_id: null, backward_fill_user_id: null, connected_components_user_id: null, connected_components_cluster_id: null}
      # Day 2: 4 events on 2 devices, both signed-in. Devices 3 and 4 are in
      # cluster {200, 201} with cluster_id 200, so connected_components_user_id
      # is 200 on every event, while backward_fill leaves them as 200 and 201.
      - {event_id: 5, device_id: 3, event_user_id: 200, event_ts: '2026-04-02 10:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sc', forward_only_user_id: 200, backward_fill_user_id: 200, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 6, device_id: 3, event_user_id: null, event_ts: '2026-04-02 10:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sc', forward_only_user_id: 200, backward_fill_user_id: 200, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 7, device_id: 4, event_user_id: 201, event_ts: '2026-04-02 11:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sd', forward_only_user_id: 201, backward_fill_user_id: 201, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 8, device_id: 4, event_user_id: null, event_ts: '2026-04-02 11:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sd', forward_only_user_id: 201, backward_fill_user_id: 201, connected_components_user_id: 200, connected_components_cluster_id: 200}
      # Day 3: 2 events on 1 device. 1 signed-in, 1 anonymous.
      - {event_id: 9,  device_id: 5, event_user_id: 300, event_ts: '2026-04-03 10:00:00', event_date: '2026-04-03', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'se', forward_only_user_id: 300, backward_fill_user_id: 300, connected_components_user_id: 300, connected_components_cluster_id: 300}
      - {event_id: 10, device_id: 5, event_user_id: null, event_ts: '2026-04-03 10:05:00', event_date: '2026-04-03', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'se', forward_only_user_id: null, backward_fill_user_id: 300, connected_components_user_id: 300, connected_components_cluster_id: 300}
  expect:
    # Day 1: forward_only identifies 1 event; backward_fill 3; connected_components 3. DAU = 1 for all three.
    - {event_date: '2026-04-01', total_events: 4, dau_forward_only: 1, dau_backward_fill: 1, dau_connected_components: 1, identified_events_forward_only: 1, identified_events_backward_fill: 3, identified_events_connected_components: 3}
    # Day 2: all 4 events identified by all three methods. DAU collapses under connected_components (2→1).
    - {event_date: '2026-04-02', total_events: 4, dau_forward_only: 2, dau_backward_fill: 2, dau_connected_components: 1, identified_events_forward_only: 4, identified_events_backward_fill: 4, identified_events_connected_components: 4}
    # Day 3: forward_only identifies 1 event; backward_fill 2; connected_components 2. DAU = 1 for all three.
    - {event_date: '2026-04-03', total_events: 2, dau_forward_only: 1, dau_backward_fill: 1, dau_connected_components: 1, identified_events_forward_only: 1, identified_events_backward_fill: 2, identified_events_connected_components: 2}
---
```

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/marts/daily_active_users_by_method.sql` (new)
- `examples/web_analytics/tests/dau_monotonicity_invariants.test.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_daily_active_users_by_method_monotonicity` fn)
- `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` (the meta-plan — §6 invariant edit per the "Required pre-edit" block above)
- `docs/plans/20260517-web-analytics-example.md` (the overall plan — Verification bullet edit per the same)
- `docs/plans/20260517-web-analytics-8-marts-readme.md` (this file — committed at the start of Phase 1 work; Progress-tracking row updated in the final commit of Phase 1)

**Docs touched.** None outside this phase's own plan/meta-plan edits (the README polish lands in Phase 3 of this plan; the docs-site page lands in Phase 4).

**Review checklist** (material findings only):

- [ ] Mart produces exactly the 8 columns listed in Scope §In scope; no extra columns leak through.
- [ ] Source reference uses path syntax (`smelt.gold.eventstream_with_identity`) — no dead `smelt.ref()` syntax.
- [ ] `COUNT(*) FILTER (WHERE ...)` and `COUNT(DISTINCT ...)` syntax accepted by smelt-parser (no diagnostics in `example_diagnostics`).
- [ ] Header comment is timeless and explains the monotonicity invariant for `identified_events_*` AND the non-monotonicity of `dau_*` under cluster collapse. Reviewers should specifically check that the *direction* of the inequality is stated correctly: `forward_only ≤ backward_fill ≤ connected_components` on `identified_events_*` (NOT on `dau_*`).
- [ ] The inline `.test.sql` mock includes at least one day where `dau_connected_components < dau_backward_fill` (Day 2 in the example mock) — this is the test that proves the mart distinguishes the two columns correctly.
- [ ] The inline `.test.sql` mock matches the full schema of `gold_eventstream_with_identity` (`event_id, device_id, event_user_id, event_ts, event_date, event_name, platform, url, session_id, forward_only_user_id, backward_fill_user_id, connected_components_user_id, connected_components_cluster_id`).
- [ ] Integration-test invariant query uses `OR` (not `,` — comma is `AND` in WHERE, which would silently mask half the violations).
- [ ] No reach into Phase 2 / Phase 3 / Phase 4 scope of this plan (no `identity_method_comparison` references, no README edits, no docs-site edits).
- [ ] Pre-edit commit to meta-plan + overall plan landed *before* the mart commit, with the `chore(web-analytics-8): clarify DAU subsumption invariant on identified_events_*` message.
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** Pre-edit commit (separate, lands first): `chore(web-analytics-8): clarify DAU subsumption invariant on identified_events_*`.

Phase 1 main commit: `feat(examples): web_analytics daily_active_users_by_method mart + DAU monotonicity test (web-analytics Phase 8)`.

---

### Phase 2: `marts/identity_method_comparison.sql` + integration-test extension

**Goal.** Land `examples/web_analytics/models/marts/identity_method_comparison.sql` — a view producing 3 rows of pairwise comparison across the three identity algorithms (`forward_vs_backward`, `forward_vs_connected`, `backward_vs_connected`), each row reporting agree / disagree / one-side-identified / both-null event counts. Extend the integration test in `crates/smelt-datagen/tests/example_web_analytics.rs::test_daily_active_users_by_method_monotonicity` (or add a sister fn) to assert the mart materialises with 3 rows of the expected `comparison_name` set AND that the subsumption shape `disagree_events = 0` holds for `forward_vs_backward` and `forward_vs_connected`.

**Pre-conditions.** Phase 1 of this plan committed (`marts/daily_active_users_by_method.sql` exists, integration test fn for the DAU monotonicity is in place).

**TDD tests to write first.**

- `cargo test -p smelt-cli --test example_diagnostics` continues to report zero diagnostics for `examples/web_analytics/` after the new mart lands.
- `crates/smelt-datagen/tests/example_web_analytics.rs::test_identity_method_comparison_materializes` — a new fn (separate from Phase 1's monotonicity fn so failures are diagnostically isolated). Identical pipeline scaffolding (datagen → setup_sources → smelt build). Assertions:
  1. `SELECT count(*) FROM main.marts_identity_method_comparison = 3`.
  2. `SELECT comparison_name FROM main.marts_identity_method_comparison ORDER BY comparison_name` returns `['backward_vs_connected', 'forward_vs_backward', 'forward_vs_connected']` in order.
  3. `SELECT disagree_events FROM main.marts_identity_method_comparison WHERE comparison_name = 'forward_vs_backward'` returns 0 (subsumption: when forward-only assigns a user u, backward-fill assigns the same user u — backward-fill never *re*assigns to a different user, only *extends* the population of identified events).
  4. `SELECT disagree_events FROM main.marts_identity_method_comparison WHERE comparison_name = 'forward_vs_connected'` returns 0 (same subsumption reason — forward-only's resolution is the signed-in observation in-session, which is also a member of the device's connected-components cluster; the cluster representative is the cluster min, which need not equal the signed-in user, so this is **NOT** zero by the same argument). **Verify against the synthetic dataset**: this assertion may need to be relaxed to `disagree_events / total_events ratio is small` if the cluster-collapse-changes-the-canonical-user case is common in the synthetic data. The implementer's TDD discovery step is allowed to soften this assertion to a ratio threshold (e.g., `disagree_events <= total_events * 0.1`) or to remove it entirely and document the relaxation in "Deferred during implementation".

     **Subtlety to resolve in discovery.** Backward-fill assigns the per-device canonical user. For a device d in cluster C with backward-fill canonical user b(d), connected-components assigns `connected_components_user_id = MIN(C)`. On every event of device d, `backward_fill_user_id = b(d)` and `connected_components_user_id = MIN(C)`. These agree iff `b(d) = MIN(C)`. For forward-only: on a session s with at least one signed-in observation, `forward_only_user_id = u` where u is the latest signed-in user in that session. On events where forward_only is non-null, `forward_vs_connected` agrees iff `u = MIN(C(device(s)))`. This is NOT always the case — a session whose latest signed-in user is u, but whose device is in a cluster with a smaller-id user, will disagree. The honest assertion is therefore: `forward_vs_backward` has `disagree_events = 0` (subsumption with equal-user); `forward_vs_connected` and `backward_vs_connected` have `disagree_events ≥ 0` and the README describes the expected shape qualitatively. **TDD tests must reflect this**: only the `forward_vs_backward` assertion is strict `= 0`; the other two are not asserted on the synthetic data.

**Implementation shape (mart).**

```sql
-- Pairwise event-level comparison of the three identity-resolution algorithms.
-- One row per pair of methods, reporting the breakdown of every event's
-- (left_user_id, right_user_id) pairing into five disjoint buckets:
--
--   agree_events         — both non-null AND equal
--   disagree_events      — both non-null AND not equal
--   only_left_identified — left non-null, right null
--   only_right_identified — left null, right non-null
--   both_null_events     — both null
--
-- The five buckets sum to total_events. The expected shape under the three
-- algorithms' subsumption relationships:
--
--   forward_vs_backward — disagree_events = 0 (subsumption: backward-fill's
--     per-device canonical user includes every signed-in observation that
--     forward-only saw in-session; when forward-only resolves, it resolves to
--     the same user as the device-level election). only_right_identified is
--     non-zero (backward-fill identifies events that span sessions without a
--     signed-in observation; forward-only does not).
--
--   forward_vs_connected — disagree_events ≥ 0. Forward-only resolves to the
--     latest in-session signed-in user; connected-components resolves to the
--     cluster representative (smallest user_id in the cluster). These differ
--     whenever the device is in a multi-user cluster whose min is not the
--     latest signed-in user.
--
--   backward_vs_connected — disagree_events ≥ 0. Backward-fill's per-device
--     canonical user differs from the cluster representative whenever the
--     cluster spans devices and the device's local election is not the
--     cluster-min user. only_right_identified is 0 (every device that has a
--     backward-fill canonical user is in the connected-components graph as
--     well — both algorithms read the same silver/device_user_edges).
SELECT
    'forward_vs_backward' AS comparison_name,
    COUNT(*) AS total_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NOT NULL AND forward_only_user_id = backward_fill_user_id) AS agree_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NOT NULL AND forward_only_user_id != backward_fill_user_id) AS disagree_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NULL) AS only_left_identified,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND backward_fill_user_id IS NOT NULL) AS only_right_identified,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND backward_fill_user_id IS NULL) AS both_null_events
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'forward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND forward_only_user_id = connected_components_user_id),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND forward_only_user_id != connected_components_user_id),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NULL),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND connected_components_user_id IS NOT NULL),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND connected_components_user_id IS NULL)
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'backward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND backward_fill_user_id = connected_components_user_id),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND backward_fill_user_id != connected_components_user_id),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NULL),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NULL AND connected_components_user_id IS NOT NULL),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NULL AND connected_components_user_id IS NULL)
FROM smelt.gold.eventstream_with_identity
```

Notes:

- No `GROUP BY` is needed because each `SELECT` is a single aggregate row over the entire eventstream. The `comparison_name` literal distinguishes the three rows.
- `UNION ALL` (not `UNION`) — the three rows have distinct `comparison_name` values by construction, but `UNION ALL` is more efficient and is the standard idiom for stitching scalar-aggregate rows.
- `!=` (not `IS DISTINCT FROM`) — see Phase 6 deferred ground rule (`IS DISTINCT FROM` is not supported by smelt-parser).

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/models/marts/identity_method_comparison.sql` (new)
- `crates/smelt-datagen/tests/example_web_analytics.rs` (extension — new `test_identity_method_comparison_materializes` fn)

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Mart produces exactly 3 rows with `comparison_name ∈ {'forward_vs_backward', 'forward_vs_connected', 'backward_vs_connected'}`.
- [ ] Each row's five count columns + `total_events` sum-check: `agree + disagree + only_left + only_right + both_null = total_events`. (Implementer should verify on the synthetic data; reviewers may add a regression assertion.)
- [ ] `forward_vs_backward.disagree_events = 0` holds on the synthetic data (subsumption with equal-user). Integration-test assertion is strict.
- [ ] `forward_vs_connected.disagree_events` and `backward_vs_connected.disagree_events` are **NOT asserted to be zero**; they reflect the cluster-collapse case. README's "Inspect the marts" section explains the expected qualitative shape.
- [ ] `backward_vs_connected.only_right_identified = 0` is a reasonable additional assertion (every device with a backward-fill canonical user is in the connected-components graph too). Implementer should add this if discovery confirms it holds.
- [ ] No `GROUP BY` mistake (the SELECT is single-aggregate-row per UNION ALL branch; adding a GROUP BY would be wrong).
- [ ] `!=` (not `IS DISTINCT FROM`) — parser support already validated.
- [ ] Header comment is timeless and explains each comparison's expected shape qualitatively.
- [ ] No reach into Phase 3 / Phase 4 scope (no README edits, no docs-site edits).
- [ ] Zero diagnostics for the file from `example_diagnostics`.

**Commit.** `feat(examples): web_analytics identity_method_comparison mart (web-analytics Phase 8)`.

**Deferred during implementation.**

- The plan's predicted `forward_vs_backward.disagree_events = 0` invariant does **not** hold on the synthetic dataset (the strict-zero assertion fired at 6304 disagreements). Root cause: forward-only resolves per-session to the *latest* in-session signed-in user; backward-fill elects the per-device *most-frequent* user. On a device with two distinct signed-in users where the most-frequent user is not the latest-in-session user of a given session, the algorithms disagree on every event of that session that forward-only resolves. With 10% shared-device + 5% multi-device users in the linked_choice distribution, disagreement actually *dominates* agreement on this dataset. Resolution: dropped the strict-zero assertion entirely, kept the disjointness-sum invariant (`agree + disagree + only_left + only_right + both_null = total_events`) as the sole correctness check, and updated the mart's `forward_vs_backward` header comment to describe the actual qualitative shape (disagree_events ≥ 0; only_right_identified is the dominant non-agree bucket). The README narrative (Sub-Phase 3) must reflect this honest characterisation.

---

### Phase 3: README polish + `examples/README.md` row

**Goal.** Replace the Phase 3 stub at `examples/web_analytics/README.md` with a full walkthrough (per the Scope §In scope README structure above). Add the `web_analytics/` row to `examples/README.md`. Both files must be timeless: no `Phase N` references in the body. The only acceptable plan-history reference is the relative link to the overall plan at the bottom of the README.

**Pre-conditions.** Phases 1–2 of this plan committed (both marts exist, integration-test gate is green).

**TDD tests to write first.**

- No automated test gates apply directly to README content (rendering is not tested in CI). The proxy gate is `examples-curator` and `docs-reviewer` review in Phase 5, plus a manual visual inspection (the implementer reads the rendered markdown in a viewer or via `glow` / `mdcat` if installed).
- The `cargo test -p smelt-cli --test example_diagnostics` standing gate continues to be green (README edits do not affect SQL diagnostics).
- The implementer is expected to run a syntax sanity check on the markdown by reading the rendered output. No automated assertion.

**Implementation shape (README structure — narrative content is the implementer's responsibility).**

```markdown
# Web analytics — three-way user stitching

A self-contained smelt example demonstrating a bronze→silver→gold pipeline over
JSON-encoded web events, with three parallel user-identity-resolution algorithms
surfaced side-by-side in a single wide event-level table for direct comparison.

## Reference

Inspired by the Amplitude identity-stitching methodology
([docs](https://amplitude.com/docs/data/sources/instrument-track-unique-users)).
The three algorithms below are not faithful reproductions of any Amplitude
implementation, but they cover the same algorithmic spectrum (in-session →
per-device → cross-device).

### Forward-only

… (2–3 sentences)

### Backward-fill

… (2–3 sentences)

### Connected-components

… (2–3 sentences)

## Pipeline

… (lineage tree, with relative links to the SQL files)

## Inline tests

- `tests/session_boundary_invariants.test.sql` — …
- `tests/forward_only_resolution_invariants.test.sql` — …
- `tests/backward_fill_resolution_invariants.test.sql` — …
- `tests/connected_components_resolution_invariants.test.sql` — …
- `tests/dau_monotonicity_invariants.test.sql` — …

## Run locally

```bash
smelt-datagen --config datagen.yaml --scale-factor 0.01
duckdb target/dev.duckdb < setup_sources.sql
smelt build
smelt test
```

## Inspect the marts

… (3–4 `duckdb`-prompt queries with one-paragraph explanations each)

## How this example was built

Multi-session implementation tracked in [`docs/plans/20260517-web-analytics-example.md`](../../docs/plans/20260517-web-analytics-example.md).
```

**Implementation shape (`examples/README.md` row).**

Add the new row alphabetically (after `timeseries/`):

```markdown
| `web_analytics/` | Bronze→silver→gold pipeline over JSON events with three parallel identity-resolution algorithms compared side-by-side | 10 SQL + 5 tests |
```

**Verify the counts at write-time** by running `ls examples/web_analytics/models/**/*.sql examples/web_analytics/tests/*.test.sql | wc -l` and adjust if the implementer has added or merged a model.

**Critical files (allowed to touch in this phase).**

- `examples/web_analytics/README.md` (rewrite)
- `examples/README.md` (one-row edit)

**Docs touched.** Both files listed above. **No `docs-site/` touch in this phase** — that lands in Phase 4.

**Review checklist** (material findings only):

- [ ] No `Phase N` references in the README body — only the single relative link to the overall plan at the bottom is acceptable.
- [ ] The three algorithm descriptions are factually correct and consistent with the SQL files' header comments. The `forward-only` description does NOT say "before signup" — it says "within session".
- [ ] The lineage tree in `## Pipeline` lists every model in the example (3 silver + 4 gold + 2 mart = 9 models, ignoring the bronze view which is `models/bronze/raw_events.sql` for 10 total; verify the count).
- [ ] The `## Inspect the marts` queries actually return the columns advertised (no copy-paste typo in column names — `dau_forward_only`, not `forward_only_dau`).
- [ ] The `examples/README.md` row's `Models` count is computed correctly (10 SQL + 5 tests as of Phase 8 completion).
- [ ] The plan link at the bottom is a relative link (`../../docs/plans/20260517-web-analytics-example.md`) and resolves correctly from the README's location.
- [ ] No mention of Phase 9 or any speculative / future work in the body.

**Commit.** `docs(examples): polish web_analytics README and add to examples index (web-analytics Phase 8)`.

---

### Phase 4: docs-site page + nav update

**Goal.** Create `docs-site/docs/examples/web_analytics.md` with the structure described in Scope §In scope. Add a new top-level `Examples:` section to `docs-site/mkdocs.yml`, with one entry `Web Analytics: examples/web_analytics.md`, positioned between `Guide:` and `Reference:`.

**Pre-conditions.** Phase 3 of this plan committed (the README is polished; the docs-site page mirrors its high-level structure).

**TDD tests to write first.**

- No automated CI gate runs `mkdocs build` in the autonomy loop. The proxy gate is `docs-reviewer` review in Phase 5. The implementer is expected to run a syntax sanity check by reading the page in a markdown viewer; if `mkdocs` is installed locally, `mkdocs build` is a stronger check but is not required (see "When to pause and ask the user" — installing the docs-site toolchain is out of loop scope).
- Standing `cargo test -p smelt-cli --test example_diagnostics` gate remains green.

**Implementation shape (page).**

```markdown
# Web analytics example: three-way user stitching

A self-contained smelt example showing how to build a bronze → silver → gold
pipeline over JSON-encoded web events and compare three parallel
user-identity-resolution algorithms side-by-side. The example lives under
[`examples/web_analytics/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics).

## What this example demonstrates

- Decoding JSON-encoded payloads via a smelt function (silver/events_parsed).
- Incremental sessionization with a 30-minute inactivity boundary
  (silver/sessions, 7-day lookback).
- Three parallel identity-resolution algorithms surfaced as columns of a single
  wide event-level table (gold/eventstream_with_identity).
- Inline `.test.sql` assertions for each algorithm's defining invariant.
- Marts (gold/marts) that quantify per-day algorithmic divergence.

## The three identity-resolution algorithms

Inspired by the
[Amplitude identity-stitching methodology](https://amplitude.com/docs/data/sources/instrument-track-unique-users)
— covering the same algorithmic spectrum (in-session → per-device → cross-device)
without being faithful re-implementations.

### Forward-only

Within-session resolution: when a session contains at least one signed-in
event, every event in that session is retroactively attributed to that signed-in
user. Events in sessions with zero signed-in events stay anonymous.

### Backward-fill

Per-device canonical user: the user with the most signed-in observations on
the device wins, retroactively tagging every prior anonymous event across all
sessions on that device. Ties broken by earliest `first_seen`. Each device is
independent.

### Connected-components

Cross-device union-find: build the bipartite graph (device nodes, user nodes,
edges from co-occurrence in signed-in events), then resolve each device to the
smallest user_id in its connected component. Implemented as iter-capped
recursive label propagation.

## Why three?

… (paragraph explaining the algorithmic tradeoff and that the marts make the
divergence visible)

## Where to find the code

- [`examples/web_analytics/models/gold/identity_forward_only.sql`](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/gold/identity_forward_only.sql)
- [`examples/web_analytics/models/gold/identity_backward_fill.sql`](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/gold/identity_backward_fill.sql)
- [`examples/web_analytics/models/gold/identity_connected_components.sql`](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/gold/identity_connected_components.sql)
- [`examples/web_analytics/models/marts/daily_active_users_by_method.sql`](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/marts/daily_active_users_by_method.sql)
```

**Implementation shape (nav update).**

Add after the `- Guide:` block, before `- Reference:`:

```yaml
  - Examples:
    - Web Analytics: examples/web_analytics.md
```

Match the existing 2-space indent and `- Title: path/to/page.md` format used elsewhere in the nav.

**Critical files (allowed to touch in this phase).**

- `docs-site/docs/examples/web_analytics.md` (new)
- `docs-site/mkdocs.yml` (one-block addition to nav)

The `docs-site/docs/examples/` directory does not exist yet — create it as part of writing the new page.

**Docs touched.** Both files listed above.

**Review checklist** (material findings only):

- [ ] No `Phase N` references in the page body.
- [ ] Relative links from the docs-site page to the GitHub view of the example resolve correctly (`https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics`). Cross-check against the repo URL declared in `docs-site/mkdocs.yml:4` (`repo_url: https://github.com/adbrowne/smelt-sql`).
- [ ] The page's three algorithm descriptions are consistent with the README's wording (not necessarily identical, but factually aligned).
- [ ] mkdocs nav YAML syntax is valid (proper indentation; one entry per line). The implementer can sanity-check by re-reading the file and matching indentation with the surrounding `- Guide:` and `- Reference:` blocks.
- [ ] The new `Examples:` section is positioned between `Guide:` and `Reference:` (not appended at the very bottom — examples are conceptual entry points, not reference material).
- [ ] No reach into Phase 5 scope (no expert dispatch happens here).

**Commit.** `docs(site): add Web Analytics example page and nav entry (web-analytics Phase 8)`.

---

### Phase 5: Expert reviewer dispatch loop

For each expert listed below, dispatch via the Agent tool with `model: "sonnet"`,
brief prompt, the per-phase plan path, the spec path (if relevant), and the
in-repo plan path. The expert returns a list of findings classified as
"material" or "stylistic". Address material findings:

  - For each material finding, either edit directly (small) or dispatch a
    nested implementer subagent (larger).
  - Commit the fix with message `review(web-analytics-8): address {expert-name} feedback`.
  - Push.
  - Re-dispatch the same expert. Loop until the expert returns "no material findings".

Bounds:

  - Max 3 rounds per expert. If unresolved after 3 rounds → emit
    `<<PAUSE_FOR_HUMAN>>`.
  - If two different experts flag the same systemic concern in one round →
    emit `<<PAUSE_FOR_HUMAN>>`.

Experts for this phase (from meta-plan §5 row 8):

  - `sql-expert` — focus per meta-plan §5: **mart aggregation correctness**. Specifically:
    - Both marts compute the right counts. `COUNT(DISTINCT col)` ignores NULL in DuckDB — the expert should verify this is what the mart wants (yes — anonymous events should not contribute to DAU). `COUNT(*) FILTER (WHERE ... IS NOT NULL)` is the standard idiom; the expert should verify the parser accepts it.
    - The monotonicity invariant is asserted on `identified_events_*` (not `dau_*`). The pre-edit to the meta-plan and overall plan correctly re-anchors the §6 verification gate; the expert should verify the re-anchoring is consistent and that the SQL test query in the integration test matches.
    - The `forward_vs_backward.disagree_events = 0` invariant is justified by subsumption AND by equal-user (forward-only and backward-fill, when they both resolve, both resolve to a signed-in observation on the device — backward-fill never *changes* the user, only extends the population). The expert should verify the SQL query encodes "non-null AND equal" correctly (`forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NOT NULL AND forward_only_user_id = backward_fill_user_id`) — a `forward_only_user_id = backward_fill_user_id` alone would be NULL-tainted.
    - The mart-level disjointness sum: `agree + disagree + only_left + only_right + both_null = total_events`. The expert should verify this sum-check holds analytically across all five `FILTER` predicates (the predicates are mutually exclusive and exhaustive over the (left non-null, right non-null) × (=, ≠) crosstab plus the two single-null and the both-null cases).
    - The `UNION ALL` shape of `identity_method_comparison` is preferred over three separate views or a self-join. The expert should verify the column types and ordering match across the three branches (DuckDB's `UNION ALL` requires aligned types).
    - The inline `.test.sql` mock for DAU monotonicity has a row that demonstrates `dau_connected_components < dau_backward_fill` (Day 2 in the mock). The expert should verify this is asserted in the `expect:` block.
    - No parser-unsupported constructs (`IS DISTINCT FROM`, `LATERAL`, etc. — `example_diagnostics` catches these but the expert may spot a semantic-level issue).

  - `examples-curator` — focus on example-pipeline quality:
    - The marts fit the bronze→silver→gold→marts pattern. `daily_active_users_by_method` and `identity_method_comparison` are materialised under `models/marts/`, a new subdirectory consistent with `examples/retail_analytics/models/marts/`.
    - The README polish replaces the stub fully and reads as a self-contained walkthrough. A reader who has never seen the example should be able to (a) understand what it demonstrates, (b) run it locally, (c) know what queries to run to see the algorithmic difference. The "Inspect the marts" `duckdb`-prompt queries are concrete and the explanations are calibrated to the reader (not assuming prior knowledge of identity-stitching jargon).
    - The `examples/README.md` row positions `web_analytics/` correctly in alphabetical order and the model-count number is accurate.
    - No mention of unimplemented Phase 9 work in the README body.
    - The README's "How this example was built" link is the only acceptable plan-vocabulary leak. The expert should flag any other Phase-N reference in the README.
    - The integration tests in `example_web_analytics.rs` are consistent with the existing tests' structure (TempDir + copy_dir_all + rewrite_outputs + run_datagen + rewrite_setup_sources_sql + setup_sources + smelt build + assertions). The expert should flag any drift from that established pattern.

  - `docs-reviewer` — focus on docs-site rendering and discoverability:
    - The new `docs-site/docs/examples/web_analytics.md` follows the existing pages' conventions (no YAML front-matter, top-level `# Title`, internal links are relative where possible).
    - The new `Examples:` nav section is correctly positioned in `mkdocs.yml`. Indentation matches the surrounding blocks.
    - All external links (Amplitude reference, GitHub view of the example dir, GitHub view of individual files) resolve to live URLs.
    - The docs-site page complements the README without duplicating it — the README has the "Run locally" steps and the "Inspect the marts" queries; the docs-site page focuses on conceptual framing and the "Where to find the code" links.
    - Code fences in the docs-site page declare the right language (` ```bash`, ` ```sql`).
    - Markdown is valid (no broken tables, no half-closed code fences). The expert can verify by reading the file in a markdown viewer; if a stronger check is needed, the expert may run `mkdocs build` locally (but this requires the Python toolchain; if not installed, defer to the visual check).

If a literal `sql-expert`, `examples-curator`, or `docs-reviewer` agent type does not exist, dispatch `general-purpose` with a prompt that frames it as such (read the plan + diff, flag plan/impl drift, missing test cases, scope creep into later phases, docs-rendering issues — material findings only).

**Loop discipline.**

1. **Round 1.** Dispatch all three experts (in parallel) with `model: "sonnet"`. The prompt MUST include:
   - This plan's path and the oracle paths (overall plan, meta-plan, Phase 3 / Phase 4 / Phase 5 / Phase 6 / Phase 7 plan "Deferred during implementation" sections, `testing.md`).
   - The exact file scope from the per-sub-phase tables above.
   - The diff range to review: commits since the start of Phase 8 of the overall plan (typically the four `feat/docs(examples)/docs(site)/chore: ... (web-analytics Phase 8)` commits — `git log --oneline {phase-8-base}..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, scope creep, missing test cases, plan/impl drift, docs-rendering issues that would break the build, missing internal links). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` if the expert's tool palette allows nested subagents.

2. **Address findings.** For each material finding:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics`, and the `crates/smelt-datagen/tests/example_web_analytics.rs` integration tests after each fix batch.
   - Commit per round: `review(web-analytics-8): address {expert-name} feedback`.
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch the same expert with the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → expert is clean and exits.

4. **Repeat** until all experts return clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - Any single expert flags a material finding on round 3 (per-expert bound).
   - Two different experts flag the same systemic concern in one round (cross-expert bound).
   - An expert's findings would force a spec change. Run `/smelt:spec` on the relevant slug first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 8.
   - The DAU monotonicity invariant on the synthetic dataset is violated (meta-plan §7 stop-the-line condition 5 verbatim).
   - The docs-site build fails (if the implementer or an expert ran `mkdocs build` and surfaced a YAML/markdown error that the autonomy loop's environment cannot resolve without the Python toolchain).

**Critical files (allowed to touch in this phase).** Anything within the experts' scope per the table above, plus
`docs/plans/20260517-web-analytics-8-marts-readme.md` (to record round counts
and the final clean status) and `docs/plans/20260517-web-analytics-example.md`
(to flip the overall-plan status row).

**Review checklist** (applied to the expert-dispatch *process*, not to a code diff):

- [ ] `sql-expert` dispatched at least once.
- [ ] `examples-curator` dispatched at least once.
- [ ] `docs-reviewer` dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`,
  `cargo test -p smelt-cli --test example_diagnostics` (zero diagnostics for `examples/web_analytics/`), the end-to-end integration tests (including the new `test_daily_active_users_by_method_monotonicity` and `test_identity_method_comparison_materializes`), and the inline `.test.sql` invariant tests all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 5 expert review: sql-expert clean (R{n}), examples-curator clean (R{m}), docs-reviewer clean (R{k}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 8 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

(Phase 9 — the optional true-fixed-point follow-up — remains `pending` after Phase 8 completes. The autonomy loop should emit `<<PHASE_COMPLETE>>` (not `<<ALL_DONE>>`) so that the next iteration can either begin Phase 9 or — if the user has decided to skip Phase 9 — return `<<ALL_DONE>>` after observing that all *required* phases (1–8) are `done`. The fresh-context iteration's logic in §3 row 9 of the meta-plan is the trigger: "Phase 9 (optional) replaces the iter-cap with a true fixed-point.")

**Commit(s).** Per round, per expert with findings:
`review(web-analytics-8): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-8): mark Phase 8 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the scope is satisfied at the end of Phase 5:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — no regression in the workspace.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `cargo test -p smelt-datagen --test example_web_analytics` passes — including the new `test_daily_active_users_by_method_monotonicity` and `test_identity_method_comparison_materializes` sub-tests, in addition to all pre-existing tests.
- Manual fresh-checkout dry run succeeds:
  ```bash
  smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01
  duckdb examples/web_analytics/target/dev.duckdb < examples/web_analytics/setup_sources.sql
  smelt build --project-dir examples/web_analytics --target dev
  smelt test --project-dir examples/web_analytics
  ```
- The `identified_events_*` monotonicity invariant holds on every day in the synthetic dataset (the integration-test assertion is the canonical check; the inline `.test.sql` is a structural check on the mart itself).
- The `examples/web_analytics/README.md` renders correctly in a markdown viewer; all internal links resolve.
- The `docs-site/docs/examples/web_analytics.md` page exists and the new `Examples:` nav section appears between `Guide:` and `Reference:` in `docs-site/mkdocs.yml`.
- Phase 5 acceptance gate met: `sql-expert`, `examples-curator`, and `docs-reviewer` all reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 8 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
