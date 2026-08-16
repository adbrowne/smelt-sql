# Outcome: Wire the definition-delta vertical (`smelt migrate`, v2 — narrow)

**Created:** 2026-08-16
**Status:** active
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 3);
supersedes `docs/outcomes/20260815-definition-delta-migrate/` (this is that outcome's original
narrow vertical — its phases 2–9 — without the 2026-08-15 build-everything widening)
**Spec anchors:** `docs/specs/definition_deltas.md`

## The outcome

The classification and emission machinery for definition changes
(`crates/smelt-logical/src/backbuild/`: diff factoring, per-group verdicts, the technique
catalogue, script assembly) stops being dead code and becomes the `smelt migrate` verb
`docs/specs/definition_deltas.md` specifies: a definition change is classified per column group
into a verdict (eclipsed / backfill in place / re-derive / skeleton change), a plan is printed
naming the technique per group, and `--apply` executes only a plan whose hash matches what was
printed and approved — giving CI a gate against unreviewed migrations. The ranged-rebuild verb
ships under its spec name (`smelt rebuild`, not `smelt backbuild`), the diagnostic code ships as
`MaintenanceSkeletonChanged`, and the generative conformance suite exercises definition edits so
the equivalence invariant covers this mechanism the way it covers the maintenance ladder.

## Success criteria (checkable)

1. `smelt migrate` exists: given a changed model definition it invokes the backbuild synthesis
   layer end to end (diff → classify → emit) and prints the per-group verdict/technique plan,
   executing nothing.
2. `smelt migrate --apply` executes only a plan whose stored hash matches the re-derived plan; a
   stale or unapproved plan refuses with a distinct CI exit code; an approval store persists the
   hash per the already-decided plan-hash scope (see Decision log). Closes "No approval store
   exists."
3. The ranged-rebuild verb is `smelt rebuild` end to end: CLI, `--help`, docs-site, examples,
   tests, and the sibling-spec sweep (`cli.md` verb table and `--dry-run` prose,
   `model_selection.md` positional-selector callout, `architecture.md` prose where it means the
   CLI verb — the `backbuild/` module path and "backbuild synthesis" mechanism name may stay).
4. The generative maintenance-conformance suite gains a definition-edit step kind — staged
   definition changes mid-history asserted against the full-refresh-on-new-definition oracle.
   Closes "The conformance harness has no definition-edit step kind yet."
5. The atomicity divergence is resolved, not left conditional: the
   `schema_evolution: strategy: full_refresh` escape either routes through the migration gate or
   gets a real repair path; the choice is recorded in the spec and the divergence bullet removed.
6. `MaintenanceSkeletonColumnAdded` is renamed to `MaintenanceSkeletonChanged` in code (single
   code, per the recorded decision), swept across sibling specs (`model_transforms.md`,
   `model_properties.md`, `incremental_models.md`, `schema_evolution.md`, `diagnostics.md`), and
   the definition-change diagnostic is surfaced ahead of a run (LSP + `smelt explain`), not only
   via the maintenance driver's I/O path.
7. A docs-site migration guide ships: `docs-site/docs/guide/backbuild-synthesis.md` rewritten in
   place around `smelt migrate`/`--apply`, its "no CLI command yet" and naming-collision
   callouts removed; `models.md`/`seeds.md`'s "no `smelt migrate`" divergence bullets removed or
   reworded to precisely what `smelt migrate` doesn't cover.
8. `/smelt:validate definition_deltas` reports no drift; every bullet this outcome closes is
   removed from the spec, not just addressed in code. All standing gates green, including the
   extended conformance suite, `statement_parity`, and `walk_coverage`.

## Out of scope

Everything the 2026-08-15 widening pulled in and the programme reassigns elsewhere: scheduler
delta-signature consumption and `smelt explain` headline (programme outcome 2), per-cell frontier
addressing, write-pin equivalence, observed-delta consumption, plan-consumer/graph-layer/proof
residues, conditional-maintenance gaps, key-grain validation gaps (keyed/partition residue
outcomes), and every `(Open Question)` product decision (decision track). See the handoff.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Wire `smelt migrate` (plan-only): CLI verb invokes the backbuild synthesis layer end to end and prints the per-group verdict/technique plan | planned |
| 2 | Approval store + `--apply`: plan-hash persistence, hash-mismatch/staleness refusal, CI exit codes | pending |
| 3 | Rename `smelt backbuild` → `smelt rebuild` across CLI, docs-site, examples, tests, and the spec sweep named in success criterion 3 | pending |
| 4 | Conformance harness gains a definition-edit step kind; wire into the generative equivalence suite | pending |
| 5 | Close the atomicity divergence (unify the `schema_evolution` full-refresh escape with the migration gate, or land its repair path) | pending |
| 6 | Diagnostic rename lands in code; surface ahead of a run via LSP and `smelt explain`; sibling-spec sweep | pending |
| 7 | docs-site migration guide: rewrite `guide/backbuild-synthesis.md` in place; update `models.md`/`seeds.md` bullets | pending |
| 8 | Validate + close out: `/smelt:validate definition_deltas` clean, Known Divergences bullets removed, full standing-gate sweep | pending |

## Decision log

- **Inherited from `20260815-definition-delta-migrate` phase 1 (done, 2026-08-15).** Plan-hash
  scope: hash the plan data structure the emitters consume (verdicts, techniques, input facts —
  source declarations, backend capabilities), not only rendered SQL; exclude region
  *enumeration*, resolved at apply time from the frontier so `--apply` stays reachable on an
  actively-loading warehouse. Diagnostic rename: `MaintenanceSkeletonColumnAdded` →
  `MaintenanceSkeletonChanged`, one code, not an add/changed split. Both already landed in
  `docs/specs/definition_deltas.md` §Design / §Known Divergences; this outcome implements
  against them.
- **2026-08-16 (phase 1 planning).** No phase reshape. Scoped into phase 1: state records only a
  `definition_hash`, never the definition text, so there is no "before" side to diff. The recorded
  definition SQL is persisted in the per-model deployed-schema snapshot
  (`.smelt/targets/<target>/schemas/<model>.json`, a `#[serde(default)]` field so pre-existing
  snapshots read back as "no recorded definition" — fail-closed). Spec §Detection already promises
  smelt records the definition the table was last maintained under, so this implements the spec
  rather than widening scope. Also decided: plan *derivation* is pure data in `smelt-logical`
  (`backbuild/plan.rs`), fact assembly lives in `smelt-runtime` (so the UI can consume it), and the
  CLI is a renderer only — keeping maintenance-plan purity and CLI↔UI parity intact.

## Blocked

_(empty)_
