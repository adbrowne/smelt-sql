window.BENCHMARK_DATA = {
  "lastUpdate": 1788579414788,
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
          "id": "982bcdf84e92b99c6da0e2a93ac6d155daa579f1",
          "message": "docs(research): assess gaps to using smelt as a production dbt replacement\n\nGap analysis assuming the current outcome backlog is fully complete: only\n3 backend crates exist, no slim-CI/--defer selection, no dbt migration\npath, no package/macro ecosystem, no shipped snapshots (SCD2).\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-05T13:33:03+10:00",
          "tree_id": "dc07480f42c860c4ad828fe5079e5ccc2e00c005",
          "url": "https://github.com/adbrowne/smelt-sql/commit/982bcdf84e92b99c6da0e2a93ac6d155daa579f1"
        },
        "date": 1788579411914,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 48.048871,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 46.084024,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.870452,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.55397,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.281292,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1134.141858,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.2412,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.4113770000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.082906,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.774576,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 964.109625,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.11979,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.88053,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.86886,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}