const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

// One sample of a clip that is running normally: the pipeline is ready, the
// element is not paused, and the clock is at the given point.
function running(at, overrides) {
  return Object.assign({
    at,
    currentTime: at / 1000,
    duration: 22.3,
    readyState: 4,
    networkState: 1,
    volume: 1,
    playbackRate: 1,
    decodedBytes: null,
    paused: false,
    ended: false,
    muted: false,
    seeking: false,
    visibilityState: 'visible',
  }, overrides || {});
}

function verdict(samples, events) {
  return mobileChatHelpers.readPlaybackStall(samples, events);
}

test('an element that ran dry is media-starved', () => {
  // A waiting or stalled event is the element saying it has nothing to play.
  assert.equal(verdict([running(0), running(250)], [{ name: 'waiting' }]), 'media-starved');
  assert.equal(verdict([running(0), running(250)], [{ name: 'stalled' }]), 'media-starved');
});

test('an element that has fallen behind its buffer is media-starved', () => {
  // No event fired, but the element is not ready while it is meant to be
  // playing: it is out of data either way.
  assert.equal(verdict([running(0, { readyState: 2 }), running(250, { readyState: 2 })], []), 'media-starved');
  assert.equal(verdict([running(0), running(250, { readyState: 0 })], []), 'media-starved');
});

test('readiness is only judged while the clip is running', () => {
  // An element that has not started yet has a low readyState for the ordinary
  // reason, and reading that as starvation would blame the network for a clip
  // that simply had not begun.
  assert.equal(verdict([running(0, { paused: true, readyState: 1 }), running(250)], []), null);
  assert.equal(verdict([running(0, { ended: true, readyState: 0 })], []), null);
});

test('a clip that went quiet on the page is output-silent', () => {
  // Nothing in the page sets these, so finding one mid-clip exonerates the
  // network and explains where the words went.
  assert.equal(verdict([running(0), running(250, { muted: true })], []), 'output-silent');
  assert.equal(verdict([running(0), running(250, { volume: 0 })], []), 'output-silent');
});

test('a lower volume is not silence', () => {
  // The page is allowed to turn a clip down; only silence is a verdict.
  assert.notEqual(verdict([running(0), running(250, { volume: 0.2 })], []), 'output-silent');
});

test('a pipeline that reported no trouble is timeline-advanced-unheard', () => {
  // The element held the clip, stayed ready, was not muted and never fired
  // anything -- yet the clock ran on. Nothing in the page can account for the
  // gap, which is what points the search below the element, at the output
  // device and the audio session.
  assert.equal(verdict([running(0), running(250), running(500)], []), 'timeline-advanced-unheard');
});

test('a clock that did not move is not evidence of anything', () => {
  // One sample cannot show the timeline advancing, and a stalled clock with no
  // event is a clip that simply stopped -- there is no verdict for it here.
  assert.equal(verdict([running(0)], []), null);
  assert.equal(verdict([running(0), running(250, { currentTime: 0 })], []), null);
  assert.equal(verdict([], []), null);
});

test('the events outrank the samples', () => {
  // A clip that went quiet and reported starvation is a starvation: the
  // element's own account of why is the stronger one.
  assert.equal(verdict([running(0), running(250, { muted: true })], [{ name: 'stalled' }]), 'media-starved');
});

test('a run of nothing is no verdict, not a crash', () => {
  assert.equal(verdict(null, null), null);
  assert.equal(verdict(undefined, undefined), null);
  assert.equal(verdict([null, undefined], [null]), null);
});

test('the element is read as plain data that cannot throw', () => {
  const snapshot = mobileChatHelpers.snapshotPlayback({
    currentTime: 7.1,
    duration: 22.3,
    readyState: 4,
    volume: 1,
    paused: false,
    ended: false,
  });

  assert.equal(snapshot.currentTime, 7.1);
  assert.equal(snapshot.ended, false);
  // A property the element does not have is null rather than undefined, so the
  // samples stay comparable across browsers.
  assert.equal(snapshot.decodedBytes, null);
  assert.equal(snapshot.networkState, null);
  assert.ok(Number.isFinite(snapshot.at));
});

test('a torn-down element is no snapshot at all', () => {
  assert.equal(mobileChatHelpers.snapshotPlayback(null), null);
  assert.equal(mobileChatHelpers.snapshotPlayback(undefined), null);
});
