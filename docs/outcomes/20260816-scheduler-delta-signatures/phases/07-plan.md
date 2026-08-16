# Phase 7 plan — live keyed-seed resolution

## Objective

Fill the keyed-seed channel `plan_since_upstream_with_keyed_seeds` has accepted since phase 3
but nobody has ever populated: at `--since-upstream` plan time, resolve each keyed origin's
affected key **values** off the group-grain fingerprint sidecar (the same diff the run-time
mechanism uses), unioned across that origin's consumers, and thread them into the plan so
`keyed_restrictions_from_plan` produces a real non-empty restriction end to end. Advances
success criterion 2 ("value-level discovery feeds the scheduler, not only the run-time
mechanism") and completes criterion 3's key-value half.

## Spec delta

`docs/specs/incremental_models.md`, two edits (spec-first — make them before code):

1. §"Keyed dirt-sets and the narrowed refusal" — the paragraph already says the caller resolves
   seeds once via the group-grain sidecar diff. Add one sentence pinning the composition rule
   the resolution itself obeys: the sidecar partition identity is per `(upstream, consumer)`
   (each consumer hashes its own digest projection), so an upstream's seed is the **union** of
   its consumers' diffs — never one consumer's diff taken as the whole, never an intersection.
   Same widen-never-narrow reasoning as §"Restrictions compose by union".
2. §Known Divergences, the "scheduler does not yet consume delta signatures end to end" bullet
   — drop the "live resolution (reading the actually-changed key values off the backend) is
   still the run-time mechanism's own job, not yet wired live into propagation" clause. Surviving
   residue in that bullet (the watermark, the uncovered-input widen) is untouched.

## Tests

Red-green, in this order:

1. `keyed_seed_diffs_to_read_names_each_consumer_of_a_keyed_origin`
   (`crates/smelt-runtime/tests/since_upstream_propagation.rs`) — a clockless `keyed upsert`
   model origin with two `grain: partition` consumers yields one descriptor per
   `(upstream, consumer)`, each carrying the upstream's admitted key columns, that consumer's
   own digest columns, and that consumer's own table name.
2. `keyed_seed_diffs_to_read_skips_a_raw_source_origin` (same file) — a declared-source
   `--source` origin yields no descriptor: no fabricated keyed seed for a node with no
   key-addressed cell.
3. `fold_keyed_seed_values_unions_across_consumers` (same file, pure) — two consumers of one
   upstream fold to one sorted/deduped `KeyValues::Resolved`; a diff that found nothing stays
   `Resolved(vec![])`, never `Unresolved` (empty-and-resolved ≠ unresolved).
4. `unsupported_dialect_diff_yields_an_unresolved_seed` (same file, pure) — the classifier that
   turns a diff result into `KeyValues` maps a `BackendError::unsupported` (non-DuckDB target)
   to `KeyValues::Unresolved` naming the dialect, so the edge widens at dispatch per
   §"Unresolved seeds" rather than failing the run or seeding an empty set.
5. `resolve_keyed_seeds_reads_changed_keys_off_the_sidecar`
   (`crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`, real-DuckDB e2e module) —
   after a full build has populated the upstream's sidecar, mutating exactly one upstream key
   makes `resolve_keyed_seeds` return `Resolved([that key])` and nothing else.
6. `since_upstream_produces_a_non_empty_keyed_restriction`
   (`crates/smelt-cli/tests/since_upstream.rs`) — end to end over a staged keyed-upsert →
   `grain: partition` project: after a build, mutate one upstream row, run
   `--since-upstream --source smelt.models.<upstream> --landed <window>`, assert the dirty-set
   report renders the keyed component with the resolved key value AND that only that key's
   downstream row is rewritten (a second, untouched key's row is unchanged).

## Tasks

1. Land the two spec edits above.
2. Make `execute.rs`'s `model_edges_for` and `build_maint_source_facts` `pub(crate)` — the
   plan-time resolver reuses them rather than re-deriving edges/source facts (maintenance-plan
   purity: one derivation, two consumers).
3. Extract the upstream identity formula `dispatch_key_addressed_model_edge` computes inline
   (`upstream_source_address = "smelt.models.{edge_name}"`,
   `upstream_table = "{config.get_target(...)-schema}.{db_name}"`) into one shared
   `pub(crate)` helper, and call it from BOTH dispatch and the new seed resolver — a divergence
   here silently misses the sidecar partition instead of failing.
4. Add `propagation::KeyedSeedDiff` (upstream model address, upstream db name, consumer model
   address, consumer db name, upstream keys, digest columns, consumer clean SQL) and the pure
   `keyed_seed_diffs_to_read(models, source_infos, deltas)`: for each delta origin that is a
   maintained model, for each consumer whose `resolve_live_key_addressed_model_edge_cells`
   yields a cell naming that origin, emit one descriptor. Bare names only — schema
   qualification is the live half's job (task 3's helper), mirroring
   `observed_delta_keys_to_read`'s "ask the planner module, don't re-derive" precedent.
5. Add the pure `fold_keyed_seed_values` + the diff-result → `KeyValues` classifier (tests 3–4).
6. Add `propagation_live::resolve_keyed_seeds(backend, config, target, diffs)` — one
   `maintenance_driver::diff_repair_group_sidecar_changed_keys` call per descriptor
   (`output_table` = the **consumer's** table, matching `execute_key_addressed_model_edge_cell`'s
   own argument), folded into `BTreeMap<String, KeyValues>`. Read-only: it must NOT refresh any
   sidecar — the refresh stays inside the write transaction, so the run-time diff re-derives
   the same set and the union at dispatch only ever widens.
7. Make `plan_since_upstream_full` public as `plan_since_upstream_live` (both channels: observed
   deltas + keyed seeds); keep the two existing wrappers unchanged.
8. Wire `smelt-cli`'s `run_since_upstream`: on the backend connection it already opens for the
   observed-delta read, resolve the keyed seeds too, and call `plan_since_upstream_live`. Runs
   under `--dry-run` for the same reason phase 6 pinned (a dry run must preview the live run's
   dirty set).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test key_addressed_model_edge_lowering --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test since_upstream --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet 2>&1 | tail -20`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches

## Commit message

`feat(incremental): --since-upstream resolves keyed seeds live from the group-grain sidecar`
