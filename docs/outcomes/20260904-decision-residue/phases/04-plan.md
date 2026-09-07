# Phase 4 plan — `KeyedRecurrenceDeclarationMismatch` + order-independent key sets

## Objective

Make key-grain rule 16 real: where a key-recurrence bound is statically derivable, that value is
authoritative and a disagreeing declared `key_recurrence` is refused at plan time with
`KeyedRecurrenceDeclarationMismatch` naming both values; an agreeing declaration is accepted and
still admits the derived (proof-backed, unchecked) slice; an underivable model still takes the
declared checked route. Also close rule 16's second clause — every key-set comparison in locality
reasoning is over sets, never lists. Advances success criterion 4 (and its share of 8/9).

## Spec delta

Deletions only — rule 16 is already specified.

- `docs/specs/diagnostics.md`: the `KeyedRecurrenceDeclarationMismatch` catalogue row loses its
  "Specified, unimplemented — see §Known divergences." clause; delete the matching Known-divergence
  bullet (`§Known divergences`, the `is specified and unimplemented` line).
- `docs/specs/incremental_shapes.md` §Known Divergences → "The key grain" → "Locality machinery
  gaps": delete the sentence "Key-grain rule 16 … is decided but unimplemented: no
  `KeyedRecurrenceDeclarationMismatch` is emitted today. Scheduled: …". The other clauses of that
  bullet (explain surface, `IN (SELECT DISTINCT …)`, granularity determination) stay.

## Tests

Red-green, in this order:

1. `smelt-logical` `maintenance::locality` unit — `route3_declared_recurrence_disagreeing_with_derived_is_refused`:
   SQL with a derivable lookback bound of 3 days + declared `key_recurrence.window` of 7 days over
   the matching key ⇒ `Err(LocalityRefusal::RecurrenceDeclarationMismatch)` whose message names both
   `3 days` and `7 days` and the source.
2. `route3_declared_recurrence_agreeing_with_derived_admits_the_derived_slice`: same bound and
   declaration both 3 days ⇒ `Ok(LocalitySlice::Window { recurrence_bounded: true, .. })` (derived
   shape, not `RecurrenceBounded`) — the declaration is a check, not the route.
3. `route3_declared_recurrence_over_a_different_key_does_not_mismatch`: derivable bound + a
   declaration whose `key` is not the model's `unique_key` ⇒ admits the derived slice, no mismatch
   (a bound over another key asserts nothing about this one).
4. `route3_declared_recurrence_on_an_underivable_model_still_takes_the_declared_route`: no
   derivable bound ⇒ `Ok(LocalitySlice::RecurrenceBounded { .. })` unchanged.
5. `key_set_comparisons_are_order_independent` (locality): permute both `unique_key` and
   `key_recurrence.key` column order across all four orderings and assert byte-identical verdicts
   for the declared-route admit, the FD route-2 admit, and the key-set-mismatch refusal.
6. `smelt-logical` `maintenance::propagate` unit — `push_keyed_dirt_is_key_order_independent`:
   two `KeyedDirt` records from the same `from` whose `keys` differ only in column order collapse to
   one entry.
7. `smelt-db` test (new `crates/smelt-db/tests/keyed_recurrence_declaration_mismatch.rs`):
   `file_diagnostics()` over a staged workspace (model + source `.yml` with the disagreeing
   `key_recurrence`) yields exactly one `DiagnosticCode::KeyedRecurrenceDeclarationMismatch`, Error
   severity, message naming both values; the agreeing variant yields none.
8. `smelt-lsp` parity — extend the existing diagnostic-slug coverage assertion so the new
   `DbCode` maps to `keyed-recurrence-declaration-mismatch` (no unmapped code).
9. `smelt-runtime` — `locality_route3_recurrence_check.rs` gains
   `disagreeing_declaration_refuses_the_run`: the run path (`cumulative.rs`'s `establish_locality`
   bail) fails with the mismatch message rather than silently ignoring the declaration.

## Tasks

1. Write tests 1–6 red.
2. `locality.rs`: add `LocalityRefusal::RecurrenceDeclarationMismatch { source, key, derived, declared }`
   with its own `message()` arm (prefix `KeyedRecurrenceDeclarationMismatch:`, both values rendered
   as intervals, naming the derived value as authoritative and the fix as correcting/removing the
   declaration).
3. `locality.rs` route 3: before returning the statically-derived `Window`, if a declared
   `key_recurrence` exists whose `key` set-equals the model's `unique_key` (case-insensitive,
   order-independent), compare `before` against `Seconds(kr.window.seconds)` — equal ⇒ admit the
   derived slice; unequal ⇒ return the new refusal. Non-matching key ⇒ admit derived unchanged.
4. Audit the remaining key comparisons in `locality.rs` (route-3 declared-key match; route 2's
   `functional_dependency_verdict_over_vector` / `key_derived` membership) and make the
   case-insensitivity/set-ness explicit where it is only incidental; add doc-comment notes.
5. `propagate.rs`: make `push_keyed_dirt`'s duplicate check compare `keys` as a set (canonical
   lowercased `BTreeSet`) rather than as an ordered `Vec`; update its doc comment.
6. `smelt-logical/src/maintenance/mod.rs`: add `Refusal::KeyedRecurrenceDeclarationMismatch { message }`
   and a `recurrence_mismatch_plan(message)` constructor alongside `locality_refused_plan`.
7. `smelt-db/src/queries/maintenance.rs`: route the new `LocalityRefusal` variant to
   `recurrence_mismatch_plan`; add the matching `MaintenanceRefusal` variant and its projection.
8. `smelt-db/src/diagnostics_types.rs` + `lib.rs`: add
   `DiagnosticCode::KeyedRecurrenceDeclarationMismatch` and map the refusal onto it (Error).
9. `smelt-lsp/src/backend.rs`: add the `keyed-recurrence-declaration-mismatch` slug.
10. Fix any newly non-exhaustive `Refusal` / `MaintenanceRefusal` matches surfaced by the compiler.
11. Write tests 7–9 and make them green.
12. Apply the spec deletions above.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, full `cargo test`,
  `example_diagnostics`)
- `cargo test -p smelt-logical --lib maintenance::locality maintenance::propagate`
- `cargo test -p smelt-db --test keyed_recurrence_declaration_mismatch`
- `cargo test -p smelt-runtime --test locality_route3_recurrence_check`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — must stay green with
  its count **unchanged**: this phase edits a key-grain bullet, not a partition-grain one
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb composed`

## Commit message

`feat(incremental): refuse a declared key_recurrence that disagrees with the derived bound`
