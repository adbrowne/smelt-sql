window.BENCHMARK_DATA = {
  "lastUpdate": 1787452094228,
  "repoUrl": "https://github.com/adbrowne/smelt-sql",
  "entries": {
    "Smelt Latency Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "be605c41ae55dae411eb47617da7860eff042a08",
          "message": "Merge pull request #169 from adbrowne/bigquery-backend-research\n\nMerges the BigQuery backend work: research, walking skeleton, and live parity legs.\n\nAlso carries three fixes made while verifying the merge:\n- the python-feature planner test, which the default CI matrix does not build\n- two argument-less format! calls that failed CI's clippy invocation\n- the Spark type oracle, which could not see the errors it classifies (stderr\n  was discarded, and Spark 4 changed its refusal wording)\n\nFollow-up filed as #171 (BigQuery dialect coverage audit).",
          "timestamp": "2026-08-23T00:16:18+10:00",
          "tree_id": "17e5ce94294c58c0907cd8f7605dfeec68abf3e5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/be605c41ae55dae411eb47617da7860eff042a08"
        },
        "date": 1787408448944,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 44.891271,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 42.849331,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.085425,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.474702,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.227526,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 890.789649,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.84816,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.310375,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.902453,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.66264,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 746.4807460000001,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.034250000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 25.9756,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.933404,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "8540b8da2877b266a60def425a4be2e8b796e260",
          "message": "ci: pin Spark image and close the local-vs-CI clippy gate\n\nTwo ways a green local run could still fail CI, both closed:\n\n1. `apache/spark:latest` was unpinned in two compat.yml jobs — the channel\n   through which a Spark 4 behaviour change reached CI with no code change.\n   Pinned to `apache/spark:4.0.0`, matching the jobs that were already pinned\n   and `scripts/spark-up.sh`. Pinning the workflow's `docker pull` alone was\n   not enough: `spark_integration.rs` and `scripts/test-spark-types.sh` name\n   the image themselves when they spawn containers, so both now use the same\n   pin with a `SMELT_SPARK_IMAGE` override.\n\n2. The local gate linted a different feature set than CI. verify-phase.sh ran\n   plain `cargo clippy --all-targets`; CI added `--no-default-features\n   --features smelt-cli/duckdb,smelt-ui/duckdb`, which strips defaults\n   workspace-wide and yields a different cfg surface. That gap is why a\n   `format!` warning survived a green local run.\n\n   Rather than copy CI's invocation into the local script (where it can drift\n   again), both callers now run `.claude/scripts/clippy-gate.sh`, which lints\n   both feature sets — neither subsumes the other. Verified: both sets pass.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-23T12:24:49+10:00",
          "tree_id": "b8c0d99d98eb0a06dc3d59186517c9159d1262d1",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8540b8da2877b266a60def425a4be2e8b796e260"
        },
        "date": 1787452092380,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 33.542747999999996,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.918478,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.833336,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.402348,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.183107,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 788.936927,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.891966,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.0445770000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.781941,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.556862,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 658.616298,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.067640000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 23.8061,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.081294,
            "unit": "ms"
          }
        ]
      }
    ],
    "Smelt Throughput Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "be605c41ae55dae411eb47617da7860eff042a08",
          "message": "Merge pull request #169 from adbrowne/bigquery-backend-research\n\nMerges the BigQuery backend work: research, walking skeleton, and live parity legs.\n\nAlso carries three fixes made while verifying the merge:\n- the python-feature planner test, which the default CI matrix does not build\n- two argument-less format! calls that failed CI's clippy invocation\n- the Spark type oracle, which could not see the errors it classifies (stderr\n  was discarded, and Spark 4 changed its refusal wording)\n\nFollow-up filed as #171 (BigQuery dialect coverage audit).",
          "timestamp": "2026-08-23T00:16:18+10:00",
          "tree_id": "17e5ce94294c58c0907cd8f7605dfeec68abf3e5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/be605c41ae55dae411eb47617da7860eff042a08"
        },
        "date": 1787408452617,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 31.529613284206818,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}