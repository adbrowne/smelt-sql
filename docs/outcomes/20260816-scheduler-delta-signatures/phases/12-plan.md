# Phase 12 plan — scheduler-driven keyed→partition recipes in the conformance suite

## Objective

Close success criterion 7 (and give criteria 1–3 their end-to-end evidence) by adding
generative `maintenance_conformance` cases that drive the **real `--since-upstream`
scheduler path** — live observed-delta read, live keyed-seed resolution, propagated keyed
restrictions — over the keyed→partition cross-model combination, and assert the result is
multiset-equal to the full-refresh oracle while `dag_kpart_b` maintains incrementally.
Today's `keyed_upstream_partition_downstream_matches_oracle` drives a plain whole-project
build, so nothing in the generative suite exercises the scheduler this outcome rebuilt.

## Spec delta

None. No user-visible behaviour changes; this phase adds coverage plus one internal
single-ownership extraction. (If the extraction in task 1 changes any CLI-visible wording,
it does not — `run.rs` keeps its own printing and error contexts.)

## Design pinned for the implementer

1. **Do not re-implement the CLI's live-plan sequence in a test.** A test that hand-rolls
   `observed_delta_keys_to_read` → `keyed_seed_diffs_to_read` → the two live reads →
   `plan_since_upstream_live` would pass while `run.rs` drifted. Extract that sequence into
   one new `smelt_runtime::propagation_live::resolve_live_plan(backend, config, target,
   models, source_infos, order, deltas) -> Result<SinceUpstreamPlan>`; `run.rs::
   run_since_upstream` delegates to it (keeping its own backend creation, `dirty_set_report`
   printing, and error contexts), and the conformance test calls the same function via
   `LinkCProject::backend()` + `LinkCProject::config`.
2. **Two scenarios, because they resolve different seeds.** A source-rooted sweep plans
   *before* `dag_kpart_a` re-runs, so the plan-time sidecar diff is legitimately empty and
   the repair is driven by the run-time sidecar union at dispatch. A model-rooted sweep
   (`--source dag_kpart_a`, planned after `a` was rebuilt) is the one that yields a genuinely
   non-empty live keyed seed. Both are real operator flows; assert what each actually
   guarantees and never fabricate a non-empty seed for the first.
3. **Assert the honest thing about the seeds.** For the model-rooted scenario, assert the
   resolved `plan.keyed_dirty` names the touched ids (via `keyed_restrictions_from_plan`)
   — that is criterion 2's "value-level discovery feeds the scheduler" evidence. If the
   observed shape is `KeyValues::Unresolved` rather than resolved values, do NOT weaken the
   test into vacuity: record the exact reason in the phase summary for row 13 and keep the
   oracle + incrementality assertions.
4. **Recipe shape stays the existing `DagBody::PartitionOverKeyedId`** (constant projection,
   `GROUP BY {id}`). Phase 11 established that the honest `GROUP BY d, id` shape now clears
   the walk but trips `MaintenanceKeyScopeColumnMissing` from `derive_affected_keys`; that
   gap is not this phase's work (see Decision log / Out of scope).
5. Run the scheduled models exactly as `run.rs` does: one `execute_project` per `plan.runs`
   entry, `start`/`end` from the run, `keyed_restrictions` = the whole
   `keyed_restrictions_from_plan(&plan)` map on every request.

## Tests (red-green)

1. `smelt-runtime` unit — `resolve_live_plan_matches_hand_wired_sequence`: over a staged
   keyed→partition project, `resolve_live_plan` returns a plan equal to the one produced by
   calling the four underlying functions in order (pins the extraction, red before task 1).
2. `maintenance_conformance/dags.rs::keyed_partition_scheduler_sweep_matches_oracle`
   (generative, `arb_keyed_case(4)` × `case_count()`): source-rooted `--since-upstream`
   sweep over `keyed_partition_sink_dag` — run exactly `plan.runs`, then every node
   multiset-equal to the full-refresh oracle.
3. Same test, incrementality leg: the sweep's `dag_kpart_b` manifest record has strategy
   `per_group_recompute` (the key-addressed cell dispatched under the scheduler, not the
   ordinary route).
4. `keyed_partition_scheduler_sweep_from_model_upstream_matches_oracle` (generative):
   delta lands, `dag_kpart_a` alone is rebuilt, then a `--source dag_kpart_a` live plan
   schedules `dag_kpart_b`; oracle-equal and `per_group_recompute`.
5. Same test, seed leg: `keyed_restrictions_from_plan(&plan)` for `dag_kpart_b` carries the
   touched ids (or, per pin 3, the recorded honest alternative).
6. `keyed_partition_scheduler_sweep_leaves_untouched_rows_bit_identical`: reuse the existing
   `touched_ids`/`untouched_ids` before/after snapshot pattern from
   `keyed_chain_*` so the sweep is shown to repair only what the dirt names.

## Tasks

1. Add `propagation_live::resolve_live_plan`; make `run.rs::run_since_upstream` delegate to
   it (no behaviour change, same error surfaces).
2. Add a testkit helper on `LinkCProject` (or in `dags.rs` if it needs nothing shared) that
   plans via `resolve_live_plan` and executes every `plan.runs` entry with the plan's keyed
   restrictions — one place both new conformance tests call.
3. Write tests 2+3 (source-rooted sweep) and get them green.
4. Write tests 4+5 (model-rooted sweep, live keyed seeds).
5. Write test 6 (untouched-row stability).
6. Doc comments on both new tests naming the spec sections they pin
   (`incremental_models.md` §"Dispatch — from propagated components to run units",
   §"Restrictions compose by union", §"Keyed dirt-sets and the narrowed refusal").
7. Write `phases/12-summary.md`, including whatever pin 3 turned up.

## Verification

- `cargo test -p smelt-runtime --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `cargo test -p smelt-cli --test since_upstream --quiet` (the CLI-surface regression net for
  the `run.rs` delegation)
- `bash .claude/scripts/verify-phase.sh` — must be all green

## Commit message

`test(conformance): drive keyed→partition recipes through the live --since-upstream scheduler`
