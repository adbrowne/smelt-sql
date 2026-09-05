# Phase 5 summary — operand-conditional verdicts: the mechanism

## Shipped

- `OperandClass` (`crates/smelt-types/src/signatures.rs`): total classifier `OperandClass::of(&DataType)`
  (`Integral`, `Decimal`, `Floating`, `String`, `Boolean`, `Temporal`, `Interval`, `Composite`,
  `Binary`, `Unresolved`), no `_` arm.
- `Emission::Conditional(&'static [ConditionalArm])`, `ConditionalArm { arity, classes, verdict }`,
  `SettledEmission` (the six ordinary verdicts, no `Conditional` — a nested conditional is
  unrepresentable by construction), `CallFacts` (`new`/`unresolved`), `Signature::settle_at`.
- `validate_conditional` + registry-construction wiring: mandatory trailing `otherwise` arm, arity
  admitted by the signature, argument indices within the guarded arity.
- `crates/smelt-dialect/src/emission_settle.rs` (new, `pub mod`): `settle_emissions(root, dialect,
  type_of)` — the compile-path walk; `settled_verdict_for(node, sig, position, ctx)` — the
  printer's one lookup, with an arity-only fallback on a miss.
- `PrintContext::settled_emissions: &[(TextRange, SettledEmission)]`, threaded through
  `print_checked_for` (now takes `Option<&TypeContext>`) from `SqlCompiler::print_checked`'s own
  `build_projection_type_context` — same construction `derive_projection_for` uses.
- `printer.rs`, `restructure.rs`, `emission_check.rs` moved off `emission_at` onto
  `settle_at`/`SettledEmission` for their resolution; `emission_at` stays for the audit/coverage
  report.
- Spec edits in `docs/specs/multi_backend.md`: `Json` → `Binary` operand class (there is no
  `DataType::Json`), `Null`/`Unknown` both → `Unresolved`; Known Divergences bullet narrowed to
  "no row is conditional yet" (mechanism ships here, population is phase 7).
- Tests: `registry_coverage.rs` (+16: totality, arm ordering, arity/class guards, otherwise
  fallback, 4 validation-failure cases, registry-build extension), `emission_ownership.rs` (+2:
  `printer_holds_no_type_context`, `printer_never_resolves_an_arm`), new
  `crates/smelt-dialect/tests/operand_conditional.rs` (3 tests against the real `//` entry — see
  Decisions), `dialect_seam.rs` (+2: unresolved-operand refusal on Spark, structural single-call
  gate on `settle_emissions`).

## Decisions

- No production registry row is `Conditional` yet (matches the plan — phase 7's job), so
  `operand_conditional.rs`'s tests exercise the walk mechanics (position/arity/class extraction,
  matching a direct `settle_at` call) against `//`, the one entry whose non-DuckDB verdict is
  already wholesale `Unsupported`. Arm-selection logic itself (first-match, arity/class guards,
  `otherwise`) is proven against synthetic signatures in `registry_coverage.rs`, independent of the
  global registry.
- `restructure::plan` and `emission_check::unsupported_emissions` settle with
  `CallFacts::unresolved(arity)` (no `TypeContext` threaded to either) rather than accepting a
  settled-verdicts parameter — arity alone is sufficient since no row's `otherwise` arm differs in
  behavior from today's wholesale verdicts yet; only `print_checked_for`'s printer path gets the
  real per-argument classes, because it's the one place a `TypeContext` was already being built
  for the projection.
- `ArityNotAdmitted`/`ArgumentIndexOutOfRange` bounds: an arity-guarded arm must equal the
  signature's fixed arity (or be `>= fixed_arity - 1` for a variadic tail); a class guard's index
  must be `< arm.arity.unwrap_or(fixed_arity)`.

## For the next planner

- Phase 7 (per the existing phase table) populates the first real `Conditional` rows (`%` on
  BigQuery, `LOG`, Spark `TRUNC`/`TO_JSON`, `//` per class) on a live engine — this is where
  `unsupported_emissions`/`restructure::plan`'s arity-only settlement should be revisited: if a
  real class-guarded arm needs to be honored pre-print (not just its `otherwise`), thread the
  already-settled list (from `print_checked_for`'s `settle_emissions` call) into
  `unsupported_emissions` instead of recomputing with unresolved facts.
- Coverage-table arm-set rendering (criterion 7's "conditional cells rendered as the set of their
  arms") is still a placeholder (`report.rs` renders `"conditional"` unconditionally) — phase 6
  ("audit probes every arm... coverage table renders arm sets") is exactly this.
- Nothing found out of scope beyond what the plan already excluded.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-types --test registry_coverage --test unknown_census` — 100 + 4 passed.
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional --test
  template_emission --test snapshots` — all passed.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test
  restructure_multiplicity` — 15 + 4 + 1 passed.
- `cargo test -p smelt-db --test dialect_audit` (53 passed) and `--test integration
  registry_consistency` (6 passed).
