//! Tests for the Databricks dogfood provisioning + credential tooling
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/04a-plan.md`).
//!
//! All offline, no workspace, no gpg: these scripts are gated entirely on
//! their own shape (existence, dispatch, redaction, settings-split) before a
//! credential exists. `scripts/dbx-provision.sh` and `scripts/dbx-verify.sh`
//! are NOT run for real here — they need phase 4b's human and a live
//! workspace.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

const SECRET_SCRIPTS: &[&str] = &["dbx-key.sh", "dbx-provision.sh"];
// dbx-auth.sh mints a fresh token but never prints it — only an expiry
// stamp (docs/outcomes/20260912-databricks-dogfood-spine/outcome.md
// "## Blocked" item (b), RESOLVED 2026-09-12) — so it is allow-listed like
// the other read-only wrappers rather than denied like the scripts above
// that touch the encrypted secret or grant/rotate its scope.
const READ_ONLY_SCRIPTS: &[&str] = &["dbx-verify.sh", "dbx-query.sh", "dbx-auth.sh"];

#[test]
fn every_dbx_script_exists_and_is_shellcheck_clean_shape() {
    let all: Vec<&str> = SECRET_SCRIPTS
        .iter()
        .chain(READ_ONLY_SCRIPTS.iter())
        .copied()
        .collect();
    for name in &all {
        let contents = read(&format!("scripts/{name}"));
        assert!(
            contents.starts_with("#!/usr/bin/env bash"),
            "{name} must start with a bash shebang"
        );
        assert!(
            contents.contains("set -euo pipefail") || contents.contains("set -uo pipefail"),
            "{name} must set strict shell options"
        );
    }

    for name in ["dbx-key.sh", "dbx-auth.sh"] {
        let script = repo_root().join("scripts").join(name);
        let out = Command::new("bash")
            .arg(&script)
            .arg("--self-test")
            .current_dir(repo_root())
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn {name}: {e}"));
        assert!(
            out.status.success(),
            "{name} --self-test failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn secret_scripts_are_denied_and_read_only_wrappers_allowed() {
    let settings: serde_json::Value =
        serde_json::from_str(&read(".claude/settings.json")).expect("valid settings.json");
    let deny: Vec<String> = settings["permissions"]["deny"]
        .as_array()
        .expect("permissions.deny array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let allow: Vec<String> = settings["permissions"]["allow"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();

    for name in SECRET_SCRIPTS {
        assert!(
            deny.iter().any(|e| e.contains(name)),
            "missing deny entry for scripts/{name} in .claude/settings.json"
        );
    }
    for name in READ_ONLY_SCRIPTS {
        assert!(
            allow.iter().any(|e| e.contains(name)) && !deny.iter().any(|e| e.contains(name)),
            "missing non-denied allow entry for scripts/{name} in .claude/settings.json"
        );
    }
    assert!(
        deny.iter()
            .any(|e| e.contains("databricks-smelt-dogfood") && e.starts_with("Read(")),
        "missing Read(...) deny entry for the dbx config dir"
    );
}

#[test]
fn no_dbx_script_echoes_the_token() {
    let sensitive = ["SMELT_DBX_TOKEN", "DATABRICKS_TOKEN", "CLIENT_SECRET"];
    for name in SECRET_SCRIPTS
        .iter()
        .chain(READ_ONLY_SCRIPTS.iter())
        .chain(["dbx-dogfood-loader.sh"].iter())
    {
        let contents = read(&format!("scripts/{name}"));
        for line in contents.lines() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("echo") || trimmed.starts_with("printf")) {
                continue;
            }
            for var in sensitive {
                let unmasked = format!("${var}");
                let unmasked_braced = format!("${{{var}}}");
                let masked = format!("${{{var}:+SET}}");
                if (line.contains(&unmasked) || line.contains(&unmasked_braced))
                    && !line.contains(&masked)
                {
                    panic!("{name} appears to echo {var} unmasked: {line}");
                }
            }
        }
    }
}

#[test]
fn auth_script_supports_both_credential_kinds() {
    let contents = read("scripts/dbx-auth.sh");
    assert!(contents.contains("oauth-m2m"), "must dispatch on oauth-m2m");
    assert!(
        contents.contains("oauth-m2m|pat)"),
        "must dispatch on both oauth-m2m and pat"
    );
    assert!(
        contents.contains("unknown credential kind"),
        "must error with a named message on an unrecognised credential kind"
    );
}

#[test]
fn provision_wizard_emits_both_schemas_and_scoped_grants() {
    let contents = read("scripts/dbx-provision.sh");
    assert!(contents.contains("smelt_dogfood"));
    assert!(contents.contains("smelt_dogfood_oracle"));
    assert!(
        contents.contains("GRANT"),
        "wizard must issue at least one GRANT"
    );
    assert!(
        !contents.contains("CATALOG workspace"),
        "must never grant at the catalog level"
    );
    assert!(
        !contents.contains("ALL PRIVILEGES"),
        "must never grant ALL PRIVILEGES"
    );
}

#[test]
fn verify_script_checks_reachability_and_refusal() {
    let contents = read("scripts/dbx-verify.sh");
    assert!(
        contents.contains("SELECT 1"),
        "must run a reachability SELECT"
    );
    assert!(
        contents.to_uppercase().contains("SHOW TABLES"),
        "must run a reachability SHOW TABLES"
    );
    // The refusal probe is CREATE SCHEMA on the catalog, not CREATE TABLE:
    // Free Edition grants every workspace user/principal CREATE TABLE (and
    // CREATE VOLUME/MODEL/MATERIALIZED VIEW/FUNCTION) on `workspace.default`
    // as a platform default unrelated to this credential's own grants, so a
    // CREATE TABLE probe would pass regardless of scope
    // (docs/outcomes/20260912-databricks-dogfood-spine/phases/04c-summary.md).
    // CREATE SCHEMA on the catalog is the credential's own scope instead.
    assert!(
        contents.contains("CREATE SCHEMA"),
        "must attempt a write outside the granted schemas"
    );
    // Inverted exit-status handling: the write succeeding must be the
    // failure branch, not the success branch.
    assert!(
        contents.contains("if query \"CREATE SCHEMA")
            && contents.contains("UNEXPECTED")
            && contents.contains("correctly refused"),
        "must invert exit-status handling for the refusal leg, not just run the SQL"
    );
}

#[test]
fn env_script_exports_the_oracle_schema() {
    let env_script = repo_root().join("scripts/dbx-dogfood-env.sh");
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "set -e; source {}; echo \"ORACLE=$SMELT_DBX_ORACLE_SCHEMA\"; echo \"HOST=${{SMELT_DBX_HOST-UNSET}}\"",
            env_script.display()
        ))
        .current_dir(repo_root())
        .env("SMELT_DBX_CONFIG_DIR", tmp.path())
        .env_remove("SMELT_DBX_HOST")
        .env_remove("SMELT_DBX_TOKEN")
        .output()
        .unwrap_or_else(|e| panic!("failed to source dbx-dogfood-env.sh: {e}"));
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ORACLE=smelt_dogfood_oracle"),
        "expected the oracle schema exported:\n{stdout}"
    );
    assert!(
        stdout.contains("HOST=UNSET"),
        "with an empty config dir, SMELT_DBX_HOST must stay unset:\n{stdout}"
    );
}

/// `SMELT_DBX_HOSTNAME` (the bare hostname the `type: databricks` target's
/// `host:` key requires) is derived from `SMELT_DBX_HOST` by stripping its
/// scheme and any trailing slash, while `SMELT_DBX_HOST` itself is left
/// untouched — `dbx-auth.sh` and the Python query wrapper still need the
/// scheme-bearing URL (`docs/outcomes/20260912-databricks-dogfood-spine/
/// outcome.md` phase 6b: the wizard writes a scheme-bearing host, but the
/// target's `host:` contract is bare-hostname-only).
#[test]
fn env_script_exports_bare_hostname() {
    let env_script = repo_root().join("scripts/dbx-dogfood-env.sh");
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("config.env"),
        "SMELT_DBX_HOST=https://dbc-test.cloud.databricks.com/\n",
    )
    .expect("write config.env");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "set -e; source {}; echo \"HOST=$SMELT_DBX_HOST\"; echo \"HOSTNAME=$SMELT_DBX_HOSTNAME\"",
            env_script.display()
        ))
        .current_dir(repo_root())
        .env("SMELT_DBX_CONFIG_DIR", tmp.path())
        .env_remove("SMELT_DBX_HOST")
        .env_remove("SMELT_DBX_TOKEN")
        .output()
        .unwrap_or_else(|e| panic!("failed to source dbx-dogfood-env.sh: {e}"));
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("HOST=https://dbc-test.cloud.databricks.com/"),
        "SMELT_DBX_HOST must stay scheme-bearing for URL-building consumers:\n{stdout}"
    );
    assert!(
        stdout.contains("HOSTNAME=dbc-test.cloud.databricks.com"),
        "SMELT_DBX_HOSTNAME must be the bare hostname, no scheme, no trailing slash:\n{stdout}"
    );
}

#[test]
fn facts_sheet_has_no_unfilled_quota_slots() {
    // Phase 4a shipped the sheet with every row TBD; phase 4c (a reachable
    // session, no human needed for the demonstrable half of criterion 4)
    // filled every row with a measured or cited value
    // (docs/outcomes/20260912-databricks-dogfood-spine/phases/04c-summary.md).
    // This test now guards the opposite direction: no TBD may survive.
    let contents = read("docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md");
    assert!(
        !contents.contains("TBD"),
        "expected every quota row and the credential kind to be filled, found a TBD in:\n{contents}"
    );
}
