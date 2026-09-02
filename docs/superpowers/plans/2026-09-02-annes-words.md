# Anne's Words Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a near-pixel-perfect Wordle clone called "Anne's Words" as a single dependency-free page published at `https://smeltsql.com/annes-words/`.

**Architecture:** Plain ES modules under `docs-site/docs/annes-words/`, which MkDocs copies verbatim into the published site. Pure game logic lives in `game.js` with zero DOM access so it can be tested with `node --test`; everything touching `document`, `localStorage`, or the clock lives in `ui.js`.

**Tech Stack:** Vanilla HTML/CSS/ES modules. No build step, no npm, no framework, no runtime dependencies. Tests via node's built-in `node --test`.

**Spec:** `docs/superpowers/specs/2026-09-02-annes-words-design.md`

## Global Constraints

- Branch: `annes-words`. Commit AND push after every task.
- Zero dependencies. No npm, no CDN scripts, no build step. Anything the page needs is a file in `docs-site/docs/annes-words/`.
- The name is **Anne's Words** everywhere user-visible. Never "Wordle" in UI copy, page title, or share text.
- This work touches no Rust crate. Do not run `verify-phase.sh`, `cargo` anything, or update `docs/ROADMAP.md`. The repo's spec/plan workflow is explicitly waived for this feature.
- Tests live in `docs-site/tests/` — never under `docs-site/docs/`, which is published.
- Run tests with: `node --test 'docs-site/tests/*.test.mjs'` (quote the glob — on Node 24 a bare directory argument is resolved as a module and errors)
- Manual check at every task: `cd docs-site/docs && python3 -m http.server 8000`, open `http://localhost:8000/annes-words/`.
- Original Wordle palette, exact values: correct `#6aaa64`, present `#c9b458`, absent `#787c7e`, tile border idle `#d3d6da`, tile border filled `#878a8c`, key idle `#d3d6da`, key text `#1a1a1b`, page background `#ffffff`.

---

### Task 1: Word list data module

**Files:**
- Create: `docs-site/docs/annes-words/words.js`
- Create: `docs-site/tests/annes-words.words.test.mjs`

**Interfaces:**
- Consumes: nothing.
- Produces: `export const ANSWERS` — array of 2,315 lowercase 5-letter strings, shuffled with a fixed seed. `export const ALLOWED` — array of the 10,657 extra accepted guesses (does NOT include the answers; consumers union the two).

- [ ] **Step 1: Fetch the two canonical lists into the scratchpad**

```bash
cd /tmp && \
curl -sL -o answers.txt https://gist.githubusercontent.com/cfreshman/a03ef2cba789d8cf00c08f767e0fad7b/raw/wordle-answers-alphabetical.txt && \
curl -sL -o allowed.txt https://gist.githubusercontent.com/cfreshman/cdcdf777450c5b5301e439061d29694c/raw/wordle-allowed-guesses.txt && \
wc -l answers.txt allowed.txt
```

Expected: 2315 and 10657 lines.

- [ ] **Step 2: Generate `words.js` with a seeded shuffle**

The shuffle must be deterministic and done once, now, at authoring time — not at
runtime. Use this exact script so the output is reproducible:

```bash
cd /tmp && node -e '
const fs = require("fs");
const read = f => fs.readFileSync(f,"utf8").split("\n").map(s=>s.trim()).filter(s=>/^[a-z]{5}$/.test(s));
const answers = read("answers.txt");
const allowed = read("allowed.txt");
// mulberry32 PRNG, fixed seed -> reproducible Fisher-Yates
let s = 0x9e3779b9;
const rnd = () => { s |= 0; s = s + 0x6D2B79F5 | 0;
  let t = Math.imul(s ^ s >>> 15, 1 | s); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
  return ((t ^ t >>> 14) >>> 0) / 4294967296; };
for (let i = answers.length - 1; i > 0; i--) { const j = Math.floor(rnd()*(i+1)); [answers[i],answers[j]]=[answers[j],answers[i]]; }
const fmt = (name, arr) => `export const ${name} = ${JSON.stringify(arr)};\n`;
fs.writeFileSync("words.js",
  "// Generated once at authoring time. ANSWERS is seed-shuffled so the daily\n" +
  "// sequence is not alphabetical. ALLOWED holds extra accepted guesses only.\n" +
  fmt("ANSWERS", answers) + fmt("ALLOWED", allowed));
console.log(answers.length, allowed.length, answers.slice(0,3));
'
# copy into the repo using an ABSOLUTE path (the script above left you in /tmp)
cp /tmp/words.js "$(git -C ~/smelt-sql rev-parse --show-toplevel)/docs-site/docs/annes-words/words.js"
```

(Create the directory first: `mkdir -p docs-site/docs/annes-words docs-site/tests`.)

- [ ] **Step 3: Write the data-integrity test**

```js
// docs-site/tests/annes-words.words.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { ANSWERS, ALLOWED } from '../docs/annes-words/words.js';

test('answer list has the expected size and shape', () => {
  assert.equal(ANSWERS.length, 2315);
  assert.ok(ANSWERS.every(w => /^[a-z]{5}$/.test(w)), 'all answers are 5 lowercase letters');
  assert.equal(new Set(ANSWERS).size, ANSWERS.length, 'no duplicate answers');
});

test('allowed list has the expected size and does not overlap answers', () => {
  assert.equal(ALLOWED.length, 10657);
  assert.ok(ALLOWED.every(w => /^[a-z]{5}$/.test(w)));
  const answers = new Set(ANSWERS);
  assert.equal(ALLOWED.filter(w => answers.has(w)).length, 0);
});

test('answers are shuffled, not alphabetical', () => {
  const sorted = [...ANSWERS].sort();
  assert.notDeepEqual(ANSWERS.slice(0, 50), sorted.slice(0, 50));
});
```

- [ ] **Step 4: Run the test**

Run: `node --test 'docs-site/tests/*.test.mjs'`
Expected: 3 passing tests. If "no overlap" fails, the two source lists changed —
subtract the answers from ALLOWED in the generator rather than editing the test.

- [ ] **Step 5: Commit and push**

```bash
git add docs-site/docs/annes-words/words.js docs-site/tests/annes-words.words.test.mjs
git commit -m "feat(annes-words): word lists with seeded shuffle"
git push -u origin annes-words
```

---

### Task 2: Pure game logic

**Files:**
- Create: `docs-site/docs/annes-words/game.js`
- Create: `docs-site/tests/annes-words.game.test.mjs`

**Interfaces:**
- Consumes: nothing (takes all inputs as arguments).
- Produces:
  - `scoreGuess(guess: string, answer: string) -> ('correct'|'present'|'absent')[]` length 5
  - `mergeKeyStates(prev: Record<string,string>, guess: string, marks: string[]) -> Record<string,string>` (returns a new object; precedence correct > present > absent)
  - `dailyIndex(date: Date, listLength: number) -> number`
  - `puzzleNumber(date: Date) -> number`
  - `msUntilNextPuzzle(date: Date) -> number`
  - `shareText(puzzleNo: number, marks2d: string[][], won: boolean) -> string`
  - `EPOCH` — the `Date` constant `new Date(2026, 0, 1)`

- [ ] **Step 1: Write the failing tests**

```js
// docs-site/tests/annes-words.game.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { scoreGuess, mergeKeyStates, dailyIndex, puzzleNumber, msUntilNextPuzzle, shareText } from '../docs/annes-words/game.js';

const C = 'correct', P = 'present', A = 'absent';

test('scores an exact match', () => {
  assert.deepEqual(scoreGuess('crane', 'crane'), [C, C, C, C, C]);
});

test('scores a miss', () => {
  assert.deepEqual(scoreGuess('crane', 'moldy'), [A, A, A, A, A]);
});

test('scores simple presents', () => {
  // guess SPENT vs answer NOTES: p is the only letter not in the answer.
  assert.deepEqual(scoreGuess('spent', 'notes'), [P, A, P, P, P]);
});

test('duplicate letters in the guess do not over-claim', () => {
  // answer has one L; guess has three. Only the positional one scores.
  assert.deepEqual(scoreGuess('lolly', 'lymph'), [C, A, A, A, P]);
});

test('duplicate letters in the guess are marked left to right', () => {
  // answer ALLOY has two Ls. Guess LLAMA: the L at index 1 is positional, which
  // consumes one L from the pool; the L at index 0 takes the remaining one.
  // The trailing A is absent because the single A was already claimed at index 2.
  assert.deepEqual(scoreGuess('llama', 'alloy'), [P, C, P, A, A]);
});

test('exact matches consume the pool before presents', () => {
  // answer ABBEY: guess BABES -> B(present) A(present) B(correct) E(correct) S(absent)
  assert.deepEqual(scoreGuess('babes', 'abbey'), [P, P, C, C, A]);
});

test('a later exact match beats an earlier present for the same letter', () => {
  // answer PIZZA has one A at index 4. Guess AROMA: leading A must be absent.
  assert.deepEqual(scoreGuess('aroma', 'pizza'), [A, A, A, A, C]);
});

test('key states take the best result seen', () => {
  let keys = mergeKeyStates({}, 'crane', [A, P, A, A, C]);
  assert.deepEqual(keys, { c: A, r: P, a: A, n: A, e: C });
  keys = mergeKeyStates(keys, 'ratio', [C, A, A, A, A]);
  assert.equal(keys.r, C, 'present is upgraded to correct');
  assert.equal(keys.e, C, 'existing correct is retained');
  assert.equal(keys.a, A, 'absent stays absent');
});

test('key states never downgrade', () => {
  const keys = mergeKeyStates({ r: 'correct' }, 'rrrrr', [A, A, A, A, A]);
  assert.equal(keys.r, 'correct');
});

test('mergeKeyStates does not mutate its input', () => {
  const prev = { a: A };
  mergeKeyStates(prev, 'aaaaa', [C, C, C, C, C]);
  assert.deepEqual(prev, { a: A });
});

test('puzzle number counts whole local days from the epoch', () => {
  assert.equal(puzzleNumber(new Date(2026, 0, 1, 0, 0)), 0);
  assert.equal(puzzleNumber(new Date(2026, 0, 1, 23, 59)), 0);
  assert.equal(puzzleNumber(new Date(2026, 0, 2, 0, 1)), 1);
  assert.equal(puzzleNumber(new Date(2026, 1, 1, 12, 0)), 31);
});

test('daily index wraps around the list', () => {
  assert.equal(dailyIndex(new Date(2026, 0, 1), 10), 0);
  assert.equal(dailyIndex(new Date(2026, 0, 12), 10), 1);
});

test('daily index is stable across times of day', () => {
  const morning = dailyIndex(new Date(2026, 5, 9, 0, 0, 1), 2315);
  const night = dailyIndex(new Date(2026, 5, 9, 23, 59, 59), 2315);
  assert.equal(morning, night);
});

test('countdown targets the next local midnight', () => {
  const ms = msUntilNextPuzzle(new Date(2026, 5, 9, 23, 0, 0));
  assert.equal(ms, 60 * 60 * 1000);
});

test('share text renders an emoji grid', () => {
  const marks = [[A, P, A, A, C], [C, C, C, C, C]];
  const out = shareText(12, marks, true);
  assert.equal(out, "Anne's Words 12 2/6\n\n⬜🟨⬜⬜🟩\n🟩🟩🟩🟩🟩");
});

test('share text marks a loss with X', () => {
  const marks = Array.from({ length: 6 }, () => [A, A, A, A, A]);
  assert.ok(shareText(3, marks, false).startsWith("Anne's Words 3 X/6"));
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test docs-site/tests/annes-words.game.test.mjs`
Expected: FAIL — cannot find module `game.js`.

- [ ] **Step 3: Write the implementation**

```js
// docs-site/docs/annes-words/game.js
// Pure game logic. No DOM, no localStorage, no implicit clock reads:
// every function that depends on "now" takes the Date as an argument.

export const WORD_LENGTH = 5;
export const MAX_GUESSES = 6;
export const EPOCH = new Date(2026, 0, 1);

const RANK = { absent: 0, present: 1, correct: 2 };

/**
 * Two-pass scoring. Pass 1 claims exact matches and decrements a pool of the
 * answer's unmatched letters; pass 2 hands out "present" only while the pool
 * still has that letter. Doing it in one pass is the classic clone bug: it
 * over-reports presents when the guess repeats a letter the answer has once.
 */
export function scoreGuess(guess, answer) {
  const marks = new Array(WORD_LENGTH).fill('absent');
  const pool = new Map();
  for (let i = 0; i < WORD_LENGTH; i++) {
    if (guess[i] === answer[i]) marks[i] = 'correct';
    else pool.set(answer[i], (pool.get(answer[i]) ?? 0) + 1);
  }
  for (let i = 0; i < WORD_LENGTH; i++) {
    if (marks[i] === 'correct') continue;
    const left = pool.get(guess[i]) ?? 0;
    if (left > 0) { marks[i] = 'present'; pool.set(guess[i], left - 1); }
  }
  return marks;
}

/** Best-result-wins merge of per-letter keyboard states. Returns a new object. */
export function mergeKeyStates(prev, guess, marks) {
  const next = { ...prev };
  for (let i = 0; i < guess.length; i++) {
    const letter = guess[i];
    const current = next[letter];
    if (current === undefined || RANK[marks[i]] > RANK[current]) next[letter] = marks[i];
  }
  return next;
}

const startOfDay = d => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** Whole local days elapsed since the epoch. */
export function puzzleNumber(date) {
  return Math.round((startOfDay(date) - startOfDay(EPOCH)) / 86400000);
}

export function dailyIndex(date, listLength) {
  return ((puzzleNumber(date) % listLength) + listLength) % listLength;
}

export function msUntilNextPuzzle(date) {
  return startOfDay(date) + 86400000 - date.getTime();
}

export function isValidGuess(word, allowedSet) {
  return word.length === WORD_LENGTH && allowedSet.has(word);
}

const EMOJI = { correct: '\u{1F7E9}', present: '\u{1F7E8}', absent: '⬜' };

export function shareText(puzzleNo, marks2d, won) {
  const score = won ? `${marks2d.length}/${MAX_GUESSES}` : `X/${MAX_GUESSES}`;
  const grid = marks2d.map(row => row.map(m => EMOJI[m]).join('')).join('\n');
  return `Anne's Words ${puzzleNo} ${score}\n\n${grid}`;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test 'docs-site/tests/*.test.mjs'`
Expected: all tests pass (Task 1's 3 plus these 15).

- [ ] **Step 5: Commit and push**

```bash
git add docs-site/docs/annes-words/game.js docs-site/tests/annes-words.game.test.mjs
git commit -m "feat(annes-words): pure scoring, daily selection and share logic"
git push
```

---

### Task 3: Playable board — checkpoint you can open in a browser

**Files:**
- Create: `docs-site/docs/annes-words/index.html`
- Create: `docs-site/docs/annes-words/style.css`
- Create: `docs-site/docs/annes-words/ui.js`

**Interfaces:**
- Consumes: `game.js` (`scoreGuess`, `mergeKeyStates`, `isValidGuess`, `WORD_LENGTH`, `MAX_GUESSES`), `words.js` (`ANSWERS`, `ALLOWED`).
- Produces: a running practice game. Task 4 replaces the random word pick with the daily word and adds persistence; Task 5 adds the stats and share UI. Keep the state object in `ui.js` named `state` with the fields `{ answer, guesses: string[], current: string, status: 'playing'|'won'|'lost', keys: {} }` — Tasks 4 and 5 extend it.

- [ ] **Step 1: Write `index.html`**

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Anne's Words</title>
<meta name="description" content="A daily five-letter word game.">
<link rel="stylesheet" href="style.css">
</head>
<body>
<header>
  <h1>Anne's Words</h1>
</header>
<main>
  <div id="toaster" aria-live="polite"></div>
  <div id="board" role="grid" aria-label="Guess board"></div>
  <div id="keyboard" aria-label="Keyboard"></div>
</main>
<script type="module" src="ui.js"></script>
</body>
</html>
```

- [ ] **Step 2: Write `style.css`**

```css
:root {
  --correct: #6aaa64;
  --present: #c9b458;
  --absent: #787c7e;
  --border: #d3d6da;
  --border-filled: #878a8c;
  --key-bg: #d3d6da;
  --fg: #1a1a1b;
  --bg: #ffffff;
}
* { box-sizing: border-box; }
html, body { height: 100%; }
body {
  margin: 0; background: var(--bg); color: var(--fg);
  font-family: "Helvetica Neue", Helvetica, Arial, sans-serif;
  display: flex; flex-direction: column; height: 100dvh;
  -webkit-user-select: none; user-select: none;
}
header {
  border-bottom: 1px solid var(--border); display: flex;
  align-items: center; justify-content: center; height: 50px; flex: none;
}
h1 { margin: 0; font-size: 28px; font-weight: 700; letter-spacing: .01em; }
main { flex: 1; display: flex; flex-direction: column; align-items: center;
       justify-content: space-between; padding: 8px; overflow: hidden; }

#board { display: grid; grid-template-rows: repeat(6, 1fr); gap: 5px;
         padding: 10px; width: 350px; height: 420px; max-width: 100%; }
.row { display: grid; grid-template-columns: repeat(5, 1fr); gap: 5px; }
.tile {
  display: flex; align-items: center; justify-content: center;
  border: 2px solid var(--border); font-size: 2rem; font-weight: 700;
  text-transform: uppercase; line-height: 1; color: var(--fg);
}
.tile[data-state="filled"] { border-color: var(--border-filled); animation: pop .1s ease-in-out; }
.tile[data-state="correct"] { background: var(--correct); border-color: var(--correct); color: #fff; }
.tile[data-state="present"] { background: var(--present); border-color: var(--present); color: #fff; }
.tile[data-state="absent"]  { background: var(--absent);  border-color: var(--absent);  color: #fff; }
.tile.reveal { animation: flip .5s ease forwards; }

@keyframes pop { from { transform: scale(.8); } to { transform: scale(1); } }
@keyframes flip { 0% { transform: rotateX(0); } 50% { transform: rotateX(-90deg); } 100% { transform: rotateX(0); } }
@keyframes shake { 10%,90% { transform: translateX(-1px); } 20%,80% { transform: translateX(2px); }
                   30%,50%,70% { transform: translateX(-4px); } 40%,60% { transform: translateX(4px); } }
.row.shake { animation: shake .6s; }

#keyboard { width: 500px; max-width: 100%; margin-bottom: 8px; }
.kb-row { display: flex; gap: 6px; margin-bottom: 8px; touch-action: manipulation; }
.kb-row .spacer { flex: .5; }
button.key {
  flex: 1; height: 58px; border: 0; border-radius: 4px; background: var(--key-bg);
  color: var(--fg); font-weight: 700; font-size: 1.05rem; text-transform: uppercase;
  cursor: pointer; font-family: inherit;
}
button.key.wide { flex: 1.5; font-size: .75rem; }
button.key[data-state="correct"] { background: var(--correct); color: #fff; }
button.key[data-state="present"] { background: var(--present); color: #fff; }
button.key[data-state="absent"]  { background: var(--absent);  color: #fff; }

#toaster { position: fixed; top: 70px; left: 50%; transform: translateX(-50%);
           display: flex; flex-direction: column; align-items: center; gap: 8px; z-index: 10; }
.toast { background: var(--fg); color: #fff; padding: 12px 16px; border-radius: 4px;
         font-weight: 600; animation: fade .3s; }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }

@media (max-width: 500px) {
  #board { width: 100%; height: min(60dvh, 420px); }
  button.key { height: 48px; }
}
```

- [ ] **Step 3: Write `ui.js` — board, keyboard, scoring, win/lose**

```js
// docs-site/docs/annes-words/ui.js
import { ANSWERS, ALLOWED } from './words.js';
import { scoreGuess, mergeKeyStates, isValidGuess, WORD_LENGTH, MAX_GUESSES } from './game.js';

const VALID = new Set([...ANSWERS, ...ALLOWED]);
const KB_ROWS = ['qwertyuiop', 'asdfghjkl', 'zxcvbnm'];

const boardEl = document.getElementById('board');
const keyboardEl = document.getElementById('keyboard');
const toasterEl = document.getElementById('toaster');

const state = {
  answer: ANSWERS[Math.floor(Math.random() * ANSWERS.length)],
  guesses: [],
  current: '',
  status: 'playing',
  keys: {},
};
let busy = false;

function buildBoard() {
  boardEl.innerHTML = '';
  for (let r = 0; r < MAX_GUESSES; r++) {
    const row = document.createElement('div');
    row.className = 'row';
    row.dataset.row = String(r);
    for (let c = 0; c < WORD_LENGTH; c++) {
      const tile = document.createElement('div');
      tile.className = 'tile';
      tile.dataset.state = 'empty';
      row.append(tile);
    }
    boardEl.append(row);
  }
}

function buildKeyboard() {
  keyboardEl.innerHTML = '';
  KB_ROWS.forEach((letters, i) => {
    const row = document.createElement('div');
    row.className = 'kb-row';
    if (i === 1) row.append(spacer());
    if (i === 2) row.append(keyButton('enter', 'Enter', true));
    for (const letter of letters) row.append(keyButton(letter, letter));
    if (i === 2) row.append(keyButton('backspace', '⌫', true));
    if (i === 1) row.append(spacer());
    keyboardEl.append(row);
  });
}

const spacer = () => Object.assign(document.createElement('div'), { className: 'spacer' });

function keyButton(value, label, wide = false) {
  const b = document.createElement('button');
  b.className = wide ? 'key wide' : 'key';
  b.textContent = label;
  b.dataset.key = value;
  b.type = 'button';
  b.addEventListener('click', () => { press(value); b.blur(); });
  return b;
}

const rowEl = i => boardEl.children[i];

function drawCurrentRow() {
  const row = rowEl(state.guesses.length);
  if (!row) return;
  [...row.children].forEach((tile, i) => {
    const ch = state.current[i] ?? '';
    if (tile.textContent !== ch) {
      tile.textContent = ch;
      tile.dataset.state = ch ? 'filled' : 'empty';
    }
  });
}

function toast(message, ms = 1500) {
  const el = document.createElement('div');
  el.className = 'toast';
  el.textContent = message;
  toasterEl.append(el);
  setTimeout(() => el.remove(), ms);
}

function shakeRow(i) {
  const row = rowEl(i);
  row.classList.add('shake');
  setTimeout(() => row.classList.remove('shake'), 600);
}

function press(key) {
  if (busy || state.status !== 'playing') return;
  if (key === 'enter') return submit();
  if (key === 'backspace') { state.current = state.current.slice(0, -1); return drawCurrentRow(); }
  if (/^[a-z]$/.test(key) && state.current.length < WORD_LENGTH) {
    state.current += key;
    drawCurrentRow();
  }
}

async function submit() {
  const guess = state.current;
  const rowIndex = state.guesses.length;
  if (guess.length < WORD_LENGTH) { toast('Not enough letters'); return shakeRow(rowIndex); }
  if (!isValidGuess(guess, VALID)) { toast('Not in word list'); return shakeRow(rowIndex); }

  const marks = scoreGuess(guess, state.answer);
  state.guesses.push(guess);
  state.current = '';
  state.keys = mergeKeyStates(state.keys, guess, marks);

  await revealRow(rowIndex, marks);
  paintKeyboard();

  if (marks.every(m => m === 'correct')) {
    state.status = 'won';
    toast(['Genius', 'Magnificent', 'Impressive', 'Splendid', 'Great', 'Phew'][rowIndex], 2500);
  } else if (state.guesses.length === MAX_GUESSES) {
    state.status = 'lost';
    toast(state.answer.toUpperCase(), 4000);
  }
}

function revealRow(rowIndex, marks) {
  busy = true;
  const tiles = [...rowEl(rowIndex).children];
  return new Promise(resolve => {
    tiles.forEach((tile, i) => {
      setTimeout(() => {
        tile.classList.add('reveal');
        // swap the colour at the midpoint of the flip, like the original
        setTimeout(() => { tile.dataset.state = marks[i]; }, 250);
        if (i === tiles.length - 1) setTimeout(() => { busy = false; resolve(); }, 500);
      }, i * 300);
    });
  });
}

function paintKeyboard() {
  for (const [letter, mark] of Object.entries(state.keys)) {
    const key = keyboardEl.querySelector(`[data-key="${letter}"]`);
    if (key) key.dataset.state = mark;
  }
}

document.addEventListener('keydown', e => {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === 'Enter') press('enter');
  else if (e.key === 'Backspace') press('backspace');
  else if (/^[a-zA-Z]$/.test(e.key)) press(e.key.toLowerCase());
});

buildBoard();
buildKeyboard();
```

- [ ] **Step 4: Play it**

```bash
cd docs-site/docs && python3 -m http.server 8000
```

Open `http://localhost:8000/annes-words/` and verify by hand:
- typing letters fills tiles with a pop; backspace removes them;
- a nonsense word (`zzzzz`) shakes the row and toasts "Not in word list";
- a real word flips tile by tile, left to right, and colours correctly;
- keyboard keys take their colours after the flip finishes;
- winning toasts praise; losing after 6 toasts the answer in caps.

- [ ] **Step 5: Commit and push**

```bash
git add docs-site/docs/annes-words/index.html docs-site/docs/annes-words/style.css docs-site/docs/annes-words/ui.js
git commit -m "feat(annes-words): playable board, keyboard and reveal animation"
git push
```

---

### Task 4: Daily puzzle, persistence and practice mode

**Files:**
- Create: `docs-site/docs/annes-words/storage.js`
- Modify: `docs-site/docs/annes-words/ui.js`
- Modify: `docs-site/docs/annes-words/index.html` (header controls, countdown element)
- Modify: `docs-site/docs/annes-words/style.css` (header buttons, countdown)
- Create: `docs-site/tests/annes-words.storage.test.mjs`

**Interfaces:**
- Consumes: `game.js` (`dailyIndex`, `puzzleNumber`, `msUntilNextPuzzle`), Task 3's `state`.
- Produces: `storage.js` exporting `load(raw) -> SaveData`, `serialize(data) -> string`, `DEFAULTS`, and `recordResult(stats, {won, guesses, puzzle, lastPuzzle}) -> stats`. `load`/`serialize` are pure (they take/return strings) so they are testable under node; `ui.js` owns the actual `localStorage` calls.

- [ ] **Step 1: Write the failing storage tests**

```js
// docs-site/tests/annes-words.storage.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { load, serialize, DEFAULTS, recordResult } from '../docs/annes-words/storage.js';

test('load returns defaults for missing or corrupt data', () => {
  assert.deepEqual(load(null), DEFAULTS);
  assert.deepEqual(load('not json'), DEFAULTS);
  assert.deepEqual(load('{"version":99}'), DEFAULTS);
});

test('load round-trips a saved game', () => {
  const data = { ...DEFAULTS, daily: { puzzle: 7, guesses: ['crane'], status: 'playing' } };
  assert.deepEqual(load(serialize(data)), data);
});

test('load repairs a partial stats object', () => {
  const out = load('{"version":1,"stats":{"played":3}}');
  assert.equal(out.stats.played, 3);
  assert.equal(out.stats.wins, 0);
  assert.deepEqual(out.stats.dist, [0, 0, 0, 0, 0, 0]);
});

test('a win increments the streak and the distribution', () => {
  const s = recordResult(DEFAULTS.stats, { won: true, guesses: 3, puzzle: 10, lastPuzzle: 9 });
  assert.equal(s.played, 1);
  assert.equal(s.wins, 1);
  assert.equal(s.streak, 1);
  assert.equal(s.maxStreak, 1);
  assert.deepEqual(s.dist, [0, 0, 1, 0, 0, 0]);
});

test('a loss resets the streak but keeps the max', () => {
  let s = recordResult(DEFAULTS.stats, { won: true, guesses: 2, puzzle: 10, lastPuzzle: 9 });
  s = recordResult(s, { won: true, guesses: 4, puzzle: 11, lastPuzzle: 10 });
  s = recordResult(s, { won: false, guesses: 6, puzzle: 12, lastPuzzle: 11 });
  assert.equal(s.streak, 0);
  assert.equal(s.maxStreak, 2);
  assert.equal(s.played, 3);
});

test('a skipped day breaks the streak', () => {
  let s = recordResult(DEFAULTS.stats, { won: true, guesses: 2, puzzle: 10, lastPuzzle: 9 });
  s = recordResult(s, { won: true, guesses: 2, puzzle: 15, lastPuzzle: 10 });
  assert.equal(s.streak, 1, 'streak restarts at 1 rather than continuing');
  assert.equal(s.maxStreak, 1);
});

test('recordResult does not mutate its input', () => {
  const before = { ...DEFAULTS.stats };
  recordResult(DEFAULTS.stats, { won: true, guesses: 1, puzzle: 1, lastPuzzle: 0 });
  assert.deepEqual(DEFAULTS.stats, before);
});
```

- [ ] **Step 2: Run them to verify they fail**

Run: `node --test docs-site/tests/annes-words.storage.test.mjs`
Expected: FAIL — cannot find module `storage.js`.

- [ ] **Step 3: Write `storage.js`**

```js
// docs-site/docs/annes-words/storage.js
// Pure serialization + stats arithmetic. The caller owns localStorage so this
// module stays testable under node.

export const KEY = 'annes-words:v1';
export const VERSION = 1;

export const DEFAULTS = Object.freeze({
  version: VERSION,
  daily: null,
  stats: Object.freeze({ played: 0, wins: 0, streak: 0, maxStreak: 0, lastPuzzle: null, dist: [0, 0, 0, 0, 0, 0] }),
});

const clone = v => JSON.parse(JSON.stringify(v));

/** Parse stored JSON, repairing anything missing. Never throws. */
export function load(raw) {
  if (!raw) return clone(DEFAULTS);
  let parsed;
  try { parsed = JSON.parse(raw); } catch { return clone(DEFAULTS); }
  if (!parsed || parsed.version !== VERSION) return clone(DEFAULTS);
  const base = clone(DEFAULTS);
  const stats = { ...base.stats, ...(parsed.stats ?? {}) };
  if (!Array.isArray(stats.dist) || stats.dist.length !== 6) stats.dist = [0, 0, 0, 0, 0, 0];
  const daily = parsed.daily && typeof parsed.daily.puzzle === 'number'
    ? { puzzle: parsed.daily.puzzle,
        guesses: Array.isArray(parsed.daily.guesses) ? parsed.daily.guesses : [],
        status: parsed.daily.status ?? 'playing' }
    : null;
  return { version: VERSION, daily, stats };
}

export const serialize = data => JSON.stringify(data);

/** Fold one finished daily game into the stats. Returns a new object. */
export function recordResult(stats, { won, guesses, puzzle, lastPuzzle }) {
  const next = { ...stats, dist: [...stats.dist] };
  next.played += 1;
  const consecutive = lastPuzzle !== null && lastPuzzle !== undefined && puzzle === lastPuzzle + 1;
  if (won) {
    next.wins += 1;
    next.dist[guesses - 1] += 1;
    next.streak = consecutive ? stats.streak + 1 : 1;
    next.maxStreak = Math.max(stats.maxStreak, next.streak);
  } else {
    next.streak = 0;
  }
  next.lastPuzzle = puzzle;
  return next;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test 'docs-site/tests/*.test.mjs'`
Expected: all pass.

- [ ] **Step 5: Add the header controls to `index.html`**

Replace the `<header>` element with:

```html
<header>
  <h1>Anne's Words</h1>
  <div class="header-right">
    <span id="mode-label"></span>
  </div>
</header>
```

and add, immediately after the `#keyboard` div inside `<main>`:

```html
<div id="footer">
  <div id="countdown" hidden>Next word in <span id="countdown-clock">--:--:--</span></div>
  <button id="practice-btn" type="button" hidden>Play a practice word</button>
</div>
```

- [ ] **Step 6: Style the new elements — append to `style.css`**

```css
header { position: relative; }
.header-right { position: absolute; right: 12px; font-size: .8rem; color: var(--absent); }
#footer { min-height: 42px; display: flex; flex-direction: column; align-items: center;
          gap: 6px; font-size: .85rem; }
#countdown { font-variant-numeric: tabular-nums; color: var(--absent); }
#practice-btn { border: 0; border-radius: 4px; background: var(--fg); color: #fff;
                font-family: inherit; font-weight: 700; padding: 10px 16px; cursor: pointer; }
```

- [ ] **Step 7: Wire the daily word, restore and persistence into `ui.js`**

Add to the imports:

```js
import { dailyIndex, puzzleNumber, msUntilNextPuzzle } from './game.js';
import { KEY, load, serialize, recordResult } from './storage.js';
```

Replace the `state` initialiser from Task 3 with:

```js
const modeLabel = document.getElementById('mode-label');
const countdownEl = document.getElementById('countdown');
const clockEl = document.getElementById('countdown-clock');
const practiceBtn = document.getElementById('practice-btn');

const readSave = () => { try { return load(localStorage.getItem(KEY)); } catch { return load(null); } };
const writeSave = data => { try { localStorage.setItem(KEY, serialize(data)); } catch { /* private mode */ } };

let save = readSave();
const todayPuzzle = puzzleNumber(new Date());

const state = { mode: 'daily', puzzle: todayPuzzle, answer: '', guesses: [], current: '', status: 'playing', keys: {} };

function startDaily() {
  state.mode = 'daily';
  state.puzzle = todayPuzzle;
  state.answer = ANSWERS[dailyIndex(new Date(), ANSWERS.length)];
  state.guesses = [];
  state.current = '';
  state.status = 'playing';
  state.keys = {};
  modeLabel.textContent = `#${todayPuzzle}`;
  buildBoard();
  buildKeyboard();
  if (save.daily && save.daily.puzzle === todayPuzzle) replay(save.daily);
  else { save.daily = null; updateFooter(); }
}

function startPractice() {
  state.mode = 'practice';
  state.answer = ANSWERS[Math.floor(Math.random() * ANSWERS.length)];
  state.guesses = [];
  state.current = '';
  state.status = 'playing';
  state.keys = {};
  modeLabel.textContent = 'practice';
  buildBoard();
  buildKeyboard();
  updateFooter();
}

/** Repaint a stored daily game with no animation. */
function replay(daily) {
  daily.guesses.forEach((guess, r) => {
    const marks = scoreGuess(guess, state.answer);
    state.guesses.push(guess);
    state.keys = mergeKeyStates(state.keys, guess, marks);
    [...rowEl(r).children].forEach((tile, i) => {
      tile.textContent = guess[i];
      tile.dataset.state = marks[i];
    });
  });
  state.status = daily.status;
  paintKeyboard();
  if (state.status === 'lost') toast(state.answer.toUpperCase(), 4000);
  updateFooter();
}
```

Then, at the end of `submit()`, replace the win/lose block with:

```js
  if (marks.every(m => m === 'correct')) {
    state.status = 'won';
    toast(['Genius', 'Magnificent', 'Impressive', 'Splendid', 'Great', 'Phew'][rowIndex], 2500);
  } else if (state.guesses.length === MAX_GUESSES) {
    state.status = 'lost';
    toast(state.answer.toUpperCase(), 4000);
  }
  persist();
  updateFooter();
```

and add:

```js
function persist() {
  if (state.mode !== 'daily') return;              // practice never touches storage
  const finished = state.status !== 'playing';
  const alreadyRecorded = save.stats.lastPuzzle === state.puzzle;
  if (finished && !alreadyRecorded) {
    save.stats = recordResult(save.stats, {
      won: state.status === 'won',
      guesses: state.guesses.length,
      puzzle: state.puzzle,
      lastPuzzle: save.stats.lastPuzzle,
    });
  }
  save.daily = { puzzle: state.puzzle, guesses: [...state.guesses], status: state.status };
  writeSave(save);
}

function updateFooter() {
  const done = state.status !== 'playing';
  countdownEl.hidden = !(done && state.mode === 'daily');
  practiceBtn.hidden = !done;
  practiceBtn.textContent = state.mode === 'daily' ? 'Play a practice word' : 'Play another';
}

practiceBtn.addEventListener('click', startPractice);

setInterval(() => {
  if (countdownEl.hidden) return;
  const ms = msUntilNextPuzzle(new Date());
  if (ms <= 0) return location.reload();
  const pad = n => String(n).padStart(2, '0');
  clockEl.textContent = `${pad(Math.floor(ms / 3600000))}:${pad(Math.floor(ms / 60000) % 60)}:${pad(Math.floor(ms / 1000) % 60)}`;
}, 1000);
```

Finally, replace the bare `buildBoard(); buildKeyboard();` at the bottom of the file with `startDaily();`. Note `buildBoard`/`buildKeyboard` are now called from the start functions, so `press()` and `submit()` need no change beyond the above.

- [ ] **Step 8: Play it**

Serve as before and verify by hand:
- the header shows `#<n>` and the word is the same on reload;
- make two guesses, reload — the board comes back coloured, mid-game;
- finish the game, reload — the finished board comes back and the countdown ticks;
- "Play a practice word" starts a fresh random game labelled `practice`;
- reload during practice returns you to the finished daily, not the practice game;
- in the browser console: `localStorage.setItem('annes-words:v1','garbage')` then reload — the page still loads a fresh daily rather than erroring.

- [ ] **Step 9: Commit and push**

```bash
git add docs-site/docs/annes-words docs-site/tests/annes-words.storage.test.mjs
git commit -m "feat(annes-words): daily puzzle, persistence, countdown and practice mode"
git push
```

---

### Task 5: Stats modal and share

**Files:**
- Modify: `docs-site/docs/annes-words/index.html` (icon buttons, modal markup)
- Modify: `docs-site/docs/annes-words/style.css` (modal, stat blocks, distribution bars)
- Modify: `docs-site/docs/annes-words/ui.js` (modal wiring, share button)

**Interfaces:**
- Consumes: `shareText` from `game.js`; `save.stats` from Task 4.
- Produces: nothing further depends on this.

- [ ] **Step 1: Add the modal markup to `index.html`**

Put the icon buttons in the header, replacing `.header-right`:

```html
  <div class="header-right">
    <span id="mode-label"></span>
    <button id="stats-btn" type="button" aria-label="Statistics">&#128202;</button>
  </div>
```

and add before the closing `</body>`:

```html
<div id="modal-backdrop" hidden>
  <div id="stats-modal" role="dialog" aria-modal="true" aria-label="Statistics">
    <button id="modal-close" class="modal-close" type="button" aria-label="Close">&times;</button>
    <h2>Statistics</h2>
    <div id="stat-numbers"></div>
    <h2>Guess Distribution</h2>
    <div id="stat-dist"></div>
    <div id="modal-footer">
      <div id="modal-countdown" hidden>
        <div class="label">Next Anne's Words</div>
        <div id="modal-clock" class="clock">--:--:--</div>
      </div>
      <button id="share-btn" type="button" hidden>Share</button>
    </div>
  </div>
</div>
```

- [ ] **Step 2: Style the modal — append to `style.css`**

```css
.header-right { display: flex; align-items: center; gap: 10px; }
#stats-btn { background: none; border: 0; font-size: 1.1rem; cursor: pointer; padding: 0; }

#modal-backdrop { position: fixed; inset: 0; background: rgba(0,0,0,.5);
                  display: flex; align-items: center; justify-content: center; z-index: 20; }
/* An explicit display beats the UA rule for [hidden], so restore it by hand.
   Without this line the modal can never be hidden. */
#modal-backdrop[hidden] { display: none; }
#stats-modal { position: relative; background: var(--bg); border-radius: 8px; padding: 16px;
               width: 320px; max-width: 92vw; box-shadow: 0 4px 23px rgba(0,0,0,.2); }
#stats-modal h2 { font-size: .9rem; text-transform: uppercase; text-align: center;
                  letter-spacing: .05em; margin: 12px 0; }
#modal-close { position: absolute; top: 8px; right: 10px; border: 0; background: none;
               font-size: 1.5rem; line-height: 1; cursor: pointer; color: var(--fg); }
#stat-numbers { display: flex; justify-content: center; }
.stat { flex: 1; text-align: center; }
.stat .value { font-size: 2rem; }
.stat .label { font-size: .7rem; }
.dist-row { display: flex; align-items: center; gap: 6px; margin-bottom: 4px; font-size: .8rem; }
.dist-bar { background: var(--absent); color: #fff; text-align: right; padding: 2px 6px;
            min-width: 20px; font-weight: 700; }
.dist-bar.best { background: var(--correct); }
#modal-footer { display: flex; align-items: center; gap: 12px; margin-top: 16px;
                border-top: 1px solid var(--border); padding-top: 16px; }
#modal-countdown { flex: 1; text-align: center; }
#modal-countdown .label { font-size: .7rem; text-transform: uppercase; }
#modal-countdown .clock { font-size: 1.6rem; font-variant-numeric: tabular-nums; }
#share-btn { flex: 1; background: var(--correct); color: #fff; border: 0; border-radius: 4px;
             height: 52px; font-family: inherit; font-weight: 700; font-size: 1.1rem;
             text-transform: uppercase; cursor: pointer; }
```

- [ ] **Step 3: Wire the modal in `ui.js`**

Add `shareText` to the `game.js` import, then append:

```js
const backdrop = document.getElementById('modal-backdrop');
const shareBtn = document.getElementById('share-btn');
const modalCountdown = document.getElementById('modal-countdown');
const modalClock = document.getElementById('modal-clock');

function renderStats() {
  const { played, wins, streak, maxStreak, dist } = save.stats;
  const winPct = played ? Math.round((wins / played) * 100) : 0;
  document.getElementById('stat-numbers').innerHTML =
    [['Played', played], ['Win %', winPct], ['Current Streak', streak], ['Max Streak', maxStreak]]
      .map(([label, value]) => `<div class="stat"><div class="value">${value}</div><div class="label">${label}</div></div>`)
      .join('');

  const max = Math.max(1, ...dist);
  const winRow = state.mode === 'daily' && state.status === 'won' ? state.guesses.length : 0;
  document.getElementById('stat-dist').innerHTML = dist.map((n, i) => {
    const width = Math.max(7, Math.round((n / max) * 100));
    const best = i + 1 === winRow ? ' best' : '';
    return `<div class="dist-row"><span>${i + 1}</span><div class="dist-bar${best}" style="width:${width}%">${n}</div></div>`;
  }).join('');

  const finishedDaily = state.mode === 'daily' && state.status !== 'playing';
  shareBtn.hidden = !finishedDaily;
  modalCountdown.hidden = !finishedDaily;
}

function openStats() { renderStats(); backdrop.hidden = false; }
function closeStats() { backdrop.hidden = true; }

document.getElementById('stats-btn').addEventListener('click', openStats);
document.getElementById('modal-close').addEventListener('click', closeStats);
backdrop.addEventListener('click', e => { if (e.target === backdrop) closeStats(); });
document.addEventListener('keydown', e => { if (e.key === 'Escape') closeStats(); });

shareBtn.addEventListener('click', async () => {
  const marks2d = state.guesses.map(g => scoreGuess(g, state.answer));
  const text = shareText(state.puzzle, marks2d, state.status === 'won');
  try {
    await navigator.clipboard.writeText(text);
    toast('Copied results to clipboard');
  } catch {
    toast('Could not copy');
  }
});
```

Then extend the existing countdown `setInterval` body so it also fills the modal
clock: after computing the formatted string, assign it to both
`clockEl.textContent` and `modalClock.textContent`, and change the early return
guard from `if (countdownEl.hidden) return;` to
`if (countdownEl.hidden && modalCountdown.hidden) return;`.

Finally, open the stats modal automatically ~2s after a daily game ends: at the
end of `submit()`, after `updateFooter()`, add:

```js
  if (state.mode === 'daily' && state.status !== 'playing') setTimeout(openStats, 2200);
```

- [ ] **Step 4: Play it**

Verify by hand:
- the chart icon opens the modal at any time; Escape, the ×, and a backdrop click all close it;
- with no games played the modal shows zeros and does not divide by zero;
- finishing the daily auto-opens the modal after the toast, with Share and the countdown;
- Share copies text of the shape `Anne's Words 244 3/6` followed by a blank line and the emoji grid — paste it somewhere to confirm;
- the practice game shows no Share button and no countdown.

- [ ] **Step 5: Commit and push**

```bash
git add docs-site/docs/annes-words
git commit -m "feat(annes-words): statistics modal, streaks and emoji share"
git push
```

---

### Task 6: Help modal, polish and publish verification

**Files:**
- Modify: `docs-site/docs/annes-words/index.html`, `style.css`, `ui.js`

**Interfaces:**
- Consumes: the modal machinery from Task 5.
- Produces: nothing.

- [ ] **Step 1: Add a help button and modal to `index.html`**

Add to `.header-right`, before the stats button:

```html
    <button id="help-btn" type="button" aria-label="How to play">?</button>
```

and a second dialog inside `#modal-backdrop`, as a sibling of `#stats-modal`:

```html
  <div id="help-modal" role="dialog" aria-modal="true" aria-label="How to play" hidden>
    <button class="modal-close" type="button" aria-label="Close">&times;</button>
    <h2>How to play</h2>
    <p>Guess the word in 6 tries. Each guess must be a valid 5-letter word.</p>
    <p>The colour of the tiles will change to show how close your guess was.</p>
    <div class="example"><div class="tile" data-state="correct">w</div><div class="tile">e</div><div class="tile">a</div><div class="tile">r</div><div class="tile">y</div></div>
    <p><strong>W</strong> is in the word and in the right spot.</p>
    <div class="example"><div class="tile">p</div><div class="tile" data-state="present">i</div><div class="tile">l</div><div class="tile">l</div><div class="tile">s</div></div>
    <p><strong>I</strong> is in the word but in the wrong spot.</p>
    <div class="example"><div class="tile">v</div><div class="tile">a</div><div class="tile">g</div><div class="tile" data-state="absent">u</div><div class="tile">e</div></div>
    <p><strong>U</strong> is not in the word in any spot.</p>
    <p class="fine">A new word is available each day.</p>
  </div>
```

- [ ] **Step 2: Style it — append to `style.css`**

```css
#help-modal { position: relative; background: var(--bg); border-radius: 8px; padding: 16px;
              width: 340px; max-width: 92vw; box-shadow: 0 4px 23px rgba(0,0,0,.2); }
#help-modal p { font-size: .9rem; line-height: 1.4; }
#help-modal .fine { border-top: 1px solid var(--border); padding-top: 12px; font-size: .8rem; }
#help-btn { background: none; border: 0; font-size: 1.1rem; font-weight: 700; cursor: pointer;
            color: var(--fg); padding: 0; }
.example { display: flex; gap: 4px; margin: 8px 0; }
.example .tile { width: 40px; height: 40px; font-size: 1.5rem; }
.modal-close { position: absolute; top: 8px; right: 10px; border: 0; background: none;
               font-size: 1.5rem; line-height: 1; cursor: pointer; color: var(--fg); }
```

- [ ] **Step 3: Wire it in `ui.js`**

Generalise the modal helpers so the backdrop can host either dialog:

```js
const helpModal = document.getElementById('help-modal');
const statsModal = document.getElementById('stats-modal');

function showModal(el) {
  statsModal.hidden = el !== statsModal;
  helpModal.hidden = el !== helpModal;
  backdrop.hidden = false;
}

document.getElementById('help-btn').addEventListener('click', () => showModal(helpModal));
for (const btn of document.querySelectorAll('.modal-close')) btn.addEventListener('click', closeStats);
```

and change `openStats()` to end with `showModal(statsModal);` instead of setting
`backdrop.hidden` directly. Show the help modal on a player's first ever visit:
at the bottom of the file, after `startDaily();`, add:

```js
if (save.stats.played === 0 && !save.daily) showModal(helpModal);
```

- [ ] **Step 4: Verify the site still builds and the page publishes**

```bash
cd docs-site && uv run mkdocs build --strict 2>&1 | tail -20
ls site/annes-words/
```

Expected: the build succeeds with no warnings, and `site/annes-words/` contains
`index.html`, `style.css`, `game.js`, `ui.js`, `storage.js`, `words.js`. If
`--strict` complains about the directory, do NOT add the page to `nav` — report
the exact warning instead; the fix is a `mkdocs.yml` `exclude_docs`/plugin
setting, not a nav entry.

- [ ] **Step 5: Check mobile layout**

In the browser devtools device toolbar at 390×844 (iPhone 14), confirm the
board and keyboard both fit with no page scroll, and that tapping keys does not
zoom or select text.

- [ ] **Step 6: Run the full test suite one more time**

Run: `node --test 'docs-site/tests/*.test.mjs'`
Expected: all tests pass.

- [ ] **Step 7: Commit and push**

```bash
git add docs-site/docs/annes-words
git commit -m "feat(annes-words): how-to-play modal and mobile polish"
git push
```

---

## Notes for the reviewer

- `words.js` is generated, ~250 KB of JSON. Do not hand-edit it; regenerate with the Task 1 script.
- No Rust crate is touched, so none of the repo's cargo gates apply. `mkdocs build --strict` (Task 6, Step 4) is the only CI-relevant check.
- The one piece of logic worth reading closely is `scoreGuess` — the duplicate-letter cases in the test file are the specification.
