// docs-site/docs/annes-words/ui.js
import { ANSWERS, ALLOWED } from './words.js';
import { scoreGuess, mergeKeyStates, isValidGuess, WORD_LENGTH, MAX_GUESSES,
         dailyIndex, puzzleNumber, msUntilNextPuzzle, shareText } from './game.js';
import { KEY, load, serialize, recordResult } from './storage.js';

const VALID = new Set([...ANSWERS, ...ALLOWED]);
const KB_ROWS = ['qwertyuiop', 'asdfghjkl', 'zxcvbnm'];

const boardEl = document.getElementById('board');
const keyboardEl = document.getElementById('keyboard');
const toasterEl = document.getElementById('toaster');
const modeLabel = document.getElementById('mode-label');
const countdownEl = document.getElementById('countdown');
const clockEl = document.getElementById('countdown-clock');
const practiceBtn = document.getElementById('practice-btn');

const readSave = () => { try { return load(localStorage.getItem(KEY)); } catch { return load(null); } };
const writeSave = data => { try { localStorage.setItem(KEY, serialize(data)); } catch { /* private mode */ } };

let save = readSave();
const todayPuzzle = puzzleNumber(new Date());

const state = { mode: 'daily', puzzle: todayPuzzle, answer: '', guesses: [], current: '', status: 'playing', keys: {} };
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
  persist();
  updateFooter();
  if (state.mode === 'daily' && state.status !== 'playing') setTimeout(openStats, 2200);
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

// Physical keystrokes must not reach the board while the stats modal is open
// (Escape is handled by the listener above regardless of this guard).
document.addEventListener('keydown', e => {
  if (!backdrop.hidden) return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === 'Enter') press('enter');
  else if (e.key === 'Backspace') press('backspace');
  else if (/^[a-zA-Z]$/.test(e.key)) press(e.key.toLowerCase());
});

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
  if (countdownEl.hidden && modalCountdown.hidden) return;
  const ms = msUntilNextPuzzle(new Date());
  if (ms <= 0) return location.reload();
  const pad = n => String(n).padStart(2, '0');
  const formatted = `${pad(Math.floor(ms / 3600000))}:${pad(Math.floor(ms / 60000) % 60)}:${pad(Math.floor(ms / 1000) % 60)}`;
  clockEl.textContent = formatted;
  modalClock.textContent = formatted;
}, 1000);

startDaily();
