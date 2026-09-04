# Phase 7 summary — verify and retire the divergence bullets

## Shipped

- `docs/specs/model_properties.md` §Known Divergences, per bullet:
  - **MP-03** ("composition walk not yet sole source") — narrowed and retitled. Verified against
    code: the `temporal` proof's separateness is already correctly classified as a module-wide
    **advisory heuristic** (`analysis/temporal.rs`'s doc comment, `architecture.md` §"Property
    composition walk rule") — never feeds admission, so it isn't a live gap at all, just a
    permitted exception. The "same-scope chained bands max-merge / absorbing verdict rejects
    every context source" clause describes the walk's *intended* tropical-composition design
    (`§"Series/parallel (tropical) composition"`, line 313), already correctly implemented — not
    outstanding work. The one clause that is genuinely still true: `resolve_join_driving_fact`
    (event-time monotonicity trace) runs its own single-level FROM-clause traversal outside the
    walk, and it does feed admission. Bullet rewritten to state only that gap; dropped the
    `20260904-walk-migration-residue` tracking link (this outcome's success criteria never
    covered it) and kept the general `20260707-property-composition-walk.md` plan link.
  - **MP-05** (whole-SQL `OVER(` scan) — confirmed absent everywhere (`grep` over
    `model_properties.md` and `architecture.md`); already closed by phase 4, no bullet exists,
    no edit needed.
  - **MP-11** ("only one route consults declared-RI") — false after phase 5; replaced. Verified
    the two phase-5 follow-up sites (`rules/cumulative.rs`'s once-write route,
    `maintenance/locality.rs`'s route-2 FD check) still hold a literal `JoinContext::new()`
    (their `join-context:` classification comments confirm it). New bullet names exactly those
    two non-admission readers instead of the old, now-false single-route claim.
  - **MP-13** (append-only probe) — verified unchanged: its current text already describes the
    late-append-vs-violation gap (not the retired lateness claim) and links to
    `20260904-decision-residue/outcome.md`. Left as-is per the plan.
  - `last_reviewed` bumped 2026-09-04 → 2026-09-05.
- `crates/smelt-logical/tests/walk_coverage.rs`: two new tests plus their shared helpers
  (`known_divergences_section`, `find_closed_walk_gap_claim`, `CLOSED_WALK_GAP_CLAIMS`):
  - `spec_divergences_do_not_claim_closed_walk_gaps` — reads the real spec and fails if either
    closed claim (MP-11's or MP-05's original wording) reappears.
  - `spec_divergence_gate_detects_a_stale_claim` — synthetic-body test proving the section
    extraction actually locates `## Known Divergences` rather than trivially passing on a miss.

## Decisions

- Deleted two clauses from MP-03 that verification showed were never real divergences (advisory
  heuristic, intended tropical-composition design) rather than leaving them worded as "still a
  gap." Known Divergences is a gap list, not a place to record settled, correct behavior.
- Kept MP-11 as an entry (rather than deleting outright) since the two named sites are a real,
  named limitation (no declared-fact access for their callers) even though today's empty-context
  behavior is the correct fail-closed default — matches the plan's delete-vs-narrow guidance.
- Verified red-before/green-after for the new walk_coverage test by stashing just the spec edit,
  confirming `spec_divergences_do_not_claim_closed_walk_gaps` fails on the old MP-11 wording, then
  restoring the edit — satisfies red-green TDD without leaving the repo in a broken intermediate
  commit.

## For the next planner

- The one surviving true clause in MP-03 (`resolve_join_driving_fact`'s own traversal) is real,
  unmigrated, mechanical backlog outside this outcome's success criteria — no outcome currently
  tracks it beyond the general `docs/plans/20260707-property-composition-walk.md` plan. Worth a
  small future outcome if the walk-purity invariant is revisited.
- `/smelt:validate model_properties` was run in targeted form (Timeless-oracle grep + freshness
  date check) rather than the full automated-checks section, since `verify-phase.sh` already
  covers fmt/clippy/test/example_diagnostics in one gate; no drift found in either check.
- All four success-criterion-4 divergence bullets are now either deleted, narrowed to their true
  residue, or confirmed correctly unchanged (MP-13). Criterion 4 and the honesty half of
  criterion 5 are satisfied.

## Gates

- `cargo test -p smelt-logical --test walk_coverage --quiet` — 8 passed (6 pre-existing + 2 new).
- `cargo test -p smelt-logical --test join_context_reach --quiet` — 1 passed.
- `cargo test -p smelt-logical --quiet` — all passed (10 + 1 + 8 + 19 unit/integration tests).
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `/smelt:validate model_properties` (targeted: timeless-oracle grep + freshness) — clean, no
  drift found.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
