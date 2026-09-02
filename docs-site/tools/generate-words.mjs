#!/usr/bin/env node
// Generator for Anne's Words' word lists (docs-site/docs/annes-words/words.js).
//
// Run manually: `node docs-site/tools/generate-words.mjs`
//
// Sources (fetched over HTTPS; all permissively licensed):
//   - Word list:        https://raw.githubusercontent.com/dwyl/english-words/master/words_alpha.txt (Unlicense)
//   - Frequency corpus:  https://norvig.com/ngrams/count_1w.txt (word<TAB>count, most-frequent-first)
//   - Profanity list:    https://raw.githubusercontent.com/LDNOOBW/List-of-Dirty-Naughty-Obscene-and-Otherwise-Bad-Words/master/en
//   - Local dictionaries: /usr/share/dict/american-english, /usr/share/dict/british-english (SCOWL)
//
// Pipeline:
//   1. Candidate pool = 5-letter lowercase words in (dwyl ∩ frequency-corpus)
//      ∪ (american-english ∪ british-english). The corpus intersection strips
//      dwyl junk entries (e.g. "arioi", "kanap") that never appear in real text.
//   2. Answer-eligible = candidate pool, ordered by frequency-corpus rank
//      (most frequent first), excluding words ending in "s" (plurals),
//      profanity-listed words, and proper nouns (a word that appears
//      capitalised, e.g. matching /^[A-Z][a-z]{4}$/, in either local
//      dictionary file).
//   3. ANSWERS = top 2000 answer-eligible words, shuffled with Fisher-Yates
//      driven by a mulberry32 PRNG seeded with 0x9e3779b9 (documented seed,
//      carried over from the original list). Shuffled at generation time.
//   4. ALLOWED = every candidate-pool word NOT in ANSWERS, minus profanity.
//
// Output: a plain ES module exporting `ANSWERS` and `ALLOWED` arrays of
// lowercase 5-letter words, consumed as-is by ui.js (VALID = union of both).
//
// Re-running this script is deterministic: given unchanged upstream sources,
// it produces byte-identical output.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const DWYL_URL = 'https://raw.githubusercontent.com/dwyl/english-words/master/words_alpha.txt';
const FREQ_URL = 'https://norvig.com/ngrams/count_1w.txt';
const PROFANITY_URL =
  'https://raw.githubusercontent.com/LDNOOBW/List-of-Dirty-Naughty-Obscene-and-Otherwise-Bad-Words/master/en';
const AMERICAN_DICT = '/usr/share/dict/american-english';
const BRITISH_DICT = '/usr/share/dict/british-english';

const SEED = 0x9e3779b9;
const ANSWER_COUNT = 2000;

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const OUTPUT_PATH = path.resolve(__dirname, '../docs/annes-words/words.js');

async function fetchText(url) {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`Failed to fetch ${url}: ${res.status} ${res.statusText}`);
  }
  return res.text();
}

// mulberry32 PRNG — small, deterministic, seedable.
function mulberry32(seed) {
  let a = seed >>> 0;
  return function () {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function fisherYatesShuffle(arr, rng) {
  const out = arr.slice();
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

async function main() {
  console.error('Fetching sources...');
  const [dwylText, freqText, profanityText] = await Promise.all([
    fetchText(DWYL_URL),
    fetchText(FREQ_URL),
    fetchText(PROFANITY_URL),
  ]);

  const americanText = readFileSync(AMERICAN_DICT, 'utf8');
  const britishText = readFileSync(BRITISH_DICT, 'utf8');

  const FIVE_LETTER_LOWER = /^[a-z]{5}$/;

  // --- dwyl word list: strip CRLF, lowercase, keep 5-letter words ---
  const dwylSet = new Set(
    dwylText
      .split('\n')
      .map((w) => w.replace(/\r$/, ''))
      .filter((w) => FIVE_LETTER_LOWER.test(w))
  );

  // --- frequency corpus: word<TAB>count, already ordered most-frequent-first ---
  const freqRank = new Map(); // word -> rank (0 = most frequent)
  {
    let rank = 0;
    for (const line of freqText.split('\n')) {
      if (!line) continue;
      const tabIdx = line.indexOf('\t');
      if (tabIdx === -1) continue;
      const word = line.slice(0, tabIdx).toLowerCase();
      if (FIVE_LETTER_LOWER.test(word) && !freqRank.has(word)) {
        freqRank.set(word, rank);
      }
      rank++;
    }
  }

  // --- profanity list ---
  const profanitySet = new Set(
    profanityText
      .split('\n')
      .map((w) => w.replace(/\r$/, '').trim().toLowerCase())
      .filter(Boolean)
  );

  // --- local dictionaries: capitalised entries = proper nouns; all entries (lowercased) = candidates ---
  const properNounSet = new Set();
  const localWordSet = new Set();
  const CAP_PROPER_NOUN = /^[A-Z][a-z]{4}$/;
  for (const text of [americanText, britishText]) {
    for (const rawLine of text.split('\n')) {
      const line = rawLine.replace(/\r$/, '');
      if (!line) continue;
      if (CAP_PROPER_NOUN.test(line)) {
        properNounSet.add(line.toLowerCase());
      }
      const lower = line.toLowerCase();
      if (FIVE_LETTER_LOWER.test(lower)) {
        localWordSet.add(lower);
      }
    }
  }

  // --- candidate pool = (dwyl ∩ freqCorpus) ∪ (american ∪ british) ---
  const candidatePool = new Set();
  for (const w of dwylSet) {
    if (freqRank.has(w)) candidatePool.add(w);
  }
  for (const w of localWordSet) {
    candidatePool.add(w);
  }

  console.error(`Candidate pool: ${candidatePool.size} words`);

  // --- answer-eligible: candidate pool, ranked by frequency, excluding plurals/profanity/proper nouns ---
  const eligible = [...candidatePool].filter((w) => {
    if (w.endsWith('s')) return false;
    if (profanitySet.has(w)) return false;
    if (properNounSet.has(w)) return false;
    return true;
  });

  // Order by frequency rank; words with no frequency rank (local-dict-only
  // words not in the freq corpus) sort after all ranked words, in a stable
  // deterministic (alphabetical) order among themselves.
  eligible.sort((a, b) => {
    const ra = freqRank.has(a) ? freqRank.get(a) : Infinity;
    const rb = freqRank.has(b) ? freqRank.get(b) : Infinity;
    if (ra !== rb) return ra - rb;
    return a < b ? -1 : a > b ? 1 : 0;
  });

  console.error(`Answer-eligible: ${eligible.length} words`);

  const topEligible = eligible.slice(0, ANSWER_COUNT);
  if (topEligible.length < ANSWER_COUNT) {
    throw new Error(
      `Only ${topEligible.length} answer-eligible words available, need ${ANSWER_COUNT}`
    );
  }

  const rng = mulberry32(SEED);
  const ANSWERS = fisherYatesShuffle(topEligible, rng);
  const answerSet = new Set(ANSWERS);

  // --- ALLOWED = candidate pool minus ANSWERS minus profanity ---
  const ALLOWED = [...candidatePool]
    .filter((w) => !answerSet.has(w) && !profanitySet.has(w))
    .sort();

  console.error(`ANSWERS: ${ANSWERS.length}, ALLOWED: ${ALLOWED.length}`);

  const header = `// Generated by docs-site/tools/generate-words.mjs — DO NOT EDIT BY HAND.
// Regenerate with: node docs-site/tools/generate-words.mjs
//
// Sources: dwyl/english-words (words_alpha.txt, Unlicense), Norvig's Google
// Books ngram unigram frequency corpus (count_1w.txt), LDNOOBW profanity list
// (en), and the local SCOWL american-english/british-english dictionaries
// (used for extra candidates and proper-noun detection).
//
// Candidate pool = 5-letter lowercase words in (dwyl ∩ frequency corpus)
// ∪ (american-english ∪ british-english).
// Answer-eligible = candidate pool, ranked by frequency (most frequent
// first), excluding words ending in "s", profanity, and proper nouns
// (capitalised in either local dictionary).
// ANSWERS = top ${ANSWER_COUNT} answer-eligible words, Fisher-Yates shuffled with a
// mulberry32 PRNG seeded 0x9e3779b9 (shuffled once, at generation time).
// ALLOWED = candidate pool minus ANSWERS minus profanity.
`;

  const body =
    `export const ANSWERS = ${JSON.stringify(ANSWERS)};\n` +
    `export const ALLOWED = ${JSON.stringify(ALLOWED)};\n`;

  writeFileSync(OUTPUT_PATH, header + body);
  console.error(`Wrote ${OUTPUT_PATH}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
