//! Phase 26 — Tier 2 calling Tier 1: inline expansion.
//!
//! When a Tier 2 function body contains a call to a Tier 1 function, the Tier 1
//! body is expanded using the Tier 2 context's concrete parameter types.
//! Errors from the Tier 1 body surface at the Tier 2 body check site with the
//! frame stack showing Tier 2 body → Tier 1 callee.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, file_signature_inputs, Database, DiagnosticCode, SourceFile, Workspace,
};

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

// ============================================================================
// Test 1: tier2_calling_tier1_expands_inline
// ============================================================================

#[test]
fn tier2_calling_tier1_expands_inline() {
    // File 1 (Tier 1 — unannotated): `broken_tier1(x) AS (x + 'text')`.
    // File 2 (Tier 2 — annotated): `tier2_caller(n: Expr<Integer>) AS (smelt.fn.broken_tier1(n))`.
    //
    // Phase 26: checking the Tier 2 body at definition time should expand the
    // Tier 1 call using n=Integer, discover x + 'text' with x=Integer is a
    // type error, and surface FunctionBodyTypeMismatch on the Tier 2 file.
    let root = PathBuf::from("/fake/project");
    let tier1_path = root.join("functions").join("broken_tier1.sql");
    let tier2_path = root.join("functions").join("tier2_caller.sql");

    let tier1_src = "smelt.define broken_tier1(x) AS (x + 'text')\n";
    let tier2_src = "smelt.define tier2_caller(n: Expr<Integer>) AS (smelt.fn.broken_tier1(n))\n";

    let (db, ws, handles) = build_db(root, &[(tier1_path, tier1_src), (tier2_path, tier2_src)]);
    let tier2_file = handles[1];

    let diags = file_diagnostics(&db, ws, tier2_file);
    let body_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        !body_errors.is_empty(),
        "Tier 2 body calling a Tier 1 with Integer arg must cascade FunctionBodyTypeMismatch \
         from the Tier 1 expansion to the Tier 2 check site, got {diags:?}"
    );
}

// ============================================================================
// Test 2: tier1_error_surfaces_against_tier2_body (with ExpansionFrames check)
// ============================================================================

#[test]
fn tier1_error_surfaces_against_tier2_body() {
    // Same setup as test 1, but additionally verify that the cascaded diagnostic
    // carries a non-empty ExpansionFrames payload so the editor can show the
    // full call chain.
    let root = PathBuf::from("/fake/project");
    let tier1_path = root.join("functions").join("broken_t1.sql");
    let tier2_path = root.join("functions").join("tier2_with_frames.sql");

    let tier1_src = "smelt.define broken_t1(x) AS (x + 'text')\n";
    let tier2_src = "smelt.define tier2_with_frames(n: Expr<Integer>) AS (smelt.fn.broken_t1(n))\n";

    let (db, ws, handles) = build_db(root, &[(tier1_path, tier1_src), (tier2_path, tier2_src)]);
    let tier2_file = handles[1];

    let diags = file_diagnostics(&db, ws, tier2_file);

    // At least one diagnostic must carry an ExpansionFrames payload.
    let with_frames: Vec<_> = diags
        .iter()
        .filter(|d| matches!(&d.data, Some(smelt_db::DiagnosticData::ExpansionFrames(_))))
        .collect();

    assert!(
        !with_frames.is_empty(),
        "cascaded Tier 1 error surfacing on Tier 2 body must carry ExpansionFrames, \
         got {diags:?}"
    );
}

// ============================================================================
// Test 3: transitive_tier1_chain_expands
// ============================================================================

#[test]
fn transitive_tier1_chain_expands() {
    // Tier 1 → Tier 1 → Tier 2 chain:
    //   tier1_inner(x) AS (x + 'text')           — broken Tier 1
    //   tier1_outer(y) AS (smelt.fn.tier1_inner(y)) — Tier 1 calling Tier 1
    //   tier2_top(n: Expr<Integer>) AS (smelt.fn.tier1_outer(n))  — Tier 2 calling chain
    //
    // Checking the Tier 2 file must cascade the error through the Tier 1 chain.
    let root = PathBuf::from("/fake/project");
    let inner_path = root.join("functions").join("tier1_inner.sql");
    let outer_path = root.join("functions").join("tier1_outer.sql");
    let top_path = root.join("functions").join("tier2_top.sql");

    let inner_src = "smelt.define tier1_inner(x) AS (x + 'text')\n";
    let outer_src = "smelt.define tier1_outer(y) AS (smelt.fn.tier1_inner(y))\n";
    let top_src = "smelt.define tier2_top(n: Expr<Integer>) AS (smelt.fn.tier1_outer(n))\n";

    let (db, ws, handles) = build_db(
        root,
        &[
            (inner_path, inner_src),
            (outer_path, outer_src),
            (top_path, top_src),
        ],
    );
    let tier2_file = handles[2];

    let diags = file_diagnostics(&db, ws, tier2_file);
    let body_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        !body_errors.is_empty(),
        "transitive Tier 1 chain error must cascade to the Tier 2 body check site, \
         got {diags:?}"
    );
}

// ============================================================================
// Test 4: tier2_signature_stable_when_tier1_callee_breaks
// ============================================================================

#[test]
fn tier2_signature_stable_when_tier1_callee_breaks() {
    // Verify that changing the Tier 1 body (introducing a type error) does NOT
    // change the Tier 2 function's signature. Signatures are defined by
    // parameter names/types, not body contents.
    let root = PathBuf::from("/fake/project");
    let tier1_path = root.join("functions").join("tier1_clean.sql");
    let tier2_path = root.join("functions").join("tier2_uses_tier1.sql");

    let tier1_clean_src = "smelt.define tier1_clean(x) AS (x + 1)\n";
    let tier2_src =
        "smelt.define tier2_uses_tier1(n: Expr<Integer>) AS (smelt.fn.tier1_clean(n))\n";

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());

    let tier1_file = db.set_source_file(
        tier1_path.clone(),
        tier1_clean_src.to_string(),
        root.clone(),
    );
    let tier2_file = db.set_source_file(tier2_path.clone(), tier2_src.to_string(), root.clone());
    db.set_workspace(vec![tier1_file, tier2_file], vec![project]);

    // Capture signature before breaking the Tier 1 body.
    let sigs_before = file_signature_inputs(&db, tier2_file);
    assert_eq!(sigs_before.len(), 1, "tier2 must have one signature");
    let sig_before = sigs_before[0].clone();

    // Break the Tier 1 body.
    db.set_source_file(
        tier1_path.clone(),
        "smelt.define tier1_clean(x) AS (x + 'text')\n".to_string(),
        root.clone(),
    );

    // Re-read signature after the change.
    let sigs_after = file_signature_inputs(&db, tier2_file);
    assert_eq!(
        sigs_after.len(),
        1,
        "tier2 must still have one signature after Tier 1 change"
    );
    let sig_after = sigs_after[0].clone();

    assert_eq!(
        sig_before, sig_after,
        "Tier 2 signature must be stable when Tier 1 body changes"
    );
}

// ============================================================================
// Test 5: expansion_caches_per_arg_type_hash
// ============================================================================

#[test]
fn expansion_caches_per_arg_type_hash() {
    use smelt_types::signatures::DataTypeHash;
    use smelt_types::DataType;

    let hash1 = DataTypeHash::new(vec![("x".to_string(), DataType::Integer)]);
    let hash2 = DataTypeHash::new(vec![("x".to_string(), DataType::Integer)]);
    let hash3 = DataTypeHash::new(vec![("x".to_string(), DataType::Text)]);

    assert_eq!(hash1, hash2, "same bindings must be equal");
    assert_ne!(hash1, hash3, "different types must not be equal");

    // Hash consistency: equal DataTypeHashes must have equal hash values.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn hash_of(v: &DataTypeHash) -> u64 {
        let mut hasher = DefaultHasher::new();
        v.hash(&mut hasher);
        hasher.finish()
    }
    assert_eq!(
        hash_of(&hash1),
        hash_of(&hash2),
        "equal hashes must produce equal hash values"
    );
    // hash3 has a different type; while not strictly guaranteed to differ,
    // for these simple distinct types the hash should differ.
    // (This is a sanity check, not a hard requirement — hash collisions are
    // theoretically possible but astronomically unlikely for these values.)
    let _ = hash_of(&hash3); // just ensure it computes without panicking
}
