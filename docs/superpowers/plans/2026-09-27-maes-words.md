# Mae's Words Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship "Mae's Words", a silent touch-first reading game for a five-year-old learning 2–4 letter words, as a dependency-free page published at `https://smeltsql.com/maes-words/`.

**Architecture:** Plain ES modules under `docs-site/docs/maes-words/`, copied verbatim into the published site by MkDocs (exactly like the existing `annes-words/` sibling). Pure logic lives in `game.js` and `storage.js` with zero DOM/clock/random access (randomness is injected as `rng`) so they run under `node --test`; everything that touches `document`, `localStorage`, timers, or `speechSynthesis` lives in `ui.js`.

**Tech Stack:** Vanilla HTML/CSS/ES modules. No build step, no npm, no framework, no runtime dependencies, no image assets (emoji are the pictures). Tests via node's built-in `node --test`.

**Spec:** `docs/superpowers/specs/2026-09-27-maes-words-design.md`

## Global Constraints

- Branch: `maes-words`, created from `main`. Commit after every task. Do **not** push to `main` until the final task: a push to `main` touching `docs-site/**` deploys the site.
- Zero dependencies. No npm, no CDN scripts, no build step, no image files. Anything the page needs is a file in `docs-site/docs/maes-words/`.
- The name is **Mae's Words** everywhere user-visible.
- Silent by design: nothing may depend on audio. The speaker button is optional and hidden when `window.speechSynthesis` is absent.
- No text instructions on child-facing screens. Every tap target is at least 64px tall and wide.
- Theme: pink. Exact tokens: primary `#e91e8c`, primary-dark `#b8126b`, page background `#fff0f6`, soft pink `#ffd6e7`, tile `#ffffff`, text `#3a2a33`, correct `#4caf50`, wrong `#b0b0b0`, star `#ffc107`.
- Word bank rules: 2–4 lowercase letters, single concrete noun, no plurals, emoji unambiguous with no caption, tiers 1–3, unique words, unique emoji.
- Progression constants (from the spec): `MAX_LEVEL = 3`, `ROUND_LENGTH = 10`, `INITIAL_INTRO = 6`, `INTRO_BATCH = 2`, `NEW_WORD_FLOOR = 6`, `UNLOCK_FRACTION = 0.75`, weight `4 - level`, at most one level-3 review word per round.
- This work touches no Rust crate. Do not run `verify-phase.sh`, `cargo` anything, or update `docs/ROADMAP.md`. The repo's spec/plan workflow is waived for this feature.
- Tests live in `docs-site/tests/` — never under `docs-site/docs/`, which is published.
- Run tests with: `node --test 'docs-site/tests/maes-words.*.test.mjs'` (quote the glob — on Node 24 a bare directory argument is resolved as a module and errors). Node is v24 on this machine.
- Manual check at every UI task: `cd docs-site/docs && python3 -m http.server 8000`, open `http://localhost:8000/maes-words/`.
- Before the final push: `cd docs-site && uv run mkdocs build --strict` must pass.

## Review Focus

Inputs the spec implies but no task's tests would otherwise exercise, most likely to bite first. Each has its pinning test added to the owning task below.

1. **`localStorage` unavailable or throwing** (iOS private mode, storage full): the game must still play a full round with in-memory state and never throw. → Task 5, `readSave`/`writeSave` wrap every access in try/catch; manual check in private mode.
2. **A word removed from the bank after progress was saved**: `load` must drop it silently, not crash on a `levels` key with no matching word. → Task 4 test `drops words no longer in the bank`.
3. **Active set smaller than a round** (start of game there are only 6 words): `planRound` must still return exactly 10 turns, repeating words as needed, never returning fewer. → Task 3 test `fills a round from a small active set`.
4. **Double-tap during the advance delay**: a second tap while the correct/wrong animation runs must not answer the next turn or double-count a star. → Task 5 `busy` guard; Task 3 `applyAnswer` is pure so a duplicate call is only possible from the UI.
5. **Every word at level 3 with nothing left to introduce** (bank exhausted): `planRound` must still produce a round (all review) rather than an empty list. → Task 3 test `plays reviews when everything is learned`.

---

### Task 1: Word bank data module

**Files:**
- Create: `docs-site/docs/maes-words/words.js`
- Create: `docs-site/tests/maes-words.words.test.mjs`

**Interfaces:**
- Consumes: nothing.
- Produces: `export const WORDS` — array of `{ w: string, e: string, tier: 1|2|3 }`, in the order words are introduced within a tier. `export const FRAMES` — array of sentence strings each containing exactly one `{}`.

- [ ] **Step 1: Write the failing test**

```js
// docs-site/tests/maes-words.words.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { WORDS, FRAMES } from '../docs/maes-words/words.js';

test('every word is 2-4 lowercase letters with an emoji and a tier', () => {
  for (const { w, e, tier } of WORDS) {
    assert.match(w, /^[a-z]{2,4}$/, `bad word ${w}`);
    assert.ok(typeof e === 'string' && e.length > 0, `no emoji for ${w}`);
    assert.ok([1, 2, 3].includes(tier), `bad tier for ${w}`);
  }
});

test('words and emoji are unique', () => {
  const ws = WORDS.map(x => x.w), es = WORDS.map(x => x.e);
  assert.equal(new Set(ws).size, ws.length, 'duplicate word');
  assert.equal(new Set(es).size, es.length, 'duplicate emoji');
});

test('bank is big enough for the progression rules', () => {
  assert.ok(WORDS.length >= 80, `only ${WORDS.length} words`);
  assert.ok(WORDS.filter(x => x.tier === 1).length >= 20, 'tier 1 too small');
  assert.ok(WORDS.filter(x => x.tier === 2).length >= 20, 'tier 2 too small');
  assert.ok(WORDS.filter(x => x.tier === 3).length >= 20, 'tier 3 too small');
});

test('no plurals', () => {
  for (const { w } of WORDS) assert.ok(!/[^s]s$/.test(w) || ['bus', 'gas'].includes(w), `plural? ${w}`);
});

test('every frame has exactly one blank', () => {
  assert.ok(FRAMES.length >= 4);
  for (const f of FRAMES) assert.equal((f.match(/\{\}/g) || []).length, 1, `bad frame ${f}`);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'`
Expected: FAIL — cannot find module `../docs/maes-words/words.js`.

- [ ] **Step 3: Write the word bank**

Hand-curated. Keep the rule in mind for every entry: a five-year-old, shown only the emoji, would say exactly this word. Use this list as the starting point; drop any entry you believe is ambiguous, add none that is. Do not exceed one emoji per word or reuse an emoji.

```js
// docs-site/docs/maes-words/words.js
// Hand-curated. Rule: shown only the emoji, a five-year-old says exactly this word.
// Tier 1: short CVC / common letters. Tier 2: blends and ck/ll/ss. Tier 3: vowel
// digraphs, magic-e, sh/th/ng, r-controlled vowels.
export const WORDS = [
  // tier 1
  { w: 'cat', e: '🐱', tier: 1 },
  { w: 'dog', e: '🐶', tier: 1 },
  { w: 'sun', e: '☀️', tier: 1 },
  { w: 'bed', e: '🛏️', tier: 1 },
  { w: 'pig', e: '🐷', tier: 1 },
  { w: 'bus', e: '🚌', tier: 1 },
  { w: 'egg', e: '🥚', tier: 1 },
  { w: 'hat', e: '🎩', tier: 1 },
  { w: 'cow', e: '🐮', tier: 1 },
  { w: 'bee', e: '🐝', tier: 1 },
  { w: 'ant', e: '🐜', tier: 1 },
  { w: 'fox', e: '🦊', tier: 1 },
  { w: 'hen', e: '🐔', tier: 1 },
  { w: 'bat', e: '🦇', tier: 1 },
  { w: 'rat', e: '🐀', tier: 1 },
  { w: 'car', e: '🚗', tier: 1 },
  { w: 'key', e: '🔑', tier: 1 },
  { w: 'box', e: '📦', tier: 1 },
  { w: 'bug', e: '🐛', tier: 1 },
  { w: 'pen', e: '🖊️', tier: 1 },
  { w: 'bag', e: '👜', tier: 1 },
  { w: 'cap', e: '🧢', tier: 1 },
  { w: 'van', e: '🚐', tier: 1 },
  { w: 'owl', e: '🦉', tier: 1 },
  { w: 'pie', e: '🥧', tier: 1 },
  { w: 'eye', e: '👁️', tier: 1 },
  { w: 'ear', e: '👂', tier: 1 },
  { w: 'leg', e: '🦵', tier: 1 },
  { w: 'ox',  e: '🐂', tier: 1 },
  { w: 'nut', e: '🥜', tier: 1 },
  { w: 'log', e: '🪵', tier: 1 },
  { w: 'web', e: '🕸️', tier: 1 },
  { w: 'jar', e: '🫙', tier: 1 },
  { w: 'gem', e: '💎', tier: 1 },
  { w: 'saw', e: '🪚', tier: 1 },
  { w: 'axe', e: '🪓', tier: 1 },
  { w: 'pin', e: '📌', tier: 1 },
  { w: 'map', e: '🗺️', tier: 1 },
  { w: 'ice', e: '🧊', tier: 1 },
  { w: 'one', e: '1️⃣', tier: 1 },
  { w: 'two', e: '2️⃣', tier: 1 },
  { w: 'six', e: '6️⃣', tier: 1 },
  { w: 'ten', e: '🔟', tier: 1 },
  // tier 2
  { w: 'frog', e: '🐸', tier: 2 },
  { w: 'milk', e: '🥛', tier: 2 },
  { w: 'duck', e: '🦆', tier: 2 },
  { w: 'bell', e: '🔔', tier: 2 },
  { w: 'ball', e: '⚽', tier: 2 },
  { w: 'drum', e: '🥁', tier: 2 },
  { w: 'crab', e: '🦀', tier: 2 },
  { w: 'hand', e: '✋', tier: 2 },
  { w: 'sock', e: '🧦', tier: 2 },
  { w: 'nest', e: '🪺', tier: 2 },
  { w: 'tent', e: '⛺', tier: 2 },
  { w: 'gift', e: '🎁', tier: 2 },
  { w: 'flag', e: '🚩', tier: 2 },
  { w: 'lamp', e: '🪔', tier: 2 },
  { w: 'corn', e: '🌽', tier: 2 },
  { w: 'wolf', e: '🐺', tier: 2 },
  { w: 'bath', e: '🛁', tier: 2 },
  { w: 'fish', e: '🐟', tier: 2 },
  { w: 'ship', e: '🚢', tier: 2 },
  { w: 'shoe', e: '👟', tier: 2 },
  { w: 'ring', e: '💍', tier: 2 },
  { w: 'king', e: '🤴', tier: 2 },
  { w: 'worm', e: '🪱', tier: 2 },
  { w: 'bird', e: '🐦', tier: 2 },
  { w: 'star', e: '⭐', tier: 2 },
  { w: 'fire', e: '🔥', tier: 2 },
  { w: 'four', e: '4️⃣', tier: 2 },
  { w: 'five', e: '5️⃣', tier: 2 },
  { w: 'nine', e: '9️⃣', tier: 2 },
  // tier 3
  { w: 'cake', e: '🍰', tier: 3 },
  { w: 'tree', e: '🌳', tier: 3 },
  { w: 'boat', e: '⛵', tier: 3 },
  { w: 'bear', e: '🐻', tier: 3 },
  { w: 'lion', e: '🦁', tier: 3 },
  { w: 'rain', e: '🌧️', tier: 3 },
  { w: 'leaf', e: '🍃', tier: 3 },
  { w: 'foot', e: '🦶', tier: 3 },
  { w: 'nose', e: '👃', tier: 3 },
  { w: 'door', e: '🚪', tier: 3 },
  { w: 'bone', e: '🦴', tier: 3 },
  { w: 'rose', e: '🌹', tier: 3 },
  { w: 'kite', e: '🪁', tier: 3 },
  { w: 'bike', e: '🚲', tier: 3 },
  { w: 'moon', e: '🌙', tier: 3 },
  { w: 'book', e: '📖', tier: 3 },
  { w: 'goat', e: '🐐', tier: 3 },
  { w: 'deer', e: '🦌', tier: 3 },
  { w: 'seal', e: '🦭', tier: 3 },
  { w: 'pear', e: '🍐', tier: 3 },
  { w: 'rice', e: '🍚', tier: 3 },
  { w: 'coin', e: '🪙', tier: 3 },
  { w: 'dice', e: '🎲', tier: 3 },
  { w: 'wand', e: '🪄', tier: 3 },
  { w: 'toad', e: '🐢', tier: 3 },  // NOTE: 🐢 is a turtle — drop this line; kept here as an example of what NOT to include
];

export const FRAMES = [
  'I like the {}',
  'Here is a {}',
  'The {} is here',
  'I can see a {}',
  'Look at the {}',
  'I have a {}',
];
```

Remove the `toad` line before committing (it is there to make the ambiguity rule concrete). Add a few more tier-3 entries if the tier-3 count drops below 20 after your own pruning; good candidates: `fork 🍴`, `vase 🏺` (only if you're sure a child says vase, not pot — probably not), `pool 🏊`? (no — swimmer), `hook 🪝`, `bowl 🥣`, `bean 🫘`, `pizza` (too long).

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'`
Expected: 5 pass.

- [ ] **Step 5: Commit**

```bash
git add docs-site/docs/maes-words/words.js docs-site/tests/maes-words.words.test.mjs
git commit -m "feat(maes-words): hand-curated word bank and sentence frames"
```

---

### Task 2: Progression logic in `game.js`

**Files:**
- Create: `docs-site/docs/maes-words/game.js`
- Create: `docs-site/tests/maes-words.game.test.mjs`

**Interfaces:**
- Consumes: `WORDS` shape from Task 1 (`{w, e, tier}`), but tests pass their own small banks.
- Produces (all pure, never mutate inputs):
  - constants `MAX_LEVEL`, `ROUND_LENGTH`, `INITIAL_INTRO`, `INTRO_BATCH`, `NEW_WORD_FLOOR`, `UNLOCK_FRACTION`, `ACTIVITY_FOR_LEVEL` (`['read','pick','build','sentence']`)
  - `emptyState() -> { levels: {}, stars: 0, learnedOrder: [] }`
  - `initialState(words) -> State` — first `INITIAL_INTRO` tier-1 words at level 0
  - `unlockedTier(state, words) -> number` — highest tier allowed to introduce from
  - `introduceWords(state, words) -> State` — one batch if level-0 count below floor
  - `applyAnswer(state, word, correct) -> State` — `word` is a bank entry
  - `activeWords(state, words) -> Word[]`, `learnedWords(state, words) -> Word[]`

- [ ] **Step 1: Write the failing tests**

```js
// docs-site/tests/maes-words.game.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, initialState, unlockedTier, introduceWords, applyAnswer,
         activeWords, learnedWords, INITIAL_INTRO, INTRO_BATCH, NEW_WORD_FLOOR,
         MAX_LEVEL } from '../docs/maes-words/game.js';

// A small bank: 8 tier-1, 4 tier-2, 4 tier-3.
const mk = (w, tier) => ({ w, e: w.toUpperCase(), tier });
const BANK = [
  ...['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat'].map(w => mk(w, 1)),
  ...['frog', 'milk', 'duck', 'bell'].map(w => mk(w, 2)),
  ...['cake', 'tree', 'boat', 'bear'].map(w => mk(w, 3)),
];
const byName = w => BANK.find(x => x.w === w);
const withLevels = levels => ({ ...emptyState(), levels: { ...levels } });

test('initialState introduces the first INITIAL_INTRO tier-1 words at level 0', () => {
  const s = initialState(BANK);
  assert.deepEqual(Object.keys(s.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus']);
  assert.ok(Object.values(s.levels).every(l => l === 0));
  assert.equal(s.stars, 0);
  assert.equal(INITIAL_INTRO, 6);
});

test('applyAnswer moves a word up on correct and down on wrong, clamped', () => {
  let s = withLevels({ cat: 0 });
  s = applyAnswer(s, byName('cat'), false);
  assert.equal(s.levels.cat, 0, 'floor at 0');
  s = applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 1);
  s = applyAnswer(s, byName('cat'), true);
  s = applyAnswer(s, byName('cat'), true);
  s = applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, MAX_LEVEL, 'cap at MAX_LEVEL');
  assert.equal(s.stars, 4);
  s = applyAnswer(s, byName('cat'), false);
  assert.equal(s.levels.cat, 2);
});

test('applyAnswer does not mutate its input', () => {
  const s = withLevels({ cat: 1 });
  applyAnswer(s, byName('cat'), true);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 0);
});

test('reaching level 3 appends to learnedOrder once', () => {
  let s = withLevels({ cat: 2, dog: 2 });
  s = applyAnswer(s, byName('dog'), true);
  s = applyAnswer(s, byName('cat'), true);
  assert.deepEqual(s.learnedOrder, ['dog', 'cat']);
  s = applyAnswer(s, byName('dog'), false);
  s = applyAnswer(s, byName('dog'), true);
  assert.deepEqual(s.learnedOrder, ['dog', 'cat'], 'no duplicate on re-learn');
});

test('unlockedTier: tier 1 always; tier 2 once 75% of tier 1 is at level >= 2', () => {
  assert.equal(unlockedTier(emptyState(), BANK), 1);
  // 5 of 8 tier-1 at >=2 is 62.5%: not enough
  let s = withLevels({ cat: 2, dog: 3, sun: 2, bed: 2, pig: 2, bus: 1, egg: 0 });
  assert.equal(unlockedTier(s, BANK), 1);
  // 6 of 8 is 75%: unlocked
  s = withLevels({ cat: 2, dog: 3, sun: 2, bed: 2, pig: 2, bus: 2, egg: 0 });
  assert.equal(unlockedTier(s, BANK), 2);
  // tier 3 needs tier 2 as well (3 of 4 = 75%)
  s = withLevels({ ...s.levels, frog: 2, milk: 3, duck: 2, bell: 0 });
  assert.equal(unlockedTier(s, BANK), 3);
});

test('unlockedTier never exceeds the highest tier in the bank', () => {
  const s = withLevels(Object.fromEntries(BANK.map(x => [x.w, 3])));
  assert.equal(unlockedTier(s, BANK), 3);
});

test('introduceWords adds INTRO_BATCH words only when fewer than NEW_WORD_FLOOR are at level 0', () => {
  assert.equal(INTRO_BATCH, 2);
  assert.equal(NEW_WORD_FLOOR, 6);
  // six at level 0: nothing happens
  let s = withLevels({ cat: 0, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0 });
  assert.deepEqual(introduceWords(s, BANK), s);
  // five at level 0: two more come in, in bank order
  s = withLevels({ cat: 1, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0 });
  const next = introduceWords(s, BANK);
  assert.deepEqual(Object.keys(next.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat']);
  assert.equal(next.levels.egg, 0);
  assert.equal(next.levels.hat, 0);
});

test('introduceWords respects the unlocked tier', () => {
  // all tier 1 introduced and mastered except none at level 0; tier 2 locked (0% of tier 1 at >=2)
  const s = withLevels({ cat: 1, dog: 1, sun: 1, bed: 1, pig: 1, bus: 1, egg: 1, hat: 1 });
  assert.deepEqual(introduceWords(s, BANK), s, 'nothing to introduce while tier 2 is locked');
  const t = withLevels({ cat: 2, dog: 2, sun: 2, bed: 2, pig: 2, bus: 2, egg: 1, hat: 1 });
  const next = introduceWords(t, BANK);
  assert.equal(next.levels.frog, 0);
  assert.equal(next.levels.milk, 0);
  assert.equal(next.levels.duck, undefined);
});

test('introduceWords introduces a single word when only one is left', () => {
  const all = Object.fromEntries(BANK.map(x => [x.w, 3]));
  delete all.bear;
  const next = introduceWords(withLevels(all), BANK);
  assert.equal(next.levels.bear, 0);
});

test('activeWords and learnedWords split introduced words by level, learned in learnedOrder', () => {
  const s = { ...withLevels({ cat: 3, dog: 1, sun: 3, bed: 0 }), learnedOrder: ['sun', 'cat'] };
  assert.deepEqual(activeWords(s, BANK).map(x => x.w), ['dog', 'bed']);
  assert.deepEqual(learnedWords(s, BANK).map(x => x.w), ['sun', 'cat']);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test 'docs-site/tests/maes-words.game.test.mjs'`
Expected: FAIL — cannot find module `../docs/maes-words/game.js`.

- [ ] **Step 3: Write the implementation**

```js
// docs-site/docs/maes-words/game.js
// Pure logic only. No DOM, no clock, no Math.random: randomness is passed in as
// `rng: () => number in [0,1)` so tests can pin it.

export const MAX_LEVEL = 3;
export const ROUND_LENGTH = 10;
export const INITIAL_INTRO = 6;
export const INTRO_BATCH = 2;
export const NEW_WORD_FLOOR = 6;
export const UNLOCK_FRACTION = 0.75;
export const ACTIVITY_FOR_LEVEL = Object.freeze(['read', 'pick', 'build', 'sentence']);

export const emptyState = () => ({ levels: {}, stars: 0, learnedOrder: [] });

const cloneState = s => ({ levels: { ...s.levels }, stars: s.stars, learnedOrder: [...s.learnedOrder] });
const isIntroduced = (state, w) => Object.prototype.hasOwnProperty.call(state.levels, w);

export function initialState(words) {
  const s = emptyState();
  for (const word of words.filter(x => x.tier === 1).slice(0, INITIAL_INTRO)) s.levels[word.w] = 0;
  return s;
}

/** Highest tier new words may be introduced from. Tier 1 is always open. */
export function unlockedTier(state, words) {
  const maxTier = Math.max(1, ...words.map(x => x.tier));
  let tier = 1;
  while (tier < maxTier) {
    const inTier = words.filter(x => x.tier === tier);
    const mastered = inTier.filter(x => (state.levels[x.w] ?? 0) >= 2 && isIntroduced(state, x.w)).length;
    if (inTier.length === 0 || mastered / inTier.length < UNLOCK_FRACTION) break;
    tier += 1;
  }
  return tier;
}

/** One trickle step: if fewer than NEW_WORD_FLOOR introduced words sit at level 0, bring in up to INTRO_BATCH more. */
export function introduceWords(state, words) {
  const atZero = Object.values(state.levels).filter(l => l === 0).length;
  if (atZero >= NEW_WORD_FLOOR) return state;
  const tier = unlockedTier(state, words);
  const fresh = words.filter(x => x.tier <= tier && !isIntroduced(state, x.w)).slice(0, INTRO_BATCH);
  if (fresh.length === 0) return state;
  const next = cloneState(state);
  for (const word of fresh) next.levels[word.w] = 0;
  return next;
}

export function applyAnswer(state, word, correct) {
  const next = cloneState(state);
  const cur = next.levels[word.w] ?? 0;
  const level = correct ? Math.min(MAX_LEVEL, cur + 1) : Math.max(0, cur - 1);
  next.levels[word.w] = level;
  if (correct) next.stars += 1;
  if (level === MAX_LEVEL && !next.learnedOrder.includes(word.w)) next.learnedOrder.push(word.w);
  return next;
}

export const activeWords = (state, words) =>
  words.filter(x => isIntroduced(state, x.w) && state.levels[x.w] < MAX_LEVEL);

export function learnedWords(state, words) {
  const byName = new Map(words.map(x => [x.w, x]));
  const ordered = state.learnedOrder.map(w => byName.get(w)).filter(x => x && state.levels[x.w] === MAX_LEVEL);
  const seen = new Set(ordered.map(x => x.w));
  const rest = words.filter(x => state.levels[x.w] === MAX_LEVEL && !seen.has(x.w));
  return [...ordered, ...rest];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'`
Expected: all pass (5 from Task 1 + 10 here).

- [ ] **Step 5: Commit**

```bash
git add docs-site/docs/maes-words/game.js docs-site/tests/maes-words.game.test.mjs
git commit -m "feat(maes-words): mastery ladder, tier unlock, and word trickle"
```

---

### Task 3: Turn construction in `game.js`

**Files:**
- Modify: `docs-site/docs/maes-words/game.js` (append)
- Create: `docs-site/tests/maes-words.turns.test.mjs`

**Interfaces:**
- Consumes: Task 2 exports; `FRAMES` shape from Task 1 (passed in as an argument).
- Produces:
  - `shuffle(arr, rng) -> newArray`
  - `editDistance(a, b) -> number`
  - `distractors(word, words, n) -> Word[]` (never includes `word`, length exactly `n` when bank allows)
  - `letterTray(word, words, rng) -> string[]` (word's letters + 1 distractor for ≤3 letters, 2 for 4, shuffled)
  - `makeTurn(word, level, words, frames, rng) -> Turn`
  - `isCorrect(turn, answer) -> boolean` — `answer` is a word string for read/pick/sentence, a letter array for build
  - `planRound(state, words, frames, rng) -> Turn[]` (length `ROUND_LENGTH`)
  - `Turn` = `{ activity, word, options?: Word[], frame?: string, tray?: string[] }`; `options` always contains `word` exactly once.

- [ ] **Step 1: Write the failing tests**

```js
// docs-site/tests/maes-words.turns.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, shuffle, editDistance, distractors, letterTray, makeTurn, isCorrect,
         planRound, ROUND_LENGTH, ACTIVITY_FOR_LEVEL } from '../docs/maes-words/game.js';

const mk = (w, tier) => ({ w, e: w.toUpperCase(), tier });
const BANK = [
  ...['cat', 'cot', 'cut', 'dog', 'dig', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat', 'ox'].map(w => mk(w, 1)),
  ...['frog', 'milk', 'duck', 'bell'].map(w => mk(w, 2)),
];
const FRAMES = ['I like the {}', 'Here is a {}'];
const byName = w => BANK.find(x => x.w === w);
const withLevels = levels => ({ ...emptyState(), levels: { ...levels } });

// mulberry32: deterministic rng for tests
const seeded = seed => () => { seed |= 0; seed = seed + 0x6D2B79F5 | 0;
  let t = Math.imul(seed ^ seed >>> 15, 1 | seed); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
  return ((t ^ t >>> 14) >>> 0) / 4294967296; };

test('shuffle returns a permutation and does not mutate', () => {
  const a = [1, 2, 3, 4, 5];
  const b = shuffle(a, seeded(1));
  assert.deepEqual(a, [1, 2, 3, 4, 5]);
  assert.deepEqual([...b].sort(), [1, 2, 3, 4, 5]);
});

test('editDistance', () => {
  assert.equal(editDistance('cat', 'cat'), 0);
  assert.equal(editDistance('cat', 'cot'), 1);
  assert.equal(editDistance('cat', 'cart'), 1);
  assert.equal(editDistance('cat', 'dog'), 3);
});

test('distractors prefer look-alikes and never include the word', () => {
  const d = distractors(byName('cat'), BANK, 3).map(x => x.w);
  assert.equal(d.length, 3);
  assert.ok(!d.includes('cat'));
  assert.ok(d.includes('cot') && d.includes('cut'), `expected cot and cut in ${d}`);
});

test('distractors are unique and capped by the bank size', () => {
  const tiny = [mk('cat', 1), mk('dog', 1)];
  assert.deepEqual(distractors(tiny[0], tiny, 3).map(x => x.w), ['dog']);
});

test('letterTray holds every letter of the word plus 1 (short) or 2 (4-letter) distractors', () => {
  const rng = seeded(7);
  const t3 = letterTray(byName('sun'), BANK, rng);
  assert.equal(t3.length, 4);
  for (const ch of 'sun') assert.ok(t3.includes(ch));
  const t4 = letterTray(byName('frog'), BANK, rng);
  assert.equal(t4.length, 6);
  for (const ch of 'frog') assert.ok(t4.includes(ch));
});

test('letterTray never admits a distractor that spells another bank word', () => {
  // cat + o could spell cot; cat + u could spell cut. Over many seeds, never.
  for (let seed = 1; seed < 200; seed++) {
    const tray = letterTray(byName('cat'), BANK, seeded(seed));
    const extra = tray.filter(ch => !'cat'.includes(ch));
    assert.equal(extra.length, 1);
    assert.ok(!['o', 'u'].includes(extra[0]), `seed ${seed} admitted ${extra[0]}`);
  }
});

test('letterTray keeps duplicate letters of the word', () => {
  const tray = letterTray(byName('egg'), BANK, seeded(3));
  assert.equal(tray.filter(ch => ch === 'g').length, 2);
});

test('makeTurn picks the activity from the level and always includes the answer once', () => {
  const rng = seeded(11);
  const read = makeTurn(byName('cat'), 0, BANK, FRAMES, rng);
  assert.equal(read.activity, 'read');
  assert.equal(read.options.length, 4);
  assert.equal(read.options.filter(x => x.w === 'cat').length, 1);

  const pick = makeTurn(byName('cat'), 1, BANK, FRAMES, rng);
  assert.equal(pick.activity, 'pick');
  assert.equal(pick.options.length, 3);
  assert.equal(pick.options.filter(x => x.w === 'cat').length, 1);

  const build = makeTurn(byName('cat'), 2, BANK, FRAMES, rng);
  assert.equal(build.activity, 'build');
  assert.equal(build.tray.length, 4);

  const sent = makeTurn(byName('cat'), 3, BANK, FRAMES, rng);
  assert.equal(sent.activity, 'sentence');
  assert.ok(FRAMES.includes(sent.frame));
  assert.equal(sent.options.length, 3);
  assert.equal(sent.options.filter(x => x.w === 'cat').length, 1);
  assert.deepEqual(ACTIVITY_FOR_LEVEL, ['read', 'pick', 'build', 'sentence']);
});

test('makeTurn shuffles options so the answer is not always first', () => {
  const positions = new Set();
  for (let seed = 1; seed < 50; seed++) {
    const t = makeTurn(byName('cat'), 0, BANK, FRAMES, seeded(seed));
    positions.add(t.options.findIndex(x => x.w === 'cat'));
  }
  assert.ok(positions.size > 1);
});

test('isCorrect per activity', () => {
  const rng = seeded(5);
  const read = makeTurn(byName('cat'), 0, BANK, FRAMES, rng);
  assert.equal(isCorrect(read, 'cat'), true);
  assert.equal(isCorrect(read, 'cot'), false);
  const build = makeTurn(byName('cat'), 2, BANK, FRAMES, rng);
  assert.equal(isCorrect(build, ['c', 'a', 't']), true);
  assert.equal(isCorrect(build, ['c', 't', 'a']), false);
  assert.equal(isCorrect(build, ['c', 'a']), false);
});

test('planRound returns ROUND_LENGTH turns with no repeats when the active set is large enough', () => {
  const levels = Object.fromEntries(BANK.map(x => [x.w, 1]));
  const turns = planRound(withLevels(levels), BANK, FRAMES, seeded(2));
  assert.equal(turns.length, ROUND_LENGTH);
  assert.equal(new Set(turns.map(t => t.word.w)).size, ROUND_LENGTH);
});

test('fills a round from a small active set by repeating words', () => {
  const turns = planRound(withLevels({ cat: 0, dog: 0, sun: 1 }), BANK, FRAMES, seeded(4));
  assert.equal(turns.length, ROUND_LENGTH);
  assert.ok(turns.every(t => ['cat', 'dog', 'sun'].includes(t.word.w)));
});

test('a round includes at most one review (level 3) word', () => {
  const levels = { cat: 3, cot: 3, cut: 3, dog: 3, dig: 1, sun: 1, bed: 1, pig: 1, bus: 1, egg: 1, hat: 1, ox: 1 };
  for (let seed = 1; seed < 40; seed++) {
    const turns = planRound(withLevels(levels), BANK, FRAMES, seeded(seed));
    assert.equal(turns.length, ROUND_LENGTH);
    const reviews = turns.filter(t => t.activity === 'sentence');
    assert.ok(reviews.length <= 1, `seed ${seed}: ${reviews.length} reviews`);
  }
});

test('plays reviews when everything is learned', () => {
  const levels = Object.fromEntries(BANK.map(x => [x.w, 3]));
  const turns = planRound(withLevels(levels), BANK, FRAMES, seeded(9));
  assert.equal(turns.length, ROUND_LENGTH);
  assert.ok(turns.every(t => t.activity === 'sentence'));
});

test('planRound weights lower levels more heavily', () => {
  const levels = { cat: 0, dog: 2 };
  let catCount = 0, total = 0;
  for (let seed = 1; seed < 200; seed++) {
    for (const t of planRound(withLevels(levels), BANK, FRAMES, seeded(seed))) { total++; if (t.word.w === 'cat') catCount++; }
  }
  // weights 4 vs 2 => cat ~ 2/3 of picks
  assert.ok(catCount / total > 0.55, `cat share ${catCount / total}`);
});

test('planRound returns an empty list for an empty state', () => {
  assert.deepEqual(planRound(emptyState(), BANK, FRAMES, seeded(1)), []);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test 'docs-site/tests/maes-words.turns.test.mjs'`
Expected: FAIL — `shuffle` is not exported.

- [ ] **Step 3: Append the implementation to `game.js`**

```js
// --- turn construction -------------------------------------------------------

export function shuffle(arr, rng) {
  const out = [...arr];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

export function editDistance(a, b) {
  const dp = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    }
  }
  return dp[a.length][b.length];
}

/** Look-alike words from the bank, closest first, ties by length difference then bank order. */
export function distractors(word, words, n) {
  return words
    .map((x, i) => ({ x, i }))
    .filter(({ x }) => x.w !== word.w)
    .sort((p, q) =>
      editDistance(p.x.w, word.w) - editDistance(q.x.w, word.w)
      || Math.abs(p.x.w.length - word.w.length) - Math.abs(q.x.w.length - word.w.length)
      || p.i - q.i)
    .slice(0, n)
    .map(({ x }) => x);
}

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz'.split('');

const canSpell = (tray, w) => {
  const pool = [...tray];
  for (const ch of w) {
    const k = pool.indexOf(ch);
    if (k < 0) return false;
    pool.splice(k, 1);
  }
  return true;
};

/** Word's letters plus 1 (<=3 letters) or 2 (4 letters) distractor letters, shuffled. A distractor is
 *  rejected if the resulting tray could spell any other bank word of the same length. */
export function letterTray(word, words, rng) {
  const letters = word.w.split('');
  const wanted = word.w.length <= 3 ? 1 : 2;
  const others = words.filter(x => x.w !== word.w && x.w.length === word.w.length).map(x => x.w);
  const tray = [...letters];
  const candidates = shuffle(ALPHABET.filter(ch => !letters.includes(ch)), rng);
  for (const ch of candidates) {
    if (tray.length - letters.length >= wanted) break;
    const trial = [...tray, ch];
    if (others.some(o => canSpell(trial, o))) continue;
    tray.push(ch);
  }
  return shuffle(tray, rng);
}

export function makeTurn(word, level, words, frames, rng) {
  const activity = ACTIVITY_FOR_LEVEL[Math.min(level, ACTIVITY_FOR_LEVEL.length - 1)];
  switch (activity) {
    case 'read':
      return { activity, word, options: shuffle([word, ...distractors(word, words, 3)], rng) };
    case 'pick':
      return { activity, word, options: shuffle([word, ...distractors(word, words, 2)], rng) };
    case 'build':
      return { activity, word, tray: letterTray(word, words, rng) };
    case 'sentence':
    default:
      return {
        activity: 'sentence', word,
        frame: frames[Math.floor(rng() * frames.length)],
        options: shuffle([word, ...distractors(word, words, 2)], rng),
      };
  }
}

export function isCorrect(turn, answer) {
  if (turn.activity === 'build') {
    return Array.isArray(answer) && answer.join('') === turn.word.w;
  }
  return answer === turn.word.w;
}

/** Weighted sample. Without replacement when the pool is at least `count` long; otherwise every pick is
 *  independent (with replacement), so a small pool still fills a round and keeps its weighting. */
function weightedPicks(pool, weightOf, count, rng) {
  if (pool.length === 0) return [];
  const withReplacement = pool.length < count;
  const remaining = [...pool];
  const out = [];
  while (out.length < count) {
    const src = withReplacement ? pool : remaining;
    const total = src.reduce((s, x) => s + weightOf(x), 0);
    let r = rng() * total;
    let idx = src.length - 1;
    for (let i = 0; i < src.length; i++) {
      r -= weightOf(src[i]);
      if (r < 0) { idx = i; break; }
    }
    out.push(src[idx]);
    if (!withReplacement) remaining.splice(idx, 1);
  }
  return out;
}

export function planRound(state, words, frames, rng) {
  const active = activeWords(state, words);
  const learned = learnedWords(state, words);
  if (active.length === 0 && learned.length === 0) return [];
  if (active.length === 0) {
    return weightedPicks(learned, () => 1, ROUND_LENGTH, rng).map(w => makeTurn(w, MAX_LEVEL, words, frames, rng));
  }
  const reviewSlots = learned.length > 0 ? 1 : 0;
  const picks = weightedPicks(active, w => 4 - state.levels[w.w], ROUND_LENGTH - reviewSlots, rng)
    .map(w => makeTurn(w, state.levels[w.w], words, frames, rng));
  if (reviewSlots) {
    const review = learned[Math.floor(rng() * learned.length)];
    const at = Math.floor(rng() * (picks.length + 1));
    picks.splice(at, 0, makeTurn(review, MAX_LEVEL, words, frames, rng));
  }
  return picks;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add docs-site/docs/maes-words/game.js docs-site/tests/maes-words.turns.test.mjs
git commit -m "feat(maes-words): turn construction, look-alike distractors, letter tray, round planning"
```

---

### Task 4: Persistence module `storage.js`

**Files:**
- Create: `docs-site/docs/maes-words/storage.js`
- Create: `docs-site/tests/maes-words.storage.test.mjs`

**Interfaces:**
- Consumes: `State` shape from Task 2 (`{levels, stars, learnedOrder}`), `MAX_LEVEL`.
- Produces: `KEY = 'maes-words:v1'`, `VERSION = 1`, `load(raw, words) -> State | null` (`null` means "no save, start fresh"; never throws), `serialize(state) -> string`.

- [ ] **Step 1: Write the failing tests**

```js
// docs-site/tests/maes-words.storage.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { KEY, VERSION, load, serialize } from '../docs/maes-words/storage.js';

const BANK = [{ w: 'cat', e: '🐱', tier: 1 }, { w: 'dog', e: '🐶', tier: 1 }];

test('key and version', () => {
  assert.equal(KEY, 'maes-words:v1');
  assert.equal(VERSION, 1);
});

test('load(null) returns null so the caller starts a fresh state', () => {
  assert.equal(load(null, BANK), null);
  assert.equal(load('', BANK), null);
});

test('round-trips a state', () => {
  const s = { levels: { cat: 2, dog: 0 }, stars: 7, learnedOrder: [] };
  assert.deepEqual(load(serialize(s), BANK), s);
});

test('serialize stamps the version', () => {
  assert.equal(JSON.parse(serialize({ levels: {}, stars: 0, learnedOrder: [] })).version, 1);
});

test('corrupt JSON and wrong version return null instead of throwing', () => {
  assert.equal(load('{not json', BANK), null);
  assert.equal(load(JSON.stringify({ version: 99, levels: {} }), BANK), null);
});

test('drops words no longer in the bank', () => {
  const raw = JSON.stringify({ version: 1, levels: { cat: 1, zebra: 3 }, stars: 2, learnedOrder: ['zebra'] });
  assert.deepEqual(load(raw, BANK), { levels: { cat: 1 }, stars: 2, learnedOrder: [] });
});

test('repairs bad levels, stars and learnedOrder', () => {
  const raw = JSON.stringify({ version: 1, levels: { cat: 9, dog: -1 }, stars: 'lots', learnedOrder: 'no' });
  assert.deepEqual(load(raw, BANK), { levels: { cat: 3, dog: 0 }, stars: 0, learnedOrder: [] });
  const raw2 = JSON.stringify({ version: 1, levels: { cat: 1.7, dog: 'x' }, stars: 3.9, learnedOrder: ['cat', 'cat', 5] });
  assert.deepEqual(load(raw2, BANK), { levels: { cat: 1 }, stars: 3, learnedOrder: ['cat'] });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test 'docs-site/tests/maes-words.storage.test.mjs'`
Expected: FAIL — cannot find module.

- [ ] **Step 3: Write the implementation**

```js
// docs-site/docs/maes-words/storage.js
// Pure serialisation. The caller owns localStorage so this runs under node.
import { MAX_LEVEL } from './game.js';

export const KEY = 'maes-words:v1';
export const VERSION = 1;

export const serialize = state => JSON.stringify({ version: VERSION, ...state });

/** Parse a stored save, repairing what it can. Returns null when there is nothing usable. Never throws. */
export function load(raw, words) {
  if (!raw) return null;
  let parsed;
  try { parsed = JSON.parse(raw); } catch { return null; }
  if (!parsed || typeof parsed !== 'object' || parsed.version !== VERSION) return null;
  const known = new Set(words.map(x => x.w));
  const levels = {};
  const src = parsed.levels && typeof parsed.levels === 'object' ? parsed.levels : {};
  for (const [w, v] of Object.entries(src)) {
    if (!known.has(w)) continue;
    const n = Number(v);
    if (!Number.isFinite(n)) continue;
    levels[w] = Math.min(MAX_LEVEL, Math.max(0, Math.floor(n)));
  }
  const starsN = Number(parsed.stars);
  const stars = Number.isFinite(starsN) ? Math.max(0, Math.floor(starsN)) : 0;
  const seen = new Set();
  const learnedOrder = (Array.isArray(parsed.learnedOrder) ? parsed.learnedOrder : [])
    .filter(w => typeof w === 'string' && known.has(w) && !seen.has(w) && seen.add(w));
  return { levels, stars, learnedOrder };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add docs-site/docs/maes-words/storage.js docs-site/tests/maes-words.storage.test.mjs
git commit -m "feat(maes-words): versioned localStorage serialisation with repair"
```

---

### Task 5: Page shell, pink theme, Home + Read + Pick, persistence

**Files:**
- Create: `docs-site/docs/maes-words/index.html`
- Create: `docs-site/docs/maes-words/style.css`
- Create: `docs-site/docs/maes-words/ui.js`

**Interfaces:**
- Consumes: everything exported by Tasks 1–4.
- Produces: the screen-switching shell (`showScreen(id)`), the round loop (`startRound`, `showTurn`, `answer`), and the `readSave`/`writeSave` wrappers that later tasks extend. Build and Sentence renderers are stubbed here and filled in by Tasks 6 and 7: `renderBuild` and `renderSentence` must exist as functions in `ui.js` from this task so the round loop's dispatch table is complete.

- [ ] **Step 1: Write `index.html`**

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">
<title>Mae's Words</title>
<meta name="description" content="A reading game for learning short words.">
<link rel="stylesheet" href="style.css">
</head>
<body>
<header>
  <h1 id="title">Mae's Words</h1>
  <div class="header-right">
    <span id="stars" class="stars" aria-label="Stars">⭐ <span id="star-count">0</span></span>
    <button id="speak-btn" type="button" aria-label="Say the word" hidden>🔊</button>
  </div>
</header>
<main>
  <section id="screen-home" class="screen">
    <div id="home-wall" class="wall" aria-label="Words I know"></div>
    <button id="play-btn" type="button" class="big-btn">▶ Play</button>
  </section>

  <section id="screen-turn" class="screen" hidden>
    <div id="progress" class="progress" aria-label="Progress"></div>
    <div id="prompt" class="prompt"></div>
    <div id="slots" class="slots" hidden></div>
    <div id="options" class="options"></div>
  </section>

  <section id="screen-end" class="screen" hidden>
    <div id="end-stars" class="end-stars"></div>
    <div id="end-wall" class="wall" aria-label="Words I know"></div>
    <button id="again-btn" type="button" class="big-btn">▶ Again</button>
  </section>

  <section id="screen-grownup" class="screen" hidden>
    <h2>Grown-up corner</h2>
    <p class="muted">Mae's progress, word by word. Levels: 0 new · 1 reads it · 2 picks it from look-alikes · 3 learned.</p>
    <table id="grownup-table"><thead><tr><th>Word</th><th>Tier</th><th>Level</th></tr></thead><tbody></tbody></table>
    <div class="grownup-actions">
      <button id="grownup-back" type="button" class="big-btn">Back</button>
      <div id="reset-box">
        <p class="muted">To start again, tap the word <strong>reset</strong>:</p>
        <div id="reset-options" class="options small"></div>
      </div>
    </div>
  </section>
</main>
<div id="confetti" aria-hidden="true"></div>
<script type="module" src="ui.js"></script>
</body>
</html>
```

- [ ] **Step 2: Write `style.css` (pink theme, big tiles)**

```css
:root {
  --pink: #e91e8c;
  --pink-dark: #b8126b;
  --bg: #fff0f6;
  --soft: #ffd6e7;
  --tile: #ffffff;
  --fg: #3a2a33;
  --correct: #4caf50;
  --wrong: #b0b0b0;
  --star: #ffc107;
}
* { box-sizing: border-box; }
html, body { height: 100%; }
body {
  margin: 0; background: var(--bg); color: var(--fg);
  font-family: "Helvetica Neue", Helvetica, Arial, sans-serif;
  display: flex; flex-direction: column;
  height: 100vh; height: 100dvh;
  -webkit-user-select: none; user-select: none;
  -webkit-tap-highlight-color: transparent;
  touch-action: manipulation;
}
header {
  background: var(--pink); color: #fff; display: flex; align-items: center;
  justify-content: space-between; padding: 0 16px; height: 56px; flex: none;
  padding-top: env(safe-area-inset-top, 0px);
}
h1 { margin: 0; font-size: 26px; font-weight: 800; letter-spacing: .01em; }
.header-right { display: flex; align-items: center; gap: 12px; }
.stars { font-size: 22px; font-weight: 700; }
#speak-btn { font-size: 24px; background: transparent; border: 0; color: #fff; min-width: 64px; min-height: 48px; cursor: pointer; }
main {
  flex: 1; display: flex; flex-direction: column; align-items: center;
  padding: 16px; overflow-y: auto; overflow-x: hidden;
  padding-bottom: calc(16px + env(safe-area-inset-bottom, 0px));
}
.screen { width: 100%; max-width: 560px; display: flex; flex-direction: column; align-items: center; gap: 20px; flex: 1; }
.big-btn {
  font-size: 32px; font-weight: 800; color: #fff; background: var(--pink); border: 0;
  border-radius: 24px; padding: 20px 40px; min-height: 80px; min-width: 200px; cursor: pointer;
  box-shadow: 0 6px 0 var(--pink-dark); margin-top: auto;
}
.big-btn:active { transform: translateY(4px); box-shadow: 0 2px 0 var(--pink-dark); }

.progress { display: flex; gap: 8px; justify-content: center; }
.progress .dot { width: 14px; height: 14px; border-radius: 50%; background: var(--soft); border: 2px solid var(--pink); }
.progress .dot.done { background: var(--pink); }
.progress .dot.star { background: var(--star); border-color: var(--star); }

.prompt { font-size: 72px; font-weight: 800; text-align: center; min-height: 110px; display: flex; align-items: center; justify-content: center; letter-spacing: .04em; }
.prompt.emoji { font-size: 120px; min-height: 150px; }
.prompt.sentence { font-size: 40px; line-height: 1.3; flex-wrap: wrap; gap: 8px; }
.prompt.sentence .blank { font-size: 64px; }

.options { display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; width: 100%; }
.options.three { grid-template-columns: 1fr; }
.options.small .tile { font-size: 24px; min-height: 64px; }
.tile {
  background: var(--tile); border: 4px solid var(--soft); border-radius: 20px;
  font-size: 44px; font-weight: 800; min-height: 96px; display: flex; align-items: center;
  justify-content: center; cursor: pointer; letter-spacing: .04em; color: var(--fg);
  font-family: inherit;
}
.tile.emoji { font-size: 72px; min-height: 120px; }
.tile.correct { background: var(--correct); border-color: var(--correct); color: #fff; animation: pop .35s ease; }
.tile.wrong { background: var(--wrong); border-color: var(--wrong); color: #fff; animation: shake .5s ease; }
.tile[disabled] { pointer-events: none; }

.slots { display: flex; gap: 12px; justify-content: center; }
.slot { width: 84px; height: 96px; border-radius: 16px; border: 4px dashed var(--pink); background: var(--tile);
        display: flex; align-items: center; justify-content: center; font-size: 52px; font-weight: 800; cursor: pointer; }
.slot.filled { border-style: solid; }
.tray { display: flex; flex-wrap: wrap; gap: 12px; justify-content: center; }
.tray .tile { width: 84px; min-height: 96px; }
.tray .tile.used { visibility: hidden; }

.wall { display: flex; flex-wrap: wrap; gap: 10px; justify-content: center; }
.wall .chip { background: var(--tile); border: 3px solid var(--soft); border-radius: 16px; padding: 8px 14px;
              font-size: 22px; font-weight: 700; display: flex; align-items: center; gap: 8px; }
.wall .chip .e { font-size: 32px; }
.end-stars { font-size: 56px; font-weight: 800; text-align: center; }

.muted { color: #7a5a6a; font-size: 16px; }
#screen-grownup { align-items: stretch; }
#screen-grownup h2 { margin: 0; color: var(--pink-dark); }
#grownup-table { width: 100%; border-collapse: collapse; font-size: 18px; }
#grownup-table th, #grownup-table td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--soft); }
#grownup-table td.lvl-3 { color: var(--correct); font-weight: 700; }
.grownup-actions { display: flex; flex-direction: column; gap: 16px; }

#confetti { position: fixed; inset: 0; pointer-events: none; overflow: hidden; }
#confetti .bit { position: absolute; top: -12px; width: 10px; height: 16px; border-radius: 3px; animation: fall 1.1s linear forwards; }

@keyframes pop { 0% { transform: scale(1); } 50% { transform: scale(1.12); } 100% { transform: scale(1); } }
@keyframes shake { 10%, 90% { transform: translateX(-3px); } 20%, 80% { transform: translateX(5px); }
                   30%, 50%, 70% { transform: translateX(-7px); } 40%, 60% { transform: translateX(7px); } }
@keyframes fall { to { transform: translateY(110vh) rotate(540deg); opacity: .6; } }

@media (min-width: 700px) { .options { grid-template-columns: repeat(4, 1fr); } .options.three { grid-template-columns: repeat(3, 1fr); } }
```

- [ ] **Step 3: Write `ui.js` with Home, Read and Pick, persistence, round loop**

`renderBuild` and `renderSentence` are stubs here (they render nothing and immediately advance); Tasks 6 and 7 replace them.

```js
// docs-site/docs/maes-words/ui.js
import { WORDS, FRAMES } from './words.js';
import { initialState, introduceWords, applyAnswer, planRound, isCorrect, learnedWords,
         ROUND_LENGTH } from './game.js';
import { KEY, load, serialize } from './storage.js';

const $ = id => document.getElementById(id);
const screens = ['screen-home', 'screen-turn', 'screen-end', 'screen-grownup'];
const rng = () => Math.random();

const readSave = () => { try { return load(localStorage.getItem(KEY), WORDS); } catch { return null; } };
const writeSave = s => { try { localStorage.setItem(KEY, serialize(s)); } catch { /* private mode: play in memory */ } };

let state = readSave() ?? initialState(WORDS);
let turns = [];
let turnIndex = 0;
let roundStars = 0;
let busy = false;

function showScreen(id) {
  for (const s of screens) $(s).hidden = s !== id;
  $('speak-btn').hidden = !(id === 'screen-turn' && 'speechSynthesis' in window);
}

function renderStars() { $('star-count').textContent = String(state.stars); }

function renderWall(el) {
  el.innerHTML = '';
  for (const w of learnedWords(state, WORDS)) {
    const chip = document.createElement('div');
    chip.className = 'chip';
    chip.innerHTML = `<span class="e">${w.e}</span><span>${w.w}</span>`;
    el.append(chip);
  }
}

function renderProgress() {
  const el = $('progress');
  el.innerHTML = '';
  turns.forEach((_, i) => {
    const d = document.createElement('div');
    d.className = 'dot' + (i < turnIndex ? ' done' : '');
    el.append(d);
  });
}

function tile(label, { emoji = false } = {}) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'tile' + (emoji ? ' emoji' : '');
  b.textContent = label;
  return b;
}

function renderRead(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt';
  prompt.textContent = turn.word.w;
  const opts = $('options');
  opts.className = 'options';
  opts.innerHTML = '';
  for (const o of turn.options) {
    const b = tile(o.e, { emoji: true });
    b.addEventListener('click', () => answer(o.w, b));
    opts.append(b);
  }
}

function renderPick(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt emoji';
  prompt.textContent = turn.word.e;
  const opts = $('options');
  opts.className = 'options three';
  opts.innerHTML = '';
  for (const o of turn.options) {
    const b = tile(o.w);
    b.addEventListener('click', () => answer(o.w, b));
    opts.append(b);
  }
}

// Filled in by later tasks.
function renderBuild(turn) { renderPick(turn); }
function renderSentence(turn) { renderPick(turn); }

const RENDER = { read: renderRead, pick: renderPick, build: renderBuild, sentence: renderSentence };

function showTurn() {
  if (turnIndex >= turns.length) return endRound();
  busy = false;
  $('slots').hidden = true;
  renderProgress();
  RENDER[turns[turnIndex].activity](turns[turnIndex]);
}

/** Resolve the current turn. `chosenEl` is the tapped tile (or null for build); marks it and advances. */
function answer(value, chosenEl) {
  if (busy) return;
  busy = true;
  const turn = turns[turnIndex];
  const correct = isCorrect(turn, value);
  state = applyAnswer(state, turn.word, correct);
  writeSave(state);
  renderStars();
  const tiles = [...$('options').querySelectorAll('.tile')];
  for (const t of tiles) t.disabled = true;
  if (correct) {
    roundStars += 1;
    if (chosenEl) chosenEl.classList.add('correct');
    $('progress').children[turnIndex]?.classList.add('star');
    celebrate();
  } else {
    if (chosenEl) chosenEl.classList.add('wrong');
    const right = tiles.find(t => t.textContent === turn.word.w || t.textContent === turn.word.e);
    if (right) right.classList.add('correct');
  }
  setTimeout(() => { turnIndex += 1; showTurn(); }, correct ? 900 : 1500);
}

function celebrate() {
  const box = $('confetti');
  const colors = ['#e91e8c', '#ffc107', '#4caf50', '#42a5f5', '#ffd6e7'];
  for (let i = 0; i < 18; i++) {
    const bit = document.createElement('div');
    bit.className = 'bit';
    bit.style.left = `${Math.random() * 100}vw`;
    bit.style.background = colors[i % colors.length];
    bit.style.animationDelay = `${Math.random() * 0.2}s`;
    box.append(bit);
    setTimeout(() => bit.remove(), 1400);
  }
}

function startRound() {
  state = introduceWords(state, WORDS);
  writeSave(state);
  turns = planRound(state, WORDS, FRAMES, rng);
  turnIndex = 0;
  roundStars = 0;
  showScreen('screen-turn');
  showTurn();
}

function endRound() {
  $('end-stars').textContent = `⭐ ${roundStars} / ${ROUND_LENGTH}`;
  renderWall($('end-wall'));
  showScreen('screen-end');
  if (roundStars === ROUND_LENGTH) celebrate();
}

function goHome() {
  renderStars();
  renderWall($('home-wall'));
  showScreen('screen-home');
}

$('play-btn').addEventListener('click', startRound);
$('again-btn').addEventListener('click', startRound);
$('speak-btn').addEventListener('click', () => {
  const turn = turns[turnIndex];
  if (!turn || !('speechSynthesis' in window)) return;
  const u = new SpeechSynthesisUtterance(turn.word.w);
  u.rate = 0.8;
  speechSynthesis.cancel();
  speechSynthesis.speak(u);
});

goHome();
```

- [ ] **Step 4: Run tests (unchanged) and check the page in a browser**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'` — expected all pass.
Then: `cd docs-site/docs && python3 -m http.server 8000` and open `http://localhost:8000/maes-words/`. Check:
- Header is pink with the star count; Play button fills the width nicely on a phone-width window (DevTools device mode, 390px).
- Tap Play: 10 progress dots, a word in big type, four emoji tiles. Tap the right one → green, confetti, star count +1, dot turns gold, auto-advance. Tap a wrong one → grey shake, right one turns green, advance.
- Refresh mid-round: star count and levels persist (check `localStorage['maes-words:v1']` in DevTools).
- Open in a private window (or set `localStorage.setItem = () => { throw new Error('x') }` in the console before Play): the round still plays.
- After enough correct answers, Pick turns appear (emoji prompt, three word tiles).

- [ ] **Step 5: Commit**

```bash
git add docs-site/docs/maes-words/index.html docs-site/docs/maes-words/style.css docs-site/docs/maes-words/ui.js
git commit -m "feat(maes-words): pink shell, home screen, Read and Pick activities, persistence"
```

---

### Task 6: Build activity (letter tray)

**Files:**
- Modify: `docs-site/docs/maes-words/ui.js` — replace the `renderBuild` stub.

**Interfaces:**
- Consumes: `turn.tray` (`string[]`) and `turn.word` from Task 3; `answer(value, chosenEl)` from Task 5, where `value` for build is the array of placed letters and `chosenEl` is `null`.
- Produces: nothing new for other tasks.

- [ ] **Step 1: Replace the stub**

Replace `function renderBuild(turn) { renderPick(turn); }` with:

```js
function renderBuild(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt emoji';
  prompt.textContent = turn.word.e;

  const slotsEl = $('slots');
  slotsEl.hidden = false;
  slotsEl.innerHTML = '';
  const placed = Array(turn.word.w.length).fill(null); // index into tray, or null

  const opts = $('options');
  opts.className = 'tray';
  opts.innerHTML = '';
  const trayTiles = turn.tray.map((ch, i) => {
    const b = tile(ch);
    b.dataset.tray = String(i);
    b.addEventListener('click', () => {
      if (busy || b.classList.contains('used')) return;
      const slot = placed.indexOf(null);
      if (slot < 0) return;
      placed[slot] = i;
      b.classList.add('used');
      paint();
      if (!placed.includes(null)) check();
    });
    opts.append(b);
    return b;
  });

  const slotEls = placed.map((_, s) => {
    const d = document.createElement('div');
    d.className = 'slot';
    d.addEventListener('click', () => {
      if (busy || placed[s] === null) return;
      trayTiles[placed[s]].classList.remove('used');
      placed[s] = null;
      paint();
    });
    slotsEl.append(d);
    return d;
  });

  function paint() {
    placed.forEach((ti, s) => {
      slotEls[s].textContent = ti === null ? '' : turn.tray[ti];
      slotEls[s].classList.toggle('filled', ti !== null);
    });
  }

  function check() {
    const letters = placed.map(ti => turn.tray[ti]);
    const correct = letters.join('') === turn.word.w;
    if (!correct) {
      // Show the right spelling in the slots before moving on.
      slotEls.forEach((el, s) => { el.classList.add('wrong'); setTimeout(() => {
        el.classList.remove('wrong'); el.textContent = turn.word.w[s]; el.classList.add('filled', 'correct');
      }, 500); });
    } else {
      slotEls.forEach(el => el.classList.add('correct'));
    }
    answer(letters, null);
  }
}
```

Also add to `style.css`:

```css
.slot.correct { border-color: var(--correct); background: var(--correct); color: #fff; }
.slot.wrong { border-color: var(--wrong); animation: shake .5s ease; }
```

- [ ] **Step 2: Browser check**

Reach a Build turn quickly by seeding progress in the console before Play:
```js
localStorage.setItem('maes-words:v1', JSON.stringify({ version: 1, levels: { cat: 2, dog: 2, sun: 2, bed: 2, pig: 2, bus: 2 }, stars: 0, learnedOrder: [] })); location.reload();
```
Check: emoji prompt, dashed slots, a tray of 4 (or 6) letter tiles; tapping fills left to right and hides the tile; tapping a filled slot returns the letter; filling all slots auto-checks; wrong shows the correct spelling in green after a shake; correct goes green with confetti; double-tapping the last tile during the delay does nothing.

- [ ] **Step 3: Run tests, commit**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'` — expected all pass.

```bash
git add docs-site/docs/maes-words/ui.js docs-site/docs/maes-words/style.css
git commit -m "feat(maes-words): Build activity with letter tray and slots"
```

---

### Task 7: Sentence activity, round-end wall, Grown-up corner

**Files:**
- Modify: `docs-site/docs/maes-words/ui.js` — replace `renderSentence`, add grown-up corner.

**Interfaces:**
- Consumes: `turn.frame`, `turn.options` from Task 3; `learnedWords`, `initialState` from Task 2; `WORDS`.
- Produces: nothing new for other tasks.

- [ ] **Step 1: Replace the `renderSentence` stub**

```js
function renderSentence(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt sentence';
  prompt.innerHTML = '';
  const [before, after] = turn.frame.split('{}');
  const blank = document.createElement('span');
  blank.className = 'blank';
  blank.textContent = turn.word.e;
  prompt.append(document.createTextNode(before), blank, document.createTextNode(after));
  const opts = $('options');
  opts.className = 'options three';
  opts.innerHTML = '';
  for (const o of turn.options) {
    const b = tile(o.w);
    b.addEventListener('click', () => answer(o.w, b));
    opts.append(b);
  }
}
```

- [ ] **Step 2: Add the Grown-up corner (long-press title, table, guarded reset)**

Append to `ui.js`:

```js
// --- grown-up corner ---------------------------------------------------------
function renderGrownup() {
  const tbody = $('grownup-table').querySelector('tbody');
  tbody.innerHTML = '';
  for (const w of WORDS) {
    const lvl = state.levels[w.w];
    const tr = document.createElement('tr');
    const lvlText = lvl === undefined ? '–' : String(lvl);
    tr.innerHTML = `<td>${w.e} ${w.w}</td><td>${w.tier}</td><td class="lvl-${lvl ?? 'none'}">${lvlText}</td>`;
    tbody.append(tr);
  }
  // Reset guard: the adult must tap the written word "reset" among look-alikes.
  const box = $('reset-options');
  box.innerHTML = '';
  for (const label of ['rest', 'reset', 'resit']) {
    const b = tile(label);
    b.addEventListener('click', () => {
      if (label !== 'reset') { b.classList.add('wrong'); setTimeout(() => b.classList.remove('wrong'), 600); return; }
      state = initialState(WORDS);
      writeSave(state);
      goHome();
    });
    box.append(b);
  }
  showScreen('screen-grownup');
}

let pressTimer = null;
const title = $('title');
const startPress = () => { pressTimer = setTimeout(renderGrownup, 1200); };
const cancelPress = () => { clearTimeout(pressTimer); pressTimer = null; };
title.addEventListener('pointerdown', startPress);
for (const ev of ['pointerup', 'pointerleave', 'pointercancel']) title.addEventListener(ev, cancelPress);
title.addEventListener('contextmenu', e => e.preventDefault());
$('grownup-back').addEventListener('click', goHome);
```

- [ ] **Step 3: Browser check**

- Seed a learned word: `localStorage.setItem('maes-words:v1', JSON.stringify({ version: 1, levels: { cat: 3, dog: 3, sun: 1, bed: 1, pig: 1, bus: 1 }, stars: 20, learnedOrder: ['cat','dog'] })); location.reload();`
- Home shows cat and dog chips on the wall. Play: within the round, one Sentence turn appears ("I like the 🐱" with three word tiles), never more than one.
- Round end shows the stars line and the wall, Again restarts.
- Long-press the title for ~1.2s: table of all words with tier and level; tapping `rest` shakes, tapping `reset` wipes progress and returns Home with an empty wall and 0 stars. Back returns Home without resetting.

- [ ] **Step 4: Run tests, commit**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs'` — expected all pass.

```bash
git add docs-site/docs/maes-words/ui.js
git commit -m "feat(maes-words): Sentence activity, words-I-know wall, grown-up corner"
```

---

### Task 8: Polish, mkdocs strict build, deploy

**Files:**
- Modify: `docs-site/docs/maes-words/style.css`, `docs-site/docs/maes-words/ui.js` as needed.

- [ ] **Step 1: iPad / phone pass**

In DevTools device mode at 390×844 (phone) and 820×1180 (iPad portrait) and 1180×820 (iPad landscape), check every screen:
- No horizontal scroll. Tap targets ≥ 64px (inspect `.tile`, `.slot`, `.big-btn`, `#speak-btn`).
- The Read grid's four emoji tiles fit above the fold on the phone; if not, reduce `.tile.emoji` `min-height` to 104px and `.prompt` font to 64px.
- The Build tray with 6 tiles wraps to two rows on the phone without clipping.
- The Sentence prompt wraps onto two lines cleanly at 390px.
- Speaker button appears only on the turn screen, and only when `speechSynthesis` exists; tapping it says the word.

Fix anything found in `style.css`; commit as `fix(maes-words): layout polish`.

- [ ] **Step 2: Strict docs build**

Run: `cd docs-site && uv run mkdocs build --strict 2>&1 | tail -5`
Expected: build succeeds with no warnings; `docs-site/site/maes-words/index.html` exists. (`site/` is a build artifact — do not commit it.)

- [ ] **Step 3: Full test run and final commit**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs' 2>&1 | tail -8`
Expected: all pass, 0 fail.

```bash
git status --short   # must show nothing under docs-site/site
git add -A docs-site/docs/maes-words docs-site/tests
git commit -m "feat(maes-words): polish and strict-build check" || true
```

- [ ] **Step 4: Ship to main and watch the deploy**

```bash
git checkout main && git pull --ff-only && git merge --ff-only maes-words && git push origin main
gh run list --workflow=docs.yml --limit 1
gh run watch "$(gh run list --workflow=docs.yml --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status
curl -sI https://smeltsql.com/maes-words/ | head -1
curl -s https://smeltsql.com/maes-words/ | grep -c "Mae's Words"
```

Expected: workflow completes successfully, `HTTP/2 200`, and the live page contains "Mae's Words".
