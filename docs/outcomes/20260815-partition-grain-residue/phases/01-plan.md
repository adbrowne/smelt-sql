# Phase 1 — Audit the four cited tracking plans against current repo state

**Outcome:** `docs/outcomes/20260815-partition-grain-residue/outcome.md`
**Spec:** `docs/specs/incremental_shapes.md` §Known Divergences → "The partition grain"

## Objective

Establish, with executable evidence rather than plan-file prose, which of the partition-grain
residues cited by the four pre-outcome tracking plans are still open and which have already
landed. Advances success criterion 1 directly, and de-risks criteria 2–7 by leaving each of
their phases a pinned, inverted-on-landing probe instead of a from-scratch investigation.

## Spec delta

None. This phase changes no user-visible behaviour; it only reads the spec's divergence list.
If a probe shows a bullet is already satisfied, the spec bullet is *not* edited here — that is
phase 8's close-out job, so the whole divergence-list edit lands as one reviewable diff.

## Tests

Characterization probes that pin **today's** behaviour, each annotated with the residue it
tracks and the phase that will invert it. A probe that already behaves as the spec requires is
evidence the residue is closed — assert the spec-required behaviour directly in that case.

- `probe_lookback_gate_sees_define_body` — a lookback filter living only inside a
  `smelt.define` body: does the bound-`NotDerivable` refusal gate see it? (phase 2)
- `probe_batch_safety_sees_over_in_define_body` — an `OVER` inside a `smelt.define` body: does
  the window-function batch-safety check fire? (phase 2)
- `probe_cte_only_event_time_column` — a CTE alias that fails to project `event_time_column`:
  `EventTimeColumnNotVisibleAtOuterSelect` at check time, or a runtime failure? (phase 3)
- `probe_modeldef_per_model_override` — a generator-emitted `ModelDef` carrying a per-model
  override: accepted, or rejected by the closed field set? (phase 4)
- `probe_integer_partition_column_run` — a monotone-integer `partition_column` model through
  first-run: does it run, and where does it first fail? (phase 5)
- `probe_explain_json_run_relative_source_bounds` — `smelt explain --json` with a concrete run
  window: is per-cell `source_bounds` run-relative or unresolved? (phase 6)
- `probe_partition_column_rename_refusal` — renaming `partition_column` on a materialized
  model: named diagnostic, or unguarded? (phase 7)

## Tasks

1. Read the partition-grain bullets in `docs/specs/incremental_shapes.md` §Known Divergences and
   the four cited plans (`20260530-thread-fn-registry-classification`,
   `20260616-smelt-feedback-fixes`, `20260509-meta-language-overall`,
   `20260704-model-updates-l4-batched`); note each plan's own Progress table state.
2. Write the probes at the cheapest layer that observes the behaviour — logical/classification
   probes in `crates/smelt-logical/tests/partition_residue_probes.rs`, surface probes
   (explain/check/run) in `crates/smelt-cli/tests/partition_residue_probes.rs`. Do not create a
   file that would hold zero probes.
3. Run them; record for each residue: OPEN (probe pins divergent behaviour) or LANDED (probe
   asserts spec behaviour and passes). No probe may be left `#[ignore]`d — a residue too
   expensive to probe here is recorded as UNPROBED with one line saying what blocked it.
4. Write `docs/outcomes/20260815-partition-grain-residue/audit.md`: one row per partition-grain
   divergence bullet → {cited plan, plan's claimed state, probe verdict, owning phase or
   "out of scope + why"}. Cover **every** bullet in the section, not only the seven with phase
   rows, so nothing is dropped silently.
5. If the audit finds a bullet that is in scope by the outcome's framing (predates
   `docs/outcomes/`, cites a `docs/plans/*` tracker, no live owner) but has no phase row, add a
   phase row for it and a dated Decision-log line. If a bullet turns out to be genuinely owned
   by a live plan, record that in the Decision log per §"Out of scope".
6. If any probe shows a residue already LANDED, mark that phase row `done` in `outcome.md` with
   a dated Decision-log line citing the probe name as the evidence.
7. Write `phases/01-summary.md`: verdict table, the probe file paths and test names, and the
   single most surprising finding — phase 2's plan step reads only this file.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test partition_residue_probes 2>&1 | tail -20`
- `cargo test -p smelt-cli --test partition_residue_probes 2>&1 | tail -20`
  (whichever of the two files exists)

## Commit message

`outcome(20260815-partition-grain-residue): audit partition-grain residues with pinned probes`
