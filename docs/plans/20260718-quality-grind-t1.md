# Plan: Quality Grind Tier 1 — small root-caused fixes

**Date**: 2026-07-18
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) §"Constraints & Invariants" items 13–14; [`docs/specs/diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: none — conformance to existing gate contracts. Root-cause notes: `docs/TODO.md` §§"2026-07-12" entries and `docs/ROADMAP.md` §"Deferred-Work Backlog".
**Master**: [`docs/plans/20260718-quality-grind.md`](20260718-quality-grind.md)
**Tracking PR / branch**: `worktree-roadmap_todo`
**Docs**: code+docs (most phases have no user-visible surface and say so; Phase 12 is the docs batch)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session, one phase per iteration.

**Before touching any code:**

1. Read this entire plan file. The correctness oracles are the standing CI gates named
   per phase — do not lower any ratchet without a reviewer sign-off note (CLAUDE.md
   §Fail-loud / §SQL dialect conformance).
2. Confirm you are on branch `worktree-roadmap_todo`. If not, stop and surface.
3. Find the next `pending` phase in Progress tracking; skip `done`/`blocked` rows.

**For each phase:** implementer subagent (red-green TDD on the listed tests) → reviewer
subagent (material findings only) → `bash .claude/scripts/verify-phase.sh` → flip the row
to `done` → atomic commit with the phase's `Commit.` line → push.

**Ledger-phase convention (Phases 1–6).** Before writing any code, re-verify the claim:
extract the category's entries from
`crates/smelt-parser-compat/tests/corpus/external_ledger.toml` and re-parse each
statement to confirm the recorded first-error still reproduces. Then fix, then close the
entries (delete the ledger rows — the ledger is shrink-only) and re-run both gates:
`cargo test -p smelt-parser-compat --test external_corpus --quiet 2>&1 | tail -20` and
`cargo test -p smelt-parser-compat --test duckdb_differential --quiet 2>&1 | tail -20`
(needs `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH`). If DuckDB rejects a statement the ledger
claims it accepts, record that in "Deferred during implementation" and leave the entry.

**When to mark a phase `blocked`:** a fix requires a semantic/design decision (e.g. it
would change inference behaviour, not just grammar), a gate must be lowered to pass, or
the reviewer repeats the same material finding across two implementer passes. Record a
dated entry under "## Blocked phases", commit, move on.

**Conventions every phase:** real-fixture tests; red-green; no scope-widening; honor all
CLAUDE.md invariants; never `--no-verify`; timeless-oracle rule for any spec/docs-site
edit.

---

## Context

The 2026-07-12 external-ledger triage (`docs/TODO.md`) root-caused 236 unclassified
parser failures into 56 categories and named the small, DuckDB-relevant ones; three
same-shape fixes already landed (quoted aliases, FIRST/LAST, leading-dot decimals), so
each phase here imitates a proven fix pattern. The non-parser phases are equally
pre-triaged one-file fixes from the ROADMAP deferred backlog.

## Scope

### In scope
- Ledger categories: `not_prefixed_binary_operator`, `double_equals_operator`,
  `quoted_table_name_in_from`, `range_keyword_as_identifier_or_function`, plus the
  `NULL::TYPE` residue and the parenthesized-set-op trailing ORDER BY hole.
- TABLESAMPLE/PIVOT/UNPIVOT vs alias ordering (parser + printer).
- VALUES-body CTE arity check; sub-day interval mis-parse; UTF-8 diagnostic positions +
  smelt-ui `LineIndex` migration; `to_seconds`/`md5` registry gaps; documentation-gap batch.

### Explicitly deferred
- `aggregate_call_order_by_clause` (WITHIN GROUP grammar) — Tier 2 territory (medium).
- `implicit_cross_join_comma_syntax` — needs a semantics decision (master D-QG-2).
- All remaining medium/pg-only ledger categories.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | fix(parser): NOT-prefixed binary operators | 2026-07-18 |
| 2     | done    | fix(lexer): accept == as equality operator alias | 2026-07-18 |
| 3     | done    | fix(parser): double-quoted table/schema names in FROM | 2026-07-18 |
| 4     | done    | fix(parser): RANGE as function name/identifier outside frame specs | 2026-07-18 |
| 5     | pending |        |      |
| 6     | pending |        |      |
| 7     | pending |        |      |
| 8     | pending |        |      |
| 9     | pending |        |      |
| 10    | pending |        |      |
| 11    | pending |        |      |
| 12    | pending |        |      |

---

### Phase 1: `NOT`-prefixed binary operators

**Goal.** `expr NOT IN (...)`, `NOT LIKE`, `NOT ILIKE`, `NOT GLOB`, `NOT SIMILAR TO` and
`NOT BETWEEN` parse as the negated binary operator (today `SELECT 2 NOT IN (2, 3)` alone
fails). Close the `not_prefixed_binary_operator` ledger entries (6).

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-parser/tests/` (parser unit suite, alongside existing operator tests) —
  `not_in_parses`, `not_like_parses`, `not_between_parses`, `not_ilike_glob_parse`: each
  asserts a clean parse, the printed round-trip preserves `NOT`, and (where the type
  path exists) inference yields `Boolean`.
- Round-trip fidelity: printed SQL re-executes on DuckDB (differential gate covers this;
  add a seed line for `SELECT 2 NOT IN (2, 3)`).

**Implementation shape.** In `crates/smelt-parser/src/parser/expr.rs`, at the
binary-operator dispatch where `IN`/`LIKE`/`ILIKE`/`GLOB`/`SIMILAR`/`BETWEEN` are
recognised, accept an optional leading `NOT_KW` (lookahead: `NOT` followed by one of
those keywords, so prefix-`NOT` on a general expression is untouched). Printer emits the
`NOT` back. Type inference: same Boolean typing as the non-negated operator.

**Critical files.**
- `crates/smelt-parser/src/parser/expr.rs` — lookahead + operator parse
- `crates/smelt-parser/src/printer.rs` — emit `NOT`
- `crates/smelt-parser/src/ast.rs` — operator accessor covers negated form
- `crates/smelt-db/src/type_inference/` — Boolean result for negated forms (only if not already generic over the operator token)
- `crates/smelt-parser-compat/tests/corpus/external_ledger.toml` — close entries
- differential seed corpus — add seed line

**Docs touched.** None — no user-visible surface change (accepted-SQL widening is tracked by the gates).

**Review checklist:**
- [ ] TDD tests exist and assert parse + print round-trip + Boolean typing
- [ ] Prefix-`NOT` on non-operator expressions unaffected (regression test)
- [ ] Ledger entries closed; `external_corpus` + `duckdb_differential` green
- [ ] No scope creep (no comma-join, no WITHIN GROUP)

**Commit.** `fix(parser): NOT-prefixed binary operators (NOT IN/LIKE/ILIKE/GLOB/SIMILAR TO/BETWEEN)`

### Phase 2: `==` as equality alias

**Goal.** Lex `==` as an alias for `=` (DuckDB accepts it). Close `double_equals_operator` (3 entries).

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit test `double_equals_parses_as_eq` — `SELECT 1 == 1` parses; printer emits `=` (canonical); inference Boolean.
- Lexer test pinning `==` tokenization.

**Implementation shape.** Lexer two-char match arm before the single `=` arm; either emit
the existing EQ token (printer then canonicalises to `=` for free) or a distinct token
mapped to the same operator.

**Critical files.**
- `crates/smelt-parser/src/lexer.rs`, ledger, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] `=` behaviour untouched; `==` round-trips to executable SQL
- [ ] Ledger entries closed; both compat gates green

**Commit.** `fix(lexer): accept == as equality operator alias`

### Phase 3: double-quoted table names in FROM

**Goal.** `FROM "flights"` (and `"schema"."table"`) parse; printer re-quotes. Same root
cause and fix shape as the landed quoted-alias fix (2026-07-12 triage fix #1), applied to
`parse_table_ref`'s primary-identifier path. Close `quoted_table_name_in_from`.

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit tests: `quoted_table_name_in_from`, `quoted_schema_qualified_table` —
  clean parse, printer preserves quotes (DuckDB requires them for names that need quoting).
- Negative: quoted string in FROM that is a file-glob path (`FROM 'x.parquet'`,
  single-quoted) keeps its current classification — do not conflate with
  `file_glob_or_path_literal_from`.

**Implementation shape.** In `crates/smelt-parser/src/parser/select.rs`
`parse_table_ref`, accept a `STRING` token produced by double-quote lexing where a
primary identifier is expected, via the existing `at_quoted_ident_alias`-style guard
(distinguish `"` from `'` — the triage notes `consume_string` doesn't, so reuse whatever
discrimination the alias fix introduced). `strip_ident_quotes` in `ast.rs` already
handles `"…"`. Printer re-quotes on emit.

**Critical files.**
- `crates/smelt-parser/src/parser/select.rs`, `parser/mod.rs`, `ast.rs`, `printer.rs`, ledger, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] Single-quoted strings in FROM unaffected (no reclassification of glob/path entries)
- [ ] Round-trip preserves quoting; both compat gates green

**Commit.** `fix(parser): double-quoted table/schema names in FROM`

### Phase 4: `RANGE` as identifier / function name

**Goal.** `RANGE` usable as a function name and identifier (DuckDB's `range()`), same
shape as the landed FIRST/LAST allowlist fix. Close
`range_keyword_as_identifier_or_function` (5 entries).

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit tests: `range_as_function_name` (`SELECT * FROM range(10)`, `SELECT range(5)`),
  `range_as_identifier` if the ledger entries include that form.
- Regression: window-frame `RANGE BETWEEN …` still parses (the keyword's existing role).

**Implementation shape.** Add `RANGE_KW` to `at_keyword_as_function_name`'s allowlist;
verify the window-frame parse path takes precedence in OVER-clause context (it dispatches
on clause position, so no conflict expected — pin with the regression test).

**Critical files.**
- `crates/smelt-parser/src/parser/mod.rs` (or wherever `at_keyword_as_function_name` lives), expr/select parse sites, ledger, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] Window-frame RANGE regression test green
- [ ] Ledger entries closed; both compat gates green

**Commit.** `fix(parser): RANGE as function name/identifier outside frame specs`

### Phase 5: `NULL::TYPE` and casts in named-arg value positions

**Goal.** `SELECT NULL::VARCHAR` and `::` casts inside `param => value` named-argument
values parse (pre-existing precedence gap, TODO 2026-07-12 walrus residue). Close the
recategorized ledger entry.

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit tests: `null_double_colon_cast` (`SELECT NULL::VARCHAR`), inference `Varchar` nullable;
  `named_arg_value_with_cast` (`f(x => 1::BIGINT)`) parses.
- Print round-trip for both.

**Implementation shape.** In `crates/smelt-parser/src/parser/expr.rs`, ensure the
postfix-`::` loop is reachable after parsing a `NULL` literal primary and after
named-argument value expressions (the error `Expected expression, found DOUBLE_COLON`
says the primary parse returns before the postfix loop in those positions). Likely a
matter of routing those positions through the standard expression entry point rather
than a restricted sub-parser.

**Critical files.**
- `crates/smelt-parser/src/parser/expr.rs` (+ the named-arg parse site), ledger, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] `CAST(NULL AS …)` behaviour unchanged; `::` on other literals still works
- [ ] Both compat gates green

**Commit.** `fix(parser): postfix :: cast on NULL and in named-arg value positions`

### Phase 6: trailing ORDER BY/LIMIT after parenthesized set-op operand

**Goal.** `SELECT a FROM t UNION (SELECT a FROM t) ORDER BY a` parses, attaching the
trailing ORDER BY/LIMIT to the whole set operation (DuckDB semantics). PR #158 residue —
the most user-visible hole in the set-op surface.

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit tests: `union_paren_operand_trailing_order_by`, `union_paren_operand_trailing_limit`,
  and the `((A) UNION B)` scalar-subquery residual if its ledger entry reproduces.
- Print round-trip executes on DuckDB (seed line).

**Implementation shape.** In `parse_select_stmt`'s set-op tail (per the TODO note),
after consuming a parenthesized operand, continue parsing trailing ORDER BY/LIMIT/OFFSET
clauses and attach them at the set-op statement level (mirroring how the unparenthesized
tail already attaches them).

**Critical files.**
- `crates/smelt-parser/src/parser/select.rs`, ledger, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] ORDER BY binds to the whole union (assert via CST shape or printed placement), not the last operand
- [ ] Both compat gates green

**Commit.** `fix(parser): trailing ORDER BY/LIMIT after parenthesized set-op operand`

### Phase 7: TABLESAMPLE/PIVOT/UNPIVOT vs alias ordering

**Goal.** Parse and print `base AS alias TABLESAMPLE(...)` (DuckDB v1.5.4 rejects the
current alias-last order smelt emits — the printer produces invalid SQL whenever
TABLESAMPLE co-occurs with an alias). Verify PIVOT/UNPIVOT ordering while there
(unprobed; oracle-verify against DuckDB first).

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit test `tablesample_after_alias` — `FROM t AS x TABLESAMPLE (10%)` parses; printed output is alias-first.
- Fidelity: differential-gate seed line so DuckDB executes the printed form.
- Oracle probe (in-test or documented in the phase commit): DuckDB acceptance of both orders for PIVOT/UNPIVOT; pin whichever is accepted.

**Implementation shape.** Swap `parser/select.rs` to parse the alias before TABLESAMPLE
(keep accepting the legacy alias-last order for lenience if trivial, but *print* only the
DuckDB-valid order). Printer emits alias then TABLESAMPLE. Mind the `Display for TableRef`
raw-text `else` fallback double-print hazard noted in TODO — add the defensive early
return while touching this.

**Critical files.**
- `crates/smelt-parser/src/parser/select.rs`, `printer.rs`, seed corpus.

**Docs touched.** None.

**Review checklist:**
- [ ] Printed order oracle-verified against real DuckDB
- [ ] Defensive early-return added to the raw-text fallback (no double print)
- [ ] Both compat gates green

**Commit.** `fix(parser): alias-first TABLESAMPLE/PIVOT/UNPIVOT ordering to match DuckDB`

### Phase 8: VALUES-body CTE arity check

**Goal.** `WITH cte(a) AS (VALUES (1, 2)) SELECT * FROM cte` fires
`AliasColumnArityMismatch`, symmetric with the SELECT-body case
(`docs/TODO.md` §"VALUES / sources resolver").

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-db/tests/` (alongside the existing arity tests): `values_body_cte_arity_mismatch`
  (1 alias col vs 2 VALUES cols → diagnostic at the right span) and
  `values_body_cte_arity_match_clean` (no diagnostic).

**Implementation shape.** In `check_cte_alias_arity`
(`crates/smelt-db/src/type_inference/values.rs`), replace the early return for non-SELECT
bodies with a VALUES branch: get the column count via `Subquery::values_clause()` +
`infer_values_columns` and compare against the alias list, mirroring the SELECT path's
diagnostic construction.

**Critical files.**
- `crates/smelt-db/src/type_inference/values.rs` + its test file.

**Docs touched.** `docs/specs/diagnostics.md` — only if `AliasColumnArityMismatch`'s
catalogue entry describes SELECT-only coverage (timeless wording).

**Review checklist:**
- [ ] Diagnostic span lands on the alias list (parity with SELECT-body case)
- [ ] Salsa purity rule respected (pure helper, thin query)

**Commit.** `fix(types): VALUES-body CTE alias arity check (AliasColumnArityMismatch parity)`

### Phase 9: sub-day intervals in `extract_interval_days_from_combined`

**Goal.** `INTERVAL '5 minutes'` no longer parses as 5 days in
`crates/smelt-logical/src/analysis/temporal.rs` (advisory `analyze_batch_safety` label
only; runtime chunk sizing unaffected — keep it that way).

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (or the module's unit tests): `interval_minutes_not_days`
  pinning `INTERVAL '5 minutes'` → sub-day result (round up to 1 day per the TODO's
  suggested contract), `interval_seconds_not_days`, and an unchanged `INTERVAL '3 days'` pin.

**Implementation shape.** Add MINUTE/SECOND (and HOUR if also missing) branches; round
up to 1 day, documenting the advisory-only granularity in a doc comment (this function
is a leaf classifier / advisory heuristic under the property-composition-walk rule —
keep its doc-comment classification accurate).

**Critical files.**
- `crates/smelt-logical/src/analysis/temporal.rs` + tests.

**Docs touched.** None (advisory JSON label).

**Review checklist:**
- [ ] `batch_safety_from_bounds` runtime path untouched
- [ ] Walk-rule doc-comment classification intact
- [ ] `cargo test -p smelt-logical --test walk_coverage` green

**Commit.** `fix(logical): sub-day INTERVAL units in extract_interval_days_from_combined`

### Phase 10: UTF-8 diagnostic positions — `body_position_to_byte` + smelt-ui LineIndex

**Goal.** Close the two diagnostics-encoding backlog items together: (a)
`body_position_to_byte` counts codepoints not UTF-8 bytes, shifting positions in
non-ASCII emission bodies; (b) `smelt-ui`'s `DiagnosticInfo` still uses legacy
`offset_to_position` instead of `line_index::LineIndex`.

**Pre-conditions.** None.

**TDD tests to write first.**
- Fixture with a non-ASCII emission body (e.g. a `é`/CJK char before the error site):
  test asserting the diagnostic's byte range lands on the true offset (the TODO calls
  this "one-line fix pending a fixture" — the fixture is the deliverable).
- `smelt-ui` unit test: `DiagnosticInfo` line/column for a multi-byte line matches
  `LineIndex` output.

**Implementation shape.** (a) index by `char_indices()`/byte offsets instead of counting
chars. (b) swap `offset_to_position` for `LineIndex` per the diagnostic-range-encoding
invariant (conversion exactly once at the boundary).

**Critical files.**
- The `body_position_to_byte` site (locate via `rg -n body_position_to_byte crates/`) + its tests
- `crates/smelt-ui/src/` `DiagnosticInfo` conversion + tests.

**Docs touched.** None.

**Review checklist:**
- [ ] Diagnostic-range-encoding invariant upheld (one boundary conversion, LineIndex-backed)
- [ ] ASCII behaviour byte-identical (regression pin)

**Commit.** `fix(diagnostics): UTF-8-correct positions in emission bodies and smelt-ui`

### Phase 11: function-registry gaps — `to_seconds`, `md5`

**Goal.** `to_seconds(n)` infers `Interval` (today unrecognised, forcing `epoch_us()`
workarounds); `md5(text)` registered (today emits a Warning that fails the diagnostics
gate, forcing CONCAT surrogate keys). Leave `arg_min` unregistered (deliberate, pending a
real call site).

**Pre-conditions.** None.

**TDD tests to write first.**
- Registry-gate-shaped tests: `to_seconds` and `md5` resolve in `BuiltinRegistry` with
  scalar classification and correct return types (`Interval`, `Text`).
- Property/oracle check: a DuckDB-verified inference test for each (extend the existing
  smoke-test pattern in `type_property_tests` helpers, not the generators).
- A model in an example workspace (or existing fixture) using `md5(...)` produces zero diagnostics.

**Implementation shape.** Add `Signature` rows in
`crates/smelt-types/src/signatures.rs` (registry-first — do not extend the legacy match;
the migration ratchet must not grow). Verify Spark-side behaviour matches or record a
divergence entry if Spark types differ.

**Critical files.**
- `crates/smelt-types/src/signatures.rs` + registry-consistency test expectations.

**Docs touched.** `docs-site` function-reference page if one enumerates supported
functions (check; timeless wording).

**Review checklist:**
- [ ] `registry_consistency` gates green; migration ratchet count unchanged
- [ ] DuckDB oracle confirms both return types (integer-width/precision exact)
- [ ] No new `divergences.rs` entry without oracle evidence

**Commit.** `feat(types): register to_seconds (Interval) and md5 (Text) in BuiltinRegistry`

### Phase 12: documentation-gap batch + TODO hygiene

**Goal.** Close the four ledgered doc gaps and one stale TODO line:
BUG-022 (`test` materialization missing from `materializations.md`), BUG-031
(calendar-invalid seed date note in `seeds.md`), BUG-062 (window-function safety check as
third non-expanding classify site in `incremental_models.md` Known Divergences), BUG-071
(Month/Quarter/Year grain limitation in `cumulative_aggregate.md` Known Divergences), and
delete the stale `docs/TODO.md` §"smelt test" selector item (landed 2026-06-21 as W6 D-41).

**Pre-conditions.** None.

**TDD tests to write first.** None (docs-only phase). Verification is
`cargo test -p smelt-cli --test example_diagnostics` staying green and a self-review pass
against each cited behaviour (confirm each claim against the current code before writing it).

**Implementation shape.** Four surgical doc edits, each verified against current
behaviour first (esp. BUG-062/071 — confirm the limitation still exists before
documenting it; if one no longer reproduces, record that instead and skip the edit).
Timeless-oracle wording throughout.

**Critical files.**
- `docs-site/docs/**/materializations.md`, `seeds.md` page
- `docs/specs/incremental_models.md`, `docs/specs/cumulative_aggregate.md` (Known Divergences)
- `docs/TODO.md`

**Review checklist:**
- [ ] Every documented limitation re-verified against current code (no stale claims written)
- [ ] Timeless wording; no phase vocabulary
- [ ] Stale TODO removed with a pointer to the W6 D-41 landing

**Commit.** `docs: close BUG-022/031/062/071 doc gaps; drop stale smelt-test selector TODO`

---

## Blocked phases

(Append dated entries here; never stop-the-line.)

## Deferred during implementation

(Append-only.)

- **2026-07-18, Phase 4.** Closing `range_keyword_as_identifier_or_function`
  surfaced that 2 of its 5 ledger entries were multi-cause: the RANGE
  failure was masking a second, unrelated gap that reproduces once RANGE
  itself parses. Re-categorized rather than closed:
  - `4b5f29a8027c361a` → `file_glob_or_path_literal_from` (single-quoted
    file-path literal as a FROM target — pre-existing, unrelated to RANGE).
  - `11364d332cae94d5` → `interval_string_literal_unit_in_arg_list` (new
    category: `interval '1' day`'s trailing unit keyword is only consumed
    via an accidental implicit-column-alias at SELECT-item top level;
    inside a function argument list it's left dangling — "Expected
    RPAREN". Root cause is in `is_typed_literal`'s INTERVAL branch, not
    RANGE; out of scope for this phase).
  - Also removed one unrelated stale entry `e7f8101e8ebcb4aa`
    (`sqllogictest_template_placeholder`) that `ledger_has_no_stale_entries`
    flagged as now-passing — a side effect of the combined Phase 1/2/4
    fixes (NOT-prefixed operators, `==`, and RANGE) unblocking a statement
    that happened to use all three.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-parser-compat --test external_corpus` and `--test duckdb_differential` — ledger strictly smaller, no new gaps
- `cargo test -p smelt-db --test type_property_tests` and `--test integration registry_consistency` green
- `cargo test -p smelt-logical --test walk_coverage` green
