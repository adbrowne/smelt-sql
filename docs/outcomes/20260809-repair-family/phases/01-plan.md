# Phase 1 — Spec: the repair family

**Outcome:** `docs/outcomes/20260809-repair-family/outcome.md`
**Kind:** spec-only (no production code changes)

## Objective

Fix the normative semantics of the two repair techniques — per-group targeted recompute
and the `diff_patch` write pattern — as first-class members of the plan matrix: which
corner each occupies, the proof obligations that admit them, and where a retraction that
today refuses now routes instead. This is the spec-first gate for success criteria 1–3
and 5; every later phase implements against the text written here.

## Spec delta

All edits in `docs/specs/incremental_models.md` unless named otherwise.

1. **New section §"The repair family"**, placed immediately after §"Per-cell write
   addressing". It must fix, with rationale:
   - **What it is.** A mutation/retraction delta into a cell whose combiner is not
     invertible is repaired by recomputing *only the affected groups* from their bounded
     input slice — full-input read restricted to a key slice × targeted write. Where it
     sits in the 2×2: it is the targeted-write refinement of recompute-a-region, and like
     a region recompute it **supersedes and resets** the ledger for the keys it rewrites
     (§"Per-cell admission", interchangeability).
   - **Why it is correct.** State the equivalence argument explicitly: recomputing key set
     `K` over an input slice that provably contains *every* row contributing to any `k ∈ K`
     reproduces `full_refresh` restricted to `K`, and leaves every other key bit-identical
     — so the equivalence invariant holds cell-wide. Name slice completeness as the load-
     bearing premise (it reuses key temporal locality, §"Key temporal locality").
   - **Admission obligations** — extend §"Per-cell admission"'s numbered list rather than
     inventing a parallel one: (a) **derivable group key** — the walk's grain names the
     groups; (b) **bounded per-group read footprint** — the key→input-slice reach is
     derived or declared-and-checked; (c) **affected-key discovery** — the changed input's
     delta names a finite key set (below). All fail-closed: any one unprovable refuses by
     name, never widens to a whole-table repair.
   - **Ledger grading and re-run safety** — per-group recompute is `Idempotent`-graded for
     the keys in its slice; re-running it is safe, and it resets any additive ledger record
     for those keys exactly as a region recompute does.
2. **Affected-key discovery** — add a §"Derived proofs" entry to
   `docs/specs/model_properties.md` (owner of the proof; this spec consumes it): from a
   changed input's delta, derive the finite set of output group keys the delta can affect,
   via the model's grain expression over the delta rows. Fail-closed: a delta shape that
   cannot be resolved to a key set (an unkeyed retraction, a delta whose grain expression
   reads columns absent from the delta) yields no key set and the repair family is not
   admitted. State that a *sound over-approximation* is admissible (a superset of affected
   keys costs work, never correctness); an under-approximation never is.
3. **`diff_patch` write pattern** — a new bullet in §"Per-cell write addressing", added to
   the registry list and given its own short subsection under
   §"The write-pattern set is open": compute the candidate rows for a slice, diff against
   stored state, and write only the difference (insert absent rows, update rows whose
   compared columns differ, delete stored rows absent from a *complete* slice). Contract
   facts it requires: declared `unique_key` (row identity), change comparability over the
   written columns (`model_properties.md` §"Change comparability") for the update leg, and
   slice completeness for the delete leg — without the completeness proof the delete leg is
   not admitted and the pattern degrades to insert+update, stated explicitly rather than
   assumed. Grade: idempotent (a second run diffs to empty), which is what makes it the
   reconciliation and drift-repair write.
4. **Refusal narrowing** — in §"Reprocessing" and the `KeyedReprocessedWindow` /
   `KeyedRetractableContribution` prose (§Diagnostics): a reprocessed window or retraction
   over a non-invertible keyed model now **routes to the repair family first**; the refusal
   fires only when a repair obligation from item 1 fails, and the diagnostic text names the
   failing obligation and its fix. This is a plan-level route, not a new mode or a user
   flag.
5. **Diagnostics** — add `MaintenanceRepairKeysNotDiscoverable` (obligation (c) failed;
   names the changed input and why the delta yields no key set) and
   `MaintenanceRepairSliceUnbounded` (obligation (b) failed; names the source and the
   unbounded reach) to §Diagnostics here and mirror both rows into
   `docs/specs/diagnostics.md`. Confirm the existing `MaintenanceNoAdmissibleTechnique`
   text stays the catch-all only for cells where no family at all survives.
6. **§"The plan matrix"** — the "technique … drawn from the open write-pattern registry"
   bullet and the 2×2 prose gain the repair family so the matrix section is not silently
   stale; keep it to a sentence plus a cross-reference.
7. **§Known Divergences** — one new entry stating the repair family and `diff_patch` are
   specified ahead of derivation/emission, naming this outcome as the tracking artifact.
   Deleted in phase 7 once the code lands.

No `Phase N` vocabulary in any spec body (timeless-oracle rule).

## Tests

Spec-only phase: no new Rust tests. The checks are mechanical and run in Verification.

- Existing suites must stay green unchanged; any failure here means code leaked into a
  spec phase.
- Diagnostic-catalogue coverage (`cargo test -p smelt-db --test integration
  diagnostics_catalogue`) must stay green with the two new catalogue rows present ahead of
  their enum variants.

## Tasks

1. Read §"The plan matrix", §"Per-cell admission", §"Per-cell write addressing",
   §"Key temporal locality", §"Reprocessing", and the `KeyedReprocessedWindow` /
   `KeyedRetractableContribution` diagnostics prose.
2. Write §"The repair family" (delta item 1).
3. Add the affected-key discovery proof entry to `model_properties.md` (item 2).
4. Add the `diff_patch` registry entry and its subsection (item 3).
5. Apply the refusal-narrowing edits (item 4) and the two diagnostics rows in both spec
   files (item 5).
6. Apply the plan-matrix cross-reference (item 6) and the Known Divergences entry (item 7).
7. Re-read the whole diff against §"The equivalence invariant" and §"Validator, not
   chooser" for internal contradiction, and for timeless-oracle compliance.
8. Write `phases/01-summary.md`: the decisions fixed (corner placement, the three
   obligations, over-approximation rule, `diff_patch` delete-leg gating), anything that
   reshapes phases 2–7, and the exact spec anchors phase 2 implements against.

## Verification

- `bash .claude/scripts/verify-phase.sh` (spec-only, but must stay green).
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/model_properties.md docs/specs/diagnostics.md`
  — hits only permitted in Known Divergences / References lines paired with a plan link.
- Every diagnostic code named in the spec edits resolves in `docs/specs/diagnostics.md`.

## Commit message

`spec(incremental): the repair family — per-group recompute, diff-then-patch, admission obligations`
