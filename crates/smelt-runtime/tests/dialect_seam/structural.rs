//! Structural assertions over `src/compile.rs`'s own source: every print,
//! restructure-plan, and settlement call must route through the single
//! `print_checked_for` seam, or a new compile path would silently skip the
//! `UnsupportedOnBackend` refusal, the restructure planning, or conditional
//! settlement.

/// No compile entry point may reach the printer directly, or it would skip
/// the refusal. `print_checked_for` is the sole permitted caller.
#[test]
fn every_compile_path_is_emission_checked() {
    const COMPILE_SRC: &str = include_str!("../../src/compile.rs");
    // The two hardwired-DuckDB helpers (`resolve_refs_in_sql` and the
    // function-body expander) are exempt: they take no dialect, return no
    // `Result`, and sit on no path that produces an executed `CompiledModel`.
    // Not because DuckDB is free of unsupported constructs — it declares
    // `PERCENTILE_CONT`/`PERCENTILE_DISC` unsupported in running-window
    // position.
    const EXEMPT: usize = 2;
    let direct = COMPILE_SRC.matches("smelt_dialect::print(").count();
    assert_eq!(
        direct,
        EXEMPT + 1,
        "compile.rs calls `smelt_dialect::print` {direct} times; only \
         `print_checked_for` plus the {EXEMPT} hardwired-DuckDB helpers may. A new \
         compile path must print through `print_checked`, or it skips the \
         `UnsupportedOnBackend` refusal."
    );
}

/// Mirrors `every_compile_path_is_emission_checked`: no compile entry point
/// may plan a statement-level restructure and then print without going
/// through `print_checked_for` — the same seam that refuses an `Unsupported`
/// construct is where a `Restructure` verdict gets planned, so a new print
/// call bypassing it would silently skip both the refusal and the planning.
#[test]
fn no_compile_entry_point_prints_without_planning() {
    const COMPILE_SRC: &str = include_str!("../../src/compile.rs");
    let plan_calls = COMPILE_SRC
        .matches("smelt_dialect::plan_restructure(")
        .count();
    assert_eq!(
        plan_calls, 1,
        "compile.rs calls `smelt_dialect::plan_restructure` {plan_calls} times; only \
         `print_checked_for` may call it. A new compile path constructing its own plan \
         (or none) would drift from the single planning site."
    );
}

/// Mirrors `every_compile_path_is_emission_checked`: settlement of
/// operand-conditional verdicts happens exactly once, inside
/// `print_checked_for`, before printing — never inside `smelt_dialect::print`
/// itself or a bespoke per-caller resolution.
#[test]
fn no_compile_path_prints_with_an_unsettled_conditional() {
    const COMPILE_SRC: &str = include_str!("../../src/compile.rs");
    let settle_calls = COMPILE_SRC
        .matches("smelt_dialect::settle_emissions(")
        .count();
    assert_eq!(
        settle_calls, 1,
        "compile.rs calls `smelt_dialect::settle_emissions` {settle_calls} times; only \
         `print_checked_for` may call it, so every print is preceded by settlement."
    );
}
