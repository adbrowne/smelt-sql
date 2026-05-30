# scoping_probe — adversarial fixtures for docs/specs/scoping.md (phase C2)

Throwaway/keepable probe workspace. NOT in any green-suite gate list.
Demonstrates two `needs-review` findings (see docs/bug-hunt/2026-05-30-findings.md):

- **BUG-018** (run-pipeline PASSING substitution): `functions/badctx.sql` (`pred: Expr<Boolean, source>`)
  called via `models/uses_badctx.sql` with `PASSING pred AS (amount > 5)` — the run pipeline
  emits `WHERE pred` verbatim (the parameter name), DuckDB binder error at `smelt build`.
- **BUG-019** (scoping diagnostics don't gate the CLI run/build pipeline; BUG-006 class):
  `functions/cyclic.sql` has a mutually-recursive CTE that `file_diagnostics` flags as
  `CteCycle`, but `smelt build` splices it into the model and fails in DuckDB with a
  low-level catalog error instead of the spec-mandated `CteCycle`.

`functions/shadow.sql` + `models/uses_shadow.sql` exercise the parameters-first rule
(`ParameterShadowsColumn` warning); `functions/passall.sql` + `uses_passall*.sql` isolate the
`source.*`-spread-through-bare-TableExpr-return schema quirk (outer `SELECT *` → `{}`).
