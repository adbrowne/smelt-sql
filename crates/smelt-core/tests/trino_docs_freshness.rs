//! Standing drift gate for docs-site's description of the `trino` target
//! (`docs/outcomes/20260913-trino-target-spine/outcome.md` phase 11). The
//! refused-key list, the literal-password rule, and the measured capability
//! profile are all enforced/established in `crates/smelt-core/src/config.rs`
//! and `docs/specs/multi_backend.md` — this test keeps `docs-site/` in sync
//! with those sources rather than restating them independently and letting
//! the two drift.

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

fn config_source() -> String {
    let path = repo_root().join("crates/smelt-core/src/config.rs");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// Parses the key names out of `TRINO_FOREIGN_KEYS` in `config.rs` by regex
/// over the source rather than restating the list — the const is private, so
/// this is the only way a test can see it without exporting implementation
/// detail across the crate boundary.
fn trino_foreign_keys() -> Vec<String> {
    let source = config_source();
    let start = source
        .find("const TRINO_FOREIGN_KEYS")
        .expect("crates/smelt-core/src/config.rs has no `TRINO_FOREIGN_KEYS` const");
    let end = source[start..]
        .find("];")
        .map(|i| start + i)
        .expect("TRINO_FOREIGN_KEYS const has no closing `];`");
    let body = &source[start..end];

    let mut keys = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('"') {
        let after_open = &rest[open + 1..];
        let close = after_open
            .find('"')
            .expect("unterminated string literal in TRINO_FOREIGN_KEYS");
        keys.push(after_open[..close].to_string());
        rest = &after_open[close + 1..];
    }

    assert!(
        !keys.is_empty(),
        "found TRINO_FOREIGN_KEYS but extracted no key names from it"
    );
    keys
}

fn targets_doc() -> String {
    let path = repo_root().join("docs-site/docs/guide/targets.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The `### Trino` section alone — scoped so a key name that also happens to
/// be a Spark/BigQuery/Databricks field (`catalog`, `schema`, `port`, `host`)
/// doesn't pass this test by appearing in an unrelated section.
fn trino_section(doc: &str) -> &str {
    let start = doc
        .find("### Trino")
        .expect("docs-site/docs/guide/targets.md has no `### Trino` section");
    let rest = &doc[start..];
    let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

/// A new foreign key added to `TRINO_FOREIGN_KEYS` without documenting it as
/// refused fails this test.
#[test]
fn docs_site_names_every_refused_trino_key() {
    let doc = targets_doc();
    let section = trino_section(&doc);
    let keys = trino_foreign_keys();

    let missing: Vec<&String> = keys
        .iter()
        .filter(|k| !section.contains(k.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs-site/docs/guide/targets.md's Trino section does not name every key \
         TRINO_FOREIGN_KEYS refuses: {missing:?}"
    );
}

/// `check_literal_secrets`' `("trino", "password")` entry in
/// `LITERAL_SECRET_KEYS` requires the password to be a `${VAR}` reference,
/// never a literal — the docs-site section must state the same rule and not
/// show a literal password in its example.
#[test]
fn docs_site_states_the_trino_password_env_only_rule() {
    let source = config_source();
    assert!(
        source.contains(r#"("trino", "password")"#),
        "crates/smelt-core/src/config.rs's LITERAL_SECRET_KEYS no longer names \
         (\"trino\", \"password\") — update this test's premise"
    );

    let doc = targets_doc();
    let section = trino_section(&doc);

    assert!(
        section.contains("${VAR}") || section.contains("${ENV}"),
        "docs-site/docs/guide/targets.md's Trino section does not state the `${{VAR}}`-only \
         rule for `password`"
    );
    assert!(
        section.contains("hard configuration error"),
        "docs-site/docs/guide/targets.md's Trino section does not state that a literal \
         password is a hard configuration error"
    );

    // No example in the section may show a literal (non-`${VAR}`) password.
    for line in section.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("password:") {
            let value = rest.trim();
            assert!(
                value.starts_with("${"),
                "docs-site/docs/guide/targets.md's Trino section shows a literal password: \
                 {line:?}"
            );
        }
    }
}

/// Extracts every capability flag name whose Trino column is `✗` in
/// `docs/specs/multi_backend.md`'s capability matrix, by parsing the table
/// rather than restating the flag list.
fn trino_measured_false_capabilities() -> Vec<String> {
    let path = repo_root().join("docs/specs/multi_backend.md");
    let spec = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));

    let header_start = spec
        .find("| Flag | DuckDB |")
        .expect("docs/specs/multi_backend.md has no capability matrix table header");
    let header_line_end = spec[header_start..]
        .find('\n')
        .map(|i| header_start + i)
        .expect("capability matrix header has no line end");
    let header = &spec[header_start..header_line_end];
    let header_cols: Vec<&str> = header.split('|').map(str::trim).collect();
    let trino_idx = header_cols
        .iter()
        .position(|c| c.contains("Trino"))
        .expect("capability matrix header has no Trino column");

    let mut flags = Vec::new();
    for line in spec[header_line_end + 1..].lines() {
        if !line.trim_start().starts_with('|') {
            break;
        }
        if line.contains("---") {
            continue;
        }
        // Column count in a row can exceed the header's due to escaped `\|`
        // sequences inside the Flag cell's parenthetical description (e.g.
        // `` `supports_concat_operator` (`\|\|`) ``); those escapes never
        // appear past the Flag cell, so indexing from the *end* of the row
        // reliably locates the last (Trino) column regardless of how many
        // extra splits the escapes introduced near the front.
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        if cols.len() < header_cols.len() {
            continue;
        }
        let from_end = header_cols.len() - trino_idx;
        let trino_cell = cols[cols.len() - from_end];
        if !trino_cell.contains('✗') {
            continue;
        }
        let flag_cell = cols[1];
        let flag_start = flag_cell
            .find('`')
            .unwrap_or_else(|| panic!("capability matrix row has no backtick-quoted flag: {line}"));
        let after = &flag_cell[flag_start + 1..];
        let flag_end = after
            .find('`')
            .unwrap_or_else(|| panic!("unterminated backtick in capability matrix row: {line}"));
        flags.push(after[..flag_end].to_string());
    }

    assert!(
        !flags.is_empty(),
        "found the capability matrix table but extracted no `✗` Trino flags from it"
    );
    flags
}

/// Every capability whose cell is `✗` in the spec's Trino column must be
/// named in the docs-site section's limitations list — closes the chain from
/// the spec table (already gated against the constructor by
/// `capability_conformance`) to the user docs without a second restatement.
#[test]
fn docs_site_names_every_measured_false_capability() {
    let doc = targets_doc();
    let section = trino_section(&doc);
    let flags = trino_measured_false_capabilities();

    let missing: Vec<&String> = flags
        .iter()
        .filter(|f| !section.contains(f.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs-site/docs/guide/targets.md's Trino section does not name every measured-`✗` \
         capability from docs/specs/multi_backend.md's Trino column: {missing:?}"
    );
}

/// Every `scripts/trino-*` path the section tells a user to run must exist,
/// and the default port stated must match `scripts/trino-env.sh`'s own
/// default.
#[test]
fn docs_site_trino_scripts_exist() {
    let doc = targets_doc();
    let section = trino_section(&doc);
    let root = repo_root();

    for script in [
        "scripts/trino-up.sh",
        "scripts/trino-down.sh",
        "scripts/trino-env.sh",
    ] {
        assert!(
            section.contains(script),
            "docs-site/docs/guide/targets.md's Trino section does not mention `{script}`"
        );
        assert!(
            root.join(script).exists(),
            "docs-site/docs/guide/targets.md's Trino section names `{script}`, which does not \
             exist"
        );
    }

    assert!(
        section.contains("scripts/README-trino.md"),
        "docs-site/docs/guide/targets.md's Trino section does not point to \
         scripts/README-trino.md"
    );
    assert!(
        root.join("scripts/README-trino.md").exists(),
        "docs-site/docs/guide/targets.md's Trino section points to scripts/README-trino.md, \
         which does not exist"
    );

    let env_script = root.join("scripts/trino-env.sh");
    let env_source =
        fs::read_to_string(&env_script).unwrap_or_else(|e| panic!("read {env_script:?}: {e}"));
    let default_port_marker = "SMELT_TRINO_PORT:-";
    let marker_start = env_source
        .find(default_port_marker)
        .expect("scripts/trino-env.sh has no `SMELT_TRINO_PORT:-` default");
    let after = &env_source[marker_start + default_port_marker.len()..];
    let end = after
        .find('}')
        .expect("scripts/trino-env.sh's SMELT_TRINO_PORT default has no closing `}`");
    let default_port = &after[..end];

    assert!(
        section.contains(default_port),
        "docs-site/docs/guide/targets.md's Trino section does not state the default port \
         `{default_port}` from scripts/trino-env.sh"
    );
}

/// `docs/outcomes/20260913-trino-ledger/outcome.md` phase 1: the state-residency
/// posture is measured against a live coordinator, not read from documentation.
/// `scripts/trino-probe-state.sh` is the measurement script; it must exist and
/// be executable, alongside the tier's other `scripts/trino-*` entry points.
#[test]
fn probe_state_script_exists_and_is_executable() {
    let path = repo_root().join("scripts/trino-probe-state.sh");
    assert!(
        path.exists(),
        "scripts/trino-probe-state.sh does not exist — the measured-not-read posture probe \
         for docs/outcomes/20260913-trino-ledger is missing"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("stat {path:?}: {e}"))
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "scripts/trino-probe-state.sh is not executable (mode {mode:o})"
        );
    }
}

/// `scripts/README-trino.md` must name the state-residency probe so the
/// measured-not-read discipline is discoverable from the tier's own docs,
/// not just from the outcome directory.
#[test]
fn readme_documents_the_state_probe() {
    let path = repo_root().join("scripts/README-trino.md");
    let readme = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert!(
        readme.contains("trino-probe-state.sh"),
        "scripts/README-trino.md does not mention scripts/trino-probe-state.sh"
    );
}
