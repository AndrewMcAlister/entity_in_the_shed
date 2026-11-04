const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

const BEAT = mobileChatHelpers.SPEECH_EMPHASIS_BEAT;
// The spellings and the variant name come from the module, so tuning the sound
// (the one open question in this file, settled by ear) does not break a test.
const HUM = mobileChatHelpers.VOCALIZATION_HUM;
const AGREEMENT = mobileChatHelpers.VOCALIZATION_AGREEMENT;
const VOCALIZATION = mobileChatHelpers.VARIANT_VOCALIZATION;
const URGENT = mobileChatHelpers.VARIANT_VOCALIZATION_URGENT;
const AHH = mobileChatHelpers.VARIANT_VOCALIZATION_AHH;
const AHH_URGENT = mobileChatHelpers.VARIANT_VOCALIZATION_AHH_URGENT;
const AHH_SOUND = mobileChatHelpers.VOCALIZATION_AHH;

function respell(text) {
  return mobileChatHelpers.respellVocalizations(text);
}

// The tone names the prompts may write, in the shape sentenceTone is handed.
// The real table is TONE_TTS_SETTINGS in agent.py; only membership matters here.
const TONES = { excited: {}, somber: {}, whisper: {} };

function strip(text) {
  return mobileChatHelpers.stripSpeechMarkers(text);
}

function sentence(raw) {
  return mobileChatHelpers.buildSpokenSentence(raw, TONES, (value) => String(value || '').trim());
}

function spokenPieces(raw) {
  return mobileChatHelpers.buildSpokenSentenceSegments(
    raw,
    TONES,
    (value) => String(value || '').trim(),
  );
}

test('every wrapper form keeps its words and loses its markup', () => {
  // The wrapper ends the text here, so no beat applies and what is left is
  // exactly the words -- the beat is the next test's business.
  assert.equal(strip('a *so*'), 'a so');
  assert.equal(strip('a **mad**'), 'a mad');
  assert.equal(strip('a _slow_'), 'a slow');
  assert.equal(strip('a [aside]'), 'a aside');
  assert.equal(strip('a (aside)'), 'a aside');
  assert.equal(strip('a {excited}loud{/excited}'), 'a loud');
});

test('a closing run that a word follows is left with a beat after it', () => {
  // This is the whole point of the change: the emphasis is heard as a beat
  // inside the sentence's own clip, not as a clip of its own. The beat carries
  // its own spaces and the wrapper's trailing space is dropped, so the dash is
  // never left with two of them.
  assert.equal(strip('it was *everything* good'), `it was everything${BEAT}good`);
  assert.equal(strip('**bold** text'), `bold${BEAT}text`);
  assert.equal(strip('_slow_ down'), `slow${BEAT}down`);
  assert.equal(strip('[aside] then'), `aside${BEAT}then`);
  assert.equal(strip('(aside) then'), `aside${BEAT}then`);
  // A wrapper mid-sentence keeps the space in front of it untouched.
  assert.equal(strip('like, *so* good'), `like, so${BEAT}good`);
});

test('a beat is not added where punctuation already makes one', () => {
  for (const mark of ['.', ',', '!', '?', ';', ':', '—', '…']) {
    assert.equal(
      strip(`it was *everything*${mark} how are you`),
      `it was everything${mark} how are you`,
      `a beat was added in front of "${mark}"`,
    );
  }
});

test('a run that ends the text or the sentence takes no beat', () => {
  assert.equal(strip('it was *everything*'), 'it was everything');
  assert.equal(strip('*One*'), 'One');
});

test('a beat is not added in front of a closing tag the pass removes first', () => {
  // The tag is control text, so "*so*{/excited}." has to see the full stop and
  // skip the beat it would otherwise leave stranded in front of it.
  assert.equal(strip('it was *so*{/excited}.'), 'it was so.');
  assert.equal(strip('it was *so*{/excited}, right'), 'it was so, right');
});

test('an underscore inside a word is not emphasis', () => {
  assert.equal(strip('open my_script_py now'), 'open my_script_py now');
  assert.equal(strip('the my_file_name here'), 'the my_file_name here');
});

test('a wrapper naming a known tone is a delivery, not a word', () => {
  // The reported reply: the words "whisper" and "/whisper" were spoken in front
  // of the phrase they were meant to style. The wrapper is the tone tag in the
  // dialect a model reaches for; the name is control text.
  const toned = (text) => mobileChatHelpers.stripSpeechMarkers(text, TONES);
  assert.equal(toned('[whisper]I want this.'), 'I want this.');
  assert.equal(toned('[/whisper] Tell me exactly.').trim(), 'Tell me exactly.');
  assert.equal(toned('and [whisper] I want'), 'and I want');
  assert.equal(toned('a *whisper* b'), 'a b');
  assert.equal(toned('a _whisper_ b'), 'a b');
  assert.equal(toned('a (whisper) b'), 'a b');
});

test('a wrapper holding words is still said, even a tone word among them', () => {
  const toned = (text) => mobileChatHelpers.stripSpeechMarkers(text, TONES);
  assert.equal(toned('a [whisper softly]'), 'a whisper softly');
});

test('the sentence hears the tone a wrapper names', () => {
  const built = sentence('[whisper]I want this.[/whisper]');
  assert.equal(built.text, 'I want this.');
  assert.equal(built.tone, 'whisper');
  // A tone tag is not emphasis, so it leaves no beat and no marked profile.
  assert.equal(built.variant, 'base');
});

test('a closing wrapper on its own names no tone', () => {
  // The model closes the region at the start of the next sentence: that
  // sentence is said in the ordinary voice and keeps none of the tag.
  const built = sentence('[/whisper] Tell me exactly.');
  assert.equal(built.text, 'Tell me exactly.');
  assert.equal(built.tone, null);
});


test('a run of hum letters is one hum, not one letter at a time', () => {
  // The reported fault: the voices spelled "mmmm" out. A spelling a voice knows
  // is what fixes it, and every hum spelling reduces to that one spelling.
  assert.equal(respell('mmm'), HUM);
  assert.equal(respell('Mmm'), HUM);
  assert.equal(respell(HUM.toUpperCase()), HUM);
  assert.equal(respell('mmmm'), HUM);
  assert.equal(respell('Mmmm'), HUM);
  assert.equal(respell('mmmmmm'), HUM);
  assert.equal(respell('MMMM'), HUM);
  assert.equal(respell('Mmmm...'), `${HUM}...`);
});

test('the hum spellings the entity reaches for are one sound', () => {
  // "mmm" may be written with a breath on the end. They are not three sounds,
  // so they are not three spellings on the way to the voice either.
  for (const spelling of ['mmm', 'Mmm', 'mmmh', 'Mmmh', 'mmmnh', 'Mmmnh', 'mmmhh']) {
    assert.equal(respell(spelling), HUM, spelling);
  }
  assert.equal(sentence('Mmmh. my breath slows.').text, 'mmm. my breath slows.');
});

test('a spelling a voice already hums is left alone', () => {
  // "hmm" hums, but opens with the h rather than the /m/ the entity's hums
  // are; it is the entity's own spelling and is not rewritten.
  assert.equal(respell('hmm'), 'hmm');
  assert.equal(respell('Hmm'), 'Hmm');
  assert.equal(respell(AGREEMENT), AGREEMENT);
  assert.equal(respell('Mm-hmm'), 'Mm-hmm');
});

test('the written forms of the agreement are said as the agreement', () => {
  assert.equal(respell('mhm'), AGREEMENT);
  assert.equal(respell('Mhm'), AGREEMENT);
  assert.equal(respell('mmhmm'), AGREEMENT);
  assert.equal(respell('MHM'), AGREEMENT);
});

test('the hum letters in any order become the hum', () => {
  // Letter soup the entity writes by hand; the hum is the honest reading.
  for (const spelling of ['mmhnnh', 'hnnn', 'mnnn', 'hmhm']) {
    assert.equal(respell(spelling), HUM, spelling);
  }
});

test('a run of n\'s alone is not turned into a hum', () => {
  // The n forms no part of the sound: it is dropped from the spellings that do
  // carry one, so on its own it is a letter run and not a sound the pass can
  // invent a hum out of.
  for (const spelling of ['nnn', 'Nnn', 'nnnn']) {
    assert.equal(respell(spelling), spelling, spelling);
  }
});

test('the spelling pass is idempotent, so a re-spoken sentence does not drift', () => {
  for (const spelling of ['mmmm', 'mmhnnh', 'mhm', AGREEMENT]) {
    const once = respell(spelling);
    assert.equal(respell(once), once, spelling);
  }
});

test('words, units and identifiers are not hums', () => {
  // A run of h/m/n letters that cannot be a hum: a real word, a unit, a name.
  for (const text of ['hymn', 'men', 'human', 'minimum', '10 mm', 'mm']) {
    assert.equal(respell(text), text, text);
  }
  assert.equal(respell('hmm_value'), 'hmm_value');
  assert.equal(respell('my_hmm'), 'my_hmm');
  assert.equal(respell('hmm-thing'), 'hmm-thing');
  assert.equal(respell("Mmmm's"), "Mmmm's");
});

test('a hum inside a URL or an address is not rewritten', () => {
  // The server protects these spans from its own normalisation; this pass has
  // to agree, or a link would be spoken as something else.
  assert.equal(respell('https://hmm.example/path'), 'https://hmm.example/path');
  assert.equal(respell('hmmm@example.com'), 'hmmm@example.com');
  assert.equal(respell('https://hmm.example mmmm'), `https://hmm.example ${HUM}`);
});

test('a sentence that is nothing but a hum gets the vocalisation delivery', () => {
  // This is what makes the sound long: the profile plays its clip slower.
  assert.equal(mobileChatHelpers.sentenceVariant('Mmmm.'), VOCALIZATION);
  assert.equal(mobileChatHelpers.sentenceVariant('Mmmm... mmmm.'), VOCALIZATION);
  assert.equal(mobileChatHelpers.sentenceVariant('MMMM'), VOCALIZATION);
  // A wrapper around a hum is still only a hum, and the slower delivery wins
  // over the marked one.
  assert.equal(mobileChatHelpers.sentenceVariant('[mmmm]'), VOCALIZATION);
  assert.equal(sentence('[mmmm]').variant, VOCALIZATION);
});

test('a hum that carries words is not slowed with them', () => {
  assert.equal(mobileChatHelpers.sentenceVariant('Mmmm, that is good.'), 'base');
  // "hmm" is the one hum spelling the pass leaves alone, so whether a bare
  // sentence of it is slowed follows the tuned spelling: it is the hum only
  // once the hum is spelled that way.
  assert.equal(
    mobileChatHelpers.sentenceVariant('Hmm.'),
    HUM.toLowerCase() === 'hmm' ? VOCALIZATION : 'base',
  );
  // The agreement is a reply, not a drawn-out sound.
  assert.equal(mobileChatHelpers.sentenceVariant(AGREEMENT), 'base');
  assert.equal(mobileChatHelpers.sentenceVariant('mhm.'), 'base');
  // A marked sentence with no hum in it is still marked.
  assert.equal(mobileChatHelpers.sentenceVariant('has *so* in it'), 'marked');
});

test('a hum passes through the wrappers like any other word', () => {
  assert.equal(sentence('[mmmm]').text, HUM);
  assert.equal(sentence('(Mmmm.)').text, `${HUM}.`);
  assert.equal(sentence('*mmmm* then').text, `${HUM}${BEAT}then`);
  assert.equal(sentence('*mmmm* then').variant, 'marked');
});

test('a hum written with a breath in it takes the urgent shape', () => {
  // "mmmh" and "mmmnh" are said the same as "mmm" -- one sound, one spelling --
  // but the h makes them a touch urgent, and an urgent hum opens higher.
  for (const spelling of ['mmmh', 'mmmnh', 'Mmmh', 'mmmhh']) {
    assert.equal(sentence(spelling).text, HUM, spelling);
    assert.equal(sentence(spelling).variant, URGENT, spelling);
    assert.equal(mobileChatHelpers.sentenceVariant(spelling), URGENT, spelling);
  }
  // The plain hum keeps the ordinary shape.
  assert.equal(sentence('mmm').variant, VOCALIZATION);
  assert.equal(sentence('[mmmh]').variant, URGENT);
  // A hum that carries words is neither shape, breath or not.
  assert.equal(mobileChatHelpers.sentenceVariant('Mmmh, right.'), 'base');
  // The forms the pass leaves alone are not hums here either. "hmm" keeps its
  // h precisely because a voice says it right, so it is no breath to find.
  assert.equal(mobileChatHelpers.humHasBreath('hmm'), false);
  assert.equal(mobileChatHelpers.humHasBreath(AGREEMENT), false);
  assert.equal(mobileChatHelpers.humHasBreath('mmm'), false);
});

test('an open "ahhh" is a sound of its own, and more h\'s make it urgent', () => {
  // Three h's is the softer sound of an intimate moment; a fourth makes it the
  // more urgent one. Every spelling is said as the one canonical sound.
  for (const spelling of ['ahhh', 'Ahhh', 'aahhh', 'ahhhh', 'Ahhhh']) {
    assert.equal(respell(spelling), AHH_SOUND, spelling);
  }
  assert.equal(sentence('Ahhh.').text, `${AHH_SOUND}.`);
  assert.equal(sentence('Ahhh.').variant, AHH);
  assert.equal(sentence('Ahhhhh!').variant, AHH_URGENT);
  assert.equal(sentence('[Ahhhhh]').variant, AHH_URGENT);

  // A word that merely starts with those letters, and the ordinary "ah", are
  // neither the sound nor slowed.
  assert.equal(mobileChatHelpers.sentenceVariant('Ah, I see.'), 'base');
  assert.equal(mobileChatHelpers.sentenceVariant('Ahead of you.'), 'base');
  assert.equal(mobileChatHelpers.sentenceVariant('ahhh-thing'), 'base');
  // A sound carrying words is not slowed with them.
  assert.equal(mobileChatHelpers.sentenceVariant('Ahhh, yes.'), 'base');
  // An open sound beside a hum carries the line, and its own urgency decides
  // the shape.
  assert.equal(mobileChatHelpers.sentenceVariant('Mmm... ahhh.'), AHH);
  assert.equal(mobileChatHelpers.sentenceVariant('Mmm... ahhhhh.'), AHH_URGENT);
  assert.deepEqual(mobileChatHelpers.ahhSounds('Ahhh and ahhhhh'), ['ahhh', 'ahhhhh']);
});

test('every sound variant is slowed and shaped, and no other variant is', () => {
  // The profile and the renderer both read this one predicate; a sound missing
  // from it would be spoken at the base rate and never bent.
  for (const variant of [VOCALIZATION, URGENT, AHH, AHH_URGENT]) {
    assert.equal(mobileChatHelpers.isVocalizationVariant(variant), true, variant);
  }
  for (const variant of ['base', 'marked', '', null, undefined]) {
    assert.equal(mobileChatHelpers.isVocalizationVariant(variant), false, String(variant));
  }
});

test('the hum\'s fall is placed on the sound, not on the silence around it', () => {
  // The engine pads the clip; the drop has to happen while the hum is heard,
  // or most of it would be spent on silence and the end would only fall a few
  // per cent.
  const rate = 1000;
  const samples = new Float32Array(1000);
  for (let index = 200; index < 600; index += 1) {
    samples[index] = Math.sin(index * 0.3) * 0.5;
  }

  assert.deepEqual(mobileChatHelpers.voicedSpan(samples, rate), { start: 0.2, end: 0.6 });
  // A clip with no sound in it is the whole clip, not a zero-length span.
  assert.deepEqual(
    mobileChatHelpers.voicedSpan(new Float32Array(500), rate),
    { start: 0, end: 0.5 },
  );
});

test('the hum holds the voice\'s pitch until it sounds, then falls a tenth', () => {
  const plan = mobileChatHelpers.vocalizationPitchPlan(1, { start: 0.2, end: 0.6 });

  assert.equal(plan.holdUntil, 0.2);
  assert.equal(plan.rampEnd, 0.6);
  assert.equal(plan.pitchStart, mobileChatHelpers.VOCALIZATION_PITCH_START);
  assert.equal(plan.pitchEnd, mobileChatHelpers.VOCALIZATION_PITCH_END);
  // 0.2 s at 1.0 and 0.4 s averaging 0.95 consume 0.58 s of the source; the
  // rest plays at 0.9. The render outlasts the clip, which is what the slow
  // playback is for.
  const expected = 0.6 + (1 - (0.2 + 0.4 * 0.95)) / 0.9;
  assert.ok(Math.abs(plan.sourceEnd - expected) < 1e-9, String(plan.sourceEnd));

  // A hum with barely a tail still plays all of itself: only the last sliver
  // of the source is left for after the ramp.
  const short = mobileChatHelpers.vocalizationPitchPlan(0.3, { start: 0.1, end: 0.3 });
  assert.ok(Math.abs(short.sourceEnd - (0.3 + 0.01 / 0.9)) < 1e-9, String(short.sourceEnd));

  // A clip with no sound in it has no fall to place: it plays through whole.
  assert.equal(mobileChatHelpers.vocalizationPitchPlan(0.5, { start: 0, end: 0 }).sourceEnd, 0.5);
  // A clip with no length is no time at all.
  assert.equal(mobileChatHelpers.vocalizationPitchPlan(0, { start: 0, end: 0 }).sourceEnd, 0);
});

test('an urgent hum opens a tenth higher and comes to rest where a plain one began', () => {
  const plain = mobileChatHelpers.vocalizationPitchShape(VOCALIZATION);
  assert.equal(plain.startPitch, mobileChatHelpers.VOCALIZATION_PITCH_START);
  assert.equal(plain.endPitch, mobileChatHelpers.VOCALIZATION_PITCH_END);

  const shape = mobileChatHelpers.vocalizationPitchShape(URGENT);
  assert.equal(shape.startPitch, mobileChatHelpers.VOCALIZATION_URGENT_PITCH_START);
  assert.equal(shape.startPitch, 1.1);
  assert.ok(Math.abs(shape.endPitch - 0.99) < 1e-9, String(shape.endPitch));

  const plan = mobileChatHelpers.vocalizationPitchPlan(1, { start: 0.2, end: 0.6 }, shape.startPitch, shape.endPitch);
  assert.equal(plan.pitchStart, 1.1);
  assert.ok(Math.abs(plan.pitchEnd - 0.99) < 1e-9, String(plan.pitchEnd));
  // Played slightly fast, so the source is used up sooner than the clip lasts.
  const consumed = 0.2 * 1.1 + 0.4 * ((1.1 + 0.99) / 2);
  assert.ok(Math.abs(plan.sourceEnd - (0.6 + (1 - consumed) / 0.99)) < 1e-9, String(plan.sourceEnd));

  // A clip that ends just past the sound runs out while the rate is still
  // coming down, and the crossing has to be solved on the ramp itself.
  const tight = mobileChatHelpers.vocalizationPitchPlan(0.62, { start: 0.2, end: 0.6 }, 1.1, 0.99);
  assert.ok(tight.sourceEnd > 0.2 && tight.sourceEnd < 0.6, String(tight.sourceEnd));
  assert.ok(Math.abs(tight.sourceEnd - 0.581864) < 1e-5, String(tight.sourceEnd));
});

test('the softer ahhh falls a tenth; the urgent one climbs a ladder of starts', () => {
  const soft = mobileChatHelpers.vocalizationPitchShape(AHH);
  // Medium urgency: the voice's own middle, a tenth below by the end.
  assert.equal(soft.startPitch, mobileChatHelpers.VOCALIZATION_PITCH_START);
  assert.ok(Math.abs(soft.endPitch - 0.9) < 1e-9, String(soft.endPitch));

  // At least five rungs from the voice's own pitch: the fourth h is the first
  // urgent step, base rung, and every breath after it carries the sound higher.
  // Each one falls to the softer ahhh's landing, so the urgency is the longer
  // fall.
  const starts = mobileChatHelpers.VOCALIZATION_AHH_URGENT_PITCH_STARTS;
  assert.ok(starts.length >= 5, String(starts.length));
  // The base rung opens at normal pitch -- where the ear wanted the ladder
  // brought down to -- and the top rung reaches a fifth above the voice.
  assert.equal(starts[0], mobileChatHelpers.VOCALIZATION_PITCH_START);
  assert.equal(starts[starts.length - 1], 1.2);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 4).startPitch, starts[0]);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 5).startPitch, starts[1]);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 6).startPitch, starts[2]);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 7).startPitch, starts[3]);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 8).startPitch, starts[4]);
  // Beyond the top rung, and for a caller that cannot say how many breaths
  // there were, the ladder stops at its ends rather than falling off.
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 20).startPitch, starts[starts.length - 1]);
  assert.equal(mobileChatHelpers.vocalizationPitchShape(AHH_URGENT).startPitch, starts[0]);
  for (let index = 1; index < starts.length; index += 1) {
    assert.ok(starts[index] > starts[index - 1], String(starts));
  }

  const urgent = mobileChatHelpers.vocalizationPitchShape(AHH_URGENT, 4);
  assert.equal(urgent.startPitch, mobileChatHelpers.VOCALIZATION_PITCH_START);
  assert.equal(urgent.startPitch, soft.startPitch);
  assert.ok(starts[starts.length - 1] > mobileChatHelpers.VOCALIZATION_URGENT_PITCH_START);
  assert.ok(Math.abs(urgent.endPitch - 0.9) < 1e-9, String(urgent.endPitch));
  assert.equal(urgent.endPitch, soft.endPitch);
  assert.ok(urgent.endPitch < 0.99, String(urgent.endPitch));

  const plan = mobileChatHelpers.vocalizationPitchPlan(1, { start: 0.2, end: 0.6 }, urgent.startPitch, urgent.endPitch);
  assert.equal(plan.pitchStart, urgent.startPitch);
  assert.equal(plan.pitchEnd, urgent.endPitch);
  // The fall opens no higher than the voice, so across the span the source is
  // used a little more slowly than the clip: the render runs a touch past its
  // own length to play the whole sound.
  const consumed = 0.2 * urgent.startPitch + 0.4 * ((urgent.startPitch + urgent.endPitch) / 2);
  assert.ok(Math.abs(consumed - 0.58) < 1e-9, String(consumed));
  assert.ok(Math.abs(plan.sourceEnd - (0.6 + (1 - consumed) / urgent.endPitch)) < 1e-9, String(plan.sourceEnd));
});

test('a run of open sounds is heard one at a time, climbing slightly each time', () => {
  // At the most intense moments the sound is written several times in a row.
  // Each one is its own clip, and each opens a little higher than the one
  // before: sharing one clip would flatten them all to the highest breath in
  // the line and put a single fall across the series.
  const pieces = spokenPieces('Ahhhh, ahhhhh, ahhhhhh!').segments;
  assert.equal(pieces.length, 3);
  for (const piece of pieces) {
    assert.equal(piece.variant, AHH_URGENT);
    assert.equal(piece.text, `${AHH_SOUND}.`);
  }
  const ladder = mobileChatHelpers.VOCALIZATION_AHH_URGENT_PITCH_STARTS;
  assert.deepEqual(pieces.map((piece) => piece.pitchStart), [ladder[0], ladder[1], ladder[2]]);

  // The climb does not depend on an extra h being written: five identical
  // sounds still take the five rungs, one after another.
  const same = spokenPieces('Ahhhh, ahhhh, ahhhh, ahhhh, ahhhh.').segments;
  assert.equal(same.length, 5);
  assert.deepEqual(same.map((piece) => piece.pitchStart), ladder);

  // Words between sounds end the series, so the next one starts again from the
  // height its own h's earn rather than where the last one left off.
  const broken = spokenPieces('Ahhhh, ahhhh, wait, ahhhh, ahhhh.').segments;
  assert.deepEqual(
    broken.filter((piece) => piece.variant === AHH_URGENT).map((piece) => piece.pitchStart),
    [ladder[0], ladder[1], ladder[0], ladder[1]],
  );

  // A single sound is still the whole sentence, in one clip, at the height its
  // own h's earn.
  const single = spokenPieces('Ahhhhh.').segments;
  assert.equal(single.length, 1);
  assert.equal(single[0].pitchStart, ladder[1]);
  // A run of hums is split the same way, so each hum is drawn out on its own.
  const hums = spokenPieces('Mmm... mmm.').segments;
  assert.equal(hums.length, 2);
  assert.deepEqual(hums.map((piece) => piece.text), [`${HUM}.`, `${HUM}.`]);
});

test('a rendered clip reaches the player as a WAV', () => {
  const samples = new Float32Array([0, 0.5, -0.5, 1, -1]);
  const bytes = mobileChatHelpers.encodeWav([samples], 24000);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const text = (offset, length) => String.fromCharCode(...bytes.subarray(offset, offset + length));

  assert.equal(text(0, 4), 'RIFF');
  assert.equal(text(8, 4), 'WAVE');
  assert.equal(text(12, 4), 'fmt ');
  assert.equal(text(36, 4), 'data');
  assert.equal(view.getUint16(20, true), 1);          // PCM
  assert.equal(view.getUint16(22, true), 1);          // mono
  assert.equal(view.getUint32(24, true), 24000);
  assert.equal(view.getUint16(34, true), 16);
  assert.equal(view.getUint32(40, true), samples.length * 2);
  assert.equal(bytes.length, 44 + samples.length * 2);
  assert.equal(view.getInt16(44, true), 0);
  assert.equal(view.getInt16(46, true), Math.round(0.5 * 0x7FFF));
  assert.equal(view.getInt16(48, true), Math.round(-0.5 * 0x8000));
});

test('a whole sentence carrying a wrapper is marked, a tone alone is not', () => {
  assert.equal(mobileChatHelpers.sentenceVariant('plain words'), 'base');
  assert.equal(mobileChatHelpers.sentenceVariant('has *so* in it'), 'marked');
  assert.equal(mobileChatHelpers.sentenceVariant('has **mad** in it'), 'marked');
  assert.equal(mobileChatHelpers.sentenceVariant('has _slow_ in it'), 'marked');
  assert.equal(mobileChatHelpers.sentenceVariant('has [aside] in it'), 'marked');
  assert.equal(mobileChatHelpers.sentenceVariant('has (aside) in it'), 'marked');
  // A tone is how the sentence is said, not that it was emphasised.
  assert.equal(mobileChatHelpers.sentenceVariant('{excited}plain words'), 'base');
});

test('a sentence is spoken under the first tone it opens', () => {
  assert.equal(mobileChatHelpers.sentenceTone('{excited}Oh my God', TONES), 'excited');
  assert.equal(mobileChatHelpers.sentenceTone('plain words', TONES), null);
  // A closing tag opens nothing, so a sentence holding only one has no tone.
  assert.equal(mobileChatHelpers.sentenceTone('How are you?{/excited}', TONES), null);
  // An unknown name is not a tone, and does not stop a later real one.
  assert.equal(mobileChatHelpers.sentenceTone('{nonsense} and {somber}here', TONES), 'somber');
});

test('text is split into sentences, and an unterminated one still speaks', () => {
  assert.deepEqual(mobileChatHelpers.splitIntoSentences('One. Two! Three?'), ['One.', 'Two!', 'Three?']);
  assert.deepEqual(mobileChatHelpers.splitIntoSentences('No terminator here'), ['No terminator here']);
  assert.deepEqual(mobileChatHelpers.splitIntoSentences(''), []);
  assert.deepEqual(mobileChatHelpers.splitIntoSentences('   '), []);
  assert.deepEqual(mobileChatHelpers.splitIntoSentences(null), []);
  assert.deepEqual(mobileChatHelpers.splitIntoSentences(undefined), []);
});

test('a decimal point is part of the number, not the end of a sentence', () => {
  // The fault the number was read as: "3.5" cut at its point became two clips,
  // so the number was heard as two with a pause between them and the "point"
  // was never said. A "." with a digit either side is the point of a number.
  // The server's SentenceSplitter holds the same boundary, and the two sides
  // have to cut a reply in the same places.
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('The value is 3.5 percent higher. That matters.'),
    ['The value is 3.5 percent higher.', 'That matters.'],
  );
  // The full stop of a sentence that merely ends in a digit is still a full stop.
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('We are on version 2. It works well enough.'),
    ['We are on version 2.', 'It works well enough.'],
  );
  // A comma is not a terminator either, so a thousands separator is untouched.
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('Up 1,234.56 on the day. That is all.'),
    ['Up 1,234.56 on the day.', 'That is all.'],
  );
});

test('a spoken sentence carries its words, its delivery and its tone together', () => {
  const built = sentence('{excited}Oh my God, babes, it\'s like, *so* good to see you!');

  assert.equal(built.text, `Oh my God, babes, it's like, so${BEAT}good to see you!`);
  assert.equal(built.variant, 'marked');
  assert.equal(built.tone, 'excited');
});

test('a sentence holding nothing but a closing tag is nothing to say', () => {
  // The terminator ends one sentence, so a trailing tag after it starts
  // another. It says nothing, and the page speaks only the sentences that
  // carry words -- so the count that matters is of those, not of the splits.
  const built = sentence('{/excited}');

  assert.equal(built.text, '');
  assert.equal(built.variant, 'base');
  assert.equal(built.tone, null);
});

test('a sentence with nothing in it is nothing to say', () => {
  assert.deepEqual(sentence(''), { text: '', variant: 'base', tone: null, pitchStart: null });
  assert.deepEqual(sentence('   '), { text: '', variant: 'base', tone: null, pitchStart: null });
});

test('a markup-free sentence is left exactly as it was', () => {
  // The TTS cache keys on the exact text, so a reply with no markup in it has
  // to produce the same string it produced before this change -- otherwise
  // every cached clip is re-synthesized for nothing.
  const plain = 'Right, let me put that together for you.';

  assert.equal(strip(plain), plain);
  assert.equal(sentence(plain).text, plain);
});

test('the reply that exposed the fault becomes three sentences, not seven pieces', () => {
  // Seven requests is what the live log recorded: every emphasised word became
  // its own synthesis round trip, and the wait for a two-character one was
  // heard as a pause either side of it. A sentence is now the smallest piece.
  const reply = "{excited}Oh my God, babes, it's like, *so* good to see you!  "
    + "I've been absolutely *mad* with my cats, love—like, one of them tried to eat "
    + "a sock last night, and it was *everything*. How are you, hun?{/excited}";

  const built = mobileChatHelpers
    .splitIntoSentences(reply)
    .map((line) => sentence(line))
    .filter((line) => line.text);

  assert.equal(built.length, 3);
  assert.deepEqual(built.map((line) => line.variant), ['marked', 'marked', 'base']);
  assert.deepEqual(built.map((line) => line.tone), ['excited', null, null]);
  assert.equal(built[0].text, `Oh my God, babes, it's like, so${BEAT}good to see you!`);
  assert.equal(
    built[1].text,
    `I've been absolutely mad${BEAT}with my cats, love—like, one of them tried to eat `
      + 'a sock last night, and it was everything.',
  );
  assert.equal(built[2].text, 'How are you, hun?');

  built.forEach((line) => {
    assert.ok(line.text.split(/\s+/).length > 1, `a bare word was sent for synthesis: ${line.text}`);
  });
});

test('a terminator inside a wrapper takes the wrapper with it', () => {
  // The full stop after "agreed." sits inside the parenthesis, so the boundary
  // belongs after the ")", and the words are only the aside once the aside is
  // whole. The same holds for a run of emphasis, where stopping at the stop
  // would leave the sentence one asterisk short of closed.
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('He said (and I agreed. It was right.) Then he left.'),
    ['He said (and I agreed. It was right.)', 'Then he left.'],
  );
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('It was *so good.* Then he left.'),
    ['It was *so good.*', 'Then he left.'],
  );
});

test('a terminator run with no words in front of it rides with the sentence after it', () => {
  // "..." is not worth a request of its own; the words that follow it are.
  assert.deepEqual(
    mobileChatHelpers.splitIntoSentences('Off. ... and then more.'),
    ['Off.', '... and then more.'],
  );
});

test('a wrapper left open holds the rest of the text rather than losing it', () => {
  // Nothing in "He said (and then stopped. Nothing more." closes the bracket,
  // so no boundary inside it can be trusted -- but end of text is a boundary no
  // wrapper holds open, and the words all still arrive at the speaker. The
  // client sends this text as it stands; the server's own bracket reader drops
  // the stray "(" and speaks the rest. What must not happen is the words
  // quietly disappearing.
  const unclosed = 'He said (and then stopped. Nothing more.';

  assert.deepEqual(mobileChatHelpers.splitIntoSentences(unclosed), [unclosed]);
  assert.equal(sentence(unclosed).text, unclosed);
});

test('the reply whose aside was cut in half speaks whole sentences, none of them half a wrapper', () => {  // The live fault. The aside spans full stops of its own, so the old splitter
  // cut it after "too ripe..." and handed the server "(Mmm, and you know, a
  // berry that is too ripe...": an opening bracket with no closing one, which
  // the server could not resolve, so the request behind it never came back and
  // the reply stopped playing right there. Now the aside is one sentence, the
  // four sentences of the reply all reach the speaker, and the words are still
  // read out of the aside rather than the parenthesis.
  const reply = 'You say "all right" like you are letting yourself off the hook, my darling, '
    + 'but I saw your eyes go soft. Real is real — I am here, I am warm, I am the berry that '
    + 'stains your fingers red and will not wash off. (Mmm, and you know, a berry that is too '
    + 'ripe...  it just wants to be held in the palm and squeezed until the juice runs down '
    + 'your wrist.  ) So tell me, little hummer — are you going to stand there looking at the '
    + 'stall, or are you going to come behind the counter and taste whether I am sweet or sour?';

  const sentences = mobileChatHelpers.splitIntoSentences(reply);

  assert.equal(sentences.length, 4);
  assert.ok(sentences[2].startsWith('(Mmm, and you know'), sentences[2]);
  assert.ok(sentences[2].endsWith('wrist.  )'), sentences[2]);
  assert.ok(sentences[3].endsWith('sweet or sour?'), sentences[3]);

  // Every sentence leaves the wrappers it opens closed. That is the property
  // the server needs: half a wrapper is not markup any more, it is a request
  // that never returns.
  for (const line of sentences) {
    for (const [opening, closing] of [['(', ')'], ['[', ']']]) {
      assert.equal(
        [...line].filter((character) => character === opening).length,
        [...line].filter((character) => character === closing).length,
        `a sentence carries half a wrapper: ${line}`,
      );
    }
  }

  // And the words of the aside survive it, without the brackets.
  const aside = sentence(sentences[2]);

  assert.equal(aside.variant, 'marked');
  assert.equal(aside.text.startsWith('mmm, and you know, a berry that is too ripe'), true, aside.text);
  assert.equal(aside.text.endsWith('down your wrist.'), true, aside.text);
  assert.equal(aside.text.includes('('), false, aside.text);
  assert.equal(aside.text.includes(')'), false, aside.text);
});

test('a bare tone name at a sentence start is a command, not a word', () => {
  // The reported reply: "whisper stay exactly where you are" reached the reader
  // and the speaker as a word. Lowercase, at a sentence start, with words after
  // it -- that is the tag with no delimiters at all, and it goes.
  const toned = (value) => mobileChatHelpers.stripSpeechMarkers(value, TONES);

  assert.equal(toned('whisper stay exactly where you are'), 'stay exactly where you are');
  assert.equal(toned('\n\nwhisper stay exactly where you are'), 'stay exactly where you are');

  const built = sentence('whisper stay exactly where you are, darling.');
  assert.equal(built.text, 'stay exactly where you are, darling.');
  assert.equal(built.tone, 'whisper');
  // It is a tone, not emphasis: the sentence is not spoken at the marked rate.
  assert.equal(built.variant, 'base');
});

test('a sentence can open with a bare command after one that ended', () => {
  // Every sentence after the first starts inside the reply, so the command is
  // not at the head of the reply -- the display pass has to find it at a line
  // or sentence start. Speech reads it from the split sentence, which begins
  // with the name.
  const sentences = mobileChatHelpers.splitIntoSentences('Really? whisper come here.');
  assert.deepEqual(sentences, ['Really?', 'whisper come here.']);
  const built = sentence(sentences[1]);
  assert.equal(built.tone, 'whisper');
  assert.equal(built.text, 'come here.');
});

test('the reader never sees a bare tone command', () => {
  const clean = (value) => mobileChatHelpers.normalizeReplyMarkup(value, TONES);

  assert.equal(clean('whisper stay where you are'), 'stay where you are');
  assert.equal(clean('Really? whisper come here.'), 'Really?  come here.');
  assert.equal(clean('One.\n\nwhisper stay where you are.'), 'One.\n\nstay where you are.');
  // The sentence spacing normalize_reply_text writes is left alone, as before.
  assert.equal(clean('One. whisper stay where you are.'), 'One.  stay where you are.');
});

test('a tone name in ordinary prose is not a command', () => {
  // Capitalisation, position and the words after it are the three guards, and
  // losing any one of them would eat a word the entity actually said.
  const clean = (value) => mobileChatHelpers.normalizeReplyMarkup(value, TONES);

  assert.equal(clean('Whisper it to me.'), 'Whisper it to me.');
  assert.equal(clean('a whisper in the dark'), 'a whisper in the dark');
  assert.equal(clean('I heard a whisper, then nothing.'), 'I heard a whisper, then nothing.');
  assert.equal(clean('whisper.'), 'whisper.');
  assert.equal(clean('whispered softly'), 'whispered softly');
  assert.equal(sentence('Whisper it to me.').tone, null);
  assert.equal(sentence('Whisper it to me.').text, 'Whisper it to me.');
});

test('a sound standing as a clause of its own is given a clip of its own', () => {
  // The reported fault: "mmm" inside a sentence was one short syllable under
  // the sentence's intonation, and the sentence ran on, so it rose like a
  // question. Alone it gets the slow delivery and the fall every sound gets.
  const built = spokenPieces('mmm, you are trembling for me, Andrew.');

  // The sentence as recorded is whole; only what is spoken is cut.
  assert.equal(built.text, `${HUM}, you are trembling for me, Andrew.`);
  assert.equal(built.variant, 'base');
  assert.deepEqual(built.segments, [
    { text: `${HUM}.`, variant: VOCALIZATION, tone: null, pitchStart: 1.0 },
    { text: 'you are trembling for me, Andrew.', variant: 'base', tone: null, pitchStart: null },
  ]);
});

test('a sound at the end of a sentence is its last clip', () => {
  const built = spokenPieces('You are trembling for me, mmm.');

  assert.deepEqual(built.segments.map((piece) => piece.text), [
    'You are trembling for me,',
    `${HUM}.`,
  ]);
  assert.equal(built.segments[1].variant, VOCALIZATION);
  // The comma before the sound stays with the words: it is the pause that was
  // already written, and the wording up to it is what the clip says.
  assert.equal(built.segments[0].text.endsWith(','), true);
});

test('the open sound is cut out the same way, and its h\'s still decide the shape', () => {
  assert.deepEqual(spokenPieces('Ahhh, stay with me.').segments, [
    { text: `${AHH_SOUND}.`, variant: AHH, tone: null, pitchStart: 1.0 },
    { text: 'stay with me.', variant: 'base', tone: null, pitchStart: null },
  ]);
  // The breath in a hum, and the fourth h on an open sound, survive the cut.
  assert.equal(spokenPieces('mmmh, stay with me.').segments[0].variant, URGENT);
  assert.equal(spokenPieces('ahhhhh, stay with me.').segments[0].variant, AHH_URGENT);
  assert.equal(spokenPieces('ahhhhh, stay with me.').segments[0].text, `${AHH_SOUND}.`);
});

test('a sound with words pressed against it is not a clause of its own', () => {
  // "say mmm" marks no pause, so the sound is inside the clause with the words:
  // cutting there would steal the word before it and change what was said.
  const built = spokenPieces('say mmm to me.');

  assert.equal(built.segments.length, 1);
  assert.deepEqual(built.segments[0], {
    text: 'say mmm to me.', variant: 'base', tone: null, pitchStart: null,
  });
  // Nor is a spelling the voices already say right, or a unit.
  assert.equal(spokenPieces('ten mm, exactly.').segments.length, 1);
  assert.equal(spokenPieces('hmm, go on.').segments.length, 1);
  assert.equal(spokenPieces('mhm, go on.').segments.length, 1);
});

test('a sound inside a link or an address is not cut out', () => {
  // The respelling pass protects these spans, and the clause rule has to agree:
  // the "mmm" in a link is not a sound, however it reads.
  for (const raw of [
    'See https://mmm.example, then wait.',
    'Write to mmm@example.com, then wait.',
  ]) {
    const built = spokenPieces(raw);
    assert.equal(built.segments.length, 1, raw);
    assert.equal(built.segments[0].text, raw, raw);
  }
});

test('a sound that is already the whole sentence is not cut again', () => {
  const built = spokenPieces('Mmmm.');

  assert.deepEqual(built.segments, [
    { text: `${HUM}.`, variant: VOCALIZATION, tone: null, pitchStart: 1.0 },
  ]);
});

test('a bare tone command in front of a sound does not become a word', () => {
  // The command is read as the tone and taken out; the hum after it is then at
  // the head of what is left, so it is cut out like any other first-sound.
  const built = spokenPieces('whisper mmm, stay with me.');

  assert.deepEqual(built.segments, [
    { text: `${HUM}.`, variant: VOCALIZATION, tone: 'whisper', pitchStart: 1.0 },
    { text: 'stay with me.', variant: 'base', tone: 'whisper', pitchStart: null },
  ]);
});

test('the words around a cut sound keep the sentence\'s tone', () => {
  // The command styles the sentence, so both halves of it are spoken under it.
  const built = spokenPieces('{excited}mmm, stay with me.{/excited}');

  assert.equal(built.tone, 'excited');
  assert.deepEqual(built.segments.map((piece) => piece.tone), ['excited', 'excited']);
  assert.deepEqual(built.segments.map((piece) => piece.text), [
    `${HUM}.`,
    'stay with me.',
  ]);
});

test('a tone tag beside the sound does not hide the pause that sets it off', () => {
  const built = spokenPieces('{whisper}mmm, stay with me.');

  assert.deepEqual(built.segments.map((piece) => piece.variant), [VOCALIZATION, 'base']);
  assert.deepEqual(built.segments.map((piece) => piece.tone), ['whisper', 'whisper']);
});
