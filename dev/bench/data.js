window.BENCHMARK_DATA = {
  "lastUpdate": 1787408453439,
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