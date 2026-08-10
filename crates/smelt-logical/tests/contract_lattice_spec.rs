//! Standing gate for `docs/specs/incremental_models.md` §"The contract lattice"
//! and §"Contract relaxations (`contract:`)"
//! (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 1): the
//! default point plus the two v1 relaxations (`frozen_horizon`, `deferral`),
//! each with a restated oracle and probe, the single-owner admission rule,
//! the `contract:` surface block, the four diagnostic codes, and the
//! specified-and-unimplemented Known Divergence all exist before any code
//! consumes them.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Extracts the body of a `### <heading>` section: everything between its
/// heading and the next `##`/`###` heading.
fn section_body<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("document must have a {heading:?} heading"));
    let after_heading = &doc[start..];
    let body_start = after_heading
        .find('\n')
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let body = &after_heading[body_start..];
    let end = body
        .find("\n## ")
        .or_else(|| body.find("\n### "))
        .unwrap_or(body.len());
    &body[..end]
}

/// Extracts the body of a `## <heading>` section, including its
/// `### `-level subsections: everything up to the next `## ` heading.
fn h2_section_body<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("document must have a {heading:?} heading"));
    let after_heading = &doc[start..];
    let body_start = after_heading
        .find('\n')
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let body = &after_heading[body_start..];
    let end = body.find("\n## ").unwrap_or(body.len());
    &body[..end]
}

#[test]
fn lattice_section_states_default_point_and_two_v1_points() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### The contract lattice");

    assert!(
        section.contains("default point") || section.contains("default"),
        "§\"The contract lattice\" must name the equivalence invariant as the default point"
    );
    for name in ["frozen_horizon", "deferral"] {
        assert!(
            section.contains(name),
            "§\"The contract lattice\" must name lattice point `{name}`"
        );
    }
}

#[test]
fn each_point_states_its_oracle_transform_and_probe() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### The contract lattice");

    assert!(
        section.contains("frozen horizon") || section.contains("Frozen horizon"),
        "§\"The contract lattice\" must have a frozen-horizon subsection"
    );
    assert!(
        section.contains("ContractLateArrivalOutsideHorizon"),
        "the frozen-horizon point must name its probe diagnostic"
    );
    assert!(
        section.contains("deferral") && section.contains("D"),
        "§\"The contract lattice\" must have a deferral subsection"
    );
    assert!(
        section.contains("ContractDeferralExceeded"),
        "the deferral point must name its probe diagnostic"
    );
    assert!(
        section.contains("oracle"),
        "each point must restate its oracle, not just name the relaxation"
    );
}

#[test]
fn admission_rule_is_the_single_owner_triple() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### The contract lattice");

    assert!(
        section.contains("declaration schema")
            && section.contains("oracle")
            && section.contains("probe"),
        "§\"The contract lattice\" must state the three-part admission rule \
         (declaration schema, pure oracle transform, probe emitter)"
    );
    assert!(
        section.contains("smelt-logical"),
        "§\"The contract lattice\" must locate single ownership in `smelt-logical`"
    );
}

#[test]
fn surface_catalogues_the_contract_block() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### Contract relaxations (`contract:`)");

    assert!(
        section.contains("contract:"),
        "§\"Contract relaxations\" must show the `contract:` block"
    );
    assert!(
        section.contains("frozen_horizon") && section.contains("deferral"),
        "§\"Contract relaxations\" must document both `frozen_horizon:` and `deferral:`"
    );
    assert!(
        section.contains("partition"),
        "§\"Contract relaxations\" must state the partition-grain-only restriction on \
         `frozen_horizon`"
    );
    assert!(
        section.contains("cells"),
        "§\"Contract relaxations\" must document per-cell refinement via `cells:`"
    );
}

#[test]
fn horizon_section_scopes_silent_exclusion_to_the_default_point() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### Windowed maintenance and the horizon");

    assert!(
        section.contains("default point"),
        "§\"Windowed maintenance and the horizon\" must scope the silent-late-arrival \
         wording to the default point"
    );
    assert!(
        section.contains("frozen horizon") || section.contains("frozen_horizon"),
        "§\"Windowed maintenance and the horizon\" must cross-reference the frozen-horizon \
         lattice point"
    );
}

#[test]
fn diagnostics_tables_carry_the_four_lattice_codes() {
    let incremental = read("docs/specs/incremental_models.md");
    let diagnostics = read("docs/specs/diagnostics.md");

    let codes = [
        "ContractFrozenHorizonInvalid",
        "ContractLateArrivalOutsideHorizon",
        "ContractDeferralInvalid",
        "ContractDeferralExceeded",
    ];
    for code in codes {
        assert!(
            incremental.contains(code),
            "incremental_models.md §Diagnostics must list `{code}`"
        );
        assert!(
            diagnostics.contains(code),
            "diagnostics.md must catalogue `{code}`"
        );
    }
}

#[test]
fn constraint_and_claude_md_state_the_lattice_invariant() {
    let spec = read("docs/specs/incremental_models.md");
    let section = section_body(&spec, "### The contract, plan, and graph layer");
    assert!(
        section.contains("lattice point") && section.contains("smelt-logical"),
        "§Constraints & Invariants → \"The contract, plan, and graph layer\" must state the \
         lattice-point single-owner rule, naming `smelt-logical`"
    );

    let claude_md = read("CLAUDE.md");
    assert!(
        claude_md.contains("lattice point") || claude_md.contains("contract lattice"),
        "CLAUDE.md's architectural-invariants list must carry a matching lattice-point bullet"
    );
}

#[test]
fn frozen_horizon_triple_is_complete() {
    let module = read("crates/smelt-logical/src/contract/frozen_horizon.rs");
    assert!(
        module.contains("pub fn validate_frozen_horizon"),
        "the declaration-schema validator leg must be present"
    );
    assert!(
        module.contains("pub fn clamp_write_range"),
        "the oracle-transform (write-clamp) leg must be present"
    );
    assert!(
        module.contains("pub fn emit_frozen_band_snapshot"),
        "the probe-emitter leg must be present — the frozen-horizon triple is now complete"
    );
    assert!(
        module.contains("pub fn late_arrivals"),
        "the pure baseline-comparison the probe emitter's dispatch consumes must be present"
    );

    let mod_doc = read("crates/smelt-logical/src/contract/mod.rs");
    assert!(
        !mod_doc.contains("lands in phase 3"),
        "contract/mod.rs's landing-status doc must be updated once the probe emitter lands"
    );
    assert!(
        mod_doc.contains("has landed") || mod_doc.contains("landed —"),
        "contract/mod.rs must state that the frozen_horizon triple has landed"
    );
}

#[test]
fn known_divergence_tracks_the_unimplemented_lattice() {
    let spec = read("docs/specs/incremental_models.md");
    let section = h2_section_body(&spec, "## Known Divergences / Open Questions");
    assert!(
        section.contains("contract lattice") || section.contains("lattice"),
        "§Known Divergences must record the lattice as specified-and-unimplemented"
    );
    assert!(
        section.contains("docs/outcomes/20260809-contract-lattice-v1/outcome.md"),
        "the divergence entry must link the tracking outcome"
    );
}
