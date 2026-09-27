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
