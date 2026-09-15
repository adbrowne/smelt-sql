//! The fail-loud leg of the staged relation group's atomicity contract
//! (`docs/outcomes/20260913-trino-ledger/phases/07-plan.md` criterion 8): a
//! group built for a `staged_relation_group_is_atomic == false` capability
//! must never carry `StatementGroup::transactional == true` into
//! `Backend::execute_statement_group`'s default sequential implementation
//! (`crates/smelt-backend/src/lib.rs`), which runs each statement one at a
//! time with no transaction and no warning — a claim silently not honoured
//! is exactly the fail-loud discipline's own failure shape.
//!
//! Every staged-relation emitter derives `transactional` from the
//! [`StagedRelation`] it is handed, never from a hardcoded `true` — this is
//! a standing gate over all five emitters (the four keyed/recompute shapes
//! plus the keyless whole-row shape), not a fix for one. A further standing
//! fact closes criterion 3 (a claim implies a builder): no source file in
//! `crates/smelt-logical/src/maintenance/emit/` spells `CREATE TEMP TABLE`
//! outside `staged_relation.rs`'s own `create_prefix`.
//!
//! `smelt_backend::maintenance_dialect(SqlDialect::Trino)` now resolves
//! (`20260913-trino-incremental` phase 3), so it no longer stands between a
//! Trino caller and the keyless executor — which families actually reach it
//! is `20260913-trino-incremental` phase 5's subject (the merge-less
//! conditional write over T3's staged relation), not this file's.

use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::{
    emit_diff_patch, emit_per_group_recompute, emit_staged_candidate_conditional,
    emit_staged_candidate_conditional_keyless, emit_staged_candidate_conditional_recompute,
    MaintenanceDialect, StagedRelation, StagedRelationResidence,
};

fn non_atomic_relation(name: &str) -> StagedRelation {
    StagedRelation::derive(
        "__smelt_staged_",
        name,
        StagedRelationResidence::TargetSchema,
        false,
    )
}

#[test]
fn no_non_atomic_backend_is_handed_a_transactional_group() {
    let key = vec!["id".to_string()];
    let compared = vec!["v".to_string()];

    let conditional = emit_staged_candidate_conditional(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(!conditional.transactional);

    let recompute = emit_staged_candidate_conditional_recompute(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(!recompute.transactional);

    let per_group = emit_per_group_recompute(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT DISTINCT id AS delta_key FROM delta",
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(!per_group.transactional);

    let diff_patch = emit_diff_patch(
        "main.t",
        &non_atomic_relation("t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        "TRUE",
        &DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert!(!diff_patch.transactional);

    // The atomic (session-temporary) shape stays `true` — this is the
    // non-regression half of the same gate.
    let atomic = emit_staged_candidate_conditional(
        "main.t",
        &StagedRelation::session_temporary("__smelt_staged_t"),
        &key,
        "SELECT id, v FROM src",
        &compared,
        MaintenanceDialect::DuckDb,
    );
    assert!(atomic.transactional);

    let keyless = emit_staged_candidate_conditional_keyless(
        "main.t",
        &non_atomic_relation("t"),
        &non_atomic_relation("t_sentinel"),
        None,
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(!keyless.transactional);

    let keyless_atomic = emit_staged_candidate_conditional_keyless(
        "main.t",
        &StagedRelation::session_temporary("__smelt_staged_t"),
        &StagedRelation::session_temporary("__smelt_sentinel_t"),
        None,
        "SELECT id, v FROM src",
        MaintenanceDialect::DuckDb,
    );
    assert!(keyless_atomic.transactional);
}

/// Regression gate: the string literal `"CREATE TEMP TABLE` must appear in
/// exactly one source file under `crates/smelt-logical/src/maintenance/
/// emit/` — `staged_relation.rs`'s own `create_prefix()` — outside test
/// modules. Doc comments that *describe* emitted SQL (numbered statement
/// lists) are not scanned; only an actual string-literal spelling counts. A
/// staged emitter that hardcodes its own `CREATE TEMP TABLE` spelling
/// instead of deriving it from a [`StagedRelation`] would defeat the whole
/// residence-as-data migration for whichever backend it is (a temp-table
/// spelling on a backend with no temp tables is exactly the
/// claim-without-builder failure criterion 3 excludes).
#[test]
fn no_staged_emitter_hardcodes_a_temp_table_spelling() {
    let emit_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .join("smelt-logical/src/maintenance/emit");
    assert!(emit_dir.is_dir(), "emit dir not found: {emit_dir:?}");

    let mut offending = Vec::new();
    for entry in std::fs::read_dir(&emit_dir).expect("read emit dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path).expect("read source file");
        // Only the non-#[cfg(test)] prefix of each file counts — test
        // modules are allowed to spell out expected literal statement text.
        let production_prefix = match contents.find("#[cfg(test)]") {
            Some(idx) => &contents[..idx],
            None => contents.as_str(),
        };
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "staged_relation.rs" {
            // `create_prefix()` itself is the single owner of this literal.
            continue;
        }
        // Only a string-literal spelling counts — doc comments that
        // *describe* the emitted SQL (e.g. a numbered statement list) are
        // not a hardcoded emission and must not trip this gate.
        if production_prefix.contains("\"CREATE TEMP TABLE") {
            offending.push(path.display().to_string());
        }
    }
    assert!(
        offending.is_empty(),
        "these emit/ source files hardcode `CREATE TEMP TABLE` outside \
         staged_relation.rs's create_prefix(): {offending:?}"
    );
}

/// `smelt_backend::maintenance_dialect(SqlDialect::Trino)` now resolves to
/// `MaintenanceDialect::Trino` (`20260913-trino-incremental` phase 3) — the
/// non-regression counterpart of this file's other assertions, pinning that
/// the mapping succeeds rather than the pre-phase-3 refusal.
#[test]
fn trino_now_resolves_a_maintenance_dialect() {
    let result = smelt_backend::maintenance_dialect(smelt_dialect::SqlDialect::Trino);
    assert_eq!(result, Ok(MaintenanceDialect::Trino));
}

/// `docs/outcomes/20260913-trino-incremental` phase 5, test 4: no
/// production line under `crates/smelt-runtime/src/` spells
/// `StagedRelationResidence::SessionTemporary` or
/// `StagedRelation::session_temporary(` literally — every derivation site
/// must read residence and atomicity off the target's own
/// `BackendCapabilities` (`StagedRelation::derive_for_capabilities`) instead
/// of hardcoding a shape (`docs/specs/multi_backend.md` §"Column-scoped
/// merge and conditional-write capabilities"). Test-only code (below a
/// file's own `#[cfg(test)]`) is exempt — a unit test asserting against a
/// known-shape expected value legitimately spells the literal.
#[test]
fn every_production_derivation_site_reads_the_capability() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(src_dir.is_dir(), "src dir not found: {src_dir:?}");

    let mut offending = Vec::new();
    let exclusions = external_test_module_files(&src_dir);
    for entry in walk_rs_files(&src_dir) {
        if exclusions.contains(&entry) {
            continue;
        }
        let contents = std::fs::read_to_string(&entry).expect("read source file");
        let production_prefix = match contents.find("#[cfg(test)]") {
            Some(idx) => &contents[..idx],
            None => contents.as_str(),
        };
        if production_prefix.contains("StagedRelationResidence::SessionTemporary")
            || production_prefix.contains("StagedRelation::session_temporary(")
        {
            offending.push(entry.display().to_string());
        }
    }
    assert!(
        offending.is_empty(),
        "these smelt-runtime source files hardcode a session-temporary staged relation shape \
         instead of deriving it from BackendCapabilities: {offending:?}"
    );
}

/// `docs/outcomes/20260913-trino-incremental` phase 5, test 5: every
/// derivation helper this outcome touched yields `TargetSchema`/non-atomic
/// under a Trino-shaped capability set and `SessionTemporary`/atomic under
/// DuckDB's — proving the threading actually reaches every site, not just
/// that no hardcoded literal remains.
#[test]
fn derivation_sites_yield_target_schema_for_trino_caps() {
    let duckdb = smelt_backend::BackendCapabilities::duckdb();
    let trino = smelt_backend::BackendCapabilities::trino_iceberg();

    for purpose in ["__smelt_staged_", "__smelt_repair_", "__smelt_diff_patch_"] {
        let relation_duckdb = StagedRelation::derive_for_capabilities(purpose, "t", &duckdb);
        assert_eq!(
            relation_duckdb.residence,
            StagedRelationResidence::SessionTemporary
        );
        assert!(relation_duckdb.atomic);

        let relation_trino = StagedRelation::derive_for_capabilities(purpose, "t", &trino);
        assert_eq!(
            relation_trino.residence,
            StagedRelationResidence::TargetSchema
        );
        assert!(!relation_trino.atomic);
    }

    // The production-facing wrappers over `derive_for_capabilities` yield
    // the same verdict for the two purposes they own.
    let repair_trino = smelt_runtime::maintenance_driver::repair_staged_relation("t", &trino);
    assert_eq!(
        repair_trino.residence,
        StagedRelationResidence::TargetSchema
    );
    assert!(!repair_trino.atomic);
    let repair_duckdb = smelt_runtime::maintenance_driver::repair_staged_relation("t", &duckdb);
    assert_eq!(
        repair_duckdb.residence,
        StagedRelationResidence::SessionTemporary
    );
    assert!(repair_duckdb.atomic);

    let diff_patch_trino =
        smelt_runtime::maintenance_driver::diff_patch_staged_relation("t", &trino);
    assert_eq!(
        diff_patch_trino.residence,
        StagedRelationResidence::TargetSchema
    );
    assert!(!diff_patch_trino.atomic);
}

/// Proves the `#[cfg(test)] mod <ident>;` exclusion is scoped to genuine
/// external test-module files, not a blanket exemption: a synthetic tree
/// with both an excluded test-module file and a genuine production file
/// hardcoding the literal must still flag the production file.
#[test]
fn census_still_flags_a_production_file() {
    let root = std::env::temp_dir().join(format!(
        "smelt_staged_relation_census_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create synthetic root");

    std::fs::write(
        root.join("mod.rs"),
        "#[cfg(test)]\nmod tests;\n\nfn production_fn() {}\n",
    )
    .expect("write mod.rs");
    std::fs::write(
        root.join("tests.rs"),
        "fn t() { let _ = StagedRelation::session_temporary(\"x\"); }\n",
    )
    .expect("write tests.rs");
    std::fs::write(
        root.join("other.rs"),
        "fn production_fn() { let _ = StagedRelation::session_temporary(\"y\"); }\n",
    )
    .expect("write other.rs");

    let exclusions = external_test_module_files(&root);
    assert!(exclusions.contains(&root.join("tests.rs")));

    let mut offending = Vec::new();
    for entry in walk_rs_files(&root) {
        if exclusions.contains(&entry) {
            continue;
        }
        let contents = std::fs::read_to_string(&entry).expect("read source file");
        let production_prefix = match contents.find("#[cfg(test)]") {
            Some(idx) => &contents[..idx],
            None => contents.as_str(),
        };
        if production_prefix.contains("StagedRelationResidence::SessionTemporary")
            || production_prefix.contains("StagedRelation::session_temporary(")
        {
            offending.push(entry);
        }
    }

    std::fs::remove_dir_all(&root).expect("clean up synthetic root");

    assert_eq!(
        offending,
        vec![root.join("other.rs")],
        "the excluded test-module file must not blind the census to a genuine production hit"
    );
}

fn walk_rs_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(walk_rs_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

/// A whole file declared as `#[cfg(test)] mod <ident>;` by a sibling module
/// is a test module living in its own file — the production-prefix scan
/// (which truncates a file at its own first `#[cfg(test)]`) cannot see that
/// declaration from *inside* the referenced file, so it must be excluded by
/// the census up front. Resolves `<dir>/<ident>.rs` and `<dir>/<ident>/mod.rs`,
/// matching Rust's own module resolution.
fn external_test_module_files(
    root: &std::path::Path,
) -> std::collections::HashSet<std::path::PathBuf> {
    let mut exclusions = std::collections::HashSet::new();
    for entry in walk_rs_files(root) {
        let contents = std::fs::read_to_string(&entry).expect("read source file");
        let dir = entry.parent().expect("file has parent dir");
        for mod_name in cfg_test_external_mod_names(&contents) {
            let sibling_file = dir.join(format!("{mod_name}.rs"));
            if sibling_file.is_file() {
                exclusions.insert(sibling_file);
            }
            let mod_dir_file = dir.join(&mod_name).join("mod.rs");
            if mod_dir_file.is_file() {
                exclusions.insert(mod_dir_file);
            }
        }
    }
    exclusions
}

/// Finds every `mod <ident>;` (no braces — an external-file module, not an
/// inline one) immediately gated by `#[cfg(test)]`, tolerant of the
/// attribute and the `mod` keyword sharing a line or not.
fn cfg_test_external_mod_names(contents: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut search_from = 0;
    while let Some(rel_idx) = contents[search_from..].find("#[cfg(test)]") {
        let attr_end = search_from + rel_idx + "#[cfg(test)]".len();
        let rest = contents[attr_end..].trim_start();
        if let Some(after_mod) = rest.strip_prefix("mod ") {
            let ident_end = after_mod.find(|c: char| !(c.is_alphanumeric() || c == '_'));
            if let Some(end) = ident_end {
                let ident = after_mod[..end].trim();
                // Only a bare `mod ident;` (external file) counts — `mod ident {`
                // is an inline module and already visible to the prefix scan.
                if after_mod[end..].trim_start().starts_with(';') && !ident.is_empty() {
                    names.push(ident.to_string());
                }
            }
        }
        search_from = attr_end;
    }
    names
}
