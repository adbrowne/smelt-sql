// docs-site/docs/maes-words/ui.js
import { WORDS, FRAMES } from './words.js';
import { initialState, introduceWords, planRound, isCorrect, learnedWords,
         shuffle, ROUND_LENGTH, refreshTurn, applyAttempt, retryAllowed } from './game.js';
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
let attempts = 0;

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
    chip.append(renderPic(w.pic, { size: 'chip' }), Object.assign(document.createElement('span'), { textContent: w.w }));
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

function tile(label) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'tile';
  b.textContent = label;
  b.dataset.w = label;
  return b;
}

/** The one place a word's picture becomes DOM. `size` is 'tile' | 'prompt' | 'chip'.
 *  The size class is namespaced `pic-<size>` rather than the bare word, since 'tile'/
 *  'prompt'/'chip' are already global component classes elsewhere in style.css — an
 *  unprefixed class here would make the pic itself match those unrelated rules. */
function renderPic(pic, { size = 'tile' } = {}) {
  if (!pic) { const s = document.createElement('span'); s.className = `pic pic-none pic-${size}`; return s; }
  if (pic.kind === 'emoji') {
    const s = document.createElement('span');
    s.className = `pic pic-emoji pic-${size}`;
    s.textContent = pic.text;
    return s;
  }
  if (pic.kind === 'colour') {
    const d = document.createElement('div');
    d.className = `pic pic-colour pic-${size}`;
    d.style.background = pic.css;
    return d;
  }
  const img = document.createElement('img');
  img.className = `pic pic-img pic-${size}`;
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

function renderRead(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt';
  prompt.textContent = turn.word.w;
  const opts = $('options');
  opts.className = 'options';
  opts.innerHTML = '';
  for (const o of turn.options) {
    const b = picTile(o);
    b.addEventListener('click', () => answer(o.w, b));
    opts.append(b);
  }
}

function renderPick(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt pic';
  prompt.innerHTML = '';
  prompt.append(renderPic(turn.word.pic, { size: 'prompt' }));
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
  prompt.className = 'prompt pic';
  prompt.innerHTML = '';
  prompt.append(renderPic(turn.word.pic, { size: 'prompt' }));

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
}

function renderSentence(turn) {
  const prompt = $('prompt');
  prompt.className = 'prompt sentence';
  prompt.innerHTML = '';
  const [before, after] = turn.frame.split('{}');
  const blank = document.createElement('span');
  blank.className = 'blank';
  blank.innerHTML = '';
  blank.append(renderPic(turn.word.pic, { size: 'prompt' }));
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
  attempts = 0;
  $('slots').hidden = true;
  turns[turnIndex] = refreshTurn(turns[turnIndex], state, WORDS, FRAMES, rng);
  renderProgress();
  RENDER[turns[turnIndex].activity](turns[turnIndex]);
}

/** Resolve the current turn. `chosenEl` is the tapped tile (or null for build); marks it and advances. */
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
  if (turns.length && turnIndex < turns.length && !$('screen-turn').hidden) return;
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
  if (speechSynthesis.speaking) speechSynthesis.cancel();
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
    const nameTd = document.createElement('td');
    nameTd.append(renderPic(w.pic, { size: 'chip' }), document.createTextNode(` ${w.w}`));
    tr.append(nameTd);
    tr.insertAdjacentHTML('beforeend', `<td>${w.tier}</td><td class="lvl-${lvl ?? 'none'}">${lvlText}</td>`);
    tbody.append(tr);
  }
  // Reset guard: the adult must tap the written word "reset" among look-alikes.
  const box = $('reset-options');
  box.innerHTML = '';
  for (const label of shuffle(['rest', 'reset', 'resit'], Math.random)) {
    const b = tile(label);
    b.addEventListener('click', () => {
      if (label !== 'reset') { goHome(); return; }
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
