# Phase 6 plan — Un-rot the gated conformance twin (`gate_composed.rs`)

## Objective

Make `smelt-maintenance-testkit`'s `families/` module compile again under the `spark` and
`bigquery` features, and add a per-PR compile guard so it cannot rot silently a second time.
Advances success criterion 6 ("all standing gates green"): today the gated conformance twin cannot
even be type-checked, so neither this outcome's remaining phases nor any other change can honestly
verify themselves under those features.

## Spec delta

None. This is a test-harness compile fix plus a CI guard — no user-visible feature behaviour
changes, so the spec-first rule does not bite. Do **not** edit `docs/specs/`.

## Background (already established, do not re-derive)

- `crates/smelt-maintenance-testkit/src/families/mod.rs:23` is
  `#![cfg(any(feature = "spark", feature = "bigquery"))]`, so no default-feature build compiles
  `gate_composed.rs`. That is why the rot went unnoticed.
- `smelt_runtime::maintenance_driver::run_windowed_keyed_maintenance` (`maintenance_driver.rs:309`)
  now takes 12 parameters, with `write_pin: Option<&'static WritePattern>` sitting **between**
  `suppression` and `compile_step`. The call at `gate_composed.rs:343` still passes 11 — every
  later argument is shifted by one, which is the whole reported "arg-count/closure-type mismatch".
- The reference-shaped call is `smelt-runtime/src/cumulative.rs:531`. The route-3 composed recipe
  stages no `maintenance.cells[].write` pin, so the correct value here is `None` — do not invent a
  pin lookup in the testkit.
- Phase 5's summary confirmed by stash-and-recheck that this failure is pre-existing and unrelated
  to `GenRow`/`Option`/`val`. Do not re-litigate that.

## Tests

Compilation *is* the red-green oracle for the fix itself; the guard is what makes it standing.

1. `cargo check -p smelt-maintenance-testkit --features spark --all-targets` — RED before the fix
   (the 5 reported errors), GREEN after. Run it **first**, unfixed, and paste the error list into
   the summary so the red state is recorded.
2. `cargo check -p smelt-maintenance-testkit --features bigquery --all-targets` — same, the
   BigQuery twin.
3. `cargo check -p smelt-cli --tests --features smelt-cli/spark` and the `smelt-cli/bigquery`
   twin — the exact commands phase 5 recorded as failing; both must go green.
4. `smelt-maintenance-testkit` unit tests under `--features spark` — the module's own
   `#[cfg(test)]` tests (including
   `composed_route3_delta_sql_is_byte_identical_for_duckdb_under_the_staged_query_shape`) must
   pass, proving the fix did not change the staged query shape.

## Tasks

1. Reproduce red: run tests 1–3 unfixed, capture the exact error list.
2. Fix the call at `crates/smelt-maintenance-testkit/src/families/gate_composed.rs:343` — insert
   `None` in the `write_pin` position (between `&composed_route3_suppression()` and
   `compile_step`), with a one-line comment saying the route-3 recipe stages no `write:` pin, in
   the file's existing comment idiom.
3. Sweep for any *other* rot in the gated module surfaced once the first error clears: re-run
   test 1 until clean and fix each remaining mismatch the same way (adapt the call site to the
   current production signature; never widen or change a production signature to suit the
   harness).
4. Add a per-PR compile guard to the `Lint` job in `.github/workflows/test.yml` (a new step after
   "Run clippy", named e.g. "Gated conformance twin compile check"):
   `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets`. Compile-only
   — it needs no Spark server and no BigQuery credentials. Keep it in `Lint`, not the gated
   `spark-parity` job in `compat.yml`, since the entire point is that it runs on every PR.
5. Add a short comment at the top of `families/mod.rs` (beside the `#![cfg(...)]`) naming the CI
   step that keeps this module compiling, so the next reader knows the guard exists.
6. Verify (below), then write `phases/06-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing bundled gate (fmt, clippy over both CI
  feature sets, full `cargo test`, `example_diagnostics`).
- `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` — the new
  guard's exact command; must be clean.
- `cargo check -p smelt-cli --tests --features smelt-cli/spark` — clean.
- `cargo check -p smelt-cli --tests --features smelt-cli/bigquery` — clean.
- `cargo test -p smelt-maintenance-testkit --features spark` — all pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — unchanged pass count (75), proving the
  default-feature conformance gate is untouched.
- Not required: a live Spark server or BigQuery credentials. If any command in this list needs one,
  stop and record it in the summary rather than skipping silently.

## Commit message

`fix(testkit): repair gate_composed's run_windowed_keyed_maintenance call and guard it per-PR`
