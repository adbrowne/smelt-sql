window.BENCHMARK_DATA = {
  "lastUpdate": 1788675846032,
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
          "id": "d263980ac54d9f0c240370241105e84bbdb95e2b",
          "message": "property-diff: reads story names the window before its sources\n\n\"Each run now reads 7 days either side of the run window from a and b\"\ninstead of \"… of a, b\"; sources joined with \"and\".\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T16:18:14+10:00",
          "tree_id": "2b57ff646d5b6a51a89d8930f6c63e2cd7a339d2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/d263980ac54d9f0c240370241105e84bbdb95e2b"
        },
        "date": 1788675843838,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 34.550578,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 33.052305,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.6918070000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.413704,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.190807,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 839.184153,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.775464,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.958129,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.686217,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.570866,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 732.576086,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 3.95444,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 24.05954,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.452837,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}