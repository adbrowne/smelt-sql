//! Integration tests for the `smelt-datagen` CLI.
//!
//! Regression for FINDINGS bug #4 from
//! `docs/plans/20260417-smelt-shop-0.3-followup.md` Phase B5:
//! the `min` parameter on the geometric generator must be discoverable from
//! `--help` (or an equivalent generator-help flag) — not buried in docs.

use std::process::Command;

fn datagen_bin() -> &'static str {
    env!("CARGO_BIN_EXE_smelt-datagen")
}

fn run(args: &[&str]) -> String {
    let output = Command::new(datagen_bin())
        .args(args)
        .output()
        .expect("failed to invoke smelt-datagen");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    format!("{stdout}{stderr}")
}

/// Top-level `--help` must point users at the per-generator help so `min`
/// (and any other generator parameter) is discoverable without reading docs.
#[test]
fn help_mentions_generator_listing() {
    let combined = run(&["--help"]);
    assert!(
        combined.contains("list-generators"),
        "smelt-datagen --help should advertise --list-generators so users can \
         discover per-generator parameters; got:\n{combined}",
    );
}

/// The per-generator help (`--list-generators`) must mention every generator
/// parameter — including `min` for `geometric`, which is the parameter that
/// motivated this regression.
#[test]
fn list_generators_mentions_geometric_min() {
    let combined = run(&["--list-generators"]);
    assert!(
        combined.to_lowercase().contains("geometric"),
        "--list-generators output should describe the geometric generator; got:\n{combined}",
    );
    assert!(
        combined.contains("min"),
        "--list-generators output should mention the `min` parameter for the \
         geometric generator (FINDINGS bug #4); got:\n{combined}",
    );
}

/// `--list-generators` must surface the `json_object` generator and its
/// `fields:` parameter, so users discover JSON-payload generation without
/// reading the spec.
#[test]
fn list_generators_mentions_json_object_fields() {
    let combined = run(&["--list-generators"]);
    assert!(
        combined.contains("json_object"),
        "--list-generators output should describe the json_object generator; got:\n{combined}",
    );
    assert!(
        combined.contains("fields"),
        "--list-generators output should mention the `fields:` parameter for \
         json_object; got:\n{combined}",
    );
}

/// `--list-generators` must describe `foreign_key` in terms of the effective
/// (scaled) row count, not `num_rows`, so users understand that the FK bound
/// moves with `--scale-factor` (D-53).
#[test]
fn list_generators_foreign_key_mentions_effective_row_count() {
    let combined = run(&["--list-generators"]);
    assert!(
        combined.contains("foreign_key"),
        "--list-generators output should describe the foreign_key generator; got:\n{combined}",
    );
    assert!(
        combined.contains("effective_row_count") || combined.contains("scale_factor"),
        "--list-generators foreign_key description should mention the effective \
         (scaled) row count so users know the FK bound moves with --scale-factor \
         (D-53); got:\n{combined}",
    );
}

/// `--list-generators` must note that `linked_pools` pool contents are
/// scale-invariant only when no shape field uses `foreign_key` (D-53).
#[test]
fn list_generators_linked_pools_mentions_scale_invariance_qualification() {
    let combined = run(&["--list-generators"]);
    assert!(
        combined.contains("linked_pools"),
        "--list-generators output should describe linked_pools; got:\n{combined}",
    );
    assert!(
        combined.contains("scale-invariant") || combined.contains("scale_invariant"),
        "--list-generators linked_pools description should mention scale-invariance \
         and its qualification by foreign_key (D-53); got:\n{combined}",
    );
}

/// `--list-generators` must surface the `linked_choice` generator and its
/// `pool:` / `field:` parameters so users discover joint-distribution
/// columns without reading the spec.
#[test]
fn list_generators_mentions_linked_choice() {
    let combined = run(&["--list-generators"]);
    assert!(
        combined.contains("linked_choice"),
        "--list-generators output should describe the linked_choice generator; got:\n{combined}",
    );
    assert!(
        combined.contains("pool:"),
        "--list-generators output should mention the `pool:` parameter for \
         linked_choice; got:\n{combined}",
    );
    assert!(
        combined.contains("field:"),
        "--list-generators output should mention the `field:` parameter for \
         linked_choice; got:\n{combined}",
    );
}
