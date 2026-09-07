use smelt_core::baseline::{materialize, materialize_in, resolve_baseline, BaselineError};

use crate::fixtures::{fixture_repo, lock};

#[test]
fn checkout_scratch_is_deleted_on_drop() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");
    let project_root = checkout.project_root().to_path_buf();
    drop(checkout);
    assert!(!project_root.exists(), "scratch must be gone after drop");
}

#[test]
fn checkout_scratch_is_deleted_when_materialization_fails() {
    let _guard = lock();
    let repo = fixture_repo();
    let mut resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    // Corrupt the resolved commit so `git archive` fails after the scratch
    // directory has already been created.
    resolved.commit = "0000000000000000000000000000000000000000".to_string();

    // A private scratch parent, not `std::env::temp_dir()`: any concurrent
    // `materialize` call elsewhere in this binary (or another test binary
    // running in parallel) creates and drops its own `smelt-baseline-*`
    // entries in the shared system temp dir, so a hygiene assertion there
    // races everything else on the box.
    let scratch_parent = tempfile::tempdir().expect("scratch parent");

    let err = materialize_in(&resolved, scratch_parent.path())
        .expect_err("bogus commit must fail materialize");
    assert!(matches!(err, BaselineError::Archive { .. }), "{err:?}");

    let leftover: Vec<_> = std::fs::read_dir(scratch_parent.path())
        .expect("read scratch parent")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        leftover.is_empty(),
        "no scratch entry may survive a failed materialize, found: {leftover:?}"
    );
}

#[test]
fn checkout_scratch_is_deleted_on_drop_uses_the_given_parent() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let scratch_parent = tempfile::tempdir().expect("scratch parent");

    let checkout = materialize_in(&resolved, scratch_parent.path()).expect("materialize_in");
    assert!(
        checkout.project_root().starts_with(scratch_parent.path()),
        "materialize_in must put its scratch under the supplied parent"
    );
    drop(checkout);

    let leftover: Vec<_> = std::fs::read_dir(scratch_parent.path())
        .expect("read scratch parent")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        leftover.is_empty(),
        "scratch parent must be empty again after the checkout drops, found: {leftover:?}"
    );
}

#[test]
fn materialize_defaults_its_scratch_parent_to_the_system_temp_dir() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");

    let checkout = materialize(&resolved).expect("materialize");
    let canonical_temp_dir =
        std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
    let canonical_root = std::fs::canonicalize(checkout.project_root())
        .unwrap_or_else(|_| checkout.project_root().to_path_buf());
    assert!(
        canonical_root.starts_with(&canonical_temp_dir),
        "plain materialize must default its scratch parent to std::env::temp_dir(), got {canonical_root:?} not under {canonical_temp_dir:?}"
    );
}
