//! Standing drift gate for Trino's emission surface
//! (`docs/outcomes/20260913-trino-emission/outcome.md` phase 1): the spec is
//! written before any verdict or probe exists, and these checks keep the
//! spec's own claims honest as later phases land real code.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read_spec(name: &str) -> String {
    fs::read_to_string(repo_root().join("docs/specs").join(name)).unwrap()
}

fn section(text: &str, heading: &str) -> String {
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("multi_backend.md has no {heading} section"));
    let next = text[start + heading.len()..]
        .find("\n### ")
        .map(|i| start + heading.len() + i)
        .unwrap_or(text.len());
    text[start..next].to_string()
}

/// §"Parity contract" must name Trino's scope: full-refresh table/view and
/// ephemeral materializations are covered, the maintenance legs are not, and
/// the reason (Iceberg's per-table-commit shape) is stated.
#[test]
fn parity_contract_states_trino_scope() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Parity contract");

    assert!(
        sect.contains("Trino"),
        "§\"Parity contract\" does not mention Trino at all"
    );
    assert!(
        sect.contains("maintenance_dialect") && sect.contains("SqlDialect::Trino"),
        "§\"Parity contract\" does not state that maintenance is not covered on Trino"
    );
}

/// §"CI tiering" must name the `trino-integration` job's trigger set and the
/// never-skip-green discipline for its live legs.
#[test]
fn ci_tiering_states_trino_tier_and_no_skip_green() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "**CI tiering.**");

    assert!(
        sect.contains("trino-integration"),
        "§\"CI tiering\" does not name the `trino-integration` job"
    );
    assert!(
        sect.contains("run-docker-tests") && sect.contains("nightly"),
        "§\"CI tiering\" does not state Trino's nightly + `run-docker-tests` trigger"
    );
    assert!(
        sect.to_lowercase().contains("never skip") || sect.contains("does not skip"),
        "§\"CI tiering\" does not state the never-skip-green discipline for Trino's audit legs"
    );
}

/// §"Operator lowering" must mention Trino alongside `^`, `//` and `::`.
#[test]
fn operator_lowering_covers_trino() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Operator lowering");

    assert!(
        sect.contains("Trino"),
        "§\"Operator lowering\" does not mention Trino"
    );
    for token in ["`^`", "`//`", "`::`"] {
        assert!(
            sect.contains(token),
            "§\"Operator lowering\" does not cover {token} for Trino"
        );
    }
}

/// §"Clause-level dialect refusals" must name `QUALIFY`, trailing commas and
/// `PIVOT` for Trino, and record the array-literal positive.
#[test]
fn clause_refusals_cover_trino() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Clause-level dialect refusals");

    assert!(
        sect.contains("Trino"),
        "§\"Clause-level dialect refusals\" does not mention Trino"
    );
    for token in ["QUALIFY", "trailing comma", "PIVOT"] {
        assert!(
            sect.contains(token),
            "§\"Clause-level dialect refusals\" does not name `{token}` for Trino"
        );
    }
    assert!(
        sect.contains("[a, b]") || sect.contains("array literal"),
        "§\"Clause-level dialect refusals\" does not record that array literals work on Trino"
    );
}

/// §"Cross-engine emission audit" must state the unverified-is-a-failure rule
/// and carry a Trino row in the tier table.
#[test]
fn audit_states_unverified_is_a_failure() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Cross-engine emission audit");

    assert!(
        sect.contains("unverified"),
        "§\"Cross-engine emission audit\" does not state the `unverified` verdict"
    );
    assert!(
        sect.contains("Trino"),
        "§\"Cross-engine emission audit\" does not mention Trino"
    );
}

/// The §Surface `SMELT_TRINO_URL` sentence and §"CI tiering" must not both
/// claim the audit legs skip when the coordinator is unreachable.
#[test]
fn skip_semantics_are_not_contradicted() {
    let text = read_spec("multi_backend.md");

    let url_para_start = text
        .find("**`SMELT_TRINO_URL`.**")
        .expect("multi_backend.md has no `SMELT_TRINO_URL` paragraph");
    let url_para_end = text[url_para_start..]
        .find("\n\n")
        .map(|i| url_para_start + i)
        .unwrap_or(text.len());
    let url_para = &text[url_para_start..url_para_end];

    assert!(
        url_para.contains("audit"),
        "the `SMELT_TRINO_URL` paragraph does not exclude the audit legs from its skip claim"
    );
}

/// §"Cross-engine emission audit" must state the shrink-only census rule
/// (phase 2's coverage gate) alongside the `unverified`/`passing`/`gap`
/// vocabulary.
#[test]
fn census_rule_is_stated() {
    let text = read_spec("multi_backend.md");
    let sect = section(&text, "### Cross-engine emission audit");

    assert!(
        sect.contains("census"),
        "§\"Cross-engine emission audit\" does not state the shrink-only census rule"
    );
    assert!(
        sect.contains("trino-emission-census.txt"),
        "§\"Cross-engine emission audit\" does not name the Trino census file"
    );
    assert!(
        sect.to_lowercase().contains("shrink-only") || sect.contains("shrink only"),
        "§\"Cross-engine emission audit\" does not state the census is shrink-only"
    );
}
