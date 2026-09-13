# Phase 4 plan — the Docker tier

## Objective

Stand up a committed, pinned `docker compose` tier — Trino coordinator + Iceberg REST catalog +
MinIO — behind `scripts/trino-{up,down,env}.sh`, with the Iceberg catalog properties committed
rather than typed by hand, and `scripts/README-trino.md` recording every version pin and why.
This is success criterion 3 in full, and it is the precondition for criteria 4–8: every later
phase's legs execute SQL against this coordinator.

## Spec delta

None. This phase adds no user-visible feature surface — the tier is contributor/CI
infrastructure. `docs/specs/multi_backend.md` §Surface already specifies `SMELT_TRINO_URL` and
its skip-when-unset rule (line ~121); this phase makes `trino-env.sh` export exactly that name.
Do not re-specify it.

## Tests

Red-green, in a new `crates/smelt-cli/tests/trino_tier_pins.rs` (runs with no Docker, so it is a
standing gate not a live leg):

1. `compose_file_exists_and_pins_every_image` — `scripts/trino-compose.yml` exists and every
   `image:` value carries an explicit non-`latest` tag; a bare name or `:latest` fails.
2. `iceberg_catalog_properties_are_committed` — `scripts/trino-catalog/iceberg.properties`
   exists, sets `connector.name=iceberg`, `iceberg.catalog.type=rest`, and points at the REST
   catalog and MinIO endpoints by service name (not `localhost`).
3. `readme_records_every_pin` — every image tag appearing in the compose file also appears in
   `scripts/README-trino.md`, so a bumped pin cannot land undocumented.
4. `tier_binds_no_port_or_container_name_another_tier_uses` — the compose file's published host
   ports and container names are disjoint from `scripts/spark-up.sh`'s (`15002`, `smelt-spark`)
   and from each other; criterion 9's "existing tiers are untouched".
5. `scripts_exist_and_are_executable` — `trino-up.sh` / `trino-down.sh` exist with the executable
   bit and a `#!/usr/bin/env bash` shebang; `trino-env.sh` has **no** shebang (sourced, the
   `spark-env.sh` convention) and exports `SMELT_TRINO_URL`.

Live legs (manual in this phase, run by the implementer, not cargo tests):

6. `trino-up.sh` reaches ready, and a `POST /v1/statement` smoke creating `iceberg.smelt_smoke`,
   writing a row and reading it back succeeds — proving the connector writes through to MinIO,
   not just that a process is listening.
7. `trino-up.sh` run a **second** time immediately after the first, without a `down`, reaches
   ready again — the idempotency-over-leftovers requirement.

## Tasks

1. Write `crates/smelt-cli/tests/trino_tier_pins.rs` with tests 1–5; watch all five fail.
2. Write `scripts/trino-compose.yml`: three services under an explicit `name: smelt-trino`
   project — `trinodb/trino:<pin>`, an Iceberg REST catalog fixture image, `minio/minio:<pin>`
   plus a one-shot `mc` bucket-create init container. Publish host ports from a
   `SMELT_TRINO_PORT`-style offset range (default coordinator `18080`) so nothing collides.
3. Use **named volumes, not host bind mounts**, for all writable state; bind-mount
   `scripts/trino-catalog/` read-only into the coordinator's `/etc/trino/catalog`. This is the
   structural answer to criterion 3's leftover hazard: there is no host-owned path for a `chmod`
   to abort on.
4. Write `scripts/trino-catalog/iceberg.properties` (REST catalog type, warehouse on the MinIO
   `s3://` bucket, path-style access, the tier's fixed local dev credentials).
5. Write `scripts/trino-up.sh`: `docker compose -f … down -v --remove-orphans` first (idempotent,
   never `set -e`-aborting on absent state), then `up -d`, then poll `GET /v1/info` until
   `"starting":false` with a bounded timeout that on failure dumps `docker compose logs --tail`
   for all three services and exits non-zero. Never exit 0 on a tier that did not come up.
6. Write `scripts/trino-down.sh` (`down -v --remove-orphans`, tolerant of nothing running) and
   `scripts/trino-env.sh` (no shebang, `# shellcheck shell=bash`, exports `SMELT_TRINO_URL`
   plus the catalog/schema/user the backend will read, echoes them).
7. Write `scripts/README-trino.md`: the version table (image → tag → why that pin), why Iceberg
   and not Hive (the write surface argument), the up/down/env recipe, the host-port map, and the
   skip-when-`SMELT_TRINO_URL`-is-unset rule.
8. Run the live legs (tests 6 and 7). Record the resolved image tags actually pulled — do not
   ship a guessed tag. If any image cannot be pulled or the tier will not come up, emit
   `<<PHASE_BLOCKED>>` with the failure; never leave a half-pinned compose file green.
9. Make tests 1–5 pass against the real files; add a decision-log entry in `outcome.md` naming
   the chosen pins, the host-port range, and the named-volume ruling.

## Verification

- `bash .claude/scripts/verify-phase.sh` — green.
- `mise run shellcheck` — zero findings on the three new scripts (the gate has no ratchet).
- `cargo test -p smelt-cli --test trino_tier_pins` — all five pass.
- `cargo test -p smelt-cli --test trino_spec_freshness` — unchanged, still green.
- Live: `bash scripts/trino-up.sh` twice, the smoke `POST /v1/statement` round-trip, then
  `bash scripts/trino-down.sh`; paste the smoke output into the summary.

## Commit message

`feat(trino): stand up the pinned docker compose tier (Trino + Iceberg REST + MinIO)`
