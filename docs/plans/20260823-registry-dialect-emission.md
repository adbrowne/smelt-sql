# Plan: the registry owns per-dialect emission, and an audit enumerates it against real engines

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Date**: 2026-08-23
**Research**: [`docs/research/20260823-registry-dialect-emission-audit.md`](../research/20260823-registry-dialect-emission-audit.md)
**Issue**: [#171](https://github.com/adbrowne/smelt-sql/issues/171)
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md), [`docs/specs/functions.md`](../specs/functions.md), [`docs/specs/architecture.md`](../specs/architecture.md), [`docs/specs/diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: Phase 0 lands every spec edit before any code. See Phase 0 for the exact section list.
**Tracking PR / branch**: `registry-dialect-emission`
**Docs**: code + specs + a new generated `docs/reference/dialect-coverage.md`

---

## Goal

A built-in's per-dialect spelling derives from `BuiltinRegistry`, never from a name-matched arm
in `printer.rs`; and a derived audit suite enumerates every `(registry entry, dialect)` pair
against real engines in two layered legs, publishing the classification as a gated standing table.

## Architecture

`Signature` gains three pieces of production data — `DialectId`-keyed `emission`, a `SyntaxForm`,
and a `RewriteId` handle for structural rewrites the printer still owns as code. The printer's
`FUNCTION_CALL` and `BINARY_EXPR` arms collapse to "resolve the entry, read its `Emission` for
`ctx.dialect.id()`, dispatch on the verdict". A new dev-only `smelt-oracle-testkit` crate lifts
the three oracles out of `smelt-db`'s test tree and gains a value-execution capability; a
registry-derived probe harness in `crates/smelt-db/tests/dialect_audit/` runs those probes
against an inline `VALUES` fixture on each engine and grades the result against a two-sided
ledger.

## Tech stack

Rust 2021, `rowan` CST, `duckdb` in-process, Spark via `docker exec spark-sql`, BigQuery via the
`python -m smelt.bigquery_type_oracle` line protocol. No new external crates.

---

## Global constraints

Copied from `CLAUDE.md` and the specs; every phase's requirements implicitly include these.

- **Verification gate is one command**: `bash .claude/scripts/verify-phase.sh` (fmt + clippy both
  feature sets + `cargo test --quiet` + `example_diagnostics`). `--fast` skips the full test run.
  Never run the four separately.
- **Red-green TDD**: a failing test before any implementation, in every phase.
- **Fail-loud discipline**: no silent fallback to a default. An unrecognised input emits a
  diagnostic.
- **`smelt-dialect` is lightweight** (`architecture.md` §Constraints #5): no Arrow / Tokio /
  DuckDB dependencies, including dev-dependencies. The audit suite therefore lives in
  `smelt-db`'s test tree, not `smelt-dialect`'s.
- **`unwrap`/`expect` ratchet**: `crates/smelt-oracle-testkit` must derive as test-support —
  dev-dependency of ≥1 crate, regular dependency of none, **no** `src/main.rs` and no `[[bin]]`.
  It must then have **no** row in `.claude/hardening-baseline.txt`; adding one trips the
  `ORPHANED BASELINE ENTRY` sweep in `.claude/scripts/hardening-budget.sh:217-234`.
- **`.claude/` is gitignore-whitelisted** (`.gitignore:10-20`). A new
  `.claude/dialect-gaps-baseline.txt` needs a matching `!.claude/dialect-gaps-baseline.txt` line
  or it will never be committed.
- **Dialect slugs are already spelled** `duckdb` / `spark` / `postgres` / `bigquery`
  (`smelt-runtime/src/compile.rs:1363-1368`, `divergences.rs::find_divergence`). `DialectId::slug()`
  must return exactly those strings — a third spelling is a defect.
- **Atomic per-phase commits** using the phase's `Commit.` line verbatim. Never `--no-verify`.

---

## Ground truth corrections to the research doc

The research doc was written from estimates. These are measured; the plan uses these numbers.

| Research doc says | Actually |
|---|---|
| 232 `Signature::new` rows | **118 call sites** in `REGISTRY` (`signatures.rs:3802-4870`), 6 inside `for` loops → **~144 canonical entries + 11 aliases** |
| `remap_function_name` — 5 names across 4 dialects | 5 canonical names, **11** `(name, dialect)` rename facts (see the matrix in Phase 3) |
| `~500` probes | ~144 entries × 4 dialects, aggregates probed twice → **~700 probe slots**, batched into a few dozen queries |
| Oracles "roughly 1,300 lines move" | The oracle *transport* is 1,000 lines; `oracle_check.rs` cannot move wholesale — it depends on `smelt-db` inference and `generators.rs`. See Phase 6 for the sharpened seam. |

**Two findings that change scope, discovered while writing this plan:**

1. **`EXPLODE` and `UNNEST` are not registry entries at all.**
   `grep -c '"EXPLODE"' crates/smelt-types/src/signatures.rs` → `0`; same for `UNNEST`. The
   printer renames names the registry has never heard of. Registry-owned emission for them
   requires adding the entries first (Phase 3).

2. **Spark's `^` is bitwise XOR — smelt emits a silently-wrong `^` on Spark today.**
   Spark's `BitwiseXor` expression is documented `expr1 ^ expr2 - Returns the result of bitwise
   exclusive OR` ([apache/spark `bitwiseExpressions.scala`](https://github.com/apache/spark/blob/master/sql/catalyst/src/main/scala/org/apache/spark/sql/catalyst/expressions/bitwiseExpressions.scala)).
   smelt's grammar reads `^` as power (DuckDB semantics), and `printer.rs:281` lowers it **only**
   for BigQuery. `crates/smelt-dialect/tests/power_lowering.rs:91`
   (`other_dialects_keep_infix_caret_and_double_star_verbatim`) actively pins the wrong
   behaviour for Spark. This is exactly the silent-divergence class issue #171 was filed about,
   and it is a live bug, not a hypothetical. Phase 3 fixes it; Phase 10 proves the fix on a live
   Spark.

---

## File structure

**Created:**

| Path | Responsibility |
|---|---|
| `crates/smelt-types/src/dialect_id.rs` | `DialectId` + slug mapping. Below every other crate. |
| `crates/smelt-oracle-testkit/Cargo.toml` | Dev-only test-support crate manifest. |
| `crates/smelt-oracle-testkit/src/lib.rs` | Re-exports. |
| `crates/smelt-oracle-testkit/src/arrow_mapping.rs` | Moved verbatim from `prop_helpers/`. |
| `crates/smelt-oracle-testkit/src/duckdb_oracle.rs` | Moved; `TypeOracle` trait + `DuckDbOracle`. |
| `crates/smelt-oracle-testkit/src/spark_oracle.rs` | Moved verbatim. |
| `crates/smelt-oracle-testkit/src/bigquery_oracle.rs` | Moved; gains `execute_rows`. |
| `crates/smelt-oracle-testkit/src/error_class.rs` | `OracleErrorKind` + `classify_oracle_error`, split out of `oracle_check.rs`. |
| `crates/smelt-oracle-testkit/src/type_comparison.rs` | Moved verbatim. |
| `crates/smelt-oracle-testkit/src/value.rs` | `Cell`, `ValueOracle`, `compare_cells`. New. |
| `crates/smelt-dialect/src/emission_check.rs` | Pure pre-print walk yielding `UnsupportedEmission`. |
| `crates/smelt-dialect/tests/emission_ownership.rs` | Source-scan gate: no name-matched dialect arms remain. |
| `crates/smelt-db/tests/dialect_audit/main.rs` | Test entry points. |
| `crates/smelt-db/tests/dialect_audit/fixture.rs` | The inline `VALUES` fixture, per dialect. |
| `crates/smelt-db/tests/dialect_audit/probe.rs` | `Probe` + registry-derived probe construction + batching. |
| `crates/smelt-db/tests/dialect_audit/overrides.rs` | The only hand-written per-function table. |
| `crates/smelt-db/tests/dialect_audit/ledger.rs` | `dialect_divergences()` + the two-sided check. |
| `crates/smelt-db/tests/dialect_audit/report.rs` | Renders `docs/reference/dialect-coverage.md`. |
| `crates/smelt-runtime/tests/dialect_seam.rs` | Five end-to-end models through `execute_project`. |
| `docs/reference/dialect-coverage.md` | Generated. `docs/reference/` does not exist yet. |
| `.claude/dialect-gaps-baseline.txt` | Shrink-only `Gap` ratchet. |
| `scripts/bigquery-dialect-audit.sh` | Manual BigQuery sweep, fail-loud on missing credentials. |

**Modified:** `crates/smelt-types/src/{lib.rs,signatures.rs}`, `crates/smelt-dialect/src/{lib.rs,dialect.rs,printer.rs}`, `crates/smelt-db/src/diagnostics_types.rs`, `crates/smelt-db/tests/prop_helpers/{mod.rs,oracle_check.rs}` (+ every importer), `crates/smelt-db/tests/integration/registry_consistency.rs`, `crates/smelt-runtime/src/compile.rs`, `python/smelt/bigquery_type_oracle.py`, `.github/workflows/{test.yml,compat.yml}`, `.gitignore`, `CLAUDE.md`, four specs.

---

## Progress tracking

| Phase | Title | Needs live warehouse | Status |
|---|---|---|---|
| 0 | Spec deltas (spec-first) | no | pending |
| 1 | `DialectId`, and `engine_native` migrates onto it | no | pending |
| 2 | `SyntaxForm` on `Signature`; operators become entries | no | pending |
| 3 | `Emission` + `RewriteId`; author the non-`Native` rows | no | pending |
| 4 | Printer refactor + emission-ownership gate | no | pending |
| 5 | `Unsupported` becomes a compile-time diagnostic | no | pending |
| 6 | Extract `smelt-oracle-testkit` | no | pending |
| 7 | Value-execution capability on all three oracles | Spark + BigQuery to verify | pending |
| 8 | Fixture, derived probes, coverage-totality gate | no | pending |
| 9 | Schema + value legs on DuckDB; ledger + ratchet | no | pending |
| 10 | Spark leg + the `^` proof; CI wiring | Spark | pending |
| 11 | BigQuery leg + manual sweep script | BigQuery | pending |
| 12 | Seam leg, generated coverage table, `CLAUDE.md` | no | pending |

---

## Phase 0: Spec deltas

The spec-first rule (`CLAUDE.md` §Key Documentation) requires the spec to change before the code.
This phase is docs-only and has no test cycle of its own; its gate is the diagnostics-catalogue
test in Phase 5 and the reviewer.

**Files:**
- Modify: `docs/specs/multi_backend.md` §Semantics `### Exact-median lowering` (:181), `### Operator lowering` (:197); §Constraints (:434); §Known Divergences (:499)
- Modify: `docs/specs/functions.md` §Surface (new `### Registry emission surface` after `### Function-declaration frontmatter` :163); §Constraints (:264)
- Modify: `docs/specs/architecture.md` §Constraints item 13 (dialect conformance gates) and item 14 (function-registry single ownership); §Known Divergences (:505-509)
- Modify: `docs/specs/diagnostics.md` §Catalogue — new `### Dialect emission` group

- [ ] **Step 1: `multi_backend.md` §"Operator lowering"** — rewrite to state emission ownership
      and correct the Spark `^` claim. The current text says "Every other dialect prints all three
      unchanged", which is wrong for Spark. New text must state: `^` is power in smelt's grammar
      and in DuckDB and PostgreSQL, but **bitwise XOR in both GoogleSQL and Spark SQL**, so both
      lower to `POWER(a, b)`; `%` has no infix form in GoogleSQL and lowers to `MOD(a, b)`;
      `//` is not lowered anywhere and is declared `Unsupported` on Spark, PostgreSQL and
      BigQuery so the compiler refuses it rather than the engine.
- [ ] **Step 2: `multi_backend.md` §"Exact-median lowering"** — append one paragraph: the
      *decision* that `MEDIAN` needs rewriting on BigQuery is registry data
      (`Emission::Rewrite(RewriteId::BigQueryMedian)`); the *position-dependent shape* of the
      rewrite stays printer code, because it depends on the CST's window/aggregate position which
      no static table can express.
- [ ] **Step 3: `multi_backend.md` §Semantics** — add `### Cross-engine emission audit` after
      §"Operator lowering": the two legs, the fixture, the tier table (copied from the research
      doc §4), the ledger verdicts, and the location + meaning of
      `docs/reference/dialect-coverage.md`. State that the table is derived from registry + ledger
      alone, so it is deterministic and gateable per-PR, and that the legs test the claims the
      table makes rather than producing it.
- [ ] **Step 4: `multi_backend.md` §Constraints** — new item: "a built-in's per-dialect spelling
      derives from `BuiltinRegistry`; `printer.rs` holds no name-matched dialect arm", naming the
      gate `cargo test -p smelt-dialect --test emission_ownership`.
- [ ] **Step 5: `functions.md` §Surface** — new `### Registry emission surface`: `DialectId`,
      `SyntaxForm`, `Emission`, `RewriteId`, `Signature::emission_for`, and the rule that
      `Native` is a **claim** an untested pair reports as *unverified*, not as *passing*.
- [ ] **Step 6: `architecture.md` §Constraints item 14** — extend "name, classification, type" to
      "name, classification, type, **and emission**". Replace the parenthetical about the
      "named exclusion list" for operators: the exemption is now derived from
      `sig.syntax_form != SyntaxForm::Call`.
- [ ] **Step 7: `architecture.md` §Constraints item 13** — add the audit suite as a fifth bullet
      alongside accept/fidelity/corpus/oracle-strictness.
- [ ] **Step 8: `architecture.md` §Known Divergences** — add: PostgreSQL carries registry
      emission verdicts but has no backend crate and no oracle, so no leg exercises them and the
      coverage table marks them *unverified*.
- [ ] **Step 9: `diagnostics.md` §Catalogue** — new `### Dialect emission` group with the
      standard `| Code | Severity | Trigger |` header and one row:
      `` | `UnsupportedOnBackend` | Error | A model uses a built-in or operator the registry declares unsupported on the selected backend's dialect; the compiler refuses rather than emitting SQL the engine will reject at runtime. | ``
- [ ] **Step 10: Commit.** `docs: spec deltas for registry-owned dialect emission (#171)`

---

## Phase 1: `DialectId`, and `engine_native` migrates onto it

**Files:**
- Create: `crates/smelt-types/src/dialect_id.rs`
- Modify: `crates/smelt-types/src/lib.rs` — add `pub mod dialect_id;` and `pub use dialect_id::DialectId;`
- Modify: `crates/smelt-types/src/signatures.rs:2737` (`engine_native` field), `:2845-2846` (defaults), `:2887-2891` (`with_engine_native`), `:2913-2922` (`needs_cast_for`), `:3825-3833` (the sole `SUM` seed use), `:5780-5838` (three unit tests)
- Modify: `crates/smelt-dialect/src/dialect.rs` — `SqlDialect::id()`

**Interfaces produced (later phases rely on these exact names):**

```rust
// crates/smelt-types/src/dialect_id.rs
/// A backend SQL dialect, as an identity the registry can key on.
///
/// Replaces the stringly-keyed `engine_native` convention, where a typo'd key
/// silently meant "no override" — a fail-loud violation this table must not
/// inherit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DialectId {
    DuckDb,
    SparkSql,
    PostgreSql,
    BigQuery,
}

impl DialectId {
    /// Every dialect, in report order. Exhaustive by construction: adding a
    /// variant without adding it here fails `all_is_exhaustive`.
    pub const ALL: &'static [DialectId] = &[
        DialectId::DuckDb,
        DialectId::SparkSql,
        DialectId::PostgreSql,
        DialectId::BigQuery,
    ];

    /// The lowercase key already used by `smelt-runtime`'s as-struct emitter and
    /// the type-divergence ledger. There must not be a second spelling.
    pub fn slug(self) -> &'static str {
        match self {
            DialectId::DuckDb => "duckdb",
            DialectId::SparkSql => "spark",
            DialectId::PostgreSql => "postgres",
            DialectId::BigQuery => "bigquery",
        }
    }

    pub fn from_slug(slug: &str) -> Option<DialectId> {
        DialectId::ALL.iter().copied().find(|d| d.slug() == slug)
    }
}
```

```rust
// crates/smelt-dialect/src/dialect.rs — new method on the existing impl
impl SqlDialect {
    pub fn id(self) -> DialectId {
        match self {
            SqlDialect::DuckDB => DialectId::DuckDb,
            SqlDialect::SparkSQL => DialectId::SparkSql,
            SqlDialect::PostgreSQL => DialectId::PostgreSql,
            SqlDialect::BigQuery => DialectId::BigQuery,
        }
    }
}
```

Migrated signatures on `Signature`: `pub engine_native: HashMap<DialectId, DataType>`,
`pub fn with_engine_native(mut self, dialect: DialectId, dt: DataType) -> Self`,
`pub fn needs_cast_for(&self, dialect: DialectId) -> bool`.

- [ ] **Step 1: Write the failing tests.** Add to `crates/smelt-types/src/dialect_id.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_exhaustive() {
        // A new variant added without extending ALL fails here: the match is
        // exhaustive, so every variant must be produced by the iteration.
        for d in DialectId::ALL {
            match d {
                DialectId::DuckDb | DialectId::SparkSql
                | DialectId::PostgreSql | DialectId::BigQuery => {}
            }
        }
        assert_eq!(DialectId::ALL.len(), 4);
    }

    #[test]
    fn slug_round_trips_and_matches_the_existing_spelling() {
        for d in DialectId::ALL {
            assert_eq!(DialectId::from_slug(d.slug()), Some(*d));
        }
        // These four strings are load-bearing: smelt-runtime's as-struct emitter
        // and the type-divergence ledger already key on them.
        assert_eq!(DialectId::DuckDb.slug(), "duckdb");
        assert_eq!(DialectId::SparkSql.slug(), "spark");
        assert_eq!(DialectId::PostgreSql.slug(), "postgres");
        assert_eq!(DialectId::BigQuery.slug(), "bigquery");
    }

    #[test]
    fn an_unknown_slug_is_none_not_a_default() {
        assert_eq!(DialectId::from_slug("duckdb "), None);
        assert_eq!(DialectId::from_slug("DuckDb"), None);
        assert_eq!(DialectId::from_slug("snowflake"), None);
    }
}
```

  And in `crates/smelt-dialect/src/dialect.rs`'s existing `mod tests`:

```rust
#[test]
fn every_sql_dialect_maps_to_a_distinct_dialect_id() {
    let dialects = [
        SqlDialect::DuckDB,
        SqlDialect::SparkSQL,
        SqlDialect::PostgreSQL,
        SqlDialect::BigQuery,
    ];
    let ids: Vec<_> = dialects.iter().map(|d| d.id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "SqlDialect::id() is not injective");
    assert_eq!(sorted.len(), DialectId::ALL.len(), "a DialectId has no SqlDialect");
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-types dialect_id 2>&1 | tail -20` — expected: does not compile,
      `unresolved module dialect_id`.
- [ ] **Step 3: Implement `DialectId`** as shown, wire `pub mod dialect_id;` +
      `pub use dialect_id::DialectId;` into `crates/smelt-types/src/lib.rs`, and add
      `SqlDialect::id()` to `crates/smelt-dialect/src/dialect.rs` with
      `use smelt_types::DialectId;`.
- [ ] **Step 4: Migrate `engine_native` to `DialectId` keys.** Change the field type, drop the
      `engine.trim().to_ascii_lowercase()` normalisation from `with_engine_native` and
      `needs_cast_for` (it exists only to paper over stringly keys), update the `SUM` seed at
      `signatures.rs:3826` to `.with_engine_native(DialectId::DuckDb, DataType::Decimal { precision: 38, scale: 0 })`,
      and update the three `needs_cast_for` unit tests at `:5781`, `:5810`, `:5823`.
      There are **no consumers outside `smelt-types`** — verify with
      `cargo test --workspace --quiet 2>&1 | tail -20` rather than by grep.
- [ ] **Step 5: Run to verify pass.**
      `cargo test -p smelt-types --quiet 2>&1 | tail -20` and
      `cargo test -p smelt-dialect --quiet 2>&1 | tail -20`. Expected: PASS.
- [ ] **Step 6:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 7: Commit.** `feat(types): DialectId replaces stringly-keyed engine_native (#171)`

---

## Phase 2: `SyntaxForm` on `Signature`; operators become entries

Registers the operator surface so a registry-only enumeration cannot walk past `^`, and deletes
the hand-written `OPERATOR_REGISTRY_ENTRIES` list by deriving the exemption from registry data.

**Files:**
- Modify: `crates/smelt-types/src/signatures.rs` — `SyntaxForm` enum, `Signature.syntax_form`, `with_syntax_form`, `try_new` default, the 9 operator-stub rows (:4807-4867), `DATE_ADD`/`DATE_SUB` (:4564, :4570), 5 new infix rows, 2 new table-function rows
- Modify: `crates/smelt-db/tests/integration/registry_consistency.rs:20-32` (delete `OPERATOR_REGISTRY_ENTRIES`), `:72-74` (the skip)
- Modify: `crates/smelt-types/tests/registry_coverage.rs` — new section

**Interfaces produced:**

```rust
/// How a built-in is spelled at a call site. Required so operators can be
/// registry entries at all, and what lets the audit harness derive a probe from
/// a signature instead of a hand-written table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SyntaxForm {
    /// `NAME(a, b)` — the default, and the only form on the callable-function surface.
    #[default]
    Call,
    /// `a OP b` — `%`, `^`, `**`, `||`, `//`, `LIKE`, `ILIKE`, `GLOB`.
    Infix,
    /// `a OP` — `IS NULL`, `IS NOT NULL`.
    Postfix,
    /// `FROM UNNEST(a)` — a table function; not a scalar call position.
    TableFn,
    /// Dedicated syntax with no uniform shape — `CAST(x AS T)`, `a BETWEEN b AND c`,
    /// `a IN (…)`, `EXISTS (…)`, interval add/sub.
    Special,
}
```

`Signature` gains `pub syntax_form: SyntaxForm`, defaulted to `SyntaxForm::Call` in `try_new`,
set by `pub fn with_syntax_form(mut self, form: SyntaxForm) -> Self`.

**Assignment (exhaustive — every non-`Call` entry):**

| Entry | Form | Note |
|---|---|---|
| `%` `^` `**` `\|\|` `//` | `Infix` | **new rows** |
| `LIKE` `ILIKE` `GLOB` | `Infix` | existing stubs, :4807-4830 |
| `IS_NULL` `IS_NOT_NULL` | `Postfix` | existing stubs |
| `BETWEEN` `IN` `EXISTS` `CAST` | `Special` | existing stubs |
| `DATE_ADD` `DATE_SUB` | `Special` | :4564, :4570 — model interval add/sub, not callable |
| `EXPLODE` `UNNEST` | `TableFn` | **new rows** — see Phase 3 finding |

That set is exactly the 11 names in today's `OPERATOR_REGISTRY_ENTRIES` plus the 7 new rows, so
the consistency gate's behaviour on existing entries is unchanged.

**New rows to seed** (append a `// ─── Infix operators` section after the operator stubs at :4867):

```rust
    // ─── Infix operators.
    //
    // These are BINARY_EXPR in the CST, not FUNCTION_CALL, and were absent from
    // the registry entirely. They are registered so per-dialect emission has one
    // owner and so the audit enumeration cannot walk past them — `^` is the
    // silent-divergence case issue #171 was filed about.
    for op in ["%", "^", "**", "//"] {
        insert(
            Signature::new(
                op,
                vec![tp("T", TypeConstraint::Numeric)],
                vec![var("T"), var("T")],
                TypeExpr::Var("T".into()),
            )
            .with_syntax_form(SyntaxForm::Infix),
        );
    }
    insert(
        Signature::new(
            "||",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_syntax_form(SyntaxForm::Infix),
    );

    // ─── Table functions.
    for name in ["EXPLODE", "UNNEST"] {
        insert(
            Signature::new(
                name,
                vec![tp("T", TypeConstraint::Any)],
                vec![var("T")],
                TypeExpr::Var("T".into()),
            )
            .with_syntax_form(SyntaxForm::TableFn),
        );
    }
```

> **Note on `%`'s type.** `MOD` is already in the residual hand-written-match set
> (`.claude/registry-migration-baseline.txt`, `architecture.md` §Known Divergences names `MOD`
> as width-widening). The `%` row's `TypeExpr::Var("T")` is therefore **not** wired into type
> inference by this phase — `dispatch.rs:335` reads `sig.kind` only for `FUNCTION_CALL` names,
> and `BINARY_EXPR` typing lives in `type_inference/binary.rs`. Confirm this with
> `cargo test -p smelt-db --quiet` after seeding; if any inference test moves, the row's
> `return_type` is being consulted and the phase must reconcile it before proceeding, not
> weaken the test.

- [ ] **Step 1: Write the failing test.** Append to `crates/smelt-types/tests/registry_coverage.rs`:

```rust
// ─── Syntax forms and the operator surface

#[test]
fn infix_operators_are_registered_with_the_infix_form() {
    for op in ["%", "^", "**", "//", "||"] {
        let sig = BuiltinRegistry::resolve(op)
            .unwrap_or_else(|| panic!("operator {op} not in registry"));
        assert_eq!(
            sig.syntax_form,
            SyntaxForm::Infix,
            "{op} must be Infix so the audit enumerates it as an operator"
        );
    }
}

#[test]
fn table_functions_are_registered_with_the_tablefn_form() {
    for name in ["EXPLODE", "UNNEST"] {
        let sig = BuiltinRegistry::resolve(name)
            .unwrap_or_else(|| panic!("{name} not in registry"));
        assert_eq!(sig.syntax_form, SyntaxForm::TableFn);
    }
}

#[test]
fn ordinary_functions_default_to_the_call_form() {
    for name in ["SUM", "LOWER", "ROW_NUMBER", "DATE_TRUNC"] {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(sig.syntax_form, SyntaxForm::Call);
    }
}

#[test]
fn dedicated_syntax_entries_are_not_call_form() {
    // The exemption the registry-consistency gate derives. Each of these is a
    // registry entry for hover/completion but not a callable function.
    for name in [
        "LIKE", "ILIKE", "GLOB", "IS_NULL", "IS_NOT_NULL",
        "BETWEEN", "IN", "EXISTS", "CAST", "DATE_ADD", "DATE_SUB",
    ] {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_ne!(
            sig.syntax_form,
            SyntaxForm::Call,
            "{name} is dedicated syntax; leaving it Call re-enters it into the \
             callable-function consistency gate"
        );
    }
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-types --test registry_coverage 2>&1 | tail -20` — expected: no
      `SyntaxForm` in scope.
- [ ] **Step 3: Implement.** Add the enum, the field, the default in `try_new`, the builder, and
      the `.with_syntax_form(...)` calls on the 13 existing entries plus the 7 new rows.
- [ ] **Step 4: Run to verify pass.** `cargo test -p smelt-types --quiet 2>&1 | tail -20`.
- [ ] **Step 5: Delete `OPERATOR_REGISTRY_ENTRIES`.** In
      `crates/smelt-db/tests/integration/registry_consistency.rs`, remove the const at `:20-32`
      and replace the skip at `:72-74` with:

```rust
        // Dedicated-syntax entries (operators, CAST, interval add/sub, table
        // functions) are exempt from the callable-function surface. The
        // exemption is registry data, not a hand-written list: a new operator
        // entry is exempt automatically, and an entry that stops being one
        // re-enters the gate.
        if sig.syntax_form != SyntaxForm::Call {
            continue;
        }
```

      Note this needs `sig` in scope where the loop currently has only `name`; resolve with
      `let Some(sig) = BuiltinRegistry::resolve(name) else { continue };` — a name from
      `BuiltinRegistry::names()` always resolves.
- [ ] **Step 6: Run the consistency gate.**
      `cargo test -p smelt-db --test integration registry_consistency 2>&1 | tail -20`.
      Expected: PASS with the same set as before. If a name newly fails, the `SyntaxForm`
      assignment table above is incomplete — extend it, do not re-add an exclusion list.
- [ ] **Step 7:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 8: Commit.** `feat(types): SyntaxForm makes the operator surface registry-visible (#171)`

---

## Phase 3: `Emission` + `RewriteId`; author the non-`Native` rows

**Files:**
- Modify: `crates/smelt-types/src/signatures.rs` — `Emission`, `RewriteId`, `Signature.emission`, `with_emission`, `emission_for`, and 10 seed rows
- Modify: `crates/smelt-types/tests/registry_coverage.rs`

**Interfaces produced:**

```rust
/// The per-dialect verdict for one registry entry: how the built-in must be
/// spelled so the backend computes what smelt's semantics say it computes.
///
/// `Native` is a **claim, not an assumption**. The audit's value leg exists to
/// test it, and an untested `Native` is reported as *unverified* rather than as
/// *passing*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// Same spelling, same semantics.
    Native,
    /// Same call shape, different name.
    Rename(&'static str),
    /// Structural rewrite: the printer owns the code, the registry owns the claim.
    Rewrite(RewriteId),
    /// The backend cannot express this. A diagnostic, never a silent pass-through.
    Unsupported { reason: &'static str },
}

/// A structural rewrite the printer implements. Enumerable by construction, so
/// the set of rewrites is knowable without reading `printer.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewriteId {
    /// `MEDIAN(x)` → `PERCENTILE_CONT(x, 0.5)` in window position, an
    /// `ARRAY_AGG`-indexing `CASE` in aggregate position. Position-dependent;
    /// the registry says *that* it needs rewriting, the printer says *how*.
    BigQueryMedian,
    /// `a % b` → `MOD(a, b)`.
    ModuloCall,
    /// `a ^ b` / `a ** b` → `POWER(a, b)`. Needed wherever infix `^` means
    /// bitwise XOR (GoogleSQL **and** Spark SQL) or `**` is unparseable.
    PowerCall,
}
```

`Signature` gains `pub emission: &'static [(DialectId, Emission)]`, defaulted to `&[]` in
`try_new`, set by `pub fn with_emission(mut self, table: &'static [(DialectId, Emission)]) -> Self`,
read by:

```rust
    /// The emission verdict for `dialect`. Unlisted dialects are `Native`.
    pub fn emission_for(&self, dialect: DialectId) -> Emission {
        self.emission
            .iter()
            .find(|(d, _)| *d == dialect)
            .map(|(_, e)| *e)
            .unwrap_or(Emission::Native)
    }
```

**The complete non-`Native` matrix.** Every row below is either transcribed from today's
`remap_function_name` / `print_bigquery_*` behaviour or is one of the two corrections named in
"Ground truth corrections". Nothing else is authored from memory.

| Entry | DuckDb | SparkSql | PostgreSql | BigQuery | Source |
|---|---|---|---|---|---|
| `EXPLODE` | `Rename("UNNEST")` | — | `Rename("UNNEST")` | `Rename("UNNEST")` | `printer.rs:1884,1893,1914` |
| `UNNEST` | — | `Rename("EXPLODE")` | — | — | `printer.rs:1900` |
| `EVERY` | `Rename("BOOL_AND")` | — | — | `Rename("LOGICAL_AND")` | `printer.rs:1886,1916` |
| `BOOL_AND` | — | `Rename("EVERY")` | — | `Rename("LOGICAL_AND")` | `printer.rs:1902,1916` |
| `BOOL_OR` | — | `Rename("SOME")` | — | `Rename("LOGICAL_OR")` | `printer.rs:1904,1918` |
| `MEDIAN` | — | — | — | `Rewrite(BigQueryMedian)` | `printer.rs:257-260` |
| `%` | — | — | — | `Rewrite(ModuloCall)` | `printer.rs:272` |
| `^` | — | **`Rewrite(PowerCall)`** | — | `Rewrite(PowerCall)` | `printer.rs:281` + Spark XOR finding |
| `**` | — | **`Rewrite(PowerCall)`** | — | `Rewrite(PowerCall)` | `printer.rs:281` + Spark has no `**` |
| `//` | — | **`Unsupported`** | **`Unsupported`** | **`Unsupported`** | `multi_backend.md:210-216`, made loud earlier |

`||` is `Native` on all four — an explicit claim the value leg tests, not an omission.

Seed authoring shape:

```rust
    insert(
        Signature::new(
            "BOOL_OR",
            /* … existing args unchanged … */
        )
        .with_kind(ExprKind::Agg)
        .with_emission(&[
            (DialectId::SparkSql, Emission::Rename("SOME")),
            (DialectId::BigQuery, Emission::Rename("LOGICAL_OR")),
        ]),
    );
```

The `//` row (inside the infix `for` loop from Phase 2, so it must be lifted out):

```rust
    insert(
        Signature::new(
            "//",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T"), var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_syntax_form(SyntaxForm::Infix)
        .with_emission(&[
            // DuckDB's `//` truncates toward zero for integer operands and
            // degrades to plain division for floats; the printer carries no
            // operand types with which to tell those cases apart, so no
            // substitution is safe. Declaring it unsupported turns an engine-side
            // syntax error into a compile-time diagnostic.
            (DialectId::SparkSql, Emission::Unsupported {
                reason: "Spark SQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)",
            }),
            (DialectId::PostgreSql, Emission::Unsupported {
                reason: "PostgreSQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)",
            }),
            (DialectId::BigQuery, Emission::Unsupported {
                reason: "GoogleSQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)",
            }),
        ]),
    );
```

- [ ] **Step 1: Write the failing test.** Append to `crates/smelt-types/tests/registry_coverage.rs`:

```rust
// ─── Emission

#[test]
fn the_rename_matrix_matches_the_printer_it_replaces() {
    // Transcribed from `remap_function_name` (printer.rs:1881-1925) before its
    // deletion. This test is what makes the printer refactor mechanical.
    let expected: &[(&str, DialectId, &str)] = &[
        ("EXPLODE", DialectId::DuckDb, "UNNEST"),
        ("EXPLODE", DialectId::PostgreSql, "UNNEST"),
        ("EXPLODE", DialectId::BigQuery, "UNNEST"),
        ("UNNEST", DialectId::SparkSql, "EXPLODE"),
        ("EVERY", DialectId::DuckDb, "BOOL_AND"),
        ("EVERY", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_AND", DialectId::SparkSql, "EVERY"),
        ("BOOL_AND", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_OR", DialectId::SparkSql, "SOME"),
        ("BOOL_OR", DialectId::BigQuery, "LOGICAL_OR"),
    ];
    for (name, dialect, renamed) in expected {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(
            sig.emission_for(*dialect),
            Emission::Rename(renamed),
            "{name} on {}", dialect.slug()
        );
    }
}

#[test]
fn every_is_native_on_postgresql() {
    // `remap_function_name` deliberately leaves PostgreSQL's EVERY alone while
    // DuckDB rewrites it; snapshots.rs:401 pins the asymmetry.
    let sig = BuiltinRegistry::resolve("EVERY").expect("EVERY");
    assert_eq!(sig.emission_for(DialectId::PostgreSql), Emission::Native);
}

#[test]
fn caret_is_rewritten_wherever_infix_caret_means_xor() {
    // GoogleSQL and Spark SQL both define infix `^` as bitwise XOR while smelt's
    // grammar reads it as power. Emitting it verbatim returns a different number
    // rather than failing — the silent-divergence class this work exists to close.
    for dialect in [DialectId::SparkSql, DialectId::BigQuery] {
        for op in ["^", "**"] {
            let sig = BuiltinRegistry::resolve(op).expect(op);
            assert_eq!(
                sig.emission_for(dialect),
                Emission::Rewrite(RewriteId::PowerCall),
                "{op} on {}", dialect.slug()
            );
        }
    }
    for op in ["^", "**"] {
        let sig = BuiltinRegistry::resolve(op).expect(op);
        assert_eq!(sig.emission_for(DialectId::DuckDb), Emission::Native);
        assert_eq!(sig.emission_for(DialectId::PostgreSql), Emission::Native);
    }
}

#[test]
fn floor_divide_is_unsupported_everywhere_it_has_no_safe_lowering() {
    let sig = BuiltinRegistry::resolve("//").expect("//");
    assert_eq!(sig.emission_for(DialectId::DuckDb), Emission::Native);
    for dialect in [DialectId::SparkSql, DialectId::PostgreSql, DialectId::BigQuery] {
        assert!(
            matches!(sig.emission_for(dialect), Emission::Unsupported { .. }),
            "// on {} must be a declared refusal, not a pass-through", dialect.slug()
        );
    }
}

#[test]
fn an_unlisted_dialect_defaults_to_native() {
    let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER");
    for d in DialectId::ALL {
        assert_eq!(sig.emission_for(*d), Emission::Native);
    }
}

#[test]
fn every_declared_rewrite_id_is_reachable_from_some_entry() {
    // A RewriteId with no registry row is printer code nothing can call.
    let mut seen: Vec<RewriteId> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, e)| match e {
            Emission::Rewrite(id) => Some(*id),
            _ => None,
        })
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        vec![RewriteId::BigQueryMedian, RewriteId::ModuloCall, RewriteId::PowerCall],
    );
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-types --test registry_coverage 2>&1 | tail -20`.
- [ ] **Step 3: Implement** `Emission`, `RewriteId`, the field, the builder, `emission_for`, and
      the 10 seed rows from the matrix.
- [ ] **Step 4: Run to verify pass.** `cargo test -p smelt-types --quiet 2>&1 | tail -20`.
- [ ] **Step 5:** `bash .claude/scripts/verify-phase.sh`. The printer still uses
      `remap_function_name` at this point, so nothing downstream changes.
- [ ] **Step 6: Commit.** `feat(types): registry owns per-dialect emission verdicts (#171)`

---

## Phase 4: Printer refactor + emission-ownership gate

**Files:**
- Modify: `crates/smelt-dialect/src/printer.rs` — the `FUNCTION_CALL` arm (:251-268), the `BINARY_EXPR` arm (:267-285), delete `remap_function_name` (:1879-1925), rename `print_bigquery_modulo`→`print_modulo_call` and `print_bigquery_power`→`print_power_call`
- Create: `crates/smelt-dialect/tests/emission_ownership.rs`
- Modify: `crates/smelt-dialect/tests/power_lowering.rs:91,138` — the two tests that pin the old Spark behaviour

**Interfaces consumed:** `Signature::emission_for`, `Emission`, `RewriteId`, `SqlDialect::id()`.

**Implementation shape:**

```rust
        SyntaxKind::FUNCTION_CALL => {
            if let Some(fc) = FunctionCall::cast(node.clone()) {
                if let Some(name) = fc.name() {
                    if emit_registered_function(node, &fc, &name, ctx, out) {
                        return;
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::BINARY_EXPR => {
            if emit_registered_operator(node, ctx, out) {
                return;
            }
            print_children(node, ctx, out);
        }
```

```rust
/// Resolve the call's registry entry and dispatch on its emission verdict for
/// this dialect. Returns `true` when the node was fully printed.
///
/// `BuiltinRegistry::resolve` folds case, matching the printer's previous
/// `eq_ignore_ascii_case` behaviour; `FunctionCall::name()` returns the raw
/// source spelling.
fn emit_registered_function(
    node: &SyntaxNode,
    fc: &FunctionCall,
    name: &str,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let Some(sig) = BuiltinRegistry::resolve(name) else {
        return false;
    };
    match sig.emission_for(ctx.dialect.id()) {
        // An `Unsupported` entry still prints verbatim; the compile path refuses
        // the model before reaching the printer (see `emission_check`), so a
        // verbatim print here is unreachable in production and harmless in a
        // printer unit test.
        Emission::Native | Emission::Unsupported { .. } => false,
        Emission::Rename(new_name) => {
            print_function_with_renamed(node, ctx, out, new_name);
            true
        }
        Emission::Rewrite(id) => apply_rewrite(id, node, Some(fc), ctx, out),
    }
}

fn emit_registered_operator(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) -> bool {
    let Some(op) = BinaryExpr::cast(node.clone()).and_then(|b| b.operator()) else {
        return false;
    };
    let Some(sig) = BuiltinRegistry::resolve(&op) else {
        return false;
    };
    match sig.emission_for(ctx.dialect.id()) {
        Emission::Rewrite(id) => apply_rewrite(id, node, None, ctx, out),
        _ => false,
    }
}

/// The one place a `RewriteId` becomes code. Adding a variant is a compile error
/// here until it is implemented.
fn apply_rewrite(
    id: RewriteId,
    node: &SyntaxNode,
    fc: Option<&FunctionCall>,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    match id {
        RewriteId::BigQueryMedian => {
            fc.is_some_and(|fc| print_bigquery_median(node, fc, ctx, out))
        }
        RewriteId::ModuloCall => print_modulo_call(node, ctx, out),
        RewriteId::PowerCall => print_power_call(node, ctx, out),
    }
}
```

The three `print_*` bodies are unchanged apart from dropping their internal operator guards —
`print_modulo_call` no longer needs `if bin.operator().as_deref() != Some("%")`, because the
registry lookup already established it. **Keep** the `bool` return protocol and the
`push_trailing_trivia(node, out)` tail call in every rewrite; `spark_combined_rewrites` and
`the_lowered_sql_parses_back_with_its_alias_intact` fail without them.

- [ ] **Step 1: Record the residual dialect references before touching anything.**
      `grep -n 'SqlDialect::\|eq_ignore_ascii_case' crates/smelt-dialect/src/printer.rs`
      Write the output into the commit message body. The three
      `matches!(ctx.dialect, SqlDialect::BigQuery)` guards at :257, :272, :281 must be gone by
      the end of this phase; anything else the grep reports is an emission fact that must move
      to the registry or to a `BackendCapabilities` flag — **report it, do not weaken the gate.**
- [ ] **Step 2: Write the failing gate.** `crates/smelt-dialect/tests/emission_ownership.rs`:

```rust
//! The printer holds no name-matched dialect arm.
//!
//! `CLAUDE.md` §"Function-registry single ownership" extends to a built-in's
//! emission: a function's per-dialect spelling derives from `BuiltinRegistry`,
//! never from a `match dialect` / `eq_ignore_ascii_case` chain in the printer.
//! This is the sibling gate to `registry_consistency`.

const PRINTER_SRC: &str = include_str!("../src/printer.rs");

#[test]
fn the_printer_matches_no_function_name() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("eq_ignore_ascii_case"))
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs matches function names by string. Per-dialect spelling is \
         registry data (`Signature::emission`); move the fact into \
         `crates/smelt-types/src/signatures.rs` rather than re-adding an arm here.\n{hits:#?}"
    );
}

#[test]
fn the_printer_branches_on_no_dialect_variant() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            ["SqlDialect::DuckDB", "SqlDialect::SparkSQL",
             "SqlDialect::PostgreSQL", "SqlDialect::BigQuery"]
                .iter()
                .any(|v| l.contains(v))
        })
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs branches on a concrete dialect. Emission facts belong in \
         `Signature::emission`; capability-shaped differences belong in \
         `BackendCapabilities`.\n{hits:#?}"
    );
}

#[test]
fn every_rewrite_id_is_dispatched() {
    // A RewriteId the printer never mentions is a registry claim with no
    // implementation — the failure mode a name-matched `if` chain hid.
    for id in ["BigQueryMedian", "ModuloCall", "PowerCall"] {
        assert!(
            PRINTER_SRC.contains(&format!("RewriteId::{id}")),
            "RewriteId::{id} is declared in the registry but never dispatched in printer.rs"
        );
    }
}
```

- [ ] **Step 3: Run to verify failure.**
      `cargo test -p smelt-dialect --test emission_ownership 2>&1 | tail -30`
      Expected: all three fail, listing the guards at :257/:272/:281 and the
      `eq_ignore_ascii_case` sites at :258, :1884-1918.
- [ ] **Step 4: Refactor** per the implementation shape above. Delete `remap_function_name`
      entirely. Import `use smelt_types::{BuiltinRegistry, Emission, RewriteId};` — no Cargo
      change is needed, `smelt-types` is already a real dependency (`Cargo.toml:10`).
- [ ] **Step 5: Update the two tests pinning the old Spark behaviour.**
      `crates/smelt-dialect/tests/power_lowering.rs`:
      - `other_dialects_keep_infix_caret_and_double_star_verbatim` (:91) — narrow to DuckDB and
        PostgreSQL, and add a new `spark_lowers_infix_caret_to_power_call` asserting
        `SELECT POWER(a, b) FROM t` for Spark. Its doc comment must state *why*: Spark's `^` is
        bitwise XOR, so a verbatim emit returns a different number.
      - `no_lowering_registered_for_floor_divide` (:138) — retitle to
        `floor_divide_is_declared_unsupported_rather_than_lowered` and assert the printer still
        emits `//` verbatim **and** that `BuiltinRegistry::resolve("//").emission_for(...)` is
        `Unsupported` for Spark/PostgreSQL/BigQuery. The refusal lives in the compile path
        (Phase 5), not the printer.
- [ ] **Step 6: Run to verify pass.**
      `cargo test -p smelt-dialect --quiet 2>&1 | tail -30` — the six rename snapshots
      (`snapshots.rs:329-415`), the three `*_lowering.rs` suites, and the new gate.
- [ ] **Step 7: Run the downstream tests coupled to emission text.**
      `cargo test -p smelt-runtime --quiet 2>&1 | tail -20` and
      `cargo test -p smelt-cli --test explain_show_sql --quiet 2>&1 | tail -20`
      (`explain_show_sql.rs:508-618` asserts the BigQuery `MEDIAN` lowering end-to-end).
- [ ] **Step 8:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 9: Commit.** `refactor(dialect): printer reads emission from the registry (#171)`

---

## Phase 5: `Unsupported` becomes a compile-time diagnostic

Today an unrecognised function passes through verbatim and the backend rejects it after a
warehouse round trip. A declared `Unsupported` verdict makes it a compile-time failure.

**Files:**
- Create: `crates/smelt-dialect/src/emission_check.rs`
- Modify: `crates/smelt-dialect/src/lib.rs` — `mod emission_check;` + re-export
- Modify: `crates/smelt-db/src/diagnostics_types.rs` — `DiagnosticCode::UnsupportedOnBackend`
- Modify: `crates/smelt-runtime/src/compile.rs` — call the check before printing
- Test: `crates/smelt-dialect/tests/unsupported_emission.rs`, plus a compile-path test in `crates/smelt-runtime/tests/`

**Interfaces produced:**

```rust
// crates/smelt-dialect/src/emission_check.rs
use rowan::TextRange;
use smelt_types::DialectId;

/// One construct the target dialect cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedEmission {
    /// The canonical registry name (`"//"`, `"REGEXP_MATCHES"`, …).
    pub name: &'static str,
    pub dialect: DialectId,
    pub reason: &'static str,
    pub range: TextRange,
}

/// Walk the tree for constructs the registry declares unsupported on `dialect`.
///
/// Pure: no I/O, no printing. Ranges are `TextRange`, per the diagnostic
/// range-encoding invariant — conversion to line/column happens once, at the
/// diagnostic boundary.
pub fn unsupported_emissions(root: &SyntaxNode, dialect: SqlDialect) -> Vec<UnsupportedEmission>;
```

Implementation: `root.descendants()`, matching `FUNCTION_CALL` (name via `FunctionCall::name()`)
and `BINARY_EXPR` (operator via `BinaryExpr::operator()`), resolving each through
`BuiltinRegistry::resolve` and collecting `Emission::Unsupported`. Range is `node.text_range()`.

- [ ] **Step 1: Write the failing tests.** `crates/smelt-dialect/tests/unsupported_emission.rs`:

```rust
#[test]
fn floor_divide_is_reported_for_bigquery() {
    let tree = parse_expr_model("SELECT a // b AS q FROM t");
    let found = unsupported_emissions(&tree, SqlDialect::BigQuery);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "//");
    assert_eq!(found[0].dialect, DialectId::BigQuery);
    assert!(found[0].reason.contains("GoogleSQL"));
}

#[test]
fn the_same_model_is_clean_on_duckdb() {
    let tree = parse_expr_model("SELECT a // b AS q FROM t");
    assert!(unsupported_emissions(&tree, SqlDialect::DuckDB).is_empty());
}

#[test]
fn an_unregistered_function_is_not_reported_here() {
    // Recognition is `UnrecognizedFunction`'s job (registry_consistency); this
    // check reports only *declared* refusals, so the two diagnostics cannot
    // double-fire on the same construct.
    let tree = parse_expr_model("SELECT nonesuch(a) AS q FROM t");
    assert!(unsupported_emissions(&tree, SqlDialect::BigQuery).is_empty());
}

#[test]
fn every_occurrence_is_reported_with_its_own_range() {
    let tree = parse_expr_model("SELECT a // b AS q, c // d AS r FROM t");
    let found = unsupported_emissions(&tree, SqlDialect::BigQuery);
    assert_eq!(found.len(), 2);
    assert_ne!(found[0].range, found[1].range);
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-dialect --test unsupported_emission 2>&1 | tail -20`.
- [ ] **Step 3: Implement** `emission_check.rs` and export it from `lib.rs`.
- [ ] **Step 4: Run to verify pass.**
      `cargo test -p smelt-dialect --test unsupported_emission --quiet 2>&1 | tail -20`.
- [ ] **Step 5: Add the diagnostic code.** Add `UnsupportedOnBackend` to the `DiagnosticCode`
      enum in `crates/smelt-db/src/diagnostics_types.rs`. Run
      `cargo test -p smelt-db --test integration diagnostics_catalogue 2>&1 | tail -20` —
      it must FAIL until Phase 0's `docs/specs/diagnostics.md` row is present. If Phase 0 landed,
      it passes; if it fails, Phase 0 was skipped and must be completed first.
- [ ] **Step 6: Wire the compile path.** In `crates/smelt-runtime/src/compile.rs`, call
      `unsupported_emissions(&model_cst, dialect)` immediately before the `PrintContext` is used
      to print, and surface each result as an `UnsupportedOnBackend` diagnostic on the existing
      compile-error path. Locate that path by reading how the projection-derivation error
      surfaces (`compile.rs`, the same function that builds `output_columns`) rather than
      inventing a second error channel.
- [ ] **Step 7: Write the compile-path test.** In `crates/smelt-runtime/tests/dialect_seam.rs`
      (created here, extended in Phase 12):

```rust
#[test]
fn a_model_using_floor_divide_fails_to_compile_for_bigquery() {
    let model = make_model("q", "SELECT id, val // 2 AS halved FROM events");
    let err = registry()
        .get("bigquery")
        .expect("bigquery target")
        .compile(&model, "main")
        .expect_err("BigQuery has no `//`; the compiler must refuse before emitting SQL");
    let msg = format!("{err}");
    assert!(msg.contains("//"), "the diagnostic must name the construct: {msg}");
    assert!(msg.contains("BigQuery") || msg.contains("bigquery"),
            "the diagnostic must name the backend: {msg}");
}

#[test]
fn the_same_model_compiles_for_duckdb() {
    let model = make_model("q", "SELECT id, val // 2 AS halved FROM events");
    registry().get("duckdb").expect("duckdb target").compile(&model, "main")
        .expect("DuckDB has `//`");
}
```

- [ ] **Step 8: Run.** `cargo test -p smelt-runtime --test dialect_seam --quiet 2>&1 | tail -20`.
- [ ] **Step 9: Check no example workspace regresses.** A model in `examples/` using `//` against
      a non-DuckDB target would now fail:
      `cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20`.
- [ ] **Step 10:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 11: Commit.** `feat(runtime): declared-unsupported constructs fail at compile time (#171)`

---

## Phase 6: Extract `smelt-oracle-testkit`

A pure move plus one split. **No logic change**; the reviewer's job is to confirm that.

**Sharpened seam (deviation from the research doc, with reason):** `oracle_check.rs` cannot move
wholesale — `check_types_against_oracle` and `run_smelt_inference` depend on `smelt-db`'s
inference and on `generators::{TypedSource, TypedExpr}` (3,192 lines of smelt-db-specific
generation). Moving it would drag `generators.rs`, `divergences.rs` and `known_unknowns.rs` into
a crate that has no business owning them. What moves is the **oracle transport plus the
comparison primitives**: the three oracles, the Arrow map, the error classifier, and
`compare_types`. `check_types_against_oracle` stays in `smelt-db` and imports from the testkit.

**Files:**
- Create: `crates/smelt-oracle-testkit/Cargo.toml`, `src/lib.rs`
- Move (verbatim): `prop_helpers/{arrow_mapping,duckdb_oracle,spark_oracle,bigquery_oracle,type_comparison}.rs` → `crates/smelt-oracle-testkit/src/`
- Split: `prop_helpers/oracle_check.rs:26-124` (`OracleErrorKind`, `classify_oracle_error`, `is_recognized_query_refusal`, and the `classify_oracle_error_tests` module at `:264-369`) → `crates/smelt-oracle-testkit/src/error_class.rs`
- Modify: `crates/smelt-db/Cargo.toml` — `[dev-dependencies] smelt-oracle-testkit = { path = "../smelt-oracle-testkit" }`
- Modify: `crates/smelt-db/tests/prop_helpers/mod.rs`, `oracle_check.rs`, and every importer (5 top-level `tests/*.rs` + the `proptests` binary's 13 sub-modules — the full list is in the OracleScout report; find them with `grep -rn 'prop_helpers::' crates/smelt-db/tests`)

**Manifest** (copy `smelt-maintenance-testkit`'s shape):

```toml
[package]
name = "smelt-oracle-testkit"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
publish = false
description = "Dev-only cross-engine oracle transport: DuckDB / Spark / BigQuery schema and value probing, error classification, and the type comparator. Not a production dependency of any crate."

[dependencies]
smelt-types = { path = "../smelt-types" }
duckdb = { workspace = true }
arrow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
```

No `src/main.rs`, no `[[bin]]`, no `[features]`. The workspace uses `members = ["crates/*"]`, so
the root `Cargo.toml` needs no edit.

- [ ] **Step 1: Write the failing test.** `crates/smelt-oracle-testkit/tests/smoke.rs`:

```rust
use smelt_oracle_testkit::{classify_oracle_error, DuckDbOracle, OracleErrorKind, TypeOracle};
use smelt_types::DataType;

#[test]
fn the_duckdb_oracle_reports_a_schema() {
    let oracle = DuckDbOracle::new();
    let cols = oracle.query_types("SELECT 1 AS a, 'x' AS b").expect("query");
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].0, "a");
}

#[test]
fn a_binder_error_is_a_refusal_not_a_harness_failure() {
    assert_eq!(
        classify_oracle_error("Binder Error: no such column"),
        OracleErrorKind::QueryRefusal
    );
    assert_eq!(classify_oracle_error("connection reset"), OracleErrorKind::Fatal);
}

#[test]
fn compare_types_is_reachable_from_the_testkit() {
    use smelt_oracle_testkit::{compare_types, TypeMatch};
    assert_eq!(compare_types(&DataType::BigInt, &DataType::BigInt), TypeMatch::Exact);
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-oracle-testkit 2>&1 | tail -20` — expected: no such package.
- [ ] **Step 3: Create the crate and move the five files verbatim.** Change only `use` paths and
      module visibility. Re-export everything from `lib.rs`:

```rust
//! Dev-only cross-engine oracle transport.
//!
//! Promoted out of `smelt-db/tests/prop_helpers/` so more than one crate's test
//! tree can probe a live engine. Derived test-support (dev-dependency of some
//! crate, regular dependency of none, no binary target), so it sits outside the
//! `unwrap`/`expect` ratchet's production set and must have no row in
//! `.claude/hardening-baseline.txt`.

mod arrow_mapping;
mod bigquery_oracle;
mod duckdb_oracle;
mod error_class;
mod spark_oracle;
mod type_comparison;

pub use arrow_mapping::arrow_to_smelt;
pub use bigquery_oracle::{BigQueryOracle, BqField};
pub use duckdb_oracle::{DuckDbOracle, TypeOracle};
pub use error_class::{classify_oracle_error, OracleErrorKind};
pub use spark_oracle::SparkOracle;
pub use type_comparison::{compare_types, TypeMatch};
```

- [ ] **Step 4: Split `error_class.rs` out of `oracle_check.rs`.** Move `OracleErrorKind` (:26),
      `classify_oracle_error` (:54), `is_recognized_query_refusal` (:71), and the
      `classify_oracle_error_tests` module (:264-369) with its `cases()` table. The remaining
      `oracle_check.rs` keeps `OracleCheckOutcome`, `run_smelt_inference`, `expr_sql_by_alias`,
      and `check_types_against_oracle`.
- [ ] **Step 5: Cut over every importer.** Delete the five moved modules from
      `prop_helpers/mod.rs` and rewrite each import site to
      `use smelt_oracle_testkit::…`. **Clean cutover: no re-export shim in `prop_helpers`.**
      Find every site with `grep -rn 'prop_helpers::\(arrow_mapping\|duckdb_oracle\|spark_oracle\|bigquery_oracle\|type_comparison\)\|classify_oracle_error' crates/`.
- [ ] **Step 6: Run to verify pass.**
      `cargo test -p smelt-oracle-testkit --quiet 2>&1 | tail -20` and
      `cargo test -p smelt-db --quiet 2>&1 | tail -40`.
- [ ] **Step 7: Verify the hardening derivation.**
      `bash .claude/scripts/hardening-budget.sh` — must exit 0 with **no**
      `smelt-oracle-testkit` row added to `.claude/hardening-baseline.txt`. If it reports
      `unregistered crate/pattern smelt-oracle-testkit …`, the crate did not derive as
      test-support: check that no crate regular-depends on it and that it has no binary target.
      Then `cargo test -p smelt-core --test hardening_budget --quiet 2>&1 | tail -20`.
- [ ] **Step 8:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 9: Commit.** `refactor(test): extract smelt-oracle-testkit from smelt-db prop_helpers (#171)`

---

## Phase 7: Value-execution capability on all three oracles

The `TypeOracle` trait is schema-only; only `DuckDbOracle` executes, and it renders cells as
`format!("{val:?}")` (`"HugeInt(42)"`, `"Null"`), which is not comparable across engines. The
value leg needs a typed cell and a second, sibling trait.

**Files:**
- Create: `crates/smelt-oracle-testkit/src/value.rs`
- Modify: `crates/smelt-oracle-testkit/src/{duckdb_oracle,spark_oracle,bigquery_oracle}.rs`
- Modify: `python/smelt/bigquery_type_oracle.py`

**Interfaces produced:**

```rust
/// One result cell, normalised so two engines' renderings are comparable.
///
/// `DuckDbOracle::execute_query`'s `format!("{val:?}")` rendering is deliberately
/// not reused: `"HugeInt(42)"` and Spark's `"42"` are the same value, and a
/// string comparator would call them divergent.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Int(i128),
    Float(f64),
    Bool(bool),
    Text(String),
    /// Value is `unscaled / 10^scale`. Kept unnormalised so the comparator can
    /// decide whether a scale difference matters.
    Decimal { unscaled: i128, scale: u32 },
    /// ISO-8601 `YYYY-MM-DD`.
    Date(String),
    /// ISO-8601, normalised to UTC.
    Timestamp(String),
}

/// Executes a query and returns typed rows. Sibling to `TypeOracle`, not a
/// widening of it: DuckDB is the only engine that had both, and keeping them
/// separate lets a schema-only engine stay a schema-only engine.
pub trait ValueOracle {
    fn execute_rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueMatch {
    Equal,
    Divergent { detail: String },
}

/// Compare a target engine's cell against DuckDB's, under the typed rules the
/// research doc specifies: exact for integers, strings and booleans; relative
/// tolerance for floats; scale-normalised for decimals; NULL equals NULL.
pub fn compare_cells(reference: &Cell, actual: &Cell) -> ValueMatch;
```

Comparator rules, in order:
- `Null` vs `Null` → `Equal`; `Null` vs anything → `Divergent`.
- `Int` vs `Int`, `Bool` vs `Bool`, `Text` vs `Text`, `Date`/`Timestamp` → exact.
- `Decimal` vs `Decimal` → rescale both to `max(scale)` and compare unscaled.
- `Int(n)` vs `Decimal { scale: 0, unscaled }` → `n == unscaled`. Engines disagree on integer
  width and on whether `SUM` returns a decimal; that is a *type* divergence already registered
  in `divergences.rs`, not a value one.
- `Float` vs `Float` / `Int` / `Decimal` → convert both to `f64`; equal if both NaN, both the
  same infinity, or `|a-b| <= 1e-9 * max(1.0, |a|.max(|b|))`.
- Anything else → `Divergent { detail }` naming both renderings.

- [ ] **Step 1: Write the failing comparator tests** in `value.rs`'s `mod tests`:

```rust
#[test]
fn null_equals_null_and_nothing_else() {
    assert_eq!(compare_cells(&Cell::Null, &Cell::Null), ValueMatch::Equal);
    assert!(matches!(compare_cells(&Cell::Null, &Cell::Int(0)), ValueMatch::Divergent { .. }));
}

#[test]
fn decimals_compare_on_value_not_on_scale() {
    let a = Cell::Decimal { unscaled: 150, scale: 2 };   // 1.50
    let b = Cell::Decimal { unscaled: 1500, scale: 3 };  // 1.500
    assert_eq!(compare_cells(&a, &b), ValueMatch::Equal);
    let c = Cell::Decimal { unscaled: 151, scale: 2 };
    assert!(matches!(compare_cells(&a, &c), ValueMatch::Divergent { .. }));
}

#[test]
fn an_integer_matches_a_scale_zero_decimal() {
    // DuckDB returns SUM(INTEGER) as Decimal(38,0); Spark returns BIGINT. Same value.
    assert_eq!(
        compare_cells(&Cell::Int(42), &Cell::Decimal { unscaled: 42, scale: 0 }),
        ValueMatch::Equal
    );
}

#[test]
fn floats_compare_under_relative_tolerance() {
    assert_eq!(compare_cells(&Cell::Float(1.0), &Cell::Float(1.0 + 1e-12)), ValueMatch::Equal);
    assert!(matches!(
        compare_cells(&Cell::Float(1.0), &Cell::Float(1.001)),
        ValueMatch::Divergent { .. }
    ));
    assert_eq!(compare_cells(&Cell::Float(f64::NAN), &Cell::Float(f64::NAN)), ValueMatch::Equal);
}

#[test]
fn the_xor_case_is_caught() {
    // 2 ^ 3: power says 8, bitwise XOR says 1. Both are INT64 on BigQuery and
    // Spark, so the schema leg cannot see this. This is the whole point.
    assert!(matches!(
        compare_cells(&Cell::Int(8), &Cell::Int(1)),
        ValueMatch::Divergent { .. }
    ));
}
```

- [ ] **Step 2: Run to verify failure.**
      `cargo test -p smelt-oracle-testkit value 2>&1 | tail -20`.
- [ ] **Step 3: Implement `Cell`, `ValueOracle`, `compare_cells`.**
- [ ] **Step 4: Implement `ValueOracle for DuckDbOracle`** over `stmt.query_arrow([])`, mapping
      each Arrow array to `Cell` (reuse `arrow_mapping.rs`'s type map for the discriminant). Do
      **not** route through the existing `execute_query`'s debug strings. Test:

```rust
#[test]
fn the_duckdb_value_oracle_returns_typed_cells() {
    let oracle = DuckDbOracle::new();
    let rows = oracle
        .execute_rows("SELECT 2 ^ 3 AS p, CAST(NULL AS INTEGER) AS n, 1.50::DECIMAL(4,2) AS d")
        .expect("execute");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Cell::Null);
    assert_eq!(rows[0][2], Cell::Decimal { unscaled: 150, scale: 2 });
    // DuckDB's `^` is power.
    assert_eq!(compare_cells(&Cell::Float(8.0), &rows[0][0]), ValueMatch::Equal);
}
```

- [ ] **Step 5: Implement `ValueOracle for SparkOracle`.** Reuse the existing sentinel framing
      (`spark_oracle.rs:99-138`): send the query plus `SELECT '__SMELT_SENTINEL_…'`, collect
      tab-separated rows until the sentinel. Cell typing comes from a preceding
      `DESCRIBE QUERY` on the same SQL — parse each column's declared Spark type and decode the
      text accordingly. Spark renders NULL as the literal `NULL`; a genuine `'NULL'` string is
      indistinguishable, so the fixture's `s_text` column must not contain the four-character
      string `NULL` — record that as a fixture constraint in Phase 8 and assert it there.
- [ ] **Step 6: Add the `exec` verb to the BigQuery oracle.** In
      `python/smelt/bigquery_type_oracle.py`, accept `{"exec": sql}` alongside `{"sql": sql}`,
      call the already-present `BigQueryAdapter.execute_sql(sql)` (`bigquery_adapter.py:83`,
      returns a `pyarrow.Table`), and reply
      `{"rows": [[{"t": "int", "v": "42"}, {"t": "null"}], …]}`. Serialise every value as a
      string with an explicit type tag — JSON numbers would lose `INT64` and `NUMERIC` precision.
      Extend the module docstring's protocol description.
      Then implement `ValueOracle for BigQueryOracle` decoding that reply into `Cell`.
- [ ] **Step 7: Test each remote leg against a live engine.** These skip green when unset:

```bash
# Spark
bash scripts/spark-up.sh && source scripts/spark-env.sh
SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) \
  cargo test -p smelt-oracle-testkit --quiet 2>&1 | tail -20
bash scripts/spark-down.sh

# BigQuery — costs money; one run
source scripts/bigquery-env.sh
cargo test -p smelt-oracle-testkit --quiet 2>&1 | tail -20
```

      Each remote impl gets one live test asserting `SELECT 2 + 3` returns `Cell::Int(5)` and
      `SELECT CAST(NULL AS INT64)` returns `Cell::Null`, guarded by the
      `let Some(x) = … else { eprintln!("… skipping"); return; }` idiom.
- [ ] **Step 8:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 9: Commit.** `feat(test): typed value execution on all three oracles (#171)`

---

## Phase 8: Fixture, derived probes, coverage-totality gate

No warehouse. This phase produces the enumeration and proves it is total.

**Files:**
- Create: `crates/smelt-db/tests/dialect_audit/{main.rs,fixture.rs,probe.rs,overrides.rs}`
- Modify: `crates/smelt-db/Cargo.toml` if a `[[test]]` entry is needed (it is not —
  `tests/dialect_audit/main.rs` auto-discovers as the `dialect_audit` binary)

**The fixture.** One deterministic 8-row table as a `VALUES` CTE, with a typed column per
`TypeConstraint` family and NULL-bearing rows. No DDL, nothing materialised, and the same text
serves a BigQuery dry run and a real execution.

```rust
// fixture.rs
/// Columns, in fixture order. `TypeConstraint` selection in `probe.rs` maps onto
/// exactly these names.
pub const COLUMNS: &[(&str, &str)] = &[
    ("g",        "grouping key, 2 distinct values"),
    ("n_int",    "INTEGER, one NULL"),
    ("n_bigint", "BIGINT, one NULL"),
    ("n_double", "DOUBLE, one NULL, includes a negative"),
    ("n_dec",    "DECIMAL(10,2), one NULL"),
    ("s_text",   "VARCHAR, one NULL; never the literal string NULL (Spark rendering)"),
    ("b_bool",   "BOOLEAN, one NULL, both values present"),
    ("d_date",   "DATE, one NULL"),
    ("ts_ts",    "TIMESTAMP, one NULL"),
    ("arr_int",  "ARRAY<BIGINT>"),
    ("j_json",   "JSON-shaped VARCHAR"),
];

/// The fixture CTE for `dialect`. DuckDB, Spark and PostgreSQL take a `VALUES`
/// table-value constructor; GoogleSQL has none and takes
/// `UNNEST([STRUCT(...), ...])`.
pub fn fixture_cte(dialect: DialectId) -> String;
```

**Probe derivation.**

```rust
// probe.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// `SELECT <expr> AS a FROM fixture`
    Scalar,
    /// `SELECT g, <expr> AS a FROM fixture GROUP BY g`
    Aggregate,
    /// `SELECT <expr> OVER (PARTITION BY g ORDER BY n_bigint) AS a FROM fixture`
    Window,
}

#[derive(Debug, Clone)]
pub struct Probe {
    /// Canonical registry name.
    pub name: &'static str,
    pub position: Position,
    /// The expression in smelt SQL, before dialect lowering.
    pub expr: String,
    /// Deterministic alias: `p_<sanitised name>_<position>`.
    pub alias: String,
}

/// Every probe the registry implies. Aggregates yield two — `MEDIAN` proves the
/// lowering differs per position.
pub fn derive_probes() -> Vec<Probe>;

/// Why an entry yields no probe, for the totality gate's failure message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotProbed {
    /// The signature's params/form give no type-correct spelling and no override exists.
    Underivable { detail: String },
    /// Deliberately skipped, with a recorded reason.
    Skipped { reason: &'static str },
}

pub fn probe_or_reason(sig: &Signature) -> Result<Vec<Probe>, NotProbed>;

/// Parse one smelt-SQL statement, print it for `dialect`, and prefix the
/// dialect's fixture CTE. The single place a probe becomes engine SQL — both
/// legs and every hand-written regression test go through it, so no test can
/// accidentally compare a differently-printed query.
pub fn print_for(dialect: DialectId, smelt_sql: &str) -> String;
```

Derivation rules — argument column by `TypeConstraint`:
`Concrete(Text)`→`s_text`, `Integer`→`n_int`, `BigInt`→`n_bigint`, `Double`→`n_double`,
`Decimal{..}`→`n_dec`, `Boolean`→`b_bool`, `Date`→`d_date`, `Timestamp{..}`→`ts_ts`,
`Array(_)`→`arr_int`; `Numeric`→`n_double`, `Ordered`→`n_bigint`, `Any`→`n_bigint`.
`SigParam::Var(t)` resolves through `sig.type_param(t).constraint`; `SigParam::Variadic(inner)`
expands to two copies of `inner`'s column.

Spelling by `SyntaxForm`: `Call` → `NAME(a, b)`; `Infix` → `a NAME b`; `Postfix` → `a NAME`
with `IS_NULL`→`IS NULL` and `IS_NOT_NULL`→`IS NOT NULL` supplied by `overrides.rs`;
`TableFn` → `FROM <fixture>, UNNEST(arr_int) AS u`; `Special` → **must** have an override, or
the totality gate fails.

Position by `sig.kind`: `Scalar`→`[Scalar]`, `Agg`→`[Aggregate, Window]`, `Window`→`[Window]`.

**The override table — the only hand-written per-function data in the design.**

```rust
// overrides.rs
/// The minority of entries where a *type-correct* argument is not a *meaningful*
/// one — regex patterns, date-part strings, JSON paths, format strings — plus
/// the dedicated-syntax forms that have no uniform spelling.
///
/// Replaces `core_functions()` (`prop_helpers/generators.rs:336-1000`, 85
/// hand-maintained registry-blind `FuncDesc` rows) as the source of probe shapes.
#[derive(Debug, Clone)]
pub struct Override {
    pub name: &'static str,
    /// Argument expressions, replacing the derived ones entirely.
    pub args: Option<&'static [&'static str]>,
    /// Full spelling template; `{0}`, `{1}`, … are the arguments.
    /// Required for every `SyntaxForm::Special` entry.
    pub spelling: Option<&'static str>,
    /// Probe the schema leg only, never the value leg, with a reason.
    /// Nondeterministic entries execute at different instants or produce no
    /// stable value: `RANDOM`, `NOW`, `CURRENT_DATE`, `CURRENT_TIMESTAMP`, `UUID`.
    pub schema_only: Option<&'static str>,
}

pub fn overrides() -> &'static [Override];
```

Minimum content (extend as the totality gate demands):
`CAST` (`spelling: "CAST({0} AS BIGINT)"`), `BETWEEN` (`"{0} BETWEEN 1 AND 10"`),
`IN` (`"{0} IN (1, 2, 3)"`), `EXISTS` (`"EXISTS (SELECT 1)"`), `IS_NULL` (`"{0} IS NULL"`),
`IS_NOT_NULL`, `LIKE`/`ILIKE`/`GLOB` (pattern literals), `DATE_TRUNC`/`DATE_PART` (part strings),
`DATE_ADD`/`DATE_SUB` (interval spelling), `LPAD`/`RPAD` (width + pad),
`PERCENTILE_CONT`/`PERCENTILE_DISC` (`0.5`), `NTILE` (`4`), `TO_CHAR`/`STRFTIME` (format),
the JSON extraction family (paths), the regex family (patterns), and
`schema_only` on `RANDOM`, `NOW`, `CURRENT_DATE`, `CURRENT_TIMESTAMP`, `UUID`.

- [ ] **Step 1: Write the failing totality gate.** `main.rs`:

```rust
mod fixture;
mod overrides;
mod probe;

use smelt_types::{BuiltinRegistry, DialectId};

#[test]
fn every_registry_entry_yields_a_probe_or_a_recorded_reason() {
    let mut underivable = Vec::new();
    for name in BuiltinRegistry::names() {
        let sig = BuiltinRegistry::resolve(name).expect("names() resolves");
        match probe::probe_or_reason(sig) {
            Ok(probes) => assert!(!probes.is_empty(), "{name} yielded an empty probe set"),
            Err(probe::NotProbed::Skipped { .. }) => {}
            Err(probe::NotProbed::Underivable { detail }) => {
                underivable.push(format!("  {name}: {detail}"));
            }
        }
    }
    underivable.sort();
    assert!(
        underivable.is_empty(),
        "{} registry entries have no derivable probe and no override. Add a row \
         to `overrides.rs` — with `schema_only` and a reason if the entry is \
         nondeterministic — rather than narrowing the enumeration:\n{}",
        underivable.len(),
        underivable.join("\n")
    );
}

#[test]
fn aggregates_are_probed_in_both_positions() {
    // MEDIAN proves the lowering differs per position; probing one position
    // would have missed the BigQuery aggregate form entirely.
    let probes = probe::derive_probes();
    let median: Vec<_> = probes.iter().filter(|p| p.name == "MEDIAN").collect();
    assert_eq!(median.len(), 2);
    assert!(median.iter().any(|p| p.position == probe::Position::Aggregate));
    assert!(median.iter().any(|p| p.position == probe::Position::Window));
}

#[test]
fn every_special_form_entry_has_a_spelling_override() {
    for name in BuiltinRegistry::names() {
        let sig = BuiltinRegistry::resolve(name).expect("resolves");
        if sig.syntax_form != smelt_types::SyntaxForm::Special {
            continue;
        }
        assert!(
            overrides::overrides().iter().any(|o| o.name == name && o.spelling.is_some()),
            "{name} is SyntaxForm::Special and has no spelling override; a Special \
             entry has no uniform shape the harness can derive"
        );
    }
}

#[test]
fn probe_aliases_are_unique() {
    // Probes are batched into one SELECT per (dialect, shape); a duplicate alias
    // would silently drop a probe from the batch.
    let probes = probe::derive_probes();
    let mut aliases: Vec<&str> = probes.iter().map(|p| p.alias.as_str()).collect();
    let total = aliases.len();
    aliases.sort_unstable();
    aliases.dedup();
    assert_eq!(aliases.len(), total, "duplicate probe alias");
}

#[test]
fn the_fixture_has_a_column_for_every_type_constraint_family() {
    for d in DialectId::ALL {
        let cte = fixture::fixture_cte(*d);
        for (col, _) in fixture::COLUMNS {
            assert!(cte.contains(col), "{} fixture lacks {col}", d.slug());
        }
        assert!(
            !cte.contains("'NULL'"),
            "{} fixture contains the literal string NULL, which Spark's text \
             rendering cannot distinguish from a real NULL", d.slug()
        );
    }
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p smelt-db --test dialect_audit 2>&1 | tail -30`.
- [ ] **Step 3: Implement `fixture.rs`.** Assert the DuckDB fixture actually executes:

```rust
#[test]
fn the_duckdb_fixture_executes_and_yields_eight_rows() {
    let oracle = DuckDbOracle::new();
    let sql = format!("{} SELECT * FROM fixture", fixture::fixture_cte(DialectId::DuckDb));
    let rows = oracle.execute_rows(&sql).expect("fixture must execute");
    assert_eq!(rows.len(), 8);
}
```

- [ ] **Step 4: Implement `probe.rs` and `overrides.rs`,** iterating until the totality gate is
      green. Each iteration either derives a shape or adds an override row.
- [ ] **Step 5: Run to verify pass.**
      `cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -20`.
- [ ] **Step 6: Retire `core_functions()` as a probe source.** `core_functions()`
      (`generators.rs:336-1000`) has exactly two consumers: `generators.rs:1245`
      (`generate_expr`) and the three reachability tests in `type_property_tests.rs:1433,1505,1576`.
      Those are the *type* property sweep, a different suite with a different purpose — **leave
      them alone**. Record in `overrides.rs`'s module doc that this table supersedes
      `core_functions()` *as the source of probe shapes for the dialect audit*, and that
      unifying the two generators is deliberately out of scope.
- [ ] **Step 7:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 8: Commit.** `feat(test): registry-derived dialect probes and the totality gate (#171)`

---

## Phase 9: Schema + value legs on DuckDB; ledger + ratchet

**Files:**
- Create: `crates/smelt-db/tests/dialect_audit/ledger.rs`
- Create: `.claude/dialect-gaps-baseline.txt`
- Modify: `.gitignore` — `!.claude/dialect-gaps-baseline.txt`
- Modify: `crates/smelt-db/tests/dialect_audit/main.rs` — the two legs

**Interfaces produced:**

```rust
// ledger.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Accepted and permanent — a semantic difference users must know about
    /// (Spark's integer-division semantics, and the like). Does not fail.
    Divergent { reason: &'static str },
    /// A lowering we owe, with a tracking issue. Does not fail, but the count
    /// ratchets down only.
    Gap { issue: &'static str },
    /// Nondeterministic: engines execute at different instants (`NOW`,
    /// `CURRENT_DATE`) or produce no stable value (`RANDOM`, `UUID`). The value
    /// leg is skipped and the reason recorded.
    SchemaOnly { reason: &'static str },
}

#[derive(Debug, Clone, Copy)]
pub struct LedgerRow {
    pub name: &'static str,
    pub dialect: DialectId,
    pub verdict: Verdict,
}

/// Every accepted `(entry, dialect)` divergence. A pair absent from this table
/// must pass both legs.
pub fn dialect_divergences() -> &'static [LedgerRow];

pub fn find(name: &str, dialect: DialectId) -> Option<&'static LedgerRow>;
```

**Baseline** `.claude/dialect-gaps-baseline.txt`, mirroring `.claude/parser-gaps-baseline.txt`'s
`<metric> <count>` shape (chosen over the bash-script shape: no per-row `--update` is needed,
because the ledger is static Rust data, so the count is knowable without a warehouse and the
ratchet test needs no engine):

```
# Dialect-emission gap ratchet baseline
# Updated: 2026-08-23
#
# Format: <metric> <count>
#
# Each metric is the number of `Verdict::Gap` rows in
# `crates/smelt-db/tests/dialect_audit/ledger.rs` for that dialect — a lowering
# smelt owes, with a tracking issue.
#
# This is a RATCHET, mirroring `.claude/parser-gaps-baseline.txt`:
#   - The count may only go DOWN as lowerings land.
#   - Raising it requires editing this file with a reviewer-visible
#     justification and a new ledger entry — never a silent skip.
# `gap_count_ratchet` in tests/dialect_audit/main.rs enforces an exact match
# (a decrease is a "stale baseline" failure prompting you to tighten here).
dialect_gaps_duckdb 0
dialect_gaps_spark 0
dialect_gaps_postgres 0
dialect_gaps_bigquery 0
```

Start every metric at 0 and let the first sweeps raise them with a reviewer-visible edit, rather
than pre-declaring gaps that may not exist.

- [ ] **Step 1: Write the failing ratchet + two-sided ledger tests** in `main.rs`:

```rust
fn baseline(metric: &str) -> usize {
    include_str!("../../../../.claude/dialect-gaps-baseline.txt")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .find_map(|l| {
            let (k, v) = l.trim().split_once(' ')?;
            if k != metric {
                return None;
            }
            v.trim().parse::<usize>().ok()
        })
        .unwrap_or_else(|| panic!("`{metric}` not found in .claude/dialect-gaps-baseline.txt"))
}

#[test]
fn gap_count_ratchet() {
    // Static ledger data, so this needs no warehouse and runs per-PR.
    for d in DialectId::ALL {
        let metric = format!("dialect_gaps_{}", d.slug());
        let current = ledger::dialect_divergences()
            .iter()
            .filter(|r| r.dialect == *d && matches!(r.verdict, ledger::Verdict::Gap { .. }))
            .count();
        let base = baseline(&metric);
        assert!(
            current <= base,
            "Registered dialect-gap count REGRESSED for {}: current={current} > baseline={base}.\n\
             A new gap must be justified by editing .claude/dialect-gaps-baseline.txt \
             (reviewer-visible), never absorbed silently.", d.slug()
        );
        assert!(
            current >= base,
            "STALE baseline for {}: current={current} < baseline={base}.\n\
             A lowering closed a gap — tighten .claude/dialect-gaps-baseline.txt to {current}.",
            d.slug()
        );
    }
}

#[test]
fn every_ledger_row_names_a_real_registry_entry_and_a_probed_pair() {
    // The unreachable-row direction, scoped to what is knowable statically: a
    // row naming an entry the registry no longer has, or a pair the harness
    // never probes, is an error telling you to delete it — the same shape as the
    // ORPHANED BASELINE ENTRY sweep in .claude/scripts/hardening-budget.sh.
    let probed: std::collections::HashSet<&str> =
        probe::derive_probes().iter().map(|p| p.name).collect();
    let mut orphans = Vec::new();
    for row in ledger::dialect_divergences() {
        if BuiltinRegistry::resolve(row.name).is_none() {
            orphans.push(format!("  {} ({}): no such registry entry", row.name, row.dialect.slug()));
        } else if !probed.contains(row.name) {
            orphans.push(format!(
                "  {} ({}): entry is never probed, so this row can never fire",
                row.name, row.dialect.slug()
            ));
        }
    }
    assert!(
        orphans.is_empty(),
        "ORPHANED LEDGER ROWS — registered but unreachable. Delete them:\n{}",
        orphans.join("\n")
    );
}

#[test]
fn a_pair_has_at_most_one_ledger_row() {
    let mut seen = std::collections::HashSet::new();
    for row in ledger::dialect_divergences() {
        assert!(
            seen.insert((row.name, row.dialect)),
            "duplicate ledger row for {} on {}", row.name, row.dialect.slug()
        );
    }
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p smelt-db --test dialect_audit 2>&1 | tail -30`.
- [ ] **Step 3: Implement `ledger.rs`** with an empty `dialect_divergences()` and create the
      baseline file plus the `.gitignore` line. Confirm the file is committable:
      `git check-ignore -v .claude/dialect-gaps-baseline.txt` must report **no** match.
- [ ] **Step 4: Write the failing schema leg** in `main.rs`:

```rust
/// Print each probe for the dialect, ask the oracle for the output schema, and
/// compare against smelt's inference. Acceptance alone is most of the value: it
/// catches every missing lowering and every declared `Unsupported`.
fn run_schema_leg(dialect: DialectId, oracle: &dyn TypeOracle) -> LegOutcome;

#[test]
fn schema_leg_duckdb() {
    let oracle = DuckDbOracle::new();
    let outcome = run_schema_leg(DialectId::DuckDb, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    // Anti-silent-skip guard, mirroring BIGQUERY_COLUMN_COVERAGE_FLOOR
    // (type_property_tests.rs:76): a leg that "ran" but compared nothing must
    // not be green.
    assert!(
        outcome.probes_compared >= 100,
        "schema leg compared only {} probes — the enumeration collapsed",
        outcome.probes_compared
    );
    eprintln!("COVERAGE[duckdb schema] probes_compared={}", outcome.probes_compared);
}
```

      `LegOutcome` carries `probes_compared: usize`, `refused: Vec<String>`, and
      `failures: Vec<String>`. Every engine error routes through
      `classify_oracle_error`: `QueryRefusal` → `refused`, `Fatal` → the leg fails outright.
      Batch by `(dialect, Position)` into one `SELECT` with one aliased column per probe; on any
      batch failure, re-run one probe per query so the error names the function, not the batch.
- [ ] **Step 5: Write the failing value leg.**

```rust
/// Execute each probe on the target and on DuckDB and compare row-wise under
/// `compare_cells`. DuckDB is the reference, matching the repo's oracle
/// convention. This is the leg that catches `^`.
fn run_value_leg(dialect: DialectId, target: &dyn ValueOracle, reference: &DuckDbOracle) -> LegOutcome;

#[test]
fn value_leg_duckdb_is_self_consistent() {
    // DuckDB against itself: proves the harness, the fixture and the comparator
    // agree before any cross-engine claim is made.
    let oracle = DuckDbOracle::new();
    let outcome = run_value_leg(DialectId::DuckDb, &oracle, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(outcome.probes_compared >= 100);
}
```

      Every probe query ends `ORDER BY g` for determinism. A probe whose entry has
      `Override::schema_only` is skipped here with its reason counted, not silently dropped.
- [ ] **Step 6: Implement both legs.** Iterate until green, adding `Gap`/`Divergent` ledger rows
      (with the baseline raised in the same commit, reviewer-visible) for anything DuckDB
      genuinely diverges on.
- [ ] **Step 7: Measure the DuckDB leg's wall time — the research doc's first open question.**

```bash
time cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -20
```

      **Decision rule, applied now rather than guessed:** if both DuckDB legs together exceed
      **20 s**, move the value leg behind `SMELT_DIALECT_VALUE_LEG=1` and add it to the nightly
      `compat.yml` schedule; the schema leg stays per-PR unconditionally. Record the measured
      number and the decision in the commit message body.
- [ ] **Step 8:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 9: Commit.** `feat(test): DuckDB schema and value legs with a two-sided ledger (#171)`

---

## Phase 10: Spark leg + the `^` proof; CI wiring

**Files:**
- Modify: `crates/smelt-db/tests/dialect_audit/main.rs` — Spark leg
- Modify: `.github/workflows/compat.yml` — `changes` filter + a `dialect-audit-spark` job
- Modify: `.github/workflows/test.yml` — explicit per-PR steps

`test.yml`'s `test` job runs `cargo test --lib` plus a named list; a new integration test
does **not** run per-PR unless named. Add these three steps:

```yaml
      - name: Dialect emission ownership gate
        run: cargo test -p smelt-dialect --test emission_ownership --quiet 2>&1 | tail -40
      - name: Dialect audit (DuckDB legs, totality, ledger)
        run: cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -40
      - name: Dialect seam (all backends, offline)
        run: cargo test -p smelt-runtime --test dialect_seam --quiet 2>&1 | tail -40
```

`compat.yml`'s `changes` paths-filter already lists `crates/smelt-types/src/signatures.rs` but
**omits `crates/smelt-dialect/**`** — actively wrong once emission lives in the registry. Add:

```yaml
              - 'crates/smelt-dialect/**'
              - 'crates/smelt-oracle-testkit/**'
              - 'crates/smelt-db/tests/dialect_audit/**'
```

- [ ] **Step 1: Write the failing Spark tests.**

```rust
static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID").ok().map(|id| SparkOracle::new(&id))
});

#[test]
fn schema_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping schema_leg_spark");
        return;
    };
    let outcome = run_schema_leg(DialectId::SparkSql, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!("COVERAGE[spark schema] probes_compared={}", outcome.probes_compared);
}

#[test]
fn value_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping value_leg_spark");
        return;
    };
    let outcome = run_value_leg(DialectId::SparkSql, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
}

#[test]
fn spark_caret_agrees_with_duckdb_power() {
    // The regression test for the finding that motivated this work: Spark's
    // infix `^` is bitwise XOR. Before the Phase 3 emission row, `SELECT 2 ^ 3`
    // returned 1 on Spark and 8 on DuckDB — a silently wrong number, not an error.
    let Some(spark) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping spark_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY n_bigint";
    let spark_rows = spark.execute_rows(&print_for(DialectId::SparkSql, smelt_expr)).expect("spark");
    let duck_rows = duckdb.execute_rows(&print_for(DialectId::DuckDb, smelt_expr)).expect("duckdb");
    assert_eq!(spark_rows.len(), duck_rows.len());
    for (s, d) in spark_rows.iter().zip(&duck_rows) {
        assert_eq!(compare_cells(&d[0], &s[0]), ValueMatch::Equal, "^ diverges on Spark");
    }
}
```

- [ ] **Step 2: Bring Spark up and run.**

```bash
bash scripts/spark-up.sh && source scripts/spark-env.sh
SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) \
  cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -60
```

      `spark_caret_agrees_with_duckdb_power` should pass with the Phase 3 fix in place. **Verify
      it would have caught the bug**: temporarily revert the `^` Spark emission row to `Native`,
      re-run, confirm the test fails naming `^`, then restore. Record that in the commit body.
- [ ] **Step 3: Register real Spark divergences.** Everything the sweep reports that is a genuine
      permanent semantic difference (Spark's integer-division semantics, decimal arithmetic
      model) becomes a `Verdict::Divergent` row with a `// verified: 2026-08-23 <probe SQL> —
      <engine output>` provenance comment, matching `divergences.rs`'s convention. Everything
      that is a lowering smelt owes becomes `Verdict::Gap { issue: "#NNN" }` with the issue filed
      and `.claude/dialect-gaps-baseline.txt` raised in the same commit.
- [ ] **Step 4: Tear down.** `bash scripts/spark-down.sh`
- [ ] **Step 5: Wire CI.** Add the three `test.yml` steps and the three `compat.yml` path
      patterns. Add a `dialect-audit-spark` job to `compat.yml` by copying `type-property-spark`
      verbatim and changing only the cache key and the final `cargo test` line to
      `cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -60`. It must carry the
      same `if: ${{ !cancelled() && (github.event_name == 'schedule' || contains(github.event.pull_request.labels.*.name, 'run-docker-tests') || needs.changes.outputs.spark == 'true') }}`
      gate and the `SPARK_CONTAINER_ID` export that `type-property-spark` uses.
- [ ] **Step 6: Validate the workflow syntax.**
      `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/compat.yml'))"`
- [ ] **Step 7:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 8: Commit.** `feat(test): Spark dialect-audit leg; fixes silently-wrong ^ on Spark (#171)`

---

## Phase 11: BigQuery leg + manual sweep script

BigQuery stays manual, per `multi_backend.md` §"BigQuery has no CI tier, by decision, not by
omission". That decision is not revisited here; its rationale applies *more* strongly to this
suite, because the value leg executes rather than dry-runs.

**Files:**
- Modify: `crates/smelt-db/tests/dialect_audit/main.rs` — BigQuery leg
- Create: `scripts/bigquery-dialect-audit.sh`

- [ ] **Step 1: Write the failing tests**, mirroring the Spark pair with
      `static BIGQUERY: LazyLock<Option<BigQueryOracle>> = LazyLock::new(BigQueryOracle::from_env);`
      and a `bigquery_caret_agrees_with_duckdb_power` regression test.
- [ ] **Step 2: Write the sweep script.** `scripts/bigquery-dialect-audit.sh`, copying
      `scripts/bigquery-conformance.sh`'s **fail-loud** gating verbatim in shape — this script
      must refuse to start without credentials rather than skip green, because a sweep that
      starts unauthenticated burns quota and proves nothing:

```bash
#!/usr/bin/env bash
# Cross-engine dialect audit against a live BigQuery.
#
# Unlike `bigquery-test.sh`, this does NOT let an absent credential fall through
# to a green skip: every #[test] in the BigQuery leg skips green when
# SMELT_BQ_PROJECT is absent, so a run without it would report success while
# covering nothing. This is also the point at which the sweep costs money — the
# value leg executes rather than dry-runs.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh

if [ -z "${SMELT_BQ_ACCESS_TOKEN:-}" ]; then
  echo "bigquery-dialect-audit.sh: no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
fi
if [ -z "${SMELT_BQ_PROJECT:-}" ]; then
  echo "bigquery-dialect-audit.sh: SMELT_BQ_PROJECT is unset — the leg would skip green and verify nothing. Run: bash scripts/bigquery-key.sh <project-id>, then source scripts/bigquery-env.sh" >&2
  exit 1
fi

cargo test -p smelt-db --test dialect_audit --quiet -- --nocapture 2>&1 | tail -80
```

- [ ] **Step 3: Test the script's gating without spending money.** Add
      `crates/smelt-db/tests/dialect_audit_script.rs`, copying
      `crates/smelt-cli/tests/bigquery_conformance_script.rs:36-97`: spawn the script with
      `.env_remove("SMELT_BQ_PROJECT")` and assert a non-zero exit whose stderr names the
      variable.
- [ ] **Step 4: Run the live sweep once.** `bash scripts/bigquery-dialect-audit.sh`
      Note the token expires after one hour; re-run `bash scripts/bigquery-auth.sh` if the sweep
      outlives it. Record `COVERAGE[bigquery …]` numbers and wall time in the commit body.
- [ ] **Step 5: Register real BigQuery divergences and gaps**, exactly as Phase 10 Step 3.
- [ ] **Step 6:** `bash .claude/scripts/verify-phase.sh` (the BigQuery leg skips green locally).
- [ ] **Step 7: Commit.** `feat(test): BigQuery dialect-audit leg and manual sweep script (#171)`

---

## Phase 12: Seam leg, generated coverage table, `CLAUDE.md`

**Files:**
- Modify: `crates/smelt-runtime/tests/dialect_seam.rs` — four more models
- Create: `crates/smelt-db/tests/dialect_audit/report.rs`
- Create: `docs/reference/dialect-coverage.md` (generated)
- Modify: `CLAUDE.md` — the invariant line and the new standing gates

**The seam leg.** The enumerating legs test the printer; this leg guards the printer →
cast-wrap → projection seam, where the `MEDIAN` re-parse bug actually lived. Five models, one
per shape, run through the real `execute_project` pipeline per backend — five, not 144, so it
does not scale with the registry.

| Model | Shape | SQL |
|---|---|---|
| `seam_scalar` | scalar | `SELECT id, UPPER(name) AS u FROM events` |
| `seam_aggregate` | aggregate | `SELECT id, MEDIAN(val) AS med FROM events GROUP BY id` |
| `seam_window` | window | `SELECT id, MEDIAN(val) OVER (PARTITION BY id) AS med FROM events` |
| `seam_operator` | operator | `SELECT id, val % 3 AS r, val ** 2 AS s FROM events` |
| `seam_tablefn` | table function | `SELECT u FROM events, UNNEST(tags) AS u` |

**The report.** Derived from registry + ledger only — deterministic, no warehouse, gateable
per-PR. One row per entry, one column per dialect:

```markdown
| Entry | Form | DuckDB | Spark SQL | PostgreSQL | BigQuery |
|---|---|---|---|---|---|
| `^` | infix | native | rewrite:PowerCall | native | rewrite:PowerCall |
| `BOOL_OR` | call | native | rename:SOME | native | rename:LOGICAL_OR |
| `//` | infix | native | unsupported | unsupported | unsupported |
```

Cell vocabulary: `native`, `rename:X`, `rewrite:Id`, `unsupported`, `divergent`, `gap #N`,
`schema-only`. A trailing "Verification tiers" section states which dialects have a live leg;
PostgreSQL's verdicts are marked *unverified* — it is a `SqlDialect` variant with no backend
crate and no oracle, so nothing exercises them.

- [ ] **Step 1: Write the failing seam tests.** For each of the five models, compile through
      `registry().get(<backend>)` for all three backends and assert `output_columns` is
      byte-identical across them — the same assertion shape as
      `crates/smelt-runtime/tests/projection_dialect_invariance.rs:139`, which is the precedent
      to read first. Then a live-DuckDB `execute_project` run per model asserting the rows land.
- [ ] **Step 2: Run to verify failure**, then implement, then
      `cargo test -p smelt-runtime --test dialect_seam --quiet 2>&1 | tail -20`.
- [ ] **Step 3: Write the failing doc-sync gate**, following
      `crates/smelt-logical/tests/backbuild_docs.rs:664-719` — the house convention is
      `SMELT_REGEN_DOCS=1`, and there is no `UPDATE_EXPECT` anywhere in the repo:

```rust
const COVERAGE_DOC: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/dialect-coverage.md");

#[test]
fn the_coverage_table_matches_the_registry() {
    let rendered = report::render();
    let on_disk = std::fs::read_to_string(COVERAGE_DOC).unwrap_or_default();
    if std::env::var("SMELT_REGEN_DOCS").as_deref() == Ok("1") {
        if on_disk != rendered {
            std::fs::write(COVERAGE_DOC, &rendered).expect("write coverage doc");
        }
        return;
    }
    assert_eq!(
        on_disk, rendered,
        "docs/reference/dialect-coverage.md is stale. Regenerate with:\n  \
         SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \
         the_coverage_table_matches_the_registry"
    );
}

#[test]
fn every_entry_and_dialect_appears_in_the_table() {
    // Totality on the published side: the deliverable is the table, and a gate
    // that only checks freshness would let an entry vanish from it silently.
    let rendered = report::render();
    for name in BuiltinRegistry::names() {
        assert!(rendered.contains(&format!("| `{name}` |")), "{name} missing from the table");
    }
}
```

- [ ] **Step 4: Implement `report.rs`.** Sort by entry name — `BuiltinRegistry::names()` is
      `HashMap` order and is not deterministic. Generate the doc:
      `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit --quiet`
- [ ] **Step 5: Add a doc header** to the generated file marking it generated and naming the
      regeneration command, above the generated region.
- [ ] **Step 6: Update `CLAUDE.md`.** In the "Function-registry single ownership" bullet
      (`:41-44`), extend "name, classification, and registry-driven type" to include
      "**and per-dialect emission**", and add two gate sub-bullets:
      - `cargo test -p smelt-dialect --test emission_ownership` — no name-matched dialect arm
        remains in `printer.rs`.
      - `cargo test -p smelt-db --test dialect_audit` — coverage totality, the two-sided
        emission ledger, the `Gap` ratchet (`.claude/dialect-gaps-baseline.txt`), and the
        `docs/reference/dialect-coverage.md` doc-sync gate.
- [ ] **Step 7: Update `docs/ROADMAP.md`** with the completed phase and the residual gaps the
      sweeps found.
- [ ] **Step 8:** `bash .claude/scripts/verify-phase.sh`
- [ ] **Step 9: Commit.** `feat(docs): generated dialect-coverage table with a drift gate (#171)`

---

## Verification

```bash
bash .claude/scripts/verify-phase.sh

# The new standing gates, named individually
cargo test -p smelt-dialect --test emission_ownership --quiet 2>&1 | tail -20
cargo test -p smelt-db      --test dialect_audit      --quiet 2>&1 | tail -20
cargo test -p smelt-runtime --test dialect_seam       --quiet 2>&1 | tail -20
bash .claude/scripts/hardening-budget.sh

# Live legs
bash scripts/spark-up.sh && source scripts/spark-env.sh
SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) \
  cargo test -p smelt-db --test dialect_audit --quiet 2>&1 | tail -60
bash scripts/spark-down.sh

bash scripts/bigquery-dialect-audit.sh
```

**Acceptance:**
1. `remap_function_name` no longer exists; `emission_ownership` is green.
2. `^` and `**` lower to `POWER(...)` on Spark, proven against a live Spark.
3. `//` on Spark / PostgreSQL / BigQuery is a compile-time `UnsupportedOnBackend` diagnostic.
4. Every `(registry entry, dialect)` pair has a probe or a recorded reason, and a verdict.
5. `docs/reference/dialect-coverage.md` exists, is generated, and is drift-gated.
6. `crates/smelt-oracle-testkit` has no row in `.claude/hardening-baseline.txt` and the gate is
   green.

---

## Deviations from the research doc, with reasons

1. **`SyntaxForm` has no `Prefix`; it has `TableFn` instead.** No registry entry is prefix — `NOT`
   and unary `-` are parser keywords, not registry candidates — and an unreachable variant is
   dead weight the totality gate cannot exercise. `TableFn` earns its place: `EXPLODE`/`UNNEST`
   are probed as `FROM …, UNNEST(arr) AS u`, not as a scalar call, so the probe deriver must
   distinguish them.
2. **`RewriteId::PowerCall`, not `BigQueryPower`.** Spark needs the same rewrite; a
   dialect-prefixed name would have to be renamed the moment the second dialect claimed it.
   Same for `ModuloCall`.
3. **`smelt-oracle-testkit` takes the oracle transport and `compare_types`, not
   `oracle_check.rs`.** `check_types_against_oracle` depends on `smelt-db` inference and on
   `generators.rs`; moving it would drag 5,800 lines of smelt-db-specific test apparatus into a
   shared crate. The error classifier splits out; the driver stays.
4. **`//` gains an `Unsupported` verdict.** The research doc did not name it. It is the cleanest
   demonstration of `Unsupported`'s new behaviour and turns a runtime syntax error into a
   compile-time diagnostic, which is what fail-loud discipline asks for.
5. **The coverage table is derived from registry + ledger only.** Making a published doc depend
   on which legs happened to run would make it nondeterministic and ungateable per-PR. The legs
   test the claims the table makes.
6. **The `Gap` ratchet is per-dialect, four metrics, not one count.** A single number would let a
   BigQuery gap close and a Spark gap open with no net movement.

---

## Execution workflow

**Skill:** `subagent-driven-development` — fresh implementer subagent per phase, persistent
ledger for recovery, artifacts passed as file paths (never inline context).

### Why SDD fits this plan

- Phases are sequential (each introduces types the next consumes), so parallelism is limited to
  Phases 10+11 (Spark and BigQuery legs). SDD's one-at-a-time dispatch with review gates matches.
- The plan is 2200 lines / 13 phases / ~50 steps. Holding it in one session's context is
  infeasible without compaction risk. SDD's ledger + task-brief extraction keeps the orchestrator
  thin (~50 lines of coordination per phase).
- Each phase has a hard verification gate (`bash .claude/scripts/verify-phase.sh`) that the
  orchestrator confirms before marking complete.

### Model selection (AWS Bedrock)

| Role | Model | Cost (in/out per 1M tokens) | Rationale |
|------|-------|----------------------------|-----------|
| Orchestrator | Claude Sonnet 4 | $3.00 / $15.00 | Coordination, judgment, rulings, review dispatch |
| Implementer (mechanical) | DeepSeek V3.2 | $0.62 / $1.85 | Clear specs, 1–2 file phases, TDD steps |
| Implementer (integration) | Claude Sonnet 4 | $3.00 / $15.00 | Multi-file coordination (Phases 4, 6, 12) |
| Fix-loop escalation (R4+) | Claude Opus 4 | $15.00 / $75.00 | Only when cheaper model is stuck |
| Task reviewer | DeepSeek V3.2 | $0.62 / $1.85 | Scoped diff review against brief |
| Final whole-branch review | Claude Opus 4 | $15.00 / $75.00 | Broad architectural judgment |
| Scout (read-only research) | Amazon Nova Micro | $0.035 / $0.14 | Codebase grep, caller enumeration |

**Estimated total cost:** $15–40 (dominated by orchestrator + integration phases).

### Phase-to-model mapping

| Phase | Implementer tier | Notes |
|-------|-----------------|-------|
| 0 | DeepSeek (mechanical) | Spec-only text edits |
| 1 | DeepSeek (mechanical) | Small new file + field migration |
| 2 | DeepSeek (mechanical) | Enum + registry rows |
| 3 | DeepSeek (mechanical) | Emission table authoring |
| 4 | Sonnet (integration) | Printer refactor across multiple arms |
| 5 | DeepSeek (mechanical) | New diagnostic, clear pattern |
| 6 | Sonnet (integration) | Crate extraction, import rewiring |
| 7 | DeepSeek (mechanical) | Oracle trait impl, clear interface |
| 8 | DeepSeek (mechanical) | Fixture + probe construction |
| 9 | DeepSeek (mechanical) | Ledger logic, ratchet file |
| 10 | DeepSeek (mechanical) | Spark leg (parallel with 11) |
| 11 | DeepSeek (mechanical) | BigQuery leg (parallel with 10) |
| 12 | Sonnet (integration) | Seam test + generated docs + CLAUDE.md |

### Context recovery

If the orchestrator session compacts or is restarted:
1. Read `<repo>/.superpowers/sdd/20260823-registry-dialect-emission/progress.md`
2. Each `Task <N>: complete (commits <base>..<head>)` line is done — skip it
3. Resume at first incomplete task
4. `git log --oneline` confirms commit history matches ledger

### Parallelism window

Phases 10 and 11 are the only pair safe to dispatch concurrently (independent engine legs,
disjoint files). All other phases are strictly sequential due to shared type definitions.
