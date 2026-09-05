# Phase 1 — `ContractFrozenHorizonInvalid`: driving-source posture leg

**Outcome:** `docs/outcomes/20260904-decided-gap-residue/outcome.md`
**Advances:** Success criterion 1 (and its share of 6).

## Objective

`contract.frozen_horizon` is currently refused only for a non-partition grain; the spec also
refuses it when the model's **driving source** has a declared mutation profile other than
`append_only`, because the late-arrival probe's row-count comparison is blind under any other
posture. Add that validation leg in `smelt-logical` (single owner), surface it through the
existing `ContractFrozenHorizonInvalid` diagnostic in `smelt-db`, prove it reaches LSP-published
diagnostics, land a `examples/broken` fixture, and delete the `docs/TODO.md` bullet.

**Semantics settled here (no new decision needed):** the spec sentence refuses on "any other
*declared* mutation profile" (`incremental_models.md` §"The contract lattice"). An **undeclared**
profile is therefore admitted — nothing is declared to contradict, and the undeclared case is
already policed by `SourceMutationProfileViolated` at run time. Document that in the validator's
doc comment.

## Spec delta

Behaviour is already normative in `incremental_models.md` (§"The contract lattice", §"Contract
relaxations", and the diagnostics table at line 571) — no edit there. Two catalogues lag it:

1. `docs/specs/diagnostics.md` — the `ContractFrozenHorizonInvalid` catalogue row (~line 548)
   gains the third condition ("…or declared on a model whose driving source has a declared
   mutation profile other than `append_only`"); the derivation-sites bullet (~line 573) names the
   new validator alongside `validate_frozen_horizon`.
2. `docs-site/docs/guide/incremental-models.md` — the `frozen_horizon` bullet (~line 656) gains
   the same user-facing condition in one clause.

Both edits land **before** the code, per the spec-first rule.

## Tests (red-green)

- `smelt-logical` unit tests in `crates/smelt-logical/src/contract/frozen_horizon.rs`:
  - `posture_refuses_mutable_snapshot` — a `mutable_snapshot` driving source errors, naming the
    source and the posture.
  - `posture_refuses_change_feed` — same for `change_feed`.
  - `posture_admits_append_only` — `Ok(())`.
  - `posture_admits_undeclared_profile` — `None` profile is `Ok(())` (the documented reading).
- `crates/smelt-db/tests/contract_frozen_horizon_diagnostics.rs` (existing harness):
  - `frozen_horizon_on_mutable_snapshot_source_raises_diagnostic` — partition-grain model over a
    `mutation_profile: mutable_snapshot` source ⇒ exactly one `ContractFrozenHorizonInvalid`.
  - `frozen_horizon_on_append_only_source_is_clean` — the same model over the existing
    append-only `orders` source ⇒ no `ContractFrozenHorizonInvalid`.
  - `frozen_horizon_joined_dimension_posture_ignored` — a partition-grain model driven by the
    append-only source that *joins* a `mutable_snapshot` dimension is clean (only the driving
    relation is judged).
- `crates/smelt-lsp/tests/` (e2e harness pattern from `publish_tests.rs`/`e2e.rs`):
  - `lsp_publishes_contract_frozen_horizon_posture_diagnostic` — the published
    `textDocument/publishDiagnostics` entry for the fixture carries code slug
    `contract-frozen-horizon-invalid`.
- `crates/smelt-cli/tests/example_diagnostics.rs`:
  - `broken_contract_frozen_horizon_mutable_source` — the new `examples/broken` model is the only
    file raising `ContractFrozenHorizonInvalid` in that workspace.

## Tasks

1. Make the two spec/doc edits above (spec-first), keeping the timeless-oracle rule (no phase
   vocabulary).
2. Add `validate_frozen_horizon_posture(driving_source: &str, profile: Option<MutationProfile>)
   -> Result<(), String>` to `crates/smelt-logical/src/contract/frozen_horizon.rs`, beside
   `validate_frozen_horizon` (same module = same owner); export it from `smelt-logical`'s `lib.rs`
   next to the grain validator. Doc-comment the undeclared-profile reading and cite the spec §.
3. Write the four unit tests; watch them fail, then implement.
4. In `crates/smelt-db/src/lib.rs`'s existing `contract.frozen_horizon` block in
   `check_file_diagnostics`, resolve the model's driving relation from the FROM clause
   (`smelt_logical::analysis::source_bounds::from_clause_alias_sources`, first entry) and its
   `SourceInfo` via the existing `ref_source_info` helper — the same resolution shape the
   `cells[].deferral` check below it already uses — and accumulate the validator's `Err` under
   `DiagnosticCode::ContractFrozenHorizonInvalid` with the `ContractFrozenHorizonInvalid: {why}`
   message prefix. Emit nothing when the driving relation does not resolve to a declared source.
5. Add the three `smelt-db` diagnostics tests (a `mutable_snapshot` source yml fixture string
   beside `ORDERS_SOURCE`).
6. Add the fixture: `examples/broken/models/sources/contract_mutable_orders.yml` (a
   `mutation_profile: mutable_snapshot` source) plus
   `examples/broken/models/contract_frozen_horizon_mutable_source.sql` (partition grain +
   `contract.frozen_horizon`), and the `example_diagnostics` assertion.
7. Add the LSP e2e test.
8. Delete the "Frozen-horizon append-only gate" bullet from `docs/TODO.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_lattice_spec --quiet 2>&1 | tail -20`
- `cargo test -p smelt-db --test contract_frozen_horizon_diagnostics --quiet 2>&1 | tail -20`
- `cargo test -p smelt-lsp --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20`

## Commit message

`feat(contract): refuse frozen_horizon on a non-append-only driving source`
