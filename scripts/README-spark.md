# Running Spark for parity tests

smelt's Spark backend talks to a **Spark Connect** server. For local parity
testing we run that server in Docker and a matching pyspark *client* in a pinned
venv that the PyO3 adapter imports. Spark-targeted tests are gated on
`SPARK_CONNECT_URL`: when it is unset they **skip** (the suite stays green
without Spark); when it is set they run against the live server.

See `docs/specs/multi_backend.md` for the parity contract and capability matrix.

## One-time client setup

The host needs `pyspark[connect]` matching the server's Spark version (the
`apache/spark` image is Spark 4.1.x). `python3.12-venv` (ensurepip) may be
absent on Ubuntu; bootstrap pip with `get-pip.py` rather than apt:

```bash
python3 -m venv --without-pip .smelt-spark-venv
curl -sL https://bootstrap.pypa.io/get-pip.py | .smelt-spark-venv/bin/python
.smelt-spark-venv/bin/pip install 'pyspark[connect]==4.1.1' pandas pyarrow grpcio grpcio-status protobuf
```

(`.smelt-spark-venv/` and `.smelt-spark-warehouse/` are gitignored.)

## Bring Spark up / down

```bash
bash scripts/spark-up.sh      # detached container, Connect server on :15002
source scripts/spark-env.sh   # export SPARK_CONNECT_URL + PYTHONPATH + warehouse
bash scripts/spark-down.sh    # stop + remove the container
```

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
before launching `autonomy-loop-forever.sh`). Otherwise W1·P3 (smoke + break
list) blocks with "Spark not provisioned" by design.

## Notes / gotchas

- The Connect jar is bundled in the image, so the core server needs no
  `--packages`. **Delta** on Spark Connect 4.1 is a separate concern (extension
  packages + catalog config) — W1 surfaces empirically whether Delta-on-Connect
  works here or whether the smoke runs on Parquet format first; see the W1
  break list.
- The client venv must be Python 3.12 to match the host interpreter PyO3 links
  against; C extensions (pyarrow) are ABI-specific.
