# Phase 27d summary — the `write:` pin selects the keyed-fold write mechanism

**Shipped:**
- `choice::resolve_keyed_write_mechanism` (`crates/smelt-logical/src/maintenance/choice.rs`) now
  takes an `Option<&'static WritePattern>` write pin and returns
  `Result<Option<KeyedWriteMechanism>, ChoiceRefusal>`. `keyed`/`keyed_conditional` pin `MERGE`
  (fail-closed refusal on a merge-less backend, defence-in-depth behind the registry's own
  `WriteCapability::Merge` gate); `staged_candidate` pins the staged conditional shape even on a
  `MERGE`-capable backend, and refuses (never silently substitutes `MERGE`) over an
  `Unconditional` suppression verdict, which the staged emitter has no shape for. Any other pin
  (e.g. `region`) falls through to the pre-existing unpinned default, unchanged. The no-pin path
  is byte-identical to before (`default_keyed_write_mechanism`, extracted verbatim).
- `emit::keyed_fold_candidate_select(table, key, folds, delta_sql, dialect) -> String`
  (`crates/smelt-logical/src/maintenance/emit.rs`): the post-fold candidate rows a keyed fold's
  staged-candidate mechanism needs — `LEFT JOIN` of the delta to the target on the key, each fold
  column resolving to its own combine expression for a matched key and to the raw delta value
  (via a `CASE WHEN target.<key0> IS NULL` guard) for a delta-only key, mirroring
  `emit_keyed_fold`'s `WHEN MATCHED`/`WHEN NOT MATCHED` split.
- 11 new/updated unit tests in `choice.rs`'s `keyed_write_mechanism_tests` and 4 new tests in
  `emit.rs`'s new `keyed_fold_candidate_select_tests` (one exercising the candidate select as
  `emit_staged_candidate_conditional`'s `candidate_select` end to end).
- Spec delta: `docs/specs/incremental_models.md` §"Per-cell write addressing" gains a
  "Within-family mechanism pins" paragraph. `docs-site/docs/reference/smelt-yml.md`'s
  `cells[].write` section gains the same clause for users.

**Decisions:**
- No `trigger` context is available inside `resolve_keyed_write_mechanism` (it only sees a
  suppression verdict, a backend capability bool, and an optional pin) — refusals use a fixed
  `"keyed-fold cell"` trigger label. 27g's live caller has real trigger context and may choose to
  rebuild/relabel the refusal it propagates; this function's own contract doesn't promise a
  specific trigger string.
- Kept `default_keyed_write_mechanism` as a private helper rather than inlining, so the no-pin
  path stays trivially byte-identical to the pre-phase function and is easy to diff against.

**For the next planner:**
- This phase is intentionally **not wired into any live path** — `resolve_keyed_write_mechanism`
  has no production call site yet (confirmed via `rg`); only its own tests call it. Phase 27g is
  where `cumulative.rs`'s live keyed-fold write path threads the matching `write:` pin through to
  this function and to `keyed_fold_candidate_select`, extends `statement_parity`, and narrows the
  `incremental_models.md` Known Divergences bullet — none of that happened here per the plan's
  explicit scope boundary ("Do not touch the §Known Divergences bullet... 27g narrows it").
- `keyed_fold_candidate_select`'s `dialect` parameter is currently unused (prefixed `_dialect`),
  matching `emit_staged_candidate_conditional`'s own unused-dialect convention — if a future
  dialect needs different NULL-handling syntax for the `CASE WHEN`/`COALESCE` shapes, that's the
  seam to extend.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --lib maintenance::choice` — 46 passed.
- `cargo test -p smelt-logical --lib maintenance::emit` — 42 passed.
- `cargo test -p smelt-logical --test emit_statements` — 59 passed.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — 32 passed,
  no behaviour change (as expected — live path untouched until 27g).
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics` — 5 passed.
