# Phase 08 plan — Conformance: a trimmed-retention source whose bound advances between run steps

## Objective

Give `smelt-maintenance-testkit` a `SourceRecipe` that declares a rolling `retention:` bound,
have the harness physically trim departed rows before each run-bearing step so the bound
advances with the schedule's own clock, and drive that pool through the standing
`maintenance_conformance` gate against the unmodified S-restricted oracle. Advances success
criterion 6 (and re-proves 4 and 5 end-to-end through the real `execute_project` pipeline
rather than at a unit seam).

## Spec delta

**None.** This phase adds test machinery only; no user-visible behaviour changes. Per the
planning decision recorded in `outcome.md`, retention mints no contract-lattice point and
needs no oracle transform: `STracker::s_restricted_oracle_sql` materialises its baseline from
the tracker's recorded rows (`materialize_rows`), never from the physical source relation, so
the existing oracle already *is* phase 1's quantifier (all history ever processed).

## Tests

Red-green, in this order.

1. `smelt-maintenance-testkit` unit — `render_source_yaml_emits_a_declared_retention`:
   a `SourceRecipe` carrying a retention bound renders `retention: '<n> days'` in the source
   YAML alongside its `timeseries:` block.
2. `smelt-maintenance-testkit` unit — `render_source_yaml_without_retention_is_byte_identical`:
   regression pin that `retention: None` renders exactly today's string.
3. `smelt-maintenance-testkit` unit — `retained_cutoff_is_the_run_clock_minus_the_bound`:
   the pure cutoff helper — rows strictly older than `as_of - bound` depart, rows on the
   boundary are retained.
4. Conformance gate — `retention_pool_upholds_equivalence_under_an_advancing_bound`:
   deterministic-seeded sample of retention-bearing recipes driven through the real pipeline;
   S-restricted multiset equivalence holds after every run step even though rows have
   physically departed the source.
5. Conformance gate — `retention_pool_actually_trims_rows` (anti-vacuity, mirrors
   `admission_rate_stays_above_floor`): at least one row physically departed the source over
   the sample, else the leg asserts nothing.
6. Conformance gate, pinned (non-generative) —
   `an_aged_backfill_past_the_advancing_bound_refuses_and_leaves_state_unchanged`:
   after the bound advances past an already-processed region, a `BackfillRegion` over it
   errors (`SourceRetentionExceeded`) and the maintained table still equals the oracle — the
   no-silent-under-read assertion at pipeline level.

## Tasks

1. Add `retention: Option<RetentionDecl>` (a day count, matching `sources.md`'s rolling
   interval spelling) to `SourceRecipe` in `crates/smelt-maintenance-testkit/src/recipe.rs`,
   defaulting `None` in every existing constructor, plus a `with_retention(days)` builder.
2. Render it in `render::render_source_yaml` (test 1); leave `feed::change_feed_source_yaml`
   alone unless a retention recipe reaches it.
3. Add the pure `retained_cutoff(as_of, bound)` helper and a
   `trim_source_to_retention(backend, source, as_of)` DELETE alongside it — new
   `src/retention.rs` in the testkit (keeps `recipe.rs`/`feed.rs` off their large-file
   baselines; check `.claude/scripts/large-file-check.sh` before choosing a home).
4. Call the trim from `gate/partition_pool.rs`'s `drive_and_assert_collecting` immediately
   before each run-bearing step, gated on `recipe.source.retention.is_some()` — a no-op for
   every existing case, so no existing leg's behaviour moves.
5. New `crates/smelt-cli/tests/maintenance_conformance/gate/retention_pool.rs` holding the
   generator (existing partition pool + a retention bound wide enough that steady-state
   forward runs stay admitted), tests 4-6, and its `case_count()`; register it in
   `gate/mod.rs`.
6. Confirm the leg is not silently all-refusals: if the seeded sample admits nothing, widen
   the bound rather than weakening the assertion, and say so in the summary.
7. If the aged-backfill refusal turns out unreachable through the schedule shapes available,
   flip this phase to `blocked` rather than substituting a weaker assertion.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance` (plus one run at
  `SMELT_CONFORMANCE_CASES` raised, to check the new leg is not seed-lucky)
- `cargo test -p smelt-maintenance-testkit`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-logical --test walk_coverage`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(maintenance): conformance leg for a trimmed source whose retention bound advances`
