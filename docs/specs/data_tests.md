---
feature: data_tests
status: experimental
last_reviewed: 2026-07-20
owners: [andrew]
---

# Declarative column tests

> **What this is.** A normative spec for `columns.<name>.tests` — declarative, dbt-familiar column-level test constraints (`not_null`, `unique`, `accepted_values`, `relationships`) attached to a model's `columns:` frontmatter. It covers the test-kind grammar, the diagnostics for unknown kinds and out-of-schema columns, and the resolution order that consults a model's derived properties before ever emitting a scan. Out of scope: the `columns:` map's other keys and its silent-drop rule for unmodeled columns (see `models.md` §"`columns:` — column metadata"); the `smelt.test`/`smelt.check` declaration kinds and the failing-rows execution machinery a test lowers into (see `testing.md`); the `smelt` CLI surface that runs and reports tests (see `cli.md`); the derived nullability, grain, and functional-dependency proofs a test's proof step consults (see `model_properties.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### `columns.<name>.tests`

Each column entry under a model's `columns:` frontmatter map (`models.md` §"`columns:` — column metadata") may carry a `tests` key. The value is a list; each list entry is one of four test kinds:

```yaml
columns:
  order_id:
    tests:
      - not_null
      - unique
  status:
    tests:
      - accepted_values: ['pending', 'shipped', 'cancelled']
  customer_id:
    tests:
      - relationships:
          to: customers
          field: id
```

| Kind | Form | Parameters |
|------|------|------------|
| `not_null` | bare string list entry | none |
| `unique` | bare string list entry | none |
| `accepted_values` | `{accepted_values: [<literal>, ...]}` | `accepted_values` — a non-empty list of scalar literals the column's value must be one of |
| `relationships` | `{relationships: {to: <model>, field: <column>}}` | `to` — the bare address path of the referenced model (same address shape as `testing.md`'s `PASSING <dep>`); `field` — the column in `to` the value must match |

A column's `tests` list may contain any combination of these four kinds, including more than one of the same kind (e.g. `unique` alongside a `relationships` entry on the same column).

### Fail-loud validation

Two conditions are hard diagnostics, raised at the same validation pass that reads `columns:` frontmatter:

- **Unknown test kind.** A `tests` list entry that is not one of `not_null`, `unique`, `accepted_values`, `relationships` — including a misspelled kind name or an unrecognized parameterized form — is a hard error naming the offending entry and the four recognized kinds.
- **Test on a column absent from the inferred schema.** A `columns.<name>.tests` entry where `<name>` does not appear in the model's inferred output schema is a hard error naming the column and the model.

This is a deliberate **contrast** with `models.md`'s rule for the rest of the `columns:` map: a `description` (or other non-`tests` key) on a column absent from the inferred schema is silently dropped from catalog output, because a stale description is merely inert. A stale or misspelled *test* is not inert — a `tests` entry that silently no-ops on a renamed or dropped column is a test that was never running, and a project that believes it is asserting `not_null` on a column is not. Fail-loud is the only behavior that keeps a passing test suite meaningful.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `UnknownColumnTestKind` | Error | A `columns.<c>.tests` entry does not match `not_null`, `unique`, `accepted_values`, or `relationships`. Anchored at the offending entry. |
| `ColumnTestOnUnknownColumn` | Error | A `columns.<c>.tests` entry names a column `<c>` absent from the model's inferred output schema. Anchored at the column key. |

## Semantics

### Resolution order

Each declared column test is resolved independently, in two steps, in this order:

1. **Consult derived properties first.** Before any scan is considered, the test is checked against what the model's SQL already proves:
   - `not_null` is **proven** when the column's inferred nullability (`model_properties.md`'s nullability analysis, driven by the shared bottom-up walk) is non-nullable.
   - `unique` is **proven** when the column (or column set, for a composite test) is exactly the model's declared grain key (`unique_key:`, `models.md` §"`columns:` — column metadata") or a set the walk has otherwise proven to be a grain/functional-dependency key for the model's output.
   - `accepted_values` and `relationships` have no derived-property proof path today — see §Known Divergences.

2. **Lower to a scan when unproven.** A test that is not proven in step 1 lowers to a failing-rows SELECT, executed by the same `smelt check` machinery `testing.md` §"Check execution model" documents (real target, zero rows = PASS, one or more rows = violation). The generated failing-rows predicate per kind:
   - `not_null`: rows where the column `IS NULL`.
   - `unique`: rows whose column value (or column-set tuple) appears more than once.
   - `accepted_values`: rows whose column value is not `IN (<accepted_values>)` (and is not `NULL` — a separate `not_null` test governs nullability).
   - `relationships`: rows whose column value has no matching row in `to` where `to.field` equals the value (a left-anti-join), excluding `NULL` values.

### Proof is a scan-elimination, never a failure-suppression

A derived-property proof may only **remove** a scan; it may never suppress a genuine failure. If the proof engine cannot decide a test's truth from the model's derived properties — including any case where the walk's verdict is itself unproven, ambiguous, or the property in question does not yet have a proof path (as with `accepted_values`/`relationships` today) — the test **falls through to the scan**. There is no partial-credit or best-effort proof state that skips the scan without a positive proof; undecidable resolves to "run the scan," never to "assume pass."

### Reporting

A proven test reports a compile-time verdict of `proven — no scan emitted` in `smelt test`/`smelt check` output (per `cli.md`'s reporting conventions for those commands) and in the data catalog (`data_catalog.md`), rather than a `PASS`/`FAIL` scan result. An unproven test reports through the same `PASS`/`FAIL`/`WARN` machinery as any other `smelt.check` (`testing.md` §"Reporting"). Both proven and scanned tests for a model are visible together in the same report, distinguished by verdict kind, so a reader can see at a glance which of a model's tests cost a scan and which did not.

### Severity

Declarative column tests are **error-severity only** — a failing test blocks the same way an `error`-severity `smelt.check` does (`testing.md` §"Check frontmatter knobs", §"Build integration"). There is no `warn`-severity declarative test today; see §Known Divergences for the extension point.

## Design

**Derived-property-aware, not a pure runtime-scan clone.** dbt's `not_null`/`unique` generic tests always compile to a runtime scan, even when the column's own definition (a primary key, a `NOT NULL` constraint the warehouse already enforces) already guarantees the property. smelt's type system and grain/functional-dependency proofs (`model_properties.md`) already derive nullability and key-ness for large classes of models as a side effect of compiling them — re-proving the same fact with a warehouse scan is redundant work paid on every run. Consulting the derived properties first means a `not_null` test on a column the type checker already proved non-nullable, or a `unique` test on the model's own declared grain key, costs nothing at run time and cannot silently rot: the proof is re-checked on every compile, so a code change that breaks the guarantee re-opens the scan rather than leaving a stale, no-longer-true green result. This is the same "derive, don't declare" principle applied elsewhere in the spec set (`incremental_models.md`'s batch-safety classification, `model_properties.md`'s bound/reach analysis): a property the compiler can already see should not also need a human to declare and maintain it as a separate, driftable fact.

**Rejected: pure dbt-clone, scan-only tests.** Always lowering every declared test to a scan (dbt's model) was rejected because it forfeits information the compiler already has. It is the simpler implementation and remains available as the fallback path for `accepted_values`/`relationships` and any case the proof engine cannot decide — the resolution order in §Semantics degrades to exactly this behavior whenever a proof is unavailable, so nothing is lost by trying the proof first.

**Rejected: a separate top-level `tests:` block.** dbt's `schema.yml` supports both column-level tests nested under `columns:` and a `models:`-level `tests:` list for multi-column assertions. smelt's `columns:` map is the single canonical home for per-column metadata (`models.md`'s "Canonical home" note on `columns:`); a column test is a fact about a column, and `relationships`/`accepted_values`/`not_null`/`unique` are all single-column assertions in this surface. Introducing a second top-level `tests:` block for the same declarations the `columns:` map already owns would duplicate the grammar-ownership question `models.md` settles for every other column-scoped key. Multi-column and model-level assertions remain available today via `smelt.check` (`testing.md`), which already supports an arbitrary failing-rows query; nothing in this design restricts it.

## Constraints & Invariants

1. **A proof may only remove a scan, never suppress a failure.** An undecidable or absent proof always falls through to the scan; there is no proof state that reports PASS without either a positive derived-property proof or an executed scan returning zero rows.
2. **Unknown test kinds and tests on unmodeled columns are hard diagnostics.** Neither condition silently drops the test, in contrast to the silent-drop rule for other `columns:` keys (`models.md`).
3. **A column's derived-property proof is re-evaluated on every compile.** A proof is never cached across a change to the model's SQL or its declared `unique_key:` — the same recompute-on-change guarantee the rest of the derived-property system provides (`model_properties.md`).
4. **Declarative column tests carry no `PASSING`/`EXPECT`/`#` surface.** They are not a `smelt.test`; an unproven test lowers to a `smelt.check`-shaped failing-rows scan, which has no mock-data or CTE-isolation surface (`testing.md`).

## Known Divergences / Open Questions

- **No derived-property proof path for `accepted_values`/`relationships`.** Only `not_null` and `unique` have a defined proof step in §Semantics today; `accepted_values` and `relationships` always lower to a scan. A future enumerated-domain type or a declared foreign-key fact could give either kind a proof path, but neither exists yet. Tracked in `docs/plans/20260719-prod-w3-adoption.md`.
- **No `warn` severity.** `smelt.check`'s `severity: warn` (`testing.md`) has no declarative-column-test equivalent yet; every declarative test is error-severity. The extension point is recorded here so a future `severity:` key on a test entry has a settled home. Tracked in `docs/plans/20260719-prod-w3-adoption.md`.
- **No generic/reusable test macros.** dbt's custom generic tests (user-defined parameterized test macros beyond the four built-ins) are not part of this surface. The four built-in kinds cover the common adoption case; whether and how to add user-defined kinds is open.
- **`unique`'s proof consults only the declared `unique_key:`.** The walk-proven grain/functional-dependency key sets §Semantics mentions as an additional proof source ("or a set the walk has otherwise proven to be a grain/functional-dependency key") are not yet wired into the proof step; only the frontmatter-declared `unique_key:` is consulted. A `unique` test on a column set the walk could prove a key by other means (without a matching `unique_key:` declaration) falls through to a scan today. Tracked in `docs/plans/20260719-prod-w3-adoption.md`.

## References

- **Code**: `crates/smelt-core/src/metadata.rs` (`ColumnTest`, `validate_column_tests`, `validate_column_tests_against_schema`), `crates/smelt-logical/src/data_tests.rs` (`resolve_not_null_verdict`, `resolve_unique_verdict`, `lower_column_test`, `ScanLowering`), `crates/smelt-cli/src/commands/check.rs` (proven-verdict reporting, pending-scan lowering and execution via `smelt_runtime::run_single_check`)
- **Tests**: `crates/smelt-core/src/metadata.rs` (`parses_column_tests_list`, `unknown_test_kind_is_metadata_error`), `crates/smelt-logical/src/data_tests.rs` unit tests, `crates/smelt-cli/tests/data_tests.rs`
- **User docs**: `docs-site/docs/guide/testing.md` §"Declarative column tests"
- **Plans (history)**:
  - `docs/plans/20260719-prod-w3-adoption.md` — introduces this spec and the declarative-test implementation
- **Related specs**:
  - `models.md` §"`columns:` — column metadata" — the canonical `columns:` grammar this spec's `tests` key joins
  - `testing.md` — the `smelt.check` failing-rows execution machinery an unproven test lowers into, and the `smelt.test` unit-test kind this spec is distinct from
  - `cli.md` — the `smelt test`/`smelt check` commands that run and report declarative tests
  - `model_properties.md` — the derived nullability and grain/functional-dependency proofs the resolution order consults
