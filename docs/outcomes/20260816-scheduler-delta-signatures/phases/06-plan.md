# Phase 6 plan — live observed-delta consumption

## Objective

Make `--since-upstream` read the recorded `_smelt_observed_delta` table off the backend instead
of falling back to an always-empty lookup, so a delta origin naming a locality-admitted composed
model propagates its *observed* partitions rather than the operator's declared window. This lands
success criterion 3 (live observed-delta consumption, including the settle-bound × observed-delta
"delta empty" leg) and narrows two Known Divergences bullets toward criterion 6.

The pure consumer already exists and is tested (`propagation::plan_since_upstream_with_observed_deltas`);
this phase supplies the live lookup it has always taken as a parameter, and wires the CLI to it.

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences — narrow, do not add surface (§Surface
already claims the behaviour at "refined live by the recorded observed-delta table where a record
exists"):

- **"Observed-delta consumption is partial"** — drop `--since-upstream doesn't read the recorded
  delta table live` and `the settle-bound × observed-delta composition has no live "delta empty"
  leg`. Keep the real residue: backward resolution consumes none; the keyed-fold and
  staged-candidate write families record nothing.
- **"The scheduler does not yet consume delta signatures end to end"** — the parenthetical "live
  resolution (reading the actually-changed key values off the backend, **and consuming the
  recorded observed-delta table for `--since-upstream` rather than trusting the command line**)"
  loses its observed-delta clause; the key-value half stays (row 7).

## Tests

Red-green, in this order:

1. `read_observed_delta_distinguishes_absent_from_present_empty`
   (`crates/smelt-runtime/tests/observed_delta.rs`) — real DuckDB: after the real
   `generate_observed_delta_upsert_sql` records a fully-suppressed run, the new
   `maintenance_driver::read_observed_delta` returns `Some(ObservedDelta)` that `is_empty()`;
   an unrecorded window returns `None`. Both `changed_keys` and `partitions` decode.
2. `observed_delta_keys_to_read_lists_only_locality_admitted_origins`
   (`crates/smelt-runtime/tests/since_upstream_propagation.rs`, pure) — for a delta on the
   composed `silver.events_deduped`-shaped origin the returned key is
   `(model, iso_start, iso_end)`; a raw `sources.*` origin and a bare `grain: partition` model
   origin contribute no key at all.
3. `live_observed_lookup_suppresses_downstream_for_present_and_empty_delta`
   (`since_upstream_propagation.rs`, real DuckDB) — the recorded-empty row is read through the
   new live resolver (not a hand-built map) and the resulting plan schedules zero downstream
   regions: the live "delta empty" leg.
4. `live_observed_lookup_falls_back_to_declared_window_when_absent` — same fixture, nothing
   recorded: the downstream is scheduled over the full declared `--landed` window (widen-never-
   narrow; the live read must not turn absent into empty).
5. `since_upstream_consumes_recorded_empty_delta` (`crates/smelt-cli/tests/since_upstream.rs`,
   real `smelt` binary over `stage_composed_origin_workspace`) — with a present-and-empty row
   recorded for the origin's exact window, `smelt run --since-upstream --source <model> --landed
   <w>` propagates nothing and runs zero models; the existing
   `composed_model_address_landed_delta_propagates` (no recorded row) still passes unchanged.
   Prefer recording the row via a real upstream run if that fixture's write family records one;
   otherwise record it with the real `generate_observed_delta_upsert_sql` against the target
   DuckDB file (the write-family recording gap stays in the narrowed divergence bullet).

## Tasks

1. `maintenance_driver::read_observed_delta(backend, schema, model, start, end) ->
   Result<Option<ObservedDelta>, BackendError>` — decode both `changed_keys` and `partitions`
   from `generate_observed_delta_select_sql`; zero rows → `None`; non-DuckDB → `None` (a legal
   fallback trigger, same rationale as the existing read). Reduce
   `read_observed_delta_changed_keys` to a delegating wrapper over it.
2. `propagation::observed_delta_keys_to_read(models, source_infos, deltas) ->
   Result<Vec<ObservedDeltaKey>>` — pure; returns exactly the keys
   `plan_since_upstream_with_observed_deltas` would consult, derived from the SAME
   `derive_clamp_and_locality` `key_locality_slice` the planner uses (no second eligibility rule).
   Refactor `plan_since_upstream_full`'s lookup site to consult that one owner.
3. New `smelt-runtime` live resolver (`propagation_live.rs`, or an async fn beside the planner):
   `resolve_observed_delta_lookup(backend, schema, keys) -> Result<ObservedDeltaLookup>` — one
   `read_observed_delta` per key, absent keys simply omitted (absent ≠ empty).
4. `smelt-cli/src/commands/run.rs::run_since_upstream` — create the backend via the existing
   `CliBackendFactory` + `config.targets[&args.target]` before planning, call
   `observed_delta_keys_to_read` → `resolve_observed_delta_lookup`, and switch the planner call
   to `plan_since_upstream_with_observed_deltas`. Runs under `--dry-run` too (decision log,
   phase 6 planning). A backend-creation failure is a named error, never a silent fallback to
   the empty lookup.
5. Land the two divergence-bullet edits from §Spec delta.

## Verification

- `cargo test -p smelt-runtime --test since_upstream_propagation --test observed_delta`
- `cargo test -p smelt-cli --test since_upstream` (needs `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`)
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(incremental): --since-upstream reads the recorded observed-delta table live`
