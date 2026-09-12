# Phase 6f summary — `LAG`/`LEAD` window frames elided on SparkSQL

## Shipped

- `docs/specs/multi_backend.md` §"Frame elision on offset functions" (new subsection, between
  "Statement-level lowering" and "Cross-engine emission audit").
- `RewriteId::ElideWindowFrame` (`crates/smelt-types/src/signatures/emission/position_rewrite.rs`),
  registered on `LAG`/`LEAD` at `(DialectId::SparkSql, Position::Window)` **and**
  `(DialectId::SparkSql, Position::WholePartitionWindow)` (`crates/smelt-types/src/signatures/
  builtins/window.rs`) — both window positions carry the verdict because the coverage-totality
  gate (`registry_coverage::position_retired::window_verdict_totality`) requires either both or
  neither, and Spark refuses a frame regardless of whether it happens to cover the whole
  partition.
- `crates/smelt-dialect/src/frame_elision.rs` — a new, small, **live**-decision module (`pub(crate)
  fn should_elide(frame: &SyntaxNode, dialect: &SqlDialect) -> bool`), the only place outside
  `printer/` that reads a `WINDOW_FRAME`'s parent `WINDOW_SPEC` and its sibling call to resolve a
  registry verdict.
- `printer/mod.rs` gained a `SyntaxKind::WINDOW_FRAME` dispatch arm that calls `should_elide` and,
  when true, prints nothing but the frame's own trailing trivia (`registry_emit::
  push_trailing_trivia`, now `pub(crate)`) so a following comment or the closing `)` is not lost.
  `apply_rewrite`'s new `RewriteId::ElideWindowFrame` arm returns `false` — the call itself prints
  natively; the frame is dropped separately, where the printer visits the `WINDOW_FRAME` node.
- Tests: `smelt-types` `registry_coverage::emission::lag_and_lead_elide_frame_on_sparksql`;
  `smelt-dialect` new `tests/frame_elision.rs` (5 tests: elision on `LAG`/`LEAD`, DuckDB
  byte-identity, a non-offset window function's frame kept, a named-window `LAG`'s frame kept —
  i.e. never silently dropped); `smelt-cli --features databricks`
  `github_activity_databricks::actor_sessions_compiles_without_window_frame` (pins the exact
  post-elision shape of `sessionize`'s two `LAG` calls, and that `MAX`'s frame survives).
  `docs/reference/dialect-coverage.md` regenerated (`LAG`/`LEAD` SparkSQL cells: `native` →
  `rewrite:ElideWindowFrame`).

## Decisions

- **Elision is decided live, at print time, from the `WINDOW_FRAME` node's own sibling
  structure — not planned ahead against the model's `syntax` tree like `restructure::plan`.**
  The plan's literal design (a `frame_elision::plan(root, dialect) -> Vec<TextRange>` pre-pass,
  threaded into a new `PrintContext.elided_frames` field mirroring `settled_emissions`) would not
  have fixed the actual failing model: `silver.actor_sessions`'s `LAG` calls live inside
  `functions/sessionize.sql`, a `smelt.define` body that is inlined by **textual re-parse at print
  time** (`printer::reexpand_call_body`), so those calls never appear in the top-level model's
  `syntax` tree a pre-pass over it would walk — confirmed by reading `settled_verdict_for`
  (`emission_settle.rs`), which already falls back to a live, range-lookup-miss settlement for
  exactly this reason. `RewriteId::BigQueryMedian`'s existing live-decision shape (position
  re-derived per visit, no precomputed range list) already proves this pattern reaches reexpanded
  bodies correctly; the new module follows it instead. No `PrintContext` field was added — nothing
  needs cross-visit state, because a `WINDOW_FRAME` node and its call are visited together, inside
  the same single recursive descent, in whichever tree (original or reexpanded) is currently
  printing.
- **A named-window `LAG`/`LEAD` (`OVER w`, frame on a shared `WINDOW w AS (...)` clause) is not
  eligible for elision.** The frame is not the call's own sibling there (`window_spec_sibling`'s
  precondition), so `should_elide` returns `false` and the frame reaches Spark unchanged — a live
  refusal from the engine, not a silently wrong answer. This is the safe direction per the plan's
  own fallback framing ("refused rather than silently emitted"); building a proper diagnostic-time
  refusal for this edge case was judged out of this phase's scope (`sessionize.sql` and every
  live-run model use only inline frames).
- **The dialect-audit's own probe coverage for `LAG`/`LEAD` needed no new work.** `Position::Window`
  probes derive with no window frame at all (confirmed reading `probe.rs`), so the schema/value
  legs never exercise a framed `LAG` on a live Spark engine — the smelt-cli integration test is
  what actually proves the regression scenario compiles correctly; `cargo test -p smelt-db --test
  dialect_audit` needed only a doc-sync regen (`SMELT_REGEN_DOCS=1`), no probe-injection mechanism.
  Left as a gap for the next planner (see below), not built here.

## For the next planner

- **A new, unrelated live blocker now occupies the failure point.** The full refresh
  (`20260912-124833-91da82`) moved from 11 success / 1 failed / 4 skipped to **14 success / 1
  failed / 1 skipped**: `silver.actor_sessions` and its 3 former dependents now all succeed
  (confirmed — `silver.actor_sessions` completed with 957 rows). The one remaining failure is
  `gold.events_enriched`: `Feature not supported by Spark SQL: key-addressed model-edge
  affected-key discovery over a KeyedUpsert upstream (group-grain fingerprint-sidecar diff)`, with
  `marts.star_growth` skipping as its dependent. This is squarely a maintenance-layer capability
  gap on Spark, unrelated to window frames or `LAG`/`LEAD` — recorded, not fixed, since 14/16
  models still complete and the outcome's own "only fix what's needed to complete at all" exception
  does not apply. Same "one construct clears, the next one at the same failure point is exposed"
  shape as 6c → 6d → 6e → 6f. Left for row 7's planner exactly as prior rows left their own
  findings.
- **The dialect-audit probe framework has no mechanism to inject an explicit window frame into a
  derived `Position::Window` probe** (`overrides.rs` only lets you override a call's `args`/
  `spelling`, not its surrounding `OVER (...)`), so `ElideWindowFrame` is unverified by the
  standing cross-engine audit end-to-end — only by the targeted `frame_elision.rs`/
  `github_activity_databricks.rs` tests and this phase's own live run. Worth a follow-on note in
  row 10's findings handoff if a future `RewriteId` needs the same treatment.
- Rows 7-11 are otherwise unaffected; nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `cargo test -p smelt-dialect --test emission_ownership` — 11/11 ok.
- `cargo test -p smelt-dialect --test frame_elision --test template_emission` — 5/5, 7/7 ok.
- `cargo test -p smelt-types --test registry_coverage` — 107/107 ok.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance` — 20/20,
  4/4 ok.
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks` — 7/7 ok.
- `cargo test -p smelt-db --test dialect_audit` — 61/61 ok (after `SMELT_REGEN_DOCS=1` regen of
  `docs/reference/dialect-coverage.md`).
- `bash .claude/scripts/verify-phase.sh` — all green (fmt, clippy both feature sets, shellcheck,
  full `cargo test` workspace, `example_diagnostics`).
- Live: `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
  2026-08-05 --event-time-end 2026-08-07` → run `20260912-124833-91da82`, **14 success / 1 failed
  / 1 skipped** (up from 11/1/4).
