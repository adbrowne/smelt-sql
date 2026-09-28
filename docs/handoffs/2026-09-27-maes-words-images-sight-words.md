# Handoff — Mae's Words: pictogram word bank + sight-word activity

**Date:** 2026-09-27
**From:** session that built and shipped Mae's Words v1
**For:** a fresh session, any model; coding on Sonnet, review on Fable worked well

## Where things stand

Mae's Words v1 is live at https://smeltsql.com/maes-words/ (merged to `main` at
`ef1a0b317`, deployed by the `Docs` workflow). It is a static sibling of Anne's
Words under `docs-site/docs/maes-words/`, unrelated to smelt, exempt from the
smelt spec/plan/ROADMAP workflow and the Rust CI gates.

- Spec: `docs/superpowers/specs/2026-09-27-maes-words-design.md`
- Plan (done): `docs/superpowers/plans/2026-09-27-maes-words.md`
- Code: `words.js` (bank + frames), `game.js` (pure logic, `rng` injected),
  `storage.js` (pure serialisation, key `maes-words:v1`), `ui.js` (all DOM),
  `style.css`, `index.html`.
- Tests: `node --test 'docs-site/tests/maes-words.*.test.mjs'` — 46 passing.
- Headless checks: Playwright is installed at `docs/demos/node_modules`;
  `NODE_PATH=docs/demos/node_modules node script.mjs` with the page served by
  `cd docs-site/docs && python3 -m http.server 8000`. Never `run_in_background`
  the server from a subagent that then waits on it — it never exits.
- PR previews: any PR touching `docs-site/**` gets
  `https://smeltsql.com/branches/pr-<N>/maes-words/` via `docs-pr-preview.yml`.
  A push to `main` deploys for real.

The bank is **87 words** (35 / 25 / 27 by tier), all emoji.

## The goal of the next session

Take the bank from 87 to a few hundred words and teach sight words directly,
without changing the progression model:

1. **Pictogram images** for the words emoji cannot show unambiguously.
2. **Colour words** as coloured tiles (no image needed).
3. **Sight-word activity**: the blank is the sight word, not the noun.

Andrew has approved this direction in principle. It still needs a spec update
and a plan (the repo's brainstorm → spec → plan → subagent-driven flow).

## 1. Pictogram sources (decided)

| Source | Licence | Use |
|---|---|---|
| **Mulberry Symbols** | CC BY-SA 2.0 UK (commercial OK) | Primary. ~3,400 SVGs, AAC vocabulary, British English ("mum"). GitHub: `straight-street/mulberry-symbols` (verify the current repo/URL before scripting). |
| **ARASAAC** | CC BY-NC-SA 4.0 | Gap-filler only. ~12,000 pictograms. Search API `https://api.arasaac.org/api/pictograms/en/search/<term>`; images at `https://static.arasaac.org/pictograms/<id>/<id>_500.png` (verify). NC licence is fine for a family game; note it in the attribution. |
| OpenMoji / Twemoji / Noto | CC BY-SA 4.0 / CC BY 4.0 / Apache 2.0 | Optional: self-host emoji SVGs so rendering is identical on every device and the "too new to render" pruning rule goes away. |

Constraints that must survive:

- **Vendor the SVGs** into `docs-site/docs/maes-words/img/` (no CDN, no
  fetches at runtime — the game is dependency-free and works offline-ish).
  Optimise them (`svgo` or a one-off script); a few hundred small SVGs is fine
  for a static site but check the total size.
- **Attribution page** required by both licences: a small
  `docs-site/docs/maes-words/credits.html` (adult-facing, linked from the
  Grown-up corner) listing Mulberry (CC BY-SA 2.0 UK) and ARASAAC (Government
  of Aragon, CC BY-NC-SA 4.0). BY-SA also means derivative symbols keep the
  licence — we don't modify them, so this is just attribution.
- **The ambiguity rule stays**: a five-year-old shown only the picture says
  exactly that word. Pictograms are *more* ambiguous than emoji for some
  concepts (an AAC "in" symbol is not obvious). Keep a per-word review pass
  and let reviewers drop entries, as the v1 reviewers did.
- Keep a generator script under `docs-site/tools/` (like
  `generate-words.mjs` for Anne's Words) that maps word → source → asset id,
  so the bank is reproducible and re-runnable. Hand-curation lives in the
  script's word list, not in edits to `words.js`.

### Word-bank shape change

`words.js` entries are `{ w, e, tier, noA? }` today (`e` = emoji, `noA` = no
"a" article: mass nouns and numbers). Proposed:

```js
{ w: 'mum', tier: 1, pic: { kind: 'svg', src: 'img/mum.svg' } }
{ w: 'cat', tier: 1, pic: { kind: 'emoji', text: '🐱' } }   // keep emoji where they're good
{ w: 'red', tier: 1, pic: { kind: 'colour', css: '#e53935' } }
```

`ui.js` renders a `pic` in one place (`renderPic(pic)`), used by Read tiles,
the Pick/Build/Sentence prompt, and the wall chips. Tests: every `pic.kind` is
one of the three; every `svg` `src` exists on disk (a words test can
`fs.existsSync` from `docs-site/tests/`); emoji uniqueness rule extends to
`src` uniqueness.

### Candidate words to add (start list, prune on review)

- Tier 1 CVC nouns: mum, dad, boy, girl, man, kid, toy, cup, mug, jug, pot,
  pan, lid, tap, cot, mat, rug, jam, mud, bun, bib, wig, fan, net, tub, hut,
  peg, pup, cub, hen, bug, nut, jar, nest, lamp, rat, van, gem (the last eight
  were dropped for emoji ambiguity; a pictogram may fix them).
- Verbs: run, jump, sit, hop, dig, eat, cut, hug, nap, sip, see, look, go,
  stop, play, help, swim, read.
- Adjectives: big, hot, wet, sad, mad, fun, old, new, bad, top, up, down.
- Colours (tiles, no image): red, blue, pink, green (5 letters — allow it, or
  keep to ≤4: red, blue, pink, grey, gold).

Verbs and adjectives in the Read activity ("jump" → pick the picture of
jumping) work with pictograms; check each one passes the ambiguity rule.

## 2. Sight-word activity (design sketch)

Peter and Jane key words: the, is, a, and, here, I, my, to, in, on, up, look,
come, see, go, you, we, it, at, for, are, was, has, said, this, they, with.
None are picturable, and today they appear only as the fixed frames.

New activity **Fill** (name open): a short sentence with the picture noun
filled in and the *sight word* blanked.

```
I ___ the 🐶       choices: like / see / am
___ is a 🐱        choices: Here / The / My
The 🐸 is ___      choices: here / up / in
```

- Sight words get their own bank (`SIGHT = [{ w: 'the', tier: 1 }, …]`) and
  their own mastery levels in the same `levels` map (no key clash: none of
  them are picturable nouns, but assert it in a test).
- Frames become templates with two slots: `{noun}` and `{sight}`, plus which
  sight words each frame admits. Reuse `framesFor` logic for articles.
- Distractors: other sight words of similar shape (the/then/them, is/it/in),
  same `distractors()` ranking by edit distance.
- Level ladder for sight words can be shorter (Fill at 0–1, review at 2) or
  the same four levels with Build for spelling "the". Decide in the spec.
- Round planning: `planRound` draws from active nouns and active sight words;
  keep "at most one review per round".

## 3. Suggested order of work

1. Spec diff (short): image sources + `pic` shape + credits page + Fill
   activity + colour tiles. Update the Requirements table and Word bank
   section; keep the timeless-oracle rule (no phase vocabulary in the spec).
2. Plan via the writing-plans skill; execute with subagent-driven development.
   Model routing that worked: Sonnet for implementers and first-pass reviews,
   Haiku for scoped re-reviews of tiny diffs, Fable for the final
   whole-branch review (it found the two real design gaps in v1).
3. Tasks, roughly: generator script + vendored SVGs + credits page →
   `pic` rendering + tests → colour tiles → sight bank + Fill activity in
   `game.js` (pure, tested) → Fill UI → polish/headless checks → merge.
4. Open a draft PR early so Andrew can play the preview URL while it builds.

## Gotchas learned in v1

- Author CSS with `display:flex` beats the UA `[hidden]` rule; `style.css`
  now has `[hidden] { display: none !important; }`. Keep it.
- A word repeated within a round must not climb the ladder through one
  activity: `refreshTurn` re-derives the activity from the current level in
  `showTurn`. Any new activity must be in `ACTIVITY_FOR_LEVEL` or handled by
  the same mechanism.
- "a {}" frames are withheld from vowel-initial and `noA` words. A pictogram
  bank adds more mass nouns (mud, jam) — flag them.
- Local `main` can diverge from `origin/main` if spec/plan commits are made on
  `main` before branching; rebase the feature branch onto `origin/main` and
  fast-forward, don't merge-commit.
- Reviewers found real problems every round; don't skip the per-task review
  to save time.

## Shipped 2026-09-28

The work this handoff scoped is done and merged: the word bank grew from 87
emoji-only words to **158 words** (67 vendored pictograms from Mulberry
Symbols and ARASAAC, 4 colour tiles, 87 emoji), a 28-word `SIGHT` bank was
added, and the Fill activity teaches those sight words directly against a
32-frame template set. Try-again (a wrong answer stays live until Mae gets
it, earning no level and no star) shipped alongside. Spec:
`docs/superpowers/specs/2026-09-27-maes-words-design.md` (updated in the same
work to describe `pos` on words/frames, `fits` exhaustiveness, and the
sight-word Build look-then-spell reveal).

What landed, by piece:

- `pic` descriptor (`emoji` / `svg` / `colour`) unified how every activity
  renders a word, via the single `renderPic(pic)` in `ui.js`.
- `pos` (`'verb'` | `'adj'`, default noun) on word and frame entries, so
  Fill/Sentence only pair a word with a frame of its own part of speech.
  Colour words are `pos: 'adj'` and always `noA`.
- `SIGHT` bank + `FRAMES.fits` (exhaustive per frame) drive the Fill
  activity; `pickFillFrameAndNoun` in `game.js` chooses the frame and the
  pictured word jointly so the article rule ("a" vs "the") never has to
  rewrite frame text.
- The generator, `docs-site/tools/generate-maes-pics.mjs`, fetches and
  optimises the vendored images and writes the `// BEGIN/END GENERATED
  PICS` block in `words.js` and the matching block in `credits.html`.

### How to add a word

1. Add a row to the `PICS` table at the top of
   `docs-site/tools/generate-maes-pics.mjs` — `{ w, tier, src: 'mulberry' |
   'arasaac', id, pos?, noA? }`.
2. Run `node docs-site/tools/generate-maes-pics.mjs` to fetch, optimise, and
   rewrite the generated blocks in `words.js` and `credits.html`.
3. Run `node docs-site/tools/generate-maes-pics.mjs --check` to confirm the
   generator is idempotent against what's checked in (it exits non-zero if
   the generated blocks would change — useful after hand-editing the table
   or after a source image changes upstream).
4. Open `docs-site/docs/maes-words/credits.html` in a browser and use it as
   a contact sheet: every pictogram is listed there next to its word, so a
   full-page skim catches ambiguity problems (the ambiguity rule: shown only
   the picture, a five-year-old says exactly that word) that are easy to
   miss reviewing the `PICS` table as text.
5. Run `node --test 'docs-site/tests/maes-words.*.test.mjs'` before
   committing.

### Borderline words for Andrew to judge on the preview

These passed review but are worth a second look by an actual five-year-old
(or Andrew's judgment of one) against the rendered pictogram, since some
concepts read differently in AAC symbol style than in emoji: **mop, bull,
sofa, leek, lock, wet, jam, read, play, paw**. If any fails the ambiguity
rule in practice, drop its row from `PICS`, remove the corresponding image
under `docs-site/docs/maes-words/img/`, and re-run the generator.

### Test and headless commands

```bash
node --test 'docs-site/tests/maes-words.*.test.mjs'   # unit suite

cd docs-site/docs && python3 -m http.server 8000      # serve for headless/manual play
# in another shell:
NODE_PATH=docs/demos/node_modules node docs-site/tests/maes-words-headless.mjs
```

The headless script now also covers the sight-word Build look phase: it
leaves a Build turn mid-look (before the 1200ms reveal) via the Grown-up
corner's long-press, waits past when the old timer would have fired, and
asserts a new round's prompt isn't wiped or force-revealed by it — the
regression a stale `setTimeout` produced when `turnIndex`/the visible screen
had already moved on.
