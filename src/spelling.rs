// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Candidate boundaries and correction validation, independent of macOS editor I/O.
use crate::acceptance::ReplacementPlan;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Clone, Debug)]
pub struct WordCandidate {
    pub word: String,
    pub context: String,
    pub context_start_utf16: usize,
    pub word_utf16: usize,
    start_byte: usize,
    trailing: String,
}
#[derive(Clone, Debug)]
pub struct Correction {
    pub word: String,
    pub corrected: String,
    pub plan: ReplacementPlan,
}
fn letter(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c) || matches!(c, '\'' | '’')
}
impl WordCandidate {
    pub fn new(prefix: &str, suffix: &str) -> Option<Self> {
        let end = prefix.trim_end_matches([' ', '\u{a0}', '\u{202f}']).len();
        let trailing = &prefix[end..];
        if trailing.is_empty() && suffix.starts_with(letter) {
            return None;
        }
        if trailing.chars().count() > 3 {
            return None;
        }
        let start = prefix[..end]
            .char_indices()
            .rev()
            .find(|(_, c)| !letter(*c))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = &prefix[start..end];
        if !(4..=48).contains(&word.chars().count())
            || word.chars().all(|c| !c.is_lowercase())
            || word.starts_with(['\'', '’'])
            || word.ends_with(['\'', '’'])
            || prefix[..start].ends_with(['@', '/', '.', ':', '_', '-'])
        {
            return None;
        }
        let before = &prefix[..start];
        let count = before.chars().count();
        let context_start = before
            .char_indices()
            .nth(count.saturating_sub(256))
            .map_or(0, |(i, _)| i);
        let after: String = suffix.chars().take(128).collect();
        Some(Self {
            word: word.into(),
            context: format!("{}{after}", &prefix[context_start..]),
            context_start_utf16: prefix[context_start..start].encode_utf16().count(),
            word_utf16: word.encode_utf16().count(),
            start_byte: start,
            trailing: trailing.into(),
        })
    }
    pub fn corrected(&self, prefix: &str, suffix: &str, corrected: &str) -> Option<Correction> {
        if corrected.is_empty()
            || corrected.chars().count() > 80
            || corrected
                .chars()
                .any(|c| c.is_control() || !(letter(c) || c == ' '))
        {
            return None;
        }
        let original: String = self.word.nfc().flat_map(char::to_lowercase).collect();
        let proposed: String = corrected.nfc().flat_map(char::to_lowercase).collect();
        // Ordinary word completion is handled by the language model, not a typo fix.
        if proposed.starts_with(&original) {
            return None;
        }
        let plan = ReplacementPlan::new(
            prefix,
            suffix,
            self.start_byte,
            &format!("{corrected}{}", self.trailing),
        )?;
        Some(Correction {
            word: self.word.clone(),
            corrected: corrected.into(),
            plan,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typo_boundaries_preserve_whitespace_suffix_and_partial_words() {
        let c = WordCandidate::new("Please recieve  ", "the file.").unwrap();
        assert_eq!(c.word, "recieve");
        assert_eq!(c.context_start_utf16, 7);
        assert_eq!(
            c.corrected("Please recieve  ", "the file.", "receive")
                .unwrap()
                .plan
                .preview(),
            "Please receive  the file."
        );
        for (before, after) in [
            ("hel", ""),
            ("ABCD", ""),
            ("user@exmaple", ""),
            ("https://exmaple", ""),
            ("foo.barz", ""),
            ("recie", "ve"),
            ("recieve    ", ""),
        ] {
            assert!(
                WordCandidate::new(before, after).is_none(),
                "{before:?}|{after:?}"
            );
        }
        let partial = WordCandidate::new("We should sched", "").unwrap();
        assert!(partial
            .corrected("We should sched", "", "schedule")
            .is_none());
        assert!(c
            .corrected("Please recieve  ", "the file.", "receive\n")
            .is_none());
        let unicode = WordCandidate::new("🙂 cafe\u{301} recieve", "").unwrap();
        assert_eq!(unicode.context_start_utf16, 9);
    }
}
