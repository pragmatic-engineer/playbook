// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A small hidden banner. Typing `xyzzy` on keys that do nothing else shows a
//! one-line message in place of the footer for a few seconds. It keeps no
//! state outside memory and is not listed in the key table or the help.

use std::time::{Duration, Instant};

const SEQUENCE: [char; 5] = ['x', 'y', 'z', 'z', 'y'];
/// The longest pause allowed between two keys of the sequence.
const GAP: Duration = Duration::from_secs(2);
/// How long the banner stays up.
const SHOWN: Duration = Duration::from_secs(4);

/// The text of the banner. The TV is first so a terminal that draws it at the
/// other width only shifts the words on this one line.
pub const BANNER: &str = "\u{1F4FA} Nothing is on. Your tokens are still being counted.";

#[derive(Default)]
pub struct Egg {
    matched: usize,
    last: Option<Instant>,
    until: Option<Instant>,
}

/// Opaque on purpose: the sequence progress is not view state, so it stays out
/// of the state dump the key table test compares.
impl std::fmt::Debug for Egg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Egg")
    }
}

impl Egg {
    /// Takes a key the view has no use for. Returns true when it completes
    /// the sequence and the banner starts.
    pub fn feed(&mut self, ch: char, now: Instant) -> bool {
        let stale = self.last.is_some_and(|t| now.duration_since(t) > GAP);
        if stale {
            self.matched = 0;
        }
        self.last = Some(now);
        if ch == SEQUENCE[self.matched] {
            self.matched += 1;
        } else {
            self.matched = usize::from(ch == SEQUENCE[0]);
        }
        if self.matched < SEQUENCE.len() {
            return false;
        }
        self.matched = 0;
        self.until = Some(now + SHOWN);
        true
    }

    /// Any key the view does use breaks the sequence.
    pub fn reset(&mut self) {
        self.matched = 0;
    }

    pub fn visible(&self, now: Instant) -> bool {
        self.until.is_some_and(|t| now < t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_all(egg: &mut Egg, text: &str, start: Instant, step: Duration) -> bool {
        let mut done = false;
        for (i, ch) in text.chars().enumerate() {
            done = egg.feed(ch, start + step * i as u32);
        }
        done
    }

    #[test]
    fn typing_the_sequence_quickly_shows_the_banner() {
        let mut egg = Egg::default();
        let t0 = Instant::now();

        let done = type_all(&mut egg, "xyzzy", t0, Duration::from_millis(100));

        assert!(done);
        assert!(egg.visible(t0 + Duration::from_secs(1)));
    }

    #[test]
    fn the_banner_goes_away_after_a_few_seconds() {
        let mut egg = Egg::default();
        let t0 = Instant::now();
        type_all(&mut egg, "xyzzy", t0, Duration::ZERO);

        let shown = egg.visible(t0 + SHOWN + Duration::from_millis(1));

        assert!(!shown);
    }

    #[test]
    fn a_wrong_key_in_the_middle_starts_the_sequence_over() {
        let mut egg = Egg::default();
        let t0 = Instant::now();

        let done = type_all(&mut egg, "xyqzzy", t0, Duration::from_millis(50));

        assert!(!done);
        assert!(!egg.visible(t0));
    }

    #[test]
    fn a_slow_typist_does_not_trigger_the_banner() {
        let mut egg = Egg::default();
        let t0 = Instant::now();

        let done = type_all(&mut egg, "xyzzy", t0, Duration::from_secs(3));

        assert!(!done);
    }

    #[test]
    fn a_used_key_between_the_letters_breaks_the_sequence() {
        let mut egg = Egg::default();
        let t0 = Instant::now();
        egg.feed('x', t0);
        egg.feed('y', t0);

        egg.reset();
        let done = type_all(&mut egg, "zzy", t0, Duration::ZERO);

        assert!(!done);
    }

    #[test]
    fn a_repeated_first_letter_still_lets_the_sequence_finish() {
        let mut egg = Egg::default();
        let t0 = Instant::now();

        let done = type_all(&mut egg, "xxyzzy", t0, Duration::from_millis(50));

        assert!(done);
    }
}
