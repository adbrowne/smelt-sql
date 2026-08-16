# Phase 8 plan — persisted per-source watermark

## Objective

Persist the per-source watermark pinned in `run_state.md` §"Per-source watermark" and consume it
on the read side, so `smelt run --since-upstream --source <addr>` needs no `--landed` for a
source a prior run already propagated through. Advances success criterion 4 (and criterion 6's
"explicit-only" divergence bullet); the `state.mode`-aware residency and refuse-loudly leg come
free from routing every write through `FileStore`.

## Spec delta

`docs/specs/incremental_models.md` §Surface (the `smelt run --since-upstream` bullet and the
"**Run flags.**" paragraph) — two clarifications only, no new semantics:

1. **`--landed` pairing spelling.** State the rule the "optional per source" phrasing implies:
   a `--landed` value is either bare `<start>..<end>`, paired positionally with the `--source`
   at the same index (requires equal counts — today's rule, unchanged), or address-qualified
   `<address>=<start>..<end>`, pairing by address with no positional constraint. Mixing the two
   spellings in one invocation is refused. A `--source` with no paired `--landed` resolves from
   its watermark.
2. **The refusal is a named run error.** A `--source` with neither a paired `--landed` nor a
   persisted watermark makes the run fail with an error naming that source and the missing
   watermark (and pointing at `--landed`) — not a per-source skip that quietly under-propagates.

## Tests

- `smelt-state` `landed_deltas.rs`: `watermark_field_roundtrips_and_never_regresses` — `SourceLanding::watermark` serialises, `advance_watermark` is monotone (an earlier `to` is a no-op).
- `smelt-state` `file_store.rs` (or `tests/landed_deltas.rs`): `landed_deltas_file_without_watermark_still_loads` — a pre-existing `landed_deltas.json` with no `watermark` key deserialises to `None`.
- `smelt-state` `file_store.rs`: `stateless_mode_persists_no_watermark` — under `StateMode::Stateless`, saving an advanced store leaves no file and a reload yields no watermark.
- `smelt-runtime` `watermark` unit: `watermark_advances_only_when_every_consumer_completed` — a source all of whose consumers succeeded advances to the run window's end; one consumer missing (selective run) or failed → no advance.
- `smelt-runtime` `propagation` unit: `missing_landed_resolves_watermark_to_now_span` — a source with no `--landed` and watermark `W` yields delta `[W, now)`; an explicit `--landed` for the same source overrides it.
- `smelt-runtime` `propagation` unit: `qualified_landed_pairs_by_address` — `<addr>=<start>..<end>` pairs by address; mixed spellings and unequal bare counts are named errors.
- `smelt-runtime` `propagation` unit: `source_without_landed_or_watermark_is_named_error` — the error text names the source and the missing watermark.
- `smelt-cli` `tests/since_upstream.rs` e2e: `full_run_then_since_upstream_without_landed_propagates` — a full `smelt run` over a window advances the source's watermark; a following `--since-upstream --source <src>` with **no** `--landed` prints a non-empty dirty set and runs the downstream.
- `smelt-cli` `tests/since_upstream.rs` e2e: `since_upstream_without_landed_or_watermark_refuses` — same project with no prior run fails, the message naming the source and the missing watermark.

## Tasks

1. `smelt-state::landed_deltas`: add `SourceLanding::watermark: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`) plus `LandedDeltaStore::watermark(&self, source)` and `advance_watermark(&mut self, source, to)` (monotone; string compare on the source's own ISO axis, same convention `intervals.rs` already uses).
2. New pure `smelt-runtime` module `watermark.rs`: `watermark_advances(consumers_by_source, completed_models, window_end, store) -> Vec<(String, String)>` — advance a source iff its whole consumer set is in `completed_models` and `window_end` is beyond the recorded watermark.
3. Derive `consumers_by_source` from the SAME forward graph propagation already builds (`propagation::build_forward_graph` over `all_models` + `discover_source_infos`) — never a second ref scan. A graph-build failure means coverage is unprovable: no advance, `tracing::warn!` naming the source set, never a speculative advance.
4. Call it at `execute_project`'s single success path (`execute.rs`, right after `manifest.completed_at = Some(...)` / `save_run`), inside the existing `state_io_lock` critical section and through `FileStore` — `state.mode: stateless` then writes nothing with no extra branch. Window end = `request.end`; absent (`None`) window → no advance.
5. `propagation`: replace `pair_source_deltas`'s count check with the two-spelling rule from the spec delta, and add `pair_source_deltas_with_watermarks(sources, landed, store, now) -> Result<Vec<SourceDelta>>` resolving an unpaired source to `[watermark, now)` and refusing (named error) when no watermark exists. Keep `pair_source_deltas` as the no-watermark delegating wrapper so `explain`/existing callers are untouched.
6. `smelt-cli` `run.rs::run_since_upstream`: load the landed-delta store via `FileStore` and call the watermark-aware pairing with `now` = `Utc::now()` formatted on the day axis; everything downstream (observed-delta read, keyed seeds, `plan_since_upstream_live`) is unchanged.
7. Land the two §Surface spec edits (task order: spec first, per the spec-first rule) and narrow the `incremental_models.md` Known Divergences bullets at ~L1949 and ~L2030 to the residue that actually remains (the watermark is now written and read; `--landed` stays required for a source no completed run has covered).
8. Record in the phase summary that a `--since-upstream` sweep — one `execute_project` per model — is by construction selective and therefore never advances a watermark; only a run completing every consumer does. Spec-conformant (§Design "per source, not per `(source, consumer)`"), but the next planner should know it.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --test landed_deltas --quiet`
- `cargo test -p smelt-runtime --test since_upstream_propagation --quiet`
- `cargo test -p smelt-cli --test since_upstream --quiet`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/run_state.md` — no matches

## Commit message

`feat(incremental): persist and consume the per-source watermark so --landed is optional`
