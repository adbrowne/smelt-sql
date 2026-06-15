# Plan: Sherlock-feedback fixes (parser, types, incremental, config)

**Date**: 2026-06-16
**Spec**: per-phase — `docs/specs/types.md` §3, `docs/specs/incremental_models.md`, targets/config spec
**Tracking branch**: `worktree-analytics_fixes`
**Docs**: code+docs
**Spec increments**: pre-authorized in Phases 2, 6, 8, 9 (the human who approved this plan authorizes those spec edits; all other phases are code+test only).

---

## Execution prompt (for a fresh Claude session / the sweep loop)

You are executing this plan one phase per iteration. Find the next phase whose
status is `pending` in the Progress-tracking table — that is your phase. Run it
end-to-end via `/smelt:implement`:

- Red-green TDD: write the failing test(s) listed in the phase first.
- Implementer subagent → reviewer subagent → iterate → commit + push.
- Exercise the feature in a real `examples/` fixture, not just unit tests.
- Honor architectural invariants from root `CLAUDE.md` (Salsa purity: pure
  functions in `type_inference/`; thin Salsa wrappers).
- **Timeless-oracle rule**: phase vocabulary lives in *this plan file only*.
  Edits to `docs/specs/*` and `docs-site/docs/*` describe the feature as if it
  has always existed — no `Phase N` headings/labels in spec or user-doc bodies.
- Set the phase row to `done` with the commit hash + date; use the phase's
  `Commit.` line verbatim. When the LAST pending phase lands, flip this
  sub-plan's Status to `done (<today>)` in the master registry table
  (`docs/plans/20260530-feature-sweep.md` → `## Spawned sub-plans`) in the same
  commit.
- **Spec increments are allowed ONLY in the phases that explicitly list one**
  (Phases 2, 6, 8, 9). All other phases must not edit specs; log spec-vs-code
  conflicts and block instead.

**Record-and-continue (block rule).** If a phase hits a design decision not
answered by this plan or the spec, or pre-flight `cargo test` is red on
something that is NOT this phase's own acceptance target: set the row to
`blocked` with a one-line reason, append a dated entry to "## Blocked phases",
restore a clean committed tree, commit+push, and move on.

---

## Context

While building the Sherlock mart layer (~1B-row Meridian events, DuckDB target,
monthly incremental marts) the user logged nine issues in
`~/analysis/sherlock/docs/smelt-feedback.md` spanning the parser, type
inference, incremental execution, and target configuration. Several are
silent-correctness bugs (wrong numbers, dropped rows). The load-bearing insight:
for the SQL defects the real bug is usually a **test-coverage gap** — the
parser/type property generators never exercise the failing constructs and never
nest them inside CTEs, which is exactly where Sherlock broke. So every
parser/type fix is paired with a generator extension that catches the whole
class, not just the reported instance.

### Assessment: issue → confirmed root cause → phase
| # | Issue | Root cause (verified in code) | Phase |
|---|-------|-------------------------------|-------|
| 7 | `--per-partition` drops finer-than-partition rows | `windowing.rs:118-145` tiles per-partition batches by **fixed** `granularity_days` (Month=30, `:273`); drifts off calendar months — loss grows with month index. Calendar alignment already exists at `:179-243` but the tiling loop ignores it | 1 |
| 3 | `int/int → BIGINT` (0.92 stored as 1) | `promote_numeric_operands_for_op` (`binary.rs:146-191`) falls through to integer-preserving arms for `/`; spec §3 mandates truncating; oracle masks it (`generators.rs:1005` excludes `/`; `divergences.rs:127-162` ByDesign) | 2 |
| 1 | Named `WINDOW` clause errors (esp. in CTE) | `parse_select_stmt` (`select.rs:14`) never parses a `WINDOW` clause; CTE reuses same fn — unsupported everywhere; top-level "worked" only via lossless error-recovery passthrough | 3 |
| 2 | `INTERVAL n DAY` won't parse | `is_typed_literal` (`expr.rs:1559`) only accepts `INTERVAL`+`STRING` | 4 |
| — | Parser generators miss the above + never nest in CTEs | `proptest_generators.rs` emits no `WINDOW`/numeric-`INTERVAL`, no CTE nesting | 5 |
| 5 | event_time predicate fails on UNION/aggregate models | `inject_time_filter` (`transformer.rs:272-313`) appends `WHERE` to outermost SELECT; `incremental.rs:194-200` only checks the column appears *somewhere* | 6 |
| 4 | One broken model aborts every `--select` run | `check_parse_errors` (`run_setup.rs:101-124`) gates **all** models before `--select` resolves (`run.rs:66`) | 7 |
| 6/FR1 | No DuckDB tuning on target | `Target` (`config.rs:120-139`) has no settings; `DuckDbBackend::new` (`smelt-backend-duckdb/src/lib.rs:52-84`) opens with defaults | 8 |
| FR2 | Wide incremental window OOMs as one query | windowing defaults to a single batch when `FullyBatchSafe`; no size guard | 9 |

---

## Progress tracking
| Phase | Status | Commit | Date |
|-------|--------|--------|------|
| 1 | done | | 2026-06-16 |
| 2 | done | 378a37ca | 2026-06-16 |
| 3 | done | fbf9f0f0 | 2026-06-16 |
| 4 | done | b5ebe1d2 | 2026-06-16 |
| 5 | pending | | |
| 6 | pending | | |
| 7 | pending | | |
| 8 | pending | | |
| 9 | pending | | |

---

### Phase 1 — Calendar-aware `--per-partition` tiling (data loss, #7)
**Goal.** Per-partition (and calendar-granularity) batching advances by **calendar
units** (1 month/quarter/year), not a fixed `granularity_days` step, so a mart
whose output grain is finer than its partition grain keeps every row.

**Pre-conditions.** None.

**TDD tests first.**
- `crates/smelt-runtime/tests` (windowing): tile a 24-month range with
  `per_partition=true`; assert each batch is exactly one calendar month
  (28/29/30/31 days), boundaries on month starts, full coverage, zero drift.
  Fails today (fixed 30-day step drifts).
- Execute-level fixture: an incremental mart partitioned by month
  (`granularity: month`, `partition_column: month_start`) emitting **daily** rows;
  a `--per-partition` backfill over ≥3 months keeps all days for every month
  (the `mart_metric_daily` shape). Currently drops a growing tail.

**Implementation shape.** In `crates/smelt-runtime/src/windowing.rs`, replace the
fixed-day tiling loop (`:134-145`) for Month/Quarter/Year with calendar stepping,
reusing the alignment logic at `:179-243`; keep Day/Week fixed-day. Ensure each
batch's `partition_start/end`, the DELETE window, and the injected time filter
agree on true calendar boundaries.

**Critical files.** `crates/smelt-runtime/src/windowing.rs`; runtime windowing +
execute tests; a fixture under `examples/`.

**Spec increment (pre-authorized).** `docs/specs/incremental_models.md`: state
per-partition batching is calendar-aligned for Month/Quarter/Year and output
grain may be finer than partition grain.

**Review checklist.**
- [ ] Calendar tiling: each Month/Quarter/Year batch lands on true boundaries; no drift over 24 months
- [ ] Day/Week unchanged
- [ ] DELETE window, partition window, and time filter agree per batch
- [ ] Daily-grain-in-monthly-partition fixture keeps all days every month
- [ ] `cargo test -p smelt-runtime` incl. `execute_parity` green

**Commit.** `fix(incremental): calendar-aligned per-partition batching preserves sub-partition-grain rows`

---

### Phase 2 — `/` infers Double + oracle coverage (#3)
**Goal.** Numeric `/` infers `DOUBLE` (DuckDB/Spark-aligned); the property oracle
actually generates division and no longer masks it.

**Pre-conditions.** None (independent of Phase 1).

**Spec increment (pre-authorized).** Amend `docs/specs/types.md` §3 from "Integer
division is truncating" to "`/` is float division returning Double" — rewrite the
rationale paragraph (the smelt-internal type now matches the backend result;
no truncation). Update `docs/type_semantics.md`'s truncating-int-division note
accordingly.

**TDD tests first.**
- Type-inference unit tests: `Integer/Integer`, `BigInt/BigInt`,
  `SmallInt/SmallInt`, and `Integer / NULLIF(SUM(Integer),0)` all infer `Double`.
- Rewrite the existing `integer_division_still_truncating` test (in
  `crates/smelt-db/tests/decimal_arithmetic_tests.rs`) to
  `integer_division_returns_double`.
- Oracle: add a nested-CTE division case (`WITH inner AS (…), outer AS (SELECT
  a/b FROM inner) …`) that passes against DuckDB.

**Implementation shape.** In `crates/smelt-db/src/type_inference/binary.rs`,
special-case `op == "/"` for numeric non-Decimal operands to return
`DataType::Double` (keep the Decimal-`/` rejection at `:92-108`; Float/Double
already promote). In `tests/prop_helpers/generators.rs:~1005` add `/` to the
generated ops (non-Decimal operands). In `tests/prop_helpers/divergences.rs:127-162`
remove/flip `integer_division`/`smallint_division`/`bigint_division`/`float_division`
so a regression to integer fails the oracle. Add a nested-CTE assembly variant
to the generators.

**Critical files.** `crates/smelt-db/src/type_inference/binary.rs`;
`crates/smelt-db/tests/prop_helpers/{generators,divergences}.rs`;
`crates/smelt-db/tests/decimal_arithmetic_tests.rs`; `docs/specs/types.md`;
`docs/type_semantics.md`; an `examples/` ratio fixture.

**Review checklist.**
- [ ] Numeric `/` (all integer families) infers `Double`; Decimal-`/` rejection unchanged
- [ ] `integer_division_returns_double` replaces the truncating test
- [ ] Generators emit `/`; the four division divergences removed/flipped
- [ ] Nested-CTE division case in the oracle, green vs DuckDB
- [ ] Spec §3 + type_semantics edits are timeless
- [ ] `cargo test -p smelt-db --test type_property_tests` green

**Commit.** `fix(types): division returns Double (DuckDB/Spark-aligned); oracle covers division + nested CTEs`

---

### Phase 3 — Parser: named `WINDOW` clause (#1)
**Goal.** `SELECT … WINDOW w AS (…)` parses at top level **and** inside a CTE
(one shared `parse_select_stmt`), round-trips losslessly.

**Pre-conditions.** None.

**TDD tests first.** In `crates/smelt-parser/src/parser/tests.rs`: parse
`SELECT x, sum(y) OVER w FROM t WINDOW w AS (PARTITION BY x ORDER BY y)` with no
error node; same inside `WITH c AS (…) SELECT * FROM c`; assert parse→print→parse
stability.

**Implementation shape.** Add a `WINDOW_CLAUSE` syntax kind (`syntax_kind.rs`),
parse it in `parse_select_stmt` (`select.rs`) in clause order (after
HAVING/QUALIFY, before ORDER BY), reusing `parse_window_spec` (`expr.rs:1584`);
add the AST wrapper + printer coverage.

**Critical files.** `crates/smelt-parser/src/{syntax_kind.rs,parser/select.rs}`,
AST/printer module, `parser/tests.rs`.

**Docs.** SQL-surface spec if one exists; else none.

**Review checklist.**
- [ ] WINDOW clause parses top-level and in CTE — no error node
- [ ] Round-trip (parse→print→parse) stable
- [ ] `parse_window_spec` reused (no duplicate window grammar)
- [ ] `cargo test -p smelt-parser` green

**Commit.** `feat(parser): named WINDOW clause in SELECT (top-level and CTE)`

---

### Phase 4 — Parser: `INTERVAL n DAY` numeric forms (#2)
**Goal.** `INTERVAL 1 DAY`, `INTERVAL (n) DAY`, and `n * INTERVAL 1 DAY` parse,
alongside the existing `INTERVAL 'str'` form.

**Pre-conditions.** None.

**TDD tests first.** `parser/tests.rs`: the three forms above parse with no error
node and round-trip stably; the string form still parses.

**Implementation shape.** Extend the INTERVAL path (`expr.rs:~537` /
`is_typed_literal:1559`) to accept `INTERVAL <number|paren-expr> <unit-kw>`;
confirm the unit-keyword set (DAY/MONTH/YEAR/HOUR/MINUTE/SECOND/WEEK…); preserve
the string form.

**Critical files.** `crates/smelt-parser/src/parser/expr.rs`, `parser/tests.rs`.

**Review checklist.**
- [ ] Numeric, parenthesized, and multiplier INTERVAL forms parse + round-trip
- [ ] String form unaffected
- [ ] `cargo test -p smelt-parser` green

**Commit.** `feat(parser): numeric and parenthesized INTERVAL forms`

---

### Phase 5 — Parser generators: WINDOW + INTERVAL + CTE nesting (deeper guard)
**Goal.** The round-trip generators emit named WINDOW clauses, numeric INTERVAL
forms, and **wrap any generated SELECT inside a CTE/subquery**, so the
"works-top-level-breaks-in-CTE" class is permanently guarded.

**Pre-conditions.** Phases 3–4 done (the constructs now parse).

**TDD/verification.** Extend `crates/smelt-parser/tests/proptest_generators.rs`
and run `proptest_round_trip`; the new constructs round-trip; add a CTE-wrapping
combinator covering both `WITH c AS (…)` and scalar-subquery positions.

**Critical files.** `crates/smelt-parser/tests/proptest_generators.rs`,
`crates/smelt-parser/tests/proptest_round_trip.rs`.

**Review checklist.**
- [ ] Generators emit WINDOW + numeric INTERVAL
- [ ] CTE-wrapping combinator exercises constructs nested one level down
- [ ] `proptest_round_trip` green with extended generators

**Commit.** `test(parser): generators emit WINDOW/INTERVAL and nest constructs in CTEs`

---

### Phase 6 — Incremental: validate event_time visibility + diagnostic (#5)
**Goal.** A clear smelt diagnostic when `event_time_column` isn't resolvable at
the outer SELECT, replacing DuckDB's opaque "column not found".

**Pre-conditions.** None.

**Spec increment (pre-authorized).** Document the injection point + requirement in
`docs/specs/incremental_models.md`; add the new diagnostic code to
`docs/specs/diagnostics.md`.

**TDD tests first.** A UNION/aggregate model omitting the event-time column at the
outer SELECT yields the new diagnostic (naming column + model + remedy), not a
DuckDB error; the single-SELECT case still builds clean.

**Implementation shape.** Before `inject_time_filter`
(`crates/smelt-runtime/src/transformer.rs:272-313`), validate the column is
resolvable at the outer SELECT using existing schema/column-origin analysis in
`smelt-db`; emit the diagnostic via the standard path. Strengthen the check at
`crates/smelt-logical/src/rules/incremental.rs:194-200`. Push-to-source recorded
under "Deferred".

**Critical files.** `crates/smelt-runtime/src/transformer.rs`;
`crates/smelt-logical/src/rules/incremental.rs`; `smelt-db` diagnostic code +
`map_metadata_error_to_diagnostic` exhaustiveness if a new `MetadataError`
variant is added; specs above; an `examples/` UNION-incremental fixture.

**Review checklist.**
- [ ] UNION/aggregate model without the column → new diagnostic (column + model + remedy)
- [ ] Single-SELECT incremental still builds
- [ ] Diagnostic code in catalogue; coverage gate green
- [ ] `cargo test -p smelt-runtime` and `-p smelt-cli --test example_diagnostics` green

**Commit.** `feat(incremental): validate event_time_column visibility with a precise diagnostic`

---

### Phase 7 — CLI: scope parse-error gating to the selected subgraph (#4)
**Goal.** A broken **unrelated** model no longer aborts `smelt run --select
<good_model>`; errors in the selected subgraph (incl. transitive deps) still
block.

**Pre-conditions.** None.

**TDD tests first.** `smelt-cli` test: a workspace with one broken unrelated model
runs `--select <good_model>`; a `--select` that depends on the broken model still
fails; the whole-workspace gate is preserved when no `--select` is given.

**Implementation shape.** Resolve the `--select` subgraph before gating; restrict
`check_parse_errors` (`run_setup.rs:101-124`, called at `run.rs:66`) to the
selected models + their transitive deps.

**Critical files.** `crates/smelt-cli/src/commands/{run.rs,run_setup.rs}`.

**Docs.** Note `--select`-scoped diagnostic gating in the CLI/run spec if present.

**Review checklist.**
- [ ] Broken unrelated model doesn't block `--select` of a good model
- [ ] Broken model that IS a transitive dep still blocks
- [ ] No-`--select` whole-workspace gate unchanged
- [ ] `cargo test -p smelt-cli` green

**Commit.** `fix(cli): scope parse-error gating to the --select subgraph`

---

### Phase 8 — Config: target-level `settings:` for DuckDB (#6/FR1)
**Goal.** `targets.<t>.settings: { memory_limit, threads, temp_directory, … }`
applied on connection open; unknown keys fail loud.

**Pre-conditions.** None.

**Spec increment (pre-authorized).** Add `settings:` to the targets/config spec
and the `docs-site/` targets page.

**TDD tests first.** Config parses a `settings:` map; a DuckDB backend test
asserts `memory_limit`/`threads` take effect (read back via `current_setting`);
an unknown/invalid setting errors rather than being silently ignored (fail-loud
discipline).

**Implementation shape.** Add `settings: Option<BTreeMap<String,String>>` to
`Target` (`config.rs:120-139`); thread through `backend_factory.rs:41-69` into
`DuckDbBackend::new` (`smelt-backend-duckdb/src/lib.rs:52-84`), applying each as
`SET k = v;` right after `Connection::open`.

**Critical files.** `crates/smelt-core/src/config.rs`;
`crates/smelt-cli/src/backend_factory.rs`;
`crates/smelt-backend-duckdb/src/lib.rs`; targets spec + `docs-site/`.

**Review checklist.**
- [ ] `settings:` parses from `smelt.yml`
- [ ] `memory_limit`/`threads`/`temp_directory` applied on open (verified via `current_setting`)
- [ ] Unknown setting fails loud
- [ ] Spec + docs-site edits timeless
- [ ] `cargo test -p smelt-backend-duckdb` / config tests green

**Commit.** `feat(config): target-level DuckDB settings (memory_limit/threads/temp_directory)`

---

### Phase 9 — Incremental DX: warn on wide single-batch builds (FR2)
**Goal.** Warn (don't change defaults silently) when an incremental build spans
many partition periods in a single batch — the OOM footgun — recommending
`--per-partition`/`--batch-size`.

**Pre-conditions.** Phase 1 done (per-partition is now correct, so the
recommendation is safe).

**Spec increment (pre-authorized).** Document the heuristic + levers in
`docs/specs/incremental_models.md`.

**TDD tests first.** Windowing/CLI test: the warning fires for a wide single-batch
range and not for bounded ones.

**Implementation shape.** In `windowing.rs` (single-batch `FullyBatchSafe` path,
`:125`) compute the period span and surface a warning via the reporter when above
a threshold. Pairs with Phase 8 (memory_limit spill) as the real guard.

**Critical files.** `crates/smelt-runtime/src/windowing.rs`; reporter path;
`docs/specs/incremental_models.md`.

**Review checklist.**
- [ ] Warning fires for wide single-batch ranges, not bounded ones
- [ ] No silent default change
- [ ] `cargo test -p smelt-runtime` green

**Commit.** `feat(incremental): warn when a single-batch build spans many partitions`

---

## Blocked phases

(Append-only. The loop adds dated entries here when it blocks a phase.)

---

## Deferred during implementation
- Push the event_time predicate down to the source scan (Phase 6 ships
  validate+diagnostic only).
- Auto-default (vs warn) per-partition for heavy models (Phase 9 warns only).

---

## Verification (whole sub-plan)

`cargo fmt --all -- --check`, `cargo clippy --all-targets` clean; `cargo test`
green; targeted: `cargo test -p smelt-parser` (+`proptest_round_trip`),
`cargo test -p smelt-db --test type_property_tests`, `cargo test -p smelt-runtime`
(windowing + `execute_parity`), `cargo test -p smelt-cli --test example_diagnostics`,
`cargo test -p smelt-lsp --test example_workspaces`, `cargo test -p smelt-backend-duckdb`.
Reproduce the Sherlock shapes in `examples/` (monthly-partition daily-grain mart,
UNION incremental model, ratio column, WINDOW/INTERVAL model, `settings:` target)
and confirm each formerly-failing case builds with correct output.

## References
- Feedback log: `~/analysis/sherlock/docs/smelt-feedback.md`
- Master sweep: `docs/plans/20260530-feature-sweep.md`
