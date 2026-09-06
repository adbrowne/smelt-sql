# Outcome: An externally-produced relation is a node in smelt's DAG

**Created:** 2026-09-06
**Status:** queued
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"Black-box steps in the DAG", §Open questions 2
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/models.md`; `docs/specs/model_selection.md`; `docs/specs/run_state.md`; `docs/specs/diagnostics.md`

## The outcome

A relation smelt does not author but does depend on — the GitHub-activity loader is the
motivating case, but any externally-produced table is the same shape — is declared as a
**black-box step**: a node smelt orders in the DAG, invokes when a run reaches it, and
treats as the producer of a declared source. smelt never authors its SQL and never
inspects its internals; the source declaration is the whole contract for what it produces
and where. A run that selects a downstream model runs the step first; a run that cannot
invoke it says so with a named diagnostic rather than reading a stale table silently. The
step's failure is a run failure, its success advances the source's frontier, and both are
visible in the run report.

## Success criteria (checkable)

1. **Spec first.** `docs/specs/sources.md` gains the normative surface — the declaration
   shape, what smelt guarantees (ordering, invocation, failure propagation) and what it
   explicitly does not (authorship, retries beyond the existing policy, idempotence of the
   external program) — written to the timeless-oracle rule. The open question the research
   doc leaves (a `produced_by:` key on today's source YAML, versus a distinct declaration
   kind) is decided in this outcome's decision log with reasoning, before any code.
2. **Declaration and validation.** The declaration parses, and every malformed form is
   refused with a named `DiagnosticCode` exercised by a fixture under `examples/broken/`
   — never a silent default, per the fail-loud discipline. `diagnostics_catalogue` green.
3. **DAG membership.** The step is a node: `smelt run` selecting a downstream model runs
   it first; `smelt list` and the DAG/graph surfaces show it; model selection
   (`docs/specs/model_selection.md`) reaches it through the same selectors as any node.
4. **Invocation and failure.** A run invokes the step, propagates a non-zero exit as a run
   failure with the step named, and leaves downstream models unbuilt. A run that is not
   permitted to invoke it (no command, dry run, or an environment that cannot) refuses
   with a named code rather than proceeding against a possibly-stale table.
5. **Explain.** `smelt explain` (text and `--json`) renders the step: what it produces,
   how it is invoked, and that smelt does not author it. `cli_docs_coverage` green.
6. **Fixture and docs.** An example workspace carries a black-box step and a model reading
   its source, with zero diagnostics (`example_diagnostics`, `example_workspaces`); a
   docs-site page documents the declaration, the contract, and the failure modes.
7. **Gates green.** `bash .claude/scripts/verify-phase.sh`; `execute_parity`;
   hardening ratchets unmoved.

## Out of scope

- Authoring, generating, or type-checking the external program.
- Scheduling it independently of a smelt run (Cloud Scheduler and friends belong to
  `20260906-bigquery-unattended`).
- Incremental reasoning *about* the step's internals — smelt observes what the source
  declaration claims, exactly as it does today for any source.
- Retention semantics for what the step produces — owned by
  `20260906-trimmed-history-sources`.
- A general plugin/hook system. This is one declaration kind for one contract.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Decide the declaration shape (`produced_by:` on a source vs. a distinct kind) with reasoning in the decision log, then land the spec delta in `docs/specs/sources.md` | pending |
| 2 | (written by phase 1's planner from the spine's requirements) | pending |

## Decision log

- 2026-09-06 (scaffold): **deliberately short.** Only the spec-decision phase is written.
  The phase list is completed by the phase-1 planner once
  `docs/outcomes/20260906-bigquery-dogfood-spine` has produced a real loader and its
  findings handoff names what the contract actually has to carry. Scaffolding a full phase
  table now would freeze the shape against an imagined loader — the failure mode the
  outcome loop exists to avoid.
- 2026-09-06 (scaffold): open question for the human, to settle in phase 1 —
  the research doc's §Open questions 2. A `produced_by:` key reuses the source's existing
  contract surface and costs one field; a distinct declaration kind separates "a relation
  someone else fills" from "a relation smelt builds" at the type level. The spine's loader
  is a scheduled BigQuery query, which is close enough to a source that the cheap answer
  may be right — but the research doc notes the feature generalises well past ingest, and
  a key on a source cannot express a step producing several relations.

## Blocked

(none)
