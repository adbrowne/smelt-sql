# Phase 31 — Validate + close out (extended)

## Objective

Close the outcome by auditing the two remaining anchor specs the way phase 10 already audited
`definition_deltas.md`: run `/smelt:validate incremental_models` and `/smelt:validate
incremental_shapes`, and confirm that every Known Divergences bullet phases 11–29 claim to close
is actually gone (or honestly narrowed to its stated residue) in the spec text, not merely fixed
in code. Advances success criteria 9 and 20, and is the evidence line for 10–19.

## Spec delta

No new feature behaviour. Spec edits are **corrections only**, made where the audit finds drift:
a bullet describing a gap that no longer exists is removed; a bullet whose residue narrowed is
reworded to the true residue; a bullet pointing at a `done` outcome/plan as its owner is repointed
or restated as open on its own terms. Files: `docs/specs/incremental_models.md` §Known
Divergences, `docs/specs/incremental_shapes.md` §Known Divergences, plus any §Surface/§Semantics
line the validate step reports as drifted against shipped behaviour. If the audit finds a genuine
*implementation* gap (not a wording gap), do not fix it here — record it as a new phase row.

## Tests

No new product tests are expected; this is an audit phase. Add a test only if the audit finds a
normative rule with no test covering it:
- `<crate>::<rule>_is_covered` — one targeted test per uncovered normative rule the validate step
  flags, named for the rule, asserting the shipped behaviour the spec states.

## Tasks

1. Run `/smelt:validate incremental_models`; capture the drift report verbatim into
   `phases/31-validate-incremental_models.md`.
2. Run `/smelt:validate incremental_shapes`; capture into `phases/31-validate-incremental_shapes.md`.
3. Build the close-out checklist: for each of success criteria 10–19, list the bullet(s) it names
   and the current spec text at that location. Classify each as **removed**, **narrowed-correctly**,
   **still-present-but-should-be-closed**, or **worded-wrong**.
   - crit 10 → "The scheduler does not yet consume delta signatures end to end" (expect: narrowed
     to the clockless-cross-model-watermark / value-level-discovery residue only).
   - crit 11 → per-cell `deferral` scheduling; `diff_patch` over the region `DeleteInsert` default
     (expect both narrowed to their stated residues, not absent).
   - crit 12 → write-pin equivalence factor; inadmissible write-*variant* pin pre-execution gate
     (expect both removed).
   - crit 13 → "Observed-delta consumption is partial" (expect removed).
   - crit 14 → maintained-model-creation execution technique; `GROUP BY`-derived `grain: key`
     frontmatter check (expect removed; cross-check `models.md`).
   - crit 15 → "Plan-consumer gaps" and "Graph-layer gaps" (expect: plan-consumer removed except
     the explicitly-excluded cost model, which must be stated as a documented fixed preference
     order; graph-layer narrowed to the key-temporal-locality refusal only).
   - crit 16 → "Locality and diagnostic residues"; "`INTERSECT`/`EXCEPT` are unclassified set
     operations" (expect removed; sweep `model_properties.md` §Known Divergences for the same
     residue).
   - crit 17 → "Conditional-maintenance gaps" (expect narrowed to the declared
     `supports_fingerprint_sidecar` backend-capability gap only).
   - crit 18 → out-of-band-edit tripwire, `on_column_add`, group-merge-provenance, `change_feed`
     `UpstreamMutation` (expect: `(Open Question)` tags dropped, decisions recorded in §Design).
   - crit 19 → the two `incremental_shapes.md` key-grain bullets (windowless window-forward keyed
     run; `safety_overrides:` hard error) — expect both removed.
4. Apply the wording corrections the checklist identifies. Every bullet that stays must be
   accurate on its own terms and must not cite a `done` outcome as its current owner.
5. Cross-check every bullet still present in both specs against the outcome's §"Out of scope"
   list. A surviving bullet must map to an out-of-scope entry, a still-live sibling outcome
   (`20260815-keyed-grain-residue`, `20260815-partition-grain-residue`), or a §Future Extensions
   item. Anything mapping to none of those is a finding: add a phase row rather than silently
   leaving or deleting it.
6. Timeless-oracle sweep on both specs: no `Phase [A-Z0-9]` vocabulary in body sections; plan
   links only in §Known Divergences / §References.
7. Refresh `last_reviewed` on both specs and confirm their §References Code/Tests/User-docs
   blocks name files that exist.
8. Run the full standing-gate sweep (below) and record results in `phases/31-summary.md`.
9. If and only if every criterion 1–20 is satisfied, note that the outcome is ready to close; the
   Status flip itself is the loop's next plan step, not this phase's edit.

## Verification

- `bash .claude/scripts/verify-phase.sh` — fmt, clippy (both CI feature sets), full workspace
  test, `example_diagnostics`.
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-runtime --test execute_parity`
- `cargo test -p smelt-runtime --test dialect_seam`
- `cargo test -p smelt-runtime --test projection_dialect_invariance`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-dialect --test emission_ownership`
- `cargo test -p smelt-core --test hardening_budget`
- `cargo test -p smelt-types --test unknown_census`
- `cargo test -p smelt-db --test integration registry_consistency`
- `cargo test -p smelt-lsp --test example_workspaces`
- Both `/smelt:validate` reports committed under `phases/`, with every reported drift item either
  fixed in this phase or recorded as a new phase row / out-of-scope line.

Set `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` before building (see CLAUDE.md).

## Commit message

`docs(incremental): close out incremental_models/incremental_shapes divergence audit (phase 31)`
