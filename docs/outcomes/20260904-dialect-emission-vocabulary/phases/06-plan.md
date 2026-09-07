# Phase 6 plan — the audit probes every arm

## Objective

Extend the cross-engine audit's probe derivation, coverage totality gate, ledger key and coverage
table from `(entry, dialect, position)` to `(entry, dialect, position, arm)`, so that the operand
axis phase 5 built is *verified* rather than merely representable. Advances criteria 7 (arm-set
rendering in `docs/reference/dialect-coverage.md`) and 9 (`dialect_audit` green), and is the
precondition for phase 7: every conditional row it lands on a live Spark is probed per arm the
moment it exists, with no further harness work.

No production entry is `Conditional` yet (phase 7 populates them), so the registry-wide gates land
green-but-vacuous by construction; the mechanism itself is proven red-green against synthetic
signatures built in the test module, exactly as `registry_coverage.rs` does. That is deliberate:
the gate must exist *before* the rows, or phase 7 lands unprobed arms.

## Spec delta

`docs/specs/multi_backend.md` §"Operand-conditional verdicts", final paragraph ("The audit probes
every arm"): add one sentence — no typed fixture column can classify as `Unresolved`, so an arm
guarded on that class is probed with a bare `NULL` literal rather than a column, which is what the
class means at a call site. Everything else that paragraph already states normatively (one column
per class, totality counts arms, ledger keyed on the arm, arm-set cells) is what this phase
implements; no other spec edit.

## Tests

Red-green, all in `crates/smelt-db/tests/dialect_audit/` unless noted.

1. `the_fixture_has_a_column_for_every_operand_class` — every `OperandClass` except `Unresolved`
   resolves to a fixture column whose `column_types()` entry classifies back to that same class;
   `Unresolved` resolves to the `NULL` literal.
2. `the_duckdb_fixture_executes_and_yields_eight_rows` (existing) — stays green with the two new
   columns; this is what verifies the interval and blob literals on a live DuckDB in-process.
3. `a_conditional_entry_is_probed_once_per_arm` — a synthetic two-argument conditional
   (`[a0:Integral,a1:Integral → Native, otherwise → Unsupported]`) yields two scalar probes with
   distinct aliases.
4. `an_arm_probe_selects_the_arm_it_was_derived_for` — the arguments chosen for arm *k* produce
   `CallFacts` that `settle_at` resolves to arm *k*'s verdict, for every arm of the synthetic entry.
   Guards against an earlier arm capturing a later arm's probe.
5. `an_unreachable_arm_is_named_never_skipped` — a synthetic entry whose second arm is shadowed by
   its first reports that arm by index, rather than yielding one fewer probe silently.
6. `every_conditional_arm_is_covered_by_a_probe` — registry-wide: for every `(name, dialect,
   position)` whose emission is `Conditional`, every arm is selected by some derived probe; an
   uncovered arm is named with its index.
7. `probe_aliases_are_unique` (existing) — extended over the synthetic arm probes: the arm suffix
   must not collide with a position suffix.
8. `a_ledger_row_scoped_to_an_arm_does_not_exempt_another_arm` — `ledger::find` unit test on the
   new arm key.
9. `a_ledger_row_naming_an_arm_the_entry_does_not_have_is_reported` — the arm side of the ledger's
   two-sidedness.
10. `the_declared_unsupported_exemption_settles_the_probes_arm` — the audit's own
    "registry declares this unsupported" exemption resolves through `settle_at` with the probe's
    facts, so an arm-specific `Unsupported` exempts that arm and no other.
11. `a_conditional_cell_renders_its_arm_set` (`report.rs`) — a synthetic conditional renders as
    `conditional(a0:integral,a1:integral→native | otherwise→unsupported)`, not `conditional`.
12. `the_coverage_table_matches_the_registry` (existing doc-sync) — green after regeneration.

## Tasks

1. `fixture.rs`: add `iv_interval` and `bin_blob` columns — per-dialect type names (`INTERVAL` /
   `BLOB`, `INTERVAL DAY` / `BINARY` on Spark, `INTERVAL` / `BYTES` on BigQuery) and literals, one
   NULL each, `ROWS` widened to 14 cells, `column_types()` extended with `Interval` and `Blob`.
2. `probe.rs`: `arg_for_class(OperandClass) -> &'static str` — the fixture column per class, the
   literal `NULL` for `Unresolved`; the single owner test 1 checks.
3. `probe.rs`: `Probe` gains `arm: Option<usize>` and the `CallFacts` its arguments imply; alias
   gains an `_a{k}` suffix when `arm` is set.
4. `probe.rs`: arm enumeration. For an entry whose emission is `Conditional` on any dialect at a
   position, derive one probe per distinct arm guard across dialects (deduplicated by guard), by
   searching class assignments over the guarded argument indices for one that `settle_at` resolves
   to the target arm; an arm no assignment reaches is `NotProbed::UnreachableArm { index, detail }`.
   A non-conditional entry keeps producing exactly today's probe (`arm: None`) — assert byte-equal
   aliases for the existing set before/after.
5. `main.rs`: add the totality gate (test 6); change the declared-unsupported exemption from
   `emission_at` to `settle_at` with the probe's facts (test 10).
6. `ledger.rs`: `LedgerRow.arm: Option<usize>` with an `arm_at` constructor, threaded through
   `find(name, dialect, position, arm, leg)` and `is_registered`; extend the existing
   "names a real entry and a probed pair" test with the arm side (test 9).
7. `report.rs`: render `Emission::Conditional` as its arm set (replacing the phase-5 placeholder
   label and its outcome-referencing comment) and add the legend bullet; regenerate
   `docs/reference/dialect-coverage.md` with
   `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit the_coverage_table_matches_the_registry`.
8. Land the spec sentence above.

## Risks (bounded, do not defer)

- The new fixture columns must render legally on **all three** dialects, but only DuckDB is
  verifiable in-process here; Spark's interval/binary spelling is confirmed by phase 7's live run.
  If a spelling proves wrong there, it is phase 7's to fix, not a new phase.
- Mapping `TypeConstraint::Concrete(Interval)`/`Concrete(Blob)` in `column_for` (as opposed to the
  new per-class mapping) would change the *existing* probe set and could surface new gaps. Leave
  `column_for` untouched; the two mappings answer different questions (a declared constraint versus
  an arm guard) and test 1 keeps the new one honest.
- `.claude/dialect-gaps-baseline.txt` must be **unchanged** by this phase: no arm probe exists to
  add a gap. A changed count means task 4 perturbed the non-conditional probe set — fix that rather
  than the baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test dialect_audit` and `--test integration registry_consistency`
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional`
- `git diff --stat .claude/dialect-gaps-baseline.txt` — empty.

## Commit message

`feat(audit): probe every operand-conditional arm — per-class fixture columns, arm-keyed totality, ledger and coverage cells`
