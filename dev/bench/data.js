window.BENCHMARK_DATA = {
  "lastUpdate": 1784578885272,
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
          "id": "2a02b22e4fa4c562649f023bf994742fc0cbfe52",
          "message": "Merge pull request #165 from adbrowne/worktree-production\n\nProduction readiness (v0.5): W1 fail-loud, W2 operability, W4 Spark first-class, W3 adoption (tracking PR)",
          "timestamp": "2026-07-21T06:16:45+10:00",
          "tree_id": "b37ea82516d2ac5f2958d976a03348a82731de80",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2a02b22e4fa4c562649f023bf994742fc0cbfe52"
        },
        "date": 1784578883526,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.13637,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.997726,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.8524719999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6061719999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.380451,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1012.303484,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.534461,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.912159,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.690816,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.68548,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 816.959828,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.20249,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.299589999999995,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.835038,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}