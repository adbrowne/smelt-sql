# Changelog

All notable changes to smelt are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps may include
breaking changes).

## [Unreleased]

## [0.5.0]

### Added
- Typed meta-language: `smelt.define`, `smelt.extern`, generics, HOFs, row
  polymorphism, struct/list/lambda types, reflection over the model graph
  (`docs/specs/functions.md`, `docs/specs/gradual_typing.md`,
  `docs/specs/scoping.md`, `docs/specs/expansion.md`).
- Universal `smelt.<path>` addressing, replacing the legacy
  `smelt.ref`/`smelt.source`/`smelt.fn.*` forms.
- Composed-axes conditional maintenance: key + time incremental models with
  bounded batching, backward/forward-fill and connected-components identity
  resolution, event-time monotonicity proofs, partition-grain pruning and
  derived output windows.
- Virtual environments: data model, reuse evaluator, and Stage 0 prototype.
- Data catalog (`smelt docs generate`), schema diff (`smelt diff`), and
  schema evolution (column `default:`/`backfill:`, ALTER-based migration,
  complex-type structural diff) for both DuckDB and Spark backends.
- `smelt test` data-testing framework, declarative column tests
  (`not_null`/`unique`/`accepted_values`/`relationships`) that compile to a
  compile-time verdict when a property is already proven, only lowering to a
  SQL scan otherwise (`docs/specs/data_tests.md`).
- `smelt init`, `smelt list`, `smelt clean`, grouped end-of-run failure
  summaries, per-run JSON report artifacts, and structured `--log-format
  json` output.
- Operability: `${ENV_VAR}` interpolation in `smelt.yml`, state locking and a
  versioned state schema with atomic writes, per-target state partitioning,
  DAG-parallel execution (`--jobs`), bounded retry for transient errors, and
  `--resume`.
- Full type-system axis coverage: nullability (sound-upper-bound contract),
  decimal precision/scale arithmetic, and timezone-awareness, each backed by
  a DuckDB differential property-test oracle.
- YAML/JSON loader parsers with schema validation as first-class Salsa
  inputs; seeds as backend-portable reference data.
- Salsa 0.26 upgrade across `smelt-db`, `smelt-lsp`, `smelt-cli`, `smelt-ui`.
- LSP: Test Explorer/gutter icons for `smelt test`, rename refactoring,
  find-references, code actions/quick fixes, hover and goto-definition
  parity across the meta-language and `smelt.<path>` forms.
- Spark backend hardening: dual-target parity suite, divergence ledger
  re-verified against a live Spark Connect server, per-PR paths-gated CI.
- Fail-loud discipline gates: `unwrap`/`expect` ratchet, `println!` gate,
  `Unknown`-type census, function-registry single-ownership, standardized
  CLI exit codes (0 success / 1 detected failure / 2 usage-or-config error).
- Spec-driven workflow (`/smelt:spec`, `/smelt:plan`, `/smelt:implement`,
  `/smelt:validate`) and the full `docs/specs/` catalogue as the normative
  reference for every feature.

### Changed
- CLI execute-loop unified onto `smelt-runtime::execute_project`, consumed
  identically by the CLI and the web UI (`docs/specs/architecture.md`
  "Run pipeline parity rule").
- `smelt-parser`/`smelt-db`/`smelt-planner` layering hardened: pure logical
  plan model moved to `smelt-logical`, sitting below both `smelt-db` and
  `smelt-planner`.

### Fixed
- Numerous parser/type-inference conformance fixes verified against a real
  DuckDB (`UNION BY NAME`, `MATERIALIZED` CTEs, `NATURAL JOIN`, `TRY_CAST`,
  `GROUP BY ALL`/`ORDER BY ALL`, `IGNORE`/`RESPECT NULLS`, SQL-standard
  `TRIM`/`SUBSTRING`/`POSITION` forms, and more) — see
  `docs/specs/architecture.md` "SQL dialect conformance gates".
- Address/selector resolution: workspace identity and discovery
  consolidation, unresolvable-selector hard errors, project isolation for
  multi-project workspaces.
- `smelt ui`: origin-restricted CORS and an explicit `--allow-remote` gate
  for non-loopback binds (default bind stays `127.0.0.1`).

### Security
- `smelt ui` no longer serves permissive CORS by default; binding to a
  non-loopback address requires an explicit opt-in flag and emits a startup
  warning.

## [0.3.2] - 2026-05-05
Stale schema cleanup, unknown-selector tolerance, `smelt docs list/show`
embedded user docs, and the first `docs/specs/` extraction
(architecture, incremental models) launching the spec-driven workflow.

## [0.3.1] - 2026-04-18
smelt-shop regression follow-up (idempotent builds, seed resolution,
`--dry-run` diagnostics, geometric-distribution defaults), Salsa
incremental-query rewrite, and a round of Rust 1.95 clippy fixes.

## [0.3.0] - 2026-04-17
Seeds as first-class `smelt.ref()` targets, CASE/EXTRACT/CTE parser and
type-inference fixes, and sdist packaging for PyPI.

## [0.2.0] - 2026-04-09
`smelt-datagen` synthetic data generator, seed-as-data design, and the LSP
visual-documentation demo suite (diagnostics, goto-definition, hover,
completion, rename, quick fixes) with Playwright-recorded walkthroughs.

## [0.1.0] - 2026-03-21
Initial public release: Rowan-based error-recovery parser, Salsa-incremental
`smelt-db`, planner with cross-model optimization, DuckDB backend, and the
`smelt-lsp` language server.

[Unreleased]: https://github.com/adbrowne/smelt-sql/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/adbrowne/smelt-sql/compare/v0.3.2...v0.5.0
[0.3.2]: https://github.com/adbrowne/smelt-sql/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/adbrowne/smelt-sql/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/adbrowne/smelt-sql/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/adbrowne/smelt-sql/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/adbrowne/smelt-sql/releases/tag/v0.1.0
