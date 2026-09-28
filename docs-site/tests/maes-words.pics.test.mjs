import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, statSync, readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { WORDS } from '../docs/maes-words/words.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const GAME = join(HERE, '..', 'docs', 'maes-words');
const REPO_ROOT = join(HERE, '..', '..');

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

test('the generated blocks and credits.html are up to date with the PICS table (network-free)', () => {
  const result = spawnSync(process.execPath, [join(REPO_ROOT, 'docs-site', 'tools', 'generate-maes-pics.mjs'), '--check'], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, `--check exited ${result.status}\nstdout: ${result.stdout}\nstderr: ${result.stderr}`);
});

test('the bank has grown and every pictogram word is credited', () => {
  assert.ok(WORDS.length >= 150, `only ${WORDS.length} words`);
  const credits = readFileSync(join(GAME, 'credits.html'), 'utf8');
  for (const x of WORDS) if (x.pic.kind === 'svg') assert.ok(credits.includes(`>${x.w}<`), `${x.w} not credited`);
});
