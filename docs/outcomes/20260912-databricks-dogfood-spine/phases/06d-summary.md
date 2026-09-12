# Phase 6d summary — source-cast spelling fixed and confirmed live; a new, unrelated blocker now stands at the same seam

## Shipped

- `crates/smelt-dialect/src/type_conformance.rs`: `source_cast_type_sql(type_text, dialect)`,
  the single owner of per-dialect spelling shared with the cast-wrap's own `type_cast_sql` —
  parses a source-written type string and returns `Some(spelling)` only when it differs from
  the canonical spelling the source already stands for. 5 new unit tests.
- `crates/smelt-dialect/src/printer/mod.rs` + `printer/rewrites.rs`: `TYPE_SPEC` now dispatches
  to `print_type_spec`, which calls the shared owner and substitutes the spelling while
  preserving leading/trailing trivia byte-for-byte; falls through to verbatim `print_children`
  on a parse miss or no-op rewrite. No `SqlDialect` branch or function-name match added to
  `printer/` — `emission_ownership` stays green.
- `docs/specs/multi_backend.md` §"Output-schema type conformance": a new paragraph stating the
  rule applies to every cast smelt prints, source-written or wrap-synthesized alike, naming the
  SparkSQL bare-`VARCHAR`/`TEXT` → `STRING` spelling and the shared owner.
- Two new printer tests (`cast_target_spelling_is_dialect_aware`,
  `double_colon_cast_target_spelling_is_dialect_aware`) plus a corrected pre-existing test and
  snapshot (`test_double_colon_varchar_rewrite_spark` / `double_colon_varchar_rewrite_spark`)
  that had encoded the bug's own behaviour (`CAST(name AS VARCHAR)` unchanged on Spark) as its
  expectation.
- `crates/smelt-cli/tests/github_activity_databricks.rs::actor_sessions_compiles_without_bare_varchar`
  — compiles `silver.actor_sessions` for `--target databricks --dry-run`, no workspace needed,
  asserts no ` AS VARCHAR)` survives.

## Decisions

- Routed `TYPE_SPEC` through `print_node`'s own dispatch (unconditional, not capability-gated)
  rather than only from `print_cast_rewrite`'s `::` path, so a plain `CAST(x AS VARCHAR)` (no
  `::`, `supports_double_colon_cast` irrelevant) gets the same spelling — this is what
  `actor_sessions.sql`'s own `CAST(actor_id AS VARCHAR)` needed.
- `source_cast_type_sql` returns `None` (verbatim passthrough) rather than a guessed spelling on
  a `parse_type` miss, matching the plan's explicit test 4 and the fail-loud-adjacent posture the
  wrap function already has — a malformed source type is not this fix's problem to diagnose.

## For the next planner

**The live full refresh confirms the fix but does not reach 16/16 — a second, unrelated blocker
now sits at the exact same failure point.** Run `20260912-110921-5ecbf6` (target `databricks`,
`--full-refresh --allow-full-refresh --event-time-start 2026-08-05 --event-time-end 2026-08-07`):
**11 success / 1 failed / 4 skipped** (16 total) — the identical shape 6c left, but the failing
model's error has changed. `bronze.events`'s compiled SQL confirms the fix is live (`CAST(actor_id
AS STRING)`, not `VARCHAR`). The new failure:

```
silver.actor_sessions: Execution failed for 'spark sql': AnalysisException:
[UNRESOLVED_ROUTINE] Cannot resolve routine `epoch_us` on search path
[`system`.`session`, `system`.`builtin`, `system`.`ai`, `workspace`.`smelt_dogfood`].
SQLSTATE: 42883; line 52 pos 21
```

`examples/github_activity/functions/sessionize.sql:46` calls `epoch_us(ts_col) -
epoch_us(_prev_ts)` — a DuckDB-only builtin with **no registry entry at all** (not merely
un-emitted for Spark): `rg epoch_us crates/smelt-types/src/signatures/` finds nothing. It reaches
the Spark backend printed verbatim because no registry entry means no dialect check ever runs
against it (the compile-path refusal gate only catches a registry entry marked
`Emission::Unsupported`, not a name absent from the registry altogether). Same three models still
skip as `actor_sessions`'s dependents. This is squarely outside phase 6d's own boundary (cast-target
spelling) and a run still completes 11/16, so the outcome's own "only fix what's needed to
complete at all" exception does not apply here either — recorded, not fixed, exactly as 6b/6c's
findings were. Row 7's planner has the same two options 6c's summary posed: a small registry-entry
fix ahead of the incremental windows (register `epoch_us` — likely `date_diff('microsecond', ...)`
or `unix_micros`/`CAST(... AS DOUBLE) * 1e6` on SparkSQL — through the `BuiltinRegistry` per the
Function-registry single-ownership invariant), or an explicit exclusion of `silver.actor_sessions`
and its 3 dependents from criteria 6/7/8's scope. The former keeps the model set whole; the fix
shape is a normal `Signature` addition, not a structural one.

Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `cargo test -p smelt-dialect --lib type_conformance --quiet` — 14 passed.
- `cargo test -p smelt-dialect --quiet` (full crate incl. snapshots) — all passed after fixing
  the stale snapshot/assertion.
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --quiet` — 17
  passed.
- `cargo test -p smelt-runtime --test projection_dialect_invariance --test dialect_seam --quiet`
  — 24 passed.
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --quiet` — 5
  passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).
- Live: `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
  2026-08-05 --event-time-end 2026-08-07` → run `20260912-110921-5ecbf6`, **11 success / 1
  failed / 4 skipped** (not the target 16/0/0 — see "For the next planner").
