# Phase 4 summary — presentation projection: state columns invisible to consumers

**Shipped:**
- New pure emitter `presentation_projection` (`smelt-logical`'s `maintenance/emit.rs`, next to
  `state_augmented_projection`): rewrites wildcard select items via CST `range()` splicing —
  never a whole-text scan. A bare `*` over a single state-bearing relation expands to its
  unqualified presented columns; over multiple relations, expands the state-bearing relation(s)
  qualified and leaves siblings as `<rel>.*`; `<alias>.*` expands only when the alias resolves to
  a state-bearing relation. Refuses (`PresentationRefusal::UnresolvableWildcard`) when a wildcard's
  relation can't be named while a state-bearing ref is in scope; returns the SQL byte-identical
  when none is. 6 new tests in `emit_statements.rs`.
- `SqlCompiler::state_bearing_models: BTreeSet<String>` + `set_state_bearing_models`/
  `CompilerRegistry::set_state_bearing_models_all` (mirrors `set_upstream_schemas`). A private
  `presentation_map()` derives the presented-column map from `upstream_schemas.models` at compile
  time — no second source of truth. `hide_state_columns` calls `presentation_projection` in
  `compile()`/`compile_with_sql()`/`compile_with_sql_and_ephemerals()` right after meta-eval
  expansion, before parsing/printing — while `smelt.models.*` ref text is still present, so a
  refusal names the user's path. 2 new unit tests in `compile.rs`.
- `execute.rs`: new `build_state_bearing_models` helper classifies every `refresh: keyed` model
  with `classify_cumulative_sql` and collects which carry `AggregatorColumn.state.is_some()`;
  wired at both `set_upstream_schemas_all` call sites (dry-run ~709, main path ~986). Always empty
  today (admission still closed) — correct the moment rows 5-6 populate `state`.
- `smelt-db`: no code change needed. `model_schema`/`resolved_model_schema` derive from the
  model's own written select list, which never contains `__part` columns (those are appended only
  by `state_augmented_projection` at compile time in `smelt-runtime`). Locked in by
  `public_schema_excludes_state_columns` (`src/tests.rs`).
- Spec: `docs/specs/incremental_models.md` §"Decomposed state (rung 2)…" → "Presentation
  projection" gained the two sentences the plan specified (wildcard-rewrite mechanism + hard
  refusal).

**Decisions:**
- 2026-08-09: the rewrite point is `compile()`/`compile_with_sql*()` on the pre-print SQL text
  (`smelt.models.*` still literal), not `apply_type_casts`'s post-print SQL (physical table names)
  — the plan required refusal messages to name the user's path.
- 2026-08-09: `state_bearing_models` on `SqlCompiler` is a bare `BTreeSet<String>` (membership
  only); the presented-column values are derived on demand from `upstream_schemas.models` inside
  `presentation_map()` rather than duplicated into the set, per the plan's "no new source of truth"
  instruction.
- 2026-08-09: test 9's diagnostic assertion uses `type_diagnostics`/`UndeclaredColumn`, not
  `file_diagnostics`/`ColumnTypeUnresolved` as originally sketched — a hand-written
  `agg.avg_amount__sum` reference against a resolvable upstream model fires `UndeclaredColumn`
  (the column-existence check), which is the more precise diagnostic for "this name isn't in the
  model's public schema" and was what the harness actually produced.

**For the next planner:**
- Rows 5-6 (admission) can now populate `AggregatorColumn.state` freely — `build_state_bearing_
  models` and `presentation_projection` are both live and will pick it up automatically with no
  further wiring.
- No end-to-end fixture exists yet (a real keyed model whose downstream consumer's `SELECT *` gets
  rewritten against real DuckDB) — that's row 7's job per the outcome's existing reshape.
- Discovered but out of scope: `crates/smelt-db/src/tests.rs`'s `TestDb` harness resolves refs
  differently for `file_diagnostics` (`resolve_ref_path`, needs full project/address-index setup)
  than for `model_schema`/`type_diagnostics` (`resolve_ref`) — a plain multi-file `file_diagnostics`
  test with a legitimately-defined upstream ref reports a spurious `UndefinedModelRef`. Not
  touched (pre-existing, unrelated to this phase); worth a follow-up ticket if another phase hits
  it.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`
  workspace, example_diagnostics).
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage` — pass (35 + 4 tests).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — pass (18 + 4).
- `cargo test -p smelt-cli --test maintenance_conformance` — pass (47 tests, unchanged verdicts).
- `cargo test -p smelt-db --lib` — pass (564 tests, +1 new).
- `cargo test -p smelt-runtime --lib compile::` — pass (36 tests, +2 new).
