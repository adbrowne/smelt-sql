# Phase 3 plan — `BackendType::Trino` and the `trino` target shape in `smelt-core::config`

## Objective

Land the *implementation* half of criterion 1: a `type: trino` target parses its own keys
(`host`, `port`, `user`, `catalog`, `schema`, `tls`, `password`), refuses every key belonging to
another backend's shape by name, and refuses a literal `password` before interpolation. Adding
`BackendType::Trino` breaks a handful of workspace matches; each is resolved deliberately
(fail-loud where the answer is not yet measured), never defaulted onto Spark's or DuckDB's arm.

## Spec delta

None. Phase 1 already landed `docs/specs/smelt_yml.md` §"Target shape" (the seven Trino keys, the
nine refused keys, the `${ENV}`-only `password` rule) and `docs/specs/multi_backend.md`
§"Connection security" (the Trino paragraph at L921). This phase implements that text; the
standing gate `cargo test -p smelt-cli --test trino_spec_freshness` must stay green.

## Tests

Red-green, all in `crates/smelt-core/src/config.rs` unit tests unless named otherwise.

- `backend_type_resolves_trino` — `type: trino` → `BackendType::Trino`.
- `trino_target_has_no_table_format` — `table_format()` is `None` (Iceberg connector decides).
- `trino_target_parses_its_own_keys` — `host`/`port`/`user`/`catalog`/`schema`/`tls` round-trip.
- `trino_target_requires_catalog_host_and_user` — each absent key is its own named error, and a
  target missing all three reports all three (not the first only).
- `trino_host_must_be_bare_hostname` — a scheme or trailing slash is refused, naming the value.
- `trino_target_refuses_every_foreign_key` — one case per key in
  `connect_url`/`warehouse`/`format`/`database`/`settings`/`project`/`dataset`/`location`/`token`;
  message names both the key and `trino`.
- `trino_target_names_every_foreign_key_at_once` — a target carrying three foreign keys yields
  three errors (the "not the first only" half of the row's intent).
- `trino_literal_password_is_refused_pre_interpolation` — `password: hunter2` is a
  `ConfigError::LoadError`; `password: ${TRINO_PASSWORD}` loads.
- `trino_password_is_redacted` — `password` prints `<redacted>` through both `Debug` and
  `serde` Serialize, mirroring `token`.
- `trino_effective_port_follows_tls` — absent `port` is `8080` when `tls: false`, `443` when
  `tls: true`; an explicit `port` wins.
- `row_set_body_trino_uses_values` (`smelt-core/src/sql/row_set.rs`) — Trino renders
  `VALUES …` like DuckDB/Spark, not BigQuery's `UNION ALL` shape.
- `dialect_and_capabilities_refuses_trino_until_measured` (`smelt-runtime/src/compile.rs`) —
  a Trino target errors by name rather than borrowing Spark's capability profile.
- `examples/trino_broken_foreign_keys` fixture test in `crates/smelt-cli/tests/` — loading the
  committed workspace fails and the message names all the foreign keys it carries.

## Tasks

1. Add `port`, `user`, `tls`, `password` to `Target` with the spec's doc comments; `password`
   gets `serialize_with = "redact_token"` and a `<redacted>` field in the hand-written `Debug`.
2. Add `BackendType::Trino`; wire `backend_type()`'s `"trino"` arm and `table_format()`'s
   `None` arm (join the DuckDB/BigQuery/Databricks group with a reason in the doc comment).
3. Add `Target::effective_trino_port()` — the single owner of the 8080/443 default, so phase 5's
   HTTP client reads it rather than restating it.
4. Add `TRINO_FOREIGN_KEYS: &[ForeignKeyCheck]` (the nine keys) next to `DATABRICKS_FOREIGN_KEYS`.
5. Restructure `Config::validate_targets` to dispatch on `backend_type()` — a `databricks` leg
   (unchanged behaviour) and a `trino` leg (required `catalog`/`host`/`user`, bare-hostname check,
   foreign-key loop) — accumulating into the same `errors` vec so every violation is named.
6. Generalise `check_literal_secrets` from hardcoded `databricks`/`token` to a
   `&[(backend, secret_key)]` table with `("databricks","token")` and `("trino","password")`;
   the message names the target, the key, and the `${ENV_VAR}` remedy.
7. Update `validate_targets`' and `check_literal_secrets`' doc comments — they currently say
   "enforced for `databricks` targets only".
8. Resolve `BackendType::Trino` compile fallout deliberately, auditing for `_ =>` arms that
   would absorb it (the `type_cast_sql` lesson from phase 2):
   - `smelt-runtime/src/compile.rs::dialect_and_capabilities` → returns `Result`, refusing Trino
     with a typed error naming the unmeasured capability profile (see decision log); thread the
     `?` through its callers.
   - `smelt-runtime/src/execute/targets.rs` (2 sites) and `smelt-ui/src/build.rs` →
     `SqlDialect::Trino` (`maintenance_dialect` already refuses it, phase 2).
   - `smelt-core/src/sql/row_set.rs` (2 sites) → Trino joins the `VALUES` arm, doc-commented as
     standard-SQL and flagged for confirmation by execution in phase 6.
   - `smelt-maintenance-testkit`, `smelt-cli/src/test_compiler.rs`, `smelt-ui/tests/api.rs` and
     any other site surfaced by `cargo check --workspace --all-targets` — refuse or name Trino;
     do not add it to a `|`-group without a reason in the diff.
9. Commit `examples/trino_broken_foreign_keys/` (a `smelt.yml` with a `trino` target carrying
   several foreign keys plus one model), confirming no existing example sweep loads it.
10. Re-run `trino_spec_freshness` — the Trino capability cells must still read `?` (phase 8 owns
    replacing them).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --lib config`
- `cargo test -p smelt-cli --test trino_spec_freshness`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`
- `cargo test -p smelt-core --test hardening_budget` — no baseline drift
- `cargo check --workspace --all-targets` clean (the exhaustiveness proof for the new variant)

## Commit message

`feat(config): land BackendType::Trino and the trino target shape with named key refusals`
