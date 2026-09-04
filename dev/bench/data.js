window.BENCHMARK_DATA = {
  "lastUpdate": 1788525244564,
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
          "id": "8f12d106d8e3e09e84406c26d11aba3757ff9347",
          "message": "docs: roadmap + TODO for the dialect emission vocabulary outcome and the #181 decision\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>",
          "timestamp": "2026-09-04T22:31:28+10:00",
          "tree_id": "31519853982f3519e2ef26c97773b5a6f3c5af2d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/8f12d106d8e3e09e84406c26d11aba3757ff9347"
        },
        "date": 1788525242430,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.938936000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.365179,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.294365,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.666095,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.304451,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1177.726668,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 4.06226,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.691962,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.301385,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.774616,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 982.735934,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.518850000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.789609999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.722532,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}