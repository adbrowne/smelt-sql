# Outcome: Ship proofs as a product surface (`smelt prove`, `must_hold:`, proof-diff in CI)

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_models.md` / `docs/specs/incremental_shapes.md` §Future
Extensions ("Proofs as product"); `docs/research/20260811-delta-signatures-and-definition-deltas.md`
§6 step 4 (deliberately last); `docs/outcomes/20260815-definition-delta-migrate/outcome.md`
§"Out of scope"
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md`

## The outcome

`smelt prove` ships as a CLI verb that derives and prints a model's full guarantee summary — the
delta-signature headline and a per-column guarantee readout — closing the "neither the per-column
guarantee ledger nor the derivable forward reach is printed" divergence both anchor specs flag
today. `must_hold:` ships as frontmatter grammar declaring a proof obligation a model's plan must
satisfy; a change that would weaken a declared obligation fails loud at plan time. Proof-diff runs
in CI: a PR that changes what a model's plan can prove about it renders a before/after diff of the
guarantee summary as a review artifact. `smelt explain`'s existing guarantee-summary sections are
rewritten around this shared machinery rather than duplicating it.

## Success criteria (checkable)

1. `smelt prove <model>` prints the delta-signature headline and per-column guarantee summary for
   an admitted maintained model.
2. `must_hold:` frontmatter grammar is specified and parsed; a plan that would weaken a declared
   obligation refuses at plan time with a named diagnostic.
3. Proof-diff runs in CI: a documented workflow step renders the guarantee-summary diff for a PR
   that changes proof-relevant surface.
4. `smelt explain` and `smelt prove` share one derivation path for the guarantee summary — no
   duplicated ad hoc rendering.
5. docs-site ships a proofs-as-product guide page.
6. `incremental_models.md` §Future Extensions' "Proofs as product" entry and both anchor specs'
   "guarantee ledger / derivable forward reach unprinted" divergence bullets are removed.
   `/smelt:validate` reports no drift for both. All standing gates green.

## Out of scope

- None named beyond what's already sequenced: this outcome runs last among the 2026-08-15
  build-everything set because it needs to print whatever lattice v2, ladder rungs 3–4, and the
  retraction/change-feed work changed about what there is to prove — do not start it before those
  land, or its guarantee-summary design will need reworking.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: `smelt prove` CLI surface, `must_hold:` frontmatter grammar, proof-diff CI contract | pending |
| 2 | `must_hold:` declaration parsing + plan-time obligation check | pending |
| 3 | `smelt prove`: derives and prints the guarantee summary | pending |
| 4 | Proof-diff in CI: workflow wiring | pending |
| 5 | `smelt explain` guarantee-summary rewrite onto the shared derivation | pending |
| 6 | docs-site: proofs-as-product guide | pending |
| 7 | Validate + close out: `/smelt:validate` clean for both anchor specs, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
