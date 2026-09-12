# Phase 8 summary — the empty divergence registry fails closed, not vacuously

**Shipped:**
- `crates/smelt-cli/tests/github_activity_oracle.rs`: `assert_matches_oracle` split into
  a `Result`-returning `check_matches_oracle` plus a thin panicking wrapper; every
  existing caller stays on the wrapper.
- Five new tests: `assert_matches_oracle_fails_closed_on_an_empty_registry` (drives
  `check_matches_oracle` itself, not just `compare_databases`, over a perturbed relation),
  `check_bound_accepts_a_holding_bound`, `check_bound_rejects_a_leading_side`,
  `check_bound_rejects_divergence_outside_the_licensed_columns` (exercise `check_bound`'s
  `MonotoneDivergence` arm and both `Side` variants, locally constructed — dead code
  otherwise on an empty registry), `no_relation_diverges_unexplained` (criterion 5's
  zero-unexplained-count claim as a direct assertion over the full 30-day fixture).
- `registry_entries_are_all_live` extended with the same direct empty-registry check, so
  its own loop over `DIVERGENCE_REGISTRY` cannot pass vacuously either.
- New `perturbed_one_day_pair()` helper factors the staging + perturbation shared by
  `an_unregistered_divergence_fails` and the four new registry tests.
- `#[allow(dead_code)]` removed from `Bound` and `Side` — both are now constructed by
  real tests; clippy confirmed clean without them.
- `docs/handoffs/2026-09-08-github-activity-findings.md`'s divergence section names the
  five new tests, so "empty registry" reads as "measured and found nothing," not "never
  measured."

**Decisions:**
- Kept `an_unregistered_divergence_fails`'s existing assertions untouched; only extracted
  its staging into the shared helper (plan task 3).
- Extended `registry_entries_are_all_live` in place rather than deleting it in favour of
  `no_relation_diverges_unexplained` — the two assert the same condition today (empty
  registry) but the former is the ratchet that will regain its per-entry loop the moment a
  divergence is next registered, so both stay.

**For the next planner:**
- File grew from 975 to 1151 lines — well under the 1500-line default cap, no baseline
  entry needed.
- Criterion 5's cross-target (DuckDB-vs-BigQuery) half has no residual work: the spine
  never ran live BigQuery, so it registered no dual-target divergence at all. This is
  already covered by the outcome's Out of scope bullet on BigQuery-only defects; recorded
  again in the decision log so criterion 5 doesn't read as half-checked.
- Row 9 (characterise/fix the two known live conformance failures) and row 10 (close-out:
  regenerate dialect-coverage docs, move gap ratchets, update issue #179) are next,
  unchanged by this phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, workspace
  tests, example_diagnostics).
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb` — 16 passed,
  1 ignored (measurement-only sweep), 115.59s.
- `cargo test -p smelt-cli --test github_activity_replay --features duckdb` — 17 passed,
  56.97s.
- `bash .claude/scripts/large-file-check.sh` — OK.
