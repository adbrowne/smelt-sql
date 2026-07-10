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

# External corpus (vendored DuckDB sqllogictest + PostgreSQL regression SELECT
# statements) — same DUCKDB_LIB_DIR/LD_LIBRARY_PATH requirement as above:
cargo test -p smelt-parser-compat --test external_corpus
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

## External corpus (`tests/external_corpus.rs`)

A second, independent conformance gate against a vendored SELECT-only sample
of two upstream test suites — DuckDB's sqllogictest suite and PostgreSQL's
regression suite — rather than a hand-written seed corpus:

- `tests/corpus/external/duckdb.sql`, `tests/corpus/external/postgres.sql` —
  vendored, one normalized statement per line. See
  `tests/corpus/external/README.md` for license/attribution notices (DuckDB:
  MIT; PostgreSQL: the PostgreSQL License).
- `tests/corpus/external_ledger.toml` — the failure ledger, keyed by a
  deterministic hash of the exact corpus line (`stmt_hash` in
  `tests/external_corpus.rs`, an inline-vendored FNV-1a 64-bit — a frozen,
  fully specified algorithm, so keys are stable across runs, machines, and
  Rust toolchain releases; never replace it with `DefaultHasher`, whose
  algorithm is unspecified and may change between Rust versions). Each entry
  is `category` + free-text `note`; notes must not contain embedded `"` or
  lines starting with `[` (the restricted TOML parser has no escapes).
- **Gate:** `corpus_statements_parse_or_ledgered` — every corpus statement
  must either parse cleanly in smelt (and, if DuckDB accepts the *original*
  statement standalone with no schema — i.e. it's fully self-contained —
  the printed form must also be accepted, closing the same silent-mis-parse
  class as the seed-corpus fidelity gate) or have a ledger entry.
- **Shrink-only pressure:** `ledger_has_no_stale_entries` fails the build if
  a ledger entry's statement now passes, or no longer matches any corpus
  statement (e.g. after a corpus refresh) — remove the entry.
- **Filter self-test:** `extraction_filter_self_test` shells out to
  `python3 scripts/extract-sql-corpus.py --self-test`, which is the single
  source of truth for the extraction filter's `is_select_only` / `normalize`
  / `dedup_and_cap` logic (not duplicated in Rust — the script is the only
  thing that runs the filter, since extraction is a one-time/occasional
  refresh, not a CI step). This test **fails closed** if no `python3`/`python`
  interpreter is on PATH; an environment that genuinely cannot provide Python
  opts out explicitly with `SMELT_SKIP_PY_SELFTEST=1`.

### Refreshing the corpus

The extraction script is documented, re-runnable, and **not** run in CI:

```bash
python3 scripts/extract-sql-corpus.py
```

This re-downloads the pinned DuckDB/PostgreSQL tags (edit `DUCKDB_TAG` /
`POSTGRES_TAG` at the top of the script to bump them), re-extracts and
re-filters the SELECT-only subset, and overwrites the vendored corpus files
and README. After refreshing:

1. Run `cargo test -p smelt-parser-compat --test external_corpus` and read
   the unledgered-failure list.
2. Triage each new failure into `tests/corpus/external_ledger.toml` with a
   `category` (reuse a `gaps.rs` category id where the construct matches an
   existing registered gap) and a `note`. Given how much grammar surface
   these upstream suites probe, most entries are bucketed by pattern rather
   than hand-triaged one at a time; a residual with no matching pattern is
   recorded generically as `smelt_fails_unclassified` with the actual first
   smelt parser error as the note — do not fabricate a root cause you
   haven't actually diagnosed.
3. Re-run the test; `ledger_has_no_stale_entries` will also catch any
   pre-existing entry that a refresh caused to no longer match.

Populating this ledger deliberately does not fix any gap it surfaces — the
ledger's role is to size and make visible the follow-on grammar work (see
`docs/specs/architecture.md` Known Divergences and
`docs/plans/20260711-parser-type-testing-hardening.md` "Explicitly deferred").

## Gotchas

- **Three reference parsers, different availability.** `pg_query` and `sqlparser-rs (DatabricksDialect)` are pure-Rust dependencies, always available. `sqlglot` requires Python and is opt-in via `SQLGLOT_AVAILABLE=1`. Docker-based type-checking tests are `#[ignore]`d by default.
- **`gaps.rs` records known parse divergences.** When smelt fails to parse something that a reference parser accepts, add an entry to `gaps.rs` rather than failing the test. This keeps the test suite green while making divergences visible.
- **`normalize.rs` canonicalizes SQL** before comparison — whitespace, identifier casing, etc. — so that superficially different representations of the same query are treated as equivalent.
- **`generators.rs`** contains SQL fragment generators for property-based compatibility tests. These are distinct from the type-inference generators in `smelt-db/tests/prop_helpers/generators.rs`.

## Where things live

- `src/lib.rs` — `SmeltParseResult`, test entry points, `SQLGLOT_AVAILABLE` env check
- `src/gaps.rs` — known parse divergences registry
- `src/duckdb_oracle.rs` — in-memory DuckDB accept/execute oracle
- `src/normalize.rs` — SQL normalization for comparison
- `src/generators.rs` — SQL fragment generators for property tests
- `tests/duckdb_differential.rs`, `tests/corpus/duckdb_seed.sql` — hand-written DuckDB differential seed corpus + ratchet
- `tests/external_corpus.rs`, `tests/corpus/external/` (vendored corpus + README), `tests/corpus/external_ledger.toml` — vendored DuckDB sqllogictest / PostgreSQL regression SELECT corpus + failure ledger
- `scripts/extract-sql-corpus.py` (repo root) — the corpus extraction/refresh script
