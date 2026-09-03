## Drift Report: model_properties

**Spec**: docs/specs/model_properties.md (last_reviewed: 2026-08-16 at audit time; bumped to 2026-09-04, see Surface drift)
**Date**: 2026-09-04

### Automated checks
- cargo fmt — PASS (cited from prior run this outcome; not re-run)
- cargo clippy — PASS (cited from prior run this outcome; not re-run)
- cargo test — PASS (cited from prior run this outcome; not re-run)
- example_diagnostics — PASS (cited from prior run this outcome; not re-run)

### Surface drift
- ✅ Derived-proofs table (26 rows) — each has a corresponding function/type in
  `crates/smelt-logical/src/analysis/{monotonicity,source_bounds,temporal,model_diff,faithful_fold,footprint,locality_projection,definition_change}.rs` and `crates/smelt-logical/src/rules/{incremental,cumulative}.rs`; spot-checked `trace_event_time`, `derive_model_bounds`, `resolve_join_driving_fact`, `row_identity`, `output_delta_shape`, `skeleton_source_closure`, `fingerprint_projection`, `reflect_footprint`, `locality_verdict`, `faithful_fold`, `grain_alignment`, `classify_definition_change` — all present with matching signatures.
- ✅ Model-scoped declarations (3 rows) and probe registry (9 rows) — probe emitters `emit_monotonicity_probe`, `emit_functional_dependency_probe`, `emit_bounded_domain_probe`, `emit_append_only_posture_probe`, `emit_count_preservation_probe`/`_from_body`, `emit_recurrence_bound_probe` all found in the maintenance layer; `SourceUniqueKeyViolated` correctly has no emitter (matches its own Known-Divergences bullet).
- ❌ **Surface row "Event-time monotonicity trace" (L26) and §Semantics "Event-time monotonicity trace" understated the offset domain.** Spec text said `offset` = `Seconds` \| `Symbolic`, produced by folding constant `INTERVAL` shifts. Code (`crates/smelt-logical/src/analysis/monotonicity.rs` / `source_bounds.rs::Offset`, commits `98393e25`/`cc75fe58`, 2026-09-04) has shipped and tested a third `Offset::Integer(i64)` variant — a constant integer shift over a monotone, non-temporal partition key (`batch_id + 5`) — that folds into the same trace chain (`combine_offset`). This is the spec text lagging shipped, tested behaviour (not a code bug), so it is doc/wording drift under the phase-4 standing rule. — fixed this phase: reworded the Surface table row and the §Semantics "Event-time monotonicity trace" paragraph to name `Offset::Integer` and its `incremental_models.md` cross-reference, and bumped `last_reviewed` to 2026-09-04.
- ✅ docs-site — no standalone user page exists for this spec (per spec's own §References → User docs: "internal to the analysis layer"); confirmed no docs-site page claims ownership of any Surface row inconsistently.
- ✅ No docs-site page documents a model-properties capability absent from the spec's Surface table.

### Semantics drift
- ✅ Probe obligation — `crates/smelt-logical/tests/probe_obligation.rs` (registry gate) and `crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs` (per-row conformance pool) both exist, matching §References → Tests.
- ✅ Composition walk / transfer-rule table (Output-delta shape) — `crates/smelt-logical` walk machinery present; not independently re-derived line-by-line against every transfer rule in this pass (see Invariant drift note below).
- ✅ Skeleton-source closure, faithful-fold, footprint, locality, definition-change — each has a same-named source file and unit tests colocated in the same module (verified file existence + public fn signatures matching spec names).
- ⚠️ Per-column mutation-sensitivity "Across set-operation arms" (L161-180) — detailed per-operator combination rules (UNION ALL/UNION/INTERSECT/EXCEPT) were not individually re-verified against test cases line-by-line this pass; spec's own Known Divergences (MP-06, flagged-open) already documents the adjacent INTERSECT/EXCEPT filter-distribution gap as narrowed-but-accurate per baseline-inventory.md, so this is flagged as unverified-but-not-newly-suspect rather than a fresh finding.

### Invariant drift
- ✅ **Fail-closed proofs** — spot-checked `NotTraceable`/`Unbounded`/`NotDerivable`/`NotAligned`/`Open`/`FullRow` reject-verdict constructors exist and are the documented defaults in the reviewed files.
- ✅ **Property composition walk rule** — `cargo test -p smelt-logical --test walk_coverage` (read in full) is a structural gate requiring every `.contains("` raw text scan in `analysis/`, `rules/`, `maintenance/`, `backbuild/` to carry a `Leaf classifier`/`Advisory heuristic` doc-comment tag, with `rules/cumulative.rs` as the one named, doc-linked exception — this exactly matches the spec's own Known-Divergences bullet about `cumulative.rs` (flagged-open) and the Constraints bullet "Composition happens in the walk, not in scans." The gate covers what the spec claims: it does not prove every *composition-relevant* verdict is walk-computed (that's a design property, not grep-checkable), but it does enforce the "leaf classifier / advisory heuristic, tagged" discipline on every raw scan in the covered directories, which is the checkable half of the invariant.
- ✅ **No narrowing declaration without its probe** — probe registry table's `not-yet`/`exempt` rows (`SourceUniqueKeyViolated`, `horizon_ceiling:`, `columns.<c>.contract: plausible`) each carry a stated reason, matching the constraint's own carve-out language.
- ✅ **"This spec is the complete catalogue"** — the one gap found (`Offset::Integer` undocumented) is now closed by the Surface drift fix above; no other uncatalogued discriminant found in this pass.

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage detected in spec body: `rg -n "Phase [A-Z0-9]" docs/specs/model_properties.md` returned no matches.

### Freshness
- last_reviewed (at audit time): 2026-08-16
- most recent code change: 2026-09-04T05:52:17+10:00 at `crates/smelt-logical/src/analysis/source_bounds.rs` (also `temporal.rs` 2026-09-04T02:54:53, `analysis/mod.rs` 2026-09-04T04:32:39, `rules/cumulative.rs` 2026-09-03T21:19:51 — all after `last_reviewed`)
- Verdict: was **stale** (the `Offset::Integer` gap above); the content that made it stale is fixed this phase and `last_reviewed` is bumped to 2026-09-04. — fixed this phase

### Summary
- Drift items: 1 total — 1 surface/semantics doc/wording gap, fixed inline this phase. 0 blocked-on-decision items, 0 phase rows added, 0 items required re-litigating baseline-inventory.md (MP-06 cited as flagged-open, not drift).
- Inline edits made to `docs/specs/model_properties.md`: Surface table "Event-time monotonicity trace" row, §Semantics "Event-time monotonicity trace" paragraph, `last_reviewed` frontmatter.
- Recommended next step: none — no further drift outstanding for this spec beyond baseline-inventory.md's already-flagged-open bullets.
