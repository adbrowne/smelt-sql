// docs-site/docs/maes-words/ui.js
import { WORDS, FRAMES } from './words.js';
import { initialState, introduceWords, applyAnswer, planRound, isCorrect, learnedWords,
         shuffle, ROUND_LENGTH } from './game.js';
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
let results = [];

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
    let cls = 'dot';
    if (results[i] === true) cls += ' star';
    if (i < turnIndex) cls += ' done';
    d.className = cls;
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
    const correct = isCorrect(turn, letters);
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
  results[turnIndex] = correct;
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
  results = [];
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
  for (const label of shuffle(['rest', 'reset', 'resit'], Math.random)) {
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
