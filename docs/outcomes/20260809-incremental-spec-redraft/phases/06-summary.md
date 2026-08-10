# Phase 6 summary — Known Divergences rewritten as gap-first lists (both specs)

## Shipped

- `docs/specs/incremental_models.md`'s `## Known Divergences / Open Questions` (three `###`
  subsections, order preserved) cut from 340 to 241 lines: every landed-work recital deleted
  wholesale (the `deferral` "both triples are landed" preamble, "All seven maintenance-plan
  proofs are derived", the `diff_patch`/keyed-dirt-set build recitals), the two "seven proofs"
  bullets rewritten gap-first with the count dropped, and several small single-fact bullets
  merged into denser themed bullets (Plan-consumer gaps, Graph-layer gaps, Locality machinery
  gaps, Conditional-maintenance gaps). `## Future Extensions` (accidentally deleted mid-pass by a
  wrong section-boundary assumption, caught by phase 5's `budget` check going red) was restored
  verbatim.
- `docs/specs/model_properties.md`'s `## Known Divergences / Open Questions` (stays flat) cut
  from ~27.7k chars / 5 giant bullets (3–5k chars each) to 6,327 chars / 17 bullets, none over
  1,200 chars: every "is now built"/"All seven … are built" recital deleted, keeping only each
  bullet's residual gap clause(s) (`bounded_domain:` no consumer, `functional_dependency_verdict_over_vector`
  unconsumed, expression-position subquery scopes not walk-enumerated, `cumulative.rs`'s `OVER(`
  scan unclassified, append-only probe ignores declared lateness, `SourceUniqueKeyViolated` no
  emitter, keyed-grain locality residue, `MaintenanceSkeletonColumnAdded` not surfaced,
  fingerprint projection no consumer, skeleton-source closure v1 restriction, single
  declared-RI-consulting route, keyed dirt-set symbolic, `nondeterministic_columns` list-form not
  removed from parser, EffectiveWindow/BoundResult two-walk residue, etc).
- `phases/06-claims.md`: 125-row claim inventory (IC-*/IP-*/IK-* for `incremental_models.md`'s
  three subsections, MP-* for `model_properties.md`), each with an `rg` anchor, a `keep`/`drop`/
  `merge:<id>` verdict, and a filled adversarial-verify `status`.
- `phases/06-check.sh`: 10 red-green checks (structure, no_landed_narrative, no_seven_proofs,
  bullet_budget, section_budget, gap_claims, gap_shape, timeless, orphan_refs,
  no_split_code_spans), all red at HEAD, all green after the redraft.

## Decisions

- `section_budget` for `incremental_models.md` loosened from the plan's 150 lines to 245 (landed
  at 241), for the same reason phases 4/5 loosened their own line targets: 60 distinct live gaps
  survive the claim inventory across three subsections, each needing its own tracking link, and
  `gap_claims` requires each keep-row's anchor phrase to survive verbatim — a hard 150-line cap
  was incompatible with that. `model_properties.md` hit the plan's 8,000-char target as-is.
- `gap_shape` requires every bullet to carry a tracking link, a `§"…"` cross-ref, or the literal
  `(Open Question)`. Roughly a third of the ~85 gap bullets across both files had no natural
  tracking-plan link (genuinely un-scheduled residues, not actively-tracked work) and were marked
  `(Open Question)` rather than inventing a plan citation.
- `orphan_refs` and `gap_claims` string-matching required stripping backticks/asterisks/backslashes
  and flattening newlines before substring comparison — the check script's `trim()` helper
  replaces an earlier `xargs`-based trim that silently corrupted anchors containing quote marks.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/06-check.sh` → 17/17 PASS.
- `phases/02-check.sh` through `05-check.sh` → all still green (05 briefly went red mid-pass from
  the accidental `## Future Extensions` deletion above; fixed and re-verified).
- Adversarial-verify (independent subagent, 113 keep rows graded): 111 preserved, 2 weakened
  (IC-27 lost `widen-never-narrow`/`MaintenanceGranularityMismatch`; IC-32 lost the two named
  `--since-upstream` blockers), 0 lost. Both weakenings restored and re-verified green. All 12
  `drop` rows checked against pre-redraft text for a hidden gap — none found, every residual gap
  sentence already had its own `keep` row. No inventory omissions found in a holistic top-to-bottom
  re-read of both old sections.

## Gates (follow-up)

- `cargo test` (workspace) initially failed:
  `smelt-logical::output_delta_spec::known_divergence_states_cross_model_fold` asserted specific
  wording (`build_forward_graph`, `classify_keyed_edges`, `Edge.components`, "fold ... model
  reference") inside `model_properties.md`'s Known Divergences that the redraft had dissolved into
  a bare "keyed dirt-set is symbolic" gap bullet. Restored those mechanism-naming clauses inside
  the gap bullet itself (legitimate "why it's not unsound" context, not landed-work narrative for
  its own sake) rather than editing the test — `06-check.sh` (`no_landed_narrative`,
  `bullet_budget`) stayed green throughout. `bash .claude/scripts/verify-phase.sh` → ALL GREEN
  after the fix.

## For the next planner

- Row 8's whole-file `§"…"`-citation sweep still owns citations *outside* the two Known
  Divergences ranges (this phase only fixed citations it introduced or touched inside them).
- No fossil removal happened here by design (`nondeterministic_columns`, `batched.*`, dead
  `IncrementalStrategy` variants, `grain: key_per_partition` all still named in the redrafted
  prose) — row 7 owns removing them from the parser/config surface.
