# Mae's Words — Pictograms, Colours, Fill and Try-Again Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow Mae's Words from 87 emoji nouns to a few hundred words shown by emoji, vendored pictograms and colour tiles; teach sight words directly with a new Fill activity; and let Mae try again after a wrong answer without it counting as progress.

**Architecture:** Same layout as v1: pure logic in `game.js` (rng injected, zero DOM), pure serialisation in `storage.js`, all DOM in `ui.js`, data in `words.js`. Word entries gain a `pic` descriptor rendered by one `renderPic()` in `ui.js`; a `SIGHT` bank shares the `levels` map with `WORDS`; frames become templates with `{noun}`/`{sight}` slots. A generator script under `docs-site/tools/` vendors pictogram files into `img/` and writes the generated block of `words.js` and `credits.html` so the bank is reproducible.

**Tech Stack:** Vanilla HTML/CSS/ES modules, `node --test`, Node 24. Generator is a Node script using only `node:fs`, `node:path` and global `fetch`. Headless checks use the Playwright already installed at `docs/demos/node_modules`.

**Spec:** `docs/superpowers/specs/2026-09-27-maes-words-design.md`

## Global Constraints

- Branch: `maes-words-pics`, created from `origin/main`. Commit after every task. Open a **draft PR** after Task 1 so Andrew can play `https://smeltsql.com/branches/pr-<N>/maes-words/`. Do **not** push to `main` until the final task: a push to `main` touching `docs-site/**` deploys the site.
- Zero runtime dependencies: no npm, no CDN, no build step, no runtime fetches. Everything the page needs is a file in `docs-site/docs/maes-words/`. Pictograms are vendored files under `docs-site/docs/maes-words/img/`.
- Word rules: 2–4 lowercase letters, no plurals, tiers 1–3, unique words, unique `pic.text` (emoji) and unique `pic.src`. `pic.kind ∈ {emoji, svg, colour}`. Colour words are `red blue pink grey`. The ambiguity rule: shown only the picture, a five-year-old says exactly this word.
- Sight words: `SIGHT` entries are `{ w, tier, sight: true }`, 2–4 lowercase letters (`a` and `I` are frame text only, never targets). No word may appear in both banks.
- Frames: `{ text, fits, pos? }`; `text` has exactly one `{noun}` and at most one `{sight}`; every `fits` entry is a `SIGHT` word; `pos` defaults to `'noun'` and may be `'verb'` or `'adj'`. Word entries may carry `pos: 'verb' | 'adj'` (default noun); colour words are `pos: 'adj'`.
- Progression constants unchanged: `MAX_LEVEL = 3`, `ROUND_LENGTH = 10`, `INITIAL_INTRO = 6`, `INTRO_BATCH = 2`, `NEW_WORD_FLOOR = 6`, `UNLOCK_FRACTION = 0.75`. New: `SIGHT_INITIAL_INTRO = 2`, `SIGHT_INTRO_BATCH = 1`, `SIGHT_NEW_WORD_FLOOR = 2`. Noun ladder `['read','pick','build','sentence']`, sight ladder `['fill','fill','build','fill']`.
- Try-again: a turn's first wrong answer applies level −1 once; a later correct answer on the same turn earns no level and no star. Tile activities retry until correct; Build allows exactly one retry.
- Storage key stays `maes-words:v1`, version `1`.
- Licences: Mulberry Symbols (Steve Lee, CC BY-SA 2.0 UK) primary; ARASAAC (Government of Aragon, CC BY-NC-SA 4.0) gap-filler only. Both attributed in `credits.html`. Symbols are used unmodified apart from whitespace/comment stripping.
- Silent by design; no child-facing text instructions; every tap target ≥ 64px. Theme tokens as in `style.css` `:root`.
- Timeless-oracle rule for the spec: no phase/task vocabulary in the spec body.
- This work touches no Rust crate. Do not run `verify-phase.sh`, `cargo`, or update `docs/ROADMAP.md`.
- Tests: `node --test 'docs-site/tests/maes-words.*.test.mjs'` (quote the glob). Tests live in `docs-site/tests/`, never under `docs-site/docs/`.
- Manual/headless check: `cd docs-site/docs && python3 -m http.server 8000` then `http://localhost:8000/maes-words/`. **Never `run_in_background` the server from a subagent that then waits on it.** Start it with `(python3 -m http.server 8000 >/dev/null 2>&1 &)` and kill it with `pkill -f "http.server 8000"` at the end of the step.
- Before the final merge: `cd docs-site && uv run mkdocs build --strict` must pass.

## Review Focus

Inputs the spec implies but no task's tests would otherwise exercise. Each gets a pinning test in the owning task.

1. **A pictogram file referenced by `words.js` is missing on disk** (generator table edited, image not regenerated): the page would show a broken image with no caption, which fails the ambiguity rule silently. → Task 1 words test `every svg/png src exists on disk`.
2. **A Fill frame where two offered options both fit** ("I ___ the dog" with `like` and `see`): a right answer marked wrong. → Task 6 test `fill distractors are never in the frame's fits`.
3. **A sight word at level 2 sent to Build with a tray that can spell another sight word** (`the` + `n` → `then`): two correct spellings. → Task 6 test `letterTray for a sight word never spells another word of either bank`.
4. **A retry on Build after the second wrong attempt**: the UI must reveal and move on rather than loop forever. → Task 2 test `retryAllowed allows exactly one Build retry`, Task 7 headless script step.
5. **A verb or adjective drawn into a noun frame** ("I like the jump"): ungrammatical sentence at level 3. → Task 3 test `framesFor only returns frames whose pos matches the word`.

---

### Task 1: Branch, `pic` shape, `renderPic`, draft PR

**Files:**
- Modify: `docs-site/docs/maes-words/words.js` (every entry: `e` → `pic`)
- Modify: `docs-site/docs/maes-words/ui.js` (add `renderPic`, use it everywhere `w.e` / `turn.word.e` appeared, tiles carry `data-w`)
- Modify: `docs-site/docs/maes-words/style.css` (`.tile.pic`, `.tile.swatch`, `.prompt.pic`, `.chip .pic`)
- Modify: `docs-site/tests/maes-words.words.test.mjs`

**Interfaces:**
- Produces: `WORDS` entries `{ w, tier, pic: { kind: 'emoji', text } | { kind: 'svg', src } | { kind: 'colour', css }, noA?, pos? }`. `ui.js` `renderPic(pic, { size })` returns an `HTMLElement`. Every option tile has `dataset.w = word.w`.
- Note for later tasks: `game.js` never reads `pic`, so nothing there changes in this task.

- [ ] **Step 1: Create the branch**

```bash
cd /home/andrew/smelt-sql && git fetch origin && git checkout -b maes-words-pics origin/main
```

- [ ] **Step 2: Write the failing words tests**

Replace the first two tests in `docs-site/tests/maes-words.words.test.mjs` with:

```js
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME_DIR = join(HERE, '..', 'docs', 'maes-words');

test('every word is 2-4 lowercase letters with a tier and a well-formed pic', () => {
  for (const { w, tier, pic } of WORDS) {
    assert.match(w, /^[a-z]{2,4}$/, `bad word ${w}`);
    assert.ok([1, 2, 3].includes(tier), `bad tier for ${w}`);
    assert.ok(pic && typeof pic === 'object', `no pic for ${w}`);
    assert.ok(['emoji', 'svg', 'colour'].includes(pic.kind), `bad pic.kind for ${w}`);
    if (pic.kind === 'emoji') assert.ok(typeof pic.text === 'string' && pic.text.length > 0, `no emoji text for ${w}`);
    if (pic.kind === 'svg') assert.match(pic.src, /^img\/[a-z0-9_-]+\.(svg|png)$/, `bad src for ${w}`);
    if (pic.kind === 'colour') assert.match(pic.css, /^#[0-9a-f]{6}$/, `bad css for ${w}`);
  }
});

test('every svg/png src exists on disk', () => {
  for (const { w, pic } of WORDS) {
    if (pic.kind !== 'svg') continue;
    assert.ok(existsSync(join(GAME_DIR, pic.src)), `${w}: ${pic.src} missing`);
  }
});

test('words, emoji and image sources are unique', () => {
  const ws = WORDS.map(x => x.w);
  const es = WORDS.filter(x => x.pic.kind === 'emoji').map(x => x.pic.text);
  const srcs = WORDS.filter(x => x.pic.kind === 'svg').map(x => x.pic.src);
  assert.equal(new Set(ws).size, ws.length, 'duplicate word');
  assert.equal(new Set(es).size, es.length, 'duplicate emoji');
  assert.equal(new Set(srcs).size, srcs.length, 'duplicate src');
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `node --test 'docs-site/tests/maes-words.words.test.mjs' 2>&1 | tail -20`
Expected: FAIL, `no pic for cat`.

- [ ] **Step 4: Convert `words.js` entries (one-off, not committed as a script)**

```bash
cd /home/andrew/smelt-sql && node -e "
const fs=require('fs');const p='docs-site/docs/maes-words/words.js';
let s=fs.readFileSync(p,'utf8');
s=s.replace(/\{ w: '([a-z]+)',\s*e: '([^']+)',\s*tier: (\d)(, noA: true)? \}/g,
  (m,w,e,t,noA)=>\`{ w: '\${w}', tier: \${t}, pic: { kind: 'emoji', text: '\${e}' }\${noA||''} }\`);
fs.writeFileSync(p,s);"
grep -c "kind: 'emoji'" docs-site/docs/maes-words/words.js   # expect 87
grep -c " e: '" docs-site/docs/maes-words/words.js            # expect 0
```

Update the header comment of `words.js` to describe `pic` (`emoji` | `svg` | `colour`) and `noA`.

- [ ] **Step 5: Add `renderPic` to `ui.js` and use it everywhere**

Add after the `tile()` helper:

```js
/** The one place a word's picture becomes DOM. `size` is 'tile' | 'prompt' | 'chip'. */
function renderPic(pic, { size = 'tile' } = {}) {
  if (!pic) { const s = document.createElement('span'); s.className = `pic pic-none ${size}`; return s; }
  if (pic.kind === 'emoji') {
    const s = document.createElement('span');
    s.className = `pic pic-emoji ${size}`;
    s.textContent = pic.text;
    return s;
  }
  if (pic.kind === 'colour') {
    const d = document.createElement('div');
    d.className = `pic pic-colour ${size}`;
    d.style.background = pic.css;
    return d;
  }
  const img = document.createElement('img');
  img.className = `pic pic-img ${size}`;
  img.src = pic.src;
  img.alt = '';
  img.draggable = false;
  return img;
}

/** A tile showing a word's picture. Tap target stays ≥64px. */
function picTile(word) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'tile pic-tile';
  b.dataset.w = word.w;
  b.append(renderPic(word.pic, { size: 'tile' }));
  return b;
}
```

Then:

- `tile(label)` gains `b.dataset.w = label;` so word tiles are findable by word.
- `renderRead`: replace `tile(o.e, { emoji: true })` with `picTile(o)`.
- `renderPick`, `renderBuild`, `renderSentence`: replace `prompt.textContent = turn.word.e` (and `prompt.className = 'prompt emoji'`) with `prompt.className = 'prompt pic'; prompt.innerHTML = ''; prompt.append(renderPic(turn.word.pic, { size: 'prompt' }));`. In `renderSentence`, `blank.textContent = turn.word.e` becomes `blank.innerHTML = ''; blank.append(renderPic(turn.word.pic, { size: 'prompt' }));`.
- `renderWall`: `chip.innerHTML = ...` becomes `chip.append(renderPic(w.pic, { size: 'chip' }), Object.assign(document.createElement('span'), { textContent: w.w }));`.
- `renderGrownup`: first cell becomes a `<td>` with `renderPic(w.pic, { size: 'chip' })` then a text node ` ${w.w}`.
- `answer()`: replace `tiles.find(t => t.textContent === turn.word.w || t.textContent === turn.word.e)` with `tiles.find(t => t.dataset.w === turn.word.w)`.
- Remove the now-unused `emoji` option from `tile()` and the `.tile.emoji` selector usage.

- [ ] **Step 6: CSS for pics**

Append to `style.css`:

```css
.pic-emoji.tile { font-size: 72px; line-height: 1; }
.pic-emoji.prompt { font-size: 120px; line-height: 1; }
.pic-emoji.chip { font-size: 32px; line-height: 1; }
.pic-img.tile { width: 96px; height: 96px; object-fit: contain; }
.pic-img.prompt { width: 160px; height: 160px; object-fit: contain; }
.pic-img.chip { width: 36px; height: 36px; object-fit: contain; }
.pic-colour { border-radius: 14px; border: 3px solid rgba(0,0,0,.08); }
.pic-colour.tile { width: 96px; height: 96px; }
.pic-colour.prompt { width: 160px; height: 160px; }
.pic-colour.chip { width: 36px; height: 36px; }
.tile.pic-tile { min-height: 120px; }
.prompt.pic { min-height: 170px; }
.prompt.sentence .blank .pic-img, .prompt.sentence .blank .pic-colour { width: 72px; height: 72px; vertical-align: middle; }
.prompt.sentence .blank .pic-emoji { font-size: 64px; }
```

- [ ] **Step 7: Run all tests**

Run: `node --test 'docs-site/tests/maes-words.*.test.mjs' 2>&1 | tail -8`
Expected: all pass (the `game`/`turns`/`storage` tests build their own banks with `e` and never read it, so they still pass).

- [ ] **Step 8: Play it**

```bash
cd /home/andrew/smelt-sql/docs-site/docs && (python3 -m http.server 8000 >/dev/null 2>&1 &) ; sleep 1
NODE_PATH=/home/andrew/smelt-sql/docs/demos/node_modules node -e "
const { chromium } = require('playwright');
(async () => { const b = await chromium.launch(); const p = await b.newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e)));
  await p.goto('http://localhost:8000/maes-words/'); await p.click('#play-btn');
  await p.waitForSelector('#options .tile'); const n = await p.locator('#options .tile').count();
  console.log('tiles', n, 'errors', errs); await b.close(); })();"
pkill -f "http.server 8000"
```

Expected: `tiles 4 errors []`.

- [ ] **Step 9: Commit and open the draft PR**

```bash
cd /home/andrew/smelt-sql && git add docs-site && git commit -m "feat(maes-words): pic descriptor on word entries and one renderPic()"
git push -u origin maes-words-pics
gh pr create --draft --title "Mae's Words: pictograms, colours, Fill activity, try-again" --body "Implements docs/superpowers/plans/2026-09-28-maes-words-pics-fill.md. Preview: https://smeltsql.com/branches/pr-<N>/maes-words/ once the preview workflow runs."
```

Replace `<N>` in the body after creation with `gh pr edit <N> --body ...`.

---

### Task 2: Try-again mechanic

**Files:**
- Modify: `docs-site/docs/maes-words/game.js`
- Modify: `docs-site/docs/maes-words/ui.js` (`answer`, `renderBuild.check`)
- Modify: `docs-site/docs/maes-words/style.css`
- Test: `docs-site/tests/maes-words.game.test.mjs`

**Interfaces:**
- Produces: `retryAllowed(turn, attempts) -> boolean` (`attempts` = wrong answers so far on this turn); `applyAttempt(state, word, correct, attempts) -> State` (delegates to `applyAnswer` only when `attempts === 0`, otherwise returns `state` unchanged).

- [ ] **Step 1: Write the failing tests** (append to `maes-words.game.test.mjs`)

```js
import { applyAttempt, retryAllowed } from '../docs/maes-words/game.js';

test('applyAttempt: a wrong-then-right turn changes the level by exactly -1 and adds no star', () => {
  let s = withLevels({ cat: 2 });
  s = applyAttempt(s, byName('cat'), false, 0);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 0);
  s = applyAttempt(s, byName('cat'), false, 1);
  assert.equal(s.levels.cat, 1, 'second wrong on the same turn does not count again');
  s = applyAttempt(s, byName('cat'), true, 2);
  assert.equal(s.levels.cat, 1, 'fixing it earns no level');
  assert.equal(s.stars, 0, 'fixing it earns no star');
});

test('applyAttempt: a clean first try behaves like applyAnswer', () => {
  const s = applyAttempt(withLevels({ cat: 0 }), byName('cat'), true, 0);
  assert.equal(s.levels.cat, 1);
  assert.equal(s.stars, 1);
});

test('retryAllowed allows exactly one Build retry and unlimited tile retries', () => {
  assert.equal(retryAllowed({ activity: 'build' }, 1), true);
  assert.equal(retryAllowed({ activity: 'build' }, 2), false);
  for (const activity of ['read', 'pick', 'sentence', 'fill']) {
    assert.equal(retryAllowed({ activity }, 1), true);
    assert.equal(retryAllowed({ activity }, 3), true);
  }
});
```

- [ ] **Step 2: Run to verify failure**

Run: `node --test 'docs-site/tests/maes-words.game.test.mjs' 2>&1 | tail -12` → FAIL, `applyAttempt` not exported.

- [ ] **Step 3: Implement in `game.js`** (after `applyAnswer`)

```js
export const BUILD_RETRIES = 1;

/** Only the first attempt of a turn moves the ladder or earns a star. */
export function applyAttempt(state, word, correct, attempts) {
  return attempts === 0 ? applyAnswer(state, word, correct) : state;
}

/** May Mae have another go after `attempts` wrong answers on this turn? */
export function retryAllowed(turn, attempts) {
  return turn.activity === 'build' ? attempts <= BUILD_RETRIES : true;
}
```

- [ ] **Step 4: Run tests** → PASS.

- [ ] **Step 5: UI — tile activities retry**

In `ui.js` add module state `let attempts = 0;` and reset it in `showTurn()` (`attempts = 0;`). Import `applyAttempt, retryAllowed` and replace `answer()`:

```js
function answer(value, chosenEl) {
  if (busy) return;
  const turn = turns[turnIndex];
  const correct = isCorrect(turn, value);
  state = applyAttempt(state, turn.word, correct, attempts);
  writeSave(state);
  renderStars();
  const tiles = [...$('options').querySelectorAll('.tile')];
  if (correct) {
    busy = true;
    results[turnIndex] = attempts === 0;          // starred only when clean
    for (const t of tiles) t.disabled = true;
    if (chosenEl) chosenEl.classList.add('correct');
    if (attempts === 0) {
      roundStars += 1;
      $('progress').children[turnIndex]?.classList.add('star');
      celebrate();
    }
    setTimeout(() => { turnIndex += 1; showTurn(); }, 900);
    return;
  }
  attempts += 1;
  results[turnIndex] = false;
  if (chosenEl) { chosenEl.classList.add('wrong'); chosenEl.disabled = true; }
  if (retryAllowed(turn, attempts)) return;      // tiles: keep going on the same turn
  // Build only: out of retries — reveal and move on.
  busy = true;
  for (const t of tiles) t.disabled = true;
  setTimeout(() => { turnIndex += 1; showTurn(); }, 1500);
}
```

`renderProgress` already paints `done` for `i < turnIndex` and `star` for `results[i] === true`, so a fixed turn shows done-but-unstarred with no change.

- [ ] **Step 6: UI — Build retries once**

In `renderBuild`, replace `check()` with:

```js
function check() {
  const letters = placed.map(ti => turn.tray[ti]);
  const correct = isCorrect(turn, letters);
  if (correct) {
    slotEls.forEach(el => el.classList.add('correct'));
    answer(letters, null);
    return;
  }
  const canRetry = retryAllowed(turn, attempts + 1);
  slotEls.forEach(el => el.classList.add('wrong'));
  setTimeout(() => {
    slotEls.forEach(el => el.classList.remove('wrong'));
    if (canRetry) {
      // slide letters back to the tray, empty the slots, let her rebuild
      placed.forEach((ti, s) => { if (ti !== null) trayTiles[ti].classList.remove('used'); placed[s] = null; });
      paint();
    } else {
      slotEls.forEach((el, s) => { el.textContent = turn.word.w[s]; el.classList.add('filled', 'correct'); });
    }
  }, 500);
  answer(letters, null);
}
```

`answer()` handles the level/star accounting and, when `retryAllowed` is false, the advance.

- [ ] **Step 7: CSS** — `.tile.wrong[disabled] { opacity: .55; }` so a greyed tile visibly stays out.

- [ ] **Step 8: Headless check of wrong-then-right**

```bash
cd /home/andrew/smelt-sql/docs-site/docs && (python3 -m http.server 8000 >/dev/null 2>&1 &) ; sleep 1
NODE_PATH=/home/andrew/smelt-sql/docs/demos/node_modules node -e "
const { chromium } = require('playwright');
(async () => { const b = await chromium.launch(); const p = await b.newPage();
  await p.goto('http://localhost:8000/maes-words/'); await p.click('#play-btn');
  await p.waitForSelector('#options .tile');
  const word = await p.locator('#prompt').innerText();
  const tiles = p.locator('#options .tile');
  const ws = await tiles.evaluateAll(els => els.map(e => e.dataset.w));
  const wrong = ws.findIndex(w => w !== word.trim());
  await tiles.nth(wrong).click();
  const stillLive = await tiles.evaluateAll(els => els.filter(e => !e.disabled).length);
  const stars = await p.locator('#star-count').innerText();
  await tiles.nth(ws.indexOf(word.trim())).click();
  await p.waitForTimeout(1100);
  const stars2 = await p.locator('#star-count').innerText();
  console.log({ stillLive, stars, stars2, advanced: await p.locator('.progress .dot.done').count() });
  await b.close(); })();"
pkill -f "http.server 8000"
```

Expected: `stillLive: 3, stars: '0', stars2: '0', advanced: 1`.

- [ ] **Step 9: Commit**

```bash
git add docs-site && git commit -m "feat(maes-words): try again after a wrong answer; only a clean first try earns progress" && git push
```

---

### Task 3: Frame templates with `pos` and `fits`; Sentence via templates

**Files:**
- Modify: `docs-site/docs/maes-words/words.js` (`FRAMES` become objects)
- Modify: `docs-site/docs/maes-words/game.js` (`framesFor`, new `resolveFrame`, `makeTurn` sentence case)
- Modify: `docs-site/docs/maes-words/ui.js` (`renderSentence` unchanged contract: `turn.frame` is a string with `{}`)
- Test: `docs-site/tests/maes-words.turns.test.mjs`, `docs-site/tests/maes-words.words.test.mjs`

**Interfaces:**
- Produces: `FRAMES: { text, fits, pos? }[]`; `framesFor(word, frames) -> Frame[]` (filters by `pos` match and article rule); `resolveFrame(frame, sight) -> string` (substitutes `{sight}`, capitalises a sentence-initial sight, turns `{noun}` into `{}`); Sentence turns still carry `frame: string` with one `{}`.
- Word `pos`: `'noun'` when absent.

- [ ] **Step 1: Write failing tests**

In `maes-words.turns.test.mjs` change `const FRAMES = [...]` to:

```js
const FRAMES = [
  { text: 'I {sight} the {noun}', fits: ['like', 'see'] },
  { text: 'Here is a {noun}', fits: [] },
  { text: '{sight} is a {noun}', fits: ['here', 'this'] },
  { text: 'I can {noun}', fits: [], pos: 'verb' },
  { text: 'It is {noun}', fits: [], pos: 'adj' },
];
```

Update the existing `framesFor` tests to pass objects (`{ text: 'Here is a {noun}', fits: [] }` etc.) and assert on `f.text`; update `makeTurn ... sentence` assertions to `assert.ok(sent.frame.includes('{}'))` and `assert.equal((sent.frame.match(/\{\}/g) || []).length, 1)`. Add:

```js
import { resolveFrame } from '../docs/maes-words/game.js';

test('resolveFrame substitutes the sight word, capitalises when sentence-initial, and leaves one {} for the noun', () => {
  assert.equal(resolveFrame({ text: 'I {sight} the {noun}', fits: ['like'] }, 'like'), 'I like the {}');
  assert.equal(resolveFrame({ text: '{sight} is a {noun}', fits: ['here'] }, 'here'), 'Here is a {}');
  assert.equal(resolveFrame({ text: 'Here is a {noun}', fits: [] }, null), 'Here is a {}');
});

test('framesFor only returns frames whose pos matches the word', () => {
  const verb = framesFor({ w: 'run', pos: 'verb' }, FRAMES);
  assert.ok(verb.length > 0);
  assert.ok(verb.every(f => f.pos === 'verb'));
  const noun = framesFor({ w: 'cat' }, FRAMES);
  assert.ok(noun.every(f => (f.pos ?? 'noun') === 'noun'));
  const adj = framesFor({ w: 'red', pos: 'adj' }, FRAMES);
  assert.deepEqual(adj.map(f => f.text), ['It is {noun}']);
});

test('a sentence turn for a templated frame uses one of its fits', () => {
  for (let seed = 1; seed <= 30; seed++) {
    const t = makeTurn(byName('cat'), 3, BANK, FRAMES, seeded(seed));
    assert.equal((t.frame.match(/\{\}/g) || []).length, 1, t.frame);
    assert.doesNotMatch(t.frame, /\{sight\}|\{noun\}/);
  }
});
```

In `maes-words.words.test.mjs` replace the frame tests with:

```js
test('every frame has one {noun}, at most one {sight}, a fits list of SIGHT words, and a known pos', () => {
  assert.ok(FRAMES.length >= 6);
  for (const f of FRAMES) {
    assert.equal((f.text.match(/\{noun\}/g) || []).length, 1, `bad frame ${f.text}`);
    assert.ok((f.text.match(/\{sight\}/g) || []).length <= 1, `two sights in ${f.text}`);
    assert.ok(Array.isArray(f.fits), `no fits on ${f.text}`);
    if (f.text.includes('{sight}')) assert.ok(f.fits.length >= 1, `templated frame with empty fits: ${f.text}`);
    else assert.equal(f.fits.length, 0, `fits on an untemplated frame: ${f.text}`);
    assert.ok(['noun', 'verb', 'adj'].includes(f.pos ?? 'noun'), `bad pos on ${f.text}`);
  }
});

test('at least three noun frames use "the" so vowel/noA words still get variety', () => {
  const theFrames = FRAMES.filter(f => (f.pos ?? 'noun') === 'noun' && /\bthe\b/i.test(f.text));
  assert.ok(theFrames.length >= 3, `only ${theFrames.length} "the" frames`);
});
```

(The `fits ⊆ SIGHT` assertion is added in Task 6 when `SIGHT` exists.)

- [ ] **Step 2: Run** → FAIL (`resolveFrame` missing; `framesFor` regex on objects throws).

- [ ] **Step 3: Implement in `game.js`**

```js
export const posOf = word => word.pos ?? 'noun';

/** Frames usable for a word: same part of speech; and no "a {noun}" for vowel-initial, mass-noun or number words. */
export function framesFor(word, frames) {
  const samePos = frames.filter(f => (f.pos ?? 'noun') === posOf(word));
  const needsThe = word.noA || /^[aeiou]/.test(word.w);
  const ok = needsThe ? samePos.filter(f => !/\ba \{noun\}/.test(f.text)) : samePos;
  if (ok.length) return ok;
  return samePos.length ? samePos : frames;
}

const capitalise = s => s.charAt(0).toUpperCase() + s.slice(1);

/** Fill in the sight slot (capitalised if it opens the sentence) and leave `{}` where the pictured word goes. */
export function resolveFrame(frame, sight) {
  let text = frame.text;
  if (sight !== null && sight !== undefined) {
    text = text.startsWith('{sight}') ? text.replace('{sight}', capitalise(sight)) : text.replace('{sight}', sight);
  }
  return text.replace('{noun}', '{}');
}
```

And the `sentence` case of `makeTurn`:

```js
    case 'sentence':
    default: {
      const usable = framesFor(word, frames);
      const frame = usable[Math.floor(rng() * usable.length)];
      const sight = frame.fits.length ? frame.fits[Math.floor(rng() * frame.fits.length)] : null;
      return {
        activity: 'sentence', word,
        frame: resolveFrame(frame, sight),
        options: shuffle([word, ...distractors(word, words, 2)], rng),
      };
    }
```

- [ ] **Step 4: Update `FRAMES` in `words.js`**

```js
// Frames: `{noun}` is the pictured word's slot (any part of speech named by `pos`, default noun);
// `{sight}` is an optional sight-word slot and `fits` lists EVERY sight word that makes the sentence right.
export const FRAMES = [
  { text: 'I {sight} the {noun}',   fits: ['like', 'see', 'have'] },
  { text: 'We {sight} the {noun}',  fits: ['like', 'see', 'have'] },
  { text: '{sight} is a {noun}',    fits: ['here', 'this', 'it'] },
  { text: 'The {noun} is {sight}',  fits: ['here', 'up', 'in'] },
  { text: 'I like {sight} {noun}',  fits: ['my', 'the'] },
  { text: 'Here is {sight} {noun}', fits: ['my', 'the'] },
  { text: 'The {noun} {sight} here', fits: ['is', 'was'] },
  { text: 'I can see a {noun}',     fits: [] },
  { text: 'Look at the {noun}',     fits: [] },
  { text: 'I can {noun}',           fits: [], pos: 'verb' },
  { text: 'We {sight} {noun}',      fits: ['can'], pos: 'verb' },
  { text: 'Look at me {noun}',      fits: [], pos: 'verb' },
  { text: 'It is {noun}',           fits: [], pos: 'adj' },
  { text: '{sight} is {noun}',      fits: ['it', 'this'], pos: 'adj' },
];
```

Note: `a` is deliberately absent from `fits` on `I like {sight} {noun}` because `a` is never a target. A `fits` entry that is not a sight word (e.g. `['dog']`) is a bug the Task 6 words test catches.

- [ ] **Step 5: Run all tests** → PASS. `renderSentence` keeps splitting on `{}` so the UI needs no change.

- [ ] **Step 6: Commit**

```bash
git add docs-site && git commit -m "feat(maes-words): frame templates with pos and fits; Sentence draws from templates" && git push
```

---

### Task 4: Colour tiles

**Files:**
- Modify: `docs-site/docs/maes-words/words.js` (4 colour entries)
- Modify: `docs-site/docs/maes-words/game.js` (`distractors` colour rule)
- Test: `docs-site/tests/maes-words.turns.test.mjs`, `docs-site/tests/maes-words.words.test.mjs`

**Interfaces:**
- Produces: colour entries `{ w: 'red', tier: 1, pic: { kind: 'colour', css: '#e53935' }, pos: 'adj', noA: true }`; `distractors()` returns other colour words first for a colour word.

- [ ] **Step 1: Failing tests**

`maes-words.words.test.mjs`:

```js
test('the colour words are present as colour tiles and adjectives', () => {
  for (const c of ['red', 'blue', 'pink', 'grey']) {
    const e = WORDS.find(x => x.w === c);
    assert.ok(e, `missing ${c}`);
    assert.equal(e.pic.kind, 'colour');
    assert.equal(e.pos, 'adj');
    assert.equal(e.noA, true);
  }
  for (const e of WORDS) if (e.pic.kind === 'colour') assert.equal(e.pos, 'adj', `${e.w} colour but not adj`);
});
```

`maes-words.turns.test.mjs` (extend `mk` to accept a pic: `const mk = (w, tier, extra = {}) => ({ w, e: w.toUpperCase(), tier, ...extra });` and add colours to a local bank):

```js
test('a colour word in Read is shown among the other colours', () => {
  const colours = ['red', 'blue', 'pink', 'grey'].map(w => mk(w, 1, { pic: { kind: 'colour', css: '#000000' }, pos: 'adj', noA: true }));
  const bank = [...BANK, ...colours];
  const t = makeTurn(colours[0], 0, bank, FRAMES, seeded(3));
  assert.equal(t.options.length, 4);
  assert.ok(t.options.every(o => o.pic?.kind === 'colour'), t.options.map(o => o.w).join(','));
});
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement**

In `game.js` `distractors`:

```js
export function distractors(word, words, n) {
  const isColour = x => x.pic?.kind === 'colour';
  const pool = isColour(word) && words.filter(x => isColour(x) && x.w !== word.w).length >= n
    ? words.filter(isColour)
    : words;
  return pool
    .map((x, i) => ({ x, i }))
    // ... existing body unchanged from here
```

In `words.js`, add after tier-1 nouns:

```js
  // colours: shown as a solid tile, never an image
  { w: 'red',  tier: 1, pic: { kind: 'colour', css: '#e53935' }, pos: 'adj', noA: true },
  { w: 'blue', tier: 2, pic: { kind: 'colour', css: '#1e88e5' }, pos: 'adj', noA: true },
  { w: 'pink', tier: 2, pic: { kind: 'colour', css: '#f06292' }, pos: 'adj', noA: true },
  { w: 'grey', tier: 3, pic: { kind: 'colour', css: '#9e9e9e' }, pos: 'adj', noA: true },
```

- [ ] **Step 4: Run all tests** → PASS. Play a Read turn for `red` headlessly (as Task 1 Step 8) and confirm `.pic-colour` tiles render.

- [ ] **Step 5: Commit**

```bash
git add docs-site && git commit -m "feat(maes-words): colour words as coloured tiles" && git push
```

---

### Task 5: Pictogram generator, vendored images, credits page

**Files:**
- Create: `docs-site/tools/generate-maes-pics.mjs`
- Create: `docs-site/docs/maes-words/img/*.svg|png` (generated)
- Create: `docs-site/docs/maes-words/credits.html` (hand-written shell; generated table between markers)
- Modify: `docs-site/docs/maes-words/words.js` (generated block between markers)
- Modify: `docs-site/docs/maes-words/index.html`, `ui.js` (credits link in Grown-up corner)
- Test: `docs-site/tests/maes-words.words.test.mjs`, new `docs-site/tests/maes-words.pics.test.mjs`

**Interfaces:**
- Produces: `words.js` contains
  ```js
  // BEGIN GENERATED PICS — edit docs-site/tools/generate-maes-pics.mjs, not this block
  ...entries...
  // END GENERATED PICS
  ```
  and `credits.html` contains `<!-- BEGIN GENERATED CREDITS -->` … `<!-- END GENERATED CREDITS -->`. The generator exports nothing; it is run as `node docs-site/tools/generate-maes-pics.mjs [--check]` (`--check` exits 1 if the generated blocks would change).

- [ ] **Step 1: Failing tests** — `docs-site/tests/maes-words.pics.test.mjs`:

```js
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, statSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { WORDS } from '../docs/maes-words/words.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME = join(HERE, '..', 'docs', 'maes-words');

test('generated block markers are present in words.js and credits.html', () => {
  const words = readFileSync(join(GAME, 'words.js'), 'utf8');
  assert.ok(words.includes('// BEGIN GENERATED PICS') && words.includes('// END GENERATED PICS'));
  const credits = readFileSync(join(GAME, 'credits.html'), 'utf8');
  assert.ok(credits.includes('<!-- BEGIN GENERATED CREDITS -->') && credits.includes('<!-- END GENERATED CREDITS -->'));
  assert.match(credits, /Mulberry Symbols/);
  assert.match(credits, /CC BY-SA 2\.0 UK/);
  assert.match(credits, /ARASAAC/);
  assert.match(credits, /CC BY-NC-SA 4\.0/);
});

test('every image in img/ is referenced by exactly one word and is small', () => {
  const used = new Map(WORDS.filter(x => x.pic.kind === 'svg').map(x => [x.pic.src.replace(/^img\//, ''), x.w]));
  const files = readdirSync(join(GAME, 'img')).filter(f => /\.(svg|png)$/.test(f));
  for (const f of files) {
    assert.ok(used.has(f), `orphan image ${f}`);
    const size = statSync(join(GAME, 'img', f)).size;
    assert.ok(size < 60_000, `${f} is ${size} bytes`);
  }
  const total = files.reduce((s, f) => s + statSync(join(GAME, 'img', f)).size, 0);
  assert.ok(total < 6_000_000, `img/ totals ${total} bytes`);
});

test('the bank has grown and every pictogram word is credited', () => {
  assert.ok(WORDS.length >= 150, `only ${WORDS.length} words`);
  const credits = readFileSync(join(GAME, 'credits.html'), 'utf8');
  for (const x of WORDS) if (x.pic.kind === 'svg') assert.ok(credits.includes(`>${x.w}<`), `${x.w} not credited`);
});
```

Also in `maes-words.words.test.mjs` add:

```js
test('pos is only ever verb or adj where present, and verbs/adjs never take noA-less "a" frames by accident', () => {
  for (const x of WORDS) if ('pos' in x) assert.ok(['verb', 'adj'].includes(x.pos), `${x.w} pos ${x.pos}`);
});
```

- [ ] **Step 2: Run** → FAIL (no `credits.html`, no markers).

- [ ] **Step 3: Write the generator**

`docs-site/tools/generate-maes-pics.mjs`:

```js
#!/usr/bin/env node
// Generator for Mae's Words' pictogram entries.
//
//   node docs-site/tools/generate-maes-pics.mjs          # fetch, optimise, write img/ + generated blocks
//   node docs-site/tools/generate-maes-pics.mjs --check  # exit 1 if the generated blocks would change
//
// Sources (fetched over HTTPS at generation time only; the game never fetches):
//   mulberry: https://raw.githubusercontent.com/mulberrysymbols/mulberry-symbols/master/EN/<id>.svg
//             Mulberry Symbols, Steve Lee, CC BY-SA 2.0 UK. Verbs are named "<verb>_,_to".
//   arasaac:  https://static.arasaac.org/pictograms/<id>/<id>_300.png
//             ARASAAC, Government of Aragon, CC BY-NC-SA 4.0. Gap-filler only.
//
// Hand-curation lives in PICS below. The ambiguity rule applies to every row: shown only the
// picture, a five-year-old says exactly this word. Reviewers remove rows that fail it.
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME = join(HERE, '..', 'docs', 'maes-words');
const IMG = join(GAME, 'img');
const CHECK = process.argv.includes('--check');

// { w, tier, src: 'mulberry' | 'arasaac', id, pos?: 'verb' | 'adj', noA?: true }
export const PICS = [
  // tier 1 nouns
  { w: 'mum', tier: 1, src: 'mulberry', id: 'mum_parent' },
  { w: 'dad', tier: 1, src: 'mulberry', id: 'dad_parent' },
  { w: 'boy', tier: 1, src: 'arasaac', id: 7176 },
  { w: 'girl', tier: 1, src: 'arasaac', id: 27509 },
  { w: 'cup', tier: 1, src: 'mulberry', id: 'cup_non_spill' },
  { w: 'mug', tier: 1, src: 'mulberry', id: 'mug_2' },
  { w: 'pot', tier: 1, src: 'mulberry', id: 'pot' },
  { w: 'lid', tier: 1, src: 'mulberry', id: 'lid' },
  { w: 'tap', tier: 1, src: 'mulberry', id: 'tap' },
  { w: 'cot', tier: 1, src: 'mulberry', id: 'cot' },
  { w: 'rug', tier: 1, src: 'mulberry', id: 'rug' },
  { w: 'jam', tier: 1, src: 'mulberry', id: 'jam', noA: true },
  { w: 'bun', tier: 1, src: 'mulberry', id: 'bun_currant' },
  { w: 'bib', tier: 1, src: 'mulberry', id: 'bib' },
  { w: 'wig', tier: 1, src: 'mulberry', id: 'wig_1' },
  { w: 'jar', tier: 1, src: 'mulberry', id: 'jar' },
  { w: 'rat', tier: 1, src: 'mulberry', id: 'rat' },
  { w: 'van', tier: 1, src: 'mulberry', id: 'van' },
  { w: 'hen', tier: 1, src: 'arasaac', id: 2403 },
  { w: 'nut', tier: 1, src: 'arasaac', id: 2785 },
  { w: 'pan', tier: 1, src: 'mulberry', id: 'frying_pan' },
  { w: 'mud', tier: 1, src: 'arasaac', id: 5908, noA: true },
  { w: 'peg', tier: 1, src: 'mulberry', id: 'clothes_peg' },
  // tier 2 nouns
  { w: 'nest', tier: 2, src: 'mulberry', id: 'nest' },
  { w: 'lamp', tier: 2, src: 'mulberry', id: 'lamp' },
  // verbs (Mulberry names verbs "<verb>_,_to")
  { w: 'run',  tier: 1, src: 'mulberry', id: 'run_,_to', pos: 'verb' },
  { w: 'sit',  tier: 1, src: 'mulberry', id: 'sit_,_to', pos: 'verb' },
  { w: 'hop',  tier: 1, src: 'mulberry', id: 'hop_,_to', pos: 'verb' },
  { w: 'dig',  tier: 1, src: 'mulberry', id: 'dig_,_to', pos: 'verb' },
  { w: 'eat',  tier: 1, src: 'mulberry', id: 'eat_,_to', pos: 'verb' },
  { w: 'cut',  tier: 1, src: 'mulberry', id: 'cut', pos: 'verb' },
  { w: 'hug',  tier: 1, src: 'mulberry', id: 'hug_,_to', pos: 'verb' },
  { w: 'jump', tier: 2, src: 'mulberry', id: 'jump_,_to', pos: 'verb' },
  { w: 'swim', tier: 2, src: 'mulberry', id: 'swim_,_to', pos: 'verb' },
  { w: 'read', tier: 3, src: 'mulberry', id: 'read_,_to', pos: 'verb' },
  { w: 'look', tier: 3, src: 'mulberry', id: 'look_,_to', pos: 'verb' },
  { w: 'help', tier: 2, src: 'mulberry', id: 'help_,_to', pos: 'verb' },
  { w: 'play', tier: 3, src: 'mulberry', id: 'play_,_to', pos: 'verb' },
  // adjectives
  { w: 'hot', tier: 1, src: 'mulberry', id: 'hot', pos: 'adj' },
  { w: 'wet', tier: 1, src: 'mulberry', id: 'wet', pos: 'adj' },
  { w: 'sad', tier: 1, src: 'arasaac', id: 2606, pos: 'adj' },
  { w: 'big', tier: 1, src: 'arasaac', id: 4658, pos: 'adj' },
  { w: 'bad', tier: 1, src: 'mulberry', id: 'bad', pos: 'adj' },
  { w: 'old', tier: 1, src: 'mulberry', id: 'old_person_1', pos: 'adj' },
  { w: 'up',   tier: 1, src: 'mulberry', id: 'up', pos: 'adj' },
  { w: 'down', tier: 2, src: 'mulberry', id: 'down', pos: 'adj' },
];

const urlFor = ({ src, id }) => src === 'mulberry'
  ? `https://raw.githubusercontent.com/mulberrysymbols/mulberry-symbols/master/EN/${encodeURIComponent(id)}.svg`
  : `https://static.arasaac.org/pictograms/${id}/${id}_300.png`;
const fileFor = ({ w, src }) => `${w}.${src === 'mulberry' ? 'svg' : 'png'}`;

/** Whitespace/comment/metadata stripping only — the symbol itself is unmodified (share-alike is not triggered). */
function optimiseSvg(text) {
  return text
    .replace(/<\?xml[^>]*\?>\s*/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<metadata>[\s\S]*?<\/metadata>/g, '')
    .replace(/>\s+</g, '><')
    .trim();
}

async function fetchAsset(row) {
  const res = await fetch(urlFor(row));
  if (!res.ok) throw new Error(`${row.w}: ${res.status} for ${urlFor(row)}`);
  return row.src === 'mulberry' ? Buffer.from(optimiseSvg(await res.text())) : Buffer.from(await res.arrayBuffer());
}

const q = s => `'${s.replace(/'/g, "\\'")}'`;
const entryLine = r => {
  const extra = (r.pos ? `, pos: ${q(r.pos)}` : '') + (r.noA ? ', noA: true' : '');
  return `  { w: ${q(r.w)}, tier: ${r.tier}, pic: { kind: 'svg', src: ${q('img/' + fileFor(r))} }${extra} },`;
};

function replaceBlock(text, begin, end, body) {
  const a = text.indexOf(begin), b = text.indexOf(end);
  if (a < 0 || b < 0 || b < a) throw new Error(`markers ${begin} / ${end} not found`);
  return text.slice(0, a + begin.length) + '\n' + body + '\n' + text.slice(b);
}

const SOURCE_NAME = { mulberry: 'Mulberry Symbols (CC BY-SA 2.0 UK)', arasaac: 'ARASAAC (CC BY-NC-SA 4.0)' };
const creditsRows = () => PICS.map(r =>
  `<tr><td><img src="img/${fileFor(r)}" alt="" width="64" height="64"></td><td>${r.w}</td><td>${SOURCE_NAME[r.src]}</td><td><a href="${urlFor(r)}">${r.id}</a></td></tr>`).join('\n');

async function main() {
  const seen = new Set();
  for (const r of PICS) {
    if (!/^[a-z]{2,4}$/.test(r.w)) throw new Error(`bad word ${r.w}`);
    if (seen.has(r.w)) throw new Error(`duplicate ${r.w}`);
    seen.add(r.w);
  }
  const wordsPath = join(GAME, 'words.js');
  const creditsPath = join(GAME, 'credits.html');
  const words = readFileSync(wordsPath, 'utf8');
  const credits = readFileSync(creditsPath, 'utf8');
  const nextWords = replaceBlock(words, '// BEGIN GENERATED PICS', '// END GENERATED PICS', PICS.map(entryLine).join('\n'));
  const nextCredits = replaceBlock(credits, '<!-- BEGIN GENERATED CREDITS -->', '<!-- END GENERATED CREDITS -->', creditsRows());
  const wanted = new Set(PICS.map(fileFor));
  const missing = PICS.filter(r => !existsSync(join(IMG, fileFor(r))));
  if (CHECK) {
    const changed = nextWords !== words || nextCredits !== credits || missing.length > 0;
    if (changed) { console.error('generated output is stale; run without --check'); process.exit(1); }
    return;
  }
  mkdirSync(IMG, { recursive: true });
  for (const r of missing) {
    writeFileSync(join(IMG, fileFor(r)), await fetchAsset(r));
    console.log('fetched', fileFor(r));
  }
  for (const f of readdirSync(IMG)) if (/\.(svg|png)$/.test(f) && !wanted.has(f)) { unlinkSync(join(IMG, f)); console.log('removed', f); }
  writeFileSync(wordsPath, nextWords);
  writeFileSync(creditsPath, nextCredits);
  console.log(`${PICS.length} pictogram words`);
}

main().catch(e => { console.error(e.message); process.exit(1); });
```

Before running, **verify each `id`** exists: for Mulberry, `curl -sI <url> | head -1` must be 200; for ARASAAC, open `https://api.arasaac.org/api/pictograms/en/search/<word>` and pick the id whose first keyword is the word. The ids above were spot-checked (`mum_parent`, `run_,_to`, 2458 for "mum" on ARASAAC, 2403 for "hen") but every row must be confirmed by the implementer and the failing fetch message names the row.

- [ ] **Step 4: Add markers to `words.js` and write `credits.html`**

In `words.js`, before `export const FRAMES`, add:

```js
  // BEGIN GENERATED PICS — edit docs-site/tools/generate-maes-pics.mjs, not this block
  // END GENERATED PICS
];
```

(and remove the `];` that previously closed `WORDS`, so the generated block sits inside the array).

`credits.html`:

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mae's Words — picture credits</title>
<link rel="stylesheet" href="style.css">
<style>
  main { align-items: stretch; max-width: 720px; margin: 0 auto; }
  table { width: 100%; border-collapse: collapse; font-size: 16px; }
  td, th { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--soft); vertical-align: middle; }
  img { display: block; }
</style>
</head>
<body>
<header><h1>Picture credits</h1><a href="./" style="color:#fff">Back to the game</a></header>
<main>
<p>Mae's Words is a free family reading game. Most pictures are emoji rendered by your device. The pictograms below come from two open symbol sets, used unmodified:</p>
<ul>
  <li><strong>Mulberry Symbols</strong> — © Steve Lee, <a href="https://creativecommons.org/licenses/by-sa/2.0/uk/">CC BY-SA 2.0 UK</a>. <a href="https://mulberrysymbols.org">mulberrysymbols.org</a></li>
  <li><strong>ARASAAC</strong> — pictographic symbols © Government of Aragon, created by Sergio Palao, <a href="https://creativecommons.org/licenses/by-nc-sa/4.0/">CC BY-NC-SA 4.0</a>. <a href="https://arasaac.org">arasaac.org</a>. ARASAAC's licence is non-commercial; this game is free and not for sale.</li>
</ul>
<p>Each pictogram below links to its source.</p>
<table>
<thead><tr><th></th><th>Word</th><th>Source</th><th>Symbol</th></tr></thead>
<tbody>
<!-- BEGIN GENERATED CREDITS -->
<!-- END GENERATED CREDITS -->
</tbody>
</table>
</main>
</body>
</html>
```

- [ ] **Step 5: Run the generator, then the tests**

```bash
node docs-site/tools/generate-maes-pics.mjs 2>&1 | tail -5
node docs-site/tools/generate-maes-pics.mjs --check && echo STABLE
du -sh docs-site/docs/maes-words/img
node --test 'docs-site/tests/maes-words.*.test.mjs' 2>&1 | tail -8
```

Expected: `STABLE`, img/ well under 6 MB, all tests pass **except** `the bank has grown` if fewer than 150 words — extend `PICS` (see Step 7) until it passes.

- [ ] **Step 6: Credits link in the Grown-up corner**

`index.html`, inside `.grownup-actions` before `#reset-box`: `<p class="muted"><a href="credits.html">Picture credits</a></p>`.

- [ ] **Step 7: Ambiguity review pass**

Serve the site and open `http://localhost:8000/maes-words/credits.html` — the table doubles as a contact sheet. For each row apply the ambiguity rule. Drop rows that fail (e.g. an AAC "up" arrow may read as "arrow"; `old_person_1` may read as "grandad"). Add rows from the spec's candidate list until `WORDS.length >= 150`; candidates that emoji already covers unambiguously may be added as emoji entries by hand **outside** the generated block (`kid` 🧒? — no, ambiguous with "child"; `toy` 🧸? — no, "bear"; prefer pictograms for these). Re-run the generator after every table edit; commit the `img/` files.

- [ ] **Step 8: Commit**

```bash
git add docs-site && git commit -m "feat(maes-words): pictogram generator, vendored Mulberry/ARASAAC images, credits page" && git push
```

---

### Task 6: Sight bank, per-kind trickle, Fill turn construction

**Files:**
- Modify: `docs-site/docs/maes-words/words.js` (`SIGHT`)
- Modify: `docs-site/docs/maes-words/game.js`
- Modify: `docs-site/docs/maes-words/storage.js` (no code change; callers pass the combined bank)
- Test: `docs-site/tests/maes-words.game.test.mjs`, `maes-words.turns.test.mjs`, `maes-words.words.test.mjs`

**Interfaces:**
- Produces:
  - `SIGHT: { w, tier, sight: true }[]`; `isSight(word) -> boolean`.
  - All `game.js` functions keep their signatures and take the **combined** bank `[...WORDS, ...SIGHT]` as `words`. Internally they split by `isSight`.
  - `initialState(bank)` introduces the first 6 tier-1 nouns and first 2 tier-1 sight words.
  - `introduceWords(state, bank)` trickles nouns (floor 6, batch 2) and sight words (floor 2, batch 1) independently; `unlockedTier(state, bankSubset)` is applied per kind.
  - `activityFor(word, level) -> 'read'|'pick'|'build'|'sentence'|'fill'`; `SIGHT_ACTIVITY_FOR_LEVEL = ['fill','fill','build','fill']`.
  - `makeTurn(word, level, bank, frames, rng, ctx = {})` where `ctx.introduced` is a `Set` of introduced word names (used to pick Fill nouns); Fill turns are `{ activity: 'fill', word: sightEntry, frame: string, noun: wordEntry, options: sightEntry[], capitalise: boolean }` where `frame` has one `{}` (the sight blank) and one `{noun}` (where the picture goes).
  - `distractors()` for a sight word draws only from `SIGHT`; for a noun/verb/adj only from `WORDS`.
  - `fillDistractors(sight, frame, sightBank, level, n)` — helper exported for tests.
  - `refreshTurn` and `planRound` pass `ctx.introduced` built from `state.levels`.

- [ ] **Step 1: Failing tests**

`maes-words.words.test.mjs`:

```js
import { WORDS, SIGHT, FRAMES } from '../docs/maes-words/words.js';

test('sight words are 2-4 lowercase letters, unique, tiered, flagged, and disjoint from WORDS', () => {
  assert.ok(SIGHT.length >= 20, `only ${SIGHT.length} sight words`);
  const ws = new Set(WORDS.map(x => x.w));
  const seen = new Set();
  for (const s of SIGHT) {
    assert.match(s.w, /^[a-z]{2,4}$/, `bad sight word ${s.w}`);
    assert.equal(s.sight, true, `${s.w} not flagged`);
    assert.ok([1, 2, 3].includes(s.tier), `bad tier ${s.w}`);
    assert.ok(!ws.has(s.w), `${s.w} is in both banks`);
    assert.ok(!seen.has(s.w), `duplicate sight ${s.w}`); seen.add(s.w);
  }
  assert.ok(SIGHT.filter(s => s.tier === 1).length >= 6, 'tier-1 sight too small');
});

test('every frame fit is a SIGHT word, and every sight word fits at least one frame', () => {
  const sight = new Set(SIGHT.map(s => s.w));
  for (const f of FRAMES) for (const s of f.fits) assert.ok(sight.has(s), `${s} in fits of "${f.text}" is not a SIGHT word`);
  for (const s of SIGHT) assert.ok(FRAMES.some(f => f.fits.includes(s.w)), `no frame for sight word ${s.w}`);
});
```

`maes-words.game.test.mjs` (extend the local bank):

```js
import { SIGHT_INITIAL_INTRO, SIGHT_INTRO_BATCH, SIGHT_NEW_WORD_FLOOR, isSight } from '../docs/maes-words/game.js';
const SIGHTS = [...['the', 'is', 'my', 'in', 'on', 'it'].map(w => ({ w, tier: 1, sight: true })),
                ...['here', 'like', 'see', 'can'].map(w => ({ w, tier: 2, sight: true }))];
const ALL = [...BANK, ...SIGHTS];

test('initialState introduces 6 tier-1 nouns and 2 tier-1 sight words', () => {
  const s = initialState(ALL);
  assert.deepEqual(Object.keys(s.levels), ['cat', 'dog', 'sun', 'bed', 'pig', 'bus', 'the', 'is']);
  assert.equal(SIGHT_INITIAL_INTRO, 2);
});

test('sight words trickle independently: one at a time when fewer than 2 are at level 0', () => {
  const s0 = withLevels({ cat: 0, dog: 0, sun: 0, bed: 0, pig: 0, bus: 0, the: 1, is: 0 });
  const s1 = introduceWords(s0, ALL);
  assert.deepEqual(Object.keys(s1.levels).filter(w => isSight(ALL.find(x => x.w === w))), ['the', 'is', 'my']);
  assert.equal(Object.keys(s1.levels).length, 9, 'nouns untouched because 6 sit at level 0');
  assert.equal(SIGHT_INTRO_BATCH, 1); assert.equal(SIGHT_NEW_WORD_FLOOR, 2);
});

test('sight tier 2 unlocks on the 75% rule over SIGHT only', () => {
  const s = withLevels({ the: 2, is: 2, my: 2, in: 2, on: 2, it: 0, cat: 0 });  // 5/6 tier-1 sight mastered
  const next = introduceWords(s, ALL);
  assert.ok('here' in next.levels, 'tier-2 sight word introduced');
});
```

`maes-words.turns.test.mjs`:

```js
import { activityFor, SIGHT_ACTIVITY_FOR_LEVEL, fillDistractors } from '../docs/maes-words/game.js';
const SIGHTS = ['the', 'is', 'my', 'in', 'on', 'it', 'here', 'like', 'see', 'can', 'this', 'up', 'have', 'was'].map(w => ({ w, tier: 1, sight: true }));
const ALL = [...BANK, ...SIGHTS];
const sightByName = w => SIGHTS.find(x => x.w === w);

test('activityFor: nouns use the noun ladder, sight words the sight ladder', () => {
  assert.deepEqual([0, 1, 2, 3].map(l => activityFor(byName('cat'), l)), ['read', 'pick', 'build', 'sentence']);
  assert.deepEqual([0, 1, 2, 3].map(l => activityFor(sightByName('the'), l)), SIGHT_ACTIVITY_FOR_LEVEL);
  assert.deepEqual(SIGHT_ACTIVITY_FOR_LEVEL, ['fill', 'fill', 'build', 'fill']);
});

test('a Fill turn blanks the sight word, fills a noun, and offers exactly one fitting option', () => {
  const introduced = new Set(['cat', 'dog', 'egg']);
  for (let seed = 1; seed <= 40; seed++) {
    const t = makeTurn(sightByName('like'), 0, ALL, FRAMES, seeded(seed), { introduced });
    assert.equal(t.activity, 'fill');
    assert.equal((t.frame.match(/\{\}/g) || []).length, 1, t.frame);
    assert.equal((t.frame.match(/\{noun\}/g) || []).length, 1, t.frame);
    assert.ok(introduced.has(t.noun.w), `noun ${t.noun.w} not introduced`);
    assert.equal(t.options.length, 3);
    assert.equal(t.options.filter(o => o.w === 'like').length, 1);
    assert.ok(t.fits.includes('like'));
    for (const o of t.options) if (o.w !== 'like') assert.ok(!t.fits.includes(o.w), `${o.w} also fits "${t.frame}"`);
    assert.equal(isCorrect(t, 'like'), true);
    assert.equal(isCorrect(t, 'see'), false);
  }
});

test('fill distractors are never in the frame\'s fits', () => {
  const frame = { text: 'I {sight} the {noun}', fits: ['like', 'see', 'have'] };
  for (const level of [0, 1, 3]) {
    const d = fillDistractors(sightByName('like'), frame, SIGHTS, level, 2).map(x => x.w);
    assert.equal(d.length, 2);
    for (const w of d) assert.ok(!frame.fits.includes(w), `${w} fits`);
  }
});

test('fill distractors at level 0 differ in length; at level 1 they are look-alikes', () => {
  const frame = { text: 'The {noun} is {sight}', fits: ['here', 'up', 'in'] };
  const easy = fillDistractors(sightByName('it'), frame, SIGHTS, 0, 2).map(x => x.w);
  assert.ok(easy.every(w => w.length !== 2), `easy ${easy}`);
  const hard = fillDistractors(sightByName('it'), frame, SIGHTS, 1, 2).map(x => x.w);
  assert.ok(hard.includes('is'), `hard ${hard}`);   // 'in' fits, so excluded; 'is' is the closest remaining
});

test('a Fill noun obeys the frame\'s article', () => {
  const introduced = new Set(['egg', 'ox']);            // vowel-initial only
  for (let seed = 1; seed <= 40; seed++) {
    const t = makeTurn(sightByName('here'), 0, ALL, FRAMES, seeded(seed), { introduced });
    assert.doesNotMatch(t.frame, /\ba \{noun\}/, t.frame);
  }
});

test('a Fill turn capitalises options when the blank opens the sentence', () => {
  const frames = [{ text: '{sight} is a {noun}', fits: ['here', 'this'] }];
  const t = makeTurn(sightByName('here'), 0, ALL, frames, seeded(1), { introduced: new Set(['cat']) });
  assert.equal(t.capitalise, true);
  assert.equal(t.frame, '{} is a {noun}');
});

test('sight-word distractors come only from SIGHT and noun distractors only from WORDS', () => {
  assert.ok(distractors(sightByName('the'), ALL, 3).every(x => x.sight));
  assert.ok(distractors(byName('cat'), ALL, 3).every(x => !x.sight));
});

test('letterTray for a sight word never spells another word of either bank', () => {
  const bank = [...ALL, { w: 'then', tier: 2, sight: true }, { w: 'than', tier: 2, sight: true }];
  for (let seed = 1; seed <= 100; seed++) {
    const tray = letterTray(sightByName('the'), bank, seeded(seed));
    const extra = tray.filter(ch => !'the'.includes(ch));
    assert.equal(extra.length, 1);
    assert.ok(!['n'].includes(extra[0]), `seed ${seed} admitted n`);
  }
});

test('planRound mixes nouns and sight words and refreshTurn uses the sight ladder', () => {
  const levels = { cat: 0, dog: 0, sun: 0, the: 0, is: 0 };
  const turns = planRound(withLevels(levels), ALL, FRAMES, seeded(3));
  assert.equal(turns.length, ROUND_LENGTH);
  assert.ok(turns.some(t => t.activity === 'fill'));
  const fill = turns.find(t => t.activity === 'fill');
  const moved = refreshTurn(fill, withLevels({ ...levels, [fill.word.w]: 2 }), ALL, FRAMES, seeded(4));
  assert.equal(moved.activity, 'build');
});
```

- [ ] **Step 2: Run** → FAIL widely (`SIGHT`, `isSight`, `activityFor`, `fillDistractors` missing).

- [ ] **Step 3: Implement `SIGHT` in `words.js`**

```js
// Sight words: taught by Fill, never pictured. `a` and `I` appear only as frame text.
export const SIGHT = [
  ...['the', 'is', 'and', 'my', 'in', 'on', 'up', 'it', 'at', 'go', 'we', 'to'].map(w => ({ w, tier: 1, sight: true })),
  ...['here', 'like', 'see', 'you', 'has', 'can', 'this', 'are', 'was'].map(w => ({ w, tier: 2, sight: true })),
  ...['look', 'come', 'for', 'said', 'they', 'with', 'have'].map(w => ({ w, tier: 3, sight: true })),
];
```

Then extend `FRAMES` so **every** sight word fits at least one frame (the words test enforces it). Add frames such as:

```js
  { text: 'The {noun} {sight} big',     fits: ['is', 'was'] },
  { text: 'Come {sight} see the {noun}', fits: ['and'] },
  { text: '{sight} like the {noun}',    fits: ['we', 'you', 'they'] },
  { text: 'Look {sight} the {noun}',    fits: ['at'] },
  { text: 'I {sight} to the {noun}',    fits: ['go', 'come'] },
  { text: 'This is {sight} the {noun}', fits: ['for'] },
  { text: '{sight} {noun} is here',     fits: ['my', 'the', 'this'] },
  { text: 'I {sight} a {noun}',         fits: ['see', 'like', 'have'] },
  { text: 'The {noun} {sight} a hat',   fits: ['has'] },
  { text: 'Here {sight} the {noun}',    fits: ['is', 'are'] },
  { text: 'I {sight} {noun}',           fits: ['can'], pos: 'verb' },
  { text: 'We {sight} {noun}',          fits: ['can'], pos: 'verb' },   // not 'like'/'to': "We like jump" is wrong
  { text: 'I {sight} see the {noun}',   fits: ['can'] },
  { text: 'The {noun} is {sight} the box', fits: ['in', 'on'] },
  { text: '{sight} is a {noun}',        fits: ['here', 'this', 'it'] },
  { text: 'The {noun} {sight} up',      fits: ['is', 'was'] },
  { text: 'The {noun} {sight} with me', fits: ['is', 'was'] },
  { text: '"Look," {sight} the {noun}', fits: ['said'] },
  { text: 'The {noun} is {sight} me',   fits: ['with', 'for'] },
```

Every `fits` list must contain **every** sight word that yields a grammatical sentence; the implementer reads each frame aloud with each SIGHT word substituted and adds any that fit.

- [ ] **Step 4: Implement in `game.js`**

```js
export const SIGHT_INITIAL_INTRO = 2;
export const SIGHT_INTRO_BATCH = 1;
export const SIGHT_NEW_WORD_FLOOR = 2;
export const SIGHT_ACTIVITY_FOR_LEVEL = Object.freeze(['fill', 'fill', 'build', 'fill']);
export const isSight = word => word.sight === true;
const nouns = bank => bank.filter(x => !isSight(x));
const sights = bank => bank.filter(isSight);

export function activityFor(word, level) {
  const ladder = isSight(word) ? SIGHT_ACTIVITY_FOR_LEVEL : ACTIVITY_FOR_LEVEL;
  return ladder[Math.min(level, ladder.length - 1)];
}

export function initialState(words) {
  const s = emptyState();
  for (const word of nouns(words).filter(x => x.tier === 1).slice(0, INITIAL_INTRO)) s.levels[word.w] = 0;
  for (const word of sights(words).filter(x => x.tier === 1).slice(0, SIGHT_INITIAL_INTRO)) s.levels[word.w] = 0;
  return s;
}

function trickle(state, pool, floor, batch) {
  const atZero = pool.filter(x => state.levels[x.w] === 0).length;
  if (atZero >= floor) return state;
  const tier = unlockedTier(state, pool);
  const fresh = pool.filter(x => x.tier <= tier && !isIntroduced(state, x.w)).slice(0, batch);
  if (fresh.length === 0) return state;
  const next = cloneState(state);
  for (const word of fresh) next.levels[word.w] = 0;
  return next;
}

export function introduceWords(state, words) {
  const afterNouns = trickle(state, nouns(words), NEW_WORD_FLOOR, INTRO_BATCH);
  return trickle(afterNouns, sights(words), SIGHT_NEW_WORD_FLOOR, SIGHT_INTRO_BATCH);
}
```

`unlockedTier(state, words)` is unchanged (it is now called with a per-kind subset). Note `trickle` counts level-0 words **of its own kind** — this changes the existing `introduceWords` semantics only in that sight words no longer inflate the noun count; the existing noun tests keep passing because their banks have no sight words.

`distractors`: restrict the pool to the word's kind before the colour rule:

```js
export function distractors(word, words, n) {
  const kind = words.filter(x => isSight(x) === isSight(word));
  const isColour = x => x.pic?.kind === 'colour';
  const pool = isColour(word) && kind.filter(x => isColour(x) && x.w !== word.w).length >= n ? kind.filter(isColour) : kind;
  // ... existing ranking over `pool`
```

`letterTray`: `others` stays over the whole bank (both kinds) — that is what makes the `then` test pass.

Fill:

```js
/** Sight words that do NOT fit the frame; easy (level 0) = different length first; otherwise closest by edit distance. */
export function fillDistractors(sight, frame, sightBank, level, n) {
  const pool = sightBank.filter(x => x.w !== sight.w && !frame.fits.includes(x.w));
  if (level === 0) {
    const ranked = pool.map((x, i) => ({ x, i }))
      .sort((p, q) => (p.x.w.length === sight.w.length) - (q.x.w.length === sight.w.length)
        || Math.abs(q.x.w.length - sight.w.length) - Math.abs(p.x.w.length - sight.w.length)
        || p.i - q.i);
    return ranked.slice(0, n).map(({ x }) => x);
  }
  return distractors(sight, pool.concat([sight]), n);
}

function makeFillTurn(sight, level, words, frames, rng, ctx) {
  const usableFrames = frames.filter(f => f.fits.includes(sight.w));
  const frame = usableFrames[Math.floor(rng() * usableFrames.length)];
  const pos = frame.pos ?? 'noun';
  const introduced = ctx.introduced ?? new Set(nouns(words).map(x => x.w));
  let pool = nouns(words).filter(x => posOf(x) === pos && introduced.has(x.w) && framesFor(x, [frame]).includes(frame));
  if (pool.length === 0) pool = nouns(words).filter(x => posOf(x) === pos && framesFor(x, [frame]).includes(frame));
  if (pool.length === 0) pool = nouns(words).filter(x => posOf(x) === pos);
  const noun = pool[Math.floor(rng() * pool.length)];
  return {
    activity: 'fill', word: sight, noun, fits: frame.fits,
    frame: frame.text.replace('{sight}', '{}'),
    capitalise: frame.text.startsWith('{sight}'),
    options: shuffle([sight, ...fillDistractors(sight, frame, sights(words), level, 2)], rng),
  };
}
```

`framesFor(x, [frame]).includes(frame)` is the article check; note `framesFor` falls back to returning all frames when nothing passes, so guard: replace that call with a direct test `frameAdmits(x, frame)`:

```js
export const frameAdmits = (word, frame) =>
  (frame.pos ?? 'noun') === posOf(word) && !((word.noA || /^[aeiou]/.test(word.w)) && /\ba \{noun\}/.test(frame.text));
```

and use `frameAdmits(x, frame)` in the pool filters above (and refactor `framesFor` to use it).

`makeTurn` gains the `ctx = {}` parameter and dispatches on `activityFor(word, level)`, with `case 'fill': return makeFillTurn(word, level, words, frames, rng, ctx);`. `isCorrect` is unchanged (`answer === turn.word.w`). `refreshTurn` compares `activityFor(turn.word, level) === turn.activity` and passes `{ introduced: new Set(Object.keys(state.levels)) }`. `planRound` passes the same ctx to every `makeTurn`.

- [ ] **Step 5: Run all tests** → PASS (fix ordering in the level-0 ranking until `easy` test passes; the intent is "different length first, then bank order").

- [ ] **Step 6: Commit**

```bash
git add docs-site && git commit -m "feat(maes-words): sight-word bank, per-kind trickle, Fill turn construction" && git push
```

---

### Task 7: Fill UI, combined bank wiring, headless script

**Files:**
- Modify: `docs-site/docs/maes-words/ui.js`
- Modify: `docs-site/docs/maes-words/index.html` (Grown-up legend text; a "Sight words" heading in the table)
- Modify: `docs-site/docs/maes-words/style.css`
- Create: `docs-site/tests/maes-words-headless.mjs` (manual script, not matched by the test glob)

**Interfaces:**
- Consumes: `SIGHT`, `activityFor`, Fill turn shape `{ activity: 'fill', word, noun, frame, capitalise, options }` from Task 6.

- [ ] **Step 1: Wire the combined bank**

In `ui.js`: `import { WORDS, SIGHT, FRAMES } from './words.js'; const ALL = [...WORDS, ...SIGHT];` and replace every `WORDS` argument to `load`, `initialState`, `introduceWords`, `planRound`, `refreshTurn`, `learnedWords` with `ALL`. `renderGrownup` iterates `WORDS` then `SIGHT` with a `<tr class="section"><td colspan="3">Sight words</td></tr>` between.

- [ ] **Step 2: `renderFill`**

```js
function renderFill(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt sentence';
  prompt.innerHTML = '';
  for (const part of turn.frame.split(/(\{\}|\{noun\})/)) {
    if (part === '{}') {
      const blank = document.createElement('span');
      blank.className = 'blank sight-blank';
      blank.textContent = '    ';
      prompt.append(blank);
    } else if (part === '{noun}') {
      const n = document.createElement('span');
      n.className = 'blank';
      n.append(renderPic(turn.noun.pic, { size: 'prompt' }));
      prompt.append(n);
    } else if (part) {
      prompt.append(document.createTextNode(part));
    }
  }
  const opts = $('options');
  opts.className = 'options three';
  opts.innerHTML = '';
  for (const o of turn.options) {
    const label = turn.capitalise ? o.w.charAt(0).toUpperCase() + o.w.slice(1) : o.w;
    const b = tile(label);
    b.dataset.w = o.w;                       // answer compares the lowercase word
    b.addEventListener('click', () => answer(o.w, b));
    opts.append(b);
  }
}
const RENDER = { read: renderRead, pick: renderPick, build: renderBuild, sentence: renderSentence, fill: renderFill };
```

On a correct Fill answer, also write the word into the blank: in `answer()`, when `correct && turn.activity === 'fill'`, set `$('prompt').querySelector('.sight-blank').textContent = chosenEl.textContent`.

`renderBuild` for a sight word: `turn.word.pic` is undefined, so the prompt must show the **word** instead — a Build for a sight word shows the frame? No: keep it simple and honest to the spec ("Build to spell it"): the prompt shows the sight word in large type for ~1.2 s, then hides it and reveals the tray (a brief look-then-spell). Implement: `prompt.className = 'prompt'; prompt.textContent = turn.word.w;` then `setTimeout(() => { prompt.textContent = ''; slotsEl.hidden = false; opts.hidden = false; }, 1200)` with the slots/tray hidden until then. For pictured words, behaviour is unchanged.

CSS: `.sight-blank { border-bottom: 5px solid var(--pink); min-width: 96px; display: inline-block; }` and `#grownup-table tr.section td { font-weight: 800; color: var(--pink-dark); padding-top: 14px; }`.

- [ ] **Step 3: Grown-up legend**

`index.html` legend paragraph: "Mae's progress, word by word. Pictured words: 0 new · 1 reads it · 2 picks it from look-alikes · 3 learned. Sight words: 0 new · 1 picks it in a sentence · 2 tells it from look-alikes · 3 learned."

- [ ] **Step 4: Headless script** — `docs-site/tests/maes-words-headless.mjs`:

```js
// Manual: NODE_PATH=docs/demos/node_modules node docs-site/tests/maes-words-headless.mjs
// Assumes `cd docs-site/docs && python3 -m http.server 8000` is already running.
import { createRequire } from 'node:module';
const { chromium } = createRequire(import.meta.url)('playwright');
import assert from 'node:assert/strict';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
const errors = [];
page.on('pageerror', e => errors.push(String(e)));
await page.goto('http://localhost:8000/maes-words/');

// Force a state that reaches every activity: nouns at 0..3, one sight word at 0, one at 2.
await page.evaluate(() => localStorage.setItem('maes-words:v1', JSON.stringify({
  version: 1, stars: 0, learnedOrder: ['dog'],
  levels: { cat: 0, dog: 3, sun: 1, bed: 2, pig: 0, bus: 0, the: 0, is: 2 } })));
await page.reload();
await page.click('#play-btn');

const seen = new Set();
for (let i = 0; i < 10; i++) {
  await page.waitForSelector('#options .tile, #options .tray .tile');
  const cls = await page.locator('#options').getAttribute('class');
  const isBuild = cls.includes('tray');
  if (isBuild) {
    seen.add('build');
    await page.waitForSelector('#slots:not([hidden])');
    // Deliberately wrong: tap tiles in tray order until slots are full.
    const n = await page.locator('#slots .slot').count();
    for (let k = 0; k < n; k++) await page.locator('#options .tile:not(.used)').first().click();
    await page.waitForTimeout(700);
    const stillFilled = await page.locator('#slots .slot.filled').count();
    assert.ok(stillFilled === 0 || stillFilled === n, 'after a wrong build the slots either empty (retry) or reveal');
    if (stillFilled === 0) { for (let k = 0; k < n; k++) await page.locator('#options .tile:not(.used)').first().click(); }
    await page.waitForTimeout(1700);
    continue;
  }
  const isFill = await page.locator('#prompt .sight-blank').count() > 0;
  seen.add(isFill ? 'fill' : (await page.locator('#prompt .pic').count()) ? 'pick-or-sentence' : 'read');
  // Wrong first, then right — asserts the retry path on every tile activity.
  const tiles = page.locator('#options .tile');
  const words = await tiles.evaluateAll(els => els.map(e => e.dataset.w));
  const before = Number(await page.locator('#star-count').innerText());
  await tiles.nth(0).click();
  const disabledAfterFirst = await tiles.nth(0).evaluate(e => e.disabled);
  assert.ok(disabledAfterFirst, 'tapped tile disables');
  const live = await tiles.evaluateAll(els => els.filter(e => !e.disabled).length);
  if (live > 0) {                       // first tap was wrong: pick each remaining until it advances
    for (let k = 1; k < words.length; k++) {
      const t = tiles.nth(k);
      if (await t.evaluate(e => e.disabled)) continue;
      await t.click();
      if (await page.locator('#options .tile.correct').count()) break;
    }
    const after = Number(await page.locator('#star-count').innerText());
    assert.equal(after, before, 'a fixed turn earns no star');
  }
  await page.waitForTimeout(1100);
}
assert.deepEqual(errors, []);
assert.ok(seen.has('fill'), `no fill turn seen: ${[...seen]}`);
console.log('ok', [...seen]);
await browser.close();
```

Run it (server started with the `( … &)` form, killed after). Fix anything it finds. Also open `credits.html` in the headless page and assert no `pageerror`.

- [ ] **Step 5: Run all unit tests, then commit**

```bash
node --test 'docs-site/tests/maes-words.*.test.mjs' 2>&1 | tail -8
git add docs-site && git commit -m "feat(maes-words): Fill activity UI, sight words in the grown-up corner, headless check" && git push
```

---

### Task 8: Docs, strict build, final review, merge

**Files:**
- Modify: `docs/superpowers/specs/2026-09-27-maes-words-design.md` (add `pos` on words and frames to the Word bank and Sight sections; add `fits`-exhaustiveness rule; note Build for a sight word shows the word briefly then hides it)
- Modify: `docs/handoffs/2026-09-27-maes-words-images-sight-words.md` (append a "Shipped" section: what landed, how to add words via the generator, the review sheet)
- Modify: `docs-site/docs/maes-words/index.html` (`<meta name="description">` mentions pictograms and sight words)

- [ ] **Step 1: Spec touch-up** — keep the timeless rule: describe `pos` and `fits` as they are, no task numbers.

- [ ] **Step 2: Strict build**

Run: `cd docs-site && uv run mkdocs build --strict 2>&1 | tail -5` → exit 0. `credits.html` and `img/` are non-markdown so they copy verbatim.

- [ ] **Step 3: Whole-branch review** — dispatch the final reviewer (Fable) over `git diff origin/main...maes-words-pics -- docs-site docs/superpowers` with the spec, focusing on: the ambiguity rule per pictogram (open the PR preview's `credits.html`), `fits` exhaustiveness per frame (read each frame with every SIGHT word), and the retry accounting.

- [ ] **Step 4: Rebase, fast-forward, deploy**

```bash
git fetch origin && git rebase origin/main && git push --force-with-lease
gh pr ready <N>
git checkout main && git pull --ff-only && git merge --ff-only maes-words-pics && git push origin main
gh run list --workflow Docs --limit 1     # wait for success
curl -s -o /dev/null -w "%{http_code}\n" https://smeltsql.com/maes-words/credits.html   # 200 once deployed
```

- [ ] **Step 5: Commit any doc follow-ups on main** (handoff "Shipped" section).

---

## Self-review notes

- Spec coverage: `pic` shape (T1), try-again (T2), templates/pos/Sentence (T3), colours (T4), sources/vendoring/generator/credits/ambiguity review (T5), SIGHT/ladder/trickle/Fill construction/distractor rules/letterTray safety (T6), Fill UI/wall/grown-up/headless (T7), spec `pos` amendment + deploy (T8). Storage needs no code change: `load(raw, ALL)` already drops unknown words.
- Type consistency: `makeTurn(word, level, words, frames, rng, ctx = {})` throughout; Fill turn `{ activity, word, noun, fits, frame, capitalise, options }`; `applyAttempt(state, word, correct, attempts)`; `retryAllowed(turn, attempts)`; `frameAdmits(word, frame)`; `resolveFrame(frame, sight)`.
- Review Focus 1–5 each have a named test in T1, T6, T6, T2/T7, T3.
