# Running Trino for the Trino target

smelt's Trino backend talks to a Trino coordinator over its HTTP statement
protocol (`POST /v1/statement`), with Iceberg as the one connector profile
(`docs/specs/multi_backend.md`). For local integration testing we run the
whole tier — coordinator, Iceberg REST catalog, MinIO object storage — as a
pinned `docker compose` stack. Trino-targeted tests are gated on
`SMELT_TRINO_URL`: when it is unset they **skip** (the suite stays green
without Trino); when it is set they run against the live tier.

See `docs/specs/multi_backend.md` for the parity contract and capability
matrix.

## Versions

| Component | Image | Tag | Why this pin |
|-----------|-------|-----|---------------|
| Trino coordinator | `trinodb/trino` | `483` | Latest stable release as of 2026-09-14 (equal to `:latest` at pin time); pinned by digit so a future `:latest` move can't silently change behavior under CI. |
| Iceberg REST catalog | `apache/iceberg-rest-fixture` | `1.10.1` | The Apache Iceberg project's own REST-catalog fixture image, JDBC-backed (SQLite) catalog metadata store — stateless enough that `down -v` always gives a clean catalog. |
| MinIO server | `quay.io/minio/minio` | `RELEASE.2025-09-07T16-13-09Z` | MinIO stopped publishing to Docker Hub; `quay.io/minio/minio` is the maintained registry. Picked the newest plain `RELEASE.*` tag (no `-cpuv1`/`hotfix` suffix) at pin time. |
| MinIO client (`mc`) | `quay.io/minio/mc` | `RELEASE.2025-08-13T08-35-41Z` | Used only by the one-shot bucket-create init container; same registry as the server. |

**Why Iceberg, not Hive:** the connector — not Trino itself — decides the
write surface. `MERGE`, `UPDATE`, `DELETE` and `CREATE OR REPLACE TABLE` exist
on Iceberg and do not exist on Hive, and the whole incremental/ledger story
downstream of this tier depends on that. See
`docs/outcomes/20260913-trino-target-spine/outcome.md`.

## Bring the tier up / down

```bash
bash scripts/trino-up.sh      # coordinator on :18080, Iceberg REST + MinIO internal-only
source scripts/trino-env.sh   # export SMELT_TRINO_URL + catalog/schema/user
bash scripts/trino-down.sh    # remove all three containers and their named volumes
```

`trino-up.sh` always runs `docker compose down -v --remove-orphans` before
`up -d`, so it is safe to re-run without a prior `trino-down.sh` — there is no
host-owned leftover state (see "Named volumes, not bind mounts" below) for a
re-run to trip over.

## Named volumes, not bind mounts

All three services' writable state (MinIO's object data, the Iceberg REST
catalog's SQLite metadata DB, Trino's own `/data`) lives in Docker-managed
named volumes, removed wholesale by `down -v`. The only bind mount is the
read-only `scripts/trino-catalog/` directory. This sidesteps the failure mode
`scripts/spark-up.sh` hit: a `chmod` on a container/root-owned leftover host
directory aborting under `set -e` *before* `docker run`, after which every
test failed with no hint the server had never started. A named volume has no
host-owned path for that chmod to abort on.

## Host ports

Only the Trino coordinator publishes a host port — `18080` by default
(`SMELT_TRINO_PORT` to override) — chosen to be disjoint from
`scripts/spark-up.sh`'s `15002` and from every other tier's container names
(`smelt-spark`). MinIO and the Iceberg REST catalog are reachable only inside
the compose network (`minio:9000`, `iceberg-rest:8181`); nothing outside this
tier needs to reach them directly.

| Service | Container name | Host port |
|---------|-----------------|-----------|
| Trino coordinator | `smelt-trino-coordinator` | `${SMELT_TRINO_PORT:-18080}` → `8080` |
| Iceberg REST catalog | `smelt-trino-iceberg-rest` | none (internal only) |
| MinIO | `smelt-trino-minio` | none (internal only) |
| MinIO init (`mc`) | `smelt-trino-minio-init` | one-shot, no port |

## Run the Trino tests

```bash
source scripts/trino-env.sh
cargo test -p smelt-backend-trino               # backend integration tests
```

With `SMELT_TRINO_URL` unset, the same commands compile and pass with all
Trino-targeted tests skipped.

## In CI

The `trino-integration` job in `.github/workflows/compat.yml` brings up the
tier, runs `smelt-backend-trino`'s test suite plus the CLI's `trino_smoke`,
`seed_parity` and `materialization_parity` legs, and tears the tier down
(`if: always()`). Like `spark-parity`, it runs on the nightly `schedule`,
when a PR carries the `run-docker-tests` label, or when the `changes` job's
path filter flags a Trino-relevant change.

Locally, an unset `SMELT_TRINO_URL` is the correct green outcome — every
Trino-targeted test skips rather than fails. In this CI job the URL *is* set
by the tier startup step, so a skip there means a leg silently ran against
nothing; the job greps its captured test output for a skip line and fails
loudly if it finds one, rather than reporting a green run in which nothing
actually exercised Trino.
