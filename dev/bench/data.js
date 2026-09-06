window.BENCHMARK_DATA = {
  "lastUpdate": 1788674390311,
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
          "id": "d24913d2962597c3ea5edb7cdada183d4aa43ec5",
          "message": "Merge pull request #193 from adbrowne/property-diff-narration\n\nproperty-diff: reviewer-facing stories in the PR comment, CLI, and editor",
          "timestamp": "2026-09-06T15:55:35+10:00",
          "tree_id": "c78b1225dbba8718c10d3b72e19d2ceda780bef8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d24913d2962597c3ea5edb7cdada183d4aa43ec5"
        },
        "date": 1788674384950,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.137764,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 53.838767,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.070818,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.608066,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.316576,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1194.630874,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.67945,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.152581,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.130719,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.6806530000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 995.437021,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.6781299999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.28253,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.515357,
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
          "id": "d24913d2962597c3ea5edb7cdada183d4aa43ec5",
          "message": "Merge pull request #193 from adbrowne/property-diff-narration\n\nproperty-diff: reviewer-facing stories in the PR comment, CLI, and editor",
          "timestamp": "2026-09-06T15:55:35+10:00",
          "tree_id": "c78b1225dbba8718c10d3b72e19d2ceda780bef8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d24913d2962597c3ea5edb7cdada183d4aa43ec5"
        },
        "date": 1788674389097,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.50624448913928,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}