# Mae's Words — Design

**Date:** 2026-09-27
**Status:** Approved

## Goal

A silent, touch-first reading game for Mae (age 5) that helps her learn
2–4 letter words, published as a self-contained page on the smelt docs site
at `https://smeltsql.com/maes-words/`, a sibling of Anne's Words.

Mae can read the first Peter and Jane books (Ladybird Key Words scheme:
whole-word recognition of a controlled vocabulary, with early phonics
alongside). The game should meet her there: recognise short words, tell
look-alike words apart, and build words from letters — and it should
remember how she is doing so it steps up only as she does.

Like Anne's Words this is deliberately unrelated to smelt. It lives in the
docs site purely because that site is already published. It is exempt from
the repo's spec/plan/ROADMAP workflow and its Rust CI gates.

## Requirements

| Decision | Choice |
|---|---|
| Player | One child, age 5, playing alone on an iPad or phone |
| Sound | Silent by design. Pictures carry meaning; nothing requires audio or an adult reading aloud. An optional speaker button uses the browser's built-in speech synthesis when available; nothing depends on it |
| Words | A few hundred hand-curated 2–4 letter words in three difficulty tiers: concrete nouns, simple verbs and adjectives shown by an emoji or a vendored pictogram, and colour words shown as coloured tiles. Every picture is unambiguous to a five-year-old |
| Sight words | Peter and Jane key words, which cannot be pictured, are taught directly by the Fill activity and have their own mastery ladder |
| Activities | Read (word → picture), Pick (picture → word among look-alikes), Build (picture → letter tiles), Sentence (sight-word sentence with a picture blank), Fill (sentence with the sight word blanked) |
| Progression | Per-word mastery ladder (0–3) driving which activity a word gets; new words trickle in; tiers unlock by mastery |
| Persistence | `localStorage`, one versioned key |
| Failure | None. A wrong answer counts against the word once, then Mae tries again until she gets it. No timers, no lives, no losing |
| Grown-up corner | Hidden screen (long-press the title) with per-word status, reset, and a link to the picture credits |
| Theme | Pink. Primary accent a warm pink (around `#e91e8c`), soft pink backgrounds, white tiles, green for correct and grey for wrong |
| Out of scope | Accounts, server, analytics, build step, npm dependencies, runtime fetches, audio assets, multiple profiles, dark mode, self-hosted emoji |

## Word bank

`words.js` exports a hand-curated array of a few hundred entries. Each entry
has a word, a tier and a `pic` describing how it is shown:

```js
export const WORDS = [
  { w: 'cat',  tier: 1, pic: { kind: 'emoji',  text: '🐱' } },
  { w: 'mum',  tier: 1, pic: { kind: 'svg',    src: 'img/mum.svg' } },
  { w: 'jump', tier: 2, pic: { kind: 'svg',    src: 'img/jump.svg' }, pos: 'verb' },
  { w: 'red',  tier: 1, pic: { kind: 'colour', css: '#e53935' }, pos: 'adj', noA: true },
  { w: 'milk', tier: 2, pic: { kind: 'emoji',  text: '🥛' }, noA: true },
  // ...
];
```

`pic.kind` is one of `emoji`, `svg`, `colour`. `noA` marks words that never
take the article "a" (mass nouns such as `milk`, `mud`, `jam`; numbers).
`pos` is `'verb'` or `'adj'` for words that are not nouns; a word with no
`pos` is a noun. Colour words are adjectives (`pos: 'adj'`) and always carry
`noA`, since a colour tile stands in for "is red", not "a red". `pos` steers
which frames a word can appear in (see Sight words, sentences and Fill) — it
does not change how Read, Pick or Build present the word.
The UI renders a `pic` in exactly one place, `renderPic(pic)`, used by Read
tiles, the Pick/Build/Sentence/Fill prompt, the wall chips and the Grown-up
table.

Rules for inclusion:

- 2–4 lowercase letters, a single word, no plurals.
- Concrete nouns, simple action verbs (`run`, `jump`, `sit`, `eat`) and
  everyday adjectives (`big`, `hot`, `wet`, `sad`) are all admissible when a
  picture shows them unambiguously.
- **The ambiguity rule**: shown only the picture, a five-year-old says exactly
  this word (🐶 is "dog", not "puppy"; a picture of jumping is "jump", not
  "hop"). Anything a child could reasonably name two ways is left out.
  Pictograms are *more* ambiguous than emoji for some concepts, so every new
  entry gets a per-word review and reviewers may drop entries.
- Emoji stay wherever they are already unambiguous. Pictograms are for words
  emoji cannot show (people, verbs, adjectives, household objects).
- Colour words (`red blue pink grey`) are shown as a solid coloured tile with
  no image. Spelling stays within the 2–4 letter rule, so `green` and
  `yellow` are out.
- Tier 1: CVC and shorter words with the most common letters.
  Tier 2: 3–4 letter words with blends and common digraphs.
  Tier 3: 4-letter words with trickier vowel patterns.

### Pictogram sources

| Source | Licence | Use |
|---|---|---|
| Mulberry Symbols (`mulberrysymbols/mulberry-symbols`, `EN/*.svg`) | CC BY-SA 2.0 UK | Primary. Verbs are named `<verb>_,_to.svg` |
| ARASAAC (`api.arasaac.org` search, `static.arasaac.org` images) | CC BY-NC-SA 4.0 | Gap-filler only, for words Mulberry lacks |

Constraints:

- SVGs are **vendored** into `docs-site/docs/maes-words/img/`, one file per
  word, optimised. The game makes no runtime fetches and stays
  dependency-free. Total image size is checked and kept modest (a few hundred
  small SVGs). ARASAAC ships PNG; those are vendored as PNG under the same
  `src` field.
- A generator script, `docs-site/tools/generate-maes-pics.mjs`, holds the
  hand-curated word → source → asset-id table, fetches the assets, optimises
  them and writes `img/` and the `svg`/`png` entries of `words.js`. It is
  deterministic and re-runnable; hand-curation lives in the script's table,
  never in edits to the generated entries.
- `credits.html` (adult-facing, linked from the Grown-up corner) attributes
  Mulberry Symbols (Steve Lee, CC BY-SA 2.0 UK) and ARASAAC (Government of
  Aragon, CC BY-NC-SA 4.0), notes that ARASAAC's terms are non-commercial and
  that the game is a free family game, and lists which words use which source.
  Symbols are used unmodified, so the share-alike clause imposes nothing
  beyond attribution.

### Look-alike distractors

Pick and Build need plausible wrong answers. A build-time-free approach:
`distractors(word, bank)` returns words from the same bank ranked by edit
distance (`cat`→`cot`, `cut`, `can`), falling back to words of the same
length. Because distractors are real words from the bank, they are also
things Mae is learning — every wrong option she reads is still practice.
Letter-tile distractors for Build are letters at edit distance one from
the word's letters, never a letter that makes another bank word an
alternative correct answer.

In Read, a colour word's distractors are the other colour words, so the
options are four swatches rather than one swatch among emoji, which would
give the answer away. This colour-only pool applies only where the options
*are* pictures, i.e. Read; Pick and Sentence show one picture and rank their
word-tile options by the normal edit-distance ranking regardless of colour,
since a colour word among word tiles gives nothing away.

### Sight words, sentences and Fill

Peter and Jane's key words (`the is a and here I like my to in on up look
come see go you we it at for are was has said this they with can have`)
cannot be pictured. They are taught two ways.

**As context**, in the Sentence activity: a frame with one picture blank,
where Mae reads the sight words to pick the pictured word.

**As targets**, in the Fill activity: the same kind of frame with the
picture noun filled in and the *sight word* blanked. Mae picks the sight
word from three word tiles.

```
I ___ the 🐶       choices: like · see · am
___ is a 🐱        choices: Here · The · My
The 🐸 is ___      choices: here · up · in
```

Sight words have their own bank:

```js
export const SIGHT = [
  { w: 'the', tier: 1 }, { w: 'is', tier: 1 }, { w: 'a', tier: 1 },
  { w: 'here', tier: 2 }, { w: 'like', tier: 2 }, // ...
];
```

A test asserts no word appears in both `WORDS` and `SIGHT`, so the two banks
share the `levels` map without clashing.

Frames are templates with a `{noun}` slot (any part of speech, matched by
`pos`) and an optional `{sight}` slot, and declare **every** sight word that
fits the blank:

```js
export const FRAMES = [
  { text: 'I {sight} the {noun}',  fits: ['like', 'see', 'have'] },
  { text: '{sight} is a {noun}',   fits: ['Here', 'This', 'It'] },
  { text: 'The {noun} is {sight}', fits: ['here', 'up', 'in'] },
  { text: 'I can see a {noun}',    fits: [] },   // Sentence-only
  { text: 'I can {noun}',          fits: [], pos: 'verb' },
  { text: 'It is {noun}',          fits: [], pos: 'adj' },
  // ...
];
```

A frame with no `pos` takes a noun; `pos: 'verb'` or `pos: 'adj'` restricts
it to words of that part of speech, matching the word's own `pos`
(`framesFor`/Fill never pair a word with a frame for a different part of
speech, so "I can dog" or "It is jump" can't be generated).

`fits` is exhaustive: it lists **every** `SIGHT` word that reads correctly
in the blank, not just the ones the frame was written to teach. This matters
because Fill's distractors are drawn from the sight words *not* in `fits`
(see below) — if a fitting word were missing from the list, a correct answer
would be marked wrong the moment it was offered as a distractor.

- A Sentence turn uses a frame with its sight word chosen from `fits` (or a
  fixed frame with an empty `fits`) and blanks the noun.
- A Fill turn for sight word `s` picks a frame and a noun (or verb/adjective)
  together: the frame's `fits` must contain `s`, and the picked word must be
  a word `framesFor` admits into that frame (same `pos`, and the frame's
  article rules — no "a {noun}" for vowel-initial or `noA` words). Choosing
  the pair jointly, rather than picking a frame and then forcing a word into
  it, means a frame's own "a {noun}" is never rewritten to "the {noun}" to
  fit an incompatible word — it is simply paired with a word it already
  admits. Nouns/verbs/adjectives already introduced are preferred; if none of
  them fits any matching frame, every word of the required part of speech is
  considered. The distractors are sight words **not** in that frame's `fits`,
  so exactly one option makes the sentence right. Capitalisation follows the
  frame: a sentence-initial blank shows capitalised options.
- Fill distractors at level 0 are drawn from sight words of a different
  length; at level 1 and for review they are the closest by edit distance
  (`the`/`then`/`they`, `is`/`it`/`in`), the same `distractors()` ranking used
  for nouns.

## Progression model

Every word has a mastery level in `0..3`:

| Level | Meaning | Activity used |
|---|---|---|
| 0 | New / not yet recognised | Read |
| 1 | Recognises it | Pick |
| 2 | Tells it from look-alikes | Build |
| 3 | Learned | Sentence, occasionally, as review |

Sight words use the same four levels with their own activities:

| Level | Meaning | Activity used |
|---|---|---|
| 0 | New | Fill, easy distractors |
| 1 | Recognises it | Fill, look-alike distractors |
| 2 | Tells it from look-alikes | Build |
| 3 | Learned | Fill, occasionally, as review |

Rules:

- A clean first-try answer: level `+1` (capped at 3) and one star. A wrong
  answer: level `−1` (floored at 0), applied once per turn. **Try again**:
  after a wrong answer Mae keeps going on the same turn until she gets it;
  fixing it earns no level and no star, so a guess-until-green turn is still a
  step back. Only a clean answer moves a word up.
- **Active set**: words with level `< 3` that have been introduced. New
  words are introduced from the lowest unlocked tier, `2` at a time,
  whenever the active set has fewer than `6` words at level 0.
  Start of game: the first `6` tier-1 words are introduced.
- Sight words are introduced by the same trickle rule from their own bank,
  one at a time, whenever fewer than `2` introduced sight words are at
  level 0. Start of game: the first `2` tier-1 sight words are introduced.
  Sight-word tiers unlock on the same 75% rule over `SIGHT`.
- **Tier unlock**: tier `N+1` unlocks when at least 75% of tier `N` words
  are at level ≥ 2.
- **Round**: 10 turns. Each turn picks a word from the combined active set
  (nouns and sight words) with weight favouring lower levels
  (`weight = 4 − level`), with at most one learned word (level 3) per round
  as review, chosen uniformly from learned words of either kind. The same
  word is never picked twice in a round unless the active set is smaller
  than 10.
- Stars: one per correct answer, accumulated forever. Purely motivational.

This is spaced repetition without a schedule: a word cycles through the
activities at Mae's pace, drops back when she stumbles, and comes back for
review once learned.

## Screens and flow

1. **Home** — big "Play" button, star count, a strip of recently learned
   words as emoji. Long-press on the title opens Grown-up corner.
2. **Turn** — one activity. A progress dots row (10 dots) at the top.
   - **Read**: the word in large type; four emoji tiles below. Tap one.
   - **Pick**: one large emoji; three word tiles below. Tap one.
   - **Build**: one large emoji; empty letter slots the length of the word;
     a tray of shuffled letter tiles (the word's letters plus 1–2
     distractors). Tap tiles to fill slots in order; tap a filled slot to
     send its letter back. Auto-checks when all slots are full. A sight word
     has no picture, so its Build turn shows the word itself in large type
     for a moment (look), then hides it and reveals the empty slots and tray
     (spell) — Mae reads it before she has to spell it from memory.
   - **Sentence**: the frame in large type with the picture in the blank;
     three word tiles below. Tap one.
   - **Fill**: the frame in large type with the picture noun shown and the
     sight word as an empty blank; three sight-word tiles below. Tap one.
   - Correct first try: tile turns green, a star pops, brief confetti,
     auto-advance after ~900 ms.
   - Wrong: the tapped tile shakes, greys out and stays disabled; the other
     tiles stay live and Mae picks again until she hits the right one. The
     eventual right tile turns green with a small pop, no confetti, no star,
     and the turn auto-advances after ~900 ms. Its progress dot shows done
     but unstarred.
   - Wrong on Build: the wrong letters shake and slide back to the tray, the
     slots empty, and Mae rebuilds once. A second wrong build shows the
     correct spelling filled in and moves on after ~1500 ms.
   - Optional speaker button (top right) speaks the target word via
     `speechSynthesis` if present; hidden otherwise.
3. **Round end** — "10 stars!" style celebration, then the "Words I know"
   wall (learned words as emoji + word), and a big "Again" button.
4. **Grown-up corner** — list of every word and sight word with tier, level
   and a coloured status; "Start again" with a confirm step that requires
   tapping a written word (so Mae can't reset by accident); a link to
   `credits.html`; link back to Home.

No text instructions anywhere on the child-facing screens. Every tap
target is at least 64px. Portrait first; landscape just widens tiles.

## Architecture

Plain ES modules, no build step, no framework, no dependencies. Same
layout as Anne's Words so the two are maintained the same way.

```
docs-site/docs/maes-words/
  index.html    markup shell: screens as <section>s, one visible at a time
  style.css     pink theme, big friendly tiles, pop/shake/confetti keyframes
  words.js      data only: WORDS, SIGHT, FRAMES
  img/          vendored pictograms, written by the generator
  credits.html  picture attribution (adult-facing)
  game.js       pure logic, zero DOM — the unit under test
  storage.js    pure serialisation of state; caller owns localStorage
  ui.js         all DOM, localStorage, speechSynthesis; imports the rest
```

### `game.js` interface

All functions are pure. Randomness comes in as an `rng: () => number`
argument so tests can fix it.

```js
initialState(words)                 -> State   // introduces first 6 tier-1 words
introduceWords(state, words)        -> State   // trickle rule + tier unlock
planRound(state, words, rng)        -> Turn[]  // 10 turns
makeTurn(word, level, words, rng)   -> Turn    // activity + options/tiles
applyAnswer(state, turn, correct)   -> State   // level ±1, stars (first attempt only)
retryAllowed(turn, attempts)        -> bool    // tile activities: always; build: once
distractors(word, words, n)         -> Word[]  // look-alike ranking
letterTray(word, words, rng)        -> string[] // letters + safe distractors
unlockedTiers(state, words)         -> number
```

`Turn` is `{ word, activity, options }` where `options` is words (rendered
via their `pic`) for Read, words for Pick/Sentence (plus `frame` for
Sentence), sight words for Fill (plus `frame` and the filled `noun`), or a
letter array for Build. `word` is a `WORDS` entry or a `SIGHT` entry; the
activity is chosen by the word's kind and level. Whether an answer is correct is decided in `game.js`
(`isCorrect(turn, answer)`), not in the UI.

### Persistence

One key, `maes-words:v1`:

```jsonc
{
  "version": 1,
  "levels": { "cat": 2, "dog": 3, "the": 1 },   // introduced words and sight words
  "stars": 42,
  "learnedOrder": ["dog"]                        // for the "words I know" wall
}
```

`storage.js` mirrors Anne's Words: `load(raw)` repairs anything missing or
malformed and never throws; `save(state)` returns the JSON string. Unknown
words in `levels` (in neither `WORDS` nor `SIGHT`) are dropped on load. A
version mismatch resets to defaults.

## Publishing

Identical to Anne's Words: MkDocs copies non-markdown files under
`docs-site/docs/` verbatim, so `docs-site/docs/maes-words/index.html`
publishes to `/maes-words/` with no `nav` entry, no markdown page, and no
effect on `mkdocs build --strict`. The existing `Docs` workflow triggers on
`docs-site/**`, so a push to `main` publishes it with no CI change.

## Testing

`docs-site/tests/maes-words.*.test.mjs`, run with `node --test`, zero
dependencies, outside `docs/` so not published.

Covered:

- Word bank invariants: every word 2–4 lowercase letters, unique, has a
  tier in 1–3 and a `pic` whose `kind` is `emoji`, `svg` or `colour`; every
  `svg` `src` exists on disk under `img/`; emoji text and `src` are unique;
  every colour word's `pic.kind` is `colour` and `pos` is `adj`; no word is
  in both `WORDS` and `SIGHT`; every frame has exactly one `{noun}` and at
  most one `{sight}`, and every entry in `fits` is a `SIGHT` word; `pos` on a
  word or frame, where present, is `verb` or `adj`; every `pos` used by a
  word has at least one frame of that `pos`, so `framesFor` never comes back
  empty.
- Progression: level clamps at 0 and 3; trickle introduces 2 words only
  when fewer than 6 are at level 0; tier unlock at 75%; a round never
  repeats a word when the active set allows; at most one review word per
  round; weights favour low levels (statistical test with a fixed rng).
- Turn construction: activity matches kind and level; options always
  contain the answer exactly once; distractors never equal the answer; a
  Fill turn's distractors are never in its frame's `fits`; a Fill noun obeys
  the frame's article; colour-word Read options are all colours; Build tray
  contains every letter of the word and never admits a distractor that
  spells another bank word; `isCorrect` for each activity.
- Try again: a wrong-then-right turn changes the level by exactly −1 and
  adds no star; `retryAllowed` is unbounded for tile activities and allows
  one retry for Build.
- Storage: round-trips, repairs corrupt JSON, drops unknown words, resets
  on version mismatch.
- Pictogram generation: the vendored images and the generated blocks in
  `words.js`/`credits.html` are checked in, not built at test time, but the
  generator that produced them is idempotent — `generate-maes-pics.mjs
  --check` re-derives the blocks from its own word → source → asset-id table
  and exits non-zero if the checked-in bank has drifted from it, so the two
  never fall out of sync silently.

UI behaviour is verified by playing it in a browser at each checkpoint,
including on an iPad, and by a headless Playwright script
(`NODE_PATH=docs/demos/node_modules`) that exercises each activity,
including a wrong-then-right turn and a sight-word Build left mid-look-phase:
navigating away and starting a new round before the look timer fires must
not let the stale timer reveal or wipe the new turn's prompt.

## Checkpoints

Each is a commit on a `maes-words` branch, playable via
`cd docs-site/docs && python3 -m http.server 8000` at
`http://localhost:8000/maes-words/`.

1. Word bank + `game.js` progression and turn construction, fully tested.
2. Read and Pick activities playable end to end with persistence.
3. Build activity with the letter tray.
4. Sentence activity, round-end wall, Grown-up corner.
5. Polish — animations, speaker button, iPad check, `mkdocs build --strict`.
6. Generator script, vendored pictograms, `pic` rendering, credits page.
7. Colour tiles and the try-again mechanic.
8. Sight bank, frame templates and the Fill activity.
