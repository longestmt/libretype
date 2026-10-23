// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Local Unicode emoji catalog and shortcode replacement planning.
use crate::acceptance::ReplacementPlan;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Entry {
    emoji: String,
    aliases: Vec<String>,
    description: String,
    tags: Vec<String>,
}
fn catalog() -> &'static [Entry] {
    static DATA: OnceLock<Vec<Entry>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../vendor/gemoji/emoji.json"))
            .expect("bundled gemoji catalog must be valid")
    })
}
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}
#[derive(Debug, Clone)]
pub struct Query {
    start: usize,
    token: String,
    query: String,
    closed: bool,
}
impl Query {
    pub fn parse(prefix: &str, suffix: &str) -> Option<Self> {
        if suffix.starts_with(token_char) || suffix.starts_with(':') {
            return None;
        }
        let closed = prefix.ends_with(':') && prefix.len() > 1;
        let body = if closed {
            &prefix[..prefix.len() - 1]
        } else {
            prefix
        };
        // A lone colon starts a picker; a final colon also closes a named code.
        let (start, closed) = match body.rfind(':') {
            Some(start) if body[start + 1..].chars().all(token_char) => (start, closed),
            _ if prefix.ends_with(':') => (prefix.len() - 1, false),
            _ => return None,
        };
        let before = &prefix[..start];
        if !before.is_empty()
            && !before
                .ends_with(|c: char| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\''))
        {
            return None;
        }
        let end = prefix.len() - usize::from(closed);
        let query = &prefix[start + 1..end];
        if query.len() > 64 || !query.chars().all(token_char) || (closed && query.is_empty()) {
            return None;
        }
        Some(Self {
            start,
            token: prefix[start..].into(),
            query: query.to_ascii_lowercase(),
            closed,
        })
    }
    pub fn replacement(&self, prefix: &str, suffix: &str, emoji: &str) -> Option<ReplacementPlan> {
        if prefix.get(self.start..)? != self.token {
            return None;
        }
        ReplacementPlan::new(prefix, suffix, self.start, emoji)
    }
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub emoji: &'static str,
    pub alias: &'static str,
}
#[derive(Clone, Debug)]
pub struct Choices {
    pub query: Query,
    pub matches: Vec<Choice>,
    pub selected: usize,
}
impl Choices {
    pub fn new(query: Query) -> Self {
        let common = [
            "smile", "+1", "heart", "joy", "tada", "rocket", "eyes", "pray", "fire", "sparkles",
        ];
        let mut ranked = Vec::new();
        for entry in catalog() {
            let best = entry
                .aliases
                .iter()
                .filter_map(|alias| {
                    let score = if query.query.is_empty() {
                        common.iter().position(|s| s == alias).unwrap_or(100)
                    } else if *alias == query.query {
                        0
                    } else if query.closed {
                        return None;
                    } else if alias.starts_with(&query.query) {
                        1
                    } else if alias.contains(&query.query) {
                        2
                    } else if entry.tags.iter().any(|tag| tag.starts_with(&query.query)) {
                        3
                    } else if entry
                        .description
                        .split_whitespace()
                        .any(|w| w.starts_with(&query.query))
                    {
                        4
                    } else {
                        return None;
                    };
                    Some((score, alias))
                })
                .min_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then(a.1.len().cmp(&b.1.len()))
                        .then(a.1.cmp(b.1))
                });
            if let Some((score, alias)) = best {
                ranked.push((
                    score,
                    Choice {
                        emoji: &entry.emoji,
                        alias,
                    },
                ));
            }
        }
        ranked.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.alias.len().cmp(&b.1.alias.len()))
                .then(a.1.alias.cmp(b.1.alias))
        });
        Self {
            query,
            matches: ranked.into_iter().map(|(_, choice)| choice).collect(),
            selected: 0,
        }
    }
    pub fn current(&self) -> Option<&Choice> {
        self.matches.get(self.selected)
    }
    pub fn step(&mut self, next: bool) {
        if !self.matches.is_empty() {
            self.selected = if next {
                (self.selected + 1) % self.matches.len()
            } else {
                (self.selected + self.matches.len() - 1) % self.matches.len()
            };
        }
    }
    pub fn display(&self) -> String {
        self.current()
            .map(|c| {
                format!(
                    "{} :{}:  {}/{}  ↑↓",
                    c.emoji,
                    c.alias,
                    self.selected + 1,
                    self.matches.len()
                )
            })
            .unwrap_or_default()
    }
    pub fn plan(&self, prefix: &str, suffix: &str) -> Option<ReplacementPlan> {
        self.query
            .replacement(prefix, suffix, self.current()?.emoji)
    }
    pub fn fitting_display(&self, width: f64, measure: impl Fn(&str) -> f64) -> String {
        let Some(choice) = self.current() else {
            return String::new();
        };
        let full = self.display();
        if measure(&full) <= width {
            return full;
        }
        // Only abbreviate the name. Never split an emoji sequence or accept an
        // emoji whose entire glyph sequence was not included in the preview.
        for end in (1..choice.alias.len()).rev() {
            if !choice.alias.is_char_boundary(end) {
                continue;
            }
            let text = format!(
                "{} :{}…:  {}/{}  ↑↓",
                choice.emoji,
                &choice.alias[..end],
                self.selected + 1,
                self.matches.len()
            );
            if measure(&text) <= width {
                return text;
            }
        }
        let compact = format!(
            "{} {}/{} ↑↓",
            choice.emoji,
            self.selected + 1,
            self.matches.len()
        );
        if measure(&compact) <= width {
            compact
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_preview_never_splits_a_joined_emoji() {
        let choices = Choices::new(Query::parse(":woman_technologist", "").unwrap());
        let measure = |s: &str| s.chars().count() as f64;
        let fitted = choices.fitting_display(18., measure);
        assert!(fitted.starts_with("👩‍💻 "));
        assert!(measure(&fitted) <= 18.);
        assert!(fitted.contains('…'));
        assert!(choices.fitting_display(3., measure).is_empty());
    }
    #[test]
    fn shortcode_boundaries_do_not_claim_urls_times_or_partial_tokens() {
        for prefix in [
            ":",
            "Thanks :smi",
            "🙂 (:heart",
            ":+1",
            ":-1:",
            "\n:rocket:",
        ] {
            assert!(Query::parse(prefix, " ").is_some(), "{prefix}");
        }
        for prefix in [
            "12:30",
            "https:",
            "https://host:smi",
            "word:smi",
            "::",
            ":smile ",
            "user@:smile",
            ":smile\n",
            ":smilé",
        ] {
            assert!(Query::parse(prefix, "").is_none(), "{prefix}");
        }
        assert!(Query::parse(":sm", "ile:").is_none());
        assert!(Query::parse(":smile", ":").is_none());
        assert!(Query::parse(&format!(":{}", "a".repeat(65)), "").is_none());
    }
    #[test]
    fn catalog_search_ranks_exact_aliases_and_cycles_all_choices() {
        let choices = Choices::new(Query::parse(":smile", "").unwrap());
        assert_eq!(choices.current().unwrap().alias, "smile");
        assert_eq!(choices.current().unwrap().emoji, "😄");
        let closed = Choices::new(Query::parse(":thumbsup:", "").unwrap());
        assert_eq!(closed.matches.len(), 1);
        assert_eq!(closed.current().unwrap().emoji, "👍");
        assert!(Choices::new(Query::parse(":not_an_emoji", "").unwrap())
            .matches
            .is_empty());
        let mut choices = Choices::new(Query::parse(":", "").unwrap());
        assert_eq!(choices.matches.len(), catalog().len());
        assert_eq!(choices.current().unwrap().alias, "smile");
        choices.step(false);
        assert_eq!(choices.selected, choices.matches.len() - 1);
        choices.step(true);
        assert_eq!(choices.selected, 0);
        let mut aliases = std::collections::HashSet::new();
        for entry in catalog() {
            assert!(!entry.emoji.is_empty() && !entry.emoji.chars().any(char::is_control));
            for alias in &entry.aliases {
                assert!(aliases.insert(alias));
                assert!(alias.chars().all(token_char));
                let lookup = Choices::new(Query::parse(&format!(":{alias}:"), "").unwrap());
                assert_eq!(lookup.current().unwrap().emoji, entry.emoji);
            }
        }
    }
    #[test]
    fn replacements_preserve_unicode_suffix_and_require_unchanged_selection() {
        let before = "é🙂 :woman_technologist:";
        let after = " and more";
        let choices = Choices::new(Query::parse(before, after).unwrap());
        let plan = choices.plan(before, after).unwrap();
        assert_eq!(plan.preview(), "é🙂 👩‍💻 and more");
        assert_eq!(plan.start_utf16, 4);
        assert!(plan.selection_matches(&format!("{before}{after}"), 4, 20));
        assert!(!plan.selection_matches(&format!("{before}{after}"), 3, 20));
        assert!(choices.plan("other :woman_technologist:", after).is_none());
        let mut insertion = plan.insertion(true).unwrap();
        assert_eq!(insertion.next_chunk().unwrap(), "👩‍💻");
        assert_eq!(
            insertion.observe("é🙂 👩‍💻", after),
            crate::acceptance::Observation::Confirmed
        );
    }
}
