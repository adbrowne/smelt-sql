# Phase 4 summary — the Docker tier

**Shipped:**
- `scripts/trino-compose.yml` — three services under project `smelt-trino`: `trinodb/trino:483`
  (coordinator, publishes `${SMELT_TRINO_PORT:-18080}` only), `apache/iceberg-rest-fixture:1.10.1`
  (REST catalog, internal-only), `quay.io/minio/minio:RELEASE.2025-09-07T16-13-09Z` + a one-shot
  `quay.io/minio/mc` bucket-create init container (internal-only). All writable state is in
  Docker-managed named volumes; only `scripts/trino-catalog/` is bind-mounted, read-only.
- `scripts/trino-catalog/iceberg.properties` — REST catalog type, native-S3 filesystem
  (`fs.native-s3.enabled=true`), warehouse `s3://warehouse/`, all endpoints by compose service
  name (`minio`, `iceberg-rest`).
- `scripts/trino-up.sh` / `trino-down.sh` / `trino-env.sh` (`SMELT_TRINO_URL`,
  `SMELT_TRINO_USER`/`CATALOG`/`SCHEMA`), `scripts/README-trino.md` with the version table and
  host-port map.
- `crates/smelt-cli/tests/trino_tier_pins.rs` — 5 tests, all green, no Docker required.

**Decisions:** logged in `outcome.md` (image pins with the resolved tags, quay.io for MinIO since
it stopped publishing to Docker Hub, internal-only ports for MinIO/Iceberg REST). Named-volume
ruling was already logged in phase 3's summary period; this phase executed it.

**For the next planner:** phase 5 (HTTP statement client) is unblocked — `SMELT_TRINO_URL`,
catalog `iceberg`, default schema `smelt_dev` are the values `trino-env.sh` exports. Nothing
found out of scope. One note: Trino 483's Iceberg connector uses `fs.native-s3.enabled` +
`s3.*` properties (not legacy `hive.s3.*`) — worth keeping in mind if phase 6/7 need to tune S3
behavior further.

**Gates:**
- `cargo test -p smelt-cli --test trino_tier_pins` — 5/5 pass.
- `cargo test -p smelt-cli --test trino_spec_freshness` — unchanged, 4/4 pass.
- `mise run shellcheck` — zero findings (83 scripts).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  workspace tests, example_diagnostics).
- Live: `bash scripts/trino-up.sh` reached ready; smoke round-trip
  (`CREATE SCHEMA iceberg.smelt_dev` → `CREATE TABLE smelt_smoke` → `INSERT (1,'hello')` →
  `SELECT *`) returned `[1, "hello"]`, proving the write went through Trino → Iceberg REST →
  MinIO and back. A second `trino-up.sh` run with no intervening `down` reached ready again
  (idempotency). `bash scripts/trino-down.sh` removed all three containers and their volumes
  cleanly.
