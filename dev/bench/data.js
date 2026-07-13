window.BENCHMARK_DATA = {
  "lastUpdate": 1783917424958,
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
          "id": "11558e29cf88ff6b9f99cfd83742d8c92e35c9a0",
          "message": "ci(docs): build via uv instead of a stale requirements.txt\n\ndocs-site/pyproject.toml + uv.lock have been the documented dependency\nsource since docs-site/CLAUDE.md introduced them, but docs.yml and\ndocs-pr-preview.yml still installed from a vestigial requirements.txt\nthat never tracked new plugin deps. This went unnoticed because no CI\nhad ever built docs for a PR branch before docs-pr-preview.yml existed\ntoday; its first real run (PR #160, which adds the mkdocs-redirects\nplugin) failed with \"the 'redirects' plugin is not installed\" since\nrequirements.txt only lists mkdocs-material.\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_015PV1emYQUydUw5bRxEa7Zf",
          "timestamp": "2026-07-13T14:33:56+10:00",
          "tree_id": "b873363bed295a427200615c4b44396d0df0be59",
          "url": "https://github.com/adbrowne/smelt-sql/commit/11558e29cf88ff6b9f99cfd83742d8c92e35c9a0"
        },
        "date": 1783917420153,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 59.118235,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.958755,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.877223,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.611348,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.366692,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 922.719283,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.179859,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.158329,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.123674,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.688171,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 756.727287,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.19784,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.92515,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.015789000000002,
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
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "11558e29cf88ff6b9f99cfd83742d8c92e35c9a0",
          "message": "ci(docs): build via uv instead of a stale requirements.txt\n\ndocs-site/pyproject.toml + uv.lock have been the documented dependency\nsource since docs-site/CLAUDE.md introduced them, but docs.yml and\ndocs-pr-preview.yml still installed from a vestigial requirements.txt\nthat never tracked new plugin deps. This went unnoticed because no CI\nhad ever built docs for a PR branch before docs-pr-preview.yml existed\ntoday; its first real run (PR #160, which adds the mkdocs-redirects\nplugin) failed with \"the 'redirects' plugin is not installed\" since\nrequirements.txt only lists mkdocs-material.\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_015PV1emYQUydUw5bRxEa7Zf",
          "timestamp": "2026-07-13T14:33:56+10:00",
          "tree_id": "b873363bed295a427200615c4b44396d0df0be59",
          "url": "https://github.com/adbrowne/smelt-sql/commit/11558e29cf88ff6b9f99cfd83742d8c92e35c9a0"
        },
        "date": 1783917423911,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.59554720751004,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}