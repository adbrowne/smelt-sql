# Phase 03 plan — reach vs. retention, produced by the composition walk

**Objective.** Advance success criterion 3: a pure, walk-backed verdict comparing a
model's required look-back against each source's declared rolling retention bound. The
reach term is already the composition walk's own output (`derive_model_bounds`'s
`BoundResult`); this phase adds the comparison as a pure fold over that output — never a
scan over the model's SQL text — and carries the run-window age term that makes the bound
*rolling* rather than fixed at authoring time (which criterion 5 will feed). No refusal,
no downgrade, no `smelt explain` rendering: phases 4 and 7 consume this verdict.

## Spec delta

`docs/specs/model_properties.md`:
- Declarations/derived-proofs table (the block around the "Unified bound / reach
  derivation" row): add one row — **Retention admissibility** — `RetentionVerdict` =
  `NoDeclaredBound | Within{required_lookback, retained} | Exceeds{required_lookback,
  retained} | UnprovableWithin{retained, reason}`: the walk's per-source backward reach
  plus the run window's own age, compared against the source's declared `retention:`.
  Status `built` (verdict only; the refusal that consumes it is `not-yet` until phase 4).
- A short subsection after §"Unified bound / reach derivation" titled **"Reach versus
  retained history"**: states that the comparison is a fold over the walk's `BoundResult`
  and the declared bound, that `Unbounded`/`NotDerivable` reach fails closed to
  `UnprovableWithin` (absence of a proof is a rejection), that an undeclared `retention:`
  is `NoDeclaredBound` (trusted replayable, `sources.md` §Semantics 5), and that
  `required_lookback = before + window_age` so the verdict is re-evaluated against the
  bound in effect at plan time on every run. Cross-link `sources.md` §Semantics 5 and
  `incremental_models.md` §"The equivalence invariant".

No user-visible surface changes; no `docs-site/` edit this phase.

## Tests

New `crates/smelt-logical/tests/retention_reach.rs` (or `#[cfg(test)] mod tests` in the
new module — implementer's call, keep the production file under the ratchet):

1. `no_declared_retention_is_no_declared_bound` — a source with `retention: None` yields
   `NoDeclaredBound` regardless of reach.
2. `bounded_reach_inside_the_retained_bound_is_within` — 7-day frame reach against a
   45-day bound, zero window age → `Within{7d, 45d}`.
3. `bounded_reach_past_the_retained_bound_exceeds` — 90-day frame reach against a 45-day
   bound → `Exceeds{90d, 45d}`.
4. `window_age_pushes_an_otherwise_within_reach_past_the_bound` — 7-day reach, 45-day
   bound, 60-day window age → `Exceeds{67d, 45d}`. This is the backfill/rolling case.
5. `unbounded_reach_against_a_finite_bound_is_unprovable` — `UNBOUNDED PRECEDING` →
   `UnprovableWithin{45d, UnboundedReach}` (never `Within`).
6. `not_derivable_reach_fails_closed_to_unprovable` — a `ROWS` frame / symbolic
   `INTERVAL '1 month'` → `UnprovableWithin{.., ReachNotDerivable}`.
7. `series_composition_through_a_cte_is_visible_to_the_verdict` — two stacked frames
   (4d + 4d) across a CTE boundary against a 5-day bound → `Exceeds{8d, 5d}`, proving the
   verdict inherits the walk's *series* composition and would be `Within` under a
   whole-text max-merge. This is the phase's walk-not-scan test.
8. `a_source_absent_from_the_model_gets_no_verdict` — the map has no entry for a declared
   source the model never reads.

Plus, in the existing `crates/smelt-core/tests/source_world_facts.rs` or the new file:
9. `retention_interval_converts_to_seconds_for_the_comparison` — `'45 days'` →
   `Seconds::days(45)`, so the comparison and the declaration cannot drift in units.

## Tasks

1. Write the spec delta above (spec-first).
2. Add `pub retentions: HashMap<String, Seconds>` to `BoundContext` in
   `source_bounds.rs`, with `with_source_retention` / `add_source_retention` setters
   mirroring the existing partition-column pair; fix the two struct-literal constructions
   in `crates/smelt-logical/src/rules/incremental.rs` (≈lines 1010, 1327).
3. New module `crates/smelt-logical/src/analysis/retention_reach.rs` (registered in
   `analysis/mod.rs`, re-exported like `source_bounds`): `RetentionVerdict`,
   `UnprovableReason { UnboundedReach, ReachNotDerivable }`, and
   `derive_retention_verdicts(sql, ctx, window_age: Seconds) -> HashMap<String,
   RetentionVerdict>` — implemented as `derive_model_bounds(sql, ctx)` mapped per source
   against `ctx.retentions`. New file, not an addition to `source_bounds.rs`, which is at
   its 3367-line ratchet baseline. Module doc comment states it holds **no** text scan:
   its only input is the walk's verdict.
4. Convert `DataLatency` → `Seconds` once, at the point `BoundContext` is populated (a
   helper next to the setter), so unit conversion has one owner.
5. Write tests 1–9 red, then green.
6. Confirm `retention_reach.rs` contains no `.contains("` (nothing for the walk-coverage
   gate to classify) and that `walk_coverage` is green unchanged.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -20`
- `cargo test -p smelt-core --test source_world_facts --quiet`
- `bash .claude/scripts/large-file-check.sh` (expect unmoved — new file, not growth)

## Commit message

`feat(logical): derive a per-source reach-vs-retention verdict from the composition walk`
