# Phase 5 — Declared facts reach every `JoinContext`-taking maintenance-cell route

## Objective

Advance success criterion 3. Two cell-admission routes build their join facts from nothing today:
`append_model_edge_cells` derives its P1 closure over model edges only, with a hard `None`
referential-integrity input and a `JoinContext` carrying no external source's declared
`unique_key`; `repair::admit_per_group_recompute` builds `AffectedKeyContext` with a literal
`JoinContext::new()`. Both get the declared facts their sole production caller already holds, with
a standing structural check that no production route silently rebuilds an empty context.

## Spec delta (spec-first — the implement step makes these edits before the code)

- `docs/specs/model_properties.md` §"Skeleton-source closure" — state that a model-edge cell's P1
  verdict is an AND over **every enrichment relation joined in the scope**: each upstream model
  edge *and* each external source joined there, every one judged with its own declared facts
  (`unique_key` for conjunct 3, `referential_integrity` for conjunct 4). Unproven external
  enrichment ⇒ the shared verdict is `Open`, as for an unproven edge.
- `docs/specs/sources.md` §Known Divergences (the `referential_integrity` bullet, ~line 355) —
  delete the now-false trailing claim that "today only the source-enrichment `UpstreamMutation`
  route ever derives a declared-route closure; a model-edge creation cell's own closure is always
  derived against an empty referential-integrity map", replacing it with the model-edge route's
  real behaviour. (`model_properties.md`'s own MP-11 bullet is phase 6's deletion.)
- Do **not** touch `model_properties.md` §Semantics' closure-pruning paragraph: that pass's
  declared-RI exclusion stays deliberate (see the outcome's Out of scope).

## Tests (red → green)

1. `model_edge_cell_closure_consults_declared_source_ri` (`crates/smelt-logical/tests/maintenance_referential_integrity.rs`)
   — downstream over a model edge plus an inner-joined dimension source: the appended cell's
   `skeleton_source_closure` is `Closed { DeclaredReferentialIntegrity { source } }` **only** when
   the RI map carries that source; the same inputs with an empty map stay `Open`. (Criterion 3's
   per-route fixture for the model-edge route.)
2. `model_edge_closure_open_when_external_inner_join_unproven` (same file) — every model edge is
   closed but an inner-joined dimension declares no RI: the shared verdict is `Open` naming that
   source. Fail-closed leg of the new AND.
3. `model_edge_join_context_carries_source_unique_keys` (same file) — a fan-out-sensitive verdict
   (row identity / one-to-one conjunct) on a model-edge cell resolves only because the joined
   external source's declared `unique_key` is now in the shared context.
4. `per_group_recompute_admits_only_with_declared_join_context` (`crates/smelt-logical/tests/repair_cell.rs`)
   — a delta whose affected-key discovery must chase through a joined dimension:
   `RepairRefusal::KeysNotDiscoverable` with an empty context, `Ok(keys)` with the real one.
5. `no_production_route_builds_an_empty_join_context` (new `crates/smelt-logical/tests/join_context_reach.rs`)
   — structural: every `JoinContext::new()` in `src/maintenance/` and `src/analysis/` production
   code is either inside a documented context *builder* or named in an allow-list whose entries
   each carry a one-line reason (the site reads no context-dependent field of the vector). Fails
   on a new unlisted site.

## Tasks

1. Make the two spec edits above.
2. Extend `append_model_edge_cells` with `sources: &[SourceFacts]` and
   `source_referential_integrity: &SourceReferentialIntegrity` parameters.
3. Build the shared context as `model_edges_join_context(sql, edges)` unioned with
   `source_facts_join_context(sql, sources)`; keep it the single context both the row-identity
   proof and the closure proof see (that invariant is already documented at the call site — extend
   the doc comment rather than adding a second context).
4. Widen `model_edge_enrichment_closure` to fold each external source actually joined in the scope
   (`enrichment_join_alias`) into the same AND, calling `skeleton_source_closure` with that
   source's declared RI (`source_referential_integrity.get(name)`), mirroring
   `mutation_enrichment_closure`'s `None`-vs-attempted distinction exactly.
5. Give `repair::admit_per_group_recompute` a `join: &JoinContext` parameter (matching its sibling
   `admit_key_addressed_recompute`) and build the real context at `derive.rs`'s single production
   call site from `inputs.sources`.
6. Update the one production caller (`smelt-db` `derive_model_maintenance_plan_with_edges`, which
   already holds both `sources` and `source_referential_integrity`) and the ~16 test call sites.
7. Add the tests above; then run the conformance gates and triage every fixture whose technique or
   verdict moves: a flip is admissible only as a documented fail-closed correction (the new AND) or
   a declared-fact widening (the enlarged context) — never accepted unexplained.
8. Record in the phase summary any route the audit found that still passes an empty context, with
   the reason it is legitimate.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `rg -n 'JoinContext::new\(\)' crates/smelt-logical/src` — every survivor allow-listed by test 5.

## Commit message

`feat(logical): model-edge and repair admission routes see declared unique-key and RI facts`
