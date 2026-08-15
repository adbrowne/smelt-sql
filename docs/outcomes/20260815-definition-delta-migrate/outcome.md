# Outcome: Wire the definition-delta synthesis layer (plan-and-approve migration)

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 2
**Spec anchors:** `docs/specs/definition_deltas.md`, `docs/specs/incremental_models.md`,
`docs/specs/incremental_shapes.md`

## The outcome

The classification and emission machinery for definition changes
(`crates/smelt-logical/src/backbuild/`: diff factoring, per-group verdicts, the technique
catalogue, script assembly) stops being dead code and becomes the `smelt migrate` verb that
`docs/specs/definition_deltas.md` already specifies: a definition change (a redefined column, a
changed grain-adjacent field) is classified per column group into a verdict (eclipsed / backfill
in place / re-derive / skeleton change), a plan is printed naming the technique per group, and
`--apply` only executes a plan whose hash matches what was printed and approved — giving CI a
gate against unreviewed migrations. The ranged-rebuild verb ships under its spec name
(`smelt rebuild`, not `smelt backbuild`). The generative conformance suite exercises definition
edits, not only data deltas, so the equivalence invariant is checked for this mechanism the way
it already is for the maintenance ladder. This outcome closes the gap the incremental-spec
redraft (`docs/outcomes/20260809-incremental-spec-redraft/outcome.md`) deliberately left open:
that outcome specified this mechanism and recorded it as unwired; this one wires it.

## Success criteria (checkable)

1. `smelt migrate` exists as a CLI verb: given a model whose definition changed, it invokes the
   backbuild synthesis layer (diff → classify → emit) and prints a plan (per-group verdict +
   technique), without executing anything.
2. `smelt migrate --apply` executes only a plan whose stored hash matches the plan just
   re-derived; a stale or unapproved plan refuses with a distinct CI exit code. An approval
   store persists the hash (§"No approval store exists" divergence closed). The open question
   "plan-hash scope" (`definition_deltas.md` §Known Divergences) is resolved and the decision is
   recorded in the spec, not left implicit in the code.
3. The ranged-rebuild verb is named `smelt rebuild` (renamed from `smelt backbuild`) end to end:
   CLI, `--help`, docs-site, examples, tests.
4. The generative maintenance-conformance suite (`cargo test -p smelt-cli --test
   maintenance_conformance`) gains a definition-edit step kind — staged definition changes mid-
   history, asserted against the full-refresh-on-new-definition oracle — closing "The
   conformance harness has no definition-edit step kind yet".
5. The atomicity divergence is resolved one way or the other, not left "conditional in
   practice": either the `schema_evolution: strategy: full_refresh` escape routes through the
   same migration gate as every other backfill-in-place field, or it gets a real repair path.
   Whichever is chosen is stated in the spec, and the divergence bullet is removed.
6. `MaintenanceSkeletonColumnAdded` is renamed or split per the spec's own noted decision
   (`MaintenanceSkeletonChanged` or a split add/changed pair), and the definition-change
   diagnostic is surfaced ahead of a run (LSP + `smelt explain`), not only reachable via the
   maintenance driver's own I/O path.
7. A docs-site migration guide page ships (`definition_deltas.md` §References currently says
   "none yet — lands with the wiring plan").
8. `/smelt:validate definition_deltas` reports no drift; every Known Divergences bullet this
   outcome claims to close is actually removed from `definition_deltas.md`, not just addressed
   in code.
9. All standing gates green, including the new/extended conformance suite, `statement_parity`,
   and `walk_coverage`.

## Out of scope

- Lattice v2 (retention, reconciliation points) — research doc §6 step 3, sequenced after this
  step because it consumes this step's approved-destructive-legs machinery. Separate outcome.
- Proofs as product (`smelt prove`, `must_hold:`, proof-diff in CI, `smelt explain`'s
  guarantee-summary rewrite) — research doc §6 step 4, deliberately last because steps 1–3
  change what the proofs say.
- The scheduler-consumes-delta-types work (research doc §6 step 1) — tracked by
  `docs/outcomes/20260809-output-delta-typing/outcome.md`; this outcome does not touch dispatch
  currency.
- `incremental_shapes.md`'s own Known Divergences (partition-grain lookback classification,
  keyed-grain nullability routes, etc.) — these predate the spec split and are already tracked
  by their own plans (`20260530-thread-fn-registry-classification`, `20260705-keyed-collapse`,
  `20260715-composed-axes-conditional-maintenance`, etc.); this outcome does not re-scope them.
- Eclipse-detection breadth (algebraic identities, join reorderings) and row-local derivation
  for mid-catch-up groups — `definition_deltas.md` §Future Extensions, explicitly future work
  past the verdict set this outcome wires.
- Retraction handling / change-feed consumption — `definition_deltas.md` §"What stays
  data-side", explicitly out of this mechanism's scope.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Resolve the two open design questions (plan-hash scope, diagnostic rename/split) and land the decisions in `definition_deltas.md` before wiring against them | done |
| 2 | Wire `smelt migrate` (plan-only): CLI verb invokes the backbuild synthesis layer end to end and prints the per-group verdict/technique plan | pending |
| 3 | Approval store + `--apply`: plan-hash persistence, hash-mismatch/staleness refusal, CI exit codes | pending |
| 4 | Rename `smelt backbuild` → `smelt rebuild` across CLI, docs-site, examples, tests | pending |
| 5 | Conformance harness gains a definition-edit step kind; wire into the generative equivalence suite | pending |
| 6 | Close the atomicity divergence (unify the `schema_evolution` full-refresh escape with the migration gate, or land its repair path) | pending |
| 7 | Diagnostic rename/split lands in code; surface ahead of a run via LSP and `smelt explain` | pending |
| 8 | docs-site migration guide page | pending |
| 9 | Validate + close out: `/smelt:validate definition_deltas` clean, Known Divergences bullets removed, full standing-gate sweep | pending |

## Decision log

- **2026-08-15, phase 1.** Plan-hash scope: hash the plan data structure the emitters consume
  (verdicts, techniques, input facts — source declarations, backend capabilities), not only
  rendered SQL text; exclude region *enumeration*, which is resolved at apply time from the
  frontier so `--apply` stays reachable on an actively-loading warehouse. Diagnostic
  rename/split: rename `MaintenanceSkeletonColumnAdded` to `MaintenanceSkeletonChanged` (one
  code, not a split add/changed pair) — add and change trigger identical refusal and
  remediation, and every other `Maintenance*` code names the refused condition, not its
  trigger. Both landed in `docs/specs/definition_deltas.md` §Design and §Known Divergences;
  §Surface and body prose now use the target names. The code-side rename and the sibling-spec
  sweep (`model_transforms.md`, `model_properties.md`, `incremental_models.md`,
  `schema_evolution.md`, `diagnostics.md`) are deferred to phase 7, since renaming a
  diagnostic code is itself a code change out of scope for this docs-only phase.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
