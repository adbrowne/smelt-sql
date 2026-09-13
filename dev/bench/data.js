window.BENCHMARK_DATA = {
  "lastUpdate": 1789282759213,
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
          "distinct": false,
          "id": "3b05f095cde029afdb93832f73377ed26a0b5f45",
          "message": "outcome(databricks-dogfood-spine): plan phase 11c\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-13T16:04:01+10:00",
          "tree_id": "ea8f85885ceeba791b2e48801f40c7c792ee86a8",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3b05f095cde029afdb93832f73377ed26a0b5f45"
        },
        "date": 1789282756220,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.700634,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.409186,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.005075,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.618621,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.363322,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1221.439712,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.506858,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.328387,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.165952,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.743104,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 999.671731,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.7074,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.28443,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.77824,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}