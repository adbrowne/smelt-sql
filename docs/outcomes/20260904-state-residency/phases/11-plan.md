# Phase 11 plan — Validate + close out

## Objective

Run `/smelt:validate state` end to end, fix every drift item it surfaces that this outcome owns,
and confirm all standing gates are green. This closes success criterion 8 (divergences rewritten,
`/smelt:validate state` clean, docs-site pages updated, all gates green) and is the evidence base
the next plan step uses to mark the outcome `done`.

## Spec delta

No new normative content. `docs/specs/state.md` §Known Divergences was already reduced to "none
currently open" in phase 10 — **do not redo it**. The expected edits here are *bookkeeping* on
the spec's own metadata, which phase 10 left stale:

- §References → **Code**: add `crates/smelt-logical/src/maintenance/availability.rs` (the pure
  availability-resolution owner landed in phase 4) and the `smelt-core` `warehouse_tables` parse
  site if not already implied.
- §References → **User docs**: currently `none yet` — phase 10 rewrote
  `docs-site/docs/reference/state.md`, `smelt-yml.md` §"State Configuration",
  `guide/targets.md`, `guide/incremental-models.md`, `reference/smelt-explain.md`. List them.
- §References → **Plans (history)**: currently `none yet` — point at
  `docs/outcomes/20260904-state-residency/outcome.md`.
- Front-matter `last_reviewed:` → `2026-09-05`.

Only add further spec edits if validation surfaces genuine drift; anything that is a *code* change
rather than a doc change must be weighed against §Out of scope before acting (see Tasks step 4).

## Tests

- `state_docs_freshness` (existing, `crates/smelt-cli/tests/state_docs_freshness.rs`) — must stay
  green; extend only if validation finds a docs claim it does not cover.
- `spec_references_are_live` (new, in `crates/smelt-cli/tests/state_docs_freshness.rs`) — every
  path listed in `docs/specs/state.md` §References → Code and → User docs exists on disk, and
  neither list still reads `none yet`. Red before the References edit, green after; this is what
  keeps the closed-out spec from silently rotting.

## Tasks

1. Read `docs/specs/state.md` in full — it is the oracle for the rest of this phase.
2. Execute the `/smelt:validate state` process (steps 1-6 of `.claude/commands/smelt/validate.md`):
   automated checks, Surface drift vs code + `docs-site/`, Semantics drift vs tests, invariant
   drift, timeless-oracle grep (`grep -nE "Phase [A-Z0-9]+" docs/specs/state.md` and over the
   docs-site pages the spec references), freshness.
3. Write the drift report to `docs/validations/2026-09-05-state.md` (persisted form of step 7).
4. Triage each drift item into: (a) doc/bookkeeping fix — do it now; (b) criterion-serving code
   gap — do it now, it may not leave the outcome; (c) already listed under the outcome's
   §Out of scope — record in the report, do not fix. If a (b) item is too large for this phase,
   stop and report it in the summary rather than deferring silently.
5. Land the §References + `last_reviewed` edits above, red-green against the new
   `spec_references_are_live` test.
6. Run the full standing-gate sweep (Verification) and record each result verbatim in
   `phases/11-summary.md`, since criterion 8 is an evidence claim about exactly these gates.
7. Flip the phase 11 row to `done` in `outcome.md` and append a dated decision-log line naming the
   drift-report path and the gate results. Leave `**Status:**` as `active` — the next plan step
   makes the completion judgement.

## Verification

- `bash .claude/scripts/verify-phase.sh` (mandatory)
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test state_docs_freshness`
- `grep -nE "Phase [A-Z0-9]+" docs/specs/state.md` → no hits outside §Known Divergences/§References
- `rg -n "reconciliation\.json" crates/ docs-site/ docs/specs/` → no hit asserting `.smelt/`
  residency (the `run_state.md` legacy-migration sentence is the one allowed hit)

## Commit message

`docs(state-residency): validate state spec, refresh references, close phase 11`
