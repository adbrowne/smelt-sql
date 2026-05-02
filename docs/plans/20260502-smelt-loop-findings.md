# Plan: smelt-loop findings — spec stubs, bug fixes, and user docs

**Date**: 2026-05-02 (refined 2026-05-03)
**Tracking branch**: worktree-example-usage
**Spec**: `docs/specs/cli.md` (new), `docs/specs/seeds.md` (new), `docs/specs/functions.md` (update — fix example inconsistency, add placement table), `docs/specs/architecture.md` (update — fix matching example inconsistency), `docs/specs/incremental_models.md` (update — add default materialization), `docs/specs/types.md` (update — add aggregate inference subsection)

**Spec diff**: Phase 1 authors the spec stubs; later phases implement against them.

**Source**: accumulated TOOL_BUG and DOCS_GAP findings from `/smelt-loop --tier small --mode local --iterations 5` (2026-05-02) and one medium-tier run (2026-05-03).

## Context

The loop surfaced four bugs (TB-1 through TB-5) and ten documentation gaps (DG-1 through DG-10). The highest-leverage finding is TB-5 (function path-prefix not enforced): wrong paths silently succeed, `UnknownSmeltFn` never fires, and medium-tier iterations pass because build agents discover the workaround. None of the bugs or gaps had a pre-existing normative spec; this plan creates them so fixes can't silently regress.

Phase 1 is documentation-only. Phases 2–5 are code fixes, each preceded by TDD tests. Phase 6 is user-doc updates for all DG items.

## Phase order

```
1. Spec stubs + spec corrections  (doc-only — must land first)
2. TB-5: function path-prefix enforcement   (spec exists after Phase 1)
3. TB-2: seed DATE→TEXT narrowing            (spec exists after Phase 1)
4. TB-1: --verbose wiring                    (spec exists after Phase 1)
5. TB-4: --version flag                      (trivial clap change)
6. User docs: all DG items consolidated      (after Phases 2–4 so docs describe correct behavior)
```

**TB-3** (no project-wide compile-only flag) is **deferred** — it is design-shaped (decide `--show-plan` scope or a new `--dry-run`). The cli.md stub in Phase 1 captures it as an open question so the decision is tracked.

## Progress tracking

| Phase | Topic | Status | Date | Commit |
|-------|-------|--------|------|--------|
| 1 | Spec stubs + corrections | done | 2026-05-03 | 87b00cc |
| 2 | TB-5: path-prefix enforcement | done | 2026-05-03 | fbb9f3f |
| 3 | TB-2: seed DATE→TEXT | done | 2026-05-03 | ec0dc2e |
| 4 | TB-1: --verbose wiring | done | 2026-05-03 | 9fff792 |
| 5 | TB-4: --version flag | done | 2026-05-03 | 0ba461e |
| 6 | User docs (all DG items) | done | 2026-05-03 | d4a02e4 |

---

## Phase 1 — Spec stubs and spec corrections (doc-only)

**Goal.** Author missing specs and correct two known-bad examples before any code changes. Every subsequent phase fixes code to match a spec; without this phase there is nothing to regress-check against.

**No code changes. No test changes. Doc files only.**

### 1a. New: `docs/specs/cli.md`

Stub spec for the CLI surface. Minimum sections to capture for this plan:

**Surface to specify:**
- `smelt --help` and `smelt --version` — `--version` is a required top-level flag; absence is the TB-4 bug.
- `smelt build --verbose` — must log compiled SQL for each model immediately before execution. A run where all models are up-to-date produces no extra output (models don't execute). TB-1 is the known divergence (currently logs nothing even when models run).
- `smelt build` flag truth-table:
  - `--dry-run` — **not** a flag on `smelt build`. `smelt run --dry-run` exists.
  - `--show-plan <MODEL_FILE>` — per-model, requires a positional argument.
  - `--select` — repeatable flag; space-separated values in a single `--select` fail silently or are rejected.
  - `--verbose` — logs compiled SQL; see above.
- TB-3 (open question): whether `--show-plan` with no positional argument should compile the whole project, or whether a separate project-wide `--dry-run` is warranted. **Document as open, defer implementation.**

**Known Divergences to record:** TB-1 (verbose not wired), TB-3 (no project-wide dry-run), TB-4 (--version missing).

### 1b. New: `docs/specs/seeds.md`

Stub spec for seed loading. Minimum sections:

**Surface to specify:**
- Seed CSV → table name: `seeds/<name>.csv` loads as table `<name>`. `seeds/<subdir>/<name>.csv` loads as `<subdir>.<name>`.
- Seed ref syntax in models: top-level seeds are addressed as `smelt.models.<name>` (same namespace as SQL models). Subdirectory seeds are `smelt.sources.<schema>.<name>`.
  - **DG-7 close:** make the filename→ref mapping an explicit example in the Surface section.
- Type inference — two passes that can disagree:
  - **At runtime (DuckDB):** `read_csv_auto()` infers `INTEGER`, `BIGINT`, `DOUBLE`, `BOOLEAN`, `DATE`, `TIMESTAMP`, `VARCHAR`.
  - **At compile time (smelt LSP / `smelt table`):** simpler inferencer, recognises only `BOOLEAN`, `INTEGER`, `DOUBLE`, `TEXT`. Date- and timestamp-shaped columns show as `TEXT`. `smelt table` reflects the compile-time schema, not DuckDB's runtime schema.
  - **TB-2 as Known Divergence:** passthrough `DATE` columns infer as `TEXT` in the type checker and materialise as `VARCHAR` in downstream models. The fix (aligning compile-time inference with DuckDB's output) lives in Phase 3; once fixed, move this from Known Divergences to Semantics.
- Workaround until TB-2 is fixed: cast explicitly in the first staging model (`CAST(col AS DATE)`).

### 1c. New: `docs/specs/smelt_yml.md`

Stub spec for `smelt.yml` project configuration. Minimum sections:

**Surface to specify:**
- Top-level keys: `name` (string), `version` (string), `model_paths` (list, default `["models"]`), `seed_paths` (list, default `["seeds"]`), `targets` (map).
- Unknown top-level keys produce a warning, not an error (forward-compatible).
- **DG-4 close:** make the defaults explicit; cross-link to `reference/smelt-yml`.
- `unstable_schema: true` gate (already referenced in `functions.md`).

### 1d. Update: `docs/specs/incremental_models.md`

Add one sentence to Surface §"YAML frontmatter": if `materialization:` is omitted, the default is `table`. **DG-9 close.**

### 1e. Update: `docs/specs/functions.md`

Fix incorrect example in Surface §"Function call syntax" (line 66):

> ❌ `smelt.define helper(...)` declared in `random/x.sql` is called as `smelt.random.x.helper(...)`
> ✅ `smelt.define helper(...)` declared in `random/x.sql` is called as `smelt.random.helper(...)`

The rule is: path = workspace-relative **directory** + **declared function name**. The filename stem is not a path component. This is consistent with the table in `architecture.md` (which correctly shows `functions/patterns/x.sql` declaring `session_rollup` → `smelt.functions.patterns.session_rollup`). **TB-5 spec pre-work.**

Also add to Surface §"Function call syntax" a placement table (**DG-10 close for the WHERE/HAVING half**):

> `Expr<Boolean>`-returning functions are valid in any boolean position: `WHERE`, `HAVING`, `CASE WHEN`, `JOIN ON`, `QUALIFY`, and the `SELECT` list. Add a sentence and one example (`WHERE smelt.functions.is_shipped(status)`).

Defer the call-path mapping table (the other DG-10 half) until Phase 2 lands so the table documents the enforced behavior.

### 1f. Update: `docs/specs/architecture.md`

Fix matching inconsistency in §"Resolution" prose (line 99):

> ❌ `random/x.sql` declaring `smelt.define helper(...)` is callable as `smelt.random.x.helper(...)`
> ✅ `random/x.sql` declaring `smelt.define helper(...)` is callable as `smelt.random.helper(...)`

### 1g. Update: `docs/specs/types.md`

Add an aggregate inference subsection to Semantics §5 "Canonical built-in returns" (**DG-8 close at the spec level**):

- `COUNT(*) → BigInt` (non-nullable — guaranteed by SQL semantics for COUNT(*)).
- `SUM(Integer | BigInt | SmallInt) → BigInt`; `SUM(Double | Float) → Double`; `SUM(Decimal(p,s)) → Decimal(38, s)`.
- `AVG(any numeric) → Double`.
- `MIN` / `MAX` — return the same type as the input (nullable — empty group returns NULL).
- `COALESCE(agg, literal)` — when the literal is non-null and the type matches, the result is non-nullable.

**Commit (Phase 1):** `docs: author cli, seeds, smelt_yml spec stubs; correct function path examples`

---

## Phase 2 — TB-5: Function path-prefix enforcement

**Spec:** `docs/specs/functions.md` §"Diagnostic codes" — `UnknownSmeltFn` — "A `smelt.<path>(...)` call references a path that does not resolve to a function."
**Spec diff (from Phase 1):** the fixed example in §"Function call syntax" makes it clear that path = directory + function name; the file stem is not a path component. The resolution rule is already normative; the implementation just doesn't enforce it.

### Reproduction

```bash
# functions/status.sql declares is_shipped(status TEXT) -> Expr<Boolean>
# spec-correct: smelt.functions.is_shipped(status)
# these should emit UnknownSmeltFn but don't:
#   smelt.functions.status.is_shipped(status)    ← file stem in path
#   smelt.functions.nonexistent.is_shipped(status) ← completely wrong path
```

### Tests (write first — red-green)

In `crates/smelt-db/tests/` (or `crates/smelt-db/src/function_body_check.rs::tests`):

1. `test_unknown_smelt_fn_wrong_path_prefix` — workspace with `functions/status.sql` declaring `is_shipped`; model calling `smelt.functions.nonexistent.is_shipped(s)` → must emit `UnknownSmeltFn`.
2. `test_unknown_smelt_fn_file_stem_in_path` — same workspace; model calling `smelt.functions.status.is_shipped(s)` (including file stem) → must emit `UnknownSmeltFn`.
3. `test_known_smelt_fn_correct_path` — same workspace; model calling `smelt.functions.is_shipped(s)` → must NOT emit `UnknownSmeltFn`.
4. `test_unknown_smelt_fn_name_not_declared` — model calling `smelt.functions.totally_made_up(s)` → must emit `UnknownSmeltFn`.

### Implementation notes

- Lives in `smelt-db` function-resolution query (`crates/smelt-db/src/functions.rs` and/or `function_body_check.rs::check_smelt_path_call`).
- Current behaviour: resolves by function name alone; path prefix is not validated.
- Fix: when resolving `smelt.<segments>(...)`, reconstruct the expected directory from the segments (everything except the last segment), look up the file at that workspace-relative path, check that the file declares the named function. If no file at that directory, or the file does not declare the name, emit `UnknownSmeltFn`.
- Run `cargo test -p smelt-cli --test example_diagnostics` after fix to confirm examples still pass.

### Post-fix: add call-path mapping table to `docs/specs/functions.md`

Once the enforcement is confirmed working, add the table to Surface §"Function call syntax":

| Filesystem location | Declared name | Call path |
|---|---|---|
| `functions/status.sql` | `is_shipped` | `smelt.functions.is_shipped(...)` |
| `functions/patterns/x.sql` | `session_rollup` | `smelt.functions.patterns.session_rollup(...)` |
| `utils/math.sql` | `safe_divide` | `smelt.utils.safe_divide(...)` |

This closes the DG-10 call-path half.

**Commit (Phase 2):** `fix(smelt-db): enforce path-prefix validation for smelt.<path>() calls, emit UnknownSmeltFn on mismatch`

---

## Phase 3 — TB-2: Seed DATE→TEXT narrowing

**Spec:** `docs/specs/seeds.md` §"Type inference" (from Phase 1) — Known Divergence: DATE/TIMESTAMP columns infer as TEXT at compile time.
**Goal:** align compile-time inference with DuckDB's `read_csv_auto()` output for temporal types.

### Reproduction

```bash
rm -f orders-pipeline.duckdb
smelt seed
duckdb orders-pipeline.duckdb -c 'DESCRIBE raw_orders'   # order_date -> DATE
# stg_orders.sql: SELECT o.order_date FROM smelt.models.raw_orders o
smelt build
smelt table stg_orders                                    # order_date -> TEXT  ← bug
duckdb orders-pipeline.duckdb -c 'DESCRIBE stg_orders'   # order_date -> VARCHAR
```

### Tests (write first — red-green)

In `crates/smelt-db/tests/` (or the type property tests):

1. `test_seed_date_column_infers_as_date` — seed CSV with a `DATE`-shaped column (`2025-01-01`); `smelt table` (or `model_schema()` on a passthrough staging model) must report `Date`, not `Text`/`Varchar`.
2. `test_seed_timestamp_column_infers_as_timestamp` — same for a `TIMESTAMP`-shaped column.
3. `test_seed_text_column_infers_as_text` — free-form string column still infers as `Text` (no regression).

### Implementation notes

- Lives in `smelt-db` seed schema extraction (look in `crates/smelt-db/src/schema.rs` or wherever `read_csv_auto` schemas are inferred at compile time).
- The compile-time inferencer samples the first N rows and classifies column values. It currently recognises only `BOOLEAN`, `INTEGER`, `DOUBLE`, `TEXT`. Add `DATE` (`YYYY-MM-DD` pattern) and `TIMESTAMP` (`YYYY-MM-DD HH:MM:SS` pattern) recognition to match DuckDB's rules.
- Check whether `DECIMAL` columns (numeric with decimal points) diverge: DuckDB maps them to `DOUBLE`, so smelt should too — verify or add a test.
- After fix: update `docs/specs/seeds.md` — move TB-2 from Known Divergences to Semantics ("compile-time and runtime inference agree for DATE, TIMESTAMP, INTEGER, DOUBLE, BOOLEAN; DECIMAL columns infer as DOUBLE at both levels").

**Commit (Phase 3):** `fix(smelt-db): infer DATE and TIMESTAMP types for seed CSV columns at compile time`

---

## Phase 4 — TB-1: `--verbose` wiring

**Spec:** `docs/specs/cli.md` §"`--verbose`" (from Phase 1) — must log compiled SQL for each model immediately before execution.

### Reproduction

```bash
rm -f orders-pipeline.duckdb
smelt build --verbose
# Output identical to non-verbose run — no compiled SQL logged
```

### Tests (write first — red-green)

In `crates/smelt-cli/tests/` or as an integration test:

1. `test_verbose_build_logs_sql` — run `smelt build --verbose` on a minimal project and assert stdout/stderr contains the compiled SQL for each model (e.g., the `SELECT` statement).
2. `test_non_verbose_build_no_sql` — run `smelt build` (no `--verbose`) and assert no SQL is logged.
3. `test_verbose_uptodate_no_sql` — run `smelt build --verbose` twice; second run (up-to-date models) produces no SQL output (models don't execute).

### Implementation notes

- Lives in `smelt-cli` (`crates/smelt-cli/src/`). The `--verbose` flag is parsed but its value likely isn't threaded into the model execution loop.
- Fix: pass the verbose flag into the execution step; for each model that actually executes, print the compiled SQL string before calling the backend.
- The `reference/cli.md` docs already say `--verbose` "logs the compiled SQL for each model immediately before execution" — the docs are ahead of the implementation.

**Commit (Phase 4):** `fix(smelt-cli): wire --verbose to log compiled SQL per model before execution`

---

## Phase 5 — TB-4: `--version` flag

**Spec:** `docs/specs/cli.md` §"`smelt --version`" (from Phase 1).

### Implementation notes

- Add a top-level `--version` flag via clap. One-liner: `#[arg(long, action = clap::ArgAction::Version)]` on the root CLI struct, or use clap's built-in `.version(env!("CARGO_PKG_VERSION"))` on the command builder.
- Verify `smelt --version` prints the package version and exits 0.

**Commit (Phase 5):** `fix(smelt-cli): add --version flag`

---

## Phase 6 — User docs (DG items)

**Goal.** Update docs-site pages to close all DG items. All DG items depend on the spec stubs from Phase 1; DG-10 call-path half also waits for Phase 2.

### DG mapping to docs-site pages

| Finding | Page | Change |
|---|---|---|
| DG-1 + DG-5 | `guide/seeds` | Consolidate into "Column type inference" section (spec from Phase 1 is the source of truth); document the two-pass disagreement, the workaround (CAST in staging), and `smelt table` as the inspection tool. |
| DG-2 + DG-6 | `reference/cli` | Add a `smelt build` flag truth-table: `--verbose` (now correct after Phase 4), no `--dry-run`, `--show-plan` is per-model, `--select` is repeatable. Subsumes DG-2. Add `--version` after Phase 5. |
| DG-3 | `reference/cli` | Add sentence under `--verbose`: "No extra output when all models are up-to-date." Folds into DG-6 update. |
| DG-4 | `getting-started/quickstart` | Add callout near `smelt.yml` example listing supported keys and defaults (from seeds.md / smelt_yml.md spec stubs). |
| DG-7 | `guide/seeds` | Add explicit `seeds/raw_orders.csv` → `smelt.models.raw_orders` example near top of page. Folds into the DG-1/DG-5 "Column type inference" section work. |
| DG-8 | `reference/language` | Add "Aggregate return types" subsection based on types.md §5 update from Phase 1. |
| DG-9 | `reference/smelt-yml` + `guide/materializations` | Add one-line statement: default `materialization` is `table` when omitted. Cross-link. |
| DG-10 (call-path) | `guide/functions` | Add file-location → call-path mapping table (post Phase 2). |
| DG-10 (placement) | `guide/functions` | Add sentence + `WHERE smelt.functions.is_shipped(status)` example for boolean-position use (unblocked, do in this phase). |

**Commit (Phase 6):** `docs: close all DG items from smelt-loop findings`

---

## Deferred: TB-3 — No project-wide compile-only flag

**Decision needed before implementation.** Two options:
1. Extend `--show-plan` to accept "no positional argument means whole project".
2. Add a separate `--dry-run` flag to `smelt build` (similar to `smelt run --dry-run`).

Record the decision in `docs/specs/cli.md` §"TB-3 open question" (stub already added in Phase 1). Implement in a follow-up plan once the direction is chosen.

---

## Original findings (reference)

The detailed reproduction steps and proposed directions from the loop retros are preserved below for implementers.

### TB-1: `smelt build --verbose` produces no extra output

`smelt build --help` advertises `--verbose` as "Show compiled SQL for each model", but on a clean rebuild the output is identical to the non-verbose run.

```bash
rm -f orders-pipeline.duckdb
smelt build --verbose
# smelt: loaded 2 seed(s) (15 rows) in 0.03s
# smelt: built 3 model(s) in 0.03s
```

### TB-2: Inferred type for a passthrough DATE column is TEXT

```bash
rm -f orders-pipeline.duckdb
smelt seed
duckdb orders-pipeline.duckdb -c 'DESCRIBE raw_orders'   # order_date -> DATE
# stg_orders.sql:  SELECT o.order_date AS order_date FROM smelt.models.raw_orders o
smelt build
smelt table stg_orders                                    # order_date -> TEXT  ← divergence
duckdb orders-pipeline.duckdb -c 'DESCRIBE stg_orders'   # order_date -> VARCHAR
```

### TB-3: No project-wide "compile but don't execute" flag (deferred)

`smelt build --dry-run` is rejected. `smelt build --show-plan` requires a positional model file.

### TB-4: `smelt --version` is not a recognised flag

```bash
smelt --version
# error: unexpected argument '--version' found
```

### TB-5: `smelt.functions.*` path prefix not enforced

```bash
# functions/status.sql declares is_shipped
# spec-correct:  smelt.functions.is_shipped(status)
# all three succeed (none emit UnknownSmeltFn):
#   smelt.functions.status.is_shipped(status)     ← file stem in path (wrong per spec)
#   smelt.functions.is_shipped(status)             ← spec-correct
#   smelt.functions.nonexistent.is_shipped(status) ← completely wrong path
# only fails at DuckDB level when function name itself is unknown:
#   smelt.functions.status.totally_made_up(status) → Parser Error
```

**Also:** `functions.md` line 66 and `architecture.md` line 99 both show `random/x.sql` → `smelt.random.x.helper(...)` (file stem included). This contradicts the table in `architecture.md` which correctly shows file stem is NOT a path component. Both examples need correction in Phase 1.

### DG items

See Phase 6 mapping table above. Original sources:
- **DG-1/DG-5**: iterations 1 and 2 — seed type inference not documented.
- **DG-2/DG-3/DG-6**: iterations 1 and 3 — CLI flag surface incomplete in `reference/cli`.
- **DG-4**: iteration 2 — quickstart never lists valid `smelt.yml` keys.
- **DG-7**: iteration 3 — seed filename→ref mapping not shown with an example.
- **DG-8**: iteration 3 — aggregate return types not documented in `reference/language`.
- **DG-9**: iteration 4 — default materialization (`table`) not stated anywhere.
- **DG-10**: medium-tier iteration 1 — functions guide missing call-path table and WHERE/HAVING examples.

## Loop convergence

| # | tier | passed/total | retro signal | skill diff |
|---|------|--------------|--------------|------------|
| 1 | small | 10/10 | yes (3 TB, 3 DG, 4 SG) | applied (167 lines) |
| 2 | small | 10/10 | weak (0 TB, 2 DG, 0 SG) | none |
| 3 | small | 10/10 | weak (1 TB, 3 DG, 0 SG) | none |
| 4 | small | 10/10 | weak (0 TB, 1 DG new, 2 SG) | applied |
| 5 | small | 10/10 | none (0 TB, 0 DG new, 0 SG actionable) | none |
| 6 | medium | 14/14 | yes (1 TB, 1 DG, 2 SG deferred) | rejected (path-resolution bug) |
