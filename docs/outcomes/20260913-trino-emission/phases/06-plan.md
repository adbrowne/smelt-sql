# Phase 6 plan — the live Trino **value** leg, the two-sided ledger rows, and the census deleted at zero

## Objective

Give Trino the second audit direction: execute every derived probe on the live coordinator and on
DuckDB and compare the *answers*, catching the class of divergence a schema leg cannot see (a
spelling the engine accepts but means something else by). Register each finding two-sidedly in
`ledger.rs`, then — with both legs live — flip Trino into `census::BOTH_LEGS_LIVE`, which drives
the census to zero, and delete `.claude/trino-emission-census.txt` rather than grandfathering it.
Advances criteria 4 (both directions), 5 (two-sided ledger + ratchet) and closes criterion 1's
interim mechanism into its standing form.

## Live tier first, never skip green

`bash scripts/trino-up.sh && source scripts/trino-env.sh`, and confirm with a trivial `SELECT`
before writing anything. If the coordinator cannot be reached, this phase emits
`<<PHASE_BLOCKED>>` — it has no offline fallback (unlike phase 3's registry work). Per phase 5's
finding, run the live legs as **targeted** commands and run `verify-phase.sh` with
`SMELT_TRINO_URL` unset; the shared Iceberg REST catalog collides under full-suite concurrency.

## Spec delta

`docs/specs/multi_backend.md`, two edits, made first:

1. §"Cross-engine emission audit" — the census paragraph is rewritten to state the mechanism
   **generically** ("a dialect introduced after the registry default *may* record its outstanding
   `unverified` pairs in a shrink-only census file while its verdicts and audit legs are built;
   the file is deleted, not grandfathered, once the count reaches zero"), dropping the naming of
   `.claude/trino-emission-census.txt` as a live artifact, because it no longer exists. The
   two-sided rule and the "deleted at zero" sentence stay verbatim.
2. §Known Divergences — delete the "**No `dialect_audit` Trino value leg**" entry (the leg now
   exists). The implicit-`Native` entry (§Known Divergences' last item) stays for phase 10.

## Tests (red first)

- `smelt-backend-trino::client::execute_json_returns_columns_and_raw_rows` (live) — a new
  `TrinoClient::execute_json` returns the reported `(name, raw_type)` columns **and** the
  undecoded `serde_json::Value` rows, paging `nextUri` to completion like `execute_schema`.
- `smelt-oracle-testkit::trino_oracle::trino_json_cells_decode_by_declared_type` (offline) — the
  testkit's own `cell_from_trino_json(raw_type, value)` maps bigint/double/real/decimal/varchar/
  boolean/date/timestamp/array/NULL JSON cells to `Cell`, timestamps rendered in the ISO spelling
  `compare_cells`'s `temporal()` normalises. Deliberately **not** routed through
  `arrow_convert::trino_type_to_arrow` (it has no array/varbinary/interval arm — a decode error
  there would masquerade as a rejected probe, the exact confusion phase 5 designed out).
- `smelt-oracle-testkit::trino_oracle::trino_oracle_executes_rows` (live) — `impl ValueOracle for
  TrinoOracle` returns typed rows for a mixed-type SELECT, including a NULL.
- `dialect_audit::trino::value_leg_trino` (live) — `run_value_leg(DialectId::Trino, &TrinoOracle,
  &DuckDbOracle)`; failures empty, `probes_compared >= PROBE_COVERAGE_FLOOR`, coverage line
  printed.
- `dialect_audit::trino::trino_caret_agrees_with_duckdb_power` (live) — the regression analogue of
  `spark_caret_agrees_with_duckdb_power`: `n_bigint ^ 2` computes the same number on both engines
  through phase 3's `POWER({0}, {1})` template, proving this leg catches a meaning change a schema
  comparison cannot.
- `dialect_audit::census::no_dialect_has_unverified_pairs` — over `DialectId::ALL`: zero
  `Coverage::Unverified` pairs anywhere. This replaces `the_trino_census_matches_the_registry_
  exactly` as criterion 1's standing gate.
- `dialect_audit::census::classify_reports_unverified_without_both_legs` — the red-proof kept
  alive after every real dialect is verified: `classify` takes the both-legs set as an explicit
  parameter, so a synthetic set can still prove an unstated pair classifies `Unverified`.
- `smelt-cli::trino_emission_spec_freshness::census_rule_is_stated` — rewritten to assert the
  *generic* census rule (shrink-only, two-sided, deleted at zero) without naming a Trino census
  file that no longer exists.

## Tasks

1. Bring the tier up, `source scripts/trino-env.sh`, confirm a live `SELECT 1`; block if not.
2. Spec delta above; update `census_rule_is_stated` to match (red → green).
3. `TrinoClient::execute_json` in `crates/smelt-backend-trino/src/client.rs`, factored over the
   same page-following loop `execute`/`execute_schema` use — no third copy of the paging logic.
4. `cell_from_trino_json` + `impl ValueOracle for TrinoOracle` in the testkit; keep `row_count`
   or fold it into the new path if it becomes redundant.
5. Add `value_leg_trino` and `trino_caret_agrees_with_duckdb_power` to `dialect_audit/trino.rs`;
   run them live and read the failure list.
6. Triage every value failure in this order, recording the reason: (a) a registry verdict that
   closes it (a `Rename`/`Template`/`Conditional`/`Unsupported` — preferred, it generalises past
   the probe, as phase 5's `LOG` fix did); (b) an irreducible semantic difference → a
   `Verdict::Divergent` row with `Leg::Value` (does not ratchet); (c) a genuine missing lowering →
   `Verdict::Gap` with `Leg::Value` under #209, which **raises** `dialect_gaps_trino` and
   therefore needs a dated sign-off note appended to `.claude/dialect-gaps-baseline.txt` in the
   same style as phase 5's. Any registry verdict added here is offline-validated by
   `registry_coverage` and regenerates the census in step 8.
7. With both legs green: add `DialectId::Trino` to `census::BOTH_LEGS_LIVE`; replace
   `a_schema_only_dialect_is_not_yet_verified` and `unverified_pairs_exist_only_for_dialects_
   without_both_legs` with `no_dialect_has_unverified_pairs`; parameterise `classify` on the
   both-legs set and keep the red-proof; delete `read_census`/`write_census`/`diff`/
   `CENSUS_HEADER`, the `SMELT_REGEN_TRINO_CENSUS` path and the four file-bound tests.
8. `git rm .claude/trino-emission-census.txt` **and** remove its `!` whitelist line from
   `.gitignore` (line 22) — a stale whitelist for a deleted baseline is the trap recorded in
   `project_claude_gitignore_baseline_trap`.
9. Update `report.rs`'s Trino verification-tier row from schema-only to both legs, regenerate
   `docs/reference/dialect-coverage.md` with `SMELT_REGEN_DOCS=1`, and update
   `dialect_audit/trino.rs`'s module doc (it currently says "Schema leg only — value-leg
   comparison is phase 6's subject").
10. Write `phases/06-summary.md`.

## Verification

- Live, with the tier exported: `cargo test -p smelt-backend-trino --quiet`;
  `cargo test -p smelt-oracle-testkit --quiet`;
  `cargo test -p smelt-db --test dialect_audit --quiet` (all legs, census gate, ledger gates).
- Offline, `SMELT_TRINO_URL` **unset**: `cargo test -p smelt-types --test registry_coverage`;
  `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test
  operand_conditional`; `cargo test -p smelt-cli --test trino_emission_spec_freshness`;
  `bash .claude/scripts/verify-phase.sh`.
- `git ls-files .claude/` shows no `trino-emission-census.txt`, and
  `git status --short` is clean after the commit (the whitelist removal took effect).

## Commit message

`feat(dialect-audit): add the live Trino value leg and retire the emission census at zero`
