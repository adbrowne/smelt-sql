# Registry-driven dialect emission and the cross-engine function audit

Date: 2026-08-23
Status: design, approved in brainstorming; implementation plan not yet written
Issue: [#171](https://github.com/adbrowne/smelt-sql/issues/171)

## Problem

Backend dialect coverage is discovered incident-by-incident: probe a live warehouse,
hit a failure, add a lowering. Nothing enumerates what smelt can emit against what a
given backend accepts, so we cannot answer "is every built-in correctly translated for
Spark / BigQuery?" except anecdotally.

The dangerous residue is not the unknown function — a backend rejects that loudly. It is
the construct that exists on **both** engines with **different semantics**, so the query
succeeds and returns a different number. GoogleSQL's infix `^` (bitwise XOR, not power)
is the known member of that class. Nothing enumerates the rest.

### What exists today

Emission facts live as hand-written `match` arms in `crates/smelt-dialect/src/printer.rs`:

- `remap_function_name` — 5 names across 4 dialects (`EXPLODE`, `BOOL_AND`/`EVERY`, `BOOL_OR`, `UNNEST`).
- `print_bigquery_median` / `print_bigquery_modulo` / `print_bigquery_power` — three structural rewrites.

`BuiltinRegistry` (`crates/smelt-types/src/signatures.rs`) carries 232 `Signature::new`
rows and already holds per-backend *type* data (`canonical_return`, `engine_native`,
`needs_cast_for`). It holds no emission data. `crates/smelt-types/tests/registry_coverage.rs`
is dialect-blind.

### A finding that shapes the design

**The infix operators are not registry entries.** `POWER`, `POW` and `MOD` are registered;
the infix `^`, `**` and `%` are `BINARY_EXPR` in the CST and appear nowhere in
`BuiltinRegistry`. The operators that *are* registered use pseudo-names (`LIKE`, `ILIKE`,
`GLOB`, `IS_NULL`, `IS_NOT_NULL`, `BETWEEN`, `IN`, `EXISTS`, `CAST`).

A registry-only enumeration would therefore walk straight past the exact case this issue
was filed about. The operator surface must be enumerated too, which is why operators
become registry entries carrying a syntax form.

## What is being built

Two things, in this order:

1. **The registry becomes the single owner of per-dialect emission** — how a built-in is
   spelled so the backend computes what smelt's semantics say it computes. This is
   production data; the printer consumes it. No test data enters `smelt-types`.
2. **A cross-engine audit suite** enumerates the registry against real engines in two
   layered legs (schema, then values), classifies every `(entry, dialect)` pair, and
   publishes the classification as a standing table with a drift gate.

## Design

### 1. Registry emission data model

Three additions to `crates/smelt-types/src/signatures.rs`:

**`DialectId`** — `DuckDb | SparkSql | PostgreSql | BigQuery`, in `smelt-types`, with
`SqlDialect::id() -> DialectId` in `smelt-dialect` (which already depends on `smelt-types`,
so the layering holds). This replaces the stringly-keyed convention `engine_native` uses
(`"duckdb"`, `"spark"`), where a typo'd key silently means "no override" — a fail-loud
violation that must not propagate into a second table. `engine_native` migrates to
`DialectId` keys in the same change; it currently has no consumers outside `signatures.rs`,
so the migration is mechanical.

**`Emission`** — the per-dialect verdict for one entry:

```rust
pub enum Emission {
    Native,                                  // same spelling, same semantics
    Rename(&'static str),                    // same call shape, different name
    Rewrite(RewriteId),                      // structural; printer owns the code, registry owns the claim
    Unsupported { reason: &'static str },    // a diagnostic, never a silent pass-through
}
```

`Signature` gains `emission: &'static [(DialectId, Emission)]`, authored via
`.with_emission(...)`, defaulting to `Native` for unlisted dialects.

`Native` is a **claim, not an assumption**. The value leg exists to test it, and an
untested `Native` is reported as *unverified* rather than as *passing*. This keeps
authoring cheap — roughly ten rows are non-`Native` today — without re-creating the
silent hole the default would otherwise be.

**`SyntaxForm`** — `Call | Infix | Prefix | Postfix | Special`, added to `Signature`.
Required so the infix operators can be registry entries at all, and it is what lets the
audit harness derive a probe from a signature instead of from a hand-written table.
`^`, `**`, `%` and `||` gain rows; `^`'s BigQuery emission is `Rewrite(BigQueryPower)`,
and its `Native` claim on Spark becomes something a test exercises.

Position-dependence stays inside the rewrite. `MEDIAN` needs a *different* BigQuery
rewrite in aggregate versus window position; `print_bigquery_median` already handles both.
The registry says "this needs rewriting"; the printer says how.

### 2. Printer refactor

`remap_function_name` is deleted. `FUNCTION_CALL` and `BINARY_EXPR` handling both become
"resolve the entry, read its `Emission` for `ctx.dialect.id()`, dispatch on the verdict".
The three `print_bigquery_*` functions stay as code, reached through `RewriteId` dispatch
rather than a name-matched `if` chain, so the set of rewrites is enumerable.

`Unsupported` gains a real behaviour. Today an unrecognised function passes through
verbatim and the backend rejects it at runtime. With a declared verdict the compiler emits
a diagnostic naming the function and the backend — compile-time failure instead of a
warehouse round trip. This is a new diagnostic code and needs a row in
`docs/specs/diagnostics.md`.

### 3. The audit harness

**`smelt-oracle-testkit`.** The oracles (`DuckDbOracle`, `SparkOracle`, `BigQueryOracle`,
`classify_oracle_error`) live in `crates/smelt-db/tests/prop_helpers/` and are reachable
only from that crate's test tree. They are promoted into a test-support crate following
the `smelt-maintenance-testkit` precedent; `smelt-db`'s proptests import from it instead.
It is derived test-support, so it stays outside the `unwrap`/`expect` ratchet's production
set. Roughly 1,300 lines move with no logic change.

**The fixture is inline, not DDL.** One deterministic ~8-row table expressed as a `VALUES`
CTE (BigQuery: `UNNEST([STRUCT(...)])`), with a typed column per `TypeConstraint` family —
`n_int`, `n_bigint`, `n_double`, `n_dec`, `s_text`, `b_bool`, `d_date`, `ts_ts`, `arr_int`,
`j_json` — including NULL-bearing rows. No DDL, no cleanup, nothing materialised, and the
same fixture text serves a BigQuery dry run and a real execution.

**Probes are derived, not authored.** For each entry: `params`' `TypeConstraint` selects
the fixture column, `SyntaxForm` decides the spelling (`a % b` versus `MOD(a, b)`), and
`kind` decides the query shape — `Scalar` → plain `SELECT`, `Agg` → `GROUP BY`,
`Window` → `OVER (…)`. Aggregates are probed in **both** positions, since `MEDIAN` proves
the lowering differs per position. A small test-side override table covers the minority
where a type-correct argument is not a *meaningful* one — regex patterns, date-part
strings, JSON paths, format strings. Keyed by canonical name, this is the only
hand-written per-function data in the design.

This override table replaces `core_functions()` in
`crates/smelt-db/tests/prop_helpers/generators.rs` (98 hand-maintained `FuncDesc` rows,
registry-blind) as the source of probe shapes.

**Two legs over one enumeration.**

- **Schema leg** — print for the dialect, ask the oracle for the output schema (BigQuery
  dry run, Spark `DESCRIBE QUERY`, DuckDB prepare), compare against smelt's inference via
  the existing `compare_types` / `divergences` machinery. Acceptance alone is most of the
  value: it catches every missing lowering and every `Unsupported`.
- **Value leg** — execute on the target and on DuckDB, compare row-wise under a typed
  comparator: exact for integers, strings and booleans; relative tolerance for floats;
  scale-normalised for decimals; NULL equals NULL; deterministic `ORDER BY`. This is the
  leg that catches `^`.

DuckDB is the reference, matching the repo's existing oracle convention.

**Batching.** Probes are grouped by `(dialect, query shape)` and emitted as one `SELECT`
with one column per probe — a few dozen queries instead of ~500, which matters because a
BigQuery round trip costs ~440ms. On any failure the group re-runs one probe per query so
the error names the function rather than the batch.

**The seam leg.** Five end-to-end models — one per shape (scalar, aggregate, window,
operator, table function) — run through the real `execute_project` pipeline per backend.
The enumerating legs test the printer; this leg guards the printer → cast-wrap →
projection seam, where the `MEDIAN` re-parse bug actually lived. Five models, not 232, so
it does not scale with the registry.

**New BigQuery capability.** The BigQuery oracle is dry-run only today (zero bytes billed,
schema only). The value leg needs real execution, so `BigQueryOracle` gains an execute
method. This is the point at which the BigQuery sweep starts costing money per run.

### 4. Ledger and gates

**Ledger** — `dialect_divergences.rs`, one row per `(entry, dialect)`:

| Verdict | Meaning | Fails? |
|---|---|---|
| `Divergent { reason }` | Accepted and permanent (e.g. Spark integer-division semantics) | No — reported as a semantic difference users must know about |
| `Gap { issue }` | A lowering we owe, with a tracking issue | No — but the count ratchets down only |
| `SchemaOnly { reason }` | Nondeterministic entry (`RANDOM`, `NOW`, `CURRENT_DATE`, `UUID`) | No — value leg skipped, reason recorded |
| absent | Must pass both legs | Yes |

The `Gap` count ratchets via `.claude/dialect-gaps-baseline.txt`, matching
`.claude/parser-gaps-baseline.txt`. The ledger is **two-sided**: an unregistered mismatch
fails loudly, and so does an unreachable row — an entry naming a pair that no longer
diverges is an error telling you to delete it, as with the hardening baseline.

Nondeterministic entries are `SchemaOnly` because engines execute at different instants
(`NOW`, `CURRENT_DATE`) or produce no stable value at all (`RANDOM`, `UUID`). This is a
recorded gap, not a hidden one.

**Gates, by tier:**

| Gate | Needs a warehouse? | Tier |
|---|---|---|
| Coverage totality — every entry × dialect has a verdict; every probe derivable or overridden | no | per-PR |
| Printer/registry consistency — no name-matched dialect arms remain in `printer.rs` | no | per-PR |
| Schema + value legs, DuckDB | no (in-memory) | per-PR |
| Schema + value legs, Spark | Spark Connect | labeled PR + nightly, via the existing `spark-parity` job |
| Schema + value legs, BigQuery | live BigQuery | manual sweep, `scripts/bigquery-dialect-audit.sh`, gated on `SMELT_BQ_PROJECT` |

BigQuery stays manual, consistent with `docs/specs/multi_backend.md` §"BigQuery has no CI
tier, by decision, not by omission". That decision is not being revisited here; its
rationale applies more strongly to this suite than to any existing one, because the value
leg executes rather than dry-runs.

**The report.** The suite emits the standing table issue #171 asked for — entry × dialect
→ native / rename / rewrite / unsupported / divergent / gap — to a generated
`docs/reference/dialect-coverage.md`, with a doc-sync gate so a stale table fails. The
table is the deliverable; the gate is what keeps it honest.

## Consequences for the invariants

`CLAUDE.md` §"Function-registry single ownership" today covers a built-in's name,
classification and registry-driven type. This extends it to a built-in's **emission**:
a function's per-dialect spelling derives from `BuiltinRegistry`, never from a
name-matched arm in the printer. The existing `registry_consistency` gate gains a sibling
asserting the printer holds no such arms.

## Spec deltas this implies

Written before implementation, per the spec-first rule:

- `docs/specs/multi_backend.md` — emission ownership; the audit suite and its tiers; the
  coverage table's location and meaning.
- `docs/specs/functions.md` — `Emission`, `SyntaxForm`, `DialectId` as registry surface.
- `docs/specs/diagnostics.md` — the new `Unsupported`-on-backend diagnostic code.
- `docs/specs/architecture.md` §Constraints item 14 — single ownership extended to emission.
- `CLAUDE.md` — the same invariant line, plus the new standing gates.

## Alternatives rejected

- **Schema-only auditing.** Cheap and billable-free, but structurally cannot catch the
  silent-divergence class: `^` returns `INT64` on both engines and a different number.
- **Value-only auditing.** Catches divergence but costs a real execution for all 232
  entries on every sweep, with no cheap tier that visits every name.
- **Probe data in the registry.** Rejected: fixture column names and sample arguments are
  test vocabulary and do not belong in a production crate. The registry carries *how to
  translate*; the harness derives *what to call it with*.
- **Registry emission table plus an unchanged printer, held in sync by a gate.** Two
  sources of truth reconciled by a test is the thing single-ownership exists to avoid.
- **A test-side ledger only, no registry change** (the issue's literal suggestion).
  Zero production change, but leaves the printer's coverage unenumerable and the emission
  facts scattered.
- **Deferring operators and table functions to a follow-up phase.** Would green-light the
  registry while the one class the issue was filed about stayed unexamined.

## Open questions

- Whether the DuckDB value leg over ~232 entries is fast enough for per-PR, or needs the
  same nightly treatment as Spark. To be measured in the first implementation phase, not
  guessed.
- Whether `PostgreSql` participates. It is a `SqlDialect` variant with no backend crate
  and no oracle; the registry will carry verdicts for it, but no leg exercises them until
  a PostgreSQL backend exists. The coverage report must mark those verdicts unverified
  rather than passing.
