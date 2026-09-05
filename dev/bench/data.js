window.BENCHMARK_DATA = {
  "lastUpdate": 1788648699747,
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
          "id": "aa19cd9bb60966a5f55b5e658c4b75d2fec1580d",
          "message": "Merge remote-tracking branch 'origin/main' into scd2-keyed-succession-spec\n\n# Conflicts:\n#\tdocs/specs/diagnostics.md",
          "timestamp": "2026-09-06T08:49:19+10:00",
          "tree_id": "717ba5fe74f7ac78eac6930e370fbb660f17bbd5",
          "url": "https://github.com/adbrowne/smelt-sql/commit/aa19cd9bb60966a5f55b5e658c4b75d2fec1580d"
        },
        "date": 1788648697496,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.623951,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.350609,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.035787,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6306,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.285121,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1196.915107,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.999507,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.767148,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.510018,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.70898,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 999.855214,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.79688,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.04817,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.595304,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}