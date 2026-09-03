window.BENCHMARK_DATA = {
  "lastUpdate": 1788475184948,
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
          "id": "994e6f3fd6654d1947940ac8006656f7b732b393",
          "message": "Definition-delta migrate: smelt migrate/rebuild, contract lattice, and follow-up closures (#185)\n\nDefinition-delta migrate: smelt migrate/rebuild, contract lattice, and follow-up closures",
          "timestamp": "2026-09-04T08:34:39+10:00",
          "tree_id": "f4686216a058cd5d4e3e7e64cc481755868573c6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/994e6f3fd6654d1947940ac8006656f7b732b393"
        },
        "date": 1788475182458,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.528565,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.164082,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.12497,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.630388,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29999,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1157.207051,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.808102,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.233926,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.148839,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.73862,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 971.04476,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.79221,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.46438,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.624271,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}