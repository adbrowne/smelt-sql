# Phase 6 summary — the audit probes every arm

**Shipped:**
- `fixture.rs`: two new columns (`iv_interval`, `bin_blob`) with per-dialect type names/literals,
  one NULL each; `ROWS` widened to 14 cells; `column_types()` gained `Interval`/`Blob` entries.
  Verified live against DuckDB 1.5.4 (`the_duckdb_fixture_executes_and_yields_eight_rows`).
- `probe.rs`: `arg_for_class(OperandClass) -> &'static str` — the single-owner mapping from arm
  guard class to fixture arg (or the bare `NULL` literal for `Unresolved`), distinct from
  `column_for`'s `TypeConstraint`→column mapping. `Probe` gained `arm: Option<usize>` and
  `facts: CallFacts`. `conditional_arm_probes(name, sig)` derives one probe per distinct arm guard
  across dialects (deduped by arm-list value), proven reachable by an exhaustive `OperandClass`
  search (`find_assignment`) rather than trusted from the arm's list position; an unreachable arm
  is `NotProbed::UnreachableArm { index, detail }`, never silently dropped. `derive_probes()` now
  includes arm probes for any future `Conditional` entry with zero further harness work (criterion
  in the outcome's Objective).
- `main.rs`: `is_declared_unsupported`/`is_exempt`/`is_registered` now thread `arm`/`facts` through
  to `Signature::settle_at`, so an arm-specific `Unsupported` verdict exempts only that arm.
  `every_conditional_arm_is_covered_by_a_probe` (registry-wide totality gate, green-but-vacuous
  today) and `the_declared_unsupported_exemption_settles_the_probes_arm` (unit test on the pure
  `settles_unsupported` helper) added. `every_ledger_row_names_a_real_registry_entry_and_a_probed_pair`
  extended with the arm side; `a_pair_has_at_most_one_ledger_row`'s dedup key now includes `arm`.
- `ledger.rs`: `LedgerRow.arm: Option<usize>` (None = every arm, mirroring `position`'s own
  convention), `arm_at` constructor, `find`/`row_matches` re-keyed on `(name, dialect, position,
  arm, leg)`.
- `report.rs`: `Emission::Conditional` renders its arm set
  (`conditional(a0:integral,a1:integral→native | otherwise→unsupported)`) replacing the phase-5
  placeholder label; legend bullet added; `docs/reference/dialect-coverage.md` regenerated.
- `docs/specs/multi_backend.md` §"Operand-conditional verdicts": one sentence on the `Unresolved`
  class being probed with a bare `NULL` literal (no typed fixture column can classify as it).

**Decisions:**
- Mechanism proven entirely against synthetic signatures (`#[cfg(test)] mod tests` inside
  `probe.rs`, `ledger.rs`, `report.rs`) rather than through the real registry, since no production
  entry is `Conditional` yet — matches the plan's explicit design.
- `is_declared_unsupported`'s registry-resolving wrapper was split from a new pure
  `settles_unsupported(sig, dialect, position, facts)`, mirroring the existing
  `classify_accepted` separation, so the arm-specific-exemption behavior could be unit-tested
  against a synthetic `Conditional` signature without needing a real registered name.

**For the next planner:**
- Test-writing gotcha worth flagging: a `find_assignment` call only exercises the `Conditional`
  branch of `settle_at` if the `Signature` under test actually declares
  `Emission::Conditional(ARMS)` in its own `emission` table via `.with_emission(...)` — a bare
  signature with no emission entries at all falls through to `Native` for every arm, which made
  one first draft of test 4 fail in a confusing way (arm 0 "passed" by accident, arm 1 didn't).
  Left as a comment risk for phase 7: production `Conditional` rows (`LOG`, `TRUNC`, `TO_JSON`,
  `//`) will need this same `.with_emission` wiring done correctly the first time.
- `.claude/dialect-gaps-baseline.txt` is unchanged, as required.
- Nothing else surfaced outside phase 6's own task list.

**Gates:**
- `cargo test -p smelt-db --test dialect_audit` — 61 passed.
- `cargo test -p smelt-db --test integration registry_consistency` — 6 passed.
- `cargo test -p smelt-types --test registry_coverage` — 100 passed.
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional` — 14 passed.
- `git diff --stat .claude/dialect-gaps-baseline.txt` — empty.
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics PASS; workspace `cargo
  test` has one pre-existing, unrelated flaky failure (`smelt-runtime`
  `python::tests::non_convergent_set_errors`, a temp-file race under parallel test execution — a
  file untouched by this phase; passes in isolation and with `--test-threads=1`).
