# Running Spark for parity tests

smelt's Spark backend talks to a **Spark Connect** server. For local parity
testing we run that server in Docker and a matching pyspark *client* in a pinned
venv that the PyO3 adapter imports. Spark-targeted tests are gated on
`SPARK_CONNECT_URL`: when it is unset they **skip** (the suite stays green
without Spark); when it is set they run against the live server.

See `docs/specs/multi_backend.md` for the parity contract and capability matrix.

## Versions

| Component | Version | Notes |
|-----------|---------|-------|
| Spark server image | `apache/spark:4.0.0` | Pinned — see below |
| Delta Lake | `io.delta:delta-spark_2.13:4.0.0` | Resolved on first `spark-up.sh` run via Ivy |
| pyspark client | `4.0.0` | Must match server — pyspark 4.1.x uses config keys absent in Spark 4.0.0 (`localRelationChunkSizeRows`) |
| Scala | 2.13 | Must match the `delta-spark` artifact suffix |

**Why pinned to `apache/spark:4.0.0`:** `apache/spark:latest` (Spark 4.1.x) has
an internal API break (`LogKey.$init$`) that causes Delta 4.0.0 to fail at
runtime with `NoSuchMethodError`. When a future Delta release catches up to
Spark 4.1.x, override the image with `SMELT_SPARK_IMAGE=apache/spark:latest`.

## One-time client setup

The host needs `pyspark[connect]` for the venv the PyO3 adapter imports.
`python3.12-venv` (ensurepip) may be absent on Ubuntu; bootstrap pip with
`get-pip.py` rather than apt:

```bash
python3 -m venv --without-pip .smelt-spark-venv
curl -sL https://bootstrap.pypa.io/get-pip.py | .smelt-spark-venv/bin/python
.smelt-spark-venv/bin/pip install 'pyspark[connect]==4.0.0' pandas pyarrow grpcio grpcio-status protobuf
```

(`.smelt-spark-venv/`, `.smelt-spark-warehouse/`, and `.smelt-spark-ivy/` are gitignored.)

## Bring Spark up / down

```bash
bash scripts/spark-up.sh      # detached container, Connect server on :15002
source scripts/spark-env.sh   # export SPARK_CONNECT_URL + PYTHONPATH + warehouse
bash scripts/spark-down.sh    # stop + remove the container
```

**First-run note:** `spark-up.sh` uses `--packages` to resolve Delta Lake jars
via Ivy on first run (Ivy downloads ~30 MB; can take 1–2 minutes with network).
Jars are cached in `.smelt-spark-ivy/` (bind-mounted into the container as
`/opt/spark/work-dir/.ivy2`) so subsequent runs are fast.

## Run the Spark tests

```bash
source scripts/spark-env.sh
cargo test -p smelt-backend-spark --features spark        # backend integration tests
cargo test -p smelt-cli --features spark --test dual_target_harness   # dual-target harness (W1·P2)
```

With `SPARK_CONNECT_URL` unset, the same commands compile and pass with all
Spark assertions skipped.

## For the autonomy loop

The loop's stateless iterations do **not** stand Spark up. Bring it up once and
ensure `SPARK_CONNECT_URL` (and the venv `PYTHONPATH`) are exported into the
loop's environment (e.g. `source scripts/spark-env.sh` in the tmux session
before launching `autonomy-loop-forever.sh`).

## Notes / gotchas

- The client venv must be Python 3.12 to match the host interpreter PyO3 links
  against; C extensions (pyarrow) are ABI-specific.
- `SMELT_SPARK_IMAGE`, `SMELT_DELTA_VERSION` env-vars override the defaults in
  `spark-up.sh` if you need to test a different Spark/Delta combination.
- The Ivy cache (`SMELT_SPARK_IVY_CACHE`, default `.smelt-spark-ivy/`) is
  bind-mounted into the container at `/opt/spark/work-dir/.ivy2` and declared
  via `--conf spark.jars.ivy` so Spark uses it rather than writing to
  `/nonexistent` (the Spark image user's home).
