import test from 'node:test';
import assert from 'node:assert/strict';
import { load, serialize, DEFAULTS, recordResult, expireStreak } from '../docs/annes-words/storage.js';

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

// --- I2: a corrupt saved daily.guesses must not brick the page -----------

test('load discards a daily with more than 6 stored guesses', () => {
  const raw = JSON.stringify({ version: 1, daily: { puzzle: 5, guesses: ['crane', 'slate', 'sooty', 'roast', 'tarot', 'stray', 'extra'], status: 'playing' }, stats: {} });
  const out = load(raw);
  assert.equal(out.daily, null);
});

test('load discards a daily whose guesses are not all 5-character strings', () => {
  const raw = JSON.stringify({ version: 1, daily: { puzzle: 5, guesses: ['crane', 42, 'toolong'], status: 'playing' }, stats: {} });
  const out = load(raw);
  assert.equal(out.daily, null);
});

// --- I3: a skipped day resets the streak on load --------------------------

test('expireStreak resets the streak after a skipped day', () => {
  const stats = { ...DEFAULTS.stats, streak: 12, maxStreak: 12, lastPuzzle: 10 };
  const out = expireStreak(stats, 20);
  assert.equal(out.streak, 0);
  assert.equal(out.maxStreak, 12, 'maxStreak is untouched');
  assert.equal(out.lastPuzzle, 10, 'lastPuzzle is untouched');
});

test('expireStreak does not reset the streak on a consecutive day', () => {
  const stats = { ...DEFAULTS.stats, streak: 5, lastPuzzle: 19 };
  const out = expireStreak(stats, 20);
  assert.equal(out.streak, 5);
});

test('expireStreak does not reset the streak for today\'s own puzzle', () => {
  const stats = { ...DEFAULTS.stats, streak: 5, lastPuzzle: 20 };
  const out = expireStreak(stats, 20);
  assert.equal(out.streak, 5);
});

test('expireStreak is a no-op when lastPuzzle is null', () => {
  const stats = { ...DEFAULTS.stats, streak: 0, lastPuzzle: null };
  const out = expireStreak(stats, 20);
  assert.equal(out.streak, 0);
  assert.equal(out.lastPuzzle, null);
});

// --- M4: hostile/wrong-typed stats load as clean numbers -------------------

test('load coerces hostile or wrong-typed stats values to clean numbers', () => {
  const raw = JSON.stringify({
    version: 1,
    stats: {
      played: '<img src=x onerror=alert(1)>',
      wins: { toString: () => 'nope' },
      streak: 'NaN',
      maxStreak: undefined,
      lastPuzzle: '<script>1</script>',
      dist: ['<b>', 2, null, 'x', 5, 6],
    },
  });
  const out = load(raw);
  assert.equal(out.stats.played, 0);
  assert.equal(out.stats.wins, 0);
  assert.equal(out.stats.streak, 0);
  assert.equal(out.stats.maxStreak, 0);
  assert.equal(out.stats.lastPuzzle, null);
  assert.deepEqual(out.stats.dist, [0, 2, 0, 0, 5, 6]);
  for (const v of Object.values(out.stats)) {
    if (Array.isArray(v)) { for (const n of v) assert.equal(typeof n, 'number'); }
    else assert.ok(v === null || typeof v === 'number');
  }
});

// --- Ported verification: multi-day play simulation -----------------------

test('multi-day play simulation matches the hand-verified stats', () => {
  let stats = DEFAULTS.stats;
  const sequence = [
    { won: true, guesses: 3, puzzle: 10 },
    { won: true, guesses: 4, puzzle: 11 },
    { won: true, guesses: 2, puzzle: 14 },
    { won: false, guesses: 6, puzzle: 15 },
    { won: true, guesses: 1, puzzle: 16 },
  ];
  for (const step of sequence) {
    stats = recordResult(stats, { ...step, lastPuzzle: stats.lastPuzzle });
  }
  assert.equal(stats.played, 5);
  assert.equal(stats.wins, 4);
  assert.equal(stats.streak, 1);
  assert.equal(stats.maxStreak, 2);
  assert.deepEqual(stats.dist, [1, 1, 1, 1, 0, 0]);
  assert.equal(stats.lastPuzzle, 16);
});

// --- Ported verification: corruption sweep ---------------------------------

test('load never throws and always returns a well-shaped object for corrupt input', () => {
  const inputs = [
    null, '', 'garbage', '{}', '{"version":2}',
    '{"version":1,"stats":{"dist":"nope"}}',
    '{"version":1,"daily":{}}',
    '[]',
    '{"version":1,"daily":null,"stats":null}',
    '{"version":1,"stats":{"played":"x"}}',
    JSON.stringify({ version: 1, daily: { puzzle: 1, guesses: ['aaaaa', 'bbbbb', 'ccccc', 'ddddd', 'eeeee', 'fffff', 'ggggg'] } }),
    JSON.stringify({ version: 1, daily: { puzzle: 1, guesses: ['aaaaa', 7, 'ccccc'] } }),
    JSON.stringify({ version: 1, daily: { puzzle: 1, guesses: ['aaaaa', 'toolong'] } }),
  ];
  for (const raw of inputs) {
    let out;
    assert.doesNotThrow(() => { out = load(raw); }, `load threw for input: ${raw}`);
    assert.equal(out.version, 1);
    assert.ok(out.daily === null || (typeof out.daily.puzzle === 'number' && Array.isArray(out.daily.guesses)));
    assert.ok(out.stats && typeof out.stats.played === 'number' && Array.isArray(out.stats.dist) && out.stats.dist.length === 6);
  }
});
