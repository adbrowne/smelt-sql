use smelt_core::baseline::{edited_set, materialize, resolve_baseline};
use smelt_core::sources::discover_source_infos;
use smelt_core::workspace::load_workspace;

use crate::fixtures::{fixture_repo, git, git_commit, lock};

#[test]
fn edited_set_flags_uncommitted_sql_edit() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/m.sql"),
        "SELECT customer_id, MAX(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("edit without committing");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(set.names.contains("m"), "names={:?}", set.names);
    assert!(
        set.files.iter().any(|f| f.ends_with("models/m.sql")),
        "files={:?}",
        set.files
    );
}

#[test]
fn edited_set_ignores_a_formatting_only_edit() {
    let _guard = lock();
    let repo = fixture_repo();

    // Give the model frontmatter containing a comment line, so there is
    // something to reflow without touching a parsed metadata key.
    std::fs::write(
        repo.path().join("models/m.sql"),
        "---\nunique_key: [customer_id]\n#  reflow comment\n---\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("write frontmatter");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add frontmatter comment");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    // A true formatting-only reflow: swap one interior double-space for a
    // space+tab inside the comment — same byte length, so
    // `strip_frontmatter`'s length-preserving blank-out produces
    // byte-identical stripped SQL, and the comment carries no parsed
    // metadata key, so `ModelMetadata` is also unchanged. This is not the
    // no-op "write the same bytes back" case Phase 4's original test used —
    // the file's bytes genuinely differ here.
    let original = std::fs::read_to_string(repo.path().join("models/m.sql")).expect("read");
    assert!(
        original.contains("#  reflow comment"),
        "fixture sanity: expected the double-space comment to still be present"
    );
    let reflowed = original.replacen("#  reflow comment", "# \treflow comment", 1);
    assert_eq!(
        original.len(),
        reflowed.len(),
        "the reflow must be byte-length-preserving for this test to be meaningful"
    );
    assert_ne!(original, reflowed, "the reflow must actually change bytes");
    std::fs::write(repo.path().join("models/m.sql"), &reflowed).expect("rewrite");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        !set.names.contains("m"),
        "an unchanged file must not be in the edited set: {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_a_frontmatter_only_edit() {
    let _guard = lock();
    let repo = fixture_repo();
    // Give the model frontmatter so we have something to edit in it.
    std::fs::write(
        repo.path().join("models/m.sql"),
        "-- unique_key: [customer_id]\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("write");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add frontmatter");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/m.sql"),
        "-- unique_key: [order_id]\nSELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id\n",
    )
    .expect("edit frontmatter only");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "edit frontmatter");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("m"),
        "a frontmatter-only edit must be in the edited set (Δ2): {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_a_smelt_yml_model_override() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("smelt.yml"),
        "name: fixture\nmodels:\n  m:\n    materialization: table\n",
    )
    .expect("edit smelt.yml");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(set.names.contains("m"), "names={:?}", set.names);
    assert!(!set.project_config_changed);
}

#[test]
fn edited_set_flags_a_project_level_config_change() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("smelt.yml"),
        "name: fixture\ndefault_materialization: table\n",
    )
    .expect("edit smelt.yml");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.is_empty(),
        "no model override touched, so no model should be edited: {:?}",
        set.names
    );
    assert!(set.project_config_changed);
}

#[test]
fn edited_set_flags_a_changed_source_declaration() {
    let _guard = lock();
    let repo = fixture_repo();
    std::fs::create_dir_all(repo.path().join("models/sources")).expect("mkdir");
    std::fs::write(
        repo.path().join("models/sources/orders.yml"),
        "columns:\n  - name: id\n    type: bigint\n",
    )
    .expect("write source");
    git(repo.path(), &["add", "-A"]);
    git_commit(repo.path(), "add source");

    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(
        repo.path().join("models/sources/orders.yml"),
        "columns:\n  - name: id\n    type: bigint\n  - name: amount\n    type: double\n",
    )
    .expect("edit source");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("orders"),
        "the bare dotted source name (leading `sources` stripped) must be edited: {:?}",
        set.names
    );
}

#[test]
fn edited_set_flags_one_sided_files() {
    let _guard = lock();
    let repo = fixture_repo();
    let resolved = resolve_baseline(repo.path(), Some("HEAD")).expect("resolve");
    let checkout = materialize(&resolved).expect("materialize");

    std::fs::write(repo.path().join("models/new_model.sql"), "SELECT 1\n")
        .expect("write new model");

    let base_loaded = load_workspace(checkout.project_root());
    let base_sources = discover_source_infos(checkout.project_root(), &base_loaded.config.paths);
    let work_loaded = load_workspace(repo.path());
    let work_sources = discover_source_infos(repo.path(), &work_loaded.config.paths);
    let set = edited_set(&base_loaded, &base_sources, &work_loaded, &work_sources);

    assert!(
        set.names.contains("new_model"),
        "a model present only in the working tree must be edited: {:?}",
        set.names
    );
}
