import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, shuffle, editDistance, distractors, letterTray, makeTurn, isCorrect,
         planRound, ROUND_LENGTH, ACTIVITY_FOR_LEVEL, refreshTurn, framesFor, resolveFrame } from '../docs/maes-words/game.js';

const mk = (w, tier) => ({ w, e: w.toUpperCase(), tier });
const BANK = [
  ...['cat', 'cot', 'cut', 'dog', 'dig', 'sun', 'bed', 'pig', 'bus', 'egg', 'hat', 'ox'].map(w => mk(w, 1)),
  ...['frog', 'milk', 'duck', 'bell'].map(w => mk(w, 2)),
];
const FRAMES = [
  { text: 'I {sight} the {noun}', fits: ['like', 'see'] },
  { text: 'Here is a {noun}', fits: [] },
  { text: '{sight} is a {noun}', fits: ['here', 'this'] },
  { text: 'I can {noun}', fits: [], pos: 'verb' },
  { text: 'It is {noun}', fits: [], pos: 'adj' },
];
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
  assert.ok(sent.frame.includes('{}'));
  assert.equal((sent.frame.match(/\{\}/g) || []).length, 1);
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

test('refreshTurn re-derives a turn whose word has moved to a new level since planning', () => {
  const turn = makeTurn(byName('cat'), 0, BANK, FRAMES, seeded(1));
  assert.equal(turn.activity, 'read');
  const state = withLevels({ cat: 1 });
  const refreshed = refreshTurn(turn, state, BANK, FRAMES, seeded(2));
  assert.equal(refreshed.activity, 'pick');
  assert.equal(refreshed.word.w, 'cat');
});

test('refreshTurn returns the same object when the activity still matches the current level', () => {
  const turn = makeTurn(byName('cat'), 0, BANK, FRAMES, seeded(1));
  const state = withLevels({ cat: 0 });
  const refreshed = refreshTurn(turn, state, BANK, FRAMES, seeded(2));
  assert.equal(refreshed, turn);
});

test('framesFor drops "a {noun}" frames for vowel-initial words and returns non-empty', () => {
  const frames = [
    { text: 'I like the {noun}', fits: [] },
    { text: 'Here is a {noun}', fits: [] },
    { text: 'The {noun} is here', fits: [] },
    { text: 'I have a {noun}', fits: [] },
  ];
  const usable = framesFor({ w: 'egg' }, frames);
  assert.ok(usable.length > 0);
  for (const f of usable) assert.doesNotMatch(f.text, /\ba \{noun\}/);
});

test('framesFor drops "a {noun}" frames for noA words and returns non-empty', () => {
  const frames = [
    { text: 'I like the {noun}', fits: [] },
    { text: 'Here is a {noun}', fits: [] },
    { text: 'The {noun} is here', fits: [] },
    { text: 'I have a {noun}', fits: [] },
  ];
  const usable = framesFor({ w: 'milk', noA: true }, frames);
  assert.ok(usable.length > 0);
  for (const f of usable) assert.doesNotMatch(f.text, /\ba \{noun\}/);
});

test('framesFor returns all frames unchanged for a normal consonant-initial word', () => {
  const frames = [
    { text: 'I like the {noun}', fits: [] },
    { text: 'Here is a {noun}', fits: [] },
  ];
  assert.deepEqual(framesFor({ w: 'cat' }, frames), frames);
});

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

test('framesFor returns [] rather than falling back to a different pos when none of that pos exist', () => {
  const nounFrameOnly = [{ text: 'Here is a {noun}', fits: [] }];
  assert.deepEqual(framesFor({ w: 'zzz', pos: 'verb' }, nounFrameOnly), []);
});

test('a sentence turn for a templated frame uses one of its fits', () => {
  for (let seed = 1; seed <= 30; seed++) {
    const t = makeTurn(byName('cat'), 3, BANK, FRAMES, seeded(seed));
    assert.equal((t.frame.match(/\{\}/g) || []).length, 1, t.frame);
    assert.doesNotMatch(t.frame, /\{sight\}|\{noun\}/);
  }
});

test('makeTurn never picks an "a {}" frame for a vowel-initial word, across many seeds', () => {
  const egg = { w: 'egg', e: '🥚', tier: 1 };
  const bank = [egg, ...BANK];
  for (let seed = 1; seed <= 50; seed++) {
    const t = makeTurn(egg, 3, bank, FRAMES, seeded(seed));
    assert.doesNotMatch(t.frame, /\ba \{\}/, `seed ${seed}: ${t.frame}`);
  }
});
