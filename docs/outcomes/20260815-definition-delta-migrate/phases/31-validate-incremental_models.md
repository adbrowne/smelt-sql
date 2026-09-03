## Drift Report: incremental_models

**Spec**: docs/specs/incremental_models.md (last_reviewed: 2026-09-03)
**Date**: 2026-09-03

### Scope note

This audit targets the close-out obligation of outcome
`20260815-definition-delta-migrate` (success criteria 9–20): confirming every Known
Divergences bullet phases 10–29 claim to close is actually gone or honestly narrowed in the
spec text, and that every surviving bullet maps to an out-of-scope entry, a still-live
sibling outcome, or a §Future Extensions item. It is not a from-scratch full-spec Surface/
Semantics audit (the spec is ~2200 lines; a full re-audit is out of this phase's stated
scope, which is the divergence-closure discipline, same as criterion 8's `definition_deltas`
precedent).

### Criteria 10–19 cross-check (bullet-by-bullet)

- ✅ **Crit 10** (scheduler delta-signature consumption) — bullet narrowed correctly to the
  clockless-cross-model-watermark / value-level-discovery residue only (line 2045-2052); the
  `KeyedUpsert`→`grain: partition` dispatch gap itself is gone from the bullet text.
- ✅ **Crit 11** (per-cell `deferral` scheduling; `diff_patch` region default) — both bullets
  present but narrowed to their stated residues (lines 2057-2083), not "unscheduled"/"no
  runtime lowering" wholesale.
- ✅ **Crit 12** (write-pin equivalence factor; inadmissible write-variant pin gate) — both
  bullets removed; no hits for either phrase in the current spec text.
- ✅ **Crit 13** (observed-delta consumption partial) — bullet removed.
- ✅ **Crit 14** (maintained-model-creation technique; `GROUP BY`-derived `grain: key`
  frontmatter check) — bullet removed.
- ✅ **Crit 15** (plan-consumer / graph-layer gaps) — "Plan-consumer gaps" bullet removed; the
  cost-model carve-out is preserved as a stated fixed-preference-order item under §Future
  Extensions, not left unbuilt-with-no-fallback. "Graph-layer gaps" narrowed to the
  key-temporal-locality-establishment residue only (lines 2094-2098).
- ✅ **Crit 16** ("Locality and diagnostic residues"; `INTERSECT`/`EXCEPT` unclassified for
  mutation-sensitivity) — both bullets removed from `incremental_models.md`. Swept
  `model_properties.md` §Known Divergences: its surviving `INTERSECT`/`EXCEPT` bullet (line
  377) is a *different*, still-genuinely-open residue (filter-distribution classification,
  explicitly distinguished in its own text from the now-closed per-arm mutation-sensitivity
  combination rule) — not a stale duplicate of the closed bullet.
- ✅ **Crit 17** (conditional-maintenance gaps) — narrowed exactly to the declared
  `supports_fingerprint_sidecar` backend-capability gap (lines 2118-2122), matching the
  criterion's stated acceptable residue.
- ✅ **Crit 18** (Open Questions decisions) — no-out-of-band-edit tripwire recorded as a
  decided non-goal (line 2017, no `(Open Question)` tag); `on_column_add` dropped and recorded
  in `definition_deltas.md` (line 470); group-merge-provenance recorded as forced region
  recompute in `incremental_models.md` (line 894-898); `change_feed`+`UpstreamMutation`
  recorded with the honestly-open full-input-re-derivation residue (lines 2113-2117); the
  docs-site CLI-surface coverage audit closed as a standing gate
  (`crates/smelt-cli/tests/cli_docs_coverage.rs`). Only one `(Open Question)` tag remains in
  the whole file (`Override-ladder reach`, line 2107) — pre-existing, not one of the five
  criterion-18 items, and not decidable without new product input (see Finding 1 below).
- ✅ **Crit 19** — handled in `incremental_shapes.md` (see companion report); confirmed no
  hits for "safety_overrides" hard-error gap or "silently full-refreshes" in either spec.

### Out-of-scope cross-check

Sampled the bullets criteria 10–19 do *not* name, to confirm none were silently orphaned by
this outcome's work:

- Posture-derived key departure (`retain_departed` unimplemented, lines 2032-2038) and
  Override-ladder reach (Open Question, line 2107) do **not** appear in
  `20260815-keyed-grain-residue`/`20260815-partition-grain-residue` (both still `queued`),
  nor in this outcome's own "Out of scope" section, nor in §Future Extensions by name. See
  Finding 1/2 below — new phase rows added rather than silently left.
- Determinism scope, ledger DuckDB-only substrate, delta-signature headline in `explain`,
  straddle attribution, derived model-wide horizon, delta-detection explicit-only — all map
  cleanly to named Out-of-scope entries or §Future Extensions ("Proofs as product",
  "Automatic, watermark-diffed `--since-upstream`") or a cited decision record
  (`docs/research/20260816-open-questions-triage.md`).

### Findings (new phase rows added, not implemented here)

1. **Posture-derived key departure** (`retain_departed`) has a decision record but no
   declaration parsing/oracle transform/probe emitter/diagnostic, and maps to no live outcome.
   Added as phase 32.
2. **Override-ladder reach (Open Question)** — the first-build-vs-steady-state rule not
   reaching the keyed-fold suppression consumer — maps to no live outcome either. This one
   *is* an Open Question (not a small in-spec decision), so it is added as a phase row scoped
   to "decide or explicitly move to Out of scope," not to implementation.

### Timeless-oracle check

`grep -nE "Phase [A-Z0-9]+"` over the body (Known Divergences through References, excluding
the tolerated References→Plans links) — no hits.

### Freshness

- `last_reviewed`: 2026-09-03 (already current — most recent commit touching the spec is the
  same date).
- Verdict: fresh.

### Automated checks

Deferred to the shared gate sweep run for this phase (`phases/31-summary.md` § Gates) rather
than duplicated here — `bash .claude/scripts/verify-phase.sh` and the outcome's standing-gate
list.

### Summary

- Drift items: 0 remaining wording drift; 2 orphaned-bullet findings (new phase rows 32, 33
  added, not fixed here per the plan's own instruction).
- Recommended next step: none for wording; phases 32/33 queue the two orphaned findings.
