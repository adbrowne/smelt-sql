# Mae's Words — Design

**Date:** 2026-09-27
**Status:** Draft, awaiting review

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
| Words | ~100 hand-curated 2–4 letter concrete nouns that an emoji shows unambiguously, in three difficulty tiers |
| Activities | Read (word → picture), Pick (picture → word among look-alikes), Build (picture → letter tiles), Sentence (sight-word sentence with a picture blank) |
| Progression | Per-word mastery ladder (0–3) driving which activity a word gets; new words trickle in; tiers unlock by mastery |
| Persistence | `localStorage`, one versioned key |
| Failure | None. A wrong answer shows the right one and moves on. No timers, no lives, no losing |
| Grown-up corner | Hidden screen (long-press the title) with per-word status and reset |
| Out of scope | Accounts, server, analytics, build step, npm dependencies, audio assets, multiple profiles, dark mode |

## Word bank

`words.js` exports a hand-curated array (not generated; ~100 entries):

```js
export const WORDS = [
  { w: 'ox',   e: '🐂', tier: 1 },
  { w: 'dog',  e: '🐶', tier: 1 },
  { w: 'cat',  e: '🐱', tier: 1 },
  { w: 'sun',  e: '☀️', tier: 1 },
  { w: 'frog', e: '🐸', tier: 2 },
  { w: 'ship', e: '🚢', tier: 3 },
  // ...
];
```

Rules for inclusion:

- 2–4 lowercase letters, a single concrete noun, no plurals.
- The emoji must be unambiguous to a five-year-old with no caption
  (🐶 is "dog", not "puppy"; ⚽ is "ball"; 🍰 is "cake"). Anything a child
  could reasonably name two ways is left out.
- Tier 1: CVC and shorter words with the most common letters
  (`cat dog sun bed cup hat pig bus egg bag pen ox`).
- Tier 2: 3–4 letter words with blends and common digraphs
  (`frog milk cake tree fish duck bell boat`).
- Tier 3: 4-letter words with trickier vowel patterns
  (`ship bird book moon star kite ring sock`).

Every emoji is checked to render on iOS Safari and Chrome on Android; the
bank is small enough to eyeball.

### Look-alike distractors

Pick and Build need plausible wrong answers. A build-time-free approach:
`distractors(word, bank)` returns words from the same bank ranked by edit
distance (`cat`→`cot`, `cut`, `can`), falling back to words of the same
length. Because distractors are real words from the bank, they are also
things Mae is learning — every wrong option she reads is still practice.
Letter-tile distractors for Build are letters at edit distance one from
the word's letters, never a letter that makes another bank word an
alternative correct answer.

### Sight words and sentences

Peter and Jane's key words (`the is a and here I like my to in on has`)
cannot be pictured, so they are never targets. They appear in the Sentence
activity as fixed frames with one picture blank:

```js
export const FRAMES = [
  'I like the {}',
  'Here is a {}',
  'The {} is here',
  'I can see a {}',
  'My {} is in the {}',  // two blanks: later
];
```

A Sentence turn shows the frame with the picture in the blank and Mae picks
the word. She reads the sight words as context, exactly as the books use
them.

## Progression model

Every word has a mastery level in `0..3`:

| Level | Meaning | Activity used |
|---|---|---|
| 0 | New / not yet recognised | Read |
| 1 | Recognises it | Pick |
| 2 | Tells it from look-alikes | Build |
| 3 | Learned | Sentence, occasionally, as review |

Rules:

- Correct answer: level `+1` (capped at 3). Wrong answer: level `−1`
  (floored at 0). A wrong Read answer stays at 0.
- **Active set**: words with level `< 3` that have been introduced. New
  words are introduced from the lowest unlocked tier, `2` at a time,
  whenever the active set has fewer than `6` words at level 0.
  Start of game: the first `6` tier-1 words are introduced.
- **Tier unlock**: tier `N+1` unlocks when at least 75% of tier `N` words
  are at level ≥ 2.
- **Round**: 10 turns. Each turn picks a word from the active set with
  weight favouring lower levels (`weight = 4 − level`), with at most one
  learned word (level 3) per round as review, chosen uniformly from
  learned words. The same word is never picked twice in a round unless the
  active set is smaller than 10.
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
     send its letter back. Auto-checks when all slots are full.
   - **Sentence**: the frame in large type with the emoji in the blank;
     three word tiles below. Tap one.
   - Correct: tile turns green, a star pops, brief confetti, auto-advance
     after ~900 ms.
   - Wrong: tapped tile shakes and greys out, the correct tile highlights
     green, auto-advance after ~1500 ms. On Build, wrong letters slide back
     to the tray and the correct spelling is shown filled in.
   - Optional speaker button (top right) speaks the target word via
     `speechSynthesis` if present; hidden otherwise.
3. **Round end** — "10 stars!" style celebration, then the "Words I know"
   wall (learned words as emoji + word), and a big "Again" button.
4. **Grown-up corner** — list of every word with tier, level and a
   coloured status; "Start again" with a confirm step that requires
   tapping a written word (so Mae can't reset by accident); link back to
   Home.

No text instructions anywhere on the child-facing screens. Every tap
target is at least 64px. Portrait first; landscape just widens tiles.

## Architecture

Plain ES modules, no build step, no framework, no dependencies. Same
layout as Anne's Words so the two are maintained the same way.

```
docs-site/docs/maes-words/
  index.html    markup shell: screens as <section>s, one visible at a time
  style.css     big friendly tiles, pop/shake/confetti keyframes
  words.js      data only: WORDS, FRAMES
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
applyAnswer(state, turn, correct)   -> State   // level ±1, stars
distractors(word, words, n)         -> Word[]  // look-alike ranking
letterTray(word, words, rng)        -> string[] // letters + safe distractors
unlockedTiers(state, words)         -> number
```

`Turn` is `{ word, activity, options }` where `options` is emoji for Read,
words for Pick/Sentence (plus `frame` for Sentence), or a letter array for
Build. Whether an answer is correct is decided in `game.js`
(`isCorrect(turn, answer)`), not in the UI.

### Persistence

One key, `maes-words:v1`:

```jsonc
{
  "version": 1,
  "levels": { "cat": 2, "dog": 3, "sun": 0 },   // introduced words only
  "stars": 42,
  "learnedOrder": ["dog"]                        // for the "words I know" wall
}
```

`storage.js` mirrors Anne's Words: `load(raw)` repairs anything missing or
malformed and never throws; `save(state)` returns the JSON string. Unknown
words in `levels` (removed from the bank later) are dropped on load. A
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

- Word bank invariants: every word 2–4 lowercase letters, unique, has an
  emoji and a tier in 1–3; every frame has exactly one `{}` (v1).
- Progression: level clamps at 0 and 3; trickle introduces 2 words only
  when fewer than 6 are at level 0; tier unlock at 75%; a round never
  repeats a word when the active set allows; at most one review word per
  round; weights favour low levels (statistical test with a fixed rng).
- Turn construction: activity matches level; options always contain the
  answer exactly once; distractors never equal the answer; Build tray
  contains every letter of the word and never admits a distractor that
  spells another bank word; `isCorrect` for each activity.
- Storage: round-trips, repairs corrupt JSON, drops unknown words, resets
  on version mismatch.

UI behaviour is verified by playing it in a browser at each checkpoint,
including on an iPad.

## Checkpoints

Each is a commit on a `maes-words` branch, playable via
`cd docs-site/docs && python3 -m http.server 8000` at
`http://localhost:8000/maes-words/`.

1. Word bank + `game.js` progression and turn construction, fully tested.
2. Read and Pick activities playable end to end with persistence.
3. Build activity with the letter tray.
4. Sentence activity, round-end wall, Grown-up corner.
5. Polish — animations, speaker button, iPad check, `mkdocs build --strict`.
