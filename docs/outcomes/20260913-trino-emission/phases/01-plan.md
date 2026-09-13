# Phase 1 — Spec delta: Trino becomes a fourth stated dialect in `multi_backend.md`

**Status when planned:** pending → planned (2026-09-14)

## Objective

Write the normative statement of Trino's emission surface *before* any verdict or probe
exists, so phases 2–10 implement against a spec rather than backfilling one. Advances
success criteria 1 (the implicit-`Native` hole is named as a gate obligation), 2 and 3
(the divergences and the `PIVOT` decision get their normative home), 5/6 (the audit's
two legs and Trino's CI tier are stated), and 12 (a standing freshness gate, no ratchet
touched). Spec-only phase: no production code changes.

## Spec delta

All edits in `docs/specs/multi_backend.md` (spec-first rule; the implement step makes them).
Timeless-oracle rule applies — no phase vocabulary in the spec body; outcome links belong
only in §Known Divergences and §References.

1. **§"Parity contract"** — extend the supported-surface statement with a sentence for a
   fourth dialect: what Trino-targeted parity covers today (full-refresh table/view and
   ephemeral materializations; expression- and clause-level emission), and what it does not
   (the maintenance legs — `maintenance_dialect` returns `Err` for `SqlDialect::Trino`), with
   the reason (the Iceberg connector's per-table-commit shape, per the T1 ruling).
2. **§"CI tiering"** — state Trino's tier as it actually exists in
   `.github/workflows/compat.yml`'s `trino-integration` job: per-PR when the PR touches
   Trino-relevant paths (backend crate, Trino parity tests, the signature registry, the
   dialect printer), plus nightly and the `run-docker-tests` label. State the discipline that
   distinguishes Trino from BigQuery: a Trino leg that cannot reach the coordinator **fails**,
   it does not skip green, because a skipped audit leg is indistinguishable from a passing one.
   Reconcile this with the existing §Surface `SMELT_TRINO_URL` sentence (line ~130), which
   today says Trino-targeted tests *skip* when the variable is unset — narrow that sentence to
   the backend's own integration tests and exclude the audit legs, or state the exception
   explicitly. Do not leave the two statements contradicting each other.
3. **§"Operator lowering"** — add Trino's column to the operator narrative: `^` (Trino has no
   infix power operator at all — the third dialect needing `POWER`), `//` per operand class,
   `%` (Trino *does* have infix `%`), and `::` (Trino has no `::` cast — `CAST(x AS t)` is the
   only spelling). Each stated as a claim the audit's value leg must verify, not as folklore.
4. **§"Clause-level dialect refusals"** — add Trino's absences measured in T1: no `QUALIFY`,
   no trailing commas in a select list, no `PIVOT` clause. State that `PIVOT` makes Trino the
   first backend with `supports_pivot: false`, and that the outcome of that decision (lowering
   vs. compile-time `UnsupportedOnBackend` refusal) is stated in this section once settled —
   the section is where a reader looks for it either way. State positively that `[a, b]` array
   literal syntax **does** work on Trino, so it takes an explicit `Native` verdict rather than
   inheriting one.
5. **§"Cross-engine emission audit"** — add Trino to the two-leg description (the schema leg's
   oracle for Trino is a `/v1/statement` execution against the live coordinator, since Trino
   has no dry-run mode; the value leg compares against DuckDB as reference like every other
   dialect), add a Trino row to the "Gates, by tier" table, and add the rule that closes the
   hole: **an entry with no explicit verdict and no audit-verified probe is `unverified`,
   which is a failure — distinct from `passing` and from `Gap`.** State this as a general rule
   over all dialects, with Trino as the dialect that forced it, not as a Trino special case.
6. **§Known Divergences** — rewrite the "No `dialect_audit` Trino leg" entry (line ~1217) and
   the "Every built-in is implicitly `Native` on Trino" entry (line ~1424) to point at the
   phases that close them, and keep them until phase 10 deletes them. Do not delete either now.

## Tests

Red-green on one new standing gate file, `crates/smelt-cli/tests/trino_emission_spec_freshness.rs`,
modelled on the existing `crates/smelt-cli/tests/trino_spec_freshness.rs`:

1. `parity_contract_states_trino_scope` — §"Parity contract" names Trino and says
   maintenance is not covered on it; fails on today's spec.
2. `ci_tiering_states_trino_tier_and_no_skip_green` — §"CI tiering" names the
   `trino-integration` job's trigger set and contains the never-skip-green statement.
3. `operator_lowering_covers_trino` — §"Operator lowering" mentions Trino alongside `^`,
   `//` and `::`.
4. `clause_refusals_cover_trino` — §"Clause-level dialect refusals" names `QUALIFY`,
   trailing commas and `PIVOT` for Trino, and records the array-literal positive.
5. `audit_states_unverified_is_a_failure` — §"Cross-engine emission audit" contains the
   unverified≠passing rule and a Trino row in the tier table.
6. `skip_semantics_are_not_contradicted` — the §Surface `SMELT_TRINO_URL` sentence and the
   §"CI tiering" statement do not both claim the audit legs skip; asserted by requiring the
   `SMELT_TRINO_URL` paragraph to carry the audit-leg exclusion.

## Tasks

1. Add `crates/smelt-cli/tests/trino_emission_spec_freshness.rs` with the six tests; run it, confirm red.
2. Make spec edits 1–6 above, smallest edit that satisfies each test's intent (prose first, test second in judgement — the test guards the claim, it does not define it).
3. Re-run the gate; confirm green.
4. Sanity-check the timeless-oracle rule: `rg -n 'Phase [A-Z0-9]' docs/specs/multi_backend.md` returns nothing new.
5. Write `phases/01-summary.md`: what the spec now commits to, and anything phase 2's gate design must honour (in particular the exact `unverified`/`passing`/`gap` vocabulary the spec fixed).

## Verification

- `cargo test -p smelt-cli --test trino_emission_spec_freshness`
- `cargo test -p smelt-cli --test trino_spec_freshness` (T1's gate must stay green through the edits)
- `bash .claude/scripts/verify-phase.sh`
- No baseline files touched: `git status --porcelain .claude/` empty.

## Commit message

`spec(trino): state Trino's emission surface, audit legs and CI tier in multi_backend.md`
