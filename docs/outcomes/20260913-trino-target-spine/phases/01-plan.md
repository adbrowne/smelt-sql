# Phase 1 — Spec delta: the `trino` target

**Outcome:** `docs/outcomes/20260913-trino-target-spine/outcome.md`
**Advances:** criterion 1 (the target is specified before it is built), criterion 2's
recording obligation (the implicit-`Native` emission hole is written down, not left unremarked).

## Objective

Write the `trino` target into the two normative specs *before* any code exists, so phases 2–10
implement a described target rather than describing an implemented one. This is a
documentation-only phase: two spec files, one diagnostics-catalogue check, no crate touched.
`docs-site/` is phase 10's; this phase changes normative specs only.

## Spec delta (this phase's entire deliverable)

**`docs/specs/smelt_yml.md` §"Target shape"** — add Trino rows to the field table and a
refusal paragraph beneath it, mirroring the `databricks` paragraph already there:

- `type`: extend the enumeration to `duckdb`, `spark`, `bigquery`, `databricks`, or `trino`.
- `host` — Trino **and** Databricks; on a `trino` target the coordinator hostname, bare (no
  scheme, no trailing slash), required.
- `port` — Trino only, optional, default `8080` when `tls: false` and `443` when `tls: true`.
- `user` — Trino only, required; Trino's session user, sent as `X-Trino-User`.
- `catalog` — extend the existing row to Spark, Databricks **and** Trino; on a `trino` target
  it names the Iceberg catalog and is **required** (no default: the connector, not smelt,
  decides the write surface, so guessing a catalog would guess the write surface).
- `schema` — extend the existing row: on a `trino` target it is the schema inside `catalog`.
- `tls` — Trino only, boolean, default `false`; selects `https` and the default port.
- `password` — Trino only, optional, `${ENV}` reference **only**. A literal value is a hard
  configuration error, not a warning — same reasoning as `databricks`' `token`: the key *is*
  the secret, so no literal could be an intentional non-secret. Absent means no
  `Authorization` header at all (the unauthenticated local tier).
- Refusal paragraph: a `trino` target hard-errors, naming both the offending key and the
  backend, on `connect_url`, `warehouse`, `format`, `database`, `settings`, `project`,
  `dataset`, `location`, and `token` — Trino has no Spark Connect URL, no host-visible
  warehouse, no table-format choice (the Iceberg connector decides it), no DuckDB file and no
  BigQuery addressing; `token` is refused specifically so a Databricks-shaped credential is
  not silently ignored on a target that reads `password`. State the channel explicitly: this is
  a **hard configuration error at config load** naming every offending key (not the first),
  the same channel and shape as `databricks`', because `smelt.yml` target-shape violations do
  not flow through `DiagnosticCode` (see the decision log entry of 2026-09-13). Extend the
  "specified for `databricks` only" sentence to name both target types.

**`docs/specs/multi_backend.md`** —

- §Surface "Backends" bullet: add `trino` to the `type:` enumeration, `SqlDialect::Trino` to
  the dialect list, and a sentence: a `trino` target declares `SqlDialect::Trino` and
  `BackendCapabilities::trino_iceberg()`, naming `host`/`port`/`user`/`catalog`/`schema` in
  place of Spark's `connect_url` — and is the first backend whose dialect is *not* shared with
  another target type.
- §Surface capability matrix: add a **Trino (Iceberg)** column with every cell written `?`
  and a note directly beneath: each cell is `?` until executed against the live coordinator,
  at which point the cell and `BackendCapabilities::trino_iceberg()` are written in the same
  commit. `?` is the honest spelling of *unmeasured* — the alternative, seeding the column from
  Trino's documentation, is exactly what the rule under the table forbids. Record the prior
  (Trino sits near Spark (Delta), Iceberg shares Delta's atomicity shape) as a prior, not a
  value.
- §Surface: a `SMELT_TRINO_URL` bullet in the shape of the `SPARK_CONNECT_URL` /
  `SMELT_BQ_PROJECT` / `SMELT_DATABRICKS_HOST` ones — Trino integration tests connect to the
  coordinator it names; when unset they **skip** (not fail).
- §"Session initialization": a `trino` target issues `CREATE SCHEMA IF NOT EXISTS
  <catalog>.<schema>` against the Iceberg catalog — catalog-qualified, like Databricks'.
- §"Connection security": a `trino` target's credential is its own `password` key, not a
  substring of a URL, so — as with `databricks`' `token` — a literal is a hard configuration
  error rather than a smell; the resolved password never reaches a log line, a run report, a
  diagnostic or an error message, and the unauthenticated form (no `password`) carries no
  secret for redaction to protect.
- §"Loading data into a backend": Trino's client protocol is HTTP with no host-filesystem
  assumption; which bulk path smelt takes is measured in phase 7 and named here then.
- §"Cross-engine data exchange": a cross-backend edge into or out of a `trino` target is
  refused with a diagnostic — the `read_parquet()` substitution's precondition is a `warehouse`
  path both processes can read, and a `trino` target has no `warehouse` key at all (it is one
  of the keys §"Target shape" refuses); object storage behind the Iceberg catalog is not a
  host path.
- §Known Divergences, two new entries:
  1. **Every built-in is implicitly `Native` on Trino.** `Signature::emission_at` returns
     `Native` for any `(dialect, position)` with no entry, so `DialectId::Trino` enters the
     registry claiming every built-in is spelled natively on Trino — a claim no probe has
     tested. Names the sibling outcome `docs/outcomes/20260913-trino-emission/` as owner, and
     says plainly that until then a model may compile to SQL Trino rejects.
  2. **The Trino capability column is unmeasured.** Every cell reads `?`; the column is a
     hypothesis until phase 8 of `20260913-trino-target-spine` executes it, and
     `capability_conformance.rs` therefore asserts nothing about Trino yet.
- §References: `crates/smelt-backend-trino/` under Code (forward reference, marked as landing
  in this outcome) and this outcome under Plans.

**`docs/specs/diagnostics.md`** — no new code. Add one sentence where `smelt.yml` handling is
described, stating that target-shape key-placement and literal-secret violations are hard
config-load errors with no `DiagnosticCode`, and that this is deliberate (the config load
precedes the diagnostic pipeline). This closes the gap the decision log records rather than
leaving a criterion reading as unmet.

## Tests

Doc-only phase; the gates are textual, and each must fail before the edit and pass after.

1. `trino_target_shape_is_specified` (new, `crates/smelt-cli/tests/trino_spec_freshness.rs`,
   in the shape of `state_docs_freshness.rs`) — reads `docs/specs/smelt_yml.md` and asserts the
   §"Target shape" table names `port`, `user`, `tls` and `password` as Trino keys and the
   refusal paragraph names all nine refused keys.
2. `trino_capability_column_exists_and_is_unmeasured` (same file) — `multi_backend.md`'s
   §Surface matrix has a Trino column and every Trino cell in it is `?`; fails the moment
   someone writes a value without the measurement phase.
3. `trino_native_emission_hole_is_recorded` (same file) — §Known Divergences contains an entry
   naming `emission_at`/`Native` and the `20260913-trino-emission` outcome.
4. `trino_connection_security_rule_is_stated` (same file) — §"Connection security" names
   `password` and the literal-is-a-hard-error rule.

## Tasks

1. Write the four assertions as a failing test file first (red).
2. Edit `docs/specs/smelt_yml.md` §"Target shape": field-table rows + refusal paragraph.
3. Edit `docs/specs/multi_backend.md`: Backends bullet, matrix column + `?` note,
   `SMELT_TRINO_URL` bullet, Session initialization, Connection security, Loading data,
   Cross-engine data exchange, two Known Divergences, References.
4. Add the `diagnostics.md` sentence on the config-load refusal channel.
5. Re-read both specs for timeless-oracle compliance: no phase numbers, no outcome-phase
   vocabulary in spec *body* text (the two Known Divergences may name the outcome file, paired
   with a link, which the rule permits).
6. Run the gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test trino_spec_freshness`
- `cargo test -p smelt-dialect --test capability_conformance` — must still pass untouched; the
  `?` column is what keeps it honest rather than drifting.
- `rg -n 'Phase [A-Z0-9]' docs/specs/multi_backend.md docs/specs/smelt_yml.md` returns nothing.

## Commit message

`docs(trino): specify the type: trino target shape, capability column and connection security`
