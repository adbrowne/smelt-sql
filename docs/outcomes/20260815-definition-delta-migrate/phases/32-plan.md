# Phase 32 — `retain_departed`: the contract-lattice point triple

## Objective

Land `retain_departed` as a complete single-owner contract-lattice triple in `smelt-logical`
— declaration schema, pure oracle transform, probe emitter — plus its
`ContractRetainDepartedInvalid` admissibility diagnostic, matching what
`incremental_models.md` §"The contract lattice" §Retention already specifies. This closes the
lattice-point half of the "Posture-derived key departure is unimplemented" divergence and
satisfies the contract-lattice point single-ownership invariant (`CLAUDE.md`), which today has
a spec-declared point with no owner at all. It advances success criteria 18/20 (decidable
residue closed in code and the owning spec swept).

Scope note: the *other* half of that divergence — the default point's anti-join delete leg in
the runtime reconcile path, which makes `retain_departed` observable by contrast — is real
runtime work with its own dispatch and statement-parity surface, and is now phase 32b. This
phase makes the point declarable, validated, oracle-transformable and probe-emittable; it does
not change what a run writes.

## Spec delta

No surface change — `incremental_models.md` lines 380–381, 398–406, 567 and §Retention already
specify the declaration, both forms, the admission rule, the quotient oracle and the anti-join
probe. Two spec edits only, both catalogue/status:

- `docs/specs/diagnostics.md` — add a `ContractRetainDepartedInvalid` | Error row to the
  contract-lattice block (beside `ContractDeferralInvalid`, line ~547), and update the
  "All four contract-lattice codes…" Known Divergences bullet (line ~558) to name five codes
  and state this one's derivation site (`smelt_logical::contract::retain_departed::validate`).
- `docs/specs/incremental_models.md` §Known Divergences — narrow the "Posture-derived key
  departure is unimplemented" bullet to its remaining residue only: the runtime still retains
  departed keys unconditionally because no anti-join delete leg exists (phase 32b). Drop the
  "no declaration parsing, oracle transform, probe emitter, or diagnostic" clause.

## Tests

Red-green, in this order:

1. `smelt-core` `config::tests::retain_departed_parses_both_forms` — `retain_departed: true`
   and `retain_departed: {tombstone: is_departed}` both deserialize under
   `deny_unknown_fields`; absent stays `None`.
2. `smelt-core` `metadata::tests::retain_departed_malformed_raises_contract_error` — a
   non-bool/non-mapping value raises `MetadataError::ContractRetainDepartedInvalid`, not the
   generic YAML parse error (mirrors the `frozen_horizon` pre-validation precedent).
3. `smelt-logical` `contract::retain_departed::tests::admitted_only_on_keyed_mutable_snapshot`
   — `validate` accepts keyed-shape + `mutable_snapshot`; rejects partition grain, rejects a
   keyed model over a non-mutable-snapshot source, each naming the offending posture.
4. `smelt-logical` `…::tests::tombstone_column_must_exist_in_output` — a tombstone column
   absent from the model's output columns is rejected and named.
5. `smelt-logical` `…::tests::oracle_exempts_departed_keys` — the quotient transform: keys
   present in the current snapshot compare strictly; a stored key absent from it is exempt;
   with a tombstone declared, an unmarked departed key is *not* exempt (it is a violation).
6. `smelt-logical` `…::tests::probe_emits_antijoin_over_stored_minus_current` — the emitted
   probe SQL is the reconcile anti-join (stored keys minus current-snapshot keys), returning
   the retained-departed key count and, where declared, the unmarked-tombstone count.
7. `smelt-db` `tests/contract_retain_departed_diagnostics.rs::invalid_posture_reports_diagnostic`
   — a workspace fixture declaring `retain_departed` on a partition-grain model surfaces
   `ContractRetainDepartedInvalid` through `check_file_diagnostics` (LSP path), with a range.
8. `smelt-logical` `contract::tests::render_label_includes_retain_departed` — `EffectiveContract`
   carries the point and `render_label` renders it, so `smelt explain` shows it.

## Tasks

1. Add `RetainDeparted` to `smelt_core::config` (untagged enum: `Bool(bool)` |
   `Tombstone { tombstone: String }`), wire `retain_departed` into `ContractConfig` with the
   same doc-comment convention as `frozen_horizon`/`deferral` (semantics live in
   `smelt-logical`; this struct is schema only).
2. Add `MetadataError::ContractRetainDepartedInvalid { why }` in `smelt-core/src/metadata.rs`
   and its `contract:` pre-validation arm; add the matching `DiagnosticCode` variant and the
   arm in `map_metadata_error_to_diagnostic` (`smelt-db/src/lib.rs` — the exhaustiveness gate
   makes this a compile error otherwise).
3. Create `crates/smelt-logical/src/contract/retain_departed.rs` holding the whole triple:
   `validate(...)` (posture admissibility + tombstone-column existence),
   the pure oracle transform (departed-key quotient over stored vs current key sets), and the
   probe emitter (the reconcile anti-join). Module doc comment states it is the single owner,
   per the `frozen_horizon`/`deferral` precedent.
4. Register the module in `contract/mod.rs`, update its "Landing status per lattice point"
   doc block, and extend `EffectiveContract` + `render_label` with the point.
5. Fold `validate` into `check_file_diagnostics` alongside the other two lattice validators.
6. Make the two spec edits from §Spec delta.
7. Add the `smelt-db` diagnostics fixture test and the `smelt-logical` unit tests above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical contract:: --quiet 2>&1 | tail -20`
- `cargo test -p smelt-db --test contract_retain_departed_diagnostics --quiet 2>&1 | tail -20`
- `cargo test -p smelt-db --test diagnostic_catalogue --quiet 2>&1 | tail -20` (enum → catalogue
  coverage gate; the new `DiagnosticCode` variant needs its `diagnostics.md` row)
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20` (the probe
  emitter is a `smelt-logical` emitter — confirm the no-authoring structural leg still passes)

## Commit message

`feat(contract): land the retain_departed lattice-point triple and its admissibility diagnostic`
