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
```

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
