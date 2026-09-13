# Phase 3 summary — `BackendType::Trino` and the `trino` target shape

**Shipped:**
- `Target` gains `port`/`user`/`tls`/`password` (`crates/smelt-core/src/config.rs`); `password`
  is redacted via `serialize_with = "redact_token"` and the hand-written `Debug` impl, mirroring
  `token`.
- `BackendType::Trino`; `backend_type()` resolves `"trino"`; `table_format()` returns `None` for
  it (Iceberg decides); `Target::effective_trino_port()` is the single owner of the 8080/443
  default.
- `TRINO_FOREIGN_KEYS` (nine keys, `token` included) and a `trino` leg in
  `Config::validate_targets` requiring `catalog`/`host`/`user` and a bare `host`, accumulating
  every violation.
- `check_literal_secrets` generalised from a hardcoded `databricks`/`token` check to a
  `LITERAL_SECRET_KEYS: &[(backend, key)]` table covering `("databricks","token")` and
  `("trino","password")`.
- `row_set_body`/`build_row_set_table` (`smelt-core/src/sql/row_set.rs`) grow a `Trino` arm on
  the `VALUES` side, flagged for live confirmation in phase 6.
- Every `BackendType`/`SqlDialect` match across the workspace resolved deliberately for the new
  variant (`compile.rs::dialect_and_capabilities` now fallible and refuses Trino;
  `execute/targets.rs`, `smelt-ui/build.rs`, `smelt-cli/explain.rs` map `Trino` →
  `SqlDialect::Trino`; `smelt-backends::create_backend` refuses Trino by name;
  `smelt-maintenance-testkit::print_body_for_dialect` panics loudly — no fixture exercises it).
- `SqlCompiler::new`/`CompilerRegistry::new` are now fallible; every production caller threads
  `?` (new `ProfileWorkspaceError::CompilerRegistryFailed` arm in `smelt-runtime::profile`).
- `examples/trino_broken_foreign_keys/` (committed fixture) + `crates/smelt-cli/tests/
  trino_broken_foreign_keys.rs`; the existing `example_builds.rs` sweep already routes it through
  the `broken`-category build-failure path with no changes needed there.
- 9 new `smelt-core::config` unit tests + 1 `dialect_and_capabilities_refuses_trino_until_measured`
  in `smelt-runtime::compile` + 1 `row_set_body_trino_uses_values`.

**Decisions:** logged in `outcome.md` (fallible constructors, testkit `unimplemented!`, the
literal-secret table generalisation, large-file baseline update).

**For the next planner:** phase 4 (Docker tier) and phase 5 (HTTP client) are unblocked. Nothing
found out of scope for this phase; `smelt-backends::create_backend`'s Trino arm is a stub error
until phase 5 lands `smelt-backend-trino`.

**Gates:** `bash .claude/scripts/verify-phase.sh` — ALL GREEN. `cargo test -p smelt-core --lib
config`, `--test hardening_budget` — pass, no baseline drift. `cargo test -p smelt-cli --test
trino_spec_freshness`, `--test trino_broken_foreign_keys` — pass. `cargo test -p smelt-runtime
--test dialect_seam --test projection_dialect_invariance` — pass. `cargo check --workspace
--all-targets` — clean. `cargo test -p smelt-lsp --test example_workspaces` — pass (37/37).
`.claude/large-file-baseline.txt` updated (reviewer sign-off: growth is real Trino-shaped
content across `config.rs`, `compile.rs`, `graph.rs`, `s_tracker.rs`,
`execute/project/mod.rs`, `smelt-cli/tests/resume.rs`; no cohesion-boundary split warranted).
