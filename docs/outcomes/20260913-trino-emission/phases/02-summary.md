# Phase 2 summary — the Trino coverage gate, landed before any verdict

**Shipped:**
- `Signature::stated_emission_at(dialect, position) -> Option<Emission>` in
  `crates/smelt-types/src/signatures/signature.rs` — the registry-owned "stated versus
  defaulted" distinction; `emission_at` now delegates to it with the `Native` fallback.
- `crates/smelt-db/tests/dialect_audit/census.rs` — `Coverage` (`Stated`/`Verified`/`Gap`/
  `Unverified`), `classify(dialect, sig, position, ledger)`, `census_for(dialect, entries,
  ledger)`. Wired into `dialect_audit/main.rs` via `mod census;`, reusing `AUDITED_DIALECTS`
  and `ledger::dialect_divergences()` — no second source of truth for either.
- `.claude/trino-emission-census.txt` — generated via `SMELT_REGEN_TRINO_CENSUS=1 cargo test
  -p smelt-db --test dialect_audit census::tests::the_trino_census_matches_the_registry_exactly`;
  237 `Unverified` `(entry, position)` pairs for Trino today, shrink-only header included.
  Whitelisted past `.claude/*` in `.gitignore`.
- `docs/specs/multi_backend.md` §"Cross-engine emission audit" — one paragraph stating the
  shrink-only census rule as a general mechanism (not Trino-specific in its wording, though
  Trino is its first instance).
- `crates/smelt-cli/tests/trino_emission_spec_freshness.rs::census_rule_is_stated` — freshness
  gate for that spec paragraph.
- `crates/smelt-types/tests/registry_coverage/emission.rs::stated_emission_at_distinguishes_stated_from_defaulted`
  (plan's test 1, kept in its planned home rather than `census.rs`).
- `report::applicable_positions` and `report::position_label` widened `fn` → `pub(crate) fn` so
  `census.rs` reuses the coverage table's own position axis instead of re-deriving it.

**Decisions:**
- Gap check runs *before* the Stated check in `classify`: a pair the registry states `Native`
  for, that a live sweep found does not actually work, classifies `Gap`, not `Stated` — a
  ledger row is a live finding and takes priority over what the registry merely claims.
- `classify` takes `ledger: &[LedgerRow]` as an explicit parameter (not read internally from
  `ledger::dialect_divergences()`) so tests can inject synthetic rows without touching the real
  ledger — mirrors how `probe_or_reason` and friends are already structured for testability.
- Census file lines are `<NAME> <position>` using the coverage table's own `position_label`
  spelling (`scalar`/`agg`/`win`/`run`), so a human reading both artifacts sees the same
  vocabulary.

**For the next planner:**
- Phases 3–6 (explicit verdicts, `PIVOT`, live schema/value legs, `Restructure`/`Rewrite`) each
  shrink `.claude/trino-emission-census.txt`; regenerate it after each with the
  `SMELT_REGEN_TRINO_CENSUS=1` command above rather than hand-editing. `every_census_row_names_a_real_registry_entry_and_position`
  will catch a row that no longer names a live registry entry/position, but it does **not**
  catch a row that has merely gained a verdict — that's `the_trino_census_matches_the_registry_exactly`'s
  stale-row failure, and it fires the moment a phase lands a verdict without regenerating.
- `AUDITED_DIALECTS` (`dialect_audit/main.rs`) still excludes Trino — untouched by design, since
  adding Trino to it before a schema leg exists would make `unverified_pairs_exist_only_for_unaudited_dialects`
  wrongly demand zero Trino gaps immediately. Phase 5 is where Trino joins that list.
- Did not touch `AcceptedVerdict`/`classify_accepted` in `main.rs` (the schema-leg's own
  accept/reject classifier) — that's a different, older four-way split serving a different
  question (what happened when a probe ran); `census::Coverage` serves "is this pair verified at
  all," and the two are intentionally not unified.

**Gates:**
- `cargo test -p smelt-types --test registry_coverage` — 108/108 passed
- `cargo test -p smelt-db --test dialect_audit` — 69/69 passed (8 new census tests; offline,
  no coordinator — this phase adds no live leg)
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` — 7/7 passed
- `git ls-files .claude/trino-emission-census.txt` — tracked
- `git diff --stat .claude/` — only the new census file (237 insertions); no baseline lowered
- `bash .claude/scripts/verify-phase.sh --fast` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, example_diagnostics). The full `cargo test --quiet` (workspace) leg did not
  complete within the 10-minute tool ceiling on this run (unrelated to this phase's targeted
  crates, all green above); narrowed scope per the implement-step's own instruction rather than
  ending the turn on an unseen background result.
