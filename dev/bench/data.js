window.BENCHMARK_DATA = {
  "lastUpdate": 1786861831336,
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
          "id": "544d26bb0c312bbd6c6411fe3337e92c9e15ef49",
          "message": "Merge pull request #167 from adbrowne/spec-decision-track-1\n\nspec: decision-track diffs — posture-derived deletion, determinism-scoped equivalence, and 18 smaller decisions",
          "timestamp": "2026-08-16T16:27:34+10:00",
          "tree_id": "162a50bc68879d302ff22b2bd5a68aca8145e45e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/544d26bb0c312bbd6c6411fe3337e92c9e15ef49"
        },
        "date": 1786861826958,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.746175,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.679166,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.811759,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.605514,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.33932500000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1082.265766,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.3523340000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.265811,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.148522,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.668091,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 886.462616,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 8.5462,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 47.17970999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.155235,
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
          "id": "544d26bb0c312bbd6c6411fe3337e92c9e15ef49",
          "message": "Merge pull request #167 from adbrowne/spec-decision-track-1\n\nspec: decision-track diffs — posture-derived deletion, determinism-scoped equivalence, and 18 smaller decisions",
          "timestamp": "2026-08-16T16:27:34+10:00",
          "tree_id": "162a50bc68879d302ff22b2bd5a68aca8145e45e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/544d26bb0c312bbd6c6411fe3337e92c9e15ef49"
        },
        "date": 1786861830546,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.3532516415305,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}