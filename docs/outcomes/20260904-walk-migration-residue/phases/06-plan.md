# Phase 6 plan — retire the whole-SQL flat-scan bound floor

## Objective

`derive_model_bounds` (`crates/smelt-logical/src/analysis/source_bounds.rs`) floors every
source's walk verdict by merging in `derive_bound_for_source(sql, …)` — a scan over the entire
model text. Commit `20e74879` records its only justification: a source read inside an
expression-position subquery, "which is not a walk node", would lose that region's band. Phase 1
made those scopes walk nodes, so a composition-relevant verdict is still being floored by a
whole-SQL scan with no live reason. This phase removes the floor — or narrows it to one named,
verified shape — advancing the outcome headline and unblocking criterion 4's deletion of the
MP-03 divergence bullet, which cannot honestly be deleted while a whole-SQL scan derives bounds.

## Spec delta

None expected: `model_properties.md` §"The composition walk" already states the walk is the sole
source and (since phase 2) that an expression-position scope's reach merges into the enclosing
per-source map. Only if task 5 finds a shape that genuinely cannot be walked does the spec gain
one **§Known Divergences** line naming that shape (behaviour terms, no phase vocabulary).

## Tests

Red-green, all in `crates/smelt-logical` (unit tests in `source_bounds.rs`'s `mod tests` unless
noted):

1. `expression_subquery_reference_site_reach_needs_no_flat_floor` — the exact shape `20e74879`
   was written for: a source read both as a plain FROM leaf and inside an expression-position
   subquery carrying a band. Asserts the **walk-only** derivation equals today's merged verdict.
2. `walk_only_equals_floored_across_reach_corpus` — table-driven over every SQL shape already
   exercised by this module's bound tests plus the `Unsupported`-adjacent shapes (parenthesised
   join group, redundantly-parenthesised derived table): walk-only == floored, per source. This
   is the discriminator that says whether the floor is dead weight.
3. `unsupported_normalization_still_falls_back_to_the_flat_derivation` — pins the *other* whole-
   text path (`has_unsupported()` → legacy derivation) as untouched by this phase, and names, in
   the test, a shape that actually still normalizes `Unsupported` today.
4. If task 5 finds a surviving shape: `flat_floor_survives_for_<shape>` — a named test that
   fails if the walk ever learns to cover it (so the floor cannot outlive its reason twice).
5. Regression fences (existing, must stay green): `expr_scope_inline_equivalence.rs`,
   `maintenance_tracer_propagation.rs`, `footprint_reflection.rs`, `walk_coverage.rs`.

## Tasks

1. Read `git show 20e74879` and lift its motivating SQL into test 1.
2. Add a `#[cfg(test)]`-visible walk-only derivation seam (e.g. private
   `derive_model_bounds_inner(sql, ctx, floor: bool)` with `derive_model_bounds` passing `true`)
   so tests 1–2 can compare the two derivations before anything is deleted.
3. Write tests 1–3; run them. A red test 1/2 means a real walk gap — fix it in `ReachTransfer`
   or the normalizer, never by keeping the blanket floor.
4. With 1–2 green, delete the floor loop and the `floor` seam; run
   `cargo test -p smelt-logical --quiet`, then `maintenance_conformance` and `statement_parity`.
   Triage each failure as a walk gap (fix) — the floor only ever *widened*, so any failure is a
   narrowing that names a missing walk path.
5. If a shape resists (after honest triage, not on first failure), restore a floor for **that
   shape only**, guarded by a predicate that names it, tagged as a leaf classifier per
   `docs/specs/architecture.md` §"Property composition walk rule"; add test 4 and the spec line.
6. Correct the stale doc comments in `derive_model_bounds`: the floor's rationale block (gone or
   rewritten) and the `has_unsupported` fallback's example — per phase 1's decision log
   `FROM ((SELECT …)) AS t` nests today and parenthesised join groups are walk nodes, so name a
   construct that actually still yields `Unsupported`.
7. Write `phases/06-summary.md`: what the corpus comparison showed, every walk gap fixed, and
   whether any floor survived and why.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-logical --test expr_scope_inline_equivalence --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `rg -n 'derive_bound_for_source' crates/smelt-logical/src` — no whole-`sql` caller survives
  except a task-5 shape guard (if any) and the `has_unsupported` legacy fallback.

## Commit message

`refactor(logical): source bounds come from the walk, not a whole-SQL flat-scan floor`
