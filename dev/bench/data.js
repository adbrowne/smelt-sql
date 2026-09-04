window.BENCHMARK_DATA = {
  "lastUpdate": 1788526010772,
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
          "id": "124de4947e855a3865b2d0ad8e07c6f16aaf322d",
          "message": "docs(TODO): record measured DuckDB % sign and Spark DIV call-form results\n\nBoth dialect-spec claims verified live: DuckDB 1.4.4 float % is truncating,\nSpark 4.0.0 accepts div(a, b) in call form. No spec change needed.\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T22:41:25+10:00",
          "tree_id": "ad2ed968a200ac26d8534d68615651450dbdda01",
          "url": "https://github.com/adbrowne/smelt-sql/commit/124de4947e855a3865b2d0ad8e07c6f16aaf322d"
        },
        "date": 1788526007994,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.518861,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.328152,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.896033,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6083850000000001,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.387242,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1151.278523,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.399926,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.260419,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.188254,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.683737,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 954.835756,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.94039,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.781150000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.864568,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}