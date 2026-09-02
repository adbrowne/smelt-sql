# Phase 15 plan — Observed-delta consumption, read side

## Objective

Make `smelt run --since-upstream` read the recorded `_smelt_observed_delta` table live, so a
model-edge delta origin propagates the partitions its last run actually changed instead of the
whole declared `--landed` window (`plan_since_upstream_with_observed_deltas` exists and is
tested, but the CLI only ever hands it an empty lookup). Also settle the backward-resolution
clause of the same divergence as a decided non-goal. Advances success criterion 13 (its first
two clauses) and, through it, criterion 20's requirement that the bullet shrink in the spec.

## Spec delta (first — spec-first rule)

`docs/specs/incremental_models.md`:
- §"Backward resolution — what must exist": add a short paragraph stating that backward
  resolution does **not** consume observed deltas — it answers an existence question, which a
  change record cannot soundly narrow; a present-and-empty record means a past run changed
  nothing, not that the region is current with respect to inputs landed since. Currency is the
  reconciliation ledger's question (§"The frontier record (reconciliation ledger)",
  `smelt run --auto`). Narrowing on delta evidence would under-cover the resolved period,
  breaking `forward(backward(P)) ⊇ P`.
- §"Observed deltas on model edges": state that `--since-upstream` reads the record live for a
  model-address delta origin (the widen-never-narrow fallback applies when absent; a
  present-and-empty record propagates nothing), and that the read is DuckDB-scoped today —
  a target with no observed-delta storage reads back "absent" and falls back, never errors.
- §Known Divergences: rewrite the "Observed-delta consumption is partial" bullet, deleting the
  `--since-upstream`-doesn't-read-live clause and the backward-resolution clause (the latter
  now a stated non-goal above), leaving only the write-side clauses phase 16 owns.

`docs-site/docs/guide/` — the page documenting `smelt run --since-upstream` (locate it; likely
the maintenance/graph guide) gains a short "what a recorded delta narrows" note.

## Tests (red-green)

1. `smelt-state` / `smelt-runtime` unit — `read_observed_delta_decodes_both_columns`: a seeded
   row decodes into `ObservedDelta { changed_keys, partitions }`; no row decodes to `None`.
2. `smelt-runtime` — `read_observed_delta_changed_keys_shares_the_decoder`: the existing
   changed-keys reader, re-expressed over the new decoder, still returns `Some(&[])` vs `None`
   distinctly (regression guard on the refactor).
3. `smelt-runtime` — `load_observed_delta_lookup_keys_by_model_and_window`: the loader builds
   an `ObservedDeltaLookup` keyed exactly `(model, iso(start), iso(end))` for model-address
   deltas, and looks up nothing for a raw-source delta origin.
4. `smelt-runtime` — `load_observed_delta_lookup_is_empty_on_a_non_duckdb_target`: a non-DuckDB
   dialect returns an empty lookup, not an error (fallback, not failure).
5. `smelt-cli` e2e (`tests/since_upstream.rs`) —
   `recorded_observed_delta_narrows_the_dirty_set`: with a narrower partition set recorded for
   the upstream model over the declared window, the printed dirty set covers fewer days than
   the same invocation with no record.
6. `smelt-cli` e2e — `present_and_empty_observed_delta_propagates_nothing`: a recorded row with
   both arrays empty ⇒ "propagated nothing — no model has dirt to run".
7. `smelt-cli` e2e — `absent_observed_delta_falls_back_to_the_declared_window`: unchanged
   baseline behaviour (widen-never-narrow), asserted against the pre-phase dirty set.
8. `smelt-cli` e2e (`tests/include_upstreams.rs`) —
   `include_upstreams_ignores_a_recorded_empty_observed_delta`: the required-slices plan and
   build order are byte-identical with and without a present-and-empty record — the decided
   non-goal, pinned by a test so a later phase cannot "fix" it silently.

## Tasks

1. Land the spec delta above (and the docs-site note) before touching code.
2. In `crates/smelt-runtime/src/maintenance_driver.rs`, factor one decoder
   `read_observed_delta(backend, schema, model, window_start, window_end) ->
   Result<Option<ObservedDelta>, BackendError>` that decodes **both** `VARCHAR[]` columns; re-
   express `read_observed_delta_changed_keys` over it so there is one decode site, keeping its
   DuckDB-only read-side fallback (`Ok(None)`) unchanged.
3. Add `load_observed_delta_lookup(backend, schema, deltas: &[SourceDelta], model_names: &BTreeSet<String>)
   -> Result<ObservedDeltaLookup>`: one lookup per delta whose origin is a maintained model,
   keyed with `ordinal_to_iso` exactly as `plan_since_upstream_with_observed_deltas` keys it;
   raw-source origins are skipped; a non-DuckDB backend yields an empty map.
4. In `crates/smelt-cli/src/commands/run.rs`'s `--since-upstream` path, create the target
   backend (`CliBackendFactory`, the same construction the run below already uses) and resolve
   the target schema **before** planning, load the lookup, and call
   `plan_since_upstream_with_observed_deltas` instead of `plan_since_upstream`. A backend that
   cannot be created here must fail loud, not silently fall back to the empty lookup — the run
   itself needs that same backend moments later.
5. Update `plan_since_upstream`'s doc comment: its "live wiring … is CLI/backend-read work
   outside this phase" note is now stale — it stays the pure/empty-lookup wrapper the testkit
   and conformance harness use, and the CLI is no longer a caller.
6. Leave `resolve_build_plan` unchanged; add the doc comment recording the non-goal and citing
   the spec paragraph, so the omission reads as decided rather than unfinished.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test observed_delta`
- `cargo test -p smelt-cli --features duckdb --test since_upstream --test include_upstreams`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` (the DAG families
  call `plan_since_upstream`; the wrapper must keep its empty-lookup semantics)
- `/smelt:validate incremental_models` on the edited sections only (spot-check the rewritten
  divergence bullet matches the shipped behaviour)

## Commit message

`feat(propagation): read the recorded observed delta live in --since-upstream`
