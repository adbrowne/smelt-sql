# Phase 33 plan — override-ladder reach into the keyed-fold suppression consumer

## Objective

Close `incremental_models.md`'s last `(Open Question)` bullet by **deciding it, not deferring
it**: the override ladder's write-suppression dimension *does* reach the keyed-fold consumer,
while the structural first-build half is recorded as **unreachable by construction** on that
route. Advances success criteria 9 (standing gates stay green over a widened ladder) and 31's
close-out posture (no orphaned open question left in the spec).

## The decision this phase lands

Investigation for this plan established three facts:

1. `crates/smelt-runtime/src/cumulative.rs`'s `resolve_cumulative_write_suppression` calls
   `choice::resolve_write_suppression` and **never** `resolve_write_variant`. So a
   `maintenance.cells[].technique: suppress|unconditional` or `prefer:` pin addressing a keyed
   fold is silently ignored — a declared user intent that vanishes, against fail-loud
   discipline. **Decision: wire the ladder in.**
2. The structural first-build/steady-state half is a no-op on this route: both keyed call sites
   reach a merge only when the target table already exists (`run_windowed_keyed_maintenance`
   emits `emit_create_table_as` for a non-existent table; `cumulative.rs`'s non-windowed site
   resolves suppression inside its `else` branch of `!table_exists`). `Trigger::Backfill`/
   `ledger_catch_up` cannot be observed here — `keyed`'s classifier runs outside the
   `MaintenancePlan` machinery and has no `PlanCell`. **Decision: pass `Trigger::NewData` with
   `ledger_catch_up: false`, documented as a derivation from the route's structure, not an
   assumption.** This is also the honest answer to the bullet's "no real fixture derives a
   keyed-fold cell under a first-build trigger" — none can.
3. The residual clause (whether a future cost model needs region-level change-ratio statistics;
   `smelt bakeoff` measuring the suppression dimension) is a genuine undecided widening, not a
   divergence. **Decision: it moves to §Future Extensions.**

## Spec delta (first)

`docs/specs/incremental_models.md`:

- §Surface, the override-ladder bullet (~line 330): add one sentence — the ladder's
  write-suppression dimension (`suppress`/`unconditional`) is consulted by **every** write
  consumer that can suppress, the keyed fold included; on the keyed-fold route the structural
  first-build default never applies because a first build is a `CREATE TABLE … AS`, never a
  suppressible merge.
- §Known Divergences: **delete** the `Override-ladder reach (Open Question)` bullet (~2100–2105).
- §Future Extensions: add **"Cost-model input for the write-suppression dimension"** — ranking
  suppressed vs unconditional needs region-level change-ratio statistics from prior observed
  deltas, and `smelt bakeoff` measures technique-family cost only; not decided.

## Tests (red-green)

1. `keyed_fold_effective_override_matches_by_on_address` (`smelt-db`, beside
   `keyed_fold_write_pin`'s tests) — a `cells[]` entry addressing the driving source by `on:`
   alone resolves its `prefer`/`technique` for the whole-row keyed-fold cell; a non-matching
   `on:` resolves to the empty override.
2. `keyed_fold_unconditional_pin_reaches_the_emitted_merge` (`smelt-runtime`,
   `tests/technique_lowering.rs`) — a model whose P2/P3 proof admits suppression, pinned
   `technique: unconditional`, emits `emit_keyed_fold` (no `IS DISTINCT FROM` arm). RED today.
3. `keyed_fold_suppress_pin_over_refused_proof_refuses` — `technique: suppress` where the proof
   returned `Unconditional` fails the run loud with the `ChoiceRefusal` text, instead of
   silently emitting the unconditional arm.
4. `keyed_fold_prefer_unconditional_soft_biases_without_refusing` — `prefer: unconditional`
   flips the arm; `prefer: suppress` over a refused proof falls back silently.
5. `keyed_fold_unpinned_write_is_byte_identical` — regression: with no `maintenance:` block the
   emitted merge SQL is unchanged from today (guards against the trigger derivation altering
   the default).
6. `explain_show_sql_keyed_fold_honours_the_pin` (`smelt-runtime`, `tests/diagnostics.rs`) —
   `smelt explain --show-sql`'s keyed-fold preview renders the pinned arm, i.e. preview/live
   parity holds once the live path folds the ladder in.

## Tasks

1. Land the spec delta above (three edits) before touching code.
2. Add `keyed_fold_effective_override(metadata, driving_source) -> EffectiveOverride` to
   `crates/smelt-db/src/queries/maintenance.rs`, mirroring `keyed_fold_write_pin`'s whole-row
   (`group: "{*}"`, `on:`-only) addressing convention and delegating to
   `choice::effective_override`. Confirm `matching_cell`'s behaviour with an empty
   `group_columns` before relying on it.
3. Change `resolve_cumulative_write_suppression` to take `&EffectiveOverride` and return
   `Result<WriteSuppression, ChoiceRefusal>`, folding `resolve_write_variant(raw,
   &Trigger::NewData, false, overrides)`. Document fact 2 above in its doc comment.
4. Thread the override at both call sites (`cumulative.rs` ~508 and ~663) from
   `model.metadata` + `driving_source_name`, exactly as the `write_pin` lookup beside them
   already does; propagate the refusal as a run-failing `anyhow` error.
5. Update `crates/smelt-runtime/src/diagnostics.rs`'s keyed-fold preview (~649–695) to call the
   same resolution, and replace its "the live loop does not fold `resolve_write_variant`"
   comment with the parity statement that is now true.
6. Cross-check no other consumer of `resolve_write_suppression` is left un-laddered; if one is,
   name it in the summary rather than widening this phase.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test technique_lowering`
- `cargo test -p smelt-runtime --test diagnostics`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n 'Open Question' docs/specs/incremental_models.md` — expect no hits.

## Commit message

`feat(incremental): the override ladder's write-suppression dimension reaches the keyed fold`
