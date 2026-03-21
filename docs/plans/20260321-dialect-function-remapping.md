# Expand Backend Conformance Test Coverage via Dialect Function Remapping

## Context

The type conformance tests (`type_conformance_tests.rs`) verify that after wrapping SQL with type casts, backends produce exactly the types smelt inferred. However, these tests send raw SQL directly to DuckDB without going through the dialect printer. Functions like EVERY were excluded from generators because DuckDB doesn't support them natively.

The dialect printer (`crates/smelt-dialect/src/printer.rs`) already handles EXPLODE<->UNNEST renaming per dialect. We need to extend this pattern to cover function name differences across backends, then wire the conformance tests through the printer so we can test all functions smelt supports.

## Approach

1. Add function name remapping to the dialect printer (extending the EXPLODE/UNNEST pattern)
2. Wire the conformance tests to pass SQL through the printer before executing
3. Add back previously-excluded functions to generators (EVERY, LEFT, RIGHT)
4. One commit per logical piece, fix bugs along the way, append to existing PR

## Steps

### Step 1: Add function name remapping table to dialect printer

Replace the ad-hoc EXPLODE/UNNEST if-else chain with a general `remap_function_name()` lookup.

Mappings: DuckDB/PostgreSQL: EVERY->BOOL_AND. SparkSQL: BOOL_AND->EVERY, BOOL_OR->SOME.

### Step 2: Wire conformance tests through dialect printer

Add `translate_for_backend()` to conformance tests that parses SQL and prints through the dialect printer before DuckDB execution. Type inference still runs on the original SQL.

### Step 3: Add EVERY back to generators

Now that the dialect printer remaps EVERY->BOOL_AND for DuckDB, re-add to generators.

### Step 4: INITCAP - skipped

No DuckDB equivalent exists. Would need complex expression rewrite, not a simple rename.

### Step 5: Fix LEFT/RIGHT parser keyword conflict

Add LEFT_KW and RIGHT_KW to `at_keyword_as_function_name()` in the parser. When followed by `(`, they're function calls; otherwise they're JOIN keywords. Re-add LEFT/RIGHT to generators.

### Step 6: Update TODO.md and commit plan
