# Backbuild property-test review — mutation-testing audit

Date: 2026-08-03
Branch: `spec-redraft-incremental-models` (worktree `incremental3`)
Subject: `crates/smelt-logical/tests/backbuild_property.rs` (+ `backbuild_conformance.rs`, `backbuild_options.rs`)

## What was done

Brainstormed the ways a backbuild script could (a) render invalid SQL or (b) produce a
table not multiset-equal to a full rebuild, then verified test coverage empirically:
11 mutations injected into `emit.rs` / `classify.rs` / `mod.rs`, each run against the
three backbuild suites (`--no-fail-fast`), then reverted. Tree left clean, baseline
green. Env for runs: `DUCKDB_LIB_DIR=~/.local/lib/duckdb LD_LIBRARY_PATH=~/.local/lib/duckdb`.

## Mutation matrix

| # | Injected bug | Property gate | Other suites |
|---|---|---|---|
| M1 | `emit_difference_insert`: splice `after_sql` unparenthesized (invalid SQL) | ✅ caught | ✅ 10 conformance |
| M2 | `emit_predicate_delete`: `IS NOT TRUE` → `NOT (...)` (E1 3VL trap) | ❌ silent | ✅ `e1_null_semantics` (behavioral) |
| M3 | difference INSERT `IS NOT TRUE` → `NOT (...)` (E2/E4 3VL) | ❌ at N=24, ✅ at N=300 | ⚠️ only a `contains("IS NOT TRUE")` string assert (`e2_loosen_inserts_difference`) |
| M4 | drop identity anti-join guard at all 3 sites (E4/E2/F1), keep `rerun_safe: true` | ❌ silent | ✅ `e4_idempotent_with_identity` (behavioral rerun) |
| M4b | drop guard at **E2 + F1 only** | ❌ silent | 🔴 **entire repo suite green** |
| M5 | `assemble`: ALTER DROPs first instead of last | ❌ silent even at N=300 | ✅ `c1_dropped_column_drops_last` |
| M6 | F2 discriminator DELETE `=` → `!=` | ✅ caught | ✅ f2 tests |
| M7 | B5 re-aggregation subquery drops model WHERE | ❌ silent even at N=300 | ✅ `b5_where_clause_is_carried` |
| M8 | `emit_alter_add_column` ignores declared type (always VARCHAR) | ✅ caught | ✅ 14 conformance |
| M9 | `emit_branch_insert` positional (`SELECT *`, no column list) | ❌ silent | ✅ branch-add + column-add composite test |
| M10 | revert C2 fix: qualifier-blind `resolve_representative` | ❌ silent | ✅ one c2 unit test (`..._via_where_conjunct_..._e1`) |

## Findings (ranked)

1. **🔴 Must-fix hole: E2 (`FilterLoosenInsert`) and F1 (`UnionBranchInsert`)
   rerun-safety is completely unverified** (M4b survived everything). The property
   harness applies each script exactly once; only E4 has a conformance rerun test.
   Fix: in `generated_options_match_full_rebuild_oracle`, when every chosen option in
   a combo has `rerun_safe: true`, apply the composed script **twice** before the
   oracle check; plus E2/F1 siblings of `e4_idempotent_with_identity`. Makes
   `rerun_safe` a tested claim for all future techniques too.

2. **⚠️ Generator structurally cannot express three classifier-handled failure
   classes** (each held by exactly one conformance test):
   - *E1 3VL (M2)*: the only tighten edit rendered is `o.status IS NOT NULL`, which
     never evaluates NULL. Add a tighten variant `o.status = 'open'` (NULL statuses
     exist via boundary row 92).
   - *B5 WHERE-carry (M7)*: `Shape::Grouped` recipes hardcode empty
     `where_conjuncts`. Give Grouped a WHERE.
   - *H-ordering of drops (M5)*: no generated composed script ever reads the dropped
     column `ts` (edits that would are correctly refused when combined), so drop
     ordering is unobservable at any N.

3. **⚠️ M3 (E2/E4 complement-form NULL semantics) rests on a string assert at the
   default case count.** Reachable generatively (fails at N=300: `LoosenFilter`
   removing `StatusOpen` + NULL-status row), never drawn at N=24. Add a guaranteed
   slot: `LoosenFilter` over `where_conjuncts: [StatusOpen]` (current guaranteed case
   uses `AmountPositive`, which is NOT NULL and 3VL-blind).

4. **Acknowledged contract limits (no action urged now):**
   - Composite `row_identity`/`unique_key` SQL never executed against DuckDB (only
     string-shape asserts in `emit.rs` tests); all executed tests use single-column keys.
   - Identifier quoting / literal escaping untested — `emit.rs` is self-described
     "test-grade DuckDB dialect"; becomes real at wiring time.
   - B6 admits any explicit `ORDER BY` incl. non-total ones (`classify.rs:2985`);
     with ties the equivalence property is itself ill-defined (two rebuilds also
     differ) — spec-caveat question, not a classifier bug. Generator always orders by
     unique `order_id`.
   - Lying `unique_key`/`not_null_columns` declarations are out of contract; the
     UpdateFrom-vs-scalar-subquery duplicate-match safety asymmetry is doc-comment
     only, not demonstrated by a test.
   - `PRODUCT_CAP=8` odometer varies trailing atoms first — early atoms' later
     options unverified in large products (logged; guaranteed single-edit cases cover
     each technique individually).
   - `reads_upstream`/`write_scope` metadata asserted only pointwise.

## Verdict on the test design

Sound, and several choices proved load-bearing under mutation: name-projected
two-way `EXCEPT ALL` (not positional `SELECT *`), schema name/type **set** comparison
(caught M8 broadly), executing every statement on real DuckDB (caught M1), and the
guaranteed-slot scheme for deterministic technique coverage. Defense-in-depth is
good — every equivalence mutation except M4b was caught by something behavioral.

## Suggested next steps

1. Rerun-safety leg in the property harness + E2/F1 rerun conformance tests (finding 1).
2. Generator additions: tighten-on-`status='open'`, Grouped-with-WHERE, guaranteed
   nullable `LoosenFilter` slot (findings 2–3).
3. Optional: a drop+reader composite case if one can be made admissible, else document
   that H drop-ordering is conformance-covered only.
