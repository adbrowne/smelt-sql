# Phase 6 plan — trust the DuckDB numbers

**Status:** planned · **Advances:** criterion 7 (DuckDB half), criterion 4, criterion 10

## Objective

Turn the existing end-of-replay, row-count-only equivalence check into the thing criterion 7
actually promises: after **every** incremental window, **every** materialised model's state
is compared **row-for-row** against a full refresh over the inputs seen so far. Differences
are not tolerated — each is either zero or matches a named, bounded entry in a divergence
registry, and the two known succession divergences get an exact characterisation instead of
a magic row-count delta. Banked before any cloud spend, so a phase 9+ failure is a backend
failure and nothing else.

## Spec delta

None. This phase adds test coverage over `examples/github_activity/`; no user-visible
feature behaviour changes. (`docs/specs/incremental_models.md` §"The equivalence invariant"
is the property being checked, not edited.)

## Tests (red first)

New test binary `crates/smelt-cli/tests/github_activity_oracle.rs`:

1. `every_window_matches_the_full_refresh_oracle` — the phase's centrepiece: replay the
   30-day fixture day by day; after each day, stage a fresh workspace, load days 0..=k with
   the identical redelivery rule, `smelt run --full-refresh`, and compare every materialised
   relation row-for-row against the incremental database. Zero symmetric difference, or a
   registry entry whose bound holds.
2. `oracle_comparison_covers_every_materialised_relation` — coverage totality: the set of
   relations compared is *discovered* from the two databases, not hardcoded; a relation
   present in one and absent in the other fails, and a newly added model is compared without
   editing the test.
3. `succession_divergence_is_exactly_tied_row_multiplicity` — the registry's two entries are
   bounded, not blanket: for `silver_repo_naming`/`silver_actor_naming`, after folding both
   sides on `(key, clock)` the multisets are **equal**, the incremental side has zero rows
   the oracle lacks, and every oracle extra is a `(key, clock)` duplicate of a shared row.
4. `an_unregistered_divergence_fails` — negative control on the comparator itself: perturb
   one row of one unregistered relation in the incremental database and assert the
   comparison reports it (guards against a comparator that silently passes).
5. `registry_entries_are_all_live` — a registry entry naming a relation that no longer
   diverges fails, telling the reader to delete it (two-sided ratchet, mirroring
   `dialect_audit`'s ledger).

## Tasks

1. Extract the replay harness (`stage_workspace`, `duckdb_exec`, `duckdb_scalar_i64`,
   `create_empty_raw_table`, `load_day`, `smelt_run`, `FIXTURE_DAYS`, `day_after`,
   `replay_days`) from `github_activity_replay.rs` into
   `crates/smelt-cli/tests/github_activity_support/mod.rs`, included by both test binaries
   via `mod github_activity_support;`. Required, not cosmetic: `github_activity_replay.rs`
   is 951 lines against the 1000-line default cap in `.claude/scripts/large-file-check.sh`.
2. In the support module, add the comparator: `ATTACH` both database files read-only on one
   connection and compute the symmetric difference per relation
   (`EXCEPT ALL` in both directions), returning counts plus a sample of offending rows for
   the failure message.
3. Add the divergence registry — a `const` table of
   `(relation, reason, bound)` where `bound` is a checkable predicate, not a row count.
   Seed it with the two succession entries, citing phase 3's summary and
   `docs/outcomes/20260906-scd2-keyed-succession` as the owner of the fix.
4. Write test 1: per-window oracle over the full 30-day fixture, all relations, using the
   comparator + registry.
5. Write tests 2–5.
6. Measure the new binary's wall time and record it in the summary. If it exceeds ~5
   minutes, compare after every window over the first 10 days and after the final window
   over all 30, rather than dropping windows silently — and say so in the summary.
7. Retire the now-superseded row-count assertions in
   `github_activity_replay.rs::full_refresh_matches_incremental_replay` (including the 139 /
   145 hardcoded deltas), leaving a pointer comment to the oracle binary. The distinct-id and
   business-count assertions that are *not* oracle comparisons stay.
8. Update `examples/github_activity/README.md` with a short "Trusting the numbers" note:
   what is checked after every window, and the two registered divergences with their reason.
9. Append the phase's findings to `outcome.md`'s decision log (dated), including any
   divergence the per-window check surfaces that the end-of-replay check missed — a new
   divergence is a **finding to register**, not a phase failure; only an *unbounded* or
   business-facing one blocks.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_oracle` — all five tests
- `cargo test -p smelt-cli --test github_activity_replay` — still green after the extraction
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh` — no file over its baseline

## Commit message

`test(github_activity): per-window full-refresh oracle with a bounded divergence registry`
