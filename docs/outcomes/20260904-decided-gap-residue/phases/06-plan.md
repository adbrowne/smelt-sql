# Phase 6 plan — close the bullets, sweep the residue, prove the gates

**Objective.** Finish the outcome's paperwork half: rewrite the one spec bullet that phase 5
deliberately left stale (`incremental_models.md` §Known Divergences "Conditional-maintenance
gaps"), correct the user-facing page that still calls the sidecar DuckDB-only by dialect, and
run the drift + gate sweep. Advances criteria 5 (its bullet must reflect the shipped target)
and 6 (validate reports no drift for the closed bullets; conformance/parity/verify green).

## Spec delta (first, per spec-first rule)

1. `docs/specs/incremental_models.md` §Known Divergences, bullet **"Conditional-maintenance
   gaps"** (line ~2130). Rewrite so it states, per what phase 5 shipped:
   - the decider is `BackendCapabilities::supports_fingerprint_sidecar`, read by **every**
     sidecar consumer — never re-derived from the dialect;
   - a flagless target has **two** consequences, not one: the external-delta-restriction leg
     keeps the widened-scan recompute; the repair-family / key-addressed-model-edge group-grain
     leg refuses with `UnsupportedOnBackend` (a clamped current-source scan over a
     `mutable_snapshot` would be unsound, not merely wider);
   - the residual divergence is now only that DuckDB alone declares the flag today (the DDL
     owner is DuckDB-shaped), pointing at `multi_backend.md` §"The fingerprint sidecar
     capability" as the authority.
   Keep the existing plan/outcome refs; add `docs/outcomes/20260904-decided-gap-residue/`.
2. `docs-site/docs/guide/incremental-models.md` (~line 839, the fingerprint-sidecar paragraph):
   replace "DuckDB-only today … a Spark or BigQuery target keeps the widened-scan recompute for
   that cell" with the capability-flag framing and both legs, in user voice (a target that does
   not declare the capability; widened scan for the dimension-recompute cell, refusal for the
   repair/model-edge case). Timeless-oracle rule applies — no phase or plan vocabulary.

No behaviour change: this phase writes no production code unless a gate turns something up.

## Tests

Doc/spec-shaped phase; the "red" is a grep-shaped assertion plus the existing suites.

- `residue_grep` (ad hoc, recorded in the summary) — `rg` finds no remaining production or spec
  claim that the fingerprint sidecar is gated on `SqlDialect::DuckDB` rather than on
  `supports_fingerprint_sidecar`, outside `smelt-state`'s DuckDB-shaped DDL owner and the
  four non-sidecar dialect gates phase 5 named (sites ~526, 1956, 3486, 3609).
- `cargo test -p smelt-runtime --test fingerprint_sidecar` — unchanged, must stay green after
  any wording-driven doc-comment edits.
- `cargo test -p smelt-cli --test maintenance_conformance` — unchanged, proves the closed gaps
  did not disturb the equivalence oracle.

## Tasks

1. Rewrite the `incremental_models.md` "Conditional-maintenance gaps" bullet per the spec delta.
2. Rewrite the `docs-site/docs/guide/incremental-models.md` sidecar sentence per the spec delta.
3. Run the `residue_grep` sweep; if it finds a stale doc comment or spec sentence inside the
   crates, fix the wording (no behaviour change) and note it in the summary.
4. Confirm the three `docs/TODO.md` bullets this outcome owned ("Frozen-horizon append-only
   gate", "Deferral oracle restatement", "Sidecar per-consuming-edge audit") are absent — they
   were removed by phases 1/2/4; if any survived, remove it now.
5. Run `/smelt:validate` for `incremental_models`, `incremental_shapes`, `sources`, reading the
   drift report **only** for the five closed bullets. Fix drift attributable to this outcome's
   gaps. Record — do not chase — drift belonging to other tracks; list it in the summary so a
   later outcome can pick it up. Phase 3's blocked generative-pool clause is expected to show as
   residual and is correctly described by the rewritten `incremental_shapes.md` bullet.
6. Run the verification gates below and capture their tails.
7. Write `phases/06-summary.md`: per-criterion status (1,2,4,5 closed; 3 closed except the
   blocked generative-pool clause; 6 evidenced), the validate residue list, and a one-line
   recommendation on whether the outcome can be marked `done` at the next plan step given the
   blocked row.

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing gate. It exceeded the foreground budget
  in phase 5; run it in the **foreground** with an extended timeout, and if it still cannot
  finish, run its four legs separately (`cargo fmt --all -- --check`,
  `bash .claude/scripts/clippy-gate.sh`, `cargo test --quiet`,
  `cargo test -p smelt-cli --test example_diagnostics`) and say so explicitly in the summary.
  Never background a gate whose result this phase depends on.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --test fingerprint_sidecar --quiet 2>&1 | tail -20`
- Docs-site build/lint if the guide edit touches structure (`docs-site/` sync gate).

## Commit message

`docs(incremental): close the decided-gap residue bullets and correct the sidecar capability framing`
