# smelt-parser Tests

Comprehensive test suite for the smelt-parser crate.

## Test Organization

### Unit Tests (`src/parser.rs`)
**78 tests** covering core parser functionality:
- Lexer: Keywords, identifiers, operators, whitespace
- Parser: SELECT, FROM, WHERE, JOIN, GROUP BY, ORDER BY, LIMIT
- CTEs: WITH, RECURSIVE, UNION
- Window functions: OVER, PARTITION BY, frame specs
- Expressions: CASE, CAST, subqueries, BETWEEN, IN, EXISTS
- smelt extensions: `smelt.ref()`, `smelt.metric()` with `=>` parameters

**Location**: Inline with implementation for fast feedback during development.

### Printer Tests (`src/printer/tests.rs`)
**10 tests** verifying SQL regeneration:
- Round-trip preservation for valid SQL
- Keyword uppercasing (SELECT, WHERE, etc.)
- Expression formatting with parentheses
- Multi-clause statement formatting

### Property-Based Tests (`tests/`)
**2810+ generated test cases** using proptest:

#### `proptest_generators.rs` (~290 lines)
Grammar-based SQL generators combining small pieces into complex queries.

#### `proptest_round_trip.rs` (~180 lines)
Property tests verifying round-trip preservation and error recovery.

**Default**: 100 test cases per property (fast for PR checks)  
**CI full**: 1000 test cases (comprehensive validation)

### Fuzz Tests (`fuzz/`)
**Coverage-guided fuzzing** with cargo-fuzz. See `fuzz/README.md`.

## Running Tests

```bash
# Run all tests
cargo test

# Run only unit tests (fast, <1s)
cargo test --lib

# Run with more verbosity
cargo test -- --nocapture
```

## Testing Philosophy

1. **Fast Feedback** - Unit tests inline, property tests default to 100 cases
2. **Grammar-Based** - Generate valid SQL by construction
3. **Error Recovery** - Parser never panics, even on invalid input
4. **Round-Trip** - Valid SQL survives parse → print → parse

See full documentation in TESTING.md (if exists).
