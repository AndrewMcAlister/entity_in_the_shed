//! Turning a generated token stream into sentences the browser can speak.
//!
//! Port of `ReplyStreamPublisher` from `agent.py:5988-6031`.
//!
//! ## Where it sits
//!
//! Between the model and the chat job: [`ReplyStreamPublisher::feed`] decodes the
//! reply out of the JSON as it arrives and publishes each finished sentence, and
//! [`ReplyStreamPublisher::finish`] reports what was actually spoken so the caller
//! can work out how much of the final reply the user has already heard — which is
//! what [`crate::split_reply_after_stream`] is for.
//!
//! ## Two halves, one rule
//!
//! The scanner decides whether the document can be streamed at all and the
//! splitter decides when a sentence is safe to release. Neither is consulted
//! twice: once the scanner aborts, or once its reply string has closed, `feed`
//! stops doing anything at all, because a sentence cannot be unsaid and the
//! whole-reply audio will cover the rest of the turn.
//!
//! ## No thread, no lock
//!
//! The publish callback is called synchronously from [`ReplyStreamPublisher::feed`].
//! Whatever it does — enqueue a clip, write a websocket frame — happens on the
//! caller's thread, in order, once per sentence. Nothing is spawned and no lock is
//! held across a feed.
//!
//! ## The callback is owned, not borrowed
//!
//! Python keeps whatever `publish` was passed to the constructor, which is free
//! there and costs a lifetime in Rust: a stored `&mut dyn FnMut(&str)` would need
//! the publisher to outlive its own borrow, which is only expressible with
//! `unsafe`. So the constructor takes the callback by value, and a caller whose
//! sink is borrowed — which is the common case — reaches it through a shared
//! handle instead, exactly as the agent loop will hand this publisher a channel
//! sender. A caller with nothing to publish to uses
//! [`ReplyStreamPublisher::without_publish`], and the sentences it would have
//! received are still readable from [`ReplyStreamPublisher::spoken`].

use crate::JsonReplyStreamScanner;
use crate::SentenceSplitter;

/// The sink each finished sentence is handed to — `publish` in the source.
///
/// Owned rather than borrowed (see the module docs), and named because the
/// struct needs it inside an `Option`, a shape clippy will not accept spelled
/// out at a field.
pub type PublishFn = Box<dyn FnMut(&str)>;

/// Turns a generated token stream into sentences the browser can speak.
#[derive(Default)]
pub struct ReplyStreamPublisher {
    publish: Option<PublishFn>,
    scanner: JsonReplyStreamScanner,
    splitter: SentenceSplitter,
    /// Every sentence handed to the publish callback, in order — `spoken` in the
    /// source, and the same list the source's tests append to through their sink.
    pub spoken: Vec<String>,
    /// How many sentences were published.
    pub published: usize,
    /// The reply was not laid out the way the prompt asks, so nothing was spoken
    /// early and the turn falls back to whole-reply audio.
    pub aborted: bool,
}

impl ReplyStreamPublisher {
    /// Publish each finished sentence through `publish`.
    pub fn new(publish: PublishFn) -> Self {
        Self {
            publish: Some(publish),
            ..Self::default()
        }
    }

    /// A publisher with no callback — `publish=None` in the source.
    pub fn without_publish() -> Self {
        Self::default()
    }

    /// The reply as the scanner has decoded it so far.
    #[must_use]
    pub fn scanner(&self) -> &JsonReplyStreamScanner {
        &self.scanner
    }

    /// Feed one piece of the stream, publishing whatever sentence it completed.
    pub fn feed(&mut self, piece: Option<&str>) {
        if self.scanner.is_aborted() || self.scanner.is_done() {
            return;
        }
        let text = self.scanner.feed(piece);
        if self.scanner.is_aborted() {
            // The reply is not laid out the way the prompt asks, so nothing can
            // safely be spoken early. The turn falls back to whole-reply audio.
            self.aborted = true;
            return;
        }
        let sentences = self.splitter.feed(Some(&text));
        self.emit(&sentences);
    }

    /// Flush the last sentence and report everything that was spoken.
    pub fn finish(&mut self) -> String {
        let sentences = self.splitter.flush();
        self.emit(&sentences);
        self.spoken_text()
    }

    /// The published sentences joined the way the caller compares them against a
    /// reply: one space between clips, since each was stripped.
    pub fn spoken_text(&self) -> String {
        self.spoken.join(" ")
    }

    fn emit(&mut self, sentences: &[String]) {
        for sentence in sentences {
            let cleaned = crate::py_strip(sentence);
            if cleaned.is_empty() {
                continue;
            }
            self.spoken.push(cleaned.to_owned());
            self.published += 1;
            if let Some(publish) = self.publish.as_mut() {
                publish(cleaned);
            }
        }
    }
}
