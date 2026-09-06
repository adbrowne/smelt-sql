# Outcome: Every defect the real pipeline hits on BigQuery is fixed, and DuckDB and BigQuery agree

**Created:** 2026-09-06
**Status:** queued
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D2), §"Sequencing: models first, punch-list second", §"Findings already banked"
**Spec anchors:** `docs/specs/multi_backend.md` §"Operator lowering", §"Statement-level lowering", §"Output-schema type conformance", §"Cross-engine emission audit"; `docs/specs/architecture.md` §"Constraints & Invariants" item 14; `docs/reference/dialect-coverage.md`

## The outcome

The BigQuery defects that a real pipeline actually reaches are fixed, and each fix is held
by a gate that would have caught it. The unconditional ones — wrong for any model on any
run — are fixed first and without waiting for evidence: `emit_fingerprint_digest_select`
ignores its `dialect` parameter and hardcodes DuckDB's hash spelling on every backend.
Everything after that is driven by
`docs/outcomes/20260906-bigquery-dogfood-spine`'s findings handoff: only the registry
entries, techniques, grains and capability rows the spine's models genuinely hit are
built, and each one lands with its emission verdict, its ledger row, and its coverage in
the published dialect table. Where DuckDB and BigQuery disagree on the same rows, the
difference is either fixed or registered with a reason — never tolerated silently.

## Success criteria (checkable)

1. **The unconditional fix.** `emit_fingerprint_digest_select`
   (`crates/smelt-logical/src/maintenance/emit.rs`) threads its `dialect` through to
   `row_fingerprint_expr` instead of passing `MaintenanceDialect::DuckDb`; a unit test per
   dialect asserts the emitted expression (BigQuery gets `TO_HEX(SHA256(…))`, since
   GoogleSQL's `SHA256` returns `BYTES` and the value feeds a `STRING_AGG`). Whether the
   path is reachable on a live `mutable_snapshot` run is answered in the decision log
   either way — the fix does not depend on the answer.
2. **Punch-list harvested, not invented.** Phase 1 reads the spine's findings handoff and
   rewrites this outcome's remaining phase rows from it. A row whose only justification is
   "issue #179 lists it" and which no spine model reaches is recorded under Out of scope
   with that rationale, per §"Sequencing".
3. **Every fixed construct is gated.** Each defect fixed here gains coverage in the gate
   that owns its class — a `signatures.rs` emission verdict plus a `dialect_audit` probe
   for a spelling, a `ledger.rs` row for a registered mismatch, a `dialect_seam` case for
   a compile-time refusal, a `projection_dialect_invariance` case for a projection bug —
   so a regression fails offline. No fix lands with only a manual sweep behind it.
4. **Ratchets move the right way.** `.claude/dialect-gaps-baseline.txt` and
   `.claude/parser-gaps-baseline.txt` fall or hold; neither is raised. Registry entries
   this outcome gives verdicts to leave `#179`'s unverified count lower, and
   `docs/reference/dialect-coverage.md` is regenerated so the doc-sync gate is green.
5. **Cross-target agreement.** Every divergence the spine registered under its criterion 6
   is resolved here: fixed, or promoted to a permanent, reasoned entry in the divergence
   registry with the engines and the construct named. The count of unexplained differences
   is zero.
6. **The two known live conformance failures are characterised.**
   `dags_bigquery::diamond_propagation_suffices` and
   `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence` are each either fixed
   or explained in the handoff with the mechanism named — not left uncharacterised.
7. **Gates green.** `bash .claude/scripts/verify-phase.sh`, plus
   `cargo test -p smelt-dialect --test emission_ownership`,
   `cargo test -p smelt-runtime --test dialect_seam`,
   `cargo test -p smelt-runtime --test projection_dialect_invariance` and
   `cargo test -p smelt-db --test dialect_audit` (DuckDB legs in-process). The BigQuery
   value leg is a manual sweep (`scripts/bigquery-dialect-audit.sh`) — a phase that needs
   it and cannot run it emits `<<PHASE_BLOCKED>>` rather than skipping green.

## Out of scope

- Building the 42 no-verdict BigQuery registry entries of issue #179 speculatively. Only
  entries a spine model reaches are built here; the rest stay on #179.
- Snowflake, Redshift and Postgres emission work.
- Any new model, source or feature — this outcome only fixes what the spine surfaced.
- Widening the sample, the model set, or the pipeline's scope (the spine owns that).
- Retiring the PostgreSQL emission dialect (tracked separately by the
  dialect-emission-vocabulary outcome).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | The unconditional fix: thread `dialect` through `emit_fingerprint_digest_select` to `row_fingerprint_expr`, per-dialect unit tests, and answer in the decision log whether the path is reachable on a live `mutable_snapshot` run | pending |
| 2 | Harvest: read the spine's findings handoff and rewrite the remaining phases from it, moving anything not reached by a spine model to Out of scope with its rationale | pending |
| 3 | (written by phase 2) | pending |
| 4 | Resolve every cross-target divergence the spine registered: fix, or promote to a reasoned divergence-registry entry naming engines and construct | pending |
| 5 | Characterise or fix the two known live conformance failures (`diamond_propagation_suffices`, `composed_keyed_pool_upholds_equivalence`) | pending |
| 6 | Close: regenerate `docs/reference/dialect-coverage.md`, move the gap ratchets down, update issue #179 with what was verified, all standing gates green | pending |

## Decision log

- 2026-09-06 (scaffold): **deliberately near-empty.** Phase 3 is a placeholder the phase-2
  planner rewrites. This is the outcome loop's just-in-time planning used as intended, and
  it is the mechanism by which §"Sequencing"'s "let the real models generate the
  punch-list" is enforced rather than merely intended.
- 2026-09-06 (scaffold): **phase 1 runs before the spine finishes.** The fingerprint-dialect
  defect is wrong for any model on any backend, so it is not gated on evidence. It is
  ordered first so the loop has real work the moment this outcome is reached, even if the
  spine is still mid-flight.
- 2026-09-06 (scaffold): the BigQuery value leg cannot run in the loop's environment. A
  phase that needs it must block rather than skip — the same rule the active-plan pointer
  states for the Spark legs, for the same reason (a silently skipped live leg is a hole
  that reads as green).

## Blocked

(none)
