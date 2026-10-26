// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Validate model-selected vocabulary against the bounded, eligible samples.
//! Repetition within one sample never supplies independent support.
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use zeroize::Zeroizing;

pub const MAX_ENTRIES: usize = 16;
pub const MAX_ENTRY_CHARS: usize = 64;
pub const MAX_TOTAL_CHARS: usize = 512;

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .flat_map(char::to_lowercase)
        .collect()
}
fn entry_valid(text: &str) -> bool {
    !text.is_empty()
        && text.trim() == text
        && text.chars().count() <= MAX_ENTRY_CHARS
        && text.split_whitespace().count() <= 6
        && text.chars().any(char::is_alphabetic)
        && !text.contains("  ")
        // Complete greetings and sign-offs can contain sentence punctuation;
        // supported() still requires the exact phrase in two distinct samples.
        && text.chars().all(|c| {
            c.is_alphanumeric()
                || is_combining_mark(c)
                || matches!(
                    c,
                    ' ' | '-' | '_' | '.' | ',' | '!' | '?' | '+' | '#' | '/' | '\'' | '’'
                )
        })
}
pub fn valid(entries: &[String]) -> bool {
    if entries.len() > MAX_ENTRIES
        || entries.iter().map(|s| s.chars().count()).sum::<usize>() > MAX_TOTAL_CHARS
    {
        return false;
    }
    let mut seen = Zeroizing::new(Vec::<String>::new());
    for entry in entries {
        if !entry_valid(entry) {
            return false;
        }
        let key = Zeroizing::new(normalized(entry));
        if seen.iter().any(|prior| prior == key.as_str()) {
            return false;
        }
        seen.push(key.to_string());
    }
    true
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c) || c == '_'
}
fn boundary(mut neighbors: impl Iterator<Item = char>, before: bool) -> bool {
    match neighbors.next() {
        Some(c) if word(c) => false,
        Some('+' | '#') => false,
        Some('.') if before => false,
        // Punctuation can join a technical token/contact address, but a period
        // at the end of a sentence is still a boundary.
        Some('-' | '.' | '/' | '\'' | '’' | '@' | ':') => !neighbors.next().is_some_and(word),
        _ => true,
    }
}
fn contains_phrase(sample: &str, phrase: &str) -> bool {
    sample.match_indices(phrase).any(|(start, _)| {
        boundary(sample[..start].chars().rev(), true)
            && boundary(sample[start + phrase.len()..].chars(), false)
    })
}
/// Unsupported, duplicated or malformed candidates are omitted, not guessed.
pub fn supported(candidates: &[String], samples: &[String]) -> Vec<String> {
    let mut normalized_samples = Zeroizing::new(Vec::<String>::new());
    for sample in samples {
        let sample = Zeroizing::new(normalized(sample));
        if !normalized_samples.iter().any(|s| s == sample.as_str()) {
            normalized_samples.push(sample.to_string());
        }
    }
    let mut result = Vec::<String>::new();
    let mut seen = Zeroizing::new(Vec::<String>::new());
    let mut size = 0;
    for candidate in candidates.iter().take(MAX_ENTRIES) {
        let candidate = Zeroizing::new(candidate.trim().nfc().collect::<String>());
        if !entry_valid(&candidate) {
            continue;
        }
        let key = Zeroizing::new(normalized(&candidate));
        if seen.iter().any(|s| s == key.as_str())
            || normalized_samples
                .iter()
                .filter(|s| contains_phrase(s, &key))
                .take(2)
                .count()
                < 2
        {
            continue;
        }
        let length = candidate.chars().count();
        if size + length > MAX_TOTAL_CHARS {
            continue;
        }
        size += length;
        seen.push(key.to_string());
        result.push(candidate.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn recurring_phrases_keep_punctuation_without_relaxing_source_checks() {
        let samples = strings(&[
            "Hello, Niamh! Are you ready? All set. Kind regards, Niamh. Contact niamh@example.invalid. One time only!",
            "Hello, Niamh! Are you ready? The work is ready. Kind regards, Niamh. Email niamh@example.invalid.",
        ]);
        let entries = supported(
            &strings(&[
                "Hello, Niamh!",
                "Are you ready?",
                "Kind regards,",
                "One time only!",
                "Invented farewell!",
                "niamh@example.invalid",
                "example",
                "<think>!",
                "Hello,\nNiamh!",
            ]),
            &samples,
        );
        assert_eq!(
            entries,
            strings(&["Hello, Niamh!", "Are you ready?", "Kind regards,"])
        );
        assert!(valid(&entries));
        assert!(supported(&strings(&["Cheers!"]), &strings(&["Cheers!", "CHEERS!"])).is_empty());
    }
    #[test]
    fn requires_two_distinct_samples_not_repetition_or_invented_terms() {
        let samples = strings(&[
            "Ariadne reviews the photonic lattice. Keep the photons dancing.",
            "We asked Ariadne about the photonic lattice. Keep the photons dancing.",
            "Oneoff Oneoff Oneoff is mentioned only here.",
        ]);
        let candidates = strings(&[
            "Ariadne",
            "photonic lattice",
            "Keep the photons dancing",
            "Oneoff",
            "Invented",
            "ARIADNE",
        ]);
        assert_eq!(
            supported(&candidates, &samples),
            strings(&["Ariadne", "photonic lattice", "Keep the photons dancing"])
        );
        assert!(supported(
            &strings(&["Ariadne"]),
            &strings(&["Ariadne is here", "ARIADNE   IS HERE"])
        )
        .is_empty());
    }
    #[test]
    fn unicode_normalization_and_token_boundaries_preserve_names_and_technical_terms() {
        let samples = strings(&[
            "Zoë uses C++ with .NET. Ariadne tested the photonic lattice.",
            "We asked Zoë about .NET and C++. The photonic lattice works.",
        ]);
        assert_eq!(
            supported(
                &strings(&["Zoë", "C++", ".NET", "C", "NET", "latt", "Ariadne"]),
                &samples
            ),
            strings(&["Zoë", "C++", ".NET"])
        );
        let contacts = strings(&[
            "Contact ariadne@example.invalid for a test.",
            "Try ariadne@example.invalid on Friday.",
        ]);
        assert!(supported(
            &strings(&["ariadne", "example", "ariadne@example.invalid"]),
            &contacts
        )
        .is_empty());
    }
    #[test]
    fn entries_and_combined_vocabulary_are_bounded_and_invalid_text_is_omitted() {
        assert!(!valid(&strings(&["a\nb", "<think>"])));
        assert!(!valid(&strings(&["Ariadne", "ARIADNE"])));
        assert!(!valid(&vec!["word".into(); MAX_ENTRIES + 1]));
        let long = "a".repeat(MAX_ENTRY_CHARS + 1);
        assert!(!valid(&[long]));
        let candidates: Vec<_> = (0..MAX_ENTRIES)
            .map(|i| format!("Term{i}{}", "x".repeat(55)))
            .collect();
        let first = candidates.join(" ");
        let second = format!("Other sample: {first}");
        let chosen = supported(&candidates, &[first, second]);
        assert!(valid(&chosen));
        assert!(!chosen.is_empty());
        assert!(chosen.len() < candidates.len());
        assert!(chosen.iter().map(|s| s.chars().count()).sum::<usize>() <= MAX_TOTAL_CHARS);
    }
}
