# Phase 11 plan — Close: docs-site page, ratchets, divergences, hand-forward

## Objective

Close T1 by making the `trino` target reachable from the user docs, proving the
new crate's hardening entry and the workspace gates are green two-sided, recording
in §Known Divergences the gaps this outcome deliberately leaves open, and handing
the measured `✗` set forward to the four sibling outcomes that own its consequences.
Advances criteria 1 (the connection-security rule stated for users), 5 (the measured
profile surfaced where a user reads it) and 9 (gates green, no ratchet lowered).

## Spec delta

`docs/specs/multi_backend.md` §Known Divergences — three additions, each naming the
sibling outcome that owns it, alongside the two Trino entries already there:

1. **No maintenance dialect on Trino.** `maintenance_dialect` returns `Err` for
   `SqlDialect::Trino`, so no incremental/maintenance family runs on a `trino`
   target — a full refresh is the only route today. Owner: `20260913-trino-incremental`.
2. **No `dialect_audit` Trino leg.** The audit's `AUDITED_DIALECTS` is a three-member
   test-local const; Trino has no fixture, probe, ledger row or baseline metric, so
   `dialect-coverage.md` has no Trino column. Owner: `20260913-trino-emission`.
3. **`supports_transactional_ddl = false` is smelt's client, not Trino.** The measured
   `Client does not support transactions` comes from smelt's stateless `/v1/statement`
   client having no session continuity across `START TRANSACTION`/DDL/`ROLLBACK`, not
   from a grammar rejection. Stated so a later outcome does not "fix" Trino for it.
   Owner: `20260913-trino-ledger`.

No §Surface change: the capability column and the target shape landed in phases 1 and 8.

## Tests

Red-green, all in one new file `crates/smelt-core/tests/trino_docs_freshness.rs`,
modelled on `crates/smelt-core/tests/databricks_docs_freshness.rs` (parse the source /
spec by text, never restate the list):

1. `docs_site_names_every_refused_trino_key` — every key parsed out of
   `TRINO_FOREIGN_KEYS` in `crates/smelt-core/src/config.rs` appears in the `### Trino`
   section of `docs-site/docs/guide/targets.md`.
2. `docs_site_states_the_trino_password_env_only_rule` — the `("trino", "password")`
   pair parsed out of `LITERAL_SECRET_KEYS` has its `${ENV}`-only, literal-is-a-hard-error
   rule stated in the section, and no example in the section shows a literal password.
3. `docs_site_names_every_measured_false_capability` — every capability whose cell is `✗`
   in `multi_backend.md`'s Trino column is named in the section's limitations list.
   (Spec column ↔ constructor is already gated by `smelt-dialect`'s
   `capability_conformance`, so this closes the chain to the user docs without a
   second restatement.)
4. `docs_site_trino_scripts_exist` — every `scripts/trino-*` path and the default port
   the section tells a user to run resolve against the filesystem / `scripts/trino-env.sh`.

## Tasks

1. Write the three §Known Divergences entries in `docs/specs/multi_backend.md` (spec first).
2. Add `### Trino` to `docs-site/docs/guide/targets.md`, after `### Databricks`, in the
   Databricks section's shape: prose intro (HTTP protocol, pure Rust, Iceberg connector and
   why the connector decides the write surface), a `smelt.yml` example, the field table
   (`host`, `port`, `user`, `catalog`, `schema`, TLS, `password` as `${ENV}` only), the
   refused-key paragraph, a `#### Credentials` subsection (credential never in a log line,
   an error message or a run report), a `#### Running the local tier` subsection
   (`scripts/trino-up.sh` / `-down.sh` / `-env.sh`, the pinned images, `SMELT_TRINO_PORT`,
   pointer to `scripts/README-trino.md` and the gated `trino-integration` CI job), and a
   `#### Limitations` list covering every `✗` cell plus "no incremental maintenance yet".
3. Add the new test file and make tests 1–4 pass against the written section.
4. Run the hardening gate and confirm `smelt-backend-trino`'s three entries in
   `.claude/hardening-baseline.txt` still match two-sided; update via
   `.claude/scripts/hardening-budget.sh --update` only if the count genuinely moved, with
   a sign-off note in the file.
5. Append a dated hand-forward entry to the Decision log of each sibling outcome
   (`20260913-trino-emission`, `-trino-ledger`, `-trino-incremental`, `-trino-dogfood`),
   each stating only what that outcome must act on: emission ← the implicit-`Native` hole,
   the six grammar `✗`s (`QUALIFY`, `::`, trailing commas, `[a,b]` bracket-literal *works*,
   pipe syntax, `PIVOT` absent), no `AUDITED_DIALECTS` entry; ledger ← no transactional DDL
   (client-side), no temp tables, `IS NOT DISTINCT FROM` not `<=>`, Delta-shaped residency
   prior confirmed; incremental ← `maintenance_dialect` refusing Trino,
   `supports_native_ivm`/`merge_not_matched_by_source`/`merge_schema_write`/`insert_overwrite`
   false, `CREATE OR REPLACE TABLE` available; dogfood ← the array-to-Arrow decode gap and
   the `print_body_for_dialect` `unimplemented!`.
6. Flip the T1 phase-11 row to `done` is the *implement* step's job; also append a dated
   phase-11 Decision-log entry recording the close.

## Verification

- `cargo test -p smelt-core --test trino_docs_freshness` — the four new gates.
- `cargo test -p smelt-core --test hardening_budget` — ratchet two-sided green.
- `cargo test -p smelt-dialect --test capability_conformance` — spec column still matches
  the constructor after the spec edit.
- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_tier_pins` — unchanged.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- No live tier needed: every gate here is a file/spec assertion. If any task tempts a live
  leg, phase 10's summary already records the live confirmation; cite it rather than re-run.

## Commit message

`docs(trino): docs-site target page, divergences, and the measured-gap hand-forward`
