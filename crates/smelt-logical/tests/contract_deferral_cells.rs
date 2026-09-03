//! Per-cell `contract.cells[].deferral` addressing and licensing
//! (`docs/specs/incremental_models.md` §"The contract lattice", deferral;
//! `docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`).
//! Pure over `smelt_logical::contract::deferral` — no I/O.

use smelt_logical::contract::deferral::{cell_address, run_license, RunLicense};

#[test]
fn cell_address_is_stable_for_group_and_trigger() {
    let a = cell_address(
        &["total_amount".to_string(), "event_date".to_string()],
        "raw.events",
    );
    let b = cell_address(
        &["event_date".to_string(), "total_amount".to_string()],
        "raw.events",
    );
    assert_eq!(
        a, b,
        "cell_address must be order-insensitive within `columns`"
    );

    let different_trigger = cell_address(&["event_date".to_string()], "raw.other");
    let same_columns_same_trigger = cell_address(&["event_date".to_string()], "raw.events");
    assert_ne!(different_trigger, same_columns_same_trigger);

    let different_columns = cell_address(&["total_amount".to_string()], "raw.events");
    assert_ne!(different_columns, same_columns_same_trigger);
}

#[test]
fn cell_license_skips_only_within_its_own_d() {
    // Maintained 100, input 106: lag 6, within D=6 -> Skip.
    assert_eq!(
        run_license(Some(100), Some(106), 6),
        RunLicense::Skip { lag: 6, d: 6 }
    );

    // Maintained 100, input 110: lag 10, beyond D=6 -> Run.
    assert_eq!(run_license(Some(100), Some(110), 6), RunLicense::Run);

    // lag <= 0 never skips, even with a generous D.
    assert_eq!(run_license(Some(106), Some(100), 6), RunLicense::Run);

    // An unresolved (first-run) maintained frontier never skips.
    assert_eq!(run_license(None, Some(106), 6), RunLicense::Run);
    assert_eq!(run_license(Some(100), None, 6), RunLicense::Run);
}
