window.BENCHMARK_DATA = {
  "lastUpdate": 1790474541966,
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
          "distinct": false,
          "id": "ef1a0b317335a8c48807c06c3d1b87e3911e07a0",
          "message": "fix(maes-words): final review — ladder per activity, sentence articles, reset guard, polish\n\nEight review findings: refreshTurn re-derives a planned turn's activity from\nthe word's current level so a repeated word in one round earns each rung\nthrough its own activity; framesFor + noA flags stop \"a egg\"/\"a milk\"/\"a six\"\nby dropping a-frames for vowel-initial and mass-noun/number words; a wrong\ntap on the grown-up reset guard now exits home instead of shaking in place;\nheader uses min-height so it doesn't clip under the safe-area inset;\nstartRound guards against a double-tap starting two rounds; gift/rat/van\nremoved from the word bank for image ambiguity; the speaker only cancels an\nin-flight utterance instead of unconditionally, avoiding an iOS race.\n\nCo-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_01ADN4LtXLVBhtLTgmfpNPfe",
          "timestamp": "2026-09-27T11:55:55+10:00",
          "tree_id": "722f31f478c8a876cc5235febfe504bf60911c7c",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ef1a0b317335a8c48807c06c3d1b87e3911e07a0"
        },
        "date": 1790474538877,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.630874,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 54.222104,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.075713,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.625687,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.395933,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1246.863142,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.82316,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.176062,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.20136,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.695703,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1018.410074,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.543000000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.47287,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.91044,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}