# Phase 1 summary — pin the scheduler-currency design (spec-only)

**Shipped:**
- `docs/specs/incremental_models.md`: new §Semantics subsection "Dispatch — from propagated
  components to run units" (after §"The graph layer") pinning typed-component run units and
  addressing-not-grain dispatch.
- `docs/specs/incremental_models.md` §"The graph layer": "Keyed dirt-sets and the narrowed
  refusal" rewritten to value-carrying dirt (was symbolic key-columns-only), plus new
  "Composition rules" and "Unresolved seeds" paragraphs.
- `docs/specs/incremental_models.md` §Surface: `--since-upstream` CLI bullet and "Run flags"
  reworded — `--landed` optional per source once a watermark exists.
- `docs/specs/run_state.md`: new §Surface subsection "Per-source watermark" pinning the
  watermark as a field on the existing landed-delta record (write path, transactionality,
  `state.mode` behaviour, live `--since-upstream` consumption); `.smelt/` layout comment
  updated.
- `docs/specs/state.md`: state-structure inventory row for the landed-delta record now names
  the watermark field and points at `run_state.md` §"Per-source watermark".
- `docs/specs/incremental_models.md` §Design: three new paragraphs — dispatch typed by
  addressing not grain; key values seed propagation (propagation stays pure); watermark is a
  field on the landed-delta family, not a new one.
- Two Known Divergences bullets reworded (not deleted) to point at the newly pinned design as
  the target: "scheduler does not yet consume delta signatures end to end" and "delta
  detection for `--since-upstream` is explicit-only in v1".
- `docs/specs/incremental_shapes.md`: one clarifying sentence distinguishing the rejected
  "watermark store" (computational, engine-duplicating) from the new per-source *propagation*
  watermark (observability, opt-in) so the two "smelt does not own a watermark" design
  paragraphs don't read as contradicting the newly pinned surface.

**Decisions:**
- Dispatch is typed by the component's **addressing**, never the downstream model's `grain` —
  this is what makes a `Keyed` component into a `grain: partition` downstream dispatchable.
- Key values are resolved once by the caller (fingerprint-sidecar diff) and passed into
  propagation as a seed, keeping propagation pure — no backend I/O inside derivation, no
  symbolic-dirt-resolved-at-run-time either.
- The watermark is a field on the existing landed-delta record (observability-classified),
  not a new correctness-classified state family — keeps it `state.mode: stateless`-optional
  under the existing degradation contract.
- Divergence bullets are reworded to cite the pinned design, never deleted — the design isn't
  implemented yet, so the divergence is still real; only its *framing* changes.

**For the next planner:**
- Phase 2 (dispatch the derived key-addressed repair cell outside `grain: key`) can proceed
  directly against §"Dispatch — from propagated components to run units" and the "Upstream
  model edges" paragraph — no further design decisions pending there.
- Phase 3 (key-valued dirt-sets) should re-read `model_properties.md` §"Affected-key
  discovery" before starting: `derive_affected_keys` already computes actual key *values*
  (not just columns) today — the gap phase 3 closes is in the graph layer's dirt-set
  *representation* and propagation plumbing, not in re-deriving the values themselves.
- Not touched (out of scope for this phase, flagged for later phases per the outcome table):
  `sources.md` §"Landed-delta (derived, recorded)" was cross-checked and needs no edit — its
  wording already composes cleanly with the watermark addition. The "Graph-layer gaps" and
  "Observed-delta consumption is partial" Known Divergences bullets were left untouched
  (not named by this phase's plan) but will need rewording once phases 3/4 land the behaviour
  they describe.
- Did not run `/smelt:validate incremental_models` as a live command (heavy Opus-driven
  process); manually cross-checked Surface/Design/Known-Divergences consistency instead. If
  Andrew wants the formal drift report before reviewing this phase, it should be run
  separately.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy, cargo test, example_diagnostics)
- `cargo test -p smelt-cli --test state_docs` — PASS (3 passed)
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/run_state.md docs/specs/state.md` — no matches
