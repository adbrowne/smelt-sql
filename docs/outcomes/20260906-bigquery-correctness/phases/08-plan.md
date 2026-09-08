# Phase 8 plan — the empty divergence registry fails closed, not vacuously

## Objective

Close success criterion 5. Phases 3-7 fixed all five divergences the spine registered
rather than promoting any of them, so `DIVERGENCE_REGISTRY` is empty and the unexplained
count is zero — but three of the registry's own gates now loop over an empty slice and
pass by construction, and the machinery an empty registry hands to the *next* divergence
(`check_bound`'s `MonotoneDivergence` arm, `assert_matches_oracle`'s unregistered branch)
has no live test at all. This phase proves the sweep still fails closed on an empty
registry and names the zero unexplained count as a direct, checkable assertion rather
than as the generic sweep's silence.

## Spec delta

None. This phase changes no user-visible behaviour and no emitted SQL — it is test and
documentation work over `crates/smelt-cli/tests/github_activity_oracle.rs`. The
registry's own contract already lives in that file's doc comments and in
`docs/handoffs/2026-09-08-github-activity-findings.md`; both are updated as tasks below.

## Tests

All new tests live in `crates/smelt-cli/tests/github_activity_oracle.rs` and reuse
`full_replay_pair()` where they need the 30-day pair, so the phase adds no second
full-refresh over the whole fixture.

1. `assert_matches_oracle_fails_closed_on_an_empty_registry` — stage the one-day
   incremental/full-refresh pair, perturb one `gold_repo_dim` row in the oracle db, and
   assert the **registry-consulting comparator** (not just `compare_databases`) reports
   the mismatch, naming the relation. Red first: today no test reaches that branch, so it
   requires splitting `assert_matches_oracle` into a `Result`-returning core.
2. `check_bound_accepts_a_holding_bound` — a test-local `DivergenceEntry` over
   `gold_repo_dim` (key `repo_id`, exact `["first_seen_at"]`, monotone
   `["current_repo_name"]`, `behind_side: Side::Incremental`) against the perturbed pair
   returns `Ok`: the incremental leg is genuinely behind, which the bound licenses.
3. `check_bound_rejects_a_leading_side` — the same entry with
   `behind_side: Side::Oracle` returns `Err` naming `current_repo_name`: the side the
   bound forbids from leading is leading.
4. `check_bound_rejects_divergence_outside_the_licensed_columns` — the same entry with
   `current_repo_name` moved into `exact_columns` returns `Err`, proving a bound never
   blanket-licenses a relation.
5. `no_relation_diverges_unexplained` — over `full_replay_pair()`, every relation
   `discover_relations` finds has `incr_only == 0 && full_only == 0`, and
   `DIVERGENCE_REGISTRY` is empty: criterion 5's "count of unexplained differences is
   zero" asserted directly, in one named test, over the full 30-day fixture.
6. `registry_entries_are_all_live` (existing, extended) — on an empty registry, assert
   the emptiness is a *claim* rather than an omission by requiring test 5's condition, so
   this two-sided ratchet cannot pass vacuously in either direction.

## Tasks

1. Split `assert_matches_oracle` into `check_matches_oracle(incr_db, full_db, label) ->
   Result<(), String>` (the registry lookup + bound check + unregistered-divergence
   branch) and a thin `assert_matches_oracle` that unwraps it with `panic!`; leave every
   existing caller on the panicking wrapper so no current test's behaviour moves.
2. Add test 1 against `check_matches_oracle`, asserting the `Err` names `gold_repo_dim`
   and the phrase `unregistered divergence`.
3. Factor the one-day perturbed-pair staging shared by `an_unregistered_divergence_fails`
   and tests 1-4 into a single helper (returns the two db paths and keeps its `TempDir`
   alive); do not change `an_unregistered_divergence_fails`'s assertions.
4. Add tests 2-4 constructing `DivergenceEntry`/`Bound`/`Side` locally in the test module.
5. Remove the `#[allow(dead_code)]` attributes on `Bound` and `Side` now that tests
   construct both variants of `Side` and the `MonotoneDivergence` arm; confirm clippy is
   clean without them.
6. Add test 5 and extend `registry_entries_are_all_live` per test 6, with a doc comment
   stating why an empty registry is not a licence for the ratchet to be a no-op.
7. Rewrite `DIVERGENCE_REGISTRY`'s "**The registry is now empty**" paragraph to cite tests
   1-5 by name as the proof that emptiness is checked rather than assumed.
8. Update `docs/handoffs/2026-09-08-github-activity-findings.md`: its divergence section
   states the registry is empty *and* names the fail-closed controls, so a future reader
   harvesting it cannot read the empty table as "never measured".
9. Append a dated decision-log entry to
   `docs/outcomes/20260906-bigquery-correctness/outcome.md` recording that criterion 5's
   **cross-target** half (the spine's criterion 6 is DuckDB-vs-BigQuery dual-target
   parity) registered nothing, because the spine is `blocked` with its live-BigQuery half
   never run — so the only divergences that ever existed to resolve are the five
   incremental-vs-oracle ones, all fixed. This is already covered by the outcome's
   existing Out of scope bullet on BigQuery-only defects; the entry makes the reasoning
   explicit rather than leaving criterion 5 looking half-checked.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb`
- `cargo test -p smelt-cli --test github_activity_replay --features duckdb`
- `bash .claude/scripts/large-file-check.sh` (the oracle test file is near 1000 lines;
  if it crosses its baseline, split the registry-machinery tests into a sibling test
  module rather than bumping the baseline)

## Commit message

`test(oracle): prove the empty divergence registry still fails closed`
