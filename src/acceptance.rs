// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Acceptance bookkeeping without editor I/O. Never retries a posted chunk.
use std::collections::VecDeque;

pub fn unicode_chunks(text: &str) -> Option<VecDeque<String>> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let mut chunks = VecDeque::new();
    let mut chunk = String::new();
    let mut units = 0;
    for ch in text.chars() {
        if units + ch.len_utf16() > 20 {
            chunks.push_back(std::mem::take(&mut chunk));
            units = 0;
        }
        chunk.push(ch);
        units += ch.len_utf16();
    }
    if !chunk.is_empty() {
        chunks.push_back(chunk);
    }
    Some(chunks)
}

fn editor_equal(a: &str, b: &str) -> bool {
    // Chromium turns spaces into NBSP to preserve HTML whitespace visually.
    a.chars()
        .map(|c| if c == '\u{a0}' { ' ' } else { c })
        .eq(b.chars().map(|c| if c == '\u{a0}' { ' ' } else { c }))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Observation {
    Waiting,
    Confirmed,
    Diverged,
}

pub struct AcceptancePlan {
    prefix: String,
    suffix: String,
    chunks: VecDeque<String>,
    in_flight: Option<String>,
    word_has_letters: bool,
    verified_words: u64,
    pub remainder: String,
    pub accepted_utf16_units: usize,
}

/// A replacement must first select exactly the intended range in an unchanged
/// editor. After that, insertion uses the existing verified chunk transport.
#[derive(Clone, Debug)]
pub struct ReplacementPlan {
    pub start_utf16: usize,
    pub length_utf16: usize,
    before: String,
    after: String,
    original_text: String,
    replacement: String,
}
impl ReplacementPlan {
    pub fn new(prefix: &str, suffix: &str, start_byte: usize, replacement: &str) -> Option<Self> {
        if !prefix.is_char_boundary(start_byte)
            || start_byte >= prefix.len()
            || replacement.is_empty()
            || replacement.chars().any(char::is_control)
        {
            return None;
        }
        let before = &prefix[..start_byte];
        Some(Self {
            start_utf16: before.encode_utf16().count(),
            length_utf16: prefix[start_byte..].encode_utf16().count(),
            before: before.into(),
            after: suffix.into(),
            original_text: format!("{prefix}{suffix}"),
            replacement: replacement.into(),
        })
    }
    pub fn selection_matches(&self, text: &str, start: usize, length: usize) -> bool {
        start == self.start_utf16
            && length == self.length_utf16
            && editor_equal(text, &self.original_text)
    }
    /// A selection request is asynchronous in browser editors. Only the exact
    /// requested selection confirms it; an unchanged original caret may wait.
    /// Never treat a different selection or changed document as pending success.
    pub fn observe_selection(
        &self,
        text: &str,
        range: (usize, usize),
        original_range: (usize, usize),
        expired: bool,
    ) -> Observation {
        if expired || !editor_equal(text, &self.original_text) {
            Observation::Diverged
        } else if self.selection_matches(text, range.0, range.1) {
            Observation::Confirmed
        } else if range == original_range {
            Observation::Waiting
        } else {
            Observation::Diverged
        }
    }
    pub fn insertion(&self, unicode: bool) -> Option<AcceptancePlan> {
        AcceptancePlan::new(
            &self.before,
            &self.after,
            &self.replacement,
            true,
            unicode,
            false,
        )
    }
    pub fn preview(&self) -> String {
        format!("{}{}{}", self.before, self.replacement, self.after)
    }
}

impl AcceptancePlan {
    pub fn new(
        prefix: &str,
        suffix: &str,
        suggestion: &str,
        all: bool,
        unicode: bool,
        trailing_space: bool,
    ) -> Option<Self> {
        if suggestion.trim().is_empty() || suggestion.chars().any(char::is_control) {
            return None;
        }
        let end = if all {
            suggestion.len()
        } else {
            let mut seen = false;
            suggestion
                .char_indices()
                .find_map(|(i, c)| {
                    if c.is_whitespace() && seen {
                        Some(i)
                    } else {
                        seen |= !c.is_whitespace();
                        None
                    }
                })
                .unwrap_or(suggestion.len())
        };
        let mut end = end;
        // Move the separator from the retained suggestion into the accepted text,
        // preserving the combined text exactly across repeated word acceptance.
        let transfer_separator = if suffix.is_empty() {
            trailing_space
        } else {
            suffix.chars().next().is_some_and(char::is_alphanumeric)
        };
        if transfer_separator && !all {
            end += suggestion[end..]
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(char::len_utf8)
                .sum::<usize>();
        }
        let mut accepted = suggestion[..end].to_owned();
        if trailing_space
            && !all
            && end == suggestion.len()
            && suffix.is_empty()
            && !accepted.ends_with(char::is_whitespace)
        {
            accepted.push(' ');
        }
        let accepted_utf16_units = accepted.encode_utf16().count();
        let chunks = if unicode {
            unicode_chunks(&accepted)?
        } else {
            VecDeque::from([accepted])
        };
        Some(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
            chunks,
            in_flight: None,
            word_has_letters: false,
            verified_words: 0,
            remainder: suggestion[end..].into(),
            accepted_utf16_units,
        })
    }

    pub fn next_chunk(&mut self) -> Option<String> {
        if self.in_flight.is_some() {
            return None;
        }
        let chunk = self.chunks.pop_front()?;
        self.in_flight = Some(chunk.clone());
        Some(chunk)
    }

    /// Drain only words whose inserted characters have been observed in the
    /// editor. No source text is passed to the statistics recorder.
    pub fn take_verified_words(&mut self) -> u64 {
        std::mem::take(&mut self.verified_words)
    }

    /// Text-free explanation for an unconfirmed insertion. This is diagnostic
    /// only: matching the full text with a different caret never confirms a
    /// chunk, counts words, or allows the next chunk to be posted.
    pub fn unconfirmed_reason(&self, prefix: &str, suffix: &str) -> &'static str {
        let Some(chunk) = &self.in_flight else {
            return "no pending chunk";
        };
        let expected_prefix = format!("{}{}", self.prefix, chunk);
        if editor_equal(prefix, &expected_prefix) && editor_equal(suffix, &self.suffix) {
            "expected text and caret present"
        } else if editor_equal(prefix, &self.prefix) && editor_equal(suffix, &self.suffix) {
            "editor unchanged before timeout"
        } else if editor_equal(
            &format!("{prefix}{suffix}"),
            &format!("{expected_prefix}{}", self.suffix),
        ) {
            "expected text present but caret moved"
        } else if !editor_equal(suffix, &self.suffix) {
            "suffix changed"
        } else {
            "prefix differs from expected insertion"
        }
    }

    /// Prefix and suffix come from the editor's *current caret*, not merely its
    /// full value. A moved caret or changed text aborts the remaining insertion.
    pub fn observe(&mut self, prefix: &str, suffix: &str) -> Observation {
        let Some(chunk) = &self.in_flight else {
            return Observation::Diverged;
        };
        if !editor_equal(suffix, &self.suffix) {
            return Observation::Diverged;
        }
        if editor_equal(prefix, &format!("{}{}", self.prefix, chunk)) {
            for ch in chunk.chars() {
                if ch.is_whitespace() {
                    self.verified_words += u64::from(self.word_has_letters);
                    self.word_has_letters = false;
                } else {
                    self.word_has_letters |= ch.is_alphanumeric();
                }
            }
            // A transport boundary can fall inside a word. Count that word only
            // when its rest is verified, or this accepted selection ends.
            if self
                .chunks
                .front()
                .is_none_or(|next| next.starts_with(char::is_whitespace))
            {
                self.verified_words += u64::from(self.word_has_letters);
                self.word_has_letters = false;
            }
            self.prefix = prefix.into();
            self.in_flight = None;
            Observation::Confirmed
        } else if editor_equal(prefix, &self.prefix) {
            Observation::Waiting
        } else {
            Observation::Diverged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unconfirmed_diagnostics_do_not_confirm_moved_carets_or_retry_chunks() {
        let mut plan =
            AcceptancePlan::new("We should ", "tomorrow.", "meet ", true, true, false).unwrap();
        assert_eq!(plan.unconfirmed_reason("", ""), "no pending chunk");
        assert_eq!(plan.next_chunk().as_deref(), Some("meet "));
        for (prefix, suffix, reason, observation) in [
            (
                "We should ",
                "tomorrow.",
                "editor unchanged before timeout",
                Observation::Waiting,
            ),
            (
                "We should meet tomorrow.",
                "",
                "expected text present but caret moved",
                Observation::Diverged,
            ),
            (
                "We should\u{a0}",
                "meet\u{a0}tomorrow.",
                "expected text present but caret moved",
                Observation::Diverged,
            ),
            (
                "We should meet ",
                "Friday.",
                "suffix changed",
                Observation::Diverged,
            ),
            (
                "We should me",
                "tomorrow.",
                "prefix differs from expected insertion",
                Observation::Diverged,
            ),
        ] {
            assert_eq!(plan.unconfirmed_reason(prefix, suffix), reason);
            assert_eq!(plan.observe(prefix, suffix), observation);
            assert_eq!(plan.take_verified_words(), 0);
            assert!(plan.next_chunk().is_none());
        }
        assert_eq!(
            plan.unconfirmed_reason("We should meet ", "tomorrow."),
            "expected text and caret present"
        );
        assert_eq!(
            plan.observe("We should meet ", "tomorrow."),
            Observation::Confirmed
        );
        assert_eq!(plan.take_verified_words(), 1);
        assert!(plan.next_chunk().is_none());
    }

    #[test]
    fn delayed_replacement_selection_waits_then_verifies_exact_emoji_edit() {
        let text = "Test :thumbsup:";
        let replacement = ReplacementPlan::new(text, " later", 5, "👍").unwrap();
        let whole = "Test :thumbsup: later";
        let caret = (15, 0);
        // The setter can acknowledge before two successive AX reads catch up.
        for _ in 0..2 {
            assert_eq!(
                replacement.observe_selection(whole, caret, caret, false),
                Observation::Waiting
            );
        }
        assert_eq!(
            replacement.observe_selection(whole, (5, 10), caret, false),
            Observation::Confirmed
        );
        let mut insertion = replacement.insertion(true).unwrap();
        assert_eq!(insertion.next_chunk().as_deref(), Some("👍"));
        assert_eq!(
            insertion.observe("Test 👍", " later"),
            Observation::Confirmed
        );
        assert!(insertion.next_chunk().is_none());
        assert_eq!(insertion.take_verified_words(), 0);
    }

    #[test]
    fn replacement_selection_rejects_intervening_edits_movement_and_late_confirmation() {
        let text = "🙂 :thumbsup:";
        let replacement = ReplacementPlan::new(text, "", "🙂 ".len(), "👍").unwrap();
        let original = (13, 0);
        assert_eq!(
            replacement.observe_selection(text, (3, 10), original, false),
            Observation::Confirmed
        );
        for (value, range, expired) in [
            ("🙂 :thumbsup:!", (3, 10), false),
            (text, (2, 10), false),
            (text, (3, 9), false),
            (text, (4, 0), false),
            (text, original, true),
            (text, (3, 10), true),
        ] {
            assert_eq!(
                replacement.observe_selection(value, range, original, expired),
                Observation::Diverged
            );
        }
    }
    #[test]
    fn statistics_count_only_confirmed_whole_words_once_across_transport_chunks() {
        let mut plan = AcceptancePlan::new(
            "",
            "",
            "one supercalifragilistic word 🙂 !!!",
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(plan.take_verified_words(), 0);
        let first = plan.next_chunk().unwrap();
        assert_eq!(plan.observe("", ""), Observation::Waiting);
        assert_eq!(plan.take_verified_words(), 0);
        assert_eq!(plan.observe(&first, ""), Observation::Confirmed);
        assert_eq!(plan.take_verified_words(), 1); // Long second word still incomplete.
        assert_eq!(plan.take_verified_words(), 0);
        let mut text = first;
        let mut later = 0;
        while let Some(chunk) = plan.next_chunk() {
            text.push_str(&chunk);
            assert_eq!(plan.observe(&text, ""), Observation::Confirmed);
            later += plan.take_verified_words();
        }
        assert_eq!(later, 2);
        assert_eq!(plan.observe(&text, ""), Observation::Diverged);
        assert_eq!(plan.take_verified_words(), 0);
        let mut aborted = AcceptancePlan::new(
            "",
            "suffix",
            "one supercalifragilistic word",
            true,
            true,
            false,
        )
        .unwrap();
        let first = aborted.next_chunk().unwrap();
        assert_eq!(aborted.observe(&first, "suffix"), Observation::Confirmed);
        assert_eq!(aborted.take_verified_words(), 1);
        aborted.next_chunk().unwrap();
        assert_eq!(
            aborted.observe("user edit", "suffix"),
            Observation::Diverged
        );
        assert_eq!(aborted.take_verified_words(), 0);
    }
    #[test]
    fn statistics_handle_repeated_acceptance_fragments_and_unicode_without_counting_emoji() {
        let mut prefix = "Th".to_owned();
        let mut remaining = "anks résumé 中文 🙂 42".to_owned();
        let mut total = 0;
        while !remaining.is_empty() {
            let mut plan = AcceptancePlan::new(&prefix, "", &remaining, false, true, true).unwrap();
            while let Some(chunk) = plan.next_chunk() {
                prefix.push_str(&chunk);
                assert_eq!(plan.observe(&prefix, ""), Observation::Confirmed);
                total += plan.take_verified_words();
            }
            remaining = plan.remainder;
        }
        assert_eq!(total, 4);
        assert_eq!(prefix, "Thanks résumé 中文 🙂 42 ");
    }
    #[test]
    fn replacement_requires_exact_selection_and_preserves_surrounding_unicode() {
        let prefix = "🙂 Please recieve  ";
        let suffix = "the résumé.";
        let start = prefix.find("recieve").unwrap();
        let replacement = ReplacementPlan::new(prefix, suffix, start, "receive  ").unwrap();
        assert_eq!(replacement.start_utf16, 10);
        assert_eq!(replacement.length_utf16, 9);
        assert!(replacement.selection_matches(&format!("{prefix}{suffix}"), 10, 9));
        assert!(!replacement.selection_matches(&format!("{prefix}{suffix}"), 11, 9));
        assert!(!replacement.selection_matches(&format!("{prefix}changed"), 10, 9));
        assert_eq!(replacement.preview(), "🙂 Please receive  the résumé.");
        let mut insertion = replacement.insertion(true).unwrap();
        assert_eq!(insertion.next_chunk().as_deref(), Some("receive  "));
        assert_eq!(
            insertion.observe("🙂 Please receive  ", suffix),
            Observation::Confirmed
        );
        assert!(ReplacementPlan::new(prefix, suffix, 1, "receive").is_none());
        assert!(ReplacementPlan::new(prefix, suffix, start, "receive\n").is_none());
    }
    #[test]
    fn midline_word_acceptance_never_glues_a_word_to_the_existing_suffix() {
        let mut prefix = "We should ".to_owned();
        let mut suggestion = "meet and review ".to_owned();
        for accepted in ["meet ", "and ", "review "] {
            let mut plan =
                AcceptancePlan::new(&prefix, "tomorrow.", &suggestion, false, true, false).unwrap();
            assert_eq!(plan.next_chunk().as_deref(), Some(accepted));
            prefix.push_str(accepted);
            assert_eq!(plan.observe(&prefix, "tomorrow."), Observation::Confirmed);
            suggestion = plan.remainder;
        }
        assert!(suggestion.is_empty());
        assert_eq!(
            format!("{prefix}tomorrow."),
            "We should meet and review tomorrow."
        );
        let mut plan = AcceptancePlan::new(
            "We should ",
            " tomorrow.",
            "meet and review",
            false,
            true,
            true,
        )
        .unwrap();
        assert_eq!(plan.next_chunk().as_deref(), Some("meet"));
        assert_eq!(plan.remainder, " and review");
    }
    #[test]
    fn midline_acceptance_preserves_suffix_and_aborts_on_suffix_edits() {
        let suffix = "tomorrow. 👩‍💻";
        let mut plan = AcceptancePlan::new(
            "We should ",
            suffix,
            "meet and review the detailed proposal ",
            true,
            true,
            false,
        )
        .unwrap();
        let mut prefix = "We should ".to_owned();
        while let Some(chunk) = plan.next_chunk() {
            assert_eq!(plan.observe(&prefix, suffix), Observation::Waiting);
            prefix.push_str(&chunk);
            assert_eq!(plan.observe(&prefix, suffix), Observation::Confirmed);
        }
        assert_eq!(
            format!("{prefix}{suffix}"),
            "We should meet and review the detailed proposal tomorrow. 👩‍💻"
        );
        let mut plan =
            AcceptancePlan::new("We should ", suffix, "meet ", true, true, false).unwrap();
        plan.next_chunk();
        assert_eq!(
            plan.observe("We should meet ", "on Friday."),
            Observation::Diverged
        );
    }
    #[test]
    fn long_unicode_insertions_round_trip_without_splitting_surrogates() {
        let text = " A long continuation with café, cafe\u{301}, 👩‍💻 and 🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂.";
        let chunks = unicode_chunks(text).unwrap();
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|s| s.encode_utf16().count() <= 20));
        assert_eq!(chunks.iter().cloned().collect::<String>(), text);
        for text in ["", "send\n", "tab\t", "return\r", "escape\u{1b}"] {
            assert!(unicode_chunks(text).is_none());
        }
    }

    #[test]
    fn repeated_tabs_retain_the_exact_original_remainder() {
        let mut prefix = "Thank you for your".to_owned();
        let original = " time and thoughtful help.";
        let mut remaining = original.to_owned();
        for word in [" time", " and", " thoughtful", " help."] {
            let mut plan =
                AcceptancePlan::new(&prefix, "", &remaining, false, true, false).unwrap();
            assert_eq!(plan.next_chunk().unwrap(), word);
            assert_eq!(plan.next_chunk(), None); // never repost an unverified chunk
            assert_eq!(plan.observe(&prefix, ""), Observation::Waiting);
            prefix.push_str(word);
            assert_eq!(plan.observe(&prefix, ""), Observation::Confirmed);
            assert_eq!(plan.next_chunk(), None);
            remaining = plan.remainder;
        }
        assert!(remaining.is_empty());
        assert_eq!(prefix, format!("Thank you for your{original}"));
    }

    #[test]
    fn shift_tab_verifies_each_chunk_and_detects_cursor_or_text_changes() {
        let text = " an entire suggestion longer than twenty characters.";
        let mut plan = AcceptancePlan::new("Here is", "", text, true, true, false).unwrap();
        let mut prefix = "Here is".to_owned();
        while let Some(chunk) = plan.next_chunk() {
            assert_eq!(plan.observe(&prefix, ""), Observation::Waiting);
            assert_eq!(plan.observe(&prefix, "moved cursor"), Observation::Diverged);
            prefix.push_str(&chunk);
            assert_eq!(
                plan.observe(&prefix.replace(' ', "\u{a0}"), ""),
                Observation::Confirmed
            );
        }
        assert_eq!(prefix, format!("Here is{text}"));
        assert!(plan.remainder.is_empty());
        let mut plan = AcceptancePlan::new("Here is", "", text, true, true, false).unwrap();
        plan.next_chunk();
        assert_eq!(
            plan.observe("Here is something I typed", ""),
            Observation::Diverged
        );
    }

    #[test]
    fn trailing_space_moves_existing_separator_without_changing_remaining_words() {
        let mut first = AcceptancePlan::new(
            "Thank you for your ",
            "",
            "time and help.",
            false,
            true,
            true,
        )
        .unwrap();
        assert_eq!(first.next_chunk().as_deref(), Some("time "));
        assert_eq!(first.accepted_utf16_units, 5);
        assert_eq!(first.remainder, "and help.");
        let mut next = AcceptancePlan::new(
            "Thank you for your time ",
            "",
            &first.remainder,
            false,
            true,
            true,
        )
        .unwrap();
        assert_eq!(next.next_chunk().as_deref(), Some("and "));
        assert_eq!(next.remainder, "help.");
        let mut last = AcceptancePlan::new(
            "Thank you for your time and ",
            "",
            &next.remainder,
            false,
            true,
            true,
        )
        .unwrap();
        assert_eq!(last.next_chunk().as_deref(), Some("help. "));
        let mut midline = AcceptancePlan::new("one ", ", two", "word", false, true, true).unwrap();
        assert_eq!(midline.next_chunk().as_deref(), Some("word"));
        let mut all = AcceptancePlan::new("one ", "", "word", true, true, true).unwrap();
        assert_eq!(all.next_chunk().as_deref(), Some("word"));
    }
}
