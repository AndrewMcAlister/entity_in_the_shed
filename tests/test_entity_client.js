const test = require('node:test');
const assert = require('node:assert/strict');

const helpers = require('../static/app-helpers.js');

const CLIENT_ID = 'a1b2c3d4e5f60718';

function fakeStorage(seed) {
  const data = Object.assign({}, seed || {});
  return {
    getItem(key) {
      return Object.prototype.hasOwnProperty.call(data, key) ? data[key] : null;
    },
    setItem(key, value) {
      data[key] = String(value);
    },
    removeItem(key) {
      delete data[key];
    },
    data,
  };
}

// -- identity ---------------------------------------------------------------

test('a well-formed client id is kept', () => {
  assert.equal(helpers.normalizeClientId(CLIENT_ID), CLIENT_ID);
  assert.equal(helpers.normalizeClientId('  ' + CLIENT_ID + '  '), CLIENT_ID);
});

test('anything the server would reject becomes empty rather than an error', () => {
  // Empty means "send nothing", and the server mints one. Throwing here would
  // be the client refusing a turn the server was happy to answer.
  assert.equal(helpers.normalizeClientId('short'), '');
  assert.equal(helpers.normalizeClientId(''), '');
  assert.equal(helpers.normalizeClientId(null), '');
  assert.equal(helpers.normalizeClientId(undefined), '');
  assert.equal(helpers.normalizeClientId('has space here'), '');
  assert.equal(helpers.normalizeClientId('../../etc/passwd'), '');
  assert.equal(helpers.normalizeClientId('a'.repeat(65)), '');
});

test('a stored id is returned unchanged', () => {
  const storage = fakeStorage({ 'entity-client-id': CLIENT_ID });
  assert.equal(helpers.getOrCreateClientId(storage, () => 'unused00000000'), CLIENT_ID);
});

test('an absent id is minted once and persisted', () => {
  const storage = fakeStorage();
  const first = helpers.getOrCreateClientId(storage, () => CLIENT_ID);

  assert.equal(first, CLIENT_ID);
  assert.equal(storage.data['entity-client-id'], CLIENT_ID);
  // The second call must not mint again -- a new id per call is a new entity.
  assert.equal(helpers.getOrCreateClientId(storage, () => 'different0000000'), CLIENT_ID);
});

test('a stored id the server would reject is replaced, not sent', () => {
  const storage = fakeStorage({ 'entity-client-id': 'garbage!' });
  assert.equal(helpers.getOrCreateClientId(storage, () => CLIENT_ID), CLIENT_ID);
  assert.equal(storage.data['entity-client-id'], CLIENT_ID);
});

test('a mint failure yields empty rather than storing nonsense', () => {
  const storage = fakeStorage();
  assert.equal(helpers.getOrCreateClientId(storage, () => ''), '');
  assert.equal(storage.data['entity-client-id'], undefined);
});

test('a storage that throws does not break the chat payload', () => {
  const hostile = {
    getItem() { throw new Error('blocked'); },
    setItem() { throw new Error('blocked'); },
  };
  assert.equal(helpers.getOrCreateClientId(hostile, () => CLIENT_ID), CLIENT_ID);
});

test('adopting an id persists it, so two mints cannot both win', () => {
  // The startup probe and the first turn can both ask the server to mint an id
  // when storage is empty. Whichever is adopted must be written down at once,
  // or the loser leaves the entity talking on an id nobody reads.
  const storage = fakeStorage();
  assert.equal(helpers.storeClientId(storage, CLIENT_ID), CLIENT_ID);
  assert.equal(storage.data['entity-client-id'], CLIENT_ID);
});

test('adopting a missing or malformed id leaves storage alone', () => {
  const storage = fakeStorage({ 'entity-client-id': CLIENT_ID });
  assert.equal(helpers.storeClientId(storage, ''), '');
  assert.equal(helpers.storeClientId(storage, null), '');
  assert.equal(helpers.storeClientId(storage, 'not valid!'), '');
  // The good id is still there -- a bad answer must not erase a good identity.
  assert.equal(storage.data['entity-client-id'], CLIENT_ID);
});

test('the state url carries the client id when there is one', () => {
  assert.equal(helpers.buildEntityStateUrl(CLIENT_ID), `/api/entity/state?client_id=${CLIENT_ID}`);
  // No id yet is the normal state on a first visit: the server mints one and
  // the answer is adopted.
  assert.equal(helpers.buildEntityStateUrl(''), '/api/entity/state');
});

test('the chat payload carries the client id and no session id', () => {
  // The conversation is stored under the client id sent here, so the page sends
  // no separate identity for the transcript.
  const payload = helpers.buildChatRequestPayload({
    message: 'hi',
    clientId: CLIENT_ID,
  });
  assert.equal(payload.client_id, CLIENT_ID);
  assert.equal(Object.prototype.hasOwnProperty.call(payload, 'session_id'), false);
});

test('the chat payload sends no client id when it has none', () => {
  const payload = helpers.buildChatRequestPayload({ message: 'hi' });
  assert.equal(payload.client_id, '');
});

// -- urls -------------------------------------------------------------------

test('the stream url carries the client id and escapes it', () => {
  assert.equal(
    helpers.buildEntityStreamUrl(CLIENT_ID, 0),
    `/api/entity/stream?client_id=${CLIENT_ID}`,
  );
  assert.equal(helpers.buildEntityStreamUrl('a b', 0), '/api/entity/stream');
});

test('the stream url includes a cursor when it has one', () => {
  const url = helpers.buildEntityStreamUrl(CLIENT_ID, 7);
  assert.ok(url.includes('since=7'));
  assert.ok(!url.includes('session_id='));
});

test('a zero cursor is omitted so the first connect asks for everything', () => {
  assert.ok(!helpers.buildEntityStreamUrl(CLIENT_ID, 0).includes('since='));
  assert.ok(!helpers.buildEntityStreamUrl(CLIENT_ID, undefined).includes('since='));
});

test('the outbox url carries the client id', () => {
  assert.equal(
    helpers.buildOutboxRequestUrl(CLIENT_ID),
    `/api/entity/outbox?client_id=${CLIENT_ID}`,
  );
  assert.equal(helpers.buildOutboxRequestUrl(''), '/api/entity/outbox');
});

// -- message shape ----------------------------------------------------------

test('a REST payload and an SSE frame normalise to the same message', () => {
  const rest = helpers.readOutboxMessages({
    ok: true,
    messages: [{
      seq: 3,
      delivery_id: '3-99',
      kind: 'goal',
      trigger: 'goal',
      text: 'Still thinking about that.',
      character_slug: 'vivian',
      delivery_style: 'normal',
    }],
  });
  const sse = helpers.readOutboxMessages({
    seq: 3,
    delivery_id: '3-99',
    kind: 'goal',
    trigger: 'goal',
    text: 'Still thinking about that.',
    character_slug: 'vivian',
    delivery_style: 'normal',
  });

  assert.deepEqual(sse, rest);
  assert.equal(rest.length, 1);
  assert.equal(rest[0].deliveryId, '3-99');
  assert.equal(rest[0].text, 'Still thinking about that.');
});

test('entries without an id or text are dropped', () => {
  // No id means it cannot be de-duplicated or acknowledged, so rendering it
  // would create a bubble that arrives again on every reconnect.
  const messages = helpers.readOutboxMessages({
    messages: [
      { delivery_id: '1-1', text: 'kept' },
      { delivery_id: '', text: 'no id' },
      { delivery_id: '2-2', text: '   ' },
      { delivery_id: '3-3' },
    ],
  });
  assert.equal(messages.length, 1);
  assert.equal(messages[0].text, 'kept');
});

test('a payload with no messages at all is empty, not a crash', () => {
  assert.deepEqual(helpers.readOutboxMessages(null), []);
  assert.deepEqual(helpers.readOutboxMessages({}), []);
  assert.deepEqual(helpers.readOutboxMessages({ messages: 'nope' }), []);
});

// -- delivery gating --------------------------------------------------------

function delivery(overrides) {
  return Object.assign({
    enabled: true,
    deliveryId: '1-1',
    text: 'Spoke first.',
    seenDeliveryIds: [],
  }, overrides || {});
}

test('a first-time message is delivered', () => {
  assert.equal(helpers.shouldDeliverProactive(delivery()), true);
});

test('an already-seen delivery is not shown twice', () => {
  // At-least-once delivery is the design, so a reconnect legitimately replays
  // anything unacknowledged. Showing it twice is the bug, not the replay.
  assert.equal(
    helpers.shouldDeliverProactive(delivery({ seenDeliveryIds: ['1-1'] })),
    false,
  );
  assert.equal(
    helpers.shouldDeliverProactive(delivery({ seenDeliveryIds: new Set(['1-1']) })),
    false,
  );
});

test('identity is the delivery id, not the text', () => {
  // A deliberately repeated line is a real thing for the entity to say.
  const seen = ['1-1'];
  assert.equal(helpers.shouldDeliverProactive(delivery({ seenDeliveryIds: seen })), false);
  assert.equal(
    helpers.shouldDeliverProactive(delivery({ deliveryId: '2-2', seenDeliveryIds: seen })),
    true,
  );
});

test('nothing is delivered when the feature is off or the message is empty', () => {
  assert.equal(helpers.shouldDeliverProactive(delivery({ enabled: false })), false);
  assert.equal(helpers.shouldDeliverProactive(delivery({ text: '   ' })), false);
  assert.equal(helpers.shouldDeliverProactive(delivery({ deliveryId: '' })), false);
  assert.equal(helpers.shouldDeliverProactive(null), false);
});

test('hasSeenDeliveryId tolerates either a set or an array', () => {
  assert.equal(helpers.hasSeenDeliveryId(['a'], 'a'), true);
  assert.equal(helpers.hasSeenDeliveryId(new Set(['a']), 'a'), true);
  assert.equal(helpers.hasSeenDeliveryId(['a'], 'b'), false);
  assert.equal(helpers.hasSeenDeliveryId(null, 'a'), false);
  assert.equal(helpers.hasSeenDeliveryId(['a'], ''), false);
});

// -- polling ----------------------------------------------------------------

function polling(overrides) {
  return Object.assign({ enabled: true, streamConnected: false, sending: false, hidden: false }, overrides || {});
}

test('the poll runs when it is the only way to be reached', () => {
  assert.equal(helpers.shouldPollOutbox(polling()), true);
});

test('the poll stands down while the stream is up', () => {
  // Two delivery paths at once is how the same line renders twice.
  assert.equal(helpers.shouldPollOutbox(polling({ streamConnected: true })), false);
});

test('the poll stands down during a turn', () => {
  // waitForChatJob owns the socket then, and on a faster interval.
  assert.equal(helpers.shouldPollOutbox(polling({ sending: true })), false);
});

test('a hidden tab does not poll', () => {
  // Nothing is being read, and the outbox is durable -- the next visible poll
  // recovers everything, so waiting costs nothing.
  assert.equal(helpers.shouldPollOutbox(polling({ hidden: true })), false);
});

test('the poll is off when the feature is off', () => {
  assert.equal(helpers.shouldPollOutbox(polling({ enabled: false })), false);
  assert.equal(helpers.shouldPollOutbox(null), false);
});

test('poll backoff doubles and then stops growing', () => {
  assert.equal(helpers.nextOutboxPollDelayMs(0, 2000), 2000);
  assert.equal(helpers.nextOutboxPollDelayMs(1, 2000), 4000);
  assert.equal(helpers.nextOutboxPollDelayMs(3, 2000), 16000);
  // Capped: a server that is down should not be hammered, but recovery must
  // not need a tap from the user either.
  assert.equal(helpers.nextOutboxPollDelayMs(20, 2000), 30000);
  assert.equal(helpers.nextOutboxPollDelayMs(-1, 2000), 2000);
  assert.equal(helpers.nextOutboxPollDelayMs(0, 0), 2000);
});

// -- the "chatterbox" box ---------------------------------------------------

test('the box follows the server, which is the only thing that knows', () => {
  assert.deepEqual(
    helpers.proactiveToggleView({ proactive_enabled: true, proactive_available: true }),
    { checked: true, disabled: false, title: '' },
  );
  assert.deepEqual(
    helpers.proactiveToggleView({ proactive_enabled: false, proactive_available: true }),
    { checked: false, disabled: false, title: '' },
  );
});

test('a master switch turned off disables the box rather than unticking it quietly', () => {
  const view = helpers.proactiveToggleView({
    proactive_enabled: false,
    proactive_available: false,
  });
  assert.equal(view.checked, false);
  assert.equal(view.disabled, true);
  assert.match(view.title, /ENTITY_PROACTIVE_ENABLED/);
});

test('an answer that never mentions the box does not move it', () => {
  // Every turn response from a page cached before the box existed, and the
  // outbox poll, which says nothing about policy. A control moved by an answer
  // that was never about it is a control that flickers.
  assert.equal(helpers.proactiveToggleView({ client_id: CLIENT_ID }), null);
  assert.equal(helpers.proactiveToggleView({}), null);
  assert.equal(helpers.proactiveToggleView(null), null);
  assert.equal(helpers.proactiveToggleView(undefined), null);
});

test('a payload that is not an object at all is not a crash', () => {
  // The view is applied to whatever came back from fetch, and a proxy that
  // answers with an error page hands this a string. Reading the key with `in`
  // would throw on one, and a thrown error here takes the toggle's sync down
  // with it -- including on the paths that had nothing to do with the box.
  assert.equal(helpers.proactiveToggleView('proactive_enabled'), null);
  assert.equal(helpers.proactiveToggleView(200), null);
});

test('an unreported master switch is not a revoked one', () => {
  // Absent availability means "not reported": reading it as false would leave
  // the box dead for the rest of the session after any reply that carried just
  // the enabled flag.
  const view = helpers.proactiveToggleView({ proactive_enabled: true });
  assert.equal(view.checked, true);
  assert.equal(view.disabled, false);
});

test('a non-boolean enabled flag does not tick the box by accident', () => {
  // Naming the key is what makes a reply "about" the switch, so these are not
  // the silent case above: the server spoke about the box and did not grant it,
  // and a box left ticked over a claim never made is the one mistake that makes
  // the entity speak to somebody who did not ask. Unticking errs toward silence.
  assert.equal(helpers.proactiveToggleView({ proactive_enabled: 'true' }).checked, false);
  assert.equal(helpers.proactiveToggleView({ proactive_enabled: 1 }).checked, false);
  assert.equal(helpers.proactiveToggleView({ proactive_enabled: null }).checked, false);
  assert.equal(helpers.proactiveToggleView({ proactive_enabled: undefined }).checked, false);
});

test('availability that arrived mangled leaves the box usable, not dead', () => {
  // An unusable availability is treated as unreported rather than as revoked,
  // for the same reason an absent one is: a control stuck disabled for the rest
  // of the session cannot be recovered from the browser. Enabling it grants
  // nothing on its own -- the master switch in .env still outranks it, and the
  // server answers with enabled false whenever it is off.
  const view = helpers.proactiveToggleView({
    proactive_enabled: true,
    proactive_available: 'false',
  });
  assert.equal(view.disabled, false);
  assert.equal(view.checked, true);
});

test('a reply carrying only availability still shows the box unticked', () => {
  const view = helpers.proactiveToggleView({ proactive_available: true });
  assert.equal(view.checked, false);
  assert.equal(view.disabled, false);
});

// -- saying an unprompted line out loud -------------------------------------

function voiceContext(overrides) {
  return Object.assign(
    {
      enabled: true,
      sending: false,
      speaking: false,
      recording: false,
      handsFree: false,
      hearingSpeech: false,
    },
    overrides || {},
  );
}

test('an unprompted line is spoken when nothing else is happening', () => {
  assert.equal(helpers.shouldVoiceProactive(voiceContext()), true);
});

test('the box unticked means the line is shown and not said', () => {
  assert.equal(helpers.shouldVoiceProactive(voiceContext({ enabled: false })), false);
});

test('a turn in flight keeps the speaker, because the reply will want it', () => {
  assert.equal(helpers.shouldVoiceProactive(voiceContext({ sending: true })), false);
});

test('nothing is said over something already being said', () => {
  // speak() stops whatever is playing before it starts, so a random line
  // arriving mid-reply would cut the reply the user actually asked for.
  assert.equal(helpers.shouldVoiceProactive(voiceContext({ speaking: true })), false);
});

test('a live microphone that is hearing a person is not dictated to', () => {
  assert.equal(helpers.shouldVoiceProactive(voiceContext({ recording: true })), false);
  assert.equal(
    helpers.shouldVoiceProactive(
      voiceContext({ recording: true, handsFree: true, hearingSpeech: true }),
    ),
    false,
  );
});

test('hands-free listening between turns is set down so the line can be heard', () => {
  // The microphone is on between turns in hands-free, but that is ambient
  // listening rather than a person mid-sentence. The page stops it before the
  // line is spoken and re-arms it afterwards; refusing here made staying quiet
  // permanent, because hands-free never turns the microphone off.
  assert.equal(
    helpers.shouldVoiceProactive(
      voiceContext({ recording: true, handsFree: true, hearingSpeech: false }),
    ),
    true,
  );
});

test('a hidden tab is still spoken to, because hands-free listening is the point', () => {
  assert.equal(helpers.shouldVoiceProactive(voiceContext({ hidden: true })), true);
});

test('a missing context refuses rather than throwing', () => {
  assert.equal(helpers.shouldVoiceProactive(), false);
  assert.equal(helpers.shouldVoiceProactive(null), false);
});

// -- what an unprompted line does about the speaker -------------------------

test('an idle page speaks, a busy speaker holds, a silent one only shows', () => {
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext()), 'speak');
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext({ sending: true })), 'hold');
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext({ speaking: true })), 'hold');
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext({ sending: true, speaking: true })), 'hold');
});

test('a turn in flight owns the speaker even while the microphone is on', () => {
  // Hands-free records between turns; that must not talk the line out over a
  // reply. But a turn also owns the speaker when the microphone is hearing a
  // person, because the line would be said after that turn either way.
  assert.equal(
    helpers.proactiveDeliveryPlan(voiceContext({ sending: true, recording: true, handsFree: true })),
    'hold',
  );
  assert.equal(
    helpers.proactiveDeliveryPlan(voiceContext({ sending: true, recording: true })),
    'hold',
  );
});

test('the plan names the same three outcomes the old gate encoded', () => {
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext({ enabled: false })), 'show');
  assert.equal(helpers.proactiveDeliveryPlan(voiceContext({ recording: true })), 'show');
  assert.equal(
    helpers.proactiveDeliveryPlan(voiceContext({ recording: true, handsFree: true, hearingSpeech: true })),
    'show',
  );
  assert.equal(
    helpers.proactiveDeliveryPlan(voiceContext({ recording: true, handsFree: true })),
    'speak',
  );
  assert.equal(helpers.proactiveDeliveryPlan(), 'show');
  assert.equal(helpers.proactiveDeliveryPlan(null), 'show');
});

test('the two predicates are exactly the plan outcomes', () => {
  const cases = [
    voiceContext(),
    voiceContext({ enabled: false }),
    voiceContext({ sending: true }),
    voiceContext({ speaking: true }),
    voiceContext({ sending: true, speaking: true }),
    voiceContext({ recording: true }),
    voiceContext({ recording: true, handsFree: true }),
    voiceContext({ recording: true, handsFree: true, hearingSpeech: true }),
    voiceContext({ hidden: true }),
    {},
    null,
  ];
  cases.forEach((context) => {
    const plan = helpers.proactiveDeliveryPlan(context);
    assert.equal(helpers.shouldVoiceProactive(context), plan === 'speak');
    assert.equal(helpers.shouldHoldProactive(context), plan === 'hold');
  });
});

test('shouldHoldProactive is true only for a busy speaker', () => {
  assert.equal(helpers.shouldHoldProactive(voiceContext({ sending: true })), true);
  assert.equal(helpers.shouldHoldProactive(voiceContext({ speaking: true })), true);
  assert.equal(helpers.shouldHoldProactive(voiceContext()), false);
  assert.equal(helpers.shouldHoldProactive(voiceContext({ recording: true })), false);
  assert.equal(helpers.shouldHoldProactive(voiceContext({ enabled: false })), false);
  assert.equal(helpers.shouldHoldProactive(), false);
});
