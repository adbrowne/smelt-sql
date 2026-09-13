//! Standing gate for the Trino Docker tier
//! (`docs/outcomes/20260913-trino-target-spine/phases/04-plan.md`): every
//! image is pinned, the Iceberg REST catalog is wired to MinIO by service
//! name, every pin is documented, and the tier does not collide with any
//! other backend's Docker tier. Runs with no Docker — pure file assertions.

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

fn read_scripts_file(name: &str) -> String {
    fs::read_to_string(repo_root().join("scripts").join(name))
        .unwrap_or_else(|e| panic!("failed to read scripts/{name}: {e}"))
}

/// Every `image:` value in the compose file carries an explicit, non-`latest`
/// tag. A bare name or `:latest` would let a re-pull silently change what CI
/// runs against.
#[test]
fn compose_file_exists_and_pins_every_image() {
    let text = read_scripts_file("trino-compose.yml");

    let mut found_image = false;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("image:") else {
            continue;
        };
        found_image = true;
        let image = rest.trim().trim_matches('"');
        let tag = image
            .rsplit_once(':')
            .map(|(_, tag)| tag)
            .unwrap_or_else(|| panic!("image `{image}` in trino-compose.yml has no tag at all"));
        assert_ne!(
            tag, "latest",
            "image `{image}` in trino-compose.yml pins to `latest`, not a fixed tag"
        );
        assert!(
            !tag.is_empty(),
            "image `{image}` in trino-compose.yml has an empty tag"
        );
    }
    assert!(
        found_image,
        "trino-compose.yml has no `image:` lines at all"
    );
}

/// The Iceberg REST catalog properties file exists and points at the REST
/// catalog and MinIO by compose service name, never `localhost` — Trino
/// reaches both over the compose network.
#[test]
fn iceberg_catalog_properties_are_committed() {
    let text = read_scripts_file("trino-catalog/iceberg.properties");

    assert!(
        text.contains("connector.name=iceberg"),
        "trino-catalog/iceberg.properties does not set connector.name=iceberg"
    );
    assert!(
        text.contains("iceberg.catalog.type=rest"),
        "trino-catalog/iceberg.properties does not set iceberg.catalog.type=rest"
    );
    assert!(
        !text.contains("localhost"),
        "trino-catalog/iceberg.properties references `localhost`; endpoints must be compose \
         service names so Trino can reach them over the compose network"
    );

    let compose = read_scripts_file("trino-compose.yml");
    // The REST catalog and MinIO service names used in iceberg.properties must
    // actually be services defined in the compose file.
    for service in ["iceberg-rest", "minio"] {
        assert!(
            compose.contains(&format!("{service}:")),
            "trino-compose.yml has no `{service}:` service, but iceberg.properties expects it"
        );
        assert!(
            text.contains(service),
            "trino-catalog/iceberg.properties does not reference the `{service}` service"
        );
    }
}

/// Every image tag appearing in the compose file also appears in
/// README-trino.md, so a bumped pin cannot land undocumented.
#[test]
fn readme_records_every_pin() {
    let compose = read_scripts_file("trino-compose.yml");
    let readme = read_scripts_file("README-trino.md");

    let mut checked_any = false;
    for line in compose.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("image:") else {
            continue;
        };
        let image = rest.trim().trim_matches('"');
        let tag = image
            .rsplit_once(':')
            .map(|(_, tag)| tag)
            .unwrap_or_else(|| panic!("image `{image}` has no tag"));
        checked_any = true;
        assert!(
            readme.contains(tag),
            "README-trino.md does not mention pinned tag `{tag}` (from image `{image}`)"
        );
    }
    assert!(checked_any, "no images found in trino-compose.yml to check");
}

/// The compose file's published host ports and container names are disjoint
/// from scripts/spark-up.sh's (`15002`, `smelt-spark`) and from each other.
#[test]
fn tier_binds_no_port_or_container_name_another_tier_uses() {
    let compose = read_scripts_file("trino-compose.yml");

    assert!(
        !compose.contains("15002"),
        "trino-compose.yml publishes port 15002, colliding with scripts/spark-up.sh"
    );
    assert!(
        !compose.contains("\"smelt-spark\"") && !compose.contains(": smelt-spark\n"),
        "trino-compose.yml uses the `smelt-spark` container name"
    );

    let mut container_names = Vec::new();
    for line in compose.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("container_name:") {
            container_names.push(rest.trim().to_string());
        }
    }
    assert!(
        !container_names.is_empty(),
        "trino-compose.yml declares no container_name at all"
    );
    let mut seen = std::collections::HashSet::new();
    for name in &container_names {
        assert!(
            seen.insert(name.clone()),
            "duplicate container_name `{name}` in trino-compose.yml"
        );
        assert!(
            name.starts_with("smelt-trino"),
            "container_name `{name}` does not carry the `smelt-trino` prefix"
        );
    }

    let mut host_ports = Vec::new();
    for line in compose.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- \"") {
            if let Some((host_part, _)) = rest.split_once(':') {
                let host_port = host_part.trim_start_matches('"');
                // Strip a `${VAR:-default}` shell interpolation down to the default.
                let port = host_port
                    .rsplit(":-")
                    .next()
                    .unwrap_or(host_port)
                    .trim_end_matches('}');
                if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() {
                    host_ports.push(port.to_string());
                }
            }
        }
    }
    assert!(
        !host_ports.contains(&"15002".to_string()),
        "trino-compose.yml publishes host port 15002, colliding with Spark: {host_ports:?}"
    );
    let mut seen_ports = std::collections::HashSet::new();
    for port in &host_ports {
        assert!(
            seen_ports.insert(port.clone()),
            "duplicate host port `{port}` published in trino-compose.yml"
        );
    }
}

/// `trino-up.sh` / `trino-down.sh` exist, are executable, and have the bash
/// shebang; `trino-env.sh` is sourced (no shebang, per `spark-env.sh`'s
/// convention) and exports `SMELT_TRINO_URL`.
#[test]
fn scripts_exist_and_are_executable() {
    use std::os::unix::fs::PermissionsExt;

    for script in ["trino-up.sh", "trino-down.sh"] {
        let path = repo_root().join("scripts").join(script);
        let meta = fs::metadata(&path).unwrap_or_else(|e| panic!("scripts/{script}: {e}"));
        assert!(
            meta.permissions().mode() & 0o111 != 0,
            "scripts/{script} is not executable"
        );
        let text = read_scripts_file(script);
        assert!(
            text.starts_with("#!/usr/bin/env bash"),
            "scripts/{script} does not start with the bash shebang"
        );
    }

    let env_text = read_scripts_file("trino-env.sh");
    assert!(
        !env_text.starts_with("#!"),
        "scripts/trino-env.sh must be sourced, not executed — it must not carry a shebang"
    );
    assert!(
        env_text.contains("export SMELT_TRINO_URL"),
        "scripts/trino-env.sh does not export SMELT_TRINO_URL"
    );
}
