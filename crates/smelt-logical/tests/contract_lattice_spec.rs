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
        "ContractDepartedKeyUnmarked",
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
    let section = h2_section_body(&spec, "## Constraints & Invariants");
    assert!(
        section.contains("lattice point") && section.contains("smelt-logical"),
        "§\"Constraints & Invariants\" must state the lattice-point single-owner rule, \
         naming `smelt-logical`"
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
fn deferral_triple_is_complete() {
    let module = read("crates/smelt-logical/src/contract/deferral.rs");
    assert!(
        module.contains("pub fn validate_deferral"),
        "the declaration-schema validator leg must be present"
    );
    assert!(
        module.contains("pub fn measure_lag"),
        "the oracle-transform (lag) leg must be present"
    );
    assert!(
        module.contains("pub fn deferral_violations"),
        "the probe-emitter leg (the comparison the runtime dispatch consumes) must be present"
    );

    let mod_doc = read("crates/smelt-logical/src/contract/mod.rs");
    assert!(
        !mod_doc.contains("unbuilt"),
        "contract/mod.rs's landing-status doc must be updated once the deferral triple lands"
    );
    assert!(
        mod_doc.contains("`deferral`: the complete triple has landed"),
        "contract/mod.rs must state that the deferral triple has landed"
    );
}

#[test]
fn deferral_capabilities_are_single_owned() {
    let module = read("crates/smelt-logical/src/contract/deferral.rs");
    assert!(
        module.contains("pub fn run_license"),
        "the run-skip licensing decision must be single-owned in smelt-logical"
    );
    assert!(
        module.contains("pub fn pending_window"),
        "the pending-window transform must be single-owned in smelt-logical"
    );
    assert!(
        module.contains("pub fn subsumption"),
        "the work-subsumption proof must be single-owned in smelt-logical"
    );

    // `smelt-runtime` may only call these decisions, never re-derive them —
    // no independent comparison of the two frontiers, or of the
    // pending/scheduled ranges, outside the `smelt_logical::contract::deferral`
    // functions above.
    let runtime_dirs = ["crates/smelt-runtime/src", "crates/smelt-runtime/tests"];
    for dir in runtime_dirs {
        let dir = repo_root().join(dir);
        for entry in std::fs::read_dir(&dir).expect("read runtime dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            if path.file_name().and_then(|n| n.to_str()) == Some("contract_probes.rs")
                || contents.contains("smelt_logical::contract::deferral")
            {
                // Uses the shared functions — expected. Guard against a
                // parallel reimplementation: no local lag-vs-window
                // arithmetic that duplicates `run_license`/`subsumption`.
                assert!(
                    !contents.contains("lag.lag <= d") && !contents.contains("lag <= d"),
                    "{}: reimplements the deferral lag-vs-window comparison instead of calling \
                     smelt_logical::contract::deferral::run_license",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn conformance_gate_consumes_the_oracle_transform() {
    // Phase 6 (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`):
    // the conformance gate must dispatch on `smelt_logical::contract`'s pure
    // transforms (`oracle_obligation`, `restrict_run_window`,
    // `settled_cutoff`) rather than re-deriving a lattice point's
    // restriction locally — mirrors `deferral_capabilities_are_single_owned`
    // above, scoped to the two crates the conformance gate's own oracle
    // machinery lives in.
    let dirs = [
        "crates/smelt-maintenance-testkit/src",
        "crates/smelt-cli/tests/maintenance_conformance",
    ];

    let mut calls_oracle_obligation = false;
    let mut calls_restrict_run_window = false;
    let mut calls_settled_cutoff = false;

    for dir in dirs {
        let dir = repo_root().join(dir);
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let contents = std::fs::read_to_string(&path).unwrap_or_default();

            if contents.contains("contract::oracle_obligation") {
                calls_oracle_obligation = true;
            }
            if contents.contains("contract::restrict_run_window") {
                calls_restrict_run_window = true;
            }
            if contents.contains("contract::settled_cutoff") {
                calls_settled_cutoff = true;
            }

            // Guard against a parallel reimplementation of the per-point
            // restriction arithmetic instead of calling the shared
            // transforms above — scanned over CODE lines only (comments and
            // doc comments narrating the concept in prose are expected and
            // are not a reimplementation).
            for line in contents.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                assert!(
                    !code.contains("end - h") && !code.contains("end-h"),
                    "{}: reimplements the frozen-horizon window restriction instead of calling \
                     smelt_logical::contract::restrict_run_window: {line:?}",
                    path.display()
                );
                assert!(
                    !code.contains("input_frontier - d") && !code.contains("frontier - d"),
                    "{}: reimplements the deferral settled-cutoff arithmetic instead of calling \
                     smelt_logical::contract::settled_cutoff: {line:?}",
                    path.display()
                );
            }
        }
    }

    assert!(
        calls_oracle_obligation,
        "the conformance gate must dispatch on smelt_logical::contract::oracle_obligation"
    );
    assert!(
        calls_restrict_run_window,
        "the S-tracker must restrict recorded run windows via \
         smelt_logical::contract::restrict_run_window, not a local formula"
    );
    assert!(
        calls_settled_cutoff,
        "the deferral bracket's settled leg must call \
         smelt_logical::contract::settled_cutoff, not a local formula"
    );
}

#[test]
fn explain_contract_rendering_is_single_owned() {
    // Phase 7 (`docs/outcomes/20260809-contract-lattice-v1/phases/07-plan.md`):
    // the CLI's `smelt explain` per-cell contract row/`contract_point` JSON
    // field must resolve through `smelt_logical::contract::effective_contract`
    // — no local model-vs-cell ladder over `ContractConfig` in `smelt-cli`.
    let module = read("crates/smelt-logical/src/contract/mod.rs");
    assert!(
        module.contains("pub fn effective_contract"),
        "the per-cell effective-contract resolution must be single-owned in smelt-logical"
    );

    let explain_rs = read("crates/smelt-cli/src/explain.rs");
    assert!(
        explain_rs.contains("smelt_logical::contract::effective_contract"),
        "smelt-cli's explain.rs must call smelt_logical::contract::effective_contract, not \
         re-derive the model-vs-cell ladder locally"
    );

    // Guard against a parallel reimplementation of the narrower-wins match:
    // `smelt-cli` must never scan `contract.cells` / `ContractConfig` fields
    // itself to resolve which point wins for a cell.
    for line in explain_rs.lines() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        assert!(
            !code.contains("cfg.cells.iter()") && !code.contains("contract_cfg.cells.iter()"),
            "crates/smelt-cli/src/explain.rs: reimplements the contract.cells[] matching \
             ladder instead of calling smelt_logical::contract::effective_contract: {line:?}"
        );
    }
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
