// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Bounded requests and validated edits for continuation choices and synonyms.
use crate::acceptance::ReplacementPlan;
use serde::{Deserialize, Serialize};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealMode {
    Off,
    Automatic,
    Shortcut,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ChoiceSettings {
    pub alternatives: RevealMode,
    pub synonyms: RevealMode,
    pub delay_ms: u64,
}
impl Default for ChoiceSettings {
    fn default() -> Self {
        Self {
            alternatives: RevealMode::Off,
            synonyms: RevealMode::Automatic,
            delay_ms: 800,
        }
    }
}
impl ChoiceSettings {
    /// Automatic choices must not replace a continuation the user is accepting
    /// word by word. Explicit reveal requests use `Request::mode` instead.
    pub fn automatic_due(
        &self,
        selected: bool,
        elapsed: std::time::Duration,
        preserving_remainder: bool,
    ) -> bool {
        let mode = if selected {
            self.synonyms
        } else {
            self.alternatives
        };
        !preserving_remainder
            && mode == RevealMode::Automatic
            && elapsed >= std::time::Duration::from_millis(self.delay_ms)
    }
}
#[derive(Clone, Debug)]
pub struct Request {
    pub id: u64,
    pub prefix: String,
    pub selected: String,
    pub suffix: String,
    pub app_id: String,
    pub website: Option<String>,
    pub profile: Option<crate::history::Profile>,
    pub background: crate::background::Background,
    pub candidates: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub label: String,
    pub insertion: String,
}
impl Request {
    pub fn is_synonym(&self) -> bool {
        !self.selected.is_empty()
    }
    pub fn mode(&self, settings: &ChoiceSettings) -> RevealMode {
        if self.is_synonym() {
            settings.synonyms
        } else {
            settings.alternatives
        }
    }
    pub fn started_word(&self) -> String {
        if self.is_synonym() {
            String::new()
        } else {
            fragment(&self.prefix).nfc().collect()
        }
    }
    pub fn valid(&self) -> bool {
        if self.is_synonym() {
            let word = self.selected.trim();
            !word.is_empty()
                && self.selected.chars().count() <= 100
                && word.split_whitespace().count() <= 8
                && self
                    .selected
                    .chars()
                    .all(|c| letter(c) || c == ' ' || c == '\u{a0}' || c == '\u{202f}')
        } else {
            !self.prefix.trim().is_empty()
                && (!self.suffix.starts_with(letter) || self.prefix.ends_with(char::is_whitespace))
                && fragment(&self.prefix).chars().count() <= 48
        }
    }
    pub fn prompt(&self, context_chars: usize, instructions: &str) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let fragment = if self.is_synonym() {
            ""
        } else {
            fragment(&self.prefix)
        };
        let before = &self.prefix[..self.prefix.len() - fragment.len()];
        let budget = context_chars.clamp(200, 4000);
        let before: String = before
            .chars()
            .rev()
            .take(budget * 2 / 3)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let after: String = self.suffix.chars().take(budget / 3).collect();
        let task = if self.is_synonym() {
            format!("Find three to five synonyms for the selected text {} in this sentence: {}. The replacement must preserve its meaning, grammatical role, language and capitalization. Return only the replacement words or short phrases, without the surrounding sentence.",serde_json::json!(self.selected.trim()),serde_json::json!(format!("{before}{}{after}",self.selected)))
        } else if !fragment.is_empty() {
            format!("Autocomplete the unfinished word {}. Every answer must start with ALL of these letters, in this order: {}. Give three to five complete words, each longer than the unfinished word. For example, mar can become market, march, marvel. Prefer words that fit this context: {}. Text after cursor: {}.",serde_json::json!(fragment),fragment.chars().map(|c|c.to_string()).collect::<Vec<_>>().join("-"),serde_json::json!(format!("{before}{fragment}")),serde_json::json!(after))
        } else {
            format!("Give three to five different words that could fill the blank in this text: {}. Each choice must make a grammatically correct sentence at that exact position. Only a SINGLE WORD for each choice. Keep the language of the text. Do not repeat the sentence.",serde_json::json!(format!("{before}___{after}")))
        };
        Some(format!("{task}\nWriting preferences: {}\nReturn JSON with a choices array of nonempty words. No explanations, numbering or punctuation. Allowed dictionary candidates (when supplied, choose only from these): {}",serde_json::json!(instructions),serde_json::json!(self.candidates)))
    }

    pub fn finish(&self, raw: &str) -> Vec<Choice> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Response {
            choices: Vec<String>,
        }
        if !self.valid() || raw.len() > 8192 {
            return vec![];
        }
        let Ok(response) = serde_json::from_str::<Response>(raw) else {
            return vec![];
        };
        if response.choices.len() > 20 {
            return vec![];
        }
        let mut choices = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for proposed in response.choices {
            if proposed.chars().any(char::is_control) {
                continue;
            }
            let word: String = proposed.trim().nfc().collect();
            if word.is_empty()
                || word.chars().count() > 80
                || word.chars().any(|c| !(letter(c) || c == ' '))
                || !word.chars().any(char::is_alphabetic)
                || !word.starts_with(char::is_alphabetic)
                || !word.ends_with(|c: char| c.is_alphabetic() || is_combining_mark(c))
            {
                continue;
            }
            if !self.candidates.is_empty() && !self.candidates.iter().any(|s| s == &word) {
                continue;
            }
            let lower = word.to_lowercase();
            if !seen.insert(lower.clone()) {
                continue;
            }
            let insertion = if self.is_synonym() {
                if word.split_whitespace().count() > 8
                    || lower
                        == self
                            .selected
                            .trim()
                            .nfc()
                            .collect::<String>()
                            .to_lowercase()
                {
                    continue;
                }
                let lead_len = self.selected.len() - self.selected.trim_start().len();
                let tail = &self.selected[self.selected.trim_end().len()..];
                format!("{}{word}{tail}", &self.selected[..lead_len])
            } else {
                if word.contains(' ') {
                    continue;
                }
                let typed: String = fragment(&self.prefix).nfc().collect();
                let mut generated = word.char_indices();
                let mut end = 0;
                let mut matching = true;
                for c in typed.chars() {
                    match generated.next() {
                        Some((i, next)) if c.to_lowercase().eq(next.to_lowercase()) => {
                            end = i + next.len_utf8()
                        }
                        _ => {
                            matching = false;
                            break;
                        }
                    }
                }
                if !matching || end == word.len() {
                    continue;
                }
                let leading = if typed.is_empty()
                    && !self.prefix.ends_with(char::is_whitespace)
                    && !self.prefix.ends_with(['(', '[', '{', '"', '“'])
                {
                    " "
                } else {
                    ""
                };
                format!(
                    "{leading}{}{}",
                    &word[end..],
                    if self.suffix.starts_with(letter) {
                        " "
                    } else {
                        ""
                    }
                )
            };
            choices.push(Choice {
                label: word,
                insertion,
            });
            if choices.len() == 5 {
                break;
            }
        }
        choices
    }
    pub fn replacement(&self, choice: &Choice) -> Option<ReplacementPlan> {
        self.is_synonym()
            .then(|| {
                ReplacementPlan::new(
                    &format!("{}{}", self.prefix, self.selected),
                    &self.suffix,
                    self.prefix.len(),
                    &choice.insertion,
                )
            })
            .flatten()
    }
}
fn letter(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c) || matches!(c, '\'' | '’' | '-')
}
fn fragment(prefix: &str) -> &str {
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| !letter(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    &prefix[start..]
}
#[derive(Clone, Debug)]
pub struct Choices {
    pub request: Request,
    pub items: Vec<Choice>,
    pub selected: usize,
}
impl Choices {
    pub fn lines(&self) -> String {
        self.items
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "{} {}  {}",
                    if i == self.selected { "›" } else { " " },
                    i + 1,
                    c.label
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn step(&mut self, next: bool) {
        if !self.items.is_empty() {
            self.selected = if next {
                (self.selected + 1) % self.items.len()
            } else {
                (self.selected + self.items.len() - 1) % self.items.len()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceptance::{AcceptancePlan, Observation};
    use std::time::Duration;
    fn request(prefix: &str, selected: &str, suffix: &str) -> Request {
        Request {
            id: 1,
            prefix: prefix.into(),
            selected: selected.into(),
            suffix: suffix.into(),
            app_id: "test".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
            candidates: vec![],
        }
    }
    #[test]
    fn automatic_choices_wait_until_word_acceptance_remainder_is_exhausted() {
        let settings = ChoiceSettings {
            alternatives: RevealMode::Automatic,
            ..ChoiceSettings::default()
        };
        let elapsed = Duration::from_secs(30);
        let mut prefix = "We should ".to_owned();
        let mut suggestion = "meet tomorrow morning.".to_owned();
        for expected in ["meet", " tomorrow", " morning."] {
            let mut plan =
                AcceptancePlan::new(&prefix, "", &suggestion, false, true, false).unwrap();
            let chunk = plan.next_chunk().unwrap();
            assert_eq!(chunk, expected);
            prefix.push_str(&chunk);
            assert_eq!(plan.observe(&prefix, ""), Observation::Confirmed);
            assert!(plan.next_chunk().is_none());
            suggestion = plan.remainder;
            let preserving_remainder = !suggestion.is_empty();
            assert_eq!(
                settings.automatic_due(false, elapsed, preserving_remainder),
                !preserving_remainder,
                "an idle pause after Tab must not replace the remaining words"
            );
            // Explicit reveal remains enabled even during word-by-word acceptance.
            assert_eq!(
                request(&prefix, "", "").mode(&settings),
                RevealMode::Automatic
            );
        }
        assert_eq!(prefix, "We should meet tomorrow morning.");
        // After a genuine edit/focus change clears the remainder, the usual
        // delay applies again; off/shortcut modes still never reveal on a timer.
        assert!(!settings.automatic_due(false, Duration::from_millis(799), false));
        assert!(settings.automatic_due(false, Duration::from_millis(800), false));
        for mode in [RevealMode::Off, RevealMode::Shortcut] {
            let settings = ChoiceSettings {
                alternatives: mode,
                ..settings.clone()
            };
            assert!(!settings.automatic_due(false, elapsed, false));
            assert!(settings.automatic_due(true, elapsed, false));
        }
    }
    #[test]
    fn synonyms_replace_only_selection_and_preserve_unicode_and_spaces() {
        let r = request("🙂 That was ", "great ", "news.");
        let v = r.finish(
            r#"{"choices":["wonderful","excellent","great","Wonderful","bad\ntext","<tag>"]}"#,
        );
        assert_eq!(v.len(), 2);
        let p = r.replacement(&v[0]).unwrap();
        assert_eq!(p.preview(), "🙂 That was wonderful news.");
        assert!(p.selection_matches("🙂 That was great news.", 12, 6));
        assert!(!p.selection_matches("🙂 That was great news.", 12, 5));
        assert!(!request("", "one\ntwo", "").valid());
        assert!(!request("", "   ", "").valid());
        assert!(!request("", &"a".repeat(101), "").valid());
    }
    #[test]
    fn alternative_words_finish_typed_fragment_without_rewriting_it() {
        let r = request("We should sche", "", "");
        let v =
            r.finish(r#"{"choices":["schedule","scheme","arrange","schedule a meeting","sche"]}"#);
        assert_eq!(
            v.iter().map(|c| c.insertion.as_str()).collect::<Vec<_>>(),
            ["dule", "me"]
        );
        let r = request("Thanks for your ", "", "");
        assert_eq!(
            r.finish(r#"{"choices":["help","support"]}"#)[0].insertion,
            "help"
        );
        assert!(!request("hel", "", "lo").valid());
        let r = request("Äu", "", "");
        assert_eq!(r.finish(r#"{"choices":["äußerst"]}"#)[0].insertion, "ßerst");
        let r = request("cafe\u{301}", "", "");
        assert_eq!(r.finish(r#"{"choices":["caféine"]}"#)[0].insertion, "ine");
    }
    #[test]
    fn dictionary_choices_reject_invented_words_and_keep_midline_separator() {
        let mut r = request("We should sche", "", "");
        r.candidates = vec!["schedule".into(), "scheme".into()];
        let choices = r.finish(r#"{"choices":["schealth","schedule","scheme"]}"#);
        assert_eq!(
            choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["schedule", "scheme"]
        );
        let r = request("We should ", "", "tomorrow.");
        assert_eq!(r.finish(r#"{"choices":["meet"]}"#)[0].insertion, "meet ");
    }
    #[test]
    fn choices_reject_untrusted_output_and_bound_prompt() {
        let r = request(&"a ".repeat(5000), "good", "end");
        assert!(r.finish("not json").is_empty());
        assert!(r.finish(r#"{"choices":["fine"],"extra":"x"}"#).is_empty());
        assert!(r
            .finish(r#"{"choices":["1. fine","fine\n","fine\ttext","Fine."]}"#)
            .iter()
            .all(|c| c.label == "fine"));
        assert!(r.prompt(1200, "Use simple words.").unwrap().len() < 2000);
        assert!(r
            .prompt(1200, "Use simple words.")
            .unwrap()
            .contains("Use simple words."));
    }
}
