## Drift Report: D5 — seeds × sources × types (combination seam)

**Specs**: docs/specs/seeds.md, docs/specs/sources.md, docs/specs/types.md
**Date**: 2026-06-08
**Phase**: D5 (feature sweep combination probe)

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS
- cargo test — PASS
- example_diagnostics — PASS (101 tests)
- example_workspaces — PASS (30 tests)
- source_diagnostics — PASS (5 tests including 2 new D5 tests)

### Surface drift

#### Seeds
- ✅ Type aliases (INT, BOOL, INT8, DECIMAL(4,2), TIMESTAMPTZ, TEXT) all accepted in seed sidecar YAML — verified by `seed_source_type_join` fixture
- ✅ Type inference BOOLEAN/DATE/TIMESTAMP/INTEGER/DECIMAL/DOUBLE/VARCHAR — prior C5 coverage still green
- ✅ Sidecar column-set agreement, type-coercion, nullable — prior C5 coverage green

#### Sources
- ✅ `SourceTypeError` for unrecognised column type — **new fixture** `sources_broken_unknown_type` + test `source_type_error_surfaces_diagnostic` confirms code is correct (was previously untested)
- ✅ `MalformedSource` for `materialization:` on source — prior fixture coverage still green
- ✅ Type aliases (INT, INT8, TIMESTAMPTZ, TEXT, DECIMAL) accepted in source YAML — verified by `seed_source_type_join` fixture
- ❌ `timeseries:` declared on source YAML is silently ignored — BUG-072

#### Types (seam)
- ✅ Type aliases accepted identically in seed sidecars and source YAMLs (both use `smelt_types::parse_type`)
- ✅ Join between seed (inferred/pinned types) and source (declared types) with matching INTEGER column is type-clean
- ✅ DECIMAL arithmetic on seed column (`discount_pct DECIMAL(4,2)`) compiles without diagnostic

### Semantics drift
- ❌ `sources.md` Semantics "Sources with timeseries: opt into pushdown" — unimplemented; BUG-072 (data model) + BUG-073 (execute path)
- ✅ Seeds: all 8 semantics rules verified by prior C5 tests and D5 fixture

### Invariant drift
- ⚠️ `sources.md` §"Source YAML shape" `timeseries` key — accepted by parser (no `deny_unknown_fields`) but has no effect; SourceInfo has no timeseries field — BUG-072

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in seeds.md, sources.md, or types.md

### Freshness
- seeds.md last_reviewed: 2026-05-05 (stale — code changes since)
- sources.md last_reviewed: 2026-05-21 (stale — BUG-032 family fixed after that date)
- types.md last_reviewed: 2026-05-27 (reasonably fresh)

### Summary
- Drift items: 2 (BUG-072, BUG-073 — source timeseries data model gap + incremental pushdown not wired)
- Fixed: 0
- Needs-review: 2
- New test coverage added: `SourceTypeError` fixture (`sources_broken_unknown_type`) + `seed_source_type_join` clean fixture
