(function (root, factory) {
  const helpers = factory();

  if (typeof module === 'object' && module.exports) {
    module.exports = helpers;
  }

  if (root) {
    root.MobileChatHelpers = helpers;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  const MCP_ACCESS_TOKEN_STORAGE_KEY = 'entity-mcp-access-token';
  const ENTITY_CLIENT_ID_STORAGE_KEY = 'entity-client-id';
  const ENTITY_COUNTRY_STORAGE_KEY = 'entity-selected-country';
  // Mirrors languageStorageKey in templates/index.html. Kept in step by hand for
  // the same reason as the key above: the page owns the picker, this owns the
  // guarded read and write.
  const ENTITY_LANGUAGE_STORAGE_KEY = 'entity-selected-language';
  // Mirrors CLIENT_ID_PATTERN in utils/entity_store.py. Kept in step by hand:
  // this side only decides whether to send what it has, and the server mints a
  // replacement for anything that fails its own check, so a drift here costs a
  // forgotten client rather than a rejected request.
  const CLIENT_ID_PATTERN = /^[A-Za-z0-9_-]{8,64}$/;
  const TONE_TAG_PATTERN = /\{(\/?)[A-Za-z][A-Za-z0-9_-]*\}/g;
  // A tone tag written in an emphasis wrapper's clothes: "[whisper]",
  // "[/whisper]", "*whisper*". The prompt files teach the wrappers for
  // emphasis and the braces for tone, and a model asked to whisper something
  // sometimes names the delivery inside a wrapper instead. The name alone
  // cannot decide it -- "whisper" is also a word -- so only a name the caller
  // knows counts.
  const TONE_NAME_PATTERN = /^[a-z][a-z0-9_-]*$/;
  // Mirrors _MARKUP_TOKEN_PATTERN and _WORD_WRAPPER_PATTERN in utils/text.py.
  // Kept in step by hand, like TONE_TAG_PATTERN above and for the same reason:
  // the server sends a reply already cleaned, so this side is a backstop for a
  // page talking to a server that has not learned to clean one yet.
  //
  // One tag-shaped token: a delimiter wrapping one marker word, with an
  // optional "/" naming a closing tag. A brace on either end belongs to the
  // tone dialect and goes whole -- including the mixed form a model invents
  // when it crosses the dialects, "{/hiss]", which neither a brace rule nor a
  // bracket rule claims.
  const MARKUP_TOKEN_PATTERN = /([\[({])\s*(\/?)\s*([A-Za-z][A-Za-z0-9_-]*)\s*([\])}])/g;
  // Asterisks and underscores need a word-edge guard that brackets do not,
  // because "_" also joins the identifiers this app talks about: unwrapping a
  // span inside one would rename it, and "total_return_pct" would come back as
  // "totalreturnpct". Lookbehind is already relied on by SPEECH_EMPHASIS_PATTERN
  // below, so the guards can read exactly as the server's do.
  const WORD_WRAPPER_PATTERN = /(?:^|(?<=\s))(\*{1,2}|_)(?![\s*_])([^*_\n]+?)(?<![\s*_])\1(?=\s|[.,;:!?)\]]|$)/g;
  // A tone name written bare where a sentence starts, with words after it:
  // "whisper stay exactly where you are". Mirrors _BARE_TONE_COMMAND_PATTERN in
  // utils/text.py. Only lowercase counts, only at a line or sentence start, and
  // only when words follow: prose capitalises its sentences, so "Whisper it to
  // me" stays a word, and so does a whisper someone actually heard.
  const BARE_TONE_COMMAND_PATTERN = /(^[ \t]*|[.!?…]["'”’)\]]?[ \t]*|\n[ \t]*)([a-z][a-z0-9_-]*)([ \t]+)(?=\S)/g;
  // What counts as a word for the joins above. Python's str.isalnum is Unicode,
  // so this is too: an accented name either side of a tag must still get the
  // space it needs.
  const WORD_CHARACTER_PATTERN = /[\p{L}\p{N}]/u;
  // Names a playback the browser cut short, so the queue can tell a clip
  // the ears already met from one they never heard at all.
  const SPEECH_INTERRUPTION_ERROR_NAME = 'SpeechInterruptedError';
  // How far back a resumed clip is wound before it plays again. An interruption
  // lands wherever it lands, which is normally inside a word, so resuming at the
  // exact position would begin on that word's tail and lose its opening. Four
  // tenths of a second is under a spoken word, so the seam costs at most one
  // word said twice -- and the word the pause landed inside is heard in full.
  const SPEECH_RESUME_OVERLAP_SECONDS = 0.4;
  // The longest a silence pause keeps waiting because the engine still reports
  // the user speaking. The wait has to end: an engine that never sends its
  // end-of-speech event would otherwise hold the microphone open for ever, and
  // a turn that is never sent is worse than one sent a moment early.
  const SILENCE_SPEECH_WAIT_LIMIT_MS = 6000;

  function leadingBareTone(text, knownTones) {
    // A lowercase known tone name at the head of the text, with words after it.
    // It is the command dialect with no delimiters at all, and the head is
    // where speech reads it: the sentence it opens is the one it styles.
    // Sentence-start detection for display is stripBareToneCommand's job.
    const match = /^\s*([a-z][a-z0-9_-]*)(?=\s+\S)/.exec(String(text || ''));
    if (!match) {
      return null;
    }
    const name = match[1];
    const known = knownTones || {};
    if (!Object.prototype.hasOwnProperty.call(known, name)) {
      return null;
    }
    return { name, end: match[0].length };
  }

  function stripBareToneCommands(text, knownTones) {
    // Every bare command in a text, wherever a sentence starts. The delimiter
    // characters that mark the boundary are kept, so the spacing the reply
    // already had survives; only the name goes. At a line start the command
    // takes its indentation and the space after it with it, so the words that
    // follow begin the line; after punctuation both spaces stay, which is the
    // two-space sentence break the reply's own formatting writes.
    const known = knownTones || {};
    return String(text || '').replace(
      BARE_TONE_COMMAND_PATTERN,
      (match, boundary, word, spaces) => {
        if (!Object.prototype.hasOwnProperty.call(known, word)) {
          return match;
        }
        if (boundary.trim() === '') {
          return boundary.includes('\n') ? '\n' : '';
        }
        return boundary + spaces;
      },
    );
  }

  function normalizeReplyMarkup(text, knownTones) {
    // The reply as a reader should see it: words, without the delivery markup.
    // Mirrors normalize_reply_markup in utils/text.py, which is the authority --
    // the server cleans `reply` before sending it and keeps the raw text beside
    // it as `reply_markup`, so the page only cleans what a server old enough to
    // send neither field left behind. That is the same guard the brace-only
    // stripToneTags used to be, widened to the dialects a model actually mixes.
    let out = String(text || '');
    if (!out) {
      return '';
    }
    const known = knownTones || {};

    out = out.replace(MARKUP_TOKEN_PATTERN, (match, opener, slash, word, closer, offset, source) => {
      const before = offset ? source[offset - 1] : '';
      const after = source[offset + match.length] || '';
      // Tone tags wrap a delivery and no words, so they go whole -- and so does
      // a closing tag, whose delimiters need not agree: "{/hiss]" is one
      // mistyped tag, not two words. A wrapper around a known tone name is the
      // same tag in the other dialect, and it names no word either.
      if (
        opener === '{'
        || closer === '}'
        || slash
        || Object.prototype.hasOwnProperty.call(known, word.toLowerCase())
      ) {
        // Two words that only the markup was separating still need one.
        return WORD_CHARACTER_PATTERN.test(before) && WORD_CHARACTER_PATTERN.test(after) ? ' ' : '';
      }
      // What is left wraps real words: the characters are control markup a
      // reader should never see, and the words are what the entity said.
      const lead = WORD_CHARACTER_PATTERN.test(before) ? ' ' : '';
      const trail = WORD_CHARACTER_PATTERN.test(after) ? ' ' : '';
      return `${lead}${word}${trail}`;
    });
    out = out.replace(WORD_WRAPPER_PATTERN, (match, marker, inner) => {
      const name = inner.trim().replace(/^\//, '').toLowerCase();
      // "*whisper*" is the tone tag in a star's clothes, and names no word.
      return Object.prototype.hasOwnProperty.call(known, name) ? '' : inner.trim();
    });
    // A bare name at a sentence start -- "whisper stay exactly where you are"
    // -- is the tag with no delimiters at all. It names a delivery, so it goes
    // the way the other dialects go; the words around it stay.
    out = stripBareToneCommands(out, known);
    // Only the spacing the removal itself orphaned, so the two spaces
    // normalize_reply_text puts after a full stop are left as they are.
    out = out.replace(/(?<![.!?])[ \t]{2,}/g, ' ');
    out = out.replace(/[ \t]+([,.;:!?])/g, '$1');
    out = out.replace(/\n{3,}/g, '\n\n');
    return out.trim();
  }

  function normalizeString(value) {
    return String(value || '').trim();
  }

  function storageGet(storage, key) {
    try {
      if (storage && typeof storage.getItem === 'function') {
        return storage.getItem(key);
      }
    } catch (_error) {
      return null;
    }
    return null;
  }

  function storageSet(storage, key, value) {
    try {
      if (storage && typeof storage.setItem === 'function') {
        storage.setItem(key, value);
      }
    } catch (_error) {
      return;
    }
  }

  function storageRemove(storage, key) {
    try {
      if (storage && typeof storage.removeItem === 'function') {
        storage.removeItem(key);
      }
    } catch (_error) {
      return;
    }
  }

  function readMcpAccessToken(storage) {
    return normalizeString(storageGet(storage, MCP_ACCESS_TOKEN_STORAGE_KEY));
  }

  function storeMcpAccessToken(storage, token) {
    const normalizedToken = normalizeString(token);
    if (normalizedToken) {
      storageSet(storage, MCP_ACCESS_TOKEN_STORAGE_KEY, normalizedToken);
      return normalizedToken;
    }

    storageRemove(storage, MCP_ACCESS_TOKEN_STORAGE_KEY);
    return '';
  }

  function clearMcpAccessToken(storage) {
    storageRemove(storage, MCP_ACCESS_TOKEN_STORAGE_KEY);
    return '';
  }

  function normalizeClientId(value) {
    // Returns '' rather than throwing, so the caller sends nothing and the
    // server mints one. Rejecting here would be the client refusing to talk to
    // a server that was perfectly willing to answer.
    const candidate = normalizeString(value);
    return CLIENT_ID_PATTERN.test(candidate) ? candidate : '';
  }

  function storeClientId(storage, value) {
    // Persisting on adoption is what stops two identities existing at once. When
    // storage is empty, both the startup probe and the first turn can ask the
    // server to mint one, and whichever answer arrived second would leave the
    // entity talking on an id that nothing else is writing to.
    const normalized = normalizeClientId(value);
    if (normalized) {
      storageSet(storage, ENTITY_CLIENT_ID_STORAGE_KEY, normalized);
    }
    return normalized;
  }

  function getOrCreateClientId(storage, mint) {
    // The durable identity, and the only one the page keeps. It names the
    // client to the server and is what the conversation is stored under: an
    // address is shared by every computer behind one home router, while this
    // id belongs to this browser alone.
    const existing = normalizeClientId(storageGet(storage, ENTITY_CLIENT_ID_STORAGE_KEY));
    if (existing) {
      return existing;
    }

    return storeClientId(storage, typeof mint === 'function' ? mint() : '');
  }

  function readStoredCountry(storage) {
    return normalizeString(storageGet(storage, ENTITY_COUNTRY_STORAGE_KEY));
  }

  function readStoredLanguage(storage) {
    // The page read this one straight off `window.localStorage` while every
    // storage read beside it was guarded -- and it was the first, at the top
    // level, so a browser that blocks site data threw there and took the rest of
    // the inline script with it: no country list, no character picker, and no
    // error to explain either. Routed through storageGet so the missing storage
    // and the throwing getItem are both already handled, exactly as the country
    // read above is.
    return normalizeString(storageGet(storage, ENTITY_LANGUAGE_STORAGE_KEY));
  }

  function storeLanguage(storage, value) {
    const normalized = normalizeString(value);
    storageSet(storage, ENTITY_LANGUAGE_STORAGE_KEY, normalized);
    return normalized;
  }

  function storeCountry(storage, value) {
    // An empty country is a real choice ("None"), not a missing one, so it
    // removes the key rather than writing a blank that would read back as a
    // country named "".
    const normalized = normalizeString(value);
    if (normalized) {
      storageSet(storage, ENTITY_COUNTRY_STORAGE_KEY, normalized);
    } else {
      storageRemove(storage, ENTITY_COUNTRY_STORAGE_KEY);
    }
    return normalized;
  }

  function chooseRestoredCountry(stored, available) {
    // Returns the server's own spelling of the stored country, or '' when the
    // server no longer offers it. Falling back to '' rather than to the stored
    // string matters: a country left set but absent from the list is a select
    // showing no selection while characters load for somewhere the picker
    // cannot display.
    const wanted = normalizeString(stored);
    if (!wanted) {
      return '';
    }

    const match = (Array.isArray(available) ? available : []).find(
      (country) => normalizeString(country).toLowerCase() === wanted.toLowerCase()
    );
    return match ? normalizeString(match) : '';
  }

  function buildEntityStreamUrl(clientId, sinceSeq) {
    const id = normalizeClientId(clientId);
    const params = [];
    if (id) {
      params.push(`client_id=${encodeURIComponent(id)}`);
    }
    const since = Math.floor(Number(sinceSeq));
    if (Number.isFinite(since) && since > 0) {
      params.push(`since=${since}`);
    }
    return `/api/entity/stream${params.length ? `?${params.join('&')}` : ''}`;
  }

  function buildOutboxRequestUrl(clientId) {
    const id = normalizeClientId(clientId);
    return `/api/entity/outbox${id ? `?client_id=${encodeURIComponent(id)}` : ''}`;
  }

  function buildEntityStateUrl(clientId) {
    // The one endpoint that answers "may the entity speak, and as whom?" without
    // a turn, so a page can learn it at load instead of waiting to be spoken to.
    const id = normalizeClientId(clientId);
    return `/api/entity/state${id ? `?client_id=${encodeURIComponent(id)}` : ''}`;
  }

  function readOutboxMessages(payload) {
    // One shape for a message, whether it arrived over SSE or the REST fallback,
    // so the render path never has to know which one delivered it.
    const body = payload || {};
    const list = Array.isArray(body.messages) ? body.messages : [body];
    const messages = [];
    for (let index = 0; index < list.length; index += 1) {
      const entry = list[index] || {};
      const text = normalizeString(entry.text);
      const deliveryId = normalizeString(entry.delivery_id);
      if (!text || !deliveryId) {
        continue;
      }
      messages.push({
        deliveryId,
        text,
        // The unprompted line as the voice reads it, markers and all. The server
        // sends it only when cleaning changed something, and a message from a
        // server that sends no such field is spoken from `text`: it is already
        // what a reader sees, so the voice loses an emphasis rather than gaining
        // a bracket.
        textMarkup: normalizeString(entry.text_markup) || text,
        seq: Math.floor(Number(entry.seq)) || 0,
        kind: normalizeString(entry.kind),
        trigger: normalizeString(entry.trigger),
        characterSlug: normalizeString(entry.character_slug),
        deliveryStyle: normalizeString(entry.delivery_style),
      });
    }
    return messages;
  }

  function hasSeenDeliveryId(seen, deliveryId) {
    const wanted = normalizeString(deliveryId);
    if (!wanted) {
      return false;
    }
    if (seen && typeof seen.has === 'function') {
      return seen.has(wanted);
    }
    if (Array.isArray(seen)) {
      return seen.indexOf(wanted) !== -1;
    }
    return false;
  }

  function shouldDeliverProactive(context) {
    // A proactive line can arrive twice three different ways: the SSE frame and
    // the poll fallback can both carry it, a reconnect replays anything still
    // unacknowledged because delivery is at-least-once by design, and two open
    // tabs share one client id. The delivery id is the only stable key, so it is
    // the only thing worth remembering -- a text comparison would drop a
    // deliberately repeated line and keep a coincidental one.
    const state = context || {};
    if (!state.enabled) {
      return false;
    }
    if (!normalizeString(state.deliveryId) || !normalizeString(state.text)) {
      return false;
    }
    return !hasSeenDeliveryId(state.seenDeliveryIds, state.deliveryId);
  }

  function proactiveDeliveryPlan(context) {
    // What an unprompted line should do about the speaker, in one decision with
    // three outcomes:
    //
    //   'speak' -- say it out loud now.
    //   'show'  -- show the bubble, say nothing. Either there is no voice to
    //              make room for (replies are not spoken), or the microphone is
    //              hearing a person and would dictate the entity's own words.
    //   'hold'  -- keep the whole delivery until the speaker is free. A turn in
    //              flight, or a reply still being read out, owns the speaker:
    //              calling speak() stops whatever is playing, so a line arriving
    //              now would cut the reply the user asked for. Showing it early
    //              would also tag it "spoke up" while it is silent, and put it
    //              above a reply bubble that is still being written in place.
    //
    // A hidden tab is deliberately not a reason to hold or stay quiet.
    // Hands-free listening with the screen off is exactly the case where an
    // unprompted line should still be heard, and that is the point of the
    // feature.
    //
    // The turn checks come before the microphone ones because a turn owns the
    // speaker whatever the microphone is doing; hands-free keeps recording
    // between turns, and that is ambient listening rather than a person
    // mid-sentence. Every other recording is somebody dictating.
    const state = context || {};
    if (!state.enabled) {
      return 'show';
    }
    if (state.sending || state.speaking) {
      return 'hold';
    }
    if (!state.recording) {
      return 'speak';
    }
    return Boolean(state.handsFree) && !state.hearingSpeech ? 'speak' : 'show';
  }

  function shouldVoiceProactive(context) {
    return proactiveDeliveryPlan(context) === 'speak';
  }

  function shouldHoldProactive(context) {
    return proactiveDeliveryPlan(context) === 'hold';
  }

  function shouldPollOutbox(context) {
    // The poll is a fallback for a stream that is not up, not a second delivery
    // path running alongside one. It also stays out of the way of a turn in
    // flight, where waitForChatJob already owns the socket and its own faster
    // interval -- two polls of the same session is how a reply renders twice.
    const state = context || {};
    if (!state.enabled) {
      return false;
    }
    if (state.streamConnected || state.sending) {
      return false;
    }
    // A hidden tab is not being read. The outbox is durable, so waiting costs
    // nothing that the next visible poll does not recover.
    return !state.hidden;
  }

  function proactiveToggleView(payload) {
    // What the "chatterbox" box shows, from the server's answer. The
    // server owns this switch -- whether the entity may speak up is a fact
    // about its silence policy, not a browser preference -- so the view is
    // derived from its reply rather than remembered here. That is what makes a
    // reload, a second tab, or a master switch turned off in .env show up as the
    // box going unticked, instead of a tick over a feature that is not running.
    //
    // Null when the reply says nothing about it, which is every turn response
    // from a page cached before the box existed: a control must not be moved by
    // an answer that was never about it.
    //
    // Naming the key is what makes a reply "about" the switch; only a real
    // boolean ticks the box. A reply that carries the key with something else in
    // it -- a string from a proxy that re-encoded the JSON, a null -- is a reply
    // about the switch that never granted it, so the box reads as unticked
    // rather than keeping a tick the server did not confirm. The two mistakes
    // are not equally bad: a wrong tick makes the entity speak when nobody
    // asked, where a wrong untick is only silence, which is where the box rests
    // anyway.
    const body = payload && typeof payload === 'object' ? payload : {};
    const mentionsEnabled = 'proactive_enabled' in body;
    const mentionsAvailable = 'proactive_available' in body;
    if (!mentionsEnabled && !mentionsAvailable) {
      return null;
    }
    // Availability is the stricter test, because "not reported" and "reported as
    // something unusable" both mean the same thing here: only an explicit false
    // disables the box, so a response carrying just the enabled flag cannot
    // leave the control dead for the rest of the session, and neither can one
    // whose availability arrived mangled. A wrongly-enabled box grants nothing
    // anyway -- the master switch in .env outranks it.
    const available =
      typeof body.proactive_available === 'boolean' ? body.proactive_available : true;
    return {
      checked: body.proactive_enabled === true,
      disabled: !available,
      title: available
        ? ''
        : 'Switched off for the whole app: ENTITY_PROACTIVE_ENABLED is false in .env.',
    };
  }

  function nextOutboxPollDelayMs(attempt, baseMs) {
    const base = Math.max(500, Math.floor(Number(baseMs)) || 2000);
    // Capped at 30s: long enough not to hammer a server that is down, short
    // enough that recovery is not something the user has to trigger by tapping.
    const steps = Math.min(Math.max(0, Math.floor(Number(attempt)) || 0), 5);
    return Math.min(base * Math.pow(2, steps), 30000);
  }


  function resolveCharacterId(selectedVoice) {
    // Browser voices carry no curated persona, so they must not claim a
    // character id -- the server would have nothing to resolve it to.
    const voice = selectedVoice || {};
    if (normalizeString(voice.kind) === 'browser' || normalizeString(voice.source) === 'browser') {
      return '';
    }
    return normalizeString(voice.character_id || voice.id);
  }

  function buildSelectedVoicePayload(selectedVoice) {
    if (!selectedVoice || typeof selectedVoice !== 'object') {
      return null;
    }

    return {
      name: normalizeString(selectedVoice.name),
      display_name: normalizeString(selectedVoice.display_name || selectedVoice.name),
      gender: normalizeString(selectedVoice.gender),
      provider: normalizeString(selectedVoice.provider || selectedVoice.source),
      lang: normalizeString(selectedVoice.lang || selectedVoice.culture || selectedVoice.locale),
      culture: normalizeString(selectedVoice.culture || selectedVoice.lang || selectedVoice.locale),
      source: normalizeString(selectedVoice.source),
      character_id: resolveCharacterId(selectedVoice),
      // The country the voice was chosen under. A server character carries it in
      // its id; a browser voice has no id of ours, so this is how its persona
      // still comes from the right country.
      country: normalizeString(selectedVoice.country),
    };
  }

  function buildChatRequestPayload(options) {
    const settings = options || {};
    const token = normalizeString(settings.mcpAccessToken);

    return {
      message: normalizeString(settings.message),
      mcp_token: token,
      deferred: Boolean(settings.deferred),
      delivery_style: normalizeString(settings.deliveryStyle),
      model_name: normalizeString(settings.modelName),
      preferred_language: normalizeString(settings.preferredLanguage),
      text_report_format: Boolean(settings.textReportFormat),
      // Marks a turn that came in through the microphone so the server can ask
      // the model for short, speakable replies instead of written prose.
      voice_mode: Boolean(settings.voiceMode),
      // Empty when absent or malformed, in which case the server mints one and
      // echoes it back rather than refusing the turn.
      client_id: normalizeClientId(settings.clientId),
      selected_voice: buildSelectedVoicePayload(settings.selectedVoice),
    };
  }

  function shouldResumeListening(context) {
    // Single source of truth for "put the microphone back on". Hands-free is
    // the only thing that enables it, and every reason to stay quiet wins.
    const state = context || {};
    if (!state.handsFree || !state.recognitionAvailable) {
      return false;
    }

    return !(state.sending || state.speaking || state.userStopped || state.fatalError);
  }

  function shouldPauseAfterNoSpeech(context) {
    // Whether a session that ended with nothing heard waits for a tap. The
    // engine ends a session of its own accord after every pause, so parking
    // there turned each silence into a manual restart -- which is the exact
    // thing hands-free exists to remove. Hands-free therefore re-arms instead,
    // and everything else parks as before.
    const state = context || {};
    return !state.handsFree;
  }

  function shouldDictateIntoInput(context) {
    // Whether a recognition result may be written into the message box. The
    // microphone is not the box's only writer: the user types into the same
    // box, and hands-free brings the microphone back after every reply, so a
    // recognition result that overwrites what is being typed does so for ever.
    // A keystroke hands the box to the user until dictation is asked for again.
    const state = context || {};
    if (state.userOwnsInput) {
      return false;
    }
    return Boolean(state.recording);
  }

  function shouldAutoSendDictation(context) {
    // Whether the end of a recognition session may send what was heard as a
    // turn. The transcript has to have come from the microphone: an engine
    // dropout that heard nothing must never become a turn, or whatever happens
    // to be sitting in the box -- the user's half-typed sentence -- is sent as
    // one, and cleared out from under their hands.
    //
    // `reason` says who ended the session, and an empty one means nobody did:
    // recognition runs on a server and drops the session at a short silence, so
    // the engine ends its own session in the middle of a sentence. Treating that
    // as the end of the turn is what sent half a sentence whenever the user drew
    // breath at a comma. Only a stop somebody asked for may send.
    const state = context || {};
    if (state.sending || state.userOwnsInput || !normalizeString(state.transcript)) {
      return false;
    }
    // The user stopping the microphone is a request for this turn, so the
    // auto-send toggle has no say in it.
    if (state.reason === 'user') {
      return true;
    }
    // The silence pause ends the turn on the user's behalf rather than at their
    // request, which is what leaves the toggle something to decide.
    return state.reason === 'pause' && state.autoSendEnabled;
  }

  function shouldWaitLongerForSilence(context) {
    // Whether an armed silence pause waits again instead of ending the turn.
    // Recognition runs on a server, so its results arrive in bursts and the
    // transcript sits unchanged for longer than the pause while the user is
    // still mid-sentence. A timer watching the transcript alone therefore ended
    // the turn after a few words. Speech the engine is still hearing is the
    // honest sign the user has not finished, and it outranks a quiet
    // transcript -- but only up to the wait limit, which is what stops an
    // engine that never reports the end of speech from holding the microphone.
    const state = context || {};
    if (!state.hearingSpeech) {
      return false;
    }
    const waitedMs = Math.max(0, Number(state.waitedMs) || 0);
    return waitedMs < SILENCE_SPEECH_WAIT_LIMIT_MS;
  }

  function shouldYieldMicToTyping(context) {
    // Whether a keystroke takes the microphone off the air. Only a keystroke
    // while the microphone is live is a conflict: typing with the microphone
    // off is just typing, and must not disturb the listening state.
    const state = context || {};
    return Boolean(state.recording) && !state.userOwnsInput;
  }

  function shouldRetrySpeechRequest(status, attempt) {
    // Whether asking for this clip again could give a different answer. One
    // attempt is all there is: a retry that failed says the server is not
    // coming back this breath, and a queue that kept asking would hold the
    // reply behind a request that is not going to be answered.
    const asked = Math.floor(Number(attempt)) || 0;
    if (asked > 0) {
      return false;
    }
    // No status at all is a fetch that never happened -- a dropped connection,
    // a DNS failure -- which is exactly the case asking again fixes.
    if (status === null || status === undefined) {
      return true;
    }
    const code = Math.floor(Number(status)) || 0;
    if (code >= 500) {
      // The server said it could not, not that it would not.
      return true;
    }
    // Out of time, and out of patience: both are transient by definition, and
    // the second is the one the server asks to be tried again.
    return code === 408 || code === 429;
  }

  function mergeDictatedText(earlier, later) {
    // One utterance can span two recognition sessions: the engine drops out
    // mid-sentence and the page starts it again, and the words already spoken
    // belong to the sentence the new session is finishing. The engine usually
    // re-hears the words either side of that seam, so an overlap is counted
    // once -- dropping it loses the start of what the user said, keeping it
    // whole repeats the last thing they said.
    const before = normalizeString(earlier);
    const after = normalizeString(later);
    if (!before) {
      return after;
    }
    if (!after) {
      return before;
    }
    // A session that repeats its predecessor from the beginning, or extends it,
    // is the same speech heard twice; the longer of the two is the transcript.
    if (after.startsWith(before)) {
      return after;
    }
    if (before.startsWith(after)) {
      return before;
    }

    const limit = Math.min(before.length, after.length);
    for (let size = limit; size > 0; size -= 1) {
      if (before.slice(-size).toLowerCase() !== after.slice(0, size).toLowerCase()) {
        continue;
      }
      // Only a whole-word overlap may be dropped, or the seam cuts a word in
      // half and the transcript reads as nonsense.
      const startsWord = size === before.length || /\s/.test(before[before.length - size - 1]);
      const endsWord = size === after.length || /\s/.test(after[size]);
      if (startsWord && endsWord) {
        const rest = after.slice(size).trim();
        return rest ? `${before} ${rest}` : before;
      }
    }

    return `${before} ${after}`;
  }

  function readNewSentences(entries, consumed) {
    // Every poll carries the whole append-only sentence list, so the client has
    // to remember how far it has already spoken. A list shorter than the cursor
    // means the server started over; re-reading it is safer than going silent.
    const list = Array.isArray(entries) ? entries : [];
    const start = Math.min(Math.max(0, Math.floor(Number(consumed)) || 0), list.length);
    const texts = [];
    for (let index = start; index < list.length; index += 1) {
      const text = String((list[index] || {}).text || '').trim();
      if (text) {
        texts.push(text);
      }
    }
    return { texts, consumed: list.length };
  }

  function readNewSegments(entries, consumed) {
    // The final payload carries the whole segment list at once, unlike the
    // append-only sentence stream, so the cursor exists only to stop a repeated
    // delivery from drawing the same bubbles twice.
    //
    // A single segment is not a split: the caller wants the unchanged
    // one-bubble behaviour, so it reports no segments rather than a list of one.
    const list = Array.isArray(entries) ? entries : [];
    const start = Math.min(Math.max(0, Math.floor(Number(consumed)) || 0), list.length);
    const texts = [];
    for (let index = start; index < list.length; index += 1) {
      const text = String(list[index] || '').trim();
      if (text) {
        texts.push(text);
      }
    }
    return { texts: texts.length > 1 ? texts : [], consumed: list.length };
  }

  function streamedPrefixLength(reply, spoken) {
    // How much of `reply` the already-spoken `spoken` covers, or null when the
    // two disagree. Mirrors `_streamed_prefix_length` in agent.py: whitespace is
    // not compared, because the server's splitter strips each sentence, and
    // everything else must match exactly -- anything else means the text the
    // user heard is not the text being returned.
    let replyIndex = 0;
    for (const character of spoken) {
      if (/\s/.test(character)) {
        continue;
      }
      while (replyIndex < reply.length && /\s/.test(reply[replyIndex])) {
        replyIndex += 1;
      }
      if (replyIndex >= reply.length || reply[replyIndex] !== character) {
        return null;
      }
      replyIndex += 1;
    }
    return replyIndex;
  }

  function splitReplyAfterStream(reply, spoken) {
    // What is left of a reply this page has already started saying, and whether
    // what it said matched the reply at all.
    //
    // The server publishes the same split, but from what it *sent*: it counts a
    // sentence as spoken the moment it publishes it, and the reply's last one is
    // released as the turn ends -- after the final poll that could have carried
    // it. So the server can report nothing left to say while the tail has never
    // been heard. This side's own record of what it enqueued is the one account
    // that cannot be wrong, so the caller takes the longer of the two.
    const full = String(reply == null ? '' : reply);
    const said = String(spoken == null ? '' : spoken);
    const prefixLength = streamedPrefixLength(full, said);
    if (prefixLength === null) {
      return { remaining: full, replaced: Boolean(said.trim()) };
    }
    return { remaining: full.slice(prefixLength).replace(/^\s+/, ''), replaced: false };
  }

  function waitForMediaEnd(media, options) {
    // Resolves when a media element stops playing, however it stops. The spooky
    // preface waits on this so a fart is heard in full before the reply's voice
    // begins, rather than the reply talking over its tail.
    //
    // It always settles. The cap is a guard against broken playback, not a guess
    // at the clip's length: a clip that plays normally fires `ended` long first,
    // and a clip whose `ended` never fires must not park the reply for ever.
    const settings = options || {};
    const capMs = Math.max(1, Math.floor(Number(settings.capMs)) || 8000);
    return new Promise((resolve) => {
      if (!media || typeof media.addEventListener !== 'function') {
        resolve();
        return;
      }
      let timer = 0;
      function finish() {
        media.removeEventListener('ended', finish);
        media.removeEventListener('error', finish);
        media.removeEventListener('abort', finish);
        clearTimeout(timer);
        resolve();
      }
      media.addEventListener('ended', finish);
      media.addEventListener('error', finish);
      media.addEventListener('abort', finish);
      timer = setTimeout(finish, capMs);
    });
  }

  function buildPrefacePlan(selection) {
    // The spooky preface as one ordered list of steps to play. The message's
    // @F marker says where a fart belongs in the line, so the plan reports the
    // order and the caller needs no branching of its own.
    const chosen = selection || {};
    const text = normalizeString(chosen.text);
    const before = normalizeString(chosen.beforeText);
    const after = normalizeString(chosen.afterText);
    const fart = { type: 'fart', text: '' };
    const speech = (value) => ({ type: 'speech', text: value });

    if (!chosen.playFart) {
      return text ? [speech(text)] : [];
    }

    if (chosen.fartPosition === 'start') {
      return text ? [fart, speech(text)] : [fart];
    }

    if (chosen.fartPosition === 'middle') {
      const steps = [];
      if (before) {
        steps.push(speech(before));
      }
      steps.push(fart);
      if (after) {
        steps.push(speech(after));
      }
      return steps;
    }

    // 'end', or anything unrecognised: the line first, the fart after it.
    return text ? [speech(text), fart] : [fart];
  }

  function sleep(ms) {
    return new Promise((resolve) => {
      setTimeout(resolve, Math.max(0, Number(ms) || 0));
    });
  }

  function clipFinished(media) {
    // Whether the element played this clip to its end. The end of a clip arrives
    // as a pause as well as an ended -- and the pause first -- so a listener
    // watching for pauses sees a clip at its end stop in exactly the way a clip
    // that was interrupted stops.
    //
    // `ended` is the browser's own answer and the first thing asked, but which
    // of the two the browser has set by the time it delivers the pause is the
    // browser's business, and reading the other one wrong is how a reply comes
    // to be said twice. So the position is asked as well: a clip sitting on its
    // last sample has finished wherever the element has got to in saying so.
    // The tolerance is under a frame of speech, so a clip cut in its final
    // twentieth of a second is read as finished and nothing audible is lost.
    if (!media) {
      return false;
    }
    if (media.ended === true) {
      return true;
    }
    const total = Number(media.duration);
    const position = Number(media.currentTime);
    if (!Number.isFinite(total) || total <= 0) {
      return false;
    }
    return total - position <= 0.05;
  }

  function canResumeClip(media, clipUrl) {
    // The element still holds the clip we were asked to play again, and it is
    // part-way through it: playing it continues the sentence where the
    // interruption left off rather than repeating its opening. A clip that
    // ended, or that the element no longer holds, is not resumable -- and an
    // element that was torn down has nothing left to resume from.
    if (!media || !clipUrl) {
      return false;
    }
    if (String(media.src || '') !== String(clipUrl)) {
      return false;
    }
    return Number(media.currentTime) > 0 && !clipFinished(media);
  }

  function resumePosition(media, overlapSeconds) {
    // Where a resumed clip starts again: the point the interruption stopped it
    // at, wound back by the overlap, so the word the pause landed inside is said
    // from its beginning rather than from its middle. Clamped to the clip,
    // because a position past the end seeks nowhere.
    //
    // An element with no position to offer resumes from the top. That is the one
    // answer that loses nothing: it may say a word again, which is the price this
    // whole policy is willing to pay, and it never skips one.
    if (!media) {
      return 0;
    }
    const overlap = Math.max(0, Number(overlapSeconds) || 0);
    const stopped = Number(media.currentTime);
    if (!Number.isFinite(stopped) || stopped <= 0) {
      return 0;
    }
    const wound = Math.max(0, stopped - overlap);
    const total = Number(media.duration);
    if (!Number.isFinite(total) || total <= 0) {
      return wound;
    }
    return Math.min(wound, Math.max(0, total - 0.1));
  }

  function resumeSeekPosition(resumeFrom, media, clipUrl) {
    // Where a retry seeks to, or null when it must not seek at all.
    //
    // A seek is a resume, and a resume needs the element to still be holding the
    // clip part-way through. A clip it no longer holds is handed to it fresh,
    // and a fresh clip has no position to carry on from: seeking one to where
    // the last one stopped starts it past its own ending, so the segment is over
    // before it is heard and the words in it are never said. Playing it from the
    // top instead may repeat a word, which this policy already accepts, and
    // loses none.
    if (resumeFrom === null || resumeFrom === undefined) {
      return null;
    }
    if (!canResumeClip(media, clipUrl)) {
      return null;
    }
    const position = Number(resumeFrom);
    return Number.isFinite(position) ? position : null;
  }

  function createSpeechQueue(options) {
    // Plays styled speech segments back-to-back while the next ones are still
    // being synthesized, so the voice never waits on the network mid-reply.
    // Playback stays strictly sequential: one segment at a time, in order.
    const settings = options || {};
    const synthesize = settings.synthesize;
    const play = settings.play;
    if (typeof synthesize !== 'function' || typeof play !== 'function') {
      throw new Error('createSpeechQueue requires synthesize and play functions');
    }

    const lookahead = Math.max(1, Math.floor(Number(settings.lookahead)) || 2);
    // Something that already owns the speaker and must finish first -- the
    // spooky preface, normally. Given as a promise or a function returning one.
    const gate = settings.gate || null;
    // A clip that failed to play gets one more attempt: whatever stopped the
    // element has usually let go by the next breath, and a second attempt after
    // an interruption is a resume from just before the interruption rather than
    // a replay -- the player reads the attempt number and seeks accordingly.
    // This is the beat that lets the audio session settle before the retry.
    const retryDelayMs = settings.retryDelayMs === undefined
      ? 500
      : Math.max(0, Number(settings.retryDelayMs) || 0);
    const playAttempts = 2;
    const startedAt = Date.now();
    const entries = [];
    let nextToSynthesize = 0;
    let nextToPlay = 0;
    let playedCount = 0;
    let running = false;
    // Why the loop is no longer running, or null while it still is: 'user' when
    // the caller interrupted, otherwise the stage that tore the turn down. A
    // single boolean could not tell "the user asked for silence" from "the queue
    // gave up part way", and that difference is the whole question the caller
    // asks of a drain -- whether the words that went missing should be said.
    let stopReason = null;
    let gateWaited = false;
    let drainResult = null;
    let drainWaiters = [];
    let releaseStopped;
    // Racing the gate against this is what stops a gate that never settles
    // from parking the loop for ever and leaving drain() unresolved.
    const stoppedSignal = new Promise((resolve) => {
      releaseStopped = resolve;
    });

    function isAbort(error) {
      return Boolean(error) && error.name === 'AbortError';
    }

    function isStopped() {
      return stopReason !== null;
    }

    function makeEntry(segment) {
      // `cause` is why a segment was not heard whole, and `heardSome` is whether
      // any of it reached the ears before it stopped -- a clip cut off part way
      // is repairable from its own words, where one that never started is not.
      // Nothing else keeps that fact, which is why a reply could go quiet half
      // way through and leave no trace of what had gone missing.
      return {
        segment,
        synthesisStarted: false,
        audio: null,
        audioPromise: null,
        error: null,
        cause: null,
        heardSome: false,
        complete: false,
      };
    }

    function segmentText(segment) {
      return normalizeString(segment && segment.text);
    }

    function startSynthesis(entry) {
      if (entry.synthesisStarted) {
        return;
      }
      entry.synthesisStarted = true;
      entry.audioPromise = Promise.resolve()
        .then(() => synthesize(entry.segment))
        .then(
          (audio) => {
            entry.audio = audio;
            return audio;
          },
          (error) => {
            entry.error = error;
            throw error;
          },
        );
      // The worker loop awaits this promise; this branch only stops the
      // rejection from surfacing as an unhandled promise rejection.
      entry.audioPromise.catch(() => {});
    }

    function settleDrain() {
      if (drainResult) {
        return drainResult;
      }

      // Anything the loop stopped before reaching has no cause of its own, and
      // the stop is the reason. Naming it here is what turns silence into a
      // repair rather than an absence.
      entries.forEach((entry) => {
        if (!entry.complete && !entry.cause) {
          entry.cause = 'not-attempted';
        }
      });

      // An abort is the caller tearing the turn down on purpose, not a failure
      // to recover from, so it is neither reported nor fallen back on.
      const errors = entries
        .filter((entry) => entry.error && !isAbort(entry.error))
        .map((entry) => entry.error);
      drainResult = {
        played: playedCount,
        total: entries.length,
        unheard: entries
          .filter((entry) => !entry.complete)
          .map((entry) => ({
            text: segmentText(entry.segment),
            cause: entry.cause,
            message: entry.error && entry.error.message ? entry.error.message : '',
            heardSome: Boolean(entry.heardSome),
          })),
        errors,
        reason: stopReason,
        durationMs: Date.now() - startedAt,
        // Only a failure before anything played is worth falling back for --
        // once the user has heard part of the reply, the rest is better than none.
        firstError: !isStopped() && playedCount === 0 && errors.length ? errors[0] : null,
      };

      const waiters = drainWaiters;
      drainWaiters = [];
      waiters.forEach((resolve) => resolve(drainResult));
      return drainResult;
    }

    function pumpSynthesis() {
      // Keeps `lookahead` segments in flight ahead of the one playing. Safe to
      // call at any time, which matters because streamed sentences arrive one
      // at a time rather than all at once.
      if (isStopped()) {
        return;
      }
      const horizon = Math.min(nextToPlay + lookahead, entries.length);
      for (let index = nextToSynthesize; index < horizon; index += 1) {
        startSynthesis(entries[index]);
      }
      nextToSynthesize = Math.max(nextToSynthesize, horizon);
    }

    async function awaitGate() {
      // Waits once, for whoever held the speaker before this queue existed.
      // Synthesis above is already in flight, so waiting costs no audio: the
      // first segment is ready the moment the gate opens.
      if (gateWaited || !gate) {
        return true;
      }
      gateWaited = true;
      try {
        await Promise.race([
          Promise.resolve(typeof gate === 'function' ? gate() : gate),
          stoppedSignal,
        ]);
      } catch (_error) {
        // A gate that fails is an open gate: a broken preface must not mute
        // the reply.
      }
      return !isStopped();
    }

    async function run() {
      while (!isStopped() && nextToPlay < entries.length) {
        pumpSynthesis();

        if (!(await awaitGate())) {
          return;
        }

        const entry = entries[nextToPlay];
        let audio;
        try {
          audio = await entry.audioPromise;
        } catch (error) {
          if (isStopped() || isAbort(error)) {
            stop('synthesis');
            return;
          }
          // Skip the segment and carry on: losing one sentence beats losing
          // the rest of the answer. `startSynthesis` has already recorded the
          // error on the entry; what it could not know is that the segment is
          // now lost rather than merely late.
          entry.cause = 'synthesis';
          nextToPlay += 1;
          continue;
        }

        if (isStopped()) {
          return;
        }

        let playError = null;
        // Read across the attempts, not off the last one: a retry that fails
        // says nothing about the audio the attempt before it played.
        let clipHeard = false;
        for (let attempt = 0; attempt < playAttempts; attempt += 1) {
          if (attempt > 0) {
            await sleep(retryDelayMs);
            if (isStopped()) {
              return;
            }
          }
          try {
            await play(audio, entry.segment, attempt);
            playError = null;
            break;
          } catch (error) {
            if (isStopped() || isAbort(error)) {
              stop('playback');
              return;
            }
            playError = error;
            if (error.heard === true || isSpeechInterruption(error)) {
              clipHeard = true;
            }
            // Every described failure gets its second attempt. A clip that was
            // heard part-way cannot be replayed from its opening any more: the
            // player resumes it from just before it stopped, so the retry can
            // only say a word the ear has already met, never the sentence it
            // would otherwise lose. That trade -- a repeated word against a
            // dropped one -- is the whole point of the resume.
          }
        }

        if (!playError) {
          playedCount += 1;
          entry.complete = true;
        } else {
          entry.error = playError;
          entry.heardSome = clipHeard;
          entry.cause = clipHeard ? 'interrupted' : 'playback';
          if (clipHeard) {
            // Cut off or broken part-way, not missed: whatever played before it
            // reached the ears. Counting it keeps `firstError` -- and the
            // whole-reply fallback that only a completely unheard reply earns
            // -- for a reply the user heard nothing of. It is still listed as
            // unheard: after both attempts failed, its tail never arrived.
            playedCount += 1;
          }
        }

        if (isStopped()) {
          return;
        }
        nextToPlay += 1;
      }
    }

    function stop(reason) {
      // An abort means the caller tore the turn down; there is nothing left to
      // play, so the queue is finished rather than merely interrupted. The
      // first reason wins, because an interrupt is recorded before the abort it
      // causes reaches this loop: the user's silence must never be rewritten as
      // a failure of ours, or the caller would say the reply again over the top
      // of the quiet they asked for.
      if (stopReason === null) {
        stopReason = reason;
      }
      // Releases a loop that is still waiting on the gate.
      releaseStopped();
    }

    function kick() {
      if (running || isStopped()) {
        return;
      }
      running = true;
      run()
        .catch((error) => {
          // The loop fell over rather than winding down. Whatever the cause,
          // the turn is over and every segment still owed is owed.
          stop('playback');
          entries.forEach((entry) => {
            if (!entry.error) {
              entry.error = error;
            }
          });
        })
        .then(() => {
          running = false;
          // A late enqueue may have arrived while the loop was unwinding.
          if (!isStopped() && nextToPlay < entries.length) {
            kick();
            return;
          }
          // Always settle: a queue that bailed out early must not leave a
          // caller waiting on drain() forever.
          settleDrain();
        });
    }

    return {
      enqueue(segment) {
        const entry = makeEntry(segment);
        entries.push(entry);
        if (isStopped()) {
          // The loop will not run again, so this segment is lost -- and a lost
          // segment that leaves no record is one the caller cannot put right.
          entry.cause = 'dropped';
        } else {
          pumpSynthesis();
          kick();
        }
        // Streaming arrives in waves, so a drained queue can take more work --
        // and a report already handed out no longer covers the queue.
        drainResult = null;
      },
      drain() {
        if (drainResult) {
          return Promise.resolve(drainResult);
        }
        // A stopped queue will never settle itself, so waiting on it would hang
        // for ever. This is also what keeps a segment enqueued after the stop
        // from turning a silent drop into a wait that never ends.
        if (isStopped() || (nextToPlay >= entries.length && !running)) {
          return Promise.resolve(settleDrain());
        }
        return new Promise((resolve) => {
          drainWaiters.push(resolve);
        });
      },
      clear() {
        // The user's interrupt, and the one reason a hole in the reply is not a
        // hole: they asked for the silence.
        stop('user');
        settleDrain();
      },
      get pending() {
        return Math.max(0, entries.length - nextToPlay);
      },
      get cancelled() {
        return isStopped();
      },
    };
  }

  function chooseRememberedVoice(stored, shown) {
    // What the picker shows is not what the browser remembers. While the
    // character's own country is still arriving the picker may be showing a
    // fallback it picked itself, and reading that back as the choice loses the
    // remembered character for good -- so the memory outranks the display, and
    // the display is only consulted when there is nothing remembered.
    return normalizeString(stored) || normalizeString(shown);
  }

  function speechRecoveryText(result, wholeReply) {
    // What to say to put a drained reply right, or '' when there is nothing to
    // put right.
    const outcome = result || {};
    if (outcome.reason === 'user') {
      // The one clause standing between the repair and talking over someone who
      // pressed Stop. They asked for the silence, and saying the reply
      // afterwards would talk over the very thing they asked for.
      return '';
    }
    if (!(Number(outcome.played) > 0)) {
      // Nothing was heard at all, so the whole reply is owed -- and it is said
      // from the reply itself rather than from the queue's segments. The segment
      // text has had its tone markup stripped, and that markup is what chose its
      // delivery.
      return normalizeString(wholeReply);
    }
    // A hole part way through has only the segments to repair from: the words
    // that were not heard whole, in the order they were meant to be said.
    const unheard = Array.isArray(outcome.unheard) ? outcome.unheard : [];
    return unheard
      .map((entry) => normalizeString(entry && entry.text))
      .filter((text) => Boolean(text))
      .join(' ');
  }

  function speechInterruptionError(media) {
    // A pause on an element that still holds its source is not the end of the
    // clip. The browser pauses it when the audio session moves -- the
    // microphone opening for hands-free listening is the usual reason here --
    // and where it stopped is read off the element so the message names the
    // point rather than only the fact.
    const played = Number(media && media.currentTime) || 0;
    const total = Number(media && media.duration);
    const stopped = Number.isFinite(total) && total > 0
      ? `${played.toFixed(1)}s of ${total.toFixed(1)}s`
      : `${played.toFixed(1)}s`;
    const error = new Error(`playback stopped after ${stopped}`);
    error.name = SPEECH_INTERRUPTION_ERROR_NAME;
    return error;
  }

  function isSpeechInterruption(error) {
    return Boolean(error) && error.name === SPEECH_INTERRUPTION_ERROR_NAME;
  }

  function speechPartialFailure(result) {
    // A drain that played something and still left segments unheard has a hole
    // in the middle of the reply. Only a failure before the first segment played
    // was ever acted on, so a reply that went quiet half way through reached the
    // user as an unremarkable success.
    //
    // Counted from `unheard` rather than from the failures, because a segment
    // can go unheard with no error against it at all -- skipped, or enqueued
    // after the loop had already stopped.
    const outcome = result || {};
    if (outcome.reason === 'user') {
      return null;
    }
    const unheard = Array.isArray(outcome.unheard) ? outcome.unheard : [];
    if (!unheard.length || !(Number(outcome.played) > 0)) {
      return null;
    }
    const first = unheard[0] || {};
    const reason = first.message ? `: ${first.message}` : '';
    const what = unheard.length === 1 ? 'part of the reply' : `${unheard.length} parts of the reply`;
    return `${what} could not be spoken${reason}`;
  }

  const SPEECH_CHUNK_MAX_CHARS = 200;
  // What a closing emphasis run leaves behind, so the word it marked is heard
  // with a beat after it rather than as a clip boundary in its own right. A
  // spaced em dash is punctuation no engine reads as a letter, where a comma
  // would also change the clause's intonation. The whitespace the run left
  // behind is swallowed so the dash is not followed by two spaces.
  // This is the one place to tune it.
  const SPEECH_EMPHASIS_BEAT = ' — ';
  // Punctuation that already supplies that beat, so adding one would double it.
  const SPEECH_NO_BEAT_AFTER = '.,!?;:—–…"\')]}';
  // The wrappers the prompts use for emphasis: "*so*", "**mad**", "_slow_",
  // "[aside]", "(aside)". Sentence boundaries follow these, where they used to
  // be cut at them -- a run of two or three characters is not worth a synthesis
  // round trip, and the wait for that round trip is what the ears hear as a
  // pause on either side of the word.
  //
  // The underscore form is guarded against word characters so that a name like
  // `my_script_py` is not read as emphasis.
  //
  // The trailing space is part of the match so a beat can replace it rather than
  // sit in front of it; the group is the second thing the replace callback sees.
  const SPEECH_EMPHASIS_PATTERN = /(\*{1,2}[^*]+\*{1,2}|(?<![A-Za-z0-9_])_[^_]+_(?![A-Za-z0-9_])|\[[^\]]+\]|\([^)]+\))([ \t]*)/g;
  // The wrapper markup a sentence is never cut inside: "*x*", "_x_", "[x]",
  // "(x)". Half a wrapper is not markup any more -- the words survive the cut,
  // but whatever was meant to be taken off them does not -- so a boundary
  // inside one leaves the raw characters to be spoken or, worse, to be sent on
  // to a server that has to make sense of them. Mirrors _SPLITTER_MARKUP_PAIRS
  // in agent.py, minus "{tone}": the two sides differ there on purpose. The
  // server's streamer holds a tone region together; this file reads the tone
  // off the sentence that opens the tag and strips the tag out of every
  // sentence, so a boundary inside one is harmless and is what keeps a reply's
  // per-sentence delivery.
  const SENTENCE_MARKUP_PAIRS = [['[', ']'], ['(', ')'], ['*', '*'], ['_', '_']];
  // What closes a wrapper: the set a terminator can be sitting inside.
  // "(as I said. Really.)" ends its sentence after the ")" and not after
  // "said.", and "It was *so good.* Then he left." keeps the asterisk that
  // closes the run the full stop is inside -- without it the sentence would be
  // left one asterisk short and hold the next sentence hostage. Derived from
  // the pairs above so the two cannot drift apart.
  const SENTENCE_CLOSING_MARKUP = [...new Set(SENTENCE_MARKUP_PAIRS.map(([, closing]) => closing))].join('');
  // What ends a sentence.
  const SENTENCE_TERMINATORS = '.!?';
  // A piece holding nothing but terminators -- the "..." a reply sometimes
  // opens with -- is not a sentence of its own; the words after it are. Built
  // from the terminator list so the two cannot drift apart.
  const SENTENCE_WORD_PATTERN = new RegExp(`[^${SENTENCE_TERMINATORS}\\s]`);
  const VARIANT_BASE = 'base';
  const VARIANT_MARKED = 'marked';
  // A sentence that is nothing but a hum. It gets a delivery of its own,
  // because a hum has no words to carry the length the sound wants.
  const VARIANT_VOCALIZATION = 'vocalization';
  // The same hum written with an h in it -- "mmmh", "mmmnh". Those read as a
  // touch urgent, so the shape starts higher and falls back to about the
  // voice's own pitch.
  const VARIANT_VOCALIZATION_URGENT = 'vocalization-urgent';
  // The open sound of an intimate moment, "ahhh", and the same sound written
  // with more h's, which reads as more urgent still.
  const VARIANT_VOCALIZATION_AHH = 'vocalization-ahh';
  const VARIANT_VOCALIZATION_AHH_URGENT = 'vocalization-ahh-urgent';
  // Every variant that is a sound rather than words. Adding a sound means
  // adding its variants here, which is what the profile and the renderer read.
  const VOCALIZATION_VARIANTS = new Set([
    VARIANT_VOCALIZATION,
    VARIANT_VOCALIZATION_URGENT,
    VARIANT_VOCALIZATION_AHH,
    VARIANT_VOCALIZATION_AHH_URGENT,
  ]);

  // How a non-verbal hum is spelled for the voices: the /m/ of "mother", no h.
  // edge-tts has no SSML and reads an all-consonant run it does not know one
  // letter at a time, so the spelling is the only lever there is. This is the
  // one place to tune the sound: if the ear says "mmm" is still spelled out,
  // "hmm" is the fallback -- it hums, but opens with an h.
  const VOCALIZATION_HUM = 'mmm';
  // The open sound: one breath, carried on an a. Every "ahhh" spelling the
  // entity writes reduces to this one, and the h's are the whole of it -- how
  // many there were is what makes it softer or more urgent.
  const VOCALIZATION_AHH = 'ahhh';
  // The closed-mouth agreement, in the spelling a dictionary gives it. The
  // hyphen gives the engine a natural two-beat shape.
  const VOCALIZATION_AGREEMENT = 'mm-hmm';
  // A vocalization's inner shape: it starts at the voice's own pitch -- its
  // middle -- and falls across the part of the clip that carries the sound.
  // A hum with an h starts a tenth above that, so its fall comes to rest about
  // where the ordinary hum began. The urgent ahhh opens no higher than the
  // voice itself and climbs from there, a twentieth a step to a fifth above
  // the voice, because desire pressing harder is heard in how high the sound
  // starts; the rung it takes is how many h's beyond the fourth were written.
  // Every rung falls to the same landing as the softer ahhh. edge-tts allows
  // one pitch per request and has
  // no SSML, so the fall is rendered into the clip by the page (Web Audio)
  // before it is played; the slower playback that lengthens the sound
  // preserves pitch, so it survives.
  const VOCALIZATION_PITCH_START = 1.0;
  const VOCALIZATION_PITCH_END = 0.9;
  const VOCALIZATION_URGENT_PITCH_START = 1.1;
  // The urgent ahhh is heard in a ladder of openings: the base rung is the
  // voice's own pitch, and each step above it adds a twentieth, up to a fifth
  // above the voice. Every rung comes to rest where the softer ahhh lands, so
  // the urgency is the longer fall from higher, not a sound that merely sits
  // high. The ladder steps once per h the entity writes beyond the fourth, so
  // the more intense moments the prompt asks to be written longer are also
  // heard higher. Every step lands on the same closing pitch; only the height
  // it opens at changes.
  const VOCALIZATION_AHH_URGENT_PITCH_STARTS = [1.0, 1.05, 1.1, 1.15, 1.2];
  const VOCALIZATION_AHH_URGENT_PITCH_END = 0.9;
  // Three h's is the softer sound ("ahhh"); a fourth makes it the urgent one.
  const VOCALIZATION_AHH_URGENT_H = 4;
  // Spellings a voice already hums as written. "hmm" opens with an h rather
  // than the /m/ the entity's hums are, so it is not rewritten to the hum --
  // it is left as the entity wrote it, and it is not slowed either.
  const VOCALIZATION_KNOWN_FORMS = new Set(['hmm']);
  // Written forms that mean the agreement, keyed without the hyphen.
  const VOCALIZATION_AGREEMENT_FORMS = new Set(['mmhmm', 'mhm']);
  // Mirrors PROTECTED_TEXT_PATTERN in agent.py: the server protects URLs and
  // email addresses from its own normalisation, and this pass must not rewrite
  // a token inside one. Kept in step by hand, like the markup pairs above.
  const SPEECH_PROTECTED_TEXT_PATTERN = /https?:\/\/[^\s,]+|www\.[^\s,]+|\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/gi;
  // A standalone token of h/m/n letters, or an h/m/n hyphen compound: not part
  // of a longer word, an identifier or a possessive, which is what keeps
  // "hymn", "my_hmm" and "Mmmm's" out of it.
  const VOCALIZATION_TOKEN_PATTERN = /(?<![A-Za-z0-9_-])([hmn]+(?:-[hmn]+)*)(?![A-Za-z0-9_]|[-'][A-Za-z])/gi;
  // The open sound, written as an a carried on two or more h's: "ahh", "ahhh",
  // "aahhh". One h is the ordinary interjection ("ah"), and a word that merely
  // starts with those letters ("ahead") fails the boundary.
  const VOCALIZATION_AHH_PATTERN = /(?<![A-Za-z0-9_-])(a{1,3}h{2,})(?![A-Za-z0-9_]|[-'][A-Za-z])/gi;
  // Every sound a sentence can be made of and nothing else.
  const VOCALIZATION_SOUNDS = new Set([VOCALIZATION_HUM, VOCALIZATION_AHH]);
  // A sound standing as a clause of its own inside a sentence. Either sound, in
  // any spelling, as one token; the two patterns above are the single source of
  // truth for what a sound is, so this only joins them. Group 1 is the open
  // sound and group 2 the hum, whichever one matched.
  const VOCALIZATION_CLAUSE_PATTERN = new RegExp(
    `(?:${VOCALIZATION_AHH_PATTERN.source})|(?:${VOCALIZATION_TOKEN_PATTERN.source})`,
    'gi',
  );
  // What sets a clause off: the punctuation that marks a pause, and the
  // closers a pause can be wrapped in. A sound with words pressed against it
  // on either side -- "say mmm to me" -- is inside the clause, not a clause of
  // its own, and is left where it is.
  const VOCALIZATION_CLAUSE_BOUNDARY_PATTERN = /[.,;:!?…—"'“”‘’)\]}>»«]/;

  function splitSpeechIntoChunks(spokenText, maxChunkChars) {
    // Cuts normalised speech into pieces one synthesis request can carry. Chunks
    // follow sentence boundaries where they can and whole words where a single
    // sentence is longer than the cap allows: text is never cut mid-word and
    // never dropped, because both would be audible.
    //
    // The caller passes the cap, and the one that matters is the server's
    // TTS_CACHE_MAX_TEXT_CHARS. Past it the server declines to cache the clip, so
    // every play of that text is a fresh synthesis -- which is what makes Repeat
    // slow and what makes one interruption cost a whole long paragraph.
    // SPEECH_CHUNK_MAX_CHARS is the client's answer to a server limit of 240: the
    // gap between them is left for the two sides' normalisation disagreeing about
    // how long the same words are.
    //
    // The input is expected already normalised -- the page's
    // normalizeTextForSpeech, or the equivalent server-side. Normalising here as
    // well would re-key every clip in the TTS cache, since the key covers the
    // exact text.
    const text = String(spokenText || '').trim();
    if (!text) {
      return [];
    }
    const limit = Math.max(1, Math.floor(Number(maxChunkChars)) || 1);
    if (text.length <= limit) {
      return [text];
    }

    const sentences = splitIntoSentences(text);
    const chunks = [];
    let current = '';

    function flush() {
      const trimmed = current.trim();
      if (trimmed) {
        chunks.push(trimmed);
      }
      current = '';
    }

    function pushLongSentence(sentenceText) {
      let part = '';
      for (const word of sentenceText.trim().split(/\s+/)) {
        const candidate = part ? `${part} ${word}` : word;
        if (candidate.length <= limit) {
          part = candidate;
          continue;
        }
        if (part) {
          chunks.push(part.trim());
        }
        part = word;
      }
      if (part.trim()) {
        chunks.push(part.trim());
      }
    }

    for (const sentence of sentences) {
      if (sentence.length > limit) {
        flush();
        pushLongSentence(sentence);
        continue;
      }
      const candidate = current ? `${current} ${sentence}` : sentence;
      if (candidate.length <= limit) {
        current = candidate;
      } else {
        flush();
        current = sentence;
      }
    }

    flush();
    // A cap too small for even one word still has to return the words.
    return chunks.length ? chunks : [text];
  }

  function countCharacter(text, character) {
    let count = 0;
    for (const found of text) {
      if (found === character) {
        count += 1;
      }
    }
    return count;
  }

  function markupBalanced(text) {
    // Whether every wrapper the text opens, it also closes. This is the rule
    // that decides whether a terminator is inside a wrapper, and it is the
    // server's own rule (SentenceSplitter._markup_balanced in agent.py) so that
    // both sides agree about where a sentence ends.
    return SENTENCE_MARKUP_PAIRS.every(([opening, closing]) => {
      const opened = countCharacter(text, opening);
      if (opening === closing) {
        // "*action*" and "_emphasis_" use one character twice, so an even
        // number of them is the whole of the test.
        return opened % 2 === 0;
      }
      return opened === countCharacter(text, closing);
    });
  }

  function sentenceEnd(text, terminatorEnd) {
    // Where the sentence carrying the terminator at terminatorEnd really ends.
    // A terminator inside a wrapper takes the wrapper's closers with it, and
    // the spaces between them, so an aside holding full stops of its own --
    // "(Mmm, and you know, a berry that is too ripe... it just wants to be
    // held in the palm... wrist.  )" -- is one sentence and not three, and it
    // is still the aside when the words are read out of it.
    let end = terminatorEnd;
    let probe = terminatorEnd;
    for (;;) {
      let next = probe;
      while (next < text.length && /\s/.test(text[next])) {
        next += 1;
      }
      if (next >= text.length || !SENTENCE_CLOSING_MARKUP.includes(text[next])) {
        return end;
      }
      probe = next + 1;
      end = probe;
    }
  }

  function isDecimalPoint(text, index) {
    // Whether the "." at index is the point of a number rather than a
    // terminator. "3.5" is one number, and the whole of it has to reach the
    // engine in one piece: cut at the point, the two halves become two clips
    // and the number is heard as two numbers with a pause between them, with
    // the "point" never said at all. Both sides have to agree -- the server's
    // SentenceSplitter holds the same boundary (_is_non_terminal_period in
    // agent.py) -- because a reply cut in two places is a reply heard twice
    // over in the wrong places.
    return text[index] === '.'
      && index > 0
      && /[0-9]/.test(text[index - 1])
      && /[0-9]/.test(text[index + 1] || '');
  }

  function splitIntoSentences(text) {
    // The sentences inside a run of text: each piece up to and including its
    // terminator, with a final unterminated piece kept so a half-finished reply
    // still speaks. splitSpeechIntoChunks cuts on these same boundaries, so the
    // rule lives here once rather than in both callers.
    //
    // A terminator that is not one -- the point of "3.5" -- is stepped over
    // rather than read as an ending.
    //
    // A terminator only ends a sentence where the wrappers opened so far are
    // closed, and where the sentence so far has words in it. The balance half
    // is what keeps a wrapper whole, and a whole wrapper is what gets stripped:
    // cut in the middle of an aside, a reply was sent to the server as "(Mmm,
    // and you know, a berry that is too ripe...", an opening bracket with no
    // closing one, which the server could not resolve -- so the request behind
    // it never came back and the reply stopped playing right there.
    const trimmed = String(text || '').trim();
    if (!trimmed) {
      return [];
    }
    const sentences = [];
    let start = 0;
    let index = 0;
    while (index < trimmed.length) {
      if (!SENTENCE_TERMINATORS.includes(trimmed[index])) {
        index += 1;
        continue;
      }
      if (isDecimalPoint(trimmed, index)) {
        index += 1;
        continue;
      }
      let terminatorEnd = index + 1;
      while (terminatorEnd < trimmed.length && SENTENCE_TERMINATORS.includes(trimmed[terminatorEnd])) {
        terminatorEnd += 1;
      }
      // A terminator run with no words in front of it is left where it is: it
      // joins the sentence that follows rather than becoming a "sentence" of
      // its own.
      if (SENTENCE_WORD_PATTERN.test(trimmed.slice(start, index))) {
        const end = sentenceEnd(trimmed, terminatorEnd);
        const sentence = trimmed.slice(start, end).trim();
        if (markupBalanced(sentence)) {
          if (sentence) {
            sentences.push(sentence);
          }
          start = end;
        }
      }
      index = terminatorEnd;
    }
    // Whatever is left is spoken even when it is not balanced: end of text is
    // the one boundary no wrapper can hold open.
    const tail = trimmed.slice(start).trim();
    if (tail) {
      sentences.push(tail);
    }
    return sentences;
  }

  function markerWords(marker) {
    // What a wrapper says out loud: everything except the wrapper.
    if (marker.startsWith('[') || marker.startsWith('(')) {
      return marker.slice(1, -1);
    }
    return marker.replace(/^[*_]+/, '').replace(/[*_]+$/, '');
  }

  function needsEmphasisBeat(source, endIndex) {
    // A beat belongs after an emphasised word that a word follows. Before
    // punctuation it would only double a pause that is already there.
    const next = source.slice(endIndex).replace(/^[ \t]+/, '').charAt(0);
    return Boolean(next) && !SPEECH_NO_BEAT_AFTER.includes(next);
  }

  function markerToneName(marker, knownTones) {
    // The tone a wrapper names when it wraps nothing but a tag: "[whisper]",
    // "[/whisper]", "*whisper*". A wrapper holding words is emphasis and its
    // words are said, however it reads; only a known tone name is control
    // markup, because "whisper" is also a word the entity may use.
    const inner = markerWords(marker).trim();
    const closing = inner.startsWith('/');
    const name = (closing ? inner.slice(1) : inner).trim().toLowerCase();
    if (!name || !TONE_NAME_PATTERN.test(name)) {
      return null;
    }
    const known = knownTones || {};
    if (!Object.prototype.hasOwnProperty.call(known, name)) {
      return null;
    }
    return { name, closing };
  }

  function stripSpeechMarkers(text, knownTones) {
    // Tone tags are removed first: they are control text, and taking them out
    // before the emphasis pass is what lets "*so*{/excited}." see the full stop
    // and skip the beat it would otherwise add in front of it.
    let source = String(text || '').replace(TONE_TAG_PATTERN, '');
    // A bare tone name at the head of the sentence is the same command as a
    // tag, with no delimiters: it goes, and sentenceTone reads it as the tone.
    const bare = leadingBareTone(source, knownTones);
    if (bare) {
      source = source.slice(bare.end).replace(/^\s+/, '');
    }
    if (!source) {
      return '';
    }
    // The callback returns the space the wrapper left behind when no beat
    // replaces it, and drops it when one does, so the beat never trails a
    // doubled space.
    return source.replace(SPEECH_EMPHASIS_PATTERN, (match, marker, spaces, offset) => {
      if (markerToneName(marker, knownTones)) {
        // The tag names a delivery and no word. The space in front of it
        // already separates the words around it; the one behind it would
        // double that.
        const before = offset ? source.charAt(offset - 1) : '';
        return before && /\s/.test(before) ? '' : spaces;
      }
      const words = markerWords(marker);
      return needsEmphasisBeat(source, offset + marker.length)
        ? `${words}${SPEECH_EMPHASIS_BEAT}`
        : `${words}${spaces}`;
    });
  }

  function respellVocalizationToken(token) {
    // One token of hum letters as the voices should say it. Every hum spelling
    // the entity writes -- "mmm", "mmmh", "mmmnh", "mmmm" -- reduces to the one
    // canonical spelling, so the variants are not three sounds but one. An n
    // in the middle of one is dropped with the rest of the run: it forms no
    // part of the hum, and the engine hears it as a sound of its own. The
    // exceptions are the forms a voice already says correctly: "hmm" keeps its
    // h, and the agreement keeps its two-beat shape.
    const lower = token.toLowerCase();
    const letters = lower.replace(/-/g, '');
    if (letters.length < 3) {
      // "mm" as a unit, "hm": too short to be a hum, and a unit is not one.
      return token;
    }
    if (lower === VOCALIZATION_AGREEMENT || VOCALIZATION_KNOWN_FORMS.has(lower)) {
      return token;
    }
    if (VOCALIZATION_AGREEMENT_FORMS.has(letters)) {
      return VOCALIZATION_AGREEMENT;
    }
    if (!letters.includes('m') && !letters.includes('h')) {
      // A run of nothing but n's is not a hum. The n takes no part in forming
      // the sound -- it is dropped from the spellings that do carry one -- so
      // a run without an m or an h is left as the entity wrote it rather than
      // invented into a sound.
      return token;
    }
    return VOCALIZATION_HUM;
  }

  function respellAhhRun(text) {
    // Every "ahhh" spelling is one sound: the h's only decide how urgent it is
    // read, and that is read from the original text before this pass runs.
    return String(text || '').replace(VOCALIZATION_AHH_PATTERN, VOCALIZATION_AHH);
  }

  function respellVocalizationRun(text) {
    return respellAhhRun(
      String(text || '').replace(VOCALIZATION_TOKEN_PATTERN, respellVocalizationToken),
    );
  }

  function respellVocalizations(text) {
    // The non-verbal sounds the entity writes, spelled the way a voice says
    // them. A URL or an email is never rewritten, so the pass walks around the
    // spans the server itself protects; a text with nothing to change comes
    // back byte for byte, which keeps its TTS cache key.
    const source = String(text || '');
    if (!source) {
      return '';
    }
    let result = '';
    let last = 0;
    for (const match of source.matchAll(SPEECH_PROTECTED_TEXT_PATTERN)) {
      result += respellVocalizationRun(source.slice(last, match.index));
      result += match[0];
      last = match.index + match[0].length;
    }
    return result + respellVocalizationRun(source.slice(last));
  }

  function isVocalizationOnly(spokenText) {
    // A sound as a sentence of its own: every word left is a sound. A sentence
    // that still has words in it is not slowed, because the words would be too.
    const words = String(spokenText || '')
      .replace(/[^A-Za-z-]+/g, ' ')
      .trim()
      .split(/\s+/)
      .filter(Boolean);
    return words.length > 0 && words.every((word) => VOCALIZATION_SOUNDS.has(word.toLowerCase()));
  }

  function ahhSounds(text) {
    // Every "ahhh"-shaped run the text holds, as written. The count of h's is
    // read here, before the respelling pass reduces them all to one sound.
    const source = String(text || '');
    VOCALIZATION_AHH_PATTERN.lastIndex = 0;
    return Array.from(source.matchAll(VOCALIZATION_AHH_PATTERN), (match) => match[1].toLowerCase());
  }

  function ahhBreaths(token) {
    // How hard an open sound was pressed: the h's are the breath, and the
    // prompt asks for more of them at the more intense moments.
    return (String(token || '').match(/h/gi) || []).length;
  }

  function ahhUrgentLevel(breaths, atLeastLevel = 0) {
    // One rung of the urgent ladder per breath written beyond the fourth, and
    // never below the rung the run has already reached: a sound after another
    // in the same run always opens higher than the one before it. A caller
    // that cannot say how many breaths there were gets the first rung, and the
    // ladder stops at its top rather than falling off.
    const written = Math.floor(Number(breaths)) - VOCALIZATION_AHH_URGENT_H;
    const floor = Math.floor(Number(atLeastLevel));
    const level = Math.max(
      Number.isFinite(written) ? written : 0,
      Number.isFinite(floor) ? floor : 0,
    );
    return Math.max(0, Math.min(level, VOCALIZATION_AHH_URGENT_PITCH_STARTS.length - 1));
  }

  function ahhUrgentStartPitch(breaths, atLeastLevel = 0) {
    return VOCALIZATION_AHH_URGENT_PITCH_STARTS[ahhUrgentLevel(breaths, atLeastLevel)];
  }

  function humHasBreath(text) {
    // Whether a hum was written with an h in it -- "mmmh", "mmmnh". Those read
    // as a touch urgent where a plain "mmm" is only a sound, so the two take
    // different shapes. The h has to be found before respellVocalizations runs,
    // because that pass reduces every spelling to the one canonical hum.
    const source = String(text || '');
    VOCALIZATION_TOKEN_PATTERN.lastIndex = 0;
    for (const match of source.matchAll(VOCALIZATION_TOKEN_PATTERN)) {
      const token = match[1];
      if (respellVocalizationToken(token) === VOCALIZATION_HUM && /h/i.test(token)) {
        return true;
      }
    }
    return false;
  }

  function isVocalizationVariant(variant) {
    return VOCALIZATION_VARIANTS.has(variant);
  }

  function vocalizationPitchShape(variant, breaths = 0) {
    // The fall baked into a vocalization's clip: the rate it opens at and the
    // rate it comes to rest at, as fractions of the clip's own speed. A rate
    // below one slows the clip, and that slowing is what lowers the pitch.
    //
    // The hum and the softer ahhh open at the voice's own pitch and fall a
    // tenth. The urgent hum opens a tenth higher and falls the same tenth, so
    // it comes to rest where an ordinary hum began. The urgent ahhh opens on
    // the breath ladder -- a fifth high at the fourth h, a rung higher for
    // each breath after it -- and every rung falls to the softer sound's
    // landing.
    if (variant === VARIANT_VOCALIZATION_AHH_URGENT) {
      return {
        startPitch: ahhUrgentStartPitch(breaths),
        endPitch: VOCALIZATION_AHH_URGENT_PITCH_END,
      };
    }
    if (variant === VARIANT_VOCALIZATION_URGENT) {
      return {
        startPitch: VOCALIZATION_URGENT_PITCH_START,
        endPitch: VOCALIZATION_URGENT_PITCH_START * VOCALIZATION_PITCH_END,
      };
    }
    return {
      startPitch: VOCALIZATION_PITCH_START,
      endPitch: VOCALIZATION_PITCH_START * VOCALIZATION_PITCH_END,
    };
  }

  function voicedSpan(samples, sampleRate) {
    // Where the sound of a clip begins and ends, in seconds. The engine pads a
    // hum with silence at both ends (the server trims it to a lead and a tail);
    // the fall belongs to the voiced stretch, not to that padding.
    const rate = Number(sampleRate) || 24000;
    const total = samples ? samples.length : 0;
    if (!total) {
      return { start: 0, end: 0 };
    }
    const block = Math.max(1, Math.round(rate * 0.01));
    const levels = [];
    let peak = 0;
    for (let first = 0; first < total; first += block) {
      const last = Math.min(first + block, total);
      let energy = 0;
      for (let index = first; index < last; index += 1) {
        const value = samples[index];
        energy += value * value;
      }
      const level = Math.sqrt(energy / (last - first));
      levels.push(level);
      if (level > peak) {
        peak = level;
      }
    }
    if (!(peak > 0)) {
      return { start: 0, end: total / rate };
    }
    // A tenth of the loudest ten milliseconds -- -20 dB. A hum has fallen to
    // that at its edges and the engine's silence never reaches it.
    const floor = peak * 0.1;
    let first = -1;
    let last = -1;
    levels.forEach((level, index) => {
      if (level < floor) {
        return;
      }
      if (first < 0) {
        first = index;
      }
      last = index;
    });
    if (first < 0) {
      return { start: 0, end: total / rate };
    }
    return {
      start: (first * block) / rate,
      end: Math.min(total, (last + 1) * block) / rate,
    };
  }

  function vocalizationPitchPlan(
    duration,
    span,
    startPitch = VOCALIZATION_PITCH_START,
    endPitch = startPitch * VOCALIZATION_PITCH_END,
  ) {
    // The rate the sound is played through at, and when each part of that
    // happens. The rate holds the opening pitch until the sound begins, ramps
    // to the closing pitch across the voiced span, and holds there.
    // `sourceEnd` is when the source has played through under that shape: the
    // render lasts exactly that long, so the whole sound is heard and no
    // silence is added around it.
    //
    // The source advances at the rate's own speed, so a sound played slower
    // than real time needs a longer render than its own length; one opening
    // high with little tail can instead finish before the ramp does. The
    // crossing is solved on whichever part of the shape the source runs out in.
    const total = Math.max(0, Number(duration) || 0);
    const opening = Math.max(0.25, Math.min(Number(startPitch) || VOCALIZATION_PITCH_START, 2));
    const closing = Math.max(0.25, Math.min(Number(endPitch) || VOCALIZATION_PITCH_END, 2));
    const holdUntil = Math.max(0, Math.min(Number(span && span.start) || 0, total));
    const rampEnd = Math.max(holdUntil, Math.min(Number(span && span.end) || 0, total));
    const plan = { holdUntil, rampEnd, sourceEnd: 0, pitchStart: opening, pitchEnd: closing };
    if (!(total > 0) || !(rampEnd > holdUntil)) {
      // Nothing to shape, or nothing heard after the opening: the whole clip
      // plays through at one pitch, and the render is its length over that.
      return { ...plan, sourceEnd: total / opening };
    }
    const held = holdUntil * opening;
    const length = rampEnd - holdUntil;
    const drop = closing - opening;
    const mean = (opening + closing) / 2;
    const consumedAtRampEnd = held + length * mean;
    if (total <= held) {
      // The source runs out before the sound begins: only the opening pitch is
      // ever heard.
      return { ...plan, sourceEnd: total / opening };
    }
    if (total <= consumedAtRampEnd) {
      // The source runs out while the rate is still coming down. Consumption
      // across the ramp is u * opening + drop * u^2 / (2 * length), whose
      // inverse is this root.
      const along = drop === 0
        ? (total - held) / opening
        : (length / drop)
          * (Math.sqrt(opening * opening + (2 * drop * (total - held)) / length) - opening);
      return { ...plan, sourceEnd: holdUntil + Math.max(0, Math.min(along, length)) };
    }
    return { ...plan, sourceEnd: rampEnd + (total - consumedAtRampEnd) / closing };
  }

  function encodeWav(channels, sampleRate) {
    // 16-bit PCM WAV. The pitch fall is rendered by Web Audio, which hands back
    // samples and no container; a WAV is the one format every player takes and
    // the writer every browser can produce without an encoder.
    const list = Array.isArray(channels) ? channels : [channels];
    const count = Math.max(1, list.length);
    const frames = list[0] ? list[0].length : 0;
    const rate = Math.max(1, Math.round(Number(sampleRate) || 24000));
    const dataSize = frames * count * 2;
    const bytes = new Uint8Array(44 + dataSize);
    const view = new DataView(bytes.buffer);
    const ascii = (offset, text) => {
      for (let index = 0; index < text.length; index += 1) {
        bytes[offset + index] = text.charCodeAt(index);
      }
    };
    ascii(0, 'RIFF');
    view.setUint32(4, 36 + dataSize, true);
    ascii(8, 'WAVE');
    ascii(12, 'fmt ');
    view.setUint32(16, 16, true);              // fmt chunk size
    view.setUint16(20, 1, true);               // PCM
    view.setUint16(22, count, true);
    view.setUint32(24, rate, true);
    view.setUint32(28, rate * count * 2, true); // byte rate
    view.setUint16(32, count * 2, true);       // block align
    view.setUint16(34, 16, true);              // bits per sample
    ascii(36, 'data');
    view.setUint32(40, dataSize, true);
    let offset = 44;
    for (let frame = 0; frame < frames; frame += 1) {
      for (let channel = 0; channel < count; channel += 1) {
        const samples = list[channel] || list[0];
        const value = Math.max(-1, Math.min(1, samples ? samples[frame] : 0));
        view.setInt16(offset, Math.round(value < 0 ? value * 0x8000 : value * 0x7FFF), true);
        offset += 2;
      }
    }
    return bytes;
  }

  function sentenceVariant(rawSentence, spokenText, knownTones) {
    // One clip carries one delivery, so a sentence holding any emphasis wrapper
    // is spoken wholly at the marked profile -- the sentence is the smallest
    // unit that profile can apply to. A sentence that is nothing but a sound is
    // neither: it takes the vocalisation profile, which is what makes the sound
    // long, and the shape of the sound -- which sound it is and how urgent it
    // was written -- starts it higher or lower and decides how far it falls.
    //
    // The caller that has already stripped the wrappers passes the words it
    // has; anyone else gets them derived here, the same way.
    const stripped = stripSpeechMarkers(rawSentence, knownTones);
    const spoken = typeof spokenText === 'string'
      ? spokenText
      : respellVocalizations(stripped);
    if (isVocalizationOnly(spoken)) {
      // An open "ahhh" carries the line when one is present, even beside a hum;
      // otherwise the sound is the hum. The breath decides the urgency of
      // whichever sound it is, and it is read before the respelling pass, which
      // reduces every spelling to the one canonical sound.
      const ahh = ahhSounds(stripped);
      if (ahh.length) {
        const urgent = ahh.some(
          (sound) => soundVariantForToken(sound, true) === VARIANT_VOCALIZATION_AHH_URGENT,
        );
        return urgent ? VARIANT_VOCALIZATION_AHH_URGENT : VARIANT_VOCALIZATION_AHH;
      }
      return humHasBreath(stripped) ? VARIANT_VOCALIZATION_URGENT : VARIANT_VOCALIZATION;
    }
    // A wrapper naming a tone is control markup, not emphasis: no beat was
    // added for it, so the sentence is not spoken at the marked profile.
    const source = String(rawSentence || '');
    SPEECH_EMPHASIS_PATTERN.lastIndex = 0;
    let match;
    while ((match = SPEECH_EMPHASIS_PATTERN.exec(source)) !== null) {
      if (!markerToneName(match[1], knownTones)) {
        return VARIANT_MARKED;
      }
    }
    return VARIANT_BASE;
  }

  function sentenceTone(rawSentence, knownTones) {
    // The tone a sentence is spoken under: the first opening tag naming a tone
    // the caller knows, or a bare name at the head of the sentence, which is
    // the tag with no delimiters. The server's SentenceSplitter refuses any
    // boundary where "{" is unbalanced, so a brace region always opens and
    // closes inside the sentence it styles -- there is never one inherited from
    // an earlier sentence to weigh against this one.
    const source = String(rawSentence || '');
    const known = knownTones || {};
    const bare = leadingBareTone(source, known);
    if (bare) {
      return bare.name;
    }
    TONE_TAG_PATTERN.lastIndex = 0;
    let match;
    while ((match = TONE_TAG_PATTERN.exec(source)) !== null) {
      if (match[1] === '/') {
        continue;
      }
      const name = match[0].replace(/[{}]/g, '').toLowerCase();
      if (Object.prototype.hasOwnProperty.call(known, name)) {
        return name;
      }
    }
    // The same tag in a wrapper's clothes: "[whisper]come closer[/whisper]".
    // Only an opening wrapper names a delivery; a closing one at the start of a
    // sentence is where the region written before it ended.
    SPEECH_EMPHASIS_PATTERN.lastIndex = 0;
    let wrapped;
    while ((wrapped = SPEECH_EMPHASIS_PATTERN.exec(source)) !== null) {
      const tagged = markerToneName(wrapped[1], known);
      if (tagged && !tagged.closing) {
        return tagged.name;
      }
    }
    return null;
  }

  function sentenceVocalizationBreaths(stripped, variant) {
    // How many breaths the sentence's sound was written with. Only the open
    // sound has a ladder: the hum's urgency is a single step, the agreement is
    // not a sound, and words have no breath at all. The written count is read
    // before respellVocalizations, which leaves only the canonical spelling.
    if (variant !== VARIANT_VOCALIZATION_AHH && variant !== VARIANT_VOCALIZATION_AHH_URGENT) {
      return 0;
    }
    return ahhSounds(stripped).reduce(
      (most, sound) => Math.max(most, ahhBreaths(sound)),
      0,
    );
  }

  function buildSpokenSentence(rawSentence, knownTones, normalizeText) {
    // One sentence as the speaker needs it: the words to say, the delivery to
    // say them in, how hard its sound was pressed, and nothing inside it to
    // split. The caller passes its own text cleaner so this file keeps no
    // second copy of that rule.
    const raw = String(rawSentence || '').trim();
    if (!raw) {
      return { text: '', variant: VARIANT_BASE, tone: null, pitchStart: null };
    }
    const normalize = typeof normalizeText === 'function'
      ? normalizeText
      : (value) => String(value || '').trim();
    // The vocalisation pass runs on the words the wrappers have already left
    // behind -- "[mmmm]" is words by then -- and before the caller's cleaner,
    // so it is the canonical spelling that is normalised, chunked and sent.
    const stripped = stripSpeechMarkers(raw, knownTones);
    const spoken = respellVocalizations(stripped);
    const variant = sentenceVariant(raw, spoken, knownTones);
    const breaths = sentenceVocalizationBreaths(stripped, variant);
    return {
      text: normalize(spoken),
      variant,
      tone: sentenceTone(raw, knownTones),
      // The height this sentence's sound opens at, worked out here because the
      // breath count that decides it is gone once the sound is respelled.
      pitchStart: isVocalizationVariant(variant)
        ? vocalizationPitchShape(variant, breaths).startPitch
        : null,
    };
  }

  function soundVariantForToken(token, isAhh) {
    // Which shape one written sound takes. The spelling is all the text
    // offers: an h in a hum, or a fourth h on an open sound, is the entity
    // pressing harder. It is read before respellVocalizations, which flattens
    // every spelling to the one canonical sound.
    const source = String(token || '');
    if (isAhh) {
      return ahhBreaths(source) >= VOCALIZATION_AHH_URGENT_H
        ? VARIANT_VOCALIZATION_AHH_URGENT
        : VARIANT_VOCALIZATION_AHH;
    }
    return /h/i.test(source) && respellVocalizationToken(source) === VOCALIZATION_HUM
      ? VARIANT_VOCALIZATION_URGENT
      : VARIANT_VOCALIZATION;
  }

  function vocalizationClauseCuts(source) {
    // Where a sound stands on its own inside a sentence, as a list of
    // {start, end, token, isAhh}. On both sides the sound must meet either the
    // edge of the sentence or the punctuation that marks a pause; the spaces
    // around it do not count, because ordinary words have those too, and a tone
    // tag beside it is control text, so the check looks past it.
    //
    // Such a sound can be lifted into a clip of its own and given the delivery
    // every sound gets. Left in the sentence it is one syllable under the
    // sentence's intonation -- short, and rising if the sentence runs on, which
    // is the opposite of the fall a sound wants.
    const text = String(source || '');
    const cuts = [];
    // A link or an address is never a sound, however its letters read; the
    // respelling pass walks around these spans, and so does this one.
    const protectedSpans = Array.from(
      text.matchAll(SPEECH_PROTECTED_TEXT_PATTERN),
      (span) => [span.index, span.index + span[0].length],
    );
    const inProtectedSpan = (index) => protectedSpans.some(
      ([from, to]) => index >= from && index < to,
    );
    VOCALIZATION_CLAUSE_PATTERN.lastIndex = 0;
    let match;
    while ((match = VOCALIZATION_CLAUSE_PATTERN.exec(text)) !== null) {
      if (inProtectedSpan(match.index)) {
        continue;
      }
      const token = match[1] || match[2];
      const isAhh = Boolean(match[1]);
      // Only a sound the voices actually say as a sound: "mm" is a unit,
      // "hmm" is left to the voice as written, and the agreement keeps its
      // two beats, so none of them is slowed or cut.
      if (!isAhh && respellVocalizationToken(token) !== VOCALIZATION_HUM) {
        continue;
      }
      const before = text.slice(0, match.index)
        .replace(/(?:\{[^}]*\}[ \t]*)+$/, '')
        .replace(/[ \t]+$/, '');
      const after = text.slice(match.index + match[0].length)
        .replace(/^(?:[ \t]*\{[^}]*\})+/, '')
        .replace(/^[ \t]+/, '');
      const opens = !before || VOCALIZATION_CLAUSE_BOUNDARY_PATTERN.test(before.slice(-1));
      const closes = !after || VOCALIZATION_CLAUSE_BOUNDARY_PATTERN.test(after.charAt(0));
      if (!opens || !closes) {
        continue;
      }
      cuts.push({
        start: match.index,
        end: match.index + match[0].length,
        token,
        isAhh,
      });
    }
    return cuts;
  }

  function buildSpokenSentenceSegments(rawSentence, knownTones, normalizeText) {
    // A sentence as the speaker needs it. Usually that is the sentence's own
    // clip. Where a sound stands alone as a clause, it is lifted into a clip
    // of its own, and the words on either side of it keep the sentence's tone.
    //
    // `text` stays the whole sentence, because that is what the caller records
    // as heard and repairs from; `segments` is what is actually spoken, and
    // each piece carries the delivery it needs.
    const raw = String(rawSentence || '').trim();
    const normalize = typeof normalizeText === 'function'
      ? normalizeText
      : (value) => String(value || '').trim();
    const whole = buildSpokenSentence(raw, knownTones, normalize);
    if (!whole.text) {
      return { text: '', variant: whole.variant, tone: whole.tone, pitchStart: null, segments: [] };
    }
    // A bare tone command at the head of the sentence is read as the tone by
    // `whole`; it is also control text, so the sound that follows it stands at
    // the start of what is left. Cutting works on that remainder, which is why
    // "whisper mmm, stay" is cut at the hum rather than leaving the command to
    // be spoken as a word.
    const heading = leadingBareTone(raw, knownTones);
    const body = heading ? raw.slice(heading.end).replace(/^\s+/, '') : raw;
    // A sentence that is already nothing but a sound is not cut when the sound
    // is the whole of it: that is the case the sound profile was built for, and
    // its clip is the whole of it. A run of them is different -- "ahhhhh,
    // ahhhhhh, ahhhhhhh" is how the most intense moments are written, and each
    // one wants its own height and its own fall rather than sharing the highest
    // breath in the line.
    const clauseCuts = vocalizationClauseCuts(body);
    const cuts = isVocalizationVariant(whole.variant) && clauseCuts.length < 2
      ? []
      : clauseCuts;
    if (!cuts.length) {
      return {
        text: whole.text,
        variant: whole.variant,
        tone: whole.tone,
        pitchStart: whole.pitchStart,
        segments: [whole],
      };
    }
    const segments = [];
    // The rung the last open sound of the run opened on, so the next sound in
    // the same run steps above it; words between sounds begin a new run.
    let runLevel = -1;
    const pushWords = (slice) => {
      const words = buildSpokenSentence(slice, knownTones, normalize);
      if (words.text) {
        segments.push({
          text: words.text,
          variant: words.variant,
          tone: whole.tone,
          pitchStart: words.pitchStart,
        });
        runLevel = -1;
      }
    };
    let last = 0;
    for (const cut of cuts) {
      pushWords(body.slice(last, cut.start));
      // The sound's own clip, with a full stop: the engine reads a lone
      // syllable with a question's rise, and a sound is a statement that falls.
      const breaths = cut.isAhh ? ahhBreaths(cut.token) : 0;
      const variant = soundVariantForToken(cut.token, cut.isAhh);
      let { startPitch } = vocalizationPitchShape(variant, breaths);
      if (variant === VARIANT_VOCALIZATION_AHH_URGENT) {
        // "ahhhh, ahhhh, ahhhh": each successive sound in the series opens a
        // little higher than the one before, whether or not the entity wrote
        // an extra h to say so.
        const level = ahhUrgentLevel(breaths, runLevel + 1);
        startPitch = VOCALIZATION_AHH_URGENT_PITCH_STARTS[level];
        runLevel = level;
      }
      segments.push({
        text: normalize(`${cut.isAhh ? VOCALIZATION_AHH : VOCALIZATION_HUM}.`),
        variant,
        tone: whole.tone,
        pitchStart: startPitch,
      });
      last = cut.end;
      // The punctuation that set the sound off is the pause the clip boundary
      // already is; leaving it on the words that follow would only double it.
      const skipped = /^[ \t]*[.,;:!?…—]+/.exec(body.slice(last));
      if (skipped) {
        last += skipped[0].length;
      }
    }
    pushWords(body.slice(last));
    return { text: whole.text, variant: whole.variant, tone: whole.tone, segments };
  }

  function snapshotPlayback(media) {
    // One instant of the element as plain data. The probe keeps a run of these
    // and reads a stall out of the run, so nothing here may throw and nothing
    // may assume an optional property exists.
    if (!media) {
      return null;
    }
    const orNull = (value) => {
      const numeric = Number(value);
      return Number.isFinite(numeric) ? numeric : null;
    };
    return {
      at: Date.now(),
      currentTime: orNull(media.currentTime),
      duration: orNull(media.duration),
      readyState: orNull(media.readyState),
      networkState: orNull(media.networkState),
      volume: orNull(media.volume),
      playbackRate: orNull(media.playbackRate),
      // Chromium counts the bytes it has decoded. Bytes climbing while nothing
      // was audible is what puts the loss below the decoder rather than in it.
      decodedBytes: orNull(media.webkitAudioDecodedByteCount),
      paused: media.paused === true,
      ended: media.ended === true,
      muted: media.muted === true,
      seeking: media.seeking === true,
      visibilityState: typeof globalThis !== 'undefined' && globalThis.document
        ? String(globalThis.document.visibilityState || '') || null
        : null,
    };
  }

  function readPlaybackStall(samples, events) {
    // Reads one failed clip's recorded run for a cause inside this page.
    //
    // It is only ever asked about a clip that failed, so finding nothing is a
    // result rather than the ordinary case: the element held the clip, its
    // pipeline stayed ready, its clock ran forward and it never reported
    // trouble -- yet the words did not reach the ears. Nothing here can be the
    // reason, which is what points the search below the element, at the output
    // device and the audio session.
    //
    // It cannot see an output underrun: the element API has no event for one, so
    // a null verdict alongside a symptom the user heard is itself the evidence,
    // and the next place to look is the Windows audio log rather than this page.
    const points = (Array.isArray(samples) ? samples : []).filter(Boolean);
    const fired = (Array.isArray(events) ? events : []).filter(Boolean);
    const names = new Set(fired.map((event) => String((event && event.name) || '')));
    // Readiness is judged only on samples taken while the clip was running: an
    // element waiting to start is early, not starved.
    const running = points.filter((point) => point.paused === false && point.ended !== true);

    if (names.has('stalled') || names.has('waiting')) {
      return 'media-starved';
    }
    if (running.some((point) => Number.isFinite(Number(point.readyState)) && Number(point.readyState) < 3)) {
      return 'media-starved';
    }
    if (running.some((point) => point.muted === true
      || (point.volume !== null && point.volume !== undefined && Number(point.volume) === 0))) {
      return 'output-silent';
    }

    const first = running[0];
    const last = running[running.length - 1];
    const advanced = running.length >= 2
      && Number.isFinite(Number(first.currentTime))
      && Number(last.currentTime) > Number(first.currentTime);
    return advanced ? 'timeline-advanced-unheard' : null;
  }

  function formatVoiceDisplayName(voice) {
    const rawName = String((voice && (voice.display_name || voice.name)) || '').trim();
    if (!rawName) {
      return 'Unknown voice';
    }

    const cleanedName = rawName
      .replace(/^Microsoft\s+/i, '')
      .replace(/\s+Online(?:\s*\([^)]*\))?/gi, '')
      .replace(/\s*\(Natural\)/gi, '')
      .replace(/\s*\((?:server|edge|windows|browser)\/\w{2}(?:-\w{2})?\)\s*$/i, '')
      .replace(/\s{2,}/g, ' ')
      .trim();

    const microsoftVoiceMatch = cleanedName.match(/^(.*?)\s+-\s+(.+)$/);
    if (!microsoftVoiceMatch) {
      return cleanedName.replace(/([^\s-])\s*(Multilingual\b)/gi, '$1 - $2').trim();
    }

    let namePart = microsoftVoiceMatch[1].trim();
    const localePart = microsoftVoiceMatch[2].trim();
    const isMultilingual = /multilingual/i.test(namePart);

    namePart = namePart
      .replace(/Multilingual/gi, '')
      .replace(/\s{2,}/g, ' ')
      .trim();

    const displayParts = [namePart];
    if (isMultilingual) {
      displayParts.push('Multilingual');
    }
    displayParts.push(localePart);
    return displayParts.filter(Boolean).join(' - ');
  }

  function getVoiceSpeakerName(voice) {
    const formattedName = formatVoiceDisplayName(voice);
    const speakerName = String(formattedName || '').split(' - ')[0].trim();
    return speakerName || 'The Entity';
  }

  function foldSpeakerName(name) {
    // Fold a speaker name to an accent-insensitive, filler-stripped token so
    // localized variants of the same voice ("Remy Multilingual" vs "Rémy
    // Multilingue") resolve to the same identity when merging voice lists.
    return String(name || '')
      .toLowerCase()
      .normalize('NFD').replace(/[̀-ͯ]/g, '')
      .replace(/\b(multilingual|multilingue|online|natural|neural|desktop|microsoft|server|speech|voice)\b/g, ' ')
      .replace(/[^a-z0-9]+/g, ' ')
      .trim();
  }

  function normalizeLocale(locale) {
    const value = String(locale || '').trim();
    if (!value) {
      return '';
    }

    const parts = value.replace('_', '-').split('-').filter(Boolean);
    if (!parts.length) {
      return '';
    }

    const language = parts[0].toLowerCase();
    const region = parts[1] ? parts[1].toUpperCase() : '';
    return region ? `${language}-${region}` : language;
  }

  function isMultilingualVoice(voice) {
    return /multilingual|multilingue/i.test(`${voice.name || ''} ${voice.display_name || ''}`);
  }

  function compareVoices(left, right) {
    const tierDifference = voiceTier(right) - voiceTier(left);
    if (tierDifference) {
      return tierDifference;
    }

    const leftDisplay = formatVoiceDisplayName(left) || '';
    const rightDisplay = formatVoiceDisplayName(right) || '';

    const extractBracketText = (text) => {
      const match = text.match(/\(([^)]+)\)/);
      return match ? match[1].trim() : text;
    };

    const leftBracket = extractBracketText(leftDisplay);
    const rightBracket = extractBracketText(rightDisplay);

    if (leftBracket !== rightBracket) {
      return leftBracket.localeCompare(rightBracket);
    }

    return leftDisplay.localeCompare(rightDisplay);
  }

  function voiceTier(voice) {
    // How good the engine behind the voice is, minted by the server so the
    // classification has one authority. A voice without it -- a browser voice --
    // ranks last.
    return Number(voice && voice.tier) || 0;
  }

  function convertLegacyVoiceValue(raw, knownIds) {
    // Stored selections predate character ids and look like
    // "source:provider:ShortName" (or "browser:browser:Name"). A character id
    // contains a colon too, so an exact match is checked before splitting.
    const value = normalizeString(raw);
    if (!value) {
      return '';
    }

    const known = knownIds instanceof Set ? knownIds : new Set(knownIds || []);
    if (known.has(value)) {
      return value;
    }

    const shortName = value.slice(value.lastIndexOf(':') + 1);
    if (shortName && known.has(shortName)) {
      return shortName;
    }

    const browserId = `browser:${shortName}`;
    if (shortName && known.has(browserId)) {
      return browserId;
    }

    return '';
  }

  function characterDepthBias(pitch) {
    // Character pitch is a baseline voice depth, where 72 is neutral.
    const value = Number(pitch);
    return Number.isFinite(value) ? value - 72 : 0;
  }

  function characterPitchMultiplier(pitch) {
    // Mirrors the server's depth bias as a Web Speech pitch multiplier. The 0.6
    // factor keeps it in step with the hand-tuned delivery-style pitch values.
    const multiplier = 1 + (characterDepthBias(pitch) / 72) * 0.6;
    return Math.max(0.25, Math.min(2.0, multiplier));
  }

  function dedupeKeyFor(entry) {
    // A character is identified by country, region and name -- never by its
    // voice file, so two characters sharing one voice stay two characters. The
    // server mints that key onto the entry, so there is one authority for what
    // counts as a duplicate. Voices and browser entries keep the speaker/locale
    // fold so existing variant collapsing is unchanged.
    const voice = entry || {};
    if (normalizeString(voice.kind) === 'character') {
      return normalizeString(voice.dedupe_key);
    }

    const locale = normalizeLocale(voice.lang || voice.culture || voice.locale || '');
    const source = voice.source || 'server';
    return `${source}|${foldSpeakerName(getVoiceSpeakerName(voice))}|${locale}`;
  }

  function buildCharacterOptionLabel(entry) {
    const voice = entry || {};
    const label = normalizeString(voice.label);
    return label || formatVoiceDisplayName(voice);
  }

  function buildCharacterInfoText(character) {
    // The one string the info panel beside the Character picker shows. Composed
    // here rather than in the page so the wording has one authority and one test.
    // The region is the picker's own fact (`character_region`, the region the
    // option label already carries); the rest is the character's definition,
    // which may hold none of it -- a character is playable on a voice alone.
    const voice = character || {};
    if (!normalizeString(voice.id)) {
      return 'Choose a character to see who they are.';
    }

    const details = voice.details || {};
    const lines = Array.isArray(details.backstory_lines) ? details.backstory_lines : [];
    const backstory = [...lines, details.backstory]
      .map((line) => normalizeString(line))
      .filter((line) => line)
      .join(' ');

    const parts = [];
    const region = normalizeString(voice.character_region);
    const born = normalizeString(details.born);
    const job = normalizeString(details.job);
    if (region) {
      parts.push(`From ${region}`);
    }
    if (born) {
      parts.push(`b. ${born}`);
    }
    if (job) {
      parts.push(`Job ${job}`);
    }
    if (backstory) {
      parts.push(`Backstory ${backstory}`);
    }

    if (parts.length) {
      return parts.join(', ');
    }

    // A character whose file authored none of it still says so in its own name,
    // rather than showing an empty panel that reads as a loading failure.
    const name = normalizeString(voice.character_name || voice.label);
    return `No details are recorded for ${name || 'this character'}.`;
  }

  // How long a country's roster is trusted before it is fetched again.
  //
  // The picker caches a country's characters so switching back to it does not
  // refetch, but nothing tells the client when the server's answer has changed:
  // a restart re-reads .env, and a country file can be edited between runs. An
  // entry that never expired therefore held the pre-restart roster for the life
  // of the tab, and
  // both recoveries the page offers -- switching country away and back, and
  // returning to the tab -- re-entered the loader and took the cached path
  // without asking. Short, because the request it saves goes to a server on this
  // machine; long enough that moving between two countries still never refetches.
  const CHARACTER_CACHE_TTL_MS = 30000;

  function characterCacheEntryIsFresh(entry, now) {
    if (!entry) {
      return false;
    }
    const loadedAt = Number(entry.loadedAt);
    if (!Number.isFinite(loadedAt)) {
      // An entry written before this check existed carries no timestamp.
      // Treating it as fresh would reinstate the staleness this fixes; treating
      // it as stale costs one refetch.
      return false;
    }
    return now - loadedAt < CHARACTER_CACHE_TTL_MS;
  }

  return {
    MCP_ACCESS_TOKEN_STORAGE_KEY,
    ENTITY_CLIENT_ID_STORAGE_KEY,
    ENTITY_COUNTRY_STORAGE_KEY,
    ENTITY_LANGUAGE_STORAGE_KEY,
    readStoredCountry,
    storeCountry,
    chooseRestoredCountry,
    readMcpAccessToken,
    storeMcpAccessToken,
    clearMcpAccessToken,
    normalizeClientId,
    storeClientId,
    getOrCreateClientId,
    buildEntityStreamUrl,
    buildOutboxRequestUrl,
    buildEntityStateUrl,
    readOutboxMessages,
    hasSeenDeliveryId,
    shouldDeliverProactive,
    proactiveDeliveryPlan,
    shouldVoiceProactive,
    shouldHoldProactive,
    shouldPollOutbox,
    proactiveToggleView,
    nextOutboxPollDelayMs,
    buildSelectedVoicePayload,
    buildChatRequestPayload,
    shouldResumeListening,
    shouldPauseAfterNoSpeech,
    shouldDictateIntoInput,
    shouldAutoSendDictation,
    shouldWaitLongerForSilence,
    shouldYieldMicToTyping,
    shouldRetrySpeechRequest,
    mergeDictatedText,
    readNewSentences,
    readNewSegments,
    splitReplyAfterStream,
    buildPrefacePlan,
    waitForMediaEnd,
    createSpeechQueue,
    chooseRememberedVoice,
    speechRecoveryText,
    canResumeClip,
    clipFinished,
    resumePosition,
    resumeSeekPosition,
    SPEECH_RESUME_OVERLAP_SECONDS,
    snapshotPlayback,
    readPlaybackStall,
    SPEECH_CHUNK_MAX_CHARS,
    SPEECH_EMPHASIS_BEAT,
    splitSpeechIntoChunks,
    splitIntoSentences,
    stripSpeechMarkers,
    respellVocalizations,
    VOCALIZATION_HUM,
    VOCALIZATION_AHH,
    VOCALIZATION_AGREEMENT,
    VOCALIZATION_PITCH_START,
    VOCALIZATION_PITCH_END,
    VOCALIZATION_URGENT_PITCH_START,
    VOCALIZATION_AHH_URGENT_PITCH_STARTS,
    VOCALIZATION_AHH_URGENT_PITCH_END,
    VOCALIZATION_AHH_URGENT_H,
    humHasBreath,
    ahhSounds,
    ahhBreaths,
    ahhUrgentStartPitch,
    isVocalizationVariant,
    vocalizationPitchShape,
    voicedSpan,
    vocalizationPitchPlan,
    encodeWav,
    VARIANT_VOCALIZATION,
    VARIANT_VOCALIZATION_URGENT,
    VARIANT_VOCALIZATION_AHH,
    VARIANT_VOCALIZATION_AHH_URGENT,
    sentenceVariant,
    sentenceTone,
    buildSpokenSentence,
    buildSpokenSentenceSegments,
    soundVariantForToken,
    vocalizationClauseCuts,
    speechPartialFailure,
    speechInterruptionError,
    isSpeechInterruption,
    normalizeReplyMarkup,
    formatVoiceDisplayName,
    getVoiceSpeakerName,
    foldSpeakerName,
    normalizeLocale,
    isMultilingualVoice,
    compareVoices,
    voiceTier,
    resolveCharacterId,
    convertLegacyVoiceValue,
    characterDepthBias,
    characterPitchMultiplier,
    dedupeKeyFor,
    buildCharacterOptionLabel,
    buildCharacterInfoText,
    characterCacheEntryIsFresh,
    readStoredLanguage,
    storeLanguage,
  };
});