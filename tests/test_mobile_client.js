const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

function createStorage() {
  const entries = new Map();

  return {
    getItem(key) {
      return entries.has(key) ? entries.get(key) : null;
    },
    setItem(key, value) {
      entries.set(key, String(value));
    },
    removeItem(key) {
      entries.delete(key);
    },
  };
}

test('MCP access token persists in local storage', () => {
  const storage = createStorage();

  assert.equal(mobileChatHelpers.readMcpAccessToken(storage), '');

  const token = mobileChatHelpers.storeMcpAccessToken(storage, '  jwt-token  ');

  assert.equal(token, 'jwt-token');
  assert.equal(
    storage.getItem(mobileChatHelpers.MCP_ACCESS_TOKEN_STORAGE_KEY),
    'jwt-token'
  );
  assert.equal(mobileChatHelpers.readMcpAccessToken(storage), 'jwt-token');
});

test('clearing the MCP access token removes the persisted token', () => {
  const storage = createStorage();

  mobileChatHelpers.storeMcpAccessToken(storage, 'jwt-token');
  const clearedToken = mobileChatHelpers.clearMcpAccessToken(storage);

  assert.equal(clearedToken, '');
  assert.equal(storage.getItem(mobileChatHelpers.MCP_ACCESS_TOKEN_STORAGE_KEY), null);
  assert.equal(mobileChatHelpers.readMcpAccessToken(storage), '');
});

test('chat payload keeps the screener request token and selected voice details', () => {
  const payload = mobileChatHelpers.buildChatRequestPayload({
    message: 'Run the McTrader stock screener for broker 2 and show me the top 15 candidates',
    mcpAccessToken: 'jwt-token',
    deferred: true,
    deliveryStyle: 'normal',
    modelName: 'zai-org/glm-4.7-flash',
    preferredLanguage: 'en-GB',
    textReportFormat: true,
    selectedVoice: {
      id: 'en-GB-AriaNeural',
      kind: 'voice',
      name: 'Aria',
      display_name: 'Aria',
      gender: 'female',
      provider: 'edge',
      locale: 'en-GB',
      source: 'server',
    },
  });

  assert.deepEqual(payload, {
    message: 'Run the McTrader stock screener for broker 2 and show me the top 15 candidates',
    mcp_token: 'jwt-token',
    deferred: true,
    delivery_style: 'normal',
    model_name: 'zai-org/glm-4.7-flash',
    preferred_language: 'en-GB',
    text_report_format: true,
    voice_mode: false,
    // No clientId was passed, so this is empty rather than minted here: the
    // server owns minting, and it echoes the id back on the response.
    client_id: '',
    selected_voice: {
      name: 'Aria',
      display_name: 'Aria',
      gender: 'female',
      provider: 'edge',
      lang: 'en-GB',
      culture: 'en-GB',
      source: 'server',
      character_id: 'en-GB-AriaNeural',
      country: '',
    },
  });
});

test('chat payload sends the selected model even when MCP access is absent', () => {
  const payload = mobileChatHelpers.buildChatRequestPayload({
    message: 'Run the McTrader stock screener for broker 2 and show me the top 15 candidates',
    mcpAccessToken: '',
    deliveryStyle: 'normal',
    modelName: 'zai-org/glm-4.7-flash',
    preferredLanguage: 'en-GB',
    textReportFormat: false,
    selectedVoice: null,
  });

  assert.equal(payload.model_name, 'zai-org/glm-4.7-flash');
  assert.equal(payload.mcp_token, '');
  assert.equal(payload.selected_voice, null);
});

test('normalizeReplyMarkup removes tone markup from reply text', () => {
  assert.equal(
    mobileChatHelpers.normalizeReplyMarkup('Hello {alluring}come closer{/alluring} now'),
    'Hello come closer now'
  );
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('{alluring}come closer'), 'come closer');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('come closer{/alluring}'), 'come closer');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('{sultry}hi{/sultry}'), 'hi');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('plain text'), 'plain text');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup(''), '');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup(null), '');
});

test('normalizeReplyMarkup unwraps emphasis and keeps the words', () => {
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('[never] do that'), 'never do that');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('(quietly) he left'), 'quietly he left');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('a **mad** and *quick* thing'), 'a mad and quick thing');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('say _slow_ now'), 'say slow now');
});

test('normalizeReplyMarkup drops a malformed tag whole', () => {
  // The reported failure: a bracket opener closed by a brace, which no
  // single-dialect rule claims.
  assert.equal(
    mobileChatHelpers.normalizeReplyMarkup('breath.\n[hiss]Speak his purpose,{/hiss] lest he takes more than a name.'),
    'breath.\nhiss Speak his purpose, lest he takes more than a name.'
  );
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('{tool_calls}'), '');
});

test('normalizeReplyMarkup leaves names and dict-shaped text alone', () => {
  // Underscores join the identifiers this app talks about, so the word-edge
  // guard is what keeps a name a name.
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('total_return_pct'), 'total_return_pct');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('avgvol_200d and code_name'), 'avgvol_200d and code_name');
  // Not a tag: the body is a key and a value, not a marker name.
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('Set a {x: 1} here'), 'Set a {x: 1} here');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('a * item one * another'), 'a * item one * another');
});

test('normalizeReplyMarkup drops a wrapper that names a known tone', () => {
  // The reported reply: "[whisper]I want this.[/whisper] Tell me." -- a tone
  // tag in the emphasis wrapper's clothes, whose name was read aloud.
  const tones = { whisper: {} };
  assert.equal(
    mobileChatHelpers.normalizeReplyMarkup('[whisper]I want this.[/whisper] Tell me.', tones),
    'I want this. Tell me.'
  );
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('a *whisper* b', tones), 'a b');
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('a _whisper_ b', tones), 'a b');
});

test('normalizeReplyMarkup keeps a wrapper holding words, tone word or not', () => {
  // The name alone decides nothing: only a known tone name is control markup.
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('[never] do that', { whisper: {} }), 'never do that');
  // One marker word in a wrapper is a token; more words than that are a span
  // this rule does not claim, so the wrapper stays as the model wrote it.
  assert.equal(
    mobileChatHelpers.normalizeReplyMarkup('[whisper softly] he said.', { whisper: {} }),
    '[whisper softly] he said.'
  );
  // A caller with no tone table still gets the wrapper's words.
  assert.equal(mobileChatHelpers.normalizeReplyMarkup('[whisper]I want this.[/whisper]'), 'whisper I want this.');
});

test('the country last used is stored and read back', () => {
  const storage = createStorage();

  assert.equal(mobileChatHelpers.readStoredCountry(storage), '');

  assert.equal(mobileChatHelpers.storeCountry(storage, '  United Kingdom  '), 'United Kingdom');
  assert.equal(mobileChatHelpers.readStoredCountry(storage), 'United Kingdom');
  assert.equal(
    storage.getItem(mobileChatHelpers.ENTITY_COUNTRY_STORAGE_KEY),
    'United Kingdom'
  );
});

test('choosing no country clears the stored one rather than storing a blank', () => {
  const storage = createStorage();
  mobileChatHelpers.storeCountry(storage, 'Japan');

  assert.equal(mobileChatHelpers.storeCountry(storage, ''), '');
  // The key is gone, not blank: a stored blank would read back as a country
  // named "" and could never be told apart from never having chosen.
  assert.equal(storage.getItem(mobileChatHelpers.ENTITY_COUNTRY_STORAGE_KEY), null);
  assert.equal(mobileChatHelpers.readStoredCountry(storage), '');
});

test('a restored country is dropped when the server no longer offers it', () => {
  // Left set, it would be a select showing no selection while characters load
  // for somewhere the picker cannot display.
  assert.equal(mobileChatHelpers.chooseRestoredCountry('Atlantis', ['Japan', 'Peru']), '');
  assert.equal(mobileChatHelpers.chooseRestoredCountry('Japan', ['Japan', 'Peru']), 'Japan');
  assert.equal(mobileChatHelpers.chooseRestoredCountry('', ['Japan']), '');
  assert.equal(mobileChatHelpers.chooseRestoredCountry('Japan', []), '');
  assert.equal(mobileChatHelpers.chooseRestoredCountry('Japan', null), '');
});

test('a restored country takes the spelling the server uses', () => {
  // The stored value is the user's history; the select has to be set to an
  // option that actually exists.
  assert.equal(
    mobileChatHelpers.chooseRestoredCountry('united kingdom', ['United Kingdom', 'Peru']),
    'United Kingdom'
  );
});

test('storage that refuses to be written does not break the country', () => {
  const hostile = {
    getItem() {
      throw new Error('site data is blocked');
    },
    setItem() {
      throw new Error('site data is blocked');
    },
    removeItem() {
      throw new Error('site data is blocked');
    },
  };

  assert.equal(mobileChatHelpers.readStoredCountry(hostile), '');
  assert.equal(mobileChatHelpers.storeCountry(hostile, 'Japan'), 'Japan');
  assert.equal(mobileChatHelpers.chooseRestoredCountry('Japan', ['Japan']), 'Japan');
});

test('the language last used is stored and read back', () => {
  const storage = createStorage();

  assert.equal(mobileChatHelpers.readStoredLanguage(storage), '');

  assert.equal(mobileChatHelpers.storeLanguage(storage, 'en-GB'), 'en-GB');
  assert.equal(mobileChatHelpers.readStoredLanguage(storage), 'en-GB');
  assert.equal(
    storage.getItem(mobileChatHelpers.ENTITY_LANGUAGE_STORAGE_KEY),
    'en-GB'
  );
});

test('a browser that blocks site data leaves the language readable and writable', () => {
  // The read at the top of loadStoredDeliveryStyle was the one storage access in
  // that function left unguarded, and it was the first. A blocked store threw on
  // the property access, the exception left the function, and because the call
  // sits at the top level the rest of the inline script never ran: no country
  // options, no character options, no picker, and nothing on screen to say why.
  // These two calls are what that line and persistSelectedLanguage do now.
  const hostile = {
    getItem() {
      throw new Error('site data is blocked');
    },
    setItem() {
      throw new Error('site data is blocked');
    },
    removeItem() {
      throw new Error('site data is blocked');
    },
  };

  assert.equal(mobileChatHelpers.readStoredLanguage(hostile), '');
  assert.equal(mobileChatHelpers.storeLanguage(hostile, 'en-GB'), 'en-GB');
});

test('no storage at all is not an error for the language', () => {
  // safeLocalStorage() returns null when the property access itself throws,
  // which is how these are reached in a browser that blocks site data.
  assert.equal(mobileChatHelpers.readStoredLanguage(null), '');
  assert.equal(mobileChatHelpers.storeLanguage(null, 'en-GB'), 'en-GB');
  assert.equal(mobileChatHelpers.readStoredLanguage(undefined), '');
});

test('a restart mid-utterance keeps the words said before it', () => {
  // Recognition drops out and starts again; the new session finalizes only the
  // rest of the sentence, and the words before the drop have to survive it or
  // the reply answers the tail of what was said.
  assert.equal(
    mobileChatHelpers.mergeDictatedText('put the kettle on', 'and bring the cups'),
    'put the kettle on and bring the cups'
  );
});

test('a session that repeats what was already said does not double it', () => {
  // The engine re-hears the words either side of the seam.
  assert.equal(
    mobileChatHelpers.mergeDictatedText('put the kettle on', 'kettle on and bring cups'),
    'put the kettle on and bring cups'
  );
  assert.equal(
    mobileChatHelpers.mergeDictatedText('put the kettle on', 'put the kettle on and bring cups'),
    'put the kettle on and bring cups'
  );
  assert.equal(
    mobileChatHelpers.mergeDictatedText('put the kettle on and bring cups', 'put the kettle on'),
    'put the kettle on and bring cups'
  );
});

test('only a whole word may be treated as the overlap', () => {
  // Cutting mid-word would join "wor" to "ld" and read as nonsense, so this is
  // two words rather than one.
  assert.equal(
    mobileChatHelpers.mergeDictatedText('hello wor', 'world'),
    'hello wor world'
  );
});

test('an empty side of the seam leaves the other alone', () => {
  assert.equal(mobileChatHelpers.mergeDictatedText('', 'bring the cups'), 'bring the cups');
  assert.equal(mobileChatHelpers.mergeDictatedText('put the kettle on', ''), 'put the kettle on');
  assert.equal(mobileChatHelpers.mergeDictatedText('   ', '  '), '');
  assert.equal(mobileChatHelpers.mergeDictatedText(null, undefined), '');
});

test('unrelated speech either side of the seam is kept in order', () => {
  assert.equal(
    mobileChatHelpers.mergeDictatedText('what did you say', 'about the shed'),
    'what did you say about the shed'
  );
});

test('a dropout sends nothing while the user owns the box', () => {
  // The reported fault: hands-free brings the microphone back after every reply,
  // and a dropout would auto-send whatever the user had typed -- clearing the
  // box after only a couple of characters, again and again.
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;
  const heard = { transcript: 'put the kettle on', autoSendEnabled: true, reason: 'pause' };

  assert.equal(autoSend({ ...heard, userOwnsInput: true }), false);
  assert.equal(autoSend({ ...heard, userOwnsInput: false }), true);
});

test('the engine ending its own session is a pause, not the end of the turn', () => {
  // The reported fault. Recognition runs on a server and drops its session at a
  // short silence, so the engine ends its own session mid-sentence -- at a comma.
  // Reading that as the user finishing sent half a sentence, and no setting could
  // prevent it because the send never waited for the pause to run out.
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;
  const heard = { transcript: 'I want to buy, um, some shares', userOwnsInput: false };

  // Nobody asked for this turn: reason is empty. The toggle has nothing to say.
  assert.equal(autoSend({ ...heard, autoSendEnabled: true, reason: '' }), false);
  assert.equal(autoSend({ ...heard, autoSendEnabled: false, reason: '' }), false);
  assert.equal(autoSend({ ...heard, autoSendEnabled: true }), false);
});

test('a dropout that heard nothing never becomes a turn', () => {
  // An empty transcript is the common case -- ambient noise, the engine opening
  // its session -- and it is where the box used to be read instead.
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;

  for (const transcript of ['', '   ', null, undefined]) {
    assert.equal(autoSend({ transcript, autoSendEnabled: true, userOwnsInput: false }), false);
  }
});

test('the auto-send toggle decides whether the pause ends the turn', () => {
  // The toggle's one remaining job: the silence pause ends the turn on the user's
  // behalf, so the user may veto it. Nothing else consults the toggle.
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;
  const heard = { transcript: 'bring the cups', userOwnsInput: false, reason: 'pause' };

  assert.equal(autoSend({ ...heard, autoSendEnabled: false }), false);
  assert.equal(autoSend({ ...heard, autoSendEnabled: true }), true);
});

test('the user stopping the microphone outranks the toggle', () => {
  // Tapping the microphone off with something heard is the user asking for the
  // turn, so it does not matter what the toggle says.
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;

  assert.equal(
    autoSend({ transcript: 'bring the cups', autoSendEnabled: false, reason: 'user' }),
    true
  );
  assert.equal(
    autoSend({ transcript: 'bring the cups', autoSendEnabled: true, reason: 'user' }),
    true
  );
});

test('a turn already being sent is never doubled by a dropout', () => {
  const autoSend = mobileChatHelpers.shouldAutoSendDictation;

  assert.equal(
    autoSend({ transcript: 'bring the cups', autoSendEnabled: true, reason: 'pause', sending: true }),
    false
  );
});

test('dictation does not write into a box the user owns', () => {
  const mayWrite = mobileChatHelpers.shouldDictateIntoInput;

  assert.equal(mayWrite({ recording: true, userOwnsInput: true }), false);
  assert.equal(mayWrite({ recording: true, userOwnsInput: false }), true);
  // Not listening at all: nothing to write.
  assert.equal(mayWrite({ recording: false, userOwnsInput: false }), false);
  assert.equal(mayWrite({}), false);
});

test('the silence pause waits while the user is still speaking', () => {
  // The reported fault: recognition runs on a server, so interim results arrive
  // in bursts, and the transcript sits unchanged for longer than the pause
  // while the user is mid-sentence. The turn ended after a few words.
  const waits = mobileChatHelpers.shouldWaitLongerForSilence;

  assert.equal(waits({ hearingSpeech: true, waitedMs: 0 }), true);
  assert.equal(waits({ hearingSpeech: true, waitedMs: 4000 }), true);
  // Nothing heard: the pause is real silence, so the turn ends as it always did.
  assert.equal(waits({ hearingSpeech: false, waitedMs: 0 }), false);
  assert.equal(waits({ hearingSpeech: false, waitedMs: 4000 }), false);
  assert.equal(waits({}), false);
});

test('a speech signal that never ends still releases the microphone', () => {
  // An engine that never reports the end of speech must not hold the turn open
  // for ever: the wait is capped, and the cap is what ends it.
  const waits = mobileChatHelpers.shouldWaitLongerForSilence;

  assert.equal(waits({ hearingSpeech: true, waitedMs: 5999 }), true);
  assert.equal(waits({ hearingSpeech: true, waitedMs: 6000 }), false);
  assert.equal(waits({ hearingSpeech: true, waitedMs: 20000 }), false);
});

test('a keystroke yields the microphone only while it is live', () => {
  const yields = mobileChatHelpers.shouldYieldMicToTyping;

  assert.equal(yields({ recording: true, userOwnsInput: false }), true);
  // Already the user's: a second keystroke has nothing left to yield.
  assert.equal(yields({ recording: true, userOwnsInput: true }), false);
  // Typing with the microphone off is just typing.
  assert.equal(yields({ recording: false, userOwnsInput: false }), false);
  assert.equal(yields({}), false);
});

test('the words a reply still owes the ears are read back in order', () => {
  // What the caller says after a queue stops short: the segments that were not
  // heard whole, in the order they were meant to be said.
  const recovery = mobileChatHelpers.speechRecoveryText;

  assert.equal(
    recovery({
      played: 1,
      reason: 'playback',
      unheard: [
        { text: 'the second thing', cause: 'playback', message: '', heardSome: false },
        { text: 'and the third', cause: 'not-attempted', message: '', heardSome: false },
      ],
    }),
    'the second thing and the third'
  );

  // Everything was heard, and a drain that never happened: nothing is owed.
  assert.equal(recovery({ played: 3, reason: null, unheard: [] }), '');
  assert.equal(recovery(null), '');
  assert.equal(recovery({}), '');
  // A segment with no words of its own -- a preface's fart -- adds nothing.
  assert.equal(
    recovery({ played: 1, reason: 'playback', unheard: [{ text: '  ', cause: 'dropped' }] }),
    ''
  );
});

test('a user interrupt is never repaired', () => {
  // The one thing standing between the repair and talking over someone who
  // pressed Stop. The segments below are genuinely unheard, and they still come
  // back empty: that silence was asked for.
  const recovery = mobileChatHelpers.speechRecoveryText;
  const interrupted = {
    played: 2,
    reason: 'user',
    unheard: [{ text: 'the last thing', cause: 'not-attempted', message: '', heardSome: false }],
  };

  assert.equal(recovery(interrupted), '');
  assert.equal(mobileChatHelpers.speechPartialFailure(interrupted), null);
});

test('a reply that lost a piece part way through is reported, not forgotten', () => {
  // The failure this was written for: the voice stopped mid-reply, the rest of
  // the answer was heard, and nothing anywhere said a piece had gone missing.
  const partial = mobileChatHelpers.speechPartialFailure;
  const stopped = {
    text: 'the middle of it',
    cause: 'interrupted',
    message: 'playback stopped after 7.1s of 22.3s',
    heardSome: true,
  };

  assert.equal(
    partial({ played: 4, reason: 'playback', unheard: [stopped] }),
    'part of the reply could not be spoken: playback stopped after 7.1s of 22.3s'
  );
  assert.equal(
    partial({ played: 1, reason: 'playback', unheard: [stopped, stopped] }),
    '2 parts of the reply could not be spoken: playback stopped after 7.1s of 22.3s'
  );
  // A segment that went unheard without failing -- skipped, or enqueued after
  // the loop had stopped -- is still a hole, and has no message to quote.
  assert.equal(
    partial({ played: 3, reason: 'synthesis', unheard: [{ text: 'x', cause: 'dropped' }] }),
    'part of the reply could not be spoken'
  );
  // Nothing played at all: that is the caller's fallback to make, not a report.
  assert.equal(partial({ played: 0, reason: 'synthesis', unheard: [stopped] }), null);
  assert.equal(partial({ played: 2, reason: null, unheard: [] }), null);
  assert.equal(partial(null), null);
});

test('an interrupted playback says where in the clip it stopped', () => {
  // A pause is not an ending, so the message has to place the interruption --
  // "it stopped" is what the user can already see for themselves.
  const interrupt = mobileChatHelpers.speechInterruptionError;

  const stopped = interrupt({ currentTime: 7.123, duration: 22.34 });
  assert.equal(stopped.message, 'playback stopped after 7.1s of 22.3s');
  assert.equal(stopped.name, 'SpeechInterruptedError');
  // A clip whose duration is not known yet still reports its position.
  assert.equal(interrupt({ currentTime: 3 }).message, 'playback stopped after 3.0s');
  assert.equal(
    interrupt({ currentTime: 1.5, duration: Infinity }).message,
    'playback stopped after 1.5s'
  );
  assert.equal(interrupt(null).message, 'playback stopped after 0.0s');
});

test('an interruption is told apart from every other kind of failure', () => {
  // The queue counts an interrupted segment as heard and still falls back for
  // anything else, so this name carries weight rather than being cosmetic.
  const isInterruption = mobileChatHelpers.isSpeechInterruption;

  assert.equal(
    isInterruption(mobileChatHelpers.speechInterruptionError({ currentTime: 2 })),
    true
  );
  assert.equal(isInterruption(new Error('Audio playback failed')), false);
  const abort = new Error('aborted');
  abort.name = 'AbortError';
  assert.equal(isInterruption(abort), false);
  assert.equal(isInterruption(null), false);
  assert.equal(isInterruption(undefined), false);
});

test('the remembered character outranks what the picker is showing', () => {
  // While a country's characters are still arriving the picker may be showing a
  // fallback of its own, and that fallback must not be read back as the choice:
  // it would replace the character the browser remembered.
  assert.equal(
    mobileChatHelpers.chooseRememberedVoice('United Kingdom:Sharon', 'browser:Google UK English Female'),
    'United Kingdom:Sharon'
  );
});

test('the picker is consulted only when nothing is remembered', () => {
  assert.equal(mobileChatHelpers.chooseRememberedVoice('', 'United Kingdom:Aled'), 'United Kingdom:Aled');
  assert.equal(mobileChatHelpers.chooseRememberedVoice(null, 'United Kingdom:Aled'), 'United Kingdom:Aled');
  assert.equal(mobileChatHelpers.chooseRememberedVoice('   ', 'United Kingdom:Aled'), 'United Kingdom:Aled');
  assert.equal(mobileChatHelpers.chooseRememberedVoice('', ''), '');
  assert.equal(mobileChatHelpers.chooseRememberedVoice(null, null), '');
});
