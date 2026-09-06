# Phase 3 summary — the succession plan model and its derivation

## Shipped

- `Grain::Succession { key_cols, clock_col }`, `Technique::SuccessionPatch`,
  `StateStructure::TombstoneLedger` (`crates/smelt-logical/src/maintenance/{mod,availability}.rs`).
  `required_state_structure(SuccessionPatch) == Some(TombstoneLedger)`;
  `TombstoneLedger` realisable on DuckDB only; `recompute_equivalent` downgrades
  a succession cell straight to `DeleteInsert` (never `PerGroupRecompute`).
- `Refusal::SuccessionNotRecognized { reason }` + `succession_refused_plan`
  (`maintenance/mod.rs`); `refusal_code` maps it to `None` for now (phase 3a
  lands the eleven codes).
- The pure deriver `crates/smelt-logical/src/maintenance/succession.rs`
  (new file): `derive_succession_plan(verdict, table) -> SuccessionDerivation
  { output, plan }` — one `SuccessionPatch` cell (`Trigger::NewData`,
  `Corner::FoldDelta`, `PartitionLocal::Yes`, skeleton = `k ∪ {t}`) on
  `Recognized`, the refusal plan on `NotSuccession`.
- `crates/smelt-db/src/queries/maintenance.rs`: `build_succession_context`
  (resolves the driving source from the FROM clause, looks it up in the
  `(bare name, SourceInfo)` refs list — mirrors `build_key_recurrences`);
  `derive_model_maintenance_plan`'s `resolved_grain()`-is-`None` branch now
  classifies and derives the succession cell/refusal instead of bailing to
  `None`. Both `derive_model_maintenance_plan` and `_with_edges` gained a
  `source_refs: &[(String, Option<SourceInfo>)]` parameter, threaded through
  `smelt-runtime`'s availability seam (`derive_resolved`/
  `derive_resolved_with_edges`) and every one of its ~15 call sites — the
  real value at the two `smelt-db` production sites
  (`maintenance_plan_diagnostics`, `maintenance_refs.rs`'s Salsa query) and
  `propagation.rs` (already had `source_refs` in scope); `&[]` everywhere
  else (runtime execution-path resolvers — full succession dispatch is
  phase 5's scope, and `&[]` fails closed to the classifier's refusal, never
  a panic).
- `crates/smelt-logical/src/contract/mod.rs`: new `GrainLabel` enum
  (partition/key/key_per_partition/succession) + `Display` +
  `From<smelt_core::config::Grain>`. `validate_frozen_horizon`/
  `retain_departed::validate` retyped from `smelt_core::config::Grain` to
  `GrainLabel`. `crates/smelt-db/src/file_check.rs`'s `model_grain_label`
  resolves declared → fact-derived → succession-classified → `Key` fallback,
  replacing both `metadata.grain.unwrap_or(Grain::Key)` call sites; the
  `deferral` admissibility check's `has_clock` now also admits
  `GrainLabel::Succession` (a succession model's clock is classifier-derived,
  never a declared `timeseries:` block).
- Tests: 4 new `smelt-db` unit tests (`derive_model_maintenance_plan`
  succession coverage), 4 new `smelt-logical` deriver tests
  (`succession.rs`), 4 new `smelt-logical` availability tests, 3 new pure
  contract-validator tests, 3 new `smelt-db` integration tests
  (`tests/contract_succession_grain.rs`, real Salsa-DB harness).

## Decisions

- `SuccessionContext::source_name` carries the classifier's own comparison
  spelling (`"sources.customer_changes"`, matching
  `analysis::walk::InputItem::Table::name` verbatim) so rule 1's FROM-target
  match works; the derived `Trigger::NewData{source}` strips the `sources.`
  segment before storing, so a succession cell addresses the same way every
  other `Trigger::NewData` does (`SourceFacts::name`, bare).
- `technique_rank` (property-diff direction ladder,
  `analysis/diff.rs`) ranks `SuccessionPatch` alongside `KeyedFold` — a
  placeholder since no real technique transition into/out of succession
  exists yet to pin the rank against.

## For the next planner

- **Large-file ratchet is red**, caused entirely by this phase's own
  mechanical diff: the new `source_refs` parameter fanned out across ~20
  call sites, growing several files by 1-3 lines each (plus the two
  genuinely new files, `succession.rs` and `contract_succession_grain.rs`,
  both well under the cap). `bash .claude/scripts/hardening-budget.sh` is
  clean; `large-file-check.sh`/`large_file_ratchet::gate_passes_on_committed_tree`
  is not. Same situation phase 2b hit and the same resolution applies —
  `docs/outcome_loop.md` §"The large-file shrink step" is the dedicated
  non-blocking follow-up; do not hand-fix here.
- **Pre-existing, unrelated failure confirmed via `git log`**:
  `smelt-logical`'s `join_context_reach::every_production_join_context_new_is_tagged`
  fails on an untagged `JoinContext::new()` at
  `analysis/walk/tests.rs:472`, in a file this phase never touched (last
  commit `5107c66b`, predates this phase). Not fixed here — out of scope,
  and the same "record, don't force" instruction applies.
- Phase 3a (diagnostics) has its producer now: `Refusal::SuccessionNotRecognized`
  and `NotSuccessionReason`'s ten variants are ready to map onto the eleven
  `Succession*` `DiagnosticCode`s.
- Phase 4 (emitters) needs a real `SuccessionContext`-carrying `source_refs`
  wired through the `smelt-runtime` execution paths that currently pass
  `&[]` — those calls are the ones that will actually need to dispatch a
  live `SuccessionPatch` cell.
- `ALL_TECHNIQUES` in `smelt-runtime/src/diagnostics.rs` (the "never partial
  by omission" technique-preview registry) does not yet include
  `Technique::SuccessionPatch` — `build_technique_statements` refuses it
  cleanly (`Err`, never a panic) but the registry itself stayed a
  5-technique array. Left alone since the preview UI is out of this
  outcome's explicit scope; flagging in case a later phase's criterion
  needs it.

## Gates

- `cargo check --workspace --tests` — clean.
- `bash .claude/scripts/clippy-gate.sh` (both feature sets) — clean.
- `cargo fmt --all -- --check` — clean.
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage` — 16 + 8 passed.
- `cargo test -p smelt-logical --lib succession` / `--lib contract::` — all passed (post-fix).
- `cargo test -p smelt-db --lib queries::maintenance` — 25 passed.
- `cargo test -p smelt-db --test maintenance_diagnostics --test contract_succession_grain` — 38 + 3 passed.
- `cargo test -p smelt-runtime --test availability_seam --test execute_parity` — 6 + 4 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 122 passed, 1 ignored.
- `bash .claude/scripts/hardening-budget.sh` — clean, baseline unedited.
- `cargo test --workspace --no-fail-fast --quiet` — 385 test-binary results green;
  exactly 2 red (`large_file_ratchet`, `join_context_reach`), both discussed above.
