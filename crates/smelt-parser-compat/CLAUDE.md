# crates/smelt-parser-compat/CLAUDE.md

Multi-dialect compatibility testing — verifies that smelt's parser accepts the same SQL that reference parsers (pg_query, sqlparser-rs, sqlglot) accept, and surfaces gaps where smelt diverges from the target dialects.

## How to test

```bash
# Parse equivalence tests (pg_query + sqlparser-rs, no external tools)
cargo test -p smelt-parser-compat parse_equivalence

# With sqlglot validation (requires Python + sqlglot installed)
SQLGLOT_AVAILABLE=1 cargo test -p smelt-parser-compat

# Type-checking tests (require Docker — run explicitly)
cargo test -p smelt-parser-compat type_checking -- --ignored

# DuckDB differential harness (both dialect-conformance directions).
# Needs the DuckDB system shared library on the linker/loader path:
export DUCKDB_LIB_DIR=~/.local/lib/duckdb
export LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH
cargo test -p smelt-parser-compat --test duckdb_differential
```

## DuckDB differential harness

`tests/duckdb_differential.rs` + `src/duckdb_oracle.rs` enforce architecture
spec §Constraints #13 against a real in-memory DuckDB:

- **Accept direction** — every statement in `tests/corpus/duckdb_seed.sql` that
  DuckDB accepts must parse cleanly in smelt or match a `gaps.rs` entry
  (categories `duckdb_fails_to_parse` / `roundtrip_mismatch`).
- **Fidelity direction** — every seed statement smelt parses cleanly is printed
  back and *executed* on DuckDB; a rejection is a silent-mis-parse bug unless
  registered. A `proptest` variant runs the same fidelity check over the
  generators.
- **Gap ratchet** — `.claude/parser-gaps-baseline.txt` pins the registered
  seed-gap count (mirrors `.claude/hardening-baseline.txt`). The count may only
  shrink; the `gap_count_ratchet` test fails on both an unregistered increase
  and a stale (too-high) baseline. To add a gap: add a `gaps.rs` entry AND raise
  the baseline in the same reviewer-visible change. To close one: remove the
  entry AND lower the baseline.

Every seed line must be valid DuckDB SQL against the schema prelude in
`src/duckdb_oracle.rs` (table `t(a INTEGER, b VARCHAR, c DOUBLE, d DATE,
ts TIMESTAMP)`); statements DuckDB itself rejects create no accept-direction
pressure.

## Gotchas

- **Three reference parsers, different availability.** `pg_query` and `sqlparser-rs (DatabricksDialect)` are pure-Rust dependencies, always available. `sqlglot` requires Python and is opt-in via `SQLGLOT_AVAILABLE=1`. Docker-based type-checking tests are `#[ignore]`d by default.
- **`gaps.rs` records known parse divergences.** When smelt fails to parse something that a reference parser accepts, add an entry to `gaps.rs` rather than failing the test. This keeps the test suite green while making divergences visible.
- **`normalize.rs` canonicalizes SQL** before comparison — whitespace, identifier casing, etc. — so that superficially different representations of the same query are treated as equivalent.
- **`generators.rs`** contains SQL fragment generators for property-based compatibility tests. These are distinct from the type-inference generators in `smelt-db/tests/prop_helpers/generators.rs`.

## Where things live

- `src/lib.rs` — `SmeltParseResult`, test entry points, `SQLGLOT_AVAILABLE` env check
- `src/gaps.rs` — known parse divergences registry
- `src/normalize.rs` — SQL normalization for comparison
- `src/generators.rs` — SQL fragment generators for property tests
