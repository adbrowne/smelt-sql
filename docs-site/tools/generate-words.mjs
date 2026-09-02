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
// Two separate pools:
//
//   GUESS pool (wide, permissive — anything a player might reasonably type):
//     Candidate pool = 5-letter lowercase words in (dwyl ∩ frequency-corpus)
//     ∪ (american-english ∪ british-english). The corpus intersection strips
//     dwyl junk entries (e.g. "arioi", "kanap") that never appear in real text.
//
//   ANSWER pool (narrow, curated — only words fair to ask a player to guess):
//     A word is answer-eligible only if ALL hold:
//       1. it is in the system dictionary (american-english ∪ british-english);
//       2. AND it is also in dwyl's list (two independent dictionaries must agree
//          — this is what keeps names/slang that only one source lists, e.g.
//          "izumi", "sitka", "purdy", "strom", out of the answer pool);
//       3. AND it appears in the frequency corpus (for ranking);
//       4. AND it contains at least one vowel [aeiou] (removes non-words like
//          Roman numerals, e.g. "xxvii");
//       5. AND it does not end in "s" (plurals), is not on the LDNOOBW
//          profanity list, and is not capitalised in either system dictionary
//          (proper-noun heuristic).
//     Answer-eligible words are ranked by frequency-corpus rank (most frequent
//     first); the top 2000 become ANSWERS, shuffled once with Fisher-Yates
//     driven by a mulberry32 PRNG seeded 0x9e3779b9 (documented seed, carried
//     over from the original list).
//
//   MANUAL_EXCLUSIONS: a small hand-maintained list of vulgar, non-word
//   (acronyms/abbreviations that source lists carry as lowercase entries,
//   e.g. "ascii", "cobol"), or otherwise unsuitable words that slip past the
//   automated filters (LDNOOBW profanity list, proper-noun heuristic, vowel
//   check). Applied to both ANSWERS and ALLOWED. Add stragglers here as
//   they're found.
//
//   ALLOWED = every candidate-pool (guess pool) word NOT in ANSWERS, minus
//   the profanity list and MANUAL_EXCLUSIONS.
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

// Hand-maintained exclusions for vulgar, non-word, or otherwise unsuitable
// words that slip past the automated filters. Applied to both ANSWERS and
// ALLOWED. Add stragglers here as they're discovered.
//   - cunny: vulgar slang, not on the LDNOOBW profanity list.
//   - mccoy, mckay: proper nouns (surnames) that dwyl's word list itself
//     carries as lowercase entries, independent of local-dict capitalisation.
//   - ascii, cobol: acronyms that dwyl's word list carries as lowercase
//     entries.
//   - xxvii: a Roman numeral present as a lowercase entry in the local
//     system dictionaries; not a word.
const MANUAL_EXCLUSIONS = new Set(['cunny', 'mccoy', 'mckay', 'ascii', 'cobol', 'xxvii']);

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
  const HAS_VOWEL = /[aeiou]/;

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

  // --- local dictionaries: any entry containing an uppercase letter (acronyms
  //     like ASCII/COBOL, internal-cap names like McCoy/McKay) is a proper
  //     noun, never a candidate word. Only already-lowercase entries are
  //     admitted to localWordSet — do NOT lowercase before testing, or
  //     acronyms/names silently become ordinary lowercase "words". ---
  const properNounSet = new Set();
  const localWordSet = new Set();
  const HAS_UPPERCASE = /[A-Z]/;
  for (const text of [americanText, britishText]) {
    for (const rawLine of text.split('\n')) {
      const line = rawLine.replace(/\r$/, '');
      if (!line) continue;
      if (HAS_UPPERCASE.test(line)) {
        properNounSet.add(line.toLowerCase());
        continue;
      }
      if (FIVE_LETTER_LOWER.test(line)) {
        localWordSet.add(line);
      }
    }
  }

  // --- GUESS pool (candidate pool) = (dwyl ∩ freqCorpus) ∪ (american ∪ british) ---
  const candidatePool = new Set();
  for (const w of dwylSet) {
    if (freqRank.has(w)) candidatePool.add(w);
  }
  for (const w of localWordSet) {
    candidatePool.add(w);
  }

  console.error(`Candidate (guess) pool: ${candidatePool.size} words`);

  // --- ANSWER pool: system dict ∩ dwyl ∩ freq-corpus, with a vowel, minus
  //     plurals/profanity/proper-nouns/manual exclusions ---
  const eligible = [...localWordSet].filter((w) => {
    if (!dwylSet.has(w)) return false;
    if (!freqRank.has(w)) return false;
    if (!HAS_VOWEL.test(w)) return false;
    if (w.endsWith('s')) return false;
    if (profanitySet.has(w)) return false;
    if (properNounSet.has(w)) return false;
    if (MANUAL_EXCLUSIONS.has(w)) return false;
    return true;
  });

  // Order by frequency rank (all eligible words are guaranteed to have one).
  eligible.sort((a, b) => freqRank.get(a) - freqRank.get(b));

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

  // --- ALLOWED = guess pool minus ANSWERS minus profanity minus manual exclusions ---
  const ALLOWED = [...candidatePool]
    .filter((w) => !answerSet.has(w) && !profanitySet.has(w) && !MANUAL_EXCLUSIONS.has(w))
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
// GUESS pool (ALLOWED ∪ ANSWERS) = 5-letter lowercase words in
// (dwyl ∩ frequency corpus) ∪ (american-english ∪ british-english).
//
// ANSWER pool: a word is answer-eligible only if it is in the system
// dictionary AND in dwyl AND in the frequency corpus AND contains a vowel
// AND does not end in "s" AND is not profanity-listed AND is not
// capitalised in either system dictionary (proper-noun heuristic) AND is
// not in the hand-maintained MANUAL_EXCLUSIONS list.
// ANSWERS = top ${ANSWER_COUNT} answer-eligible words by frequency rank,
// Fisher-Yates shuffled with a mulberry32 PRNG seeded 0x9e3779b9 (shuffled
// once, at generation time).
// ALLOWED = guess pool minus ANSWERS minus profanity minus MANUAL_EXCLUSIONS.
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
