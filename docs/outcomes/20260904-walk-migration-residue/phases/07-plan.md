# Phase 7 plan — retire the divergence bullets this outcome actually closed

## Objective

Close success criterion 4 (and the honesty half of criterion 5): `model_properties.md`
§Known Divergences must state only gaps that are *live*. Each of MP-03, MP-05, MP-11 and MP-13
is verified clause-by-clause against the code, and only false text is deleted — a bullet whose
residue survives is **narrowed**, not removed, and a bullet that now describes a different live
gap stays. A new assertion in `walk_coverage` makes the deletions durable rather than a one-time
edit.

## Spec delta

No user-visible behaviour changes. The edits are all in
`docs/specs/model_properties.md` §Known Divergences (plus its `last_reviewed:`):

- **MP-03** ("The composition walk is not yet the sole source of every property") — the
  expression-position/bound/reach/grain/skew/trajectory clauses are closed by phases 1–3 and the
  bullet already reflects them. Verify the surviving clauses (`temporal` proof and the
  driving-fact/anchor join resolution running their own traversal; same-scope chained bands
  max-merging; the absorbing verdict rejecting every context source) against the code. Keep the
  clauses that are true — they are MP-02 / admission-width residue, already under §Out of scope —
  and delete any that phases 1–6 falsified. Re-point the tracking link to the out-of-scope owner
  rather than to this outcome if nothing here is left to do.
- **MP-05** (cumulative's whole-SQL `OVER(` scan) — expected already absent after phase 4.
  Confirm no restatement of it survives anywhere in the spec body (`§Constraints`,
  `§Semantics`), and if one does, delete it.
- **MP-11** ("Only one maintenance-cell route consults a declared-RI closure today") — false
  after phase 5. Delete it, and replace it *only if* the two sites phase 5 recorded as follow-up
  (`rules/cumulative.rs`'s once-write route, `maintenance/locality.rs`'s route-2 FD check) still
  read `has_fan_out_join` off an always-empty `JoinContext`, in which case the replacement is one
  narrow line naming exactly those two non-admission readers.
- **MP-13** (append-only probe: late append vs violation) — **not deleted.** Its original
  lateness claim was retired by the 2026-09-04 decision; the bullet standing today is a different
  live gap already scheduled on `docs/outcomes/20260904-decision-residue/outcome.md`. Verify the
  wording matches that gap and leave it.
- The function-call-argument bullet added by phase 6 stays untouched (live gap).

## Tests

1. `walk_coverage::spec_divergences_do_not_claim_closed_walk_gaps` (new) — reads
   `docs/specs/model_properties.md` §Known Divergences and fails if it contains a claim this
   outcome closed: an assertion that only one maintenance-cell route consults a declared-RI
   closure, or that a whole-SQL/`OVER(` scan governs cumulative classification. Red before the
   MP-11 deletion, green after.
2. `walk_coverage::spec_divergence_gate_detects_a_stale_claim` (new) — feeds the checker a
   synthetic §Known Divergences body containing one stale phrase and asserts it is reported, so
   the gate cannot silently pass on a section it failed to locate.

## Tasks

1. Grep the four residual MP-03 clauses against the code (`temporal` proof traversal;
   driving-fact/anchor join resolution; same-scope chained-band merge; absorbing-verdict handling
   of context sources) and record in the summary which are true.
2. Verify MP-05 has no surviving restatement anywhere in `model_properties.md` or
   `architecture.md` §"Property composition walk rule".
3. Verify the two phase-5 follow-up sites still hold an always-empty `JoinContext` (their
   `join-context:` classification tags name them), to decide delete-vs-narrow for MP-11.
4. Verify MP-13's current text describes the late-append-vs-violation gap and is linked to
   `20260904-decision-residue`; leave it in place.
5. Write test 1 and 2 in `crates/smelt-logical/tests/walk_coverage.rs`; watch test 1 fail.
6. Apply the §Known Divergences edits from the Spec delta; bump `last_reviewed:` to 2026-09-05.
7. Run `/smelt:validate model_properties`; fix any drift it reports that this outcome owns, and
   record in the summary any drift it reports that belongs to another outcome (do not fix it here).
8. Run the full gate set below; write `phases/07-summary.md` naming, per bullet, deleted vs kept
   vs narrowed and the evidence for each.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-logical --test join_context_reach --quiet`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `/smelt:validate model_properties` — clean, or every remaining item attributed to a named
  out-of-scope owner in the summary.

## Commit message

`docs(model_properties): retire the divergence bullets the walk migration closed`
