const test = require('node:test');
const assert = require('node:assert/strict');

const mobileChatHelpers = require('../static/app-helpers.js');

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function createHarness(options) {
  const settings = options || {};
  const events = [];
  const synthesized = [];
  const played = [];
  const playAttempts = [];

  // Every synthesis returns a fresh deferred unless the test says otherwise,
  // so a test can hold segment 2 in flight while segment 1 is playing.
  const queue = mobileChatHelpers.createSpeechQueue({
    lookahead: settings.lookahead === undefined ? 2 : settings.lookahead,
    gate: settings.gate === undefined ? null : settings.gate,
    // A retry is instant unless the test is about the wait itself.
    retryDelayMs: settings.retryDelayMs === undefined ? 0 : settings.retryDelayMs,
    synthesize(segment) {
      events.push(`synth:start:${segment.text}`);
      const gate = deferred();
      synthesized.push({ segment, gate });
      if (typeof settings.resolveSynthesis === 'function') {
        settings.resolveSynthesis(segment, gate);
      } else {
        gate.resolve({ audio: segment.text });
      }
      return gate.promise;
    },
    play(audio, segment, attempt) {
      // Attempt numbers are recorded for every call, but `played` and `events`
      // mention a segment once: they describe what the listener heard, and a
      // retried clip is still that one segment.
      const which = attempt || 0;
      playAttempts.push(`${segment.text}#${which}`);
      if (which === 0) {
        events.push(`play:start:${segment.text}`);
        played.push(audio.audio);
      }
      const gate = deferred();
      if (typeof settings.resolvePlay === 'function') {
        settings.resolvePlay(audio, segment, gate);
      } else {
        gate.resolve();
      }
      return gate.promise.then(() => {
        events.push(`play:end:${segment.text}`);
      });
    },
  });

  return { queue, events, synthesized, played, playAttempts };
}

async function flush() {
  // Let queued microtasks and the queue's worker loop advance.
  for (let index = 0; index < 10; index += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

test('segments play back strictly in order', async () => {
  const { queue, played } = createHarness();
  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  const result = await queue.drain();

  assert.deepEqual(played, ['one', 'two', 'three']);
  assert.equal(result.played, 3);
  assert.equal(result.errors.length, 0);
  assert.equal(result.firstError, null);
});

test('synthesis of the next segment starts before the current one finishes playing', async () => {
  const playGates = [];
  const { queue, events, synthesized, played } = createHarness({
    // Hold every playback open so the interleaving is observable.
    resolvePlay(audio, segment, gate) {
      playGates.push(gate);
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  await flush();

  // 'one' is still playing, yet 'two' is already synthesized -- this is the
  // whole point of the queue, and the gap it removes between sentences.
  assert.deepEqual(events, ['synth:start:one', 'synth:start:two', 'play:start:one']);
  assert.deepEqual(synthesized.map((entry) => entry.segment.text), ['one', 'two']);
  assert.equal(playGates.length, 1);

  playGates[0].resolve();
  await flush();
  assert.equal(playGates.length, 2);

  playGates[1].resolve();
  const result = await queue.drain();

  assert.deepEqual(played, ['one', 'two']);
  assert.equal(result.played, 2);
  assert.deepEqual(events, [
    'synth:start:one',
    'synth:start:two',
    'play:start:one',
    'play:end:one',
    'play:start:two',
    'play:end:two',
  ]);
});

test('lookahead bounds how far ahead synthesis runs', async () => {
  const playGates = [];
  const { queue, synthesized } = createHarness({
    lookahead: 1,
    resolvePlay(audio, segment, gate) {
      playGates.push(gate);
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  await flush();

  // With a lookahead of one, only the segment being played is in flight.
  assert.deepEqual(synthesized.map((entry) => entry.segment.text), ['one']);

  queue.clear();
  playGates.forEach((gate) => gate.resolve());
});

test('a failed segment is skipped and the rest of the reply still plays', async () => {
  const { queue, played } = createHarness({
    resolveSynthesis(segment, gate) {
      if (segment.text === 'two') {
        gate.reject(new Error('synthesis failed'));
        return;
      }
      gate.resolve({ audio: segment.text });
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  const result = await queue.drain();

  assert.deepEqual(played, ['one', 'three']);
  assert.equal(result.errors.length, 1);
  // 'one' played, so this is not a total failure and must not trigger a fallback.
  assert.equal(result.firstError, null);
});

test('a failure before anything plays is reported so the caller can fall back', async () => {
  const { queue } = createHarness({
    resolveSynthesis(segment, gate) {
      gate.reject(new Error('synthesis failed'));
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.equal(result.played, 0);
  assert.ok(result.firstError);
  assert.equal(result.firstError.message, 'synthesis failed');
});

test('a segment cut off mid-clip counts as heard, so the reply is not said again', async () => {
  // The browser pauses the clip when the audio session moves. The user heard
  // part of it; calling that "nothing was heard" is what makes the turn say the
  // whole reply over again, which is the reply read out twice.
  const { queue, played } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (segment.text === 'one') {
        gate.reject(
          mobileChatHelpers.speechInterruptionError({ currentTime: 7.1, duration: 22.3 })
        );
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.deepEqual(played, ['one', 'two']);
  assert.equal(result.played, 2);
  // Nothing to fall back for: a fallback here is the same reply spoken twice.
  assert.equal(result.firstError, null);
  // The user is still told the piece went missing.
  assert.equal(
    mobileChatHelpers.speechPartialFailure(result),
    'part of the reply could not be spoken: playback stopped after 7.1s of 22.3s'
  );
});

test('a clip that failed to play gets a second attempt rather than being dropped', async () => {
  // A failed play used to cost the segment outright: the error was recorded and
  // the queue moved on, so the sentence was never heard and the only trace of it
  // was a line of status text.
  const attempts = [];
  const { queue, played, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      attempts.push(segment.text);
      if (attempts.length === 1) {
        gate.reject(new Error('Audio playback failed'));
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.deepEqual(attempts, ['one', 'one', 'two']);
  assert.deepEqual(playAttempts, ['one#0', 'one#1', 'two#0']);
  assert.deepEqual(played, ['one', 'two']);
  assert.equal(result.played, 2);
  // Heard on the retry, so nothing went missing and there is nothing to report.
  assert.deepEqual(result.errors, []);
  assert.equal(result.firstError, null);
  assert.equal(mobileChatHelpers.speechPartialFailure(result), null);
});

test('an interrupted clip is retried as a resume, so the words after the cut are heard', async () => {
  // The retry is the repair for the tail an interruption loses. The queue asks
  // for it by attempt number, and the player reads anything past the first
  // attempt as "carry on from where you stopped" rather than "start again".
  const { queue, played, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (playAttempts.length === 1) {
        gate.reject(
          mobileChatHelpers.speechInterruptionError({ currentTime: 7.1, duration: 22.3 })
        );
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.deepEqual(playAttempts, ['one#0', 'one#1']);
  assert.deepEqual(played, ['one']);
  assert.equal(result.played, 1);
  assert.equal(result.firstError, null);
  // Resumed and heard, so no hole is left to own up to.
  assert.deepEqual(result.errors, []);
  assert.equal(mobileChatHelpers.speechPartialFailure(result), null);
});

test('the retry waits for the audio session to settle before it tries again', async () => {
  // A retry fired into the same hiccup fails the same way; the beat before it is
  // what gives the second attempt a chance to land.
  const stamps = [];
  const { queue } = createHarness({
    retryDelayMs: 40,
    resolvePlay(audio, segment, gate) {
      stamps.push(Date.now());
      if (stamps.length === 1) {
        gate.reject(new Error('Audio playback failed'));
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.equal(result.played, 1);
  assert.equal(stamps.length, 2);
  assert.ok(
    stamps[1] - stamps[0] >= 30,
    `the retry came ${stamps[1] - stamps[0]}ms after the failure`
  );
});

test('an aborted clip ends playback instead of being retried', async () => {
  // An abort is the turn being torn down on purpose. Retrying it would talk over
  // the user, and the wait before a retry would hold the turn open after they
  // asked for silence.
  let attempts = 0;
  const { queue } = createHarness({
    retryDelayMs: 0,
    resolvePlay(audio, segment, gate) {
      attempts += 1;
      const abort = new Error('aborted');
      abort.name = 'AbortError';
      gate.reject(abort);
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.equal(attempts, 1);
  assert.equal(result.reason, 'playback');
  assert.equal(result.firstError, null);
});

test('a retry resumes only the clip the element still holds part-way through', () => {
  // Resuming is what keeps the retry from repeating the opening the user has
  // already heard. Everything else has to be given the audio again.
  const canResume = mobileChatHelpers.canResumeClip;
  const clip = 'blob:http://127.0.0.1:5005/abc';
  const held = { src: clip, currentTime: 7.1, ended: false };

  assert.equal(canResume(held, clip), true);
  // Nothing was heard yet, so there is nothing to carry on from.
  assert.equal(canResume({ src: clip, currentTime: 0, ended: false }, clip), false);
  // A clip that reached its end is finished, not interrupted.
  assert.equal(canResume({ src: clip, currentTime: 22.3, ended: true }, clip), false);
  // A different clip, an element torn down under the last one, and a caller with
  // no clip at all: each has to start from the top.
  assert.equal(
    canResume({ src: 'blob:http://127.0.0.1:5005/other', currentTime: 7.1, ended: false }, clip),
    false
  );
  assert.equal(canResume({ src: '', currentTime: 7.1, ended: false }, clip), false);
  assert.equal(canResume(null, clip), false);
  assert.equal(canResume(held, ''), false);
});

test('the end of a clip is told apart from an interruption', () => {
  // A clip that reaches its end fires a pause on the way out -- before the
  // ended, in Edge -- so the pause is the moment the two cases look identical.
  // Read as an interruption, a finished clip is recorded as a failure and the
  // reply is said again. Two things separate them, and either is enough: the
  // element saying it ended, and the element sitting on its last sample.
  const finished = mobileChatHelpers.clipFinished;

  assert.equal(finished({ ended: true, currentTime: 19.66, duration: 19.66 }), true);
  // The browsers that deliver the pause before they have set `ended`: the
  // position has to answer for them, or the fix holds only in Edge -- and it is
  // the user's own browser that is being fixed.
  assert.equal(finished({ ended: false, currentTime: 19.66, duration: 19.66 }), true);
  assert.equal(finished({ ended: false, currentTime: 19.63, duration: 19.66 }), true);
  // Stopped part-way: paused, not ended, with audible clip still ahead of it.
  assert.equal(finished({ ended: false, currentTime: 7.1, duration: 22.3 }), false);
  assert.equal(finished({ ended: false, currentTime: 19.5, duration: 19.66 }), false);
  assert.equal(finished({ ended: false, currentTime: 0 }), false);
  // A torn-down element, a stream with no known end, and a caller with no
  // element at all.
  assert.equal(finished({ ended: false, currentTime: 7.1, duration: NaN }), false);
  assert.equal(finished({ ended: false, currentTime: 7.1, duration: Infinity }), false);
  assert.equal(finished({ ended: false }), false);
  assert.equal(finished(null), false);
  assert.equal(finished(undefined), false);
});

test('a resumed clip starts just before where the interruption stopped it', () => {
  // The rule that makes the second attempt safe. Winding back by less than a
  // word is what the retry costs: the word the pause landed inside is said
  // again, and in exchange no sentence is lost. Only a resume without this
  // would be the replay-from-the-top this policy is replacing.
  const resume = mobileChatHelpers.resumePosition;
  const overlap = mobileChatHelpers.SPEECH_RESUME_OVERLAP_SECONDS;

  // Half a second is under a spoken word, so the seam can repeat a word at most.
  assert.ok(overlap > 0 && overlap < 1, 'the overlap must be under a word');

  assert.equal(resume({ currentTime: 7.1, duration: 22.3 }, overlap), 7.1 - overlap);

  // Stopped inside the overlap: there is nowhere before the clip to wind back
  // to, so it starts at the top rather than at a negative position.
  assert.equal(resume({ currentTime: 0.2, duration: 22.3 }, overlap), 0);

  // At the very start, and a torn-down element: the top is the answer that
  // loses nothing, which is what this whole policy is buying.
  assert.equal(resume({ currentTime: 0, duration: 22.3 }, overlap), 0);
  assert.equal(resume(null, overlap), 0);
  assert.equal(resume(undefined, overlap), 0);

  // A wound position closer to the end than the clamp allows is held there.
  assert.equal(resume({ currentTime: 22.3, duration: 22.3 }, overlap), Math.min(22.3 - overlap, 22.2));
  // A position past the end seeks nowhere, so it comes back to the last tenth
  // of the clip rather than to a time it does not have.
  assert.equal(resume({ currentTime: 40, duration: 22.3 }, overlap), 22.2);

  // A stream with no known end still resumes from the wound position; the
  // clamp is only there for a clip that does know where it ends.
  assert.equal(resume({ currentTime: 7.1, duration: NaN }, overlap), 7.1 - overlap);
  assert.equal(resume({ currentTime: 7.1 }, overlap), 7.1 - overlap);

  // A duration that has not been read yet, and a nonsense one.
  assert.equal(resume({ currentTime: 7.1, duration: 0 }, overlap), 7.1 - overlap);
  assert.equal(resume({ currentTime: 7.1, duration: Infinity }, overlap), 7.1 - overlap);
});

test('a retry seeks only into a clip the element still holds part-way through', () => {
  // A retry is a resume only while the element still holds the clip. When it
  // does not, the clip is handed over fresh -- and seeking that fresh clip to
  // where the last one stopped starts it past its own ending, so the segment is
  // over before it is heard and its words are never said at all. That is the
  // reply that arrives half spoken.
  const seek = mobileChatHelpers.resumeSeekPosition;
  const clip = 'blob:http://127.0.0.1:5005/abc';
  const held = { src: clip, currentTime: 7.1, ended: false };

  assert.equal(seek(6.7, held, clip), 6.7);

  // The clip reached its end: the position on offer is its own last tenth, and
  // seeking a fresh copy there would say nothing audible.
  assert.equal(seek(19.56, { src: clip, currentTime: 19.66, ended: true }, clip), null);
  // The element holds a different clip, so its position belongs to that one.
  assert.equal(
    seek(6.7, { src: 'blob:http://127.0.0.1:5005/other', currentTime: 7.1 }, clip),
    null
  );
  // A torn-down element, and a caller with no element or no clip at all.
  assert.equal(seek(6.7, { src: '', currentTime: 7.1 }, clip), null);
  assert.equal(seek(6.7, null, clip), null);
  assert.equal(seek(6.7, held, ''), null);

  // No resume was asked for: the first attempt plays from the top.
  assert.equal(seek(null, held, clip), null);
  assert.equal(seek(undefined, held, clip), null);

  // A position that is not a number is not a position.
  assert.equal(seek(NaN, held, clip), null);
  assert.equal(seek(Infinity, held, clip), null);
});

test('a clip interrupted part-way is played again from where it stopped', async () => {
  // The reported bug: a clip that broke part-way through was abandoned, the
  // queue moved on, and the rest of that sentence was never said. Replaying it
  // from its opening used to be the thing worth avoiding; now the player winds
  // the position back first, so the retry can only repeat a word.
  const { queue, played, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (playAttempts.length === 1) {
        const interrupted = new Error('Audio playback was interrupted');
        interrupted.name = 'SpeechInterruptedError';
        interrupted.heard = true;
        interrupted.resumable = true;
        gate.reject(interrupted);
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.deepEqual(playAttempts, ['one#0', 'one#1'], 'a clip that broke part-way is given the audio again');
  assert.deepEqual(played, ['one']);
  assert.equal(result.played, 1);
  // It recovered, so there is nothing to own up to and no message to show.
  assert.deepEqual(result.errors, []);
  assert.equal(result.firstError, null);
  assert.equal(mobileChatHelpers.speechPartialFailure(result), null);
});

test('a clip heard part-way that fails its retry too is owned up to', async () => {
  // The other side of the same rule: when the second attempt fails as well,
  // the audio the user did not get is a hole to report -- and because part of
  // the reply reached the ears, the fallback that says the whole reply again
  // is still not earned.
  const { queue, played, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      const broken = new Error('Audio playback failed');
      broken.heard = true;
      broken.resumable = true;
      gate.reject(broken);
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.deepEqual(playAttempts, ['one#0', 'one#1']);
  assert.deepEqual(played, ['one']);
  assert.equal(result.played, 1);
  assert.equal(result.firstError, null);
  assert.equal(result.errors.length, 1);
  assert.equal(result.errors[0].message, 'Audio playback failed');
  assert.equal(
    mobileChatHelpers.speechPartialFailure(result),
    'part of the reply could not be spoken: Audio playback failed'
  );
});

test('a clip that never started playing is given the audio again', async () => {
  // The other side of the rule: a clip the element never began has not been
  // heard, so the second attempt is its first and no words are said twice.
  const { queue, played, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (playAttempts.length === 1) {
        const blocked = new Error('play() failed');
        blocked.heard = false;
        blocked.resumable = false;
        gate.reject(blocked);
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.deepEqual(playAttempts, ['one#0', 'one#1']);
  assert.deepEqual(played, ['one']);
  assert.equal(result.played, 1);
  assert.deepEqual(result.errors, []);
});

test('a retry that fails still counts the audio the attempt before it played', async () => {
  // The retry reports on itself, not on the clip: a resumed attempt that breaks
  // again must not erase the fact that the first attempt reached the ears, or
  // the turn would say the whole reply over the top of what was heard.
  const { queue, playAttempts } = createHarness({
    resolvePlay(audio, segment, gate) {
      const error = new Error('Audio playback failed');
      if (playAttempts.length === 1) {
        // Heard, and resumable, so the retry is allowed -- and then fails too.
        error.heard = true;
        error.resumable = true;
      } else {
        // The resumed attempt never got going.
        error.heard = false;
        error.resumable = false;
      }
      gate.reject(error);
    },
  });

  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.deepEqual(playAttempts, ['one#0', 'one#1']);
  assert.equal(result.played, 1);
  assert.equal(result.firstError, null);
});

test('an aborted segment ends playback without escalating an error', async () => {
  const { queue } = createHarness({
    resolveSynthesis(segment, gate) {
      if (segment.text === 'one') {
        const abort = new Error('aborted');
        abort.name = 'AbortError';
        gate.reject(abort);
        return;
      }
      gate.resolve({ audio: segment.text });
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.equal(result.played, 0);
  // Told apart from the user's own silence: this one was the pipeline's doing,
  // and the segment it stopped on is owed.
  assert.equal(result.reason, 'synthesis');
  assert.equal(result.firstError, null);
});

test('clear() stops playback and reports the user as the reason', async () => {
  const { queue, played } = createHarness();
  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const draining = queue.drain();
  queue.clear();

  const result = await draining;

  assert.equal(result.reason, 'user');
  assert.equal(played.length, 0);
  assert.equal(queue.cancelled, true);
});

test('a drained queue accepts later segments, which is how streaming arrives', async () => {
  const { queue, played } = createHarness();

  queue.enqueue({ text: 'one' });
  const first = await queue.drain();
  assert.equal(first.played, 1);

  // Sentences stream in after the queue has already fallen idle.
  queue.enqueue({ text: 'two' });
  const second = await queue.drain();

  assert.deepEqual(played, ['one', 'two']);
  assert.equal(second.played, 2);
});

test('enqueue after clear() is recorded rather than silently dropped', async () => {
  // The loop will not run again, so this segment is lost either way -- but a
  // segment that vanishes without a record is one no caller can put right, and
  // the sentence simply never arrives. Recording it is also what the drain has
  // to settle on rather than wait for a loop that has already stopped.
  const { queue, played, synthesized } = createHarness();
  queue.clear();
  queue.enqueue({ text: 'one' });

  const result = await queue.drain();

  assert.equal(result.reason, 'user');
  assert.equal(synthesized.length, 0);
  assert.deepEqual(played, []);
  assert.deepEqual(result.unheard, [
    { text: 'one', cause: 'dropped', message: '', heardSome: false },
  ]);
});

test('a drain says which segments were heard and which were not', async () => {
  // Nothing kept this fact before. A reply could go quiet half way through and
  // leave no record of which words had gone missing, so nothing could put them
  // right and nothing could even say they had gone.
  const { queue } = createHarness({
    resolveSynthesis(segment, gate) {
      if (segment.text === 'two') {
        gate.reject(new Error('synthesis failed'));
        return;
      }
      gate.resolve({ audio: segment.text });
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  const result = await queue.drain();

  assert.equal(result.total, 3);
  assert.equal(result.played, 2);
  assert.equal(result.reason, null);
  assert.deepEqual(result.unheard, [
    { text: 'two', cause: 'synthesis', message: 'synthesis failed', heardSome: false },
  ]);
  // The words the ears are still owed, which is what the caller says next.
  assert.equal(mobileChatHelpers.speechRecoveryText(result), 'two');
});

test('a drain that heard everything owes nothing', async () => {
  const { queue } = createHarness();
  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.equal(result.unheard.length, 0);
  assert.equal(mobileChatHelpers.speechRecoveryText(result), '');
  assert.equal(mobileChatHelpers.speechRecoveryText(null), '');
});

test('a drain that heard nothing at all owes the whole reply', async () => {
  // There is nothing to name from the segments here, and the reply it is
  // repairing still carries the tone markup that chose its delivery -- so the
  // whole reply is what gets said, not the bare words of its sentences.
  const { queue } = createHarness({
    resolveSynthesis(segment, gate) {
      gate.reject(new Error('synthesis failed'));
    },
  });
  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.equal(result.played, 0);
  assert.equal(result.unheard.length, 2, 'both segments went unheard');
  assert.equal(
    mobileChatHelpers.speechRecoveryText(result, '{alluring}one and two{/alluring}'),
    '{alluring}one and two{/alluring}'
  );
});

test('a drain that heard nothing at all is still not repaired on an interrupt', async () => {
  // The interrupt clause outranks the whole-reply reading: the user asked for
  // silence, and the reply is not said afterwards.
  const { queue } = createHarness();
  queue.enqueue({ text: 'one' });
  queue.clear();

  const result = await queue.drain();

  assert.equal(result.played, 0);
  assert.equal(mobileChatHelpers.speechRecoveryText(result, 'the whole reply'), '');
});

test('segments the loop never reached are owed, not lost', async () => {
  // The reported fault: playback stopped part way, the queue wound down, and
  // the sentences behind it were never said and never mentioned. Whatever the
  // loop did not reach is exactly what is still owed.
  const { queue } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (segment.text === 'two') {
        const abort = new Error('aborted');
        abort.name = 'AbortError';
        gate.reject(abort);
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  const result = await queue.drain();

  assert.equal(result.reason, 'playback');
  assert.equal(result.played, 1);
  assert.deepEqual(
    result.unheard.map((entry) => [entry.text, entry.cause]),
    [['two', 'not-attempted'], ['three', 'not-attempted']],
  );
  assert.equal(mobileChatHelpers.speechRecoveryText(result), 'two three');
});

test('a clip cut off part-way is repaired from its own words', async () => {
  // Heard in part and then lost: the opening reached the ears, the tail did
  // not, and after both attempts failed the tail is the part worth saying. The
  // repeated word that costs is the trade the queue already makes when it
  // resumes a clip rather than replaying it.
  const { queue } = createHarness({
    resolvePlay(audio, segment, gate) {
      if (segment.text === 'one') {
        gate.reject(
          mobileChatHelpers.speechInterruptionError({ currentTime: 7.1, duration: 22.3 })
        );
        return;
      }
      gate.resolve();
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  const result = await queue.drain();

  assert.equal(result.played, 2);
  assert.deepEqual(result.unheard, [
    {
      text: 'one',
      cause: 'interrupted',
      message: 'playback stopped after 7.1s of 22.3s',
      heardSome: true,
    },
  ]);
  assert.equal(mobileChatHelpers.speechRecoveryText(result), 'one');
});

test('an interrupt the user asked for is not a hole to repair', async () => {
  // The one clause standing between the repair and talking over someone who
  // pressed Stop. Everything else about this drain says a segment went unheard.
  const gates = [];
  const { queue, played } = createHarness({
    resolvePlay(audio, segment, gate) {
      gates.push(gate);
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  await flush();
  gates[0].resolve();
  await flush();

  queue.clear();
  const result = await queue.drain();

  assert.equal(result.reason, 'user');
  // Only the first reached its end; the second was cut off before it finished.
  assert.equal(result.played, 1);
  assert.equal(result.unheard.length, 1, 'the second segment is still owed');
  assert.equal(mobileChatHelpers.speechRecoveryText(result), '');
  assert.equal(mobileChatHelpers.speechPartialFailure(result), null);
});

test("the user's reason outranks the abort it causes", async () => {
  // An interrupt aborts the clip that is playing, and that abort arrives back
  // at the loop after the interrupt was recorded. Read the other way round, the
  // user's silence would be filed as a playback failure -- and a failure is
  // exactly what earns a repair.
  const gates = [];
  const { queue } = createHarness({
    resolvePlay(audio, segment, gate) {
      gates.push(gate);
    },
  });

  queue.enqueue({ text: 'one' });
  await flush();

  queue.clear();
  const abort = new Error('aborted');
  abort.name = 'AbortError';
  gates[0].reject(abort);

  const result = await queue.drain();

  assert.equal(result.reason, 'user');
  assert.equal(mobileChatHelpers.speechRecoveryText(result), '');
});

test('a speech request is asked for again only when asking again could help', () => {
  // A dropped connection and a server that is briefly unwell are both worth a
  // second ask. A refusal, a missing clip and a malformed request are not: the
  // same ask gets the same answer, and the reply waits behind it for nothing.
  const retry = mobileChatHelpers.shouldRetrySpeechRequest;

  // A fetch that never happened: no status at all.
  assert.equal(retry(null, 0), true);
  assert.equal(retry(undefined, 0), true);
  // The server said it could not, not that it would not.
  assert.equal(retry(500, 0), true);
  assert.equal(retry(503, 0), true);
  // Out of time, and told to come back later.
  assert.equal(retry(408, 0), true);
  assert.equal(retry(429, 0), true);

  // A refusal, a clip that is not there, and a request the server could not read.
  assert.equal(retry(400, 0), false);
  assert.equal(retry(401, 0), false);
  assert.equal(retry(404, 0), false);
  assert.equal(retry(200, 0), false);

  // One attempt is all there is, whatever the first one said.
  assert.equal(retry(500, 1), false);
  assert.equal(retry(429, 1), false);
  assert.equal(retry(null, 1), false);
});

test('createSpeechQueue requires both callbacks', () => {
  assert.throws(() => mobileChatHelpers.createSpeechQueue({}), /requires synthesize and play/);
  assert.throws(
    () => mobileChatHelpers.createSpeechQueue({ synthesize: () => {} }),
    /requires synthesize and play/,
  );
});

test('shouldResumeListening only re-arms the mic when nothing blocks it', () => {
  const allowed = {
    handsFree: true,
    recognitionAvailable: true,
    sending: false,
    speaking: false,
    userStopped: false,
    fatalError: false,
  };

  assert.equal(mobileChatHelpers.shouldResumeListening(allowed), true);
  // No context at all is the same as hands-free off.
  assert.equal(mobileChatHelpers.shouldResumeListening(), false);
  assert.equal(mobileChatHelpers.shouldResumeListening({}), false);

  ['handsFree', 'recognitionAvailable'].forEach((key) => {
    assert.equal(
      mobileChatHelpers.shouldResumeListening({ ...allowed, [key]: false }),
      false,
      `${key} = false must keep the mic off`,
    );
  });

  ['sending', 'speaking', 'userStopped', 'fatalError'].forEach((key) => {
    assert.equal(
      mobileChatHelpers.shouldResumeListening({ ...allowed, [key]: true }),
      false,
      `${key} = true must keep the mic off`,
    );
  });
});

test('hands-free never parks behind a tap after a session hears nothing', () => {
  // The recognizer ends a session of its own accord after every pause, so a
  // page that parks there turns each silence into a manual restart -- which is
  // the thing hands-free exists to remove.
  const pause = mobileChatHelpers.shouldPauseAfterNoSpeech;

  assert.equal(pause({ handsFree: true }), false);
  assert.equal(pause({ handsFree: false }), true);
  // Nothing said about hands-free is not hands-free: park, as before.
  assert.equal(pause(), true);
  assert.equal(pause({}), true);
});

test('voice mode is only advertised when the turn came from the microphone', () => {
  const typed = mobileChatHelpers.buildChatRequestPayload({ message: 'hello' });
  assert.equal(typed.voice_mode, false);

  const spoken = mobileChatHelpers.buildChatRequestPayload({ message: 'hello', voiceMode: true });
  assert.equal(spoken.voice_mode, true);
});

test('readNewSentences only returns what has not been spoken yet', () => {
  const entries = [
    { index: 0, text: 'First sentence.' },
    { index: 1, text: 'Second sentence.' },
  ];

  const first = mobileChatHelpers.readNewSentences(entries, 0);
  assert.deepEqual(first.texts, ['First sentence.', 'Second sentence.']);
  assert.equal(first.consumed, 2);

  // The whole list arrives again on the next poll; only the new one is spoken.
  const grown = entries.concat([{ index: 2, text: 'Third sentence.' }]);
  const second = mobileChatHelpers.readNewSentences(grown, first.consumed);
  assert.deepEqual(second.texts, ['Third sentence.']);
  assert.equal(second.consumed, 3);

  const nothingNew = mobileChatHelpers.readNewSentences(grown, second.consumed);
  assert.deepEqual(nothingNew.texts, []);
  assert.equal(nothingNew.consumed, 3);
});

test('readNewSentences copes with a cursor past the end and with junk', () => {
  const entries = [{ index: 0, text: 'Only one.' }];

  const behind = mobileChatHelpers.readNewSentences(entries, 99);
  assert.deepEqual(behind.texts, [], 'a cursor past the end must not respeak anything');
  assert.equal(behind.consumed, 1);

  assert.deepEqual(mobileChatHelpers.readNewSentences(null, 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSentences(undefined, 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSentences('nonsense', 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSentences(entries, 'nonsense').texts, ['Only one.']);
  assert.deepEqual(mobileChatHelpers.readNewSentences(entries, -5).texts, ['Only one.']);

  // Blank entries are never worth a synthesis request.
  const blanks = [{ index: 0, text: '   ' }, { index: 1 }, null, { index: 2, text: 'Real one.' }];
  assert.deepEqual(mobileChatHelpers.readNewSentences(blanks, 0).texts, ['Real one.']);
});

test('readNewSegments reports genuinely separate beats', () => {
  const split = mobileChatHelpers.readNewSegments(['First beat.', 'Second beat.'], 0);
  assert.deepEqual(split.texts, ['First beat.', 'Second beat.']);
  assert.equal(split.consumed, 2);

  // A repeated delivery must not draw the bubbles twice.
  assert.deepEqual(mobileChatHelpers.readNewSegments(['First beat.', 'Second beat.'], 2).texts, []);
});

test('readNewSegments refuses to call one beat a split', () => {
  // The unchanged one-bubble behaviour. A list of one reaching the renderer as
  // a split would replace every ordinary reply with a single-beat bubble path
  // that only looks the same by accident.
  assert.deepEqual(mobileChatHelpers.readNewSegments(['Only this.'], 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSegments([], 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSegments(null, 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSegments(undefined, 0).texts, []);
  assert.deepEqual(mobileChatHelpers.readNewSegments('nonsense', 0).texts, []);
});

test('readNewSegments drops blanks, which could otherwise empty a bubble', () => {
  const mixed = mobileChatHelpers.readNewSegments(['Real beat.', '   ', null, 'Also real.'], 0);
  assert.deepEqual(mixed.texts, ['Real beat.', 'Also real.']);

  // Two blanks and one real beat is one beat, so it is not a split at all.
  const oneReal = mobileChatHelpers.readNewSegments(['Real beat.', '  ', null], 0);
  assert.deepEqual(oneReal.texts, []);
});

test('splitReplyAfterStream returns only the part this page has not spoken', () => {
  const reply = 'The séance has begun. A hush spreads over the room.';

  assert.deepEqual(
    mobileChatHelpers.splitReplyAfterStream(reply, 'The séance has begun.'),
    { remaining: 'A hush spreads over the room.', replaced: false },
  );

  // Everything the page enqueued is everything there was, so nothing is left.
  assert.deepEqual(
    mobileChatHelpers.splitReplyAfterStream(reply, reply),
    { remaining: '', replaced: false },
  );
});

test('splitReplyAfterStream keeps a sentence published as the turn ended', () => {
  // The report this guards: a reply whose last sentence was never heard while
  // the transcript showed it. The server counts a sentence as spoken when it
  // publishes it, and the last one is released as the turn ends -- after the
  // final poll that could have carried it. So the server's own remainder is
  // empty while this page has said nothing of that sentence.
  const reply = 'The veil shivers, a curtain of mist swirling as the spirits stir. '
    + 'The séance has begun. You feel the faint pulse of unseen voices.';
  const spoken = 'The veil shivers, a curtain of mist swirling as the spirits stir. The séance has begun.';

  const split = mobileChatHelpers.splitReplyAfterStream(reply, spoken);

  assert.equal(split.remaining, 'You feel the faint pulse of unseen voices.');
  assert.equal(split.replaced, false);
});

test('splitReplyAfterStream leaves the whole reply when nothing was spoken', () => {
  const reply = 'Nothing of this has been said yet.';

  assert.deepEqual(
    mobileChatHelpers.splitReplyAfterStream(reply, ''),
    { remaining: reply, replaced: false },
  );
  // And that is not a rewrite: nothing was said, so nothing disagrees.
  assert.equal(mobileChatHelpers.splitReplyAfterStream(reply, '   ').replaced, false);
});

test('splitReplyAfterStream ignores the whitespace a splitter strips', () => {
  // Each sentence is stripped as it is published, so runs of spaces and
  // newlines legitimately differ between what was said and the reply.
  const split = mobileChatHelpers.splitReplyAfterStream(
    'One two three.   Four five six.\nSeven eight nine.',
    'One two three. Four five six.',
  );

  assert.equal(split.remaining, 'Seven eight nine.');
  assert.equal(split.replaced, false);
});

test('splitReplyAfterStream reports a reply that is not what was said', () => {
  // A mismatch must be owned rather than guessed at: the caller keeps the
  // server's answer instead of speaking a tail that does not follow from what
  // the user heard.
  const split = mobileChatHelpers.splitReplyAfterStream(
    'Something entirely different was said instead.',
    'The beginning the user actually heard.',
  );

  assert.equal(split.replaced, true);
  assert.equal(split.remaining, 'Something entirely different was said instead.');
});

test('splitReplyAfterStream tolerates junk where a reply should be', () => {
  assert.deepEqual(
    mobileChatHelpers.splitReplyAfterStream(null, null),
    { remaining: '', replaced: false },
  );
  assert.deepEqual(
    mobileChatHelpers.splitReplyAfterStream(undefined, undefined),
    { remaining: '', replaced: false },
  );
  assert.equal(mobileChatHelpers.splitReplyAfterStream('A real reply.', null).remaining, 'A real reply.');
});

test('a reply waits at a gate without holding back its synthesis', async () => {
  const gate = deferred();
  const { queue, events, played } = createHarness({ gate: gate.promise });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });

  await flush();

  // The spooky preface is still speaking, so nothing of the reply may be
  // heard -- but both segments are already synthesized, which is what lets
  // playback start the instant the gate opens instead of waiting on the TTS.
  assert.deepEqual(events, ['synth:start:one', 'synth:start:two']);
  assert.deepEqual(played, []);
  assert.equal(queue.pending, 2);

  gate.resolve();
  const result = await queue.drain();

  assert.deepEqual(events, [
    'synth:start:one',
    'synth:start:two',
    'play:start:one',
    'play:end:one',
    'play:start:two',
    'play:end:two',
  ]);
  assert.equal(result.played, 2);
  assert.equal(result.firstError, null);
});

test('the gate is opened once, however many segments follow it', async () => {
  const gate = deferred();
  let opened = 0;
  const { queue, played } = createHarness({
    gate() {
      opened += 1;
      return gate.promise;
    },
  });

  queue.enqueue({ text: 'one' });
  queue.enqueue({ text: 'two' });
  queue.enqueue({ text: 'three' });

  await flush();
  assert.equal(opened, 1);

  gate.resolve();
  const result = await queue.drain();

  assert.equal(opened, 1, 'a gate is the speaker being handed over, not a per-segment wait');
  assert.deepEqual(played, ['one', 'two', 'three']);
  assert.equal(result.played, 3);
});

test('clear() while parked on a gate settles drain as the user asked for it', async () => {
  const gate = deferred();
  const { queue, played } = createHarness({ gate: gate.promise });

  queue.enqueue({ text: 'one' });
  const draining = queue.drain();

  await flush();
  assert.deepEqual(played, []);

  // Interrupting mid-preface must not leave drain() hanging: the turn's
  // finally awaits it, so a queue that never settles would wedge the page.
  queue.clear();
  const result = await draining;

  assert.equal(result.reason, 'user');
  assert.equal(played.length, 0);

  // A preface finishing after the interrupt must not resurrect the reply.
  gate.resolve();
  await flush();
  assert.deepEqual(played, []);
});

test('a gate that fails is an open gate, so the reply is still spoken', async () => {
  const gate = deferred();
  const { queue, played } = createHarness({ gate: gate.promise });

  queue.enqueue({ text: 'one' });
  await flush();

  gate.reject(new Error('the preface blew up'));

  const result = await queue.drain();

  assert.deepEqual(played, ['one'], 'a broken preface must not mute the reply');
  assert.equal(result.played, 1);
  assert.equal(result.firstError, null);
});

test('buildPrefacePlan orders the fart the way the message wrote it', () => {
  const plan = mobileChatHelpers.buildPrefacePlan;
  const fart = { type: 'fart', text: '' };

  assert.deepEqual(plan({ text: 'The veil trembles.' }), [
    { type: 'speech', text: 'The veil trembles.' },
  ]);

  assert.deepEqual(plan({ text: 'The veil trembles.', playFart: true, fartPosition: 'start' }), [
    fart,
    { type: 'speech', text: 'The veil trembles.' },
  ]);

  assert.deepEqual(
    plan({
      text: 'The veil trembles.',
      playFart: true,
      fartPosition: 'middle',
      beforeText: 'The veil',
      afterText: 'trembles.',
    }),
    [
      { type: 'speech', text: 'The veil' },
      fart,
      { type: 'speech', text: 'trembles.' },
    ],
  );

  assert.deepEqual(plan({ text: 'The veil trembles.', playFart: true, fartPosition: 'end' }), [
    { type: 'speech', text: 'The veil trembles.' },
    fart,
  ]);

  // An unrecognised position is treated as 'end' rather than losing the fart.
  assert.deepEqual(plan({ text: 'Boo.', playFart: true, fartPosition: 'sideways' }), [
    { type: 'speech', text: 'Boo.' },
    fart,
  ]);
});

test('buildPrefacePlan never emits a step that would synthesize silence', () => {
  const plan = mobileChatHelpers.buildPrefacePlan;

  // A middle marker at the very end of the line leaves nothing to say after it.
  assert.deepEqual(
    plan({
      text: 'The veil trembles.',
      playFart: true,
      fartPosition: 'middle',
      beforeText: 'The veil trembles.',
      afterText: '   ',
    }),
    [{ type: 'speech', text: 'The veil trembles.' }, { type: 'fart', text: '' }],
  );

  // A line that is nothing but a marker is still worth its fart.
  assert.deepEqual(plan({ text: '', playFart: true, fartPosition: 'middle' }), [
    { type: 'fart', text: '' },
  ]);

  assert.deepEqual(plan({}), []);
  assert.deepEqual(plan(null), []);
  assert.deepEqual(plan(undefined), []);
  assert.deepEqual(plan({ text: '   ' }), []);
});

function fakeMedia() {
  const listeners = new Map();
  return {
    listeners,
    addEventListener(name, handler) {
      if (!listeners.has(name)) {
        listeners.set(name, new Set());
      }
      listeners.get(name).add(handler);
    },
    removeEventListener(name, handler) {
      if (listeners.has(name)) {
        listeners.get(name).delete(handler);
      }
    },
    fire(name) {
      Array.from(listeners.get(name) || []).forEach((handler) => handler());
    },
    countFor(name) {
      return (listeners.get(name) || new Set()).size;
    },
  };
}

test('waitForMediaEnd resolves when the clip has been heard, not when it starts', async () => {
  const media = fakeMedia();
  let done = false;
  const waiting = mobileChatHelpers.waitForMediaEnd(media, { capMs: 60000 })
    .then(() => { done = true; });

  await flush();
  assert.equal(done, false, 'starting playback is not the end of the clip');

  media.fire('ended');
  await waiting;
  assert.equal(done, true);
});

test('waitForMediaEnd settles however the clip stops, and unsubscribes', async () => {
  for (const event of ['ended', 'error', 'abort']) {
    const media = fakeMedia();
    const waiting = mobileChatHelpers.waitForMediaEnd(media, { capMs: 60000 });

    media.fire(event);
    await waiting;

    ['ended', 'error', 'abort'].forEach((name) => {
      assert.equal(media.countFor(name), 0, `${name} listener must be removed after ${event}`);
    });
  }
});

test('waitForMediaEnd never hangs on a clip whose ended never fires', async () => {
  const media = fakeMedia();
  // The cap is the guarantee: the spooky preface gates the reply's voice behind
  // this wait, so a wait that never settles would never speak the reply.
  await mobileChatHelpers.waitForMediaEnd(media, { capMs: 5 });
  assert.equal(media.countFor('ended'), 0);
});

test('waitForMediaEnd copes with nothing to wait for', async () => {
  await mobileChatHelpers.waitForMediaEnd(null);
  await mobileChatHelpers.waitForMediaEnd(undefined);
  await mobileChatHelpers.waitForMediaEnd({});
});

test('the entity never speaks a preconfigured phrase of its own', () => {
  // It used to announce a canned line ("Let me think about that for a moment.")
  // before every answer. Only real reply sentences are spoken now, so the
  // helper that judged those announcements is gone -- and nothing may bring
  // back a status-phrase path without a test failing here.
  assert.equal(mobileChatHelpers.shouldSpeakStatusPhrase, undefined);
});
