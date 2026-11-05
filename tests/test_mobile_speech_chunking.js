const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

const CAP = mobileChatHelpers.SPEECH_CHUNK_MAX_CHARS;

function chunks(text, cap) {
  return mobileChatHelpers.splitSpeechIntoChunks(text, cap === undefined ? CAP : cap);
}

function words(text) {
  return String(text).trim().split(/\s+/).filter(Boolean);
}

test('the client cap leaves room under the server limit', () => {
  // Past the server's TTS_CACHE_MAX_TEXT_CHARS the clip is not cached at all,
  // so every play of it is a fresh synthesis. The gap between the two is for
  // the client and server normalisers disagreeing about how long the same
  // words are -- without it a chunk the client measures at the cap arrives
  // over it and is never cached.
  const serverCap = 240;

  assert.equal(CAP, 200);
  assert.ok(CAP < serverCap, 'the client must stay under the server limit');
});

test('text short enough to be one request stays one request', () => {
  assert.deepEqual(chunks('Right, let me put that together.'), ['Right, let me put that together.']);
  assert.deepEqual(chunks('One.'), ['One.']);
});

test('a run of short sentences is packed up to the cap', () => {
  const text = 'One. Two. Three. Four.';

  assert.deepEqual(chunks(text, 10), ['One. Two.', 'Three.', 'Four.']);
});

test('a chunk boundary is a sentence boundary wherever one fits', () => {
  const text = 'The first sentence is here. The second sentence follows. The third one ends it.';

  const parts = chunks(text, 40);

  assert.deepEqual(parts, [
    'The first sentence is here.',
    'The second sentence follows.',
    'The third one ends it.',
  ]);
});

test('a chunk boundary is never the point of a number', () => {
  // Cutting "3.5" at its point sends the engine "3" and "5" as two requests,
  // and the clip boundary between them is heard as a full stop -- with the
  // "point" never spoken. Whatever the cap, the number stays in one piece.
  const text = 'The value is 3.5 percent higher on the day, which is worth a note. That matters.';

  for (const cap of [12, 20, 40, 200]) {
    const parts = chunks(text, cap);

    // A chunk ending on "3." is a boundary sitting at the number's point, which
    // is the whole of the fault. The words themselves are checked below.
    for (const part of parts) {
      assert.ok(!part.endsWith('3.'), `"${part}" was cut at the point of a number`);
    }
    assert.ok(parts.join(' ').includes('3.5'), `a cap of ${cap} lost the number`);
  }
});

test('a sentence longer than the cap is split on words, never inside one', () => {
  const sentence = 'Extraordinarily complicated considerations notwithstanding, '
    + 'the notwithstanding considerations remain extraordinarily complicated.';

  const parts = chunks(sentence, 40);

  assert.ok(parts.length > 1, 'an over-long sentence has to be cut somewhere');
  for (const part of parts) {
    assert.ok(part.length <= 40, `"${part}" is over the cap`);
    // Every piece starts and ends on a whole word: a cut mid-word is audible.
    assert.equal(part, part.trim());
  }
  assert.deepEqual(parts.flatMap(words), words(sentence));
});

test('no chunk is over the cap', () => {
  const text = 'A short one. '.repeat(20) + 'And a very much longer sentence that has to be cut '
    + 'because it does not fit inside the limit on its own at all.';

  for (const cap of [12, 40, 200]) {
    for (const part of chunks(text, cap)) {
      assert.ok(part.length <= cap, `"${part}" is over a cap of ${cap}`);
    }
  }
});

test('a cap narrower than a single word still speaks the word', () => {
  // The cap cannot be honoured without cutting a word in half, and half a word
  // is worse than an over-long request: the words come back whole.
  const parts = chunks('Extraordinarily complicated.', 5);

  assert.deepEqual(parts, ['Extraordinarily', 'complicated.']);
  assert.deepEqual(parts.flatMap(words), ['Extraordinarily', 'complicated.']);
});

test('nothing is lost or duplicated on the way out', () => {
  const text = 'One two three. Four five. Six seven eight nine ten. Eleven.';

  for (const cap of [1, 11, 25, 200]) {
    const rejoined = chunks(text, cap).join(' ');

    assert.deepEqual(words(rejoined), words(text), `a cap of ${cap} changed the words`);
  }
});

test('text that is only whitespace is nothing to say', () => {
  assert.deepEqual(chunks(''), []);
  assert.deepEqual(chunks('   '), []);
  assert.deepEqual(chunks(null), []);
  assert.deepEqual(chunks(undefined), []);
});

test('a repeat of the same text chunks the same way', () => {
  // The chunks are cache keys on the server. An unstable split would make
  // Repeat a fresh synthesis of every piece rather than a hit.
  const text = 'One line here. And another line. Followed by a third one that carries on a while.';

  assert.deepEqual(chunks(text, 30), chunks(text, 30));
});
