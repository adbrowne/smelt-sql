# Anne's Words — Design

**Date:** 2026-09-02
**Status:** Approved

## Goal

A Wordle clone, published as a single self-contained page on the smelt docs
site at `https://smeltsql.com/annes-words/`. Near-pixel-perfect clone of the
original in look and feel; only the name differs.

This is deliberately unrelated to smelt itself. It lives in the docs site
purely because that site is already published to the internet. It is exempt
from the repo's normal spec/plan/ROADMAP workflow and its CI gates — it
touches no Rust crate.

## Requirements

| Decision | Choice |
|---|---|
| Word list | Freshly derived English 5-letter list with clean provenance: 2,000-word curated answer list + ~8,300 extra accepted guesses (see Word list provenance) |
| Puzzle mode | Daily puzzle (date-derived, same for everyone) with a practice toggle after finishing |
| Extras | Share result as emoji grid; stats with streaks and guess distribution |
| Style | Near-pixel-perfect clone: original palette, tile flip, shake on invalid, on-screen keyboard, toasts, modals |
| Out of scope | Hard mode, dark mode, accounts, server, analytics, build step, npm dependencies |

## Word list provenance

`words.js` is generated, not hand-curated, by `docs-site/tools/generate-words.mjs`
(zero npm dependencies, run manually: `node docs-site/tools/generate-words.mjs`).
Regenerating overwrites `docs-site/docs/annes-words/words.js`; the script is
deterministic — the same upstream sources always produce byte-identical output.

Sources (all permissively licensed):

- **Word list** — [dwyl/english-words `words_alpha.txt`](https://raw.githubusercontent.com/dwyl/english-words/master/words_alpha.txt) (Unlicense).
- **Frequency corpus** — [Norvig's `count_1w.txt`](https://norvig.com/ngrams/count_1w.txt), a tab-separated `word<TAB>count` list already ordered most-frequent-first.
- **Profanity exclusions** — [LDNOOBW's English bad-word list](https://raw.githubusercontent.com/LDNOOBW/List-of-Dirty-Naughty-Obscene-and-Otherwise-Bad-Words/master/en).
- **Local dictionaries** — the SCOWL-derived `/usr/share/dict/american-english` and `/usr/share/dict/british-english`, used both as extra guess candidates and to detect proper nouns (a word counts as a proper noun if it appears capitalised, e.g. matching `^[A-Z][a-z]{4}$`, in either file).

Two separate pools:

- **Guess pool** (wide, permissive) = all 5-letter lowercase words in
  (dwyl ∩ frequency corpus) ∪ (american-english ∪ british-english).
  Intersecting dwyl with the frequency corpus removes dwyl entries that are
  technically words but never appear in real text (e.g. `arioi`, `kanap`).
  `ALLOWED` is this pool minus `ANSWERS`, minus the profanity list, minus
  `MANUAL_EXCLUSIONS` (roughly 8,300 words).
- **Answer pool** (narrow, curated) — a word is answer-eligible only if it
  is in the system dictionary (american-english ∪ british-english) **and**
  also in dwyl (two independent dictionaries must agree — this keeps
  names/slang that only one source lists, e.g. `izumi`, `sitka`, `purdy`,
  `strom`, out of the answers, though they remain valid guesses) **and**
  appears in the frequency corpus (for ranking) **and** contains at least
  one vowel (`[aeiou]`, which excludes non-words like Roman numerals, e.g.
  `xxvii`) **and** does not end in `s` (plurals), is not on the profanity
  list, is not capitalised in either system dictionary (proper-noun
  heuristic), and is not in the hand-maintained `MANUAL_EXCLUSIONS` list
  (vulgar/unsuitable words the profanity list misses, e.g. `cunny`).
  `ANSWERS` is the top 2,000 answer-eligible words by frequency rank,
  shuffled once at generation time with a Fisher-Yates shuffle driven by a
  mulberry32 PRNG seeded `0x9e3779b9` (the same documented seed the
  original list used).

`words.js` keeps the same export contract `ui.js` already depends on: plain
`ANSWERS` and `ALLOWED` arrays of lowercase 5-letter words; consumers union
the two for guess validation, so `ALLOWED` holds only non-answer words.

## Publishing

MkDocs copies any non-markdown file under `docs-site/docs/` verbatim into
`docs-site/site/`. Therefore `docs-site/docs/annes-words/index.html` publishes
to `/annes-words/` with:

- no entry in `mkdocs.yml` `nav`,
- no markdown page (so no "page not in nav" noise),
- no effect on `mkdocs build --strict`.

The existing `Docs` workflow (`.github/workflows/docs.yml`) already triggers on
`docs-site/**`, so a push to `main` publishes it with no CI changes.

## Architecture

Plain ES modules, no build step, no framework, no dependencies. Loaded by
`index.html` via `<script type="module">`.

```
index.html   markup shell: header, board, keyboard, modals
style.css    Wordle palette, board grid, flip/shake/pop keyframes
words.js     data only: ANSWERS (shuffled, seeded), ALLOWED (Set source)
game.js      pure logic, zero DOM — the only unit under test
ui.js        all DOM + localStorage; imports game.js and words.js
```

The split exists so `game.js` can be imported by `node --test` with no DOM
shim. Any function that touches `document`, `window`, `localStorage`, or the
current time belongs in `ui.js`; `game.js` takes dates and state as arguments.

### `game.js` interface

```js
scoreGuess(guess, answer) -> Array<'correct'|'present'|'absent'>  // length 5
mergeKeyStates(prev, guess, marks) -> {letter: state}             // green > yellow > grey
dailyIndex(date, listLength) -> number                            // local-midnight days since epoch
shareText(puzzleNo, marks2d, won) -> string                       // emoji grid
isValidGuess(word, allowedSet) -> boolean
```

`scoreGuess` uses the two-pass algorithm: mark exact matches first, decrement a
letter-count pool, then mark presents only while the pool allows. This is the
classic clone bug and is what the test suite exists for.

### Daily selection

`ANSWERS` is shuffled once at authoring time with a fixed seed, so the daily
sequence is not alphabetical and not guessable from the source order. The
puzzle number is the count of whole local days since the epoch
`2026-01-01T00:00:00` local, and the answer is `ANSWERS[n % ANSWERS.length]`.
Computing against local midnight (not UTC) means a player's day boundary
matches their own midnight. The countdown targets the next local midnight.

### Persistence

One `localStorage` key, `annes-words:v1`, holding a versioned JSON object:

```jsonc
{
  "version": 1,
  "daily": { "puzzle": 244, "guesses": ["crane"], "status": "playing" },
  "stats": { "played": 0, "wins": 0, "streak": 0, "maxStreak": 0,
             "lastPuzzle": null, "dist": [0,0,0,0,0,0] }
}
```

A stored `daily` whose `puzzle` is not today's is discarded on load. Practice
games are never persisted and never touch `stats`. Any parse failure resets to
defaults rather than throwing — a corrupt key must never brick the page.

### Stats semantics

`streak` increments on a daily win, resets to 0 on a daily loss. A skipped day
is detected on load: if the stored `daily.puzzle` is older than
`todayPuzzle - 1`, the streak resets. `dist[i]` counts daily wins in `i+1`
guesses.

## Testing

`docs-site/tests/annes-words.test.mjs`, run with `node --test` — outside
`docs/`, so it is not published. Zero npm dependencies.

Covered: duplicate-letter scoring in both directions, key-state precedence,
daily-index determinism and rollover, share-grid formatting, guess validation.
UI behaviour is verified by playing it in a browser at each checkpoint.

## Checkpoints

Each is a commit pushed to the `annes-words` branch and is playable in a
browser via `cd docs-site/docs && python3 -m http.server 8000`, then
`http://localhost:8000/annes-words/`.

1. Skeleton — board and keyboard render, typing shows letters, no scoring.
2. Playable — scoring, flip reveal, invalid shake, win/lose. Practice game.
3. Daily — date-derived word, refresh-resume, countdown, practice toggle.
4. Stats & share — stats modal, streaks, distribution, emoji copy.
5. Polish — help modal, mobile layout, `mkdocs build --strict` verified.
