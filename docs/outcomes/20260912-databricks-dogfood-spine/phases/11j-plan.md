# Phase 11j — Bootstrap `--auto`'s frontier for the job, offline

## Objective

Criterion 11's scheduled job cannot complete a run: `--auto`'s frontier detection
(`compute_auto_time_range`, `crates/smelt-cli/src/commands/run_setup.rs:185`) reads the
Volume-resident `.smelt/targets/databricks_job/intervals.json`, finds it empty (no scheduled
run has ever written to it), and refuses rather than picking a window — even though
`workspace.smelt_dogfood` already holds 12+ fixture days landed by hand during earlier
dogfood phases. This phase gives the outcome a **narrow, local fix**: a way to seed that one
interval file with the already-known ingestion history, entirely offline (no workspace, no
credential). It does not attempt the general question of whether `--auto` should reconcile
against backend-resident state for every cloud target — that stays open in
`docs/specs/run_state.md` §"Known Divergences / Open Questions" and `outcome.md` §"Blocked",
tracked for a future decision.

## Decided here (do not re-open)

- **Seed, don't unify.** The outcome's `## Blocked` entry listed unifying `databricks` and
  `databricks_job` under one location-keyed interval store as a candidate (b). This phase does
  *not* do that: going forward the job is the **sole** writer of `databricks_job`'s interval
  history (no more manual local runs against the dogfood schema), so there is nothing to keep
  in sync after the one-time seed — unifying the store would be solving a problem that stops
  recurring the moment the job takes over. If a future cloud target hits the same gap with an
  actively-mixed write pattern, that is exactly the general question left open in the spec.
- **Getting the seeded `model_hash` exactly right is a nice-to-have, not a correctness
  requirement.** `compute_auto_time_range` reads `ModelIntervals::latest_date()` directly and
  never checks `model_hash` — only the *next* successful write (`IntervalStore::get_or_create`
  in the real run path) compares hashes, and on a mismatch it invalidates `covered_intervals`
  and re-bases from that write's own hash. So a seed with a wrong or stale hash still unblocks
  `--auto` correctly on the first job run; the only cost of a wrong hash is collapsing the
  pre-job history down to just that first run's own interval instead of preserving the full
  12-day range. The new tool computes the real current hash anyway (task 2), since it's no
  harder to do right, but this fact is why a bootstrap tool that gets the hash wrong is still
  safe to run, and worth stating so a reviewer doesn't block the phase on it.
- **The seed file is generated locally, then uploaded — not run as a remote job task.** The
  Volume is reachable from a local machine via `databricks fs cp` with the `dbfs:` scheme
  (already proven by `scripts/dbx-bundle.sh seed`), so there is no need to invent a way to run
  a bootstrap step *inside* the job. The live half of this fix (phase 11k) is exactly one more
  `databricks fs cp`, scoped to the single `intervals.json` file — never a broader `.smelt/`
  sync, which would risk clobbering the lock file or any state a smoke run has already written.

## Spec delta

None beyond the Known Divergences entry already landed in `docs/specs/run_state.md` (this
plan pass). This phase is tooling, not a semantics change to `--auto` or the interval format.

## Tests

Red-green, all offline, no workspace.

1. `smelt_state::intervals::seed::seed_interval_bootstraps_auto` — given a temp project dir, a
   compiled model set, and a `(model, end_date)` list, the new bootstrap function writes
   `.smelt/targets/<target>/intervals.json` such that a subsequent `compute_auto_time_range`
   call (invoked directly, not through the CLI) returns `Some((end_date, today))` for a graph
   containing that model.
2. `seed_interval_uses_the_models_real_current_hash` — the written `ModelIntervals::model_hash`
   for a seeded model equals `compute_model_hash(&model.sql)` for that model's actual compiled
   SQL, not a placeholder.
3. `seed_interval_is_additive_across_models` — seeding model A then model B in two separate
   calls leaves both entries present (no clobbering of unrelated models already in the file).
4. `seed_interval_refuses_a_start_after_end` given `start > end` for one model's interval).
5. `crates/smelt-cli/tests/databricks_bundle.rs::seed_intervals_subcommand_prints_the_written_path`
   — the new `smelt state seed-interval` (or equivalent) CLI surface, run against a temp
   project with `--target databricks_job --model <m> --end 2026-08-16` (one model, one date;
   loop over models is the caller's job, not the subcommand's), writes the file and exits 0.

Place 1-4 in `crates/smelt-state/src/intervals.rs` (or a new `src/seed.rs` in that crate,
whichever keeps `compute_model_hash` and the new function co-located per the crate's existing
convention); place 5 in `crates/smelt-cli/tests/`.

## Tasks

1. Write tests 1-5 against the not-yet-existing function/subcommand (red).
2. Add `smelt_state::intervals::seed_interval(project_dir, target, model_name, model_hash,
   start, end)` — a thin wrapper around `FileStore::new(project_dir, target)`,
   `load_intervals`, `IntervalStore::get_or_create(...).record_interval(start, end)`,
   `save_intervals`. No new `FileStore` artifact kind; this reuses the existing `Intervals`
   posture gate as-is (a bootstrap write should not be possible on a `stateless` project any
   more than a real run's write would be — same `allows(StateArtifact::Intervals)` check
   already in `save_intervals`).
3. Add a `smelt state seed-interval --target <t> --model <m> --start <YYYY-MM-DD> --end
   <YYYY-MM-DD>` CLI subcommand under `crates/smelt-cli/src/commands/` that compiles the
   project (reusing the existing model-loading path other commands already call, not a new
   one), looks up `<m>`'s compiled SQL, computes its hash via `compute_model_hash`, and calls
   `seed_interval`. Refuses (named error) if `<m>` is not a model in the project.
4. Land tests 1-5 green.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test databricks_bundle --quiet 2>&1 | tail -20`
- No ratchet lowered.

## Commit message

`feat(state): a seed-interval bootstrap for --auto's frontier on a target with no local history`
