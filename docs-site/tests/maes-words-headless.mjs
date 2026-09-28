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
    const correctLabel = await page.locator('#options .tile.correct').innerText();
    assert.equal(blankText.trim(), correctLabel.trim(), 'the blank shows exactly the answered tile\'s label');
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

// Regression for the stale-lookTimer race: leave a sight-word Build's 1200ms look phase
// (via the Grown-up corner) before it fires, then start a fresh round and confirm the new
// turn's prompt is intact rather than wiped/force-revealed by the old timer firing late.
{
  const page3 = await browser.newPage({ viewport: { width: 390, height: 844 } });
  const p3errors = [];
  page3.on('pageerror', e => p3errors.push(String(e)));
  await page3.goto('http://localhost:8000/maes-words/');
  await page3.evaluate(() => localStorage.setItem('maes-words:v1', JSON.stringify({
    version: 1, stars: 0, learnedOrder: [], levels: { cat: 2, dog: 2, the: 2 } })));
  await page3.reload();
  await page3.click('#play-btn');

  const isSightBuildLook = async () => {
    const cls = await page3.locator('#options').getAttribute('class').catch(() => '');
    if (!cls.includes('tray')) return false;
    const slotsHidden = await page3.locator('#slots').evaluate(e => e.hidden).catch(() => false);
    if (!slotsHidden) return false;
    const text = await page3.locator('#prompt').innerText().catch(() => '');
    return text.trim().length > 0;
  };
  let hitLookPhase = false;
  for (let step = 0; step < 200 && !hitLookPhase; step++) {
    if (await isSightBuildLook()) { hitLookPhase = true; break; }
    const cls = await page3.locator('#options').getAttribute('class').catch(() => '');
    if (cls.includes('tray')) {
      const slotsHidden = await page3.locator('#slots').evaluate(e => e.hidden).catch(() => true);
      if (!slotsHidden) {
        const btn = page3.locator('#options .tile:not(.used):not([disabled])').first();
        if (await btn.count() > 0) await btn.click({ timeout: 1000 }).catch(() => {});
      }
    } else {
      const t = page3.locator('#options .tile:not([disabled])').first();
      if (await t.count() > 0) await t.click({ timeout: 1000 }).catch(() => {});
    }
    await page3.waitForTimeout(100);
  }
  assert.ok(hitLookPhase, 'never reached the sight-word build look phase to test the race fix');

  // Leave mid-look-phase (well before the 1200ms reveal) via the Grown-up corner's long-press.
  await page3.evaluate(() => document.getElementById('title').dispatchEvent(new PointerEvent('pointerdown')));
  await page3.waitForSelector('#screen-grownup:not([hidden])', { timeout: 3000 });
  await page3.evaluate(() => document.getElementById('title').dispatchEvent(new PointerEvent('pointerup')));

  // Wait past when the stale timer would have fired, then start a brand new round.
  await page3.waitForTimeout(1500);
  await page3.locator('#grownup-back').click();
  await page3.locator('#play-btn').click();
  await page3.waitForSelector('#options .tile, #options .tray .tile');

  // The new turn's prompt must not have been wiped/force-revealed by the stale timer: it
  // must show real content (a word, a sentence, or a picture), and slots must not be a
  // spuriously-revealed empty bar with no tray tiles selectable.
  const promptText = (await page3.locator('#prompt').innerText()).trim();
  const promptHasPic = await page3.locator('#prompt .pic').count() > 0;
  assert.ok(promptText.length > 0 || promptHasPic, 'stale look-timer wiped the new turn\'s prompt');
  assert.deepEqual(p3errors, []);
  await page3.close();
  console.log('stale look-timer regression check passed');
}

// credits.html: no page errors.
const creditsErrors = [];
const page2 = await browser.newPage();
page2.on('pageerror', e => creditsErrors.push(String(e)));
await page2.goto('http://localhost:8000/maes-words/credits.html');
assert.deepEqual(creditsErrors, []);
await page2.close();

await browser.close();
console.log('all checks passed');
