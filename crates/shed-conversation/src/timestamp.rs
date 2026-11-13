//! The stamp grammar: what a stored timestamp may be spelled like, and how two
//! of them compare.
//!
//! Ported from `utils/conversation_history.py:219-238` (`_parse_timestamp`,
//! `_is_at_or_after`) plus the one comparison rule those two and the three
//! freshness rules in [`crate::freshness`] are built on.
//!
//! ## Why the parse is written rather than taken from chrono
//!
//! The source calls `datetime.fromisoformat`, and what Python accepts there is a
//! *grammar*, not a format string. chrono's nearest thing — `DateTime::parse_from_rfc3339`
//! — is stricter in the places that matter to this app and looser in none, so
//! every stamp chrono refused would be a stamp the Python app read and the clone
//! forgot. The answer is a ladder, first rung that parses wins:
//!
//! | Rung | Accepts | Kind |
//! |---|---|---|
//! | `DateTime::parse_from_rfc3339` | `2026-10-27T12:00:00+00:00`, the same with `Z`, fractional seconds | aware |
//! | `NaiveDateTime::parse_from_str(…, "%Y-%m-%dT%H:%M:%S%.f")` | the same with no offset at all | naive |
//! | `NaiveDateTime::parse_from_str(…, "%Y-%m-%d %H:%M:%S%.f")` | the space separator Python allows in place of `T` | naive |
//! | `NaiveDate::parse_from_str(…, "%Y-%m-%d")` | a bare date, read at midnight | naive |
//!
//! Anything else is `None`, and every rule here reads `None` as "no stamp" —
//! which is never "old". Three narrowings of Python's grammar are deliberate and
//! recorded in `docs/PARITY.md`:
//!
//! - **Time-only strings are not supported** (decision 1 of `SLICE-4b-BRIEF`).
//!   Python 3.11+ reads `"12:00:00"` as today at noon; a bare time names no day,
//!   so it cannot order a transcript, and every stamp this app has stored is a
//!   full date-plus-time written by `isoformat()`.
//! - **A short offset** (`+02`, `+0230`) parses in Python and not through RFC 3339.
//!   The app only ever writes the full `±HH:MM`.
//! - **A fraction longer than nine digits** is refused by chrono's `%.f` and
//!   accepted by Python; `isoformat()` emits at most six digits.
//!
//! ## Naive against aware: `replace`, not a conversion
//!
//! Python refuses to compare a naive datetime with an aware one — `TypeError`.
//! The source therefore patches the aware side's offset onto the naive side's
//! *wall time* (`parsed.replace(tzinfo=cutoff.tzinfo)`) before comparing a stored
//! stamp against a caller-supplied `now` or `covered_through`. That attaches an
//! offset; it does not convert the instant, and for anyone not on UTC the two
//! answers are hours apart. [`Timestamp::at_least`] and
//! [`Timestamp::gap_at_least`] are where the rule lives: the three rules in
//! `freshness` call them instead of each re-implementing the patch.
//!
//! The reverse pair — an aware stamp against a naive cutoff — is the `TypeError`
//! the source never handles. Rust must not panic where Python raises, so that pair
//! has an answer here rather than a crash: it orders **nowhere**, which every
//! caller treats as "not inside the window, not a return" — the same treatment a
//! stamp it cannot read gets. It is unreachable in the app, because every cutoff
//! comes from `datetime.now(timezone.utc)` and every stored stamp carries the
//! offset that same clock wrote. It is recorded in `docs/PARITY.md`.

use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// A stored timestamp: a wall time, or an instant.
///
/// Both spellings occur in the app's files — the record writer stores
/// `datetime.now(timezone.utc).isoformat()`, and a stamp that arrives as text may
/// carry no offset — so one type covers both instead of two parallel APIs.
///
/// Deliberately **not** `Ord`: the natural derive would rank every naive stamp
/// below every aware one by variant, which is not how these compare. The ordering
/// is a rule about a *pair*, and it is spelled as methods that name which side is
/// the stamp and which is the cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timestamp {
    /// A date and time with no offset attached.
    Naive(NaiveDateTime),
    /// A date and time with the offset it arrived with.
    Aware(DateTime<FixedOffset>),
}

impl Timestamp {
    /// The four rungs, in order; `None` for anything else.
    ///
    /// `None` also answers for an absent value, which is what Python's
    /// `isinstance(value, str)` test amounts to: the source reads these out of raw
    /// JSON, where a stamp can be a number or `null`. A caller holding
    /// `Option<&str>` passes it straight in — a missing stamp, a stamp that is not
    /// text, and a stamp that is not a stamp are all the same thing to the rules.
    #[must_use]
    pub fn parse(value: Option<&str>) -> Option<Self> {
        let text = value?;
        if let Ok(aware) = DateTime::parse_from_rfc3339(text) {
            return Some(Self::Aware(aware));
        }
        if let Ok(wall) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f") {
            return Some(Self::Naive(wall));
        }
        if let Ok(wall) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f") {
            return Some(Self::Naive(wall));
        }
        // Python reads a bare date as midnight too, so a stamp that lost its time
        // half still orders against the others instead of dropping out of them.
        NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()
            .and_then(|day| day.and_hms_opt(0, 0, 0))
            .map(Self::Naive)
    }

    /// Whether this stamp falls at or after `cutoff`.
    ///
    /// A pair that cannot be ordered at all — an aware stamp against a naive
    /// cutoff, which is the `TypeError` the source never handles — answers `true`,
    /// the same answer an unreadable stamp gets: a stamp that cannot be placed is
    /// not evidence of age.
    #[must_use]
    pub fn at_least(self, cutoff: Self) -> bool {
        Self::instants(self, cutoff).is_none_or(|(this, that)| this >= that)
    }

    /// Whether this stamp falls strictly after `cutoff`.
    ///
    /// The profile marker needs the strict form: the row a marker names has
    /// already been read, so the next pass must not be handed it again. The
    /// unorderable pair answers `true` for the same reason [`Timestamp::at_least`]
    /// does — the row is kept and read again, which costs a pass rather than a
    /// memory.
    #[must_use]
    pub fn after(self, cutoff: Self) -> bool {
        Self::instants(self, cutoff).is_none_or(|(this, that)| this > that)
    }

    /// Whether the gap from this stamp to `later` reaches `quiet`.
    ///
    /// Subtracted, as the source subtracts, so a `later` that falls *before* this
    /// stamp is a gap of nothing rather than a negative number that clears every
    /// threshold: a clock that went backwards cannot make this turn a return. The
    /// unorderable pair is likewise no gap — being unable to place a stamp is not
    /// evidence that anyone was away.
    #[must_use]
    pub fn gap_at_least(self, later: Self, quiet: Duration) -> bool {
        let Some((stamp, then)) = Self::instants(self, later) else {
            return false;
        };
        // A negative gap — `later` before this stamp — has no `Duration` form,
        // and `to_std` says so: the answer is "not a return", not an error.
        then.signed_duration_since(stamp)
            .to_std()
            .is_ok_and(|gap| gap >= quiet)
    }

    /// Both sides on one timeline: the naive side takes the aware side's offset,
    /// which is the source's `replace` — an attachment, not a conversion.
    ///
    /// `None` for the one pair with no shared timeline: an aware stamp against a
    /// naive cutoff. Python raises `TypeError` there; each predicate above answers
    /// it as "recent", and none of them re-implements the attachment rule.
    fn instants(stamp: Self, cutoff: Self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        match (stamp, cutoff) {
            (Self::Naive(wall), Self::Aware(then)) => {
                let attached = then.offset().from_local_datetime(&wall).single()?;
                Some((attached.to_utc(), then.to_utc()))
            }
            // Two wall times: any offset compares them alike, and UTC is the one
            // that changes neither.
            (Self::Naive(wall), Self::Naive(other)) => Some((wall.and_utc(), other.and_utc())),
            (Self::Aware(this), Self::Aware(other)) => Some((this.to_utc(), other.to_utc())),
            (Self::Aware(_), Self::Naive(_)) => None,
        }
    }
}

/// Whether a stamp falls inside a window, with unreadable stamps kept.
///
/// Ported from `_is_at_or_after` (228-238). A missing or malformed timestamp is
/// not evidence of old age, so it counts as recent: reading it as expired would
/// let one bad row delete a memory. The same treatment covers the pair this
/// function cannot order — an aware stamp against a naive cutoff, which is the
/// `TypeError` the source leaves to its caller.
#[must_use]
pub fn is_at_or_after(value: Option<&str>, cutoff: Timestamp) -> bool {
    Timestamp::parse(value).is_none_or(|parsed| parsed.at_least(cutoff))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{is_at_or_after, Timestamp};

    /// A stamp the source would have written; a panic here is a broken fixture.
    fn stamp(text: &str) -> Timestamp {
        if let Some(parsed) = Timestamp::parse(Some(text)) {
            return parsed;
        }
        panic!("{text} should parse as a stamp")
    }

    #[test]
    fn a_bare_date_is_read_as_midnight() {
        // Python's fromisoformat reads "2026-10-27" as that day at midnight too, so
        // a stamp that lost its time half still orders against the others.
        assert_eq!(stamp("2026-10-27"), stamp("2026-10-27T00:00:00"));
    }

    #[test]
    fn an_offset_is_read_as_an_instant() {
        assert_eq!(
            stamp("2026-10-27T12:00:00Z"),
            stamp("2026-10-27T12:00:00+00:00")
        );
        assert_eq!(
            stamp("2026-10-27T17:00:00+05:00"),
            stamp("2026-10-27T12:00:00Z")
        );
    }

    #[test]
    fn a_space_separator_is_accepted() {
        // The source allows the space Python tolerates in place of the T.
        assert_eq!(stamp("2026-10-27 12:00:00"), stamp("2026-10-27T12:00:00"));
    }

    #[test]
    fn an_absent_or_unreadable_stamp_is_none() {
        assert_eq!(Timestamp::parse(None), None);
        assert_eq!(Timestamp::parse(Some("")), None);
        // Time-only: Python 3.11 reads it as today at noon, which names no day and
        // cannot order a transcript (decision 1, recorded in docs/PARITY.md).
        assert_eq!(Timestamp::parse(Some("12:00:00")), None);
        assert_eq!(Timestamp::parse(Some("not a stamp")), None);
    }

    #[test]
    fn a_naive_stamp_takes_the_aware_cutoffs_offset_rather_than_converting() {
        let cutoff = stamp("2026-10-27T13:00:00+05:00");

        // The same wall time as the cutoff: attaching makes one instant, so at
        // least is true and strictly-after is false.
        let same_wall = stamp("2026-10-27T13:00:00");
        assert!(same_wall.at_least(cutoff));
        assert!(!same_wall.after(cutoff));

        // Two hours earlier on the same wall clock: attaching puts it at 06:00Z,
        // before the cutoff's 08:00Z. A conversion would instead read 11:00 as an
        // instant in *its* own (absent) zone and answer true, so this pins the rule.
        let earlier_wall = stamp("2026-10-27T11:00:00");
        assert!(!earlier_wall.at_least(cutoff));
    }

    #[test]
    fn an_aware_stamp_against_a_naive_cutoff_is_recent_rather_than_a_crash() {
        // Python raises TypeError here; Rust must not panic. The pair orders
        // nowhere, which every predicate treats as "kept" and no gap.
        let aware = stamp("2026-10-27T12:00:00+05:00");
        let naive = stamp("2026-10-27T09:00:00");

        assert!(aware.at_least(naive));
        assert!(aware.after(naive));
        assert!(!aware.gap_at_least(naive, Duration::from_secs(60)));
    }

    #[test]
    fn unreadable_is_recent() {
        let cutoff = stamp("2026-10-27T12:00:00Z");

        // Missing and malformed both count as inside the window.
        assert!(is_at_or_after(None, cutoff));
        assert!(is_at_or_after(Some("not a stamp"), cutoff));

        assert!(!is_at_or_after(Some("2026-10-27T11:59:59Z"), cutoff));
        assert!(is_at_or_after(Some("2026-10-27T12:00:00Z"), cutoff));
        assert!(is_at_or_after(Some("2026-10-27T12:00:01Z"), cutoff));
    }

    #[test]
    fn a_gap_that_runs_backwards_is_no_gap() {
        // A clock that went backwards cannot make a turn a return: the gap from a
        // later stamp back to an earlier one is nothing, not a negative interval.
        let eleven = stamp("2026-10-27T11:00:00Z");
        let noon = stamp("2026-10-27T12:00:00Z");

        assert!(!noon.gap_at_least(eleven, Duration::ZERO));
        assert!(noon.gap_at_least(noon, Duration::ZERO));
        // Forwards, the same pair is the hour it looks like.
        assert!(eleven.gap_at_least(noon, Duration::from_secs(3600)));
    }
}
