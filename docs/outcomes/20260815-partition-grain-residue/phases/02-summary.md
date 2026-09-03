# Phase 2 summary — function-registry-threaded classification

## Shipped

- `crates/smelt-logical/src/analysis/temporal.rs`: `analyze_temporal_dependencies` now walks
  every FROM-clause derived table and CTE body reachable from the AST
  (`analyze_select_recursive`), not just the outer `SELECT` — a window/frame a function-call
  expansion introduces as a derived table is now seen exactly as an inline one. Two new unit
  tests (`derived_table_window_is_seen`, `cte_body_window_is_seen`) pin this.
- `crates/smelt-runtime/src/safety.rs`: `build_model_graph` now takes `fn_bodies: &FnBodyMap`
  and stores each model's `sql` as `expand_function_calls(&model.content, fn_bodies)` — the
  single call site both `check_bound_derivation` and `check_planner_safety` classify from.
  `execute.rs:607` threads the already-built `fn_bodies` through.
- `crates/smelt-cli/src/commands/explain.rs`: the `opt_graph` feeding `Planner::plan` (physical
  section of `smelt explain`) now uses expanded SQL too — it previously fed raw content.
- `crates/smelt-ui/src/build.rs` (`build_model_details`, `build_run_plan`) and `api.rs`
  (`post_run_plan` now takes `db`): both batch-safety call sites expand before classifying.
- **Root-cause fix, wider than the plan's stated scope**: `crate::compile::expand_function_calls`
  (the standalone helper, `compile.rs:773`) never stripped frontmatter before parsing. Every
  production caller passes raw `model.content`/`model_file.content` (frontmatter included, since
  every real model has one) — the parser choked on the leading `---` block and the printer fell
  back to verbatim, unexpanded text. This affected `execute.rs:5219`'s **real windowing/chunk
  computation** (`compute_incremental_windows_ordered` → `batch_safety_for_model` →
  `derive_and_classify_bounds`), used by every live `smelt run`/`rebuild`/backfill — not just the
  explain/UI classification paths phase 2 targeted. Fixed by stripping frontmatter
  (whitespace-preserving) inside `expand_function_calls` itself, so every caller benefits.
- `crates/smelt-runtime/tests/classification_expansion.rs` (new): `bound_derivation_sees_define_body_lookback`,
  `batch_safety_sees_define_body_window`, `no_fn_bodies_is_identity`, and the structural gate
  `every_production_model_info_uses_expanded_sql` (scans `smelt-{runtime,cli,ui}/src` for a
  `ModelInfo { sql: model.content.clone() }` anti-pattern).
- `crates/smelt-logical/src/analysis/temporal.rs`: fixed a latent unit-misdetection bug in
  `find_interval_in_text`'s combined-string construction — it appended the *entire rest of the
  document* after a quoted interval value when hunting for a bare-unit suffix (`INTERVAL '3'
  DAY`), so a short interval (e.g. `INTERVAL '5 minutes'`) near an unrelated `DAY`-typed interval
  later in the same statement misclassified as 5 *days*. Bounded the lookahead to 16 chars.
- Spec: `docs/specs/incremental_shapes.md` §"The partition grain" Known Divergences — removed the
  two now-landed bullets; §"Functions inside partition-grain bodies" — added the descent sentence.
- Two `docs/plans/20260530-thread-fn-registry-classification.md`-tracked phase-1 probes moved
  (inverted) from `smelt-logical/tests/partition_residue_probes.rs` to the new runtime-layer test
  file; module doc updated to note the move.

## Decisions

- Fixed `expand_function_calls`'s frontmatter bug at its root (inside the shared helper) rather
  than at each call site — every caller needed it, and a per-call-site fix would have left the
  same bug live for any future caller.
- `examples/web_analytics/models/silver/sessions.sql` (and its `tutorial_stages/05_enrichment`
  copy) needed `safety_overrides.allow_window_functions: true` added: once function expansion
  correctly threads through, the planner's window/`PARTITION BY`-alignment rule sees
  `smelt.functions.sessionize`'s internal `OVER (PARTITION BY device_id, CAST(event_ts AS
  DATE))` for the first time — a genuine (previously invisible) non-alignment. Per the model's own
  documented closed-form proof, safety here comes from the declared `RANGE BETWEEN INTERVAL '2
  days' PRECEDING` lookback frame, not partition-alignment — matching `events_parsed.sql`'s
  existing precedent for the same escape hatch. This is a **true positive** per the plan's task 6
  guidance, not a classifier bug — the classifier was fixed, not narrowed.
  2026-09-04 decision logged in outcome.md.
- `sessions`'s bound-based batch-safety chunk size changed from a stale, never-actually-expanded
  12-day-window computation to its now-correctly-composed value — `rebuild_dry_run.rs`'s golden
  chunk boundaries (`bounded_safe(chunk=7d,context=2d)`, 5×7-day chunks) were themselves computed
  against the always-broken expansion path; regenerated to the correct 3×12-day chunking
  (`smelt explain`'s `before=P4D` composed bound × 3). No classifier change was needed here — the
  golden values were simply stale.
- Regenerated `docs-site/docs/examples/web-analytics/sessions.md` via
  `examples/web_analytics/generate_tutorial.py` (only that page changed) and fixed one hardcoded
  LSP goto-def line number (`crates/smelt-lsp/tests/example_workspaces.rs`) that shifted because
  `sessions.sql` grew a documented `safety_overrides` block.

## For the next planner

- The `expand_function_calls` frontmatter fix is load-bearing far beyond phase 2's stated scope —
  it corrects the real execution windowing path for **every** project with a `smelt.define` body
  containing a lookback and frontmatter (i.e. every real project). Worth a note in
  `docs/specs/incremental_shapes.md` §Design if a future redraft wants the history; not urgent.
- `analyze_expr_temporal` (temporal.rs) never inspects a window function nested inside a `CASE`
  expression's arms — only a select-list item that is *itself* a function call with `.window_spec()`
  is checked. `sessions.sql`'s own `sessionize` body has exactly this shape (`MIN(...) OVER (...)`
  inside a `CASE ... ELSE ... END`) and it is silently invisible to the AST walk (only the
  whole-text advisory fallback might catch an INTERVAL-bearing frame, and this one has no frame at
  all — default-frame, so it should classify `Unbounded` if seen). This is a pre-existing gap
  (unrelated to phase 2's own target, and `temporal.rs` is advisory-only per
  `docs/specs/architecture.md`'s property-composition-walk rule, so it feeds no admission gate) —
  but it means `analyze_batch_safety`/`smelt explain`'s `bounded_safe(...)` label for a
  CASE-wrapped window can still under-report. Not in phase 2's scope (temporal.rs is advisory);
  flagging for awareness only.
- Phases 3–8 remain `pending`; phase 3 (CTE-only `event_time_column` detection) is next.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 4/4 pass.
- `cargo test -p smelt-runtime --test classification_expansion` — 4/4 pass (new).
- `cargo test -p smelt-logical --test partition_residue_probes` — 2/2 pass (remaining probes).
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — 3/3 pass (untouched).
