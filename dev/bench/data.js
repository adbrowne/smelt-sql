window.BENCHMARK_DATA = {
  "lastUpdate": 1786244580915,
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
          "id": "96334a84c4a3c02cdc130e0983a81829fe7d9fb8",
          "message": "docs(agents): configure Matt Pocock engineering skills for this repo\n\nWires the issue tracker (GitHub Issues via gh) and domain-docs layout\n(single-context CONTEXT.md + docs/adr/) that mattpocock-skills expects.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-09T12:58:44+10:00",
          "tree_id": "ef0ebf8b0b6d33bd26135f6036acd30f4742d835",
          "url": "https://github.com/adbrowne/smelt-sql/commit/96334a84c4a3c02cdc130e0983a81829fe7d9fb8"
        },
        "date": 1786244579028,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.831117,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 38.243412,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.69676,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.464668,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.212003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 816.616433,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.199419,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.951928,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.833361,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.668063,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 684.495018,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.69086,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.45593,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.129233,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}