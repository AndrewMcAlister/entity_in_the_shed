const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

test('a character selection sends its character id', () => {
  const payload = mobileChatHelpers.buildSelectedVoicePayload({
    id: 'en-GB:brian',
    kind: 'character',
    name: 'Brian',
    provider: 'edge',
    lang: 'en-GB',
    culture: 'en-GB',
    source: 'server',
  });

  assert.equal(payload.character_id, 'en-GB:brian');
  assert.equal(payload.name, 'Brian');
});

test('an implicit voice selection sends the voice as its character id', () => {
  const payload = mobileChatHelpers.buildSelectedVoicePayload({
    id: 'en-GB-LibbyNeural',
    kind: 'voice',
    name: 'Libby',
    provider: 'edge',
    lang: 'en-GB',
    source: 'server',
  });

  assert.equal(payload.character_id, 'en-GB-LibbyNeural');
});

test('a browser selection claims no character id', () => {
  const payload = mobileChatHelpers.buildSelectedVoicePayload({
    id: 'browser:Microsoft Hazel',
    kind: 'browser',
    name: 'Microsoft Hazel',
    lang: 'en-GB',
    source: 'browser',
  });

  assert.equal(payload.character_id, '');
});

test('convertLegacyVoiceValue keeps a character id that itself contains a colon', () => {
  const known = new Set(['en-GB:brian', 'en-GB-ThomasNeural', 'browser:Microsoft Hazel']);

  assert.equal(mobileChatHelpers.convertLegacyVoiceValue('en-GB:brian', known), 'en-GB:brian');
});

test('convertLegacyVoiceValue maps a legacy server voice id onto the voice short name', () => {
  const known = new Set(['en-GB-ThomasNeural']);

  assert.equal(
    mobileChatHelpers.convertLegacyVoiceValue('edge:edge:en-GB-ThomasNeural', known),
    'en-GB-ThomasNeural',
  );
});

test('convertLegacyVoiceValue maps a legacy browser id onto the browser character id', () => {
  const known = new Set(['browser:Microsoft Hazel']);

  assert.equal(
    mobileChatHelpers.convertLegacyVoiceValue('browser:browser:Microsoft Hazel', known),
    'browser:Microsoft Hazel',
  );
});

test('convertLegacyVoiceValue drops a selection that no longer exists', () => {
  const known = new Set(['en-GB-ThomasNeural']);

  assert.equal(mobileChatHelpers.convertLegacyVoiceValue('edge:edge:en-GB-GoneNeural', known), '');
  assert.equal(mobileChatHelpers.convertLegacyVoiceValue('', known), '');
  assert.equal(mobileChatHelpers.convertLegacyVoiceValue(null, known), '');
});

test('character pitch helpers are neutral at 72 and mirror a deeper baseline', () => {
  assert.equal(mobileChatHelpers.characterDepthBias(72), 0);
  assert.equal(mobileChatHelpers.characterPitchMultiplier(72), 1);

  // A deeper baseline sits 20 below neutral. No character is named here: the
  // pitch is a delivery choice and it changes.
  assert.equal(mobileChatHelpers.characterDepthBias(52), -20);
  assert.ok(Math.abs(mobileChatHelpers.characterPitchMultiplier(52) - 0.8333333333333334) < 1e-9);

  // Missing or unparseable pitch must not move the voice.
  assert.equal(mobileChatHelpers.characterDepthBias(undefined), 0);
  assert.equal(mobileChatHelpers.characterPitchMultiplier('nonsense'), 1);
});

test('characterPitchMultiplier stays inside the Web Speech range', () => {
  const close = (actual, expected) => assert.ok(
    Math.abs(actual - expected) < 1e-9,
    `expected ${actual} to be within 1e-9 of ${expected}`,
  );

  close(mobileChatHelpers.characterPitchMultiplier(25), 0.6083333333333333);
  close(mobileChatHelpers.characterPitchMultiplier(130), 1.4833333333333334);
});

test('dedupeKeyFor keys characters by the server identity and voices by speaker and locale', () => {
  // The server mints the identity; the client echoes it rather than deriving
  // one. No test here names a voice a character actually uses -- the voice is a
  // delivery choice and it changes.
  const character = (country, region, name) => ({
    kind: 'character',
    character_country: country,
    character_region: region,
    character_name: name,
    dedupe_key: `character|${country}|${region}|${name}`.toLowerCase(),
  });
  const key = (entry) => mobileChatHelpers.dedupeKeyFor(entry);

  assert.equal(key(character('United Kingdom', 'Surrey', 'Reginald')), 'character|united kingdom|surrey|reginald');

  // Two characters sharing one voice file, differing only in region, stay two.
  const alpha = character('Canada', 'Nunavut', 'Siqiniq');
  const beta = character('Canada', 'Nunavik', 'Taqqiq');
  Object.assign(alpha, { name: 'Microsoft Shared Online (Natural)', voice: 'shared-voice' });
  Object.assign(beta, { name: 'Microsoft Shared Online (Natural)', voice: 'shared-voice' });
  assert.notEqual(key(alpha), key(beta));

  // The only duplicate is the same country, region and name.
  assert.equal(key(alpha), key(character('Canada', 'Nunavut', 'Siqiniq')));

  // Canada ships two Siqiniq in one region, one per Inuktitut script, so the
  // identity has to carry the script or the pair would collapse.
  assert.notEqual(key(character('Canada', 'Nunavut', 'Siqiniq')), key(character('Canada', 'Nunavut', 'Siqiniq-Latn')));

  // Plain voices keep the speaker/locale fold, unchanged.
  const voiceKey = mobileChatHelpers.dedupeKeyFor({
    kind: 'voice',
    id: 'en-US-BrianNeural',
    name: 'Brian',
    display_name: 'Microsoft Brian Online (Natural) - English (United States)',
    culture: 'en-US',
    source: 'server',
  });
  assert.equal(voiceKey, 'server|brian|en-US');
});

test('buildCharacterOptionLabel prefers the server label over the engine name', () => {
  assert.equal(
    mobileChatHelpers.buildCharacterOptionLabel({ label: 'Brian — United Kingdom' }),
    'Brian — United Kingdom',
  );

  assert.equal(
    mobileChatHelpers.buildCharacterOptionLabel({
      name: 'Microsoft Ryan Online (Natural) - English (United Kingdom)',
    }),
    'Ryan - English (United Kingdom)',
  );
});

test('buildCharacterInfoText gathers the character facts into one line', () => {
  const text = mobileChatHelpers.buildCharacterInfoText({
    id: 'United Kingdom:Alpha',
    character_name: 'Alpha',
    character_region: 'Testshire',
    details: {
      born: '1954',
      job: 'Synthetic archivist',
      backstory_lines: ['Alpha came from Testshire and stayed.', 'Alpha keeps the ledger.'],
      backstory: "Alpha's long account, written in the file.",
    },
  });

  assert.equal(
    text,
    "From Testshire, b. 1954, Job Synthetic archivist, Backstory Alpha came from "
      + "Testshire and stayed. Alpha keeps the ledger. Alpha's long account, written in the file.",
  );
});

test('buildCharacterInfoText reports only the facts a character authored', () => {
  // A character can be playable on a voice alone: every part but the region is
  // optional, and a missing one is left out rather than shown as a blank.
  assert.equal(
    mobileChatHelpers.buildCharacterInfoText({
      id: 'United Kingdom:Beta',
      character_region: 'Testmere',
      details: { born: '', job: '', backstory_lines: [], backstory: '' },
    }),
    'From Testmere',
  );

  // No details key at all is the shape an entry from an older server arrives in.
  assert.equal(
    mobileChatHelpers.buildCharacterInfoText({ id: 'United Kingdom:Beta' }),
    'No details are recorded for this character.',
  );
});

test('buildCharacterInfoText says whose facts are missing and asks when none is chosen', () => {
  assert.equal(
    mobileChatHelpers.buildCharacterInfoText({
      id: 'United Kingdom:Gamma',
      character_name: 'Gamma',
      label: 'Gamma - Testshire',
      details: {},
    }),
    'No details are recorded for Gamma.',
  );

  // The picker is showing its placeholder, so there is nobody to describe.
  for (const nothing of [null, undefined, {}, { character_name: 'Alpha' }]) {
    assert.equal(
      mobileChatHelpers.buildCharacterInfoText(nothing),
      'Choose a character to see who they are.',
    );
  }
});

test('compareVoices puts the better engine tier first', () => {
  // Covers static/app-helpers.js only. The templates/index.html copy of this
  // comparator is inline in the page and is not reachable from this harness;
  // the two bodies are kept byte-identical apart from indentation.
  const sorted = [
    { name: 'legacy', tier: 0, display_name: 'Microsoft David Desktop' },
    { name: 'neural', tier: 1, display_name: 'Microsoft Brian Online (Natural) - English (United States)' },
    { name: 'multilingual', tier: 2, display_name: 'Microsoft Ava Online (Natural) - English (United States)' },
  ].sort(mobileChatHelpers.compareVoices);

  assert.deepEqual(sorted.map((voice) => voice.name), ['multilingual', 'neural', 'legacy']);
});

test('a voice with no tier from the server ranks as the lowest tier', () => {
  // A browser voice carries no tier. The names here are picked so the terms
  // below the tier would put the browser voice first -- "United Kingdom" sorts
  // before "United States" -- so this fails if the tier is skipped rather than
  // treating a missing one as 0.
  const tiered = { name: 'Aaa', tier: 1, display_name: 'Microsoft Aaa Online (Natural) - English (United States)' };
  const browser = { name: 'Microsoft Zzz', display_name: 'Microsoft Zzz - English (United Kingdom)' };

  assert.ok(mobileChatHelpers.compareVoices(browser, tiered) > 0);

  // Number(undefined) || 0 and not bare arithmetic: undefined - undefined is
  // NaN, which sort reads as "equal". The same voice spelled with an explicit 0
  // has to tie with the tierless one, which is what "treated as 0" means.
  const explicitZero = { ...browser, tier: 0 };
  assert.equal(mobileChatHelpers.compareVoices(browser, explicitZero), 0);
});

test('voices of one tier fall through to the locale, then the display name', () => {
  const sorted = [
    { name: 'b', tier: 1, display_name: 'Microsoft Zed Online (Natural) - English (United Kingdom)' },
    { name: 'a', tier: 1, display_name: 'Microsoft Ada Online (Natural) - English (Australia)' },
  ].sort(mobileChatHelpers.compareVoices);

  assert.deepEqual(sorted.map((voice) => voice.name), ['a', 'b']);
});

test('a country roster is trusted while it is young', () => {
  const entry = { characters: [], loadedAt: 1_000_000 };

  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(entry, 1_000_000), true);
  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(entry, 1_029_000), true);
});

test('a country roster expires, so a restart is not invisible', () => {
  // The bug this exists for: a tab left open across a restart kept rendering the
  // pre-restart roster, because the cache never expired and every recovery the
  // page offers -- switching country away and back, returning to the tab --
  // re-entered the loader and took the cached path without asking the server.
  const entry = { characters: [], loadedAt: 1_000_000 };

  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(entry, 1_030_001), false);
  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(entry, 9_000_000), false);
});

test('a country never fetched is not fresh, and neither is an entry with no timestamp', () => {
  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(undefined, 1_000_000), false);
  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh(null, 1_000_000), false);
  // An entry written before the timestamp existed. Reading it as fresh would
  // reinstate the staleness; reading it as stale costs one refetch.
  assert.equal(mobileChatHelpers.characterCacheEntryIsFresh({ characters: [] }, 1_000_000), false);
  assert.equal(
    mobileChatHelpers.characterCacheEntryIsFresh({ characters: [], loadedAt: 'never' }, 1_000_000),
    false,
  );
});
