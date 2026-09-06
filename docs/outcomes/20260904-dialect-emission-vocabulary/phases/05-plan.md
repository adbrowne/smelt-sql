# Phase 5 plan — operand-conditional verdicts: the mechanism

## Objective

Land the vocabulary and the resolution path for verdicts that depend on arity or operand type:
an `OperandClass` classifier beside the registry, `Emission::Conditional` arms with a mandatory
`otherwise`, and settlement on the compile path so the printer receives only settled verdicts.
Advances criterion 4 in full. No production registry row changes behaviour here — the Spark
arms (#174/#178) and `//` per class land in phase 7, on a live engine.

## Spec delta

`docs/specs/multi_backend.md`, made first:

1. §"Operand-conditional verdicts", the **Operand classes** paragraph: replace `Json` with
   `Binary` (there is no `DataType::Json` in this codebase; `Blob` is the variant that needs a
   class, and Spark's `TO_JSON` arm guards on `Composite`, which the paragraph already says).
   Add one sentence: `DataType::Null` and `DataType::Unknown` both classify as `Unresolved` — a
   NULL literal discriminates nothing, and sending it to `otherwise` is the fail-safe direction
   the same paragraph's cost rule already demands.
2. §Known Divergences, the bullet beginning "**No operand-conditional verdict exists**": narrow
   it to the registry rows that remain (#173 BigQuery `%`, #174 `LOG`/`DAYOFWEEK`, #178 Spark
   `TRUNC`/`TO_JSON`, `//` per class). The mechanism and the compile-path resolution step exist;
   delete the sentences claiming they do not.

## Design decisions (fixed here, inside the phase's own scope)

- **`OperandClass` lives in `smelt-types::signatures`** — beside the registry, as the spec
  requires — as `OperandClass::of(&DataType) -> OperandClass`, a total match with no `_` arm so
  a new `DataType` variant is a compile error rather than a silent misclassification.
- **Two types, not one.** `Emission` gains `Conditional(&'static [ConditionalArm])`;
  `ConditionalArm { arity: Option<usize>, classes: &'static [(usize, OperandClass)], verdict:
  SettledEmission }`. `SettledEmission` is the six settled verdicts (`Native`, `Rename`,
  `Template`, `Rewrite`, `Restructure`, `Unsupported`). An arm's verdict is a `SettledEmission`
  by type, so a nested conditional is unrepresentable rather than merely rejected. Every
  consumer moves to `Signature::settle_at(dialect, position, &CallFacts) -> SettledEmission`;
  `emission_at` stays for the audit and the coverage report, which must see the arms.
- **Settlement runs inside `print_checked_for`** (`smelt-runtime/src/compile.rs`) — the single
  funnel `dialect_seam::every_compile_path_is_emission_checked` already guards, so no entry
  point can skip it. It takes a `&TypeContext`; `SqlCompiler::print_checked` passes the same
  context `derive_projection_for` uses, so settlement and the projection read one inference. A
  caller with no context passes an empty one: every class is `Unresolved`, every conditional
  takes `otherwise`.
- **`smelt-dialect` cannot depend on `smelt-db`**, so the walk is
  `smelt_dialect::settle_emissions(root, dialect, |node| -> Option<DataType>)` — the callback is
  smelt-runtime's, the walk and the arm matching are smelt-dialect's. Output threads into
  `PrintContext::settled_emissions: &'a [(TextRange, SettledEmission)]` beside
  `restructure_plans`, and into `unsupported_emissions` so an `otherwise = Unsupported` arm is
  refused at compile time rather than surfacing at the printer.
- **Printer on a lookup miss** settles with `CallFacts::unresolved(arity_from_cst)` — arity is
  readable from the source CST, class is not. Total, holds no type context, and lands on
  `otherwise`, which the spec's cost rule already constrains to be loud.

## Tests (red first)

`crates/smelt-types/tests/registry_coverage.rs` (synthetic `Signature`s, as the existing
`emission_at_*` tests do — no production row is conditional until phase 7):

- `operand_class_is_total_over_every_datatype` — every `DataType` variant classifies; `Null`
  and `Unknown` both give `Unresolved`, `Blob` gives `Binary`.
- `first_matching_arm_wins` — an earlier arm shadows a later one that also matches.
- `an_arity_guard_selects_on_call_arity` / `a_class_guard_selects_on_argument_class`.
- `an_unmatched_call_takes_the_otherwise_arm`.
- `a_conditional_without_an_otherwise_arm_fails_validation`.
- `a_conditional_naming_an_arity_the_signature_does_not_admit_fails_validation`.
- `a_conditional_naming_an_argument_index_beyond_arity_fails_validation`.
- `the_full_registry_builds` — extend the existing construction test over the new validation.

`crates/smelt-dialect/tests/operand_conditional.rs` (new):

- `settle_emissions_resolves_a_call_from_its_operand_types`.
- `settle_emissions_takes_otherwise_when_an_operand_type_is_unresolved`.
- `a_settled_verdict_reaches_the_printer_by_range` — printing uses the settled arm, not a
  re-lookup.

`crates/smelt-dialect/tests/emission_ownership.rs`:

- `printer_holds_no_type_context` — `printer.rs` names no `DataType`, `TypeContext`, or
  `OperandClass`.
- `printer_never_resolves_an_arm` — `printer.rs` contains no `Emission::Conditional` match and
  no call to `settle_at`/`settle_emissions`.

`crates/smelt-runtime/tests/dialect_seam.rs`:

- `integer_division_with_an_unresolvable_operand_is_refused_on_spark` — `a // b` over an
  unresolvable operand fails with `UnsupportedOnBackend`. Green today for the wholesale
  `Unsupported` reason; phase 7 keeps it green for the `otherwise`-arm reason.
- `no_compile_path_prints_with_an_unsettled_conditional` — structural: every `PrintContext`
  built in `compile.rs` reaches the printer only via `print_checked_for`, which settles.

## Tasks

1. Make the two `multi_backend.md` spec edits.
2. Add `OperandClass` + `OperandClass::of` to `smelt-types/src/signatures.rs`; write the totality
   test red-first.
3. Add `SettledEmission`, `ConditionalArm`, `Emission::Conditional`, `CallFacts`, and
   `Signature::settle_at`; extend registry validation (mandatory `otherwise`, arity admitted by
   the signature, argument index within arity) with the arm tests.
4. Move `printer.rs`, `restructure.rs`, and `emission_check.rs` off `emission_at` onto
   `settle_at`/`SettledEmission`; leave `emission_at` for the audit and coverage report.
5. Add `smelt_dialect::settle_emissions` + `PrintContext::settled_emissions`; give
   `unsupported_emissions` the settled verdicts so an `otherwise = Unsupported` refuses.
6. Thread settlement through `print_checked_for`/`print_checked` from the projection
   `TypeContext`; update every `PrintContext` literal.
7. Extend `emission_ownership` with the two structural gates; add the `dialect_seam` tests.
8. Fix `.claude/unknown-census.toml` line numbers shifted by the `signatures.rs` edits.

## Verification

- `bash .claude/scripts/verify-phase.sh` (must be ALL GREEN)
- `cargo test -p smelt-types --test registry_coverage --test unknown_census`
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional --test template_emission --test snapshots`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test dialect_audit` and `--test integration registry_consistency`

## Commit message

`feat(dialect): settle operand-conditional emission verdicts on the compile path`
