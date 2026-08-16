window.BENCHMARK_DATA = {
  "lastUpdate": 1786839665995,
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
          "id": "20219b708fec0f03c1516fb0b7bdc7b4ba34a1ba",
          "message": "Merge pull request #166 from adbrowne/spec-redraft-incremental-models\n\nRedraft incremental_models.md as a pyramid-ordered timeless oracle",
          "timestamp": "2026-08-16T10:16:14+10:00",
          "tree_id": "6a5c9fe2cc2edea19b9fe505564d34e97bd45e8f",
          "url": "https://github.com/adbrowne/smelt-sql/commit/20219b708fec0f03c1516fb0b7bdc7b4ba34a1ba"
        },
        "date": 1786839664123,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 57.257907,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.676339,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.321589,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.649603,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29314500000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1113.152555,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 4.173698,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.823136,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.418636,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.882417,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 925.202492,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.93366,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.61648,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.89693,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}