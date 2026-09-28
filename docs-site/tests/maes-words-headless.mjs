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
  if (isFill) {
    const blankText = await page.locator('#prompt .sight-blank').innerText();
    assert.ok(blankText.trim().length > 0, 'a correct Fill answer writes the word into the blank');
  }
  await page.waitForTimeout(1100);
}
assert.deepEqual(errors, []);
assert.ok(seen.has('fill'), `no fill turn seen: ${[...seen]}`);
console.log('ok', [...seen]);

// Grown-up corner: long-press the title, check the sight-words section row exists.
await page.goto('http://localhost:8000/maes-words/');
await page.evaluate(() => {
  const title = document.getElementById('title');
  title.dispatchEvent(new PointerEvent('pointerdown'));
});
await page.waitForTimeout(1300);
await page.evaluate(() => {
  const title = document.getElementById('title');
  title.dispatchEvent(new PointerEvent('pointerup'));
});
await page.waitForSelector('#screen-grownup:not([hidden])');
const sectionCount = await page.locator('#grownup-table tr.section').count();
assert.equal(sectionCount, 1, 'grown-up table has exactly one Sight words section row');
const sectionText = await page.locator('#grownup-table tr.section').innerText();
assert.equal(sectionText.trim(), 'Sight words');

// credits.html: no page errors.
const creditsErrors = [];
const page2 = await browser.newPage();
page2.on('pageerror', e => creditsErrors.push(String(e)));
await page2.goto('http://localhost:8000/maes-words/credits.html');
assert.deepEqual(creditsErrors, []);
await page2.close();

await browser.close();
console.log('all checks passed');
