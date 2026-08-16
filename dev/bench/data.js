window.BENCHMARK_DATA = {
  "lastUpdate": 1786840096477,
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
          "id": "3611fae3c43682fa52b0806ea590c8aa763a1f74",
          "message": "research: open-questions triage doc for the decision track\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-16T10:25:19+10:00",
          "tree_id": "25a1638cf973ceb3fe945a50bbb1759362c2d127",
          "url": "https://github.com/adbrowne/smelt-sql/commit/3611fae3c43682fa52b0806ea590c8aa763a1f74"
        },
        "date": 1786840094052,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.176288,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.989782000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.8876930000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.620321,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.379941,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1093.466125,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.415487,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.28523,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.1760249999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.708196,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 903.950162,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.425300000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 35.19411,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.933855,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}