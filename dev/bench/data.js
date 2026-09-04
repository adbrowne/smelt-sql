window.BENCHMARK_DATA = {
  "lastUpdate": 1788524517039,
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
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "47a63bf265d55a13cdf29500e62666c406b53228",
          "message": "outcome(20260904-dialect-emission-vocabulary): scaffold\n\nTemplates and operand-conditional verdicts per the multi_backend spec\ndiff (03828a14), paying down the DuckDB and Spark dialect-gap ratchets\nand retiring the unverified PostgreSQL emission column (#181).\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T22:17:30+10:00",
          "tree_id": "bfa3ad91060975fdf8287b0e1b8f5cfea2b82d8b",
          "url": "https://github.com/adbrowne/smelt-sql/commit/47a63bf265d55a13cdf29500e62666c406b53228"
        },
        "date": 1788524511906,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 32.936930000000004,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 31.525830999999997,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.641514,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.390369,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.18993,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 807.287966,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.652771,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.896277,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.677911,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.5297890000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 704.0278370000001,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.46065,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 23.531,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.074029,
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
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "47a63bf265d55a13cdf29500e62666c406b53228",
          "message": "outcome(20260904-dialect-emission-vocabulary): scaffold\n\nTemplates and operand-conditional verdicts per the multi_backend spec\ndiff (03828a14), paying down the DuckDB and Spark dialect-gap ratchets\nand retiring the unverified PostgreSQL emission column (#181).\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T22:17:30+10:00",
          "tree_id": "bfa3ad91060975fdf8287b0e1b8f5cfea2b82d8b",
          "url": "https://github.com/adbrowne/smelt-sql/commit/47a63bf265d55a13cdf29500e62666c406b53228"
        },
        "date": 1788524516059,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 34.21927810610829,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}