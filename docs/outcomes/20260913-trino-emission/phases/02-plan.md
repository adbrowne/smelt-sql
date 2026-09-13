# Phase 2 plan — the Trino coverage gate, landed before any verdict

## Objective

Close the implicit-`Native` hole structurally (criterion 1) by landing a standing, offline gate
that classifies every `(registry entry, position)` pair for `DialectId::Trino` as `stated`,
`verified`, `gap` or `unverified` — the exact three-way vocabulary phase 1 fixed in
`multi_backend.md` §"Cross-engine emission audit" — and *names* every `unverified` pair. Since no
Trino verdict and no Trino audit leg exist yet, essentially every pair is `unverified` today; the
gate lands with that hole enumerated in a shrink-only census file so it is visible, bounded and
cannot grow, and phases 3–6 drive the count to zero.

## Spec delta

`docs/specs/multi_backend.md` §"Cross-engine emission audit", immediately after the
`unverified`/`passing`/`gap` paragraph: add one short paragraph stating that a dialect being
introduced *after* the registry default may record its outstanding `unverified` pairs in a
**shrink-only census file** (`.claude/trino-emission-census.txt` for Trino), which is two-sided
exactly like the gap ratchet — a pair not in the census fails immediately (so a newly-added
built-in can never silently acquire a Trino claim), and a census row that has since gained a
verdict or a probe is a stale-census failure telling you to tighten. The census is deleted, not
grandfathered, when its count reaches zero; `unverified` then reverts to a plain failure. No
user-visible behaviour changes.

## Tests

All new tests live in a new module `crates/smelt-db/tests/dialect_audit/census.rs` unless noted.

1. `stated_emission_at_distinguishes_stated_from_defaulted` (in
   `crates/smelt-types/tests/registry_coverage/emission.rs`) — `Signature::stated_emission_at`
   returns `Some` for an explicit `(dialect, position)` row, `Some` via the `Position::Any`
   fallback, and `None` where `emission_at` would return the default `Native`.
2. `census_classifies_a_stated_verdict_as_stated` — a hand-built `Signature` carrying an explicit
   Trino verdict classifies `Coverage::Stated`, not `Unverified`.
3. `census_classifies_an_audited_dialect_as_verified` — the same entry with no verdict classifies
   `Verified` for `DialectId::DuckDb` (in `AUDITED_DIALECTS`, whose live legs are total over the
   derived probes) and `Unverified` for Trino.
4. `census_classifies_a_ledger_row_as_gap` — a pair with a `Verdict::Gap`/`Divergent` ledger row
   classifies `Gap`, distinctly from both `Unverified` and `Stated`.
5. `the_trino_census_matches_the_registry_exactly` — the gate: the classifier's `Unverified` set
   for Trino equals the census file line-for-line; a missing row fails naming the entry and
   position, a stale row fails telling you to tighten.
6. `census_gate_detects_a_new_unstated_entry` — the red-proof self-test: feed the classifier a
   synthetic entry absent from the census and assert the comparison reports it by name (mirrors
   `hardening_budget::gate_detects_regression`).
7. `unverified_pairs_exist_only_for_unaudited_dialects` — for every `AUDITED_DIALECTS` member the
   `Unverified` set is empty, so the census mechanism cannot mask an audited dialect's hole.
8. `every_census_row_names_a_real_registry_entry_and_position` — two-sided: a row naming an entry
   the registry no longer has, or a position its `ExprKind` cannot occupy, is an error.
9. `the_census_header_states_the_shrink_only_rule` — the file carries the ratchet contract and a
   pointer to this outcome, so a future reader cannot mistake it for an accepted state.
10. `trino_emission_spec_freshness::census_rule_is_stated` (append to
    `crates/smelt-cli/tests/trino_emission_spec_freshness.rs`) — the spec delta above is present.

## Tasks

1. Add `Signature::stated_emission_at(dialect, position) -> Option<Emission>` in
   `crates/smelt-types/src/signatures/signature.rs`; make `emission_at` delegate to it with the
   `Native` default, so "stated versus defaulted" is a registry-owned distinction, not a
   test-local scan of the `pub emission` slice.
2. Write the spec delta (spec-first) and the freshness assertion (test 10).
3. Create `crates/smelt-db/tests/dialect_audit/census.rs`: `enum Coverage { Stated, Verified, Gap,
   Unverified }`, `fn classify(dialect, sig, position, ledger) -> Coverage`, and
   `fn census_for<'a>(dialect, entries: impl Iterator<Item = &'a Signature>) -> Vec<CensusRow>`
   taking an injectable entry iterator so tests 2–4 and 6 can feed synthetic signatures. Reuse
   `report::applicable_positions` rather than re-deriving the position axis.
4. Wire `mod census;` into `dialect_audit/main.rs`; reuse `AUDITED_DIALECTS` and `ledger` from
   there (no second source of truth for either).
5. Generate `.claude/trino-emission-census.txt` via `SMELT_REGEN_TRINO_CENSUS=1 cargo test -p
   smelt-db --test dialect_audit the_trino_census_matches_the_registry_exactly`; write the header
   (format `<NAME> <position>`, shrink-only contract, owning phases 3–6, delete-at-zero rule).
6. Whitelist the census in `.gitignore` (`!.claude/trino-emission-census.txt`) beside the other
   baselines, then verify with `git ls-files .claude/trino-emission-census.txt` — the `.claude/*`
   ignore rule silently untracks new baseline files.
7. Reshape check only: no phase-table edits beyond the ones this plan already commits.

## Verification

- `cargo test -p smelt-types --test registry_coverage` (test 1)
- `cargo test -p smelt-db --test dialect_audit` (tests 2–9; offline, no coordinator needed —
  this phase adds no live leg, so nothing here may skip)
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` (test 10)
- `git ls-files .claude/trino-emission-census.txt` — non-empty output
- `git diff --stat .claude/` — only the new census file; no existing baseline lowered
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(dialect): name every unverified Trino emission pair behind a shrink-only census gate`
