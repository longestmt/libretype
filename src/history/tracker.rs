// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Own only newly inserted/replaced spans. A field's initial contents are a
//! transient baseline, never a history record. The caller supplies a stable field
//! token after verifying native focused-element identity and secure-input state.
use super::{new_id, Error, Record, RecordId, Scope, Settings, MAX_TEXT};
use crate::config::Config;
use ring::digest::{digest, SHA256};
use zeroize::Zeroizing;
const MAX_EDIT_CHARS: usize = 32_000;
// Matches the history worker's maximum append batch. Oldest unqualified spans
// are discarded when this limit is reached; nothing is written to disk early.
const MAX_DEFERRED_RECORDS: usize = 16;

pub struct Observation<'a> {
    pub field: u64,
    pub scope: Scope,
    pub text: &'a str,
    /// UTF-8 selection start, not an AX UTF-16 offset.
    pub caret: usize,
    pub selection_end: usize,
    pub now: u64,
}
struct Span {
    start: usize,
    end: usize,
    id: RecordId,
    created_at: u64,
    last_hash: Option<[u8; 32]>,
}
struct Buffer {
    field: u64,
    scope: Scope,
    text: Zeroizing<String>,
    span: Option<Span>,
    changed: u64,
    selection: (usize, usize),
    accepted: bool,
    deferred: Vec<Record>,
}
#[derive(Default)]
pub struct Tracker {
    buffer: Option<Buffer>,
}
impl Tracker {
    /// Text-free state for diagnosing ownership loss in the native adapter.
    pub fn diagnostic(&self) -> serde_json::Value {
        let buffer = self.buffer.as_ref();
        serde_json::json!({
            "baseline": buffer.is_some(),
            "owned_nonwhitespace_chars": buffer.and_then(|b| b.span.as_ref().map(|s|
                b.text[s.start..s.end].chars().filter(|c| !c.is_whitespace()).count())).unwrap_or(0),
            "deferred_records": buffer.map_or(0, |b| b.deferred.len()),
            "accepted": buffer.is_some_and(|b| b.accepted),
        })
    }

    pub fn text_matches(&self, text: &str) -> bool {
        self.buffer
            .as_ref()
            .is_some_and(|buffer| buffer.text.as_str() == text)
    }
    /// Finish only an already-owned human span, then treat the supplied state as
    /// a baseline. Programmatic changes never become new owned writing.
    pub fn observe_passive(
        &mut self,
        config: &Config,
        settings: &Settings,
        observation: Observation<'_>,
    ) -> Result<Vec<Record>, Error> {
        let previous = self.finish(config, settings, observation.now)?;
        let mut records = self.observe(config, settings, observation)?;
        records.extend(previous);
        Ok(records)
    }
    /// Our insertion changes the baseline but preserves this field's session.
    /// It does not count as acceptance until confirm_acceptance is called.
    pub fn observe_generated(
        &mut self,
        config: &Config,
        settings: &Settings,
        observation: Observation<'_>,
    ) -> Result<Vec<Record>, Error> {
        if let Some(buffer) = self.buffer.as_mut().filter(|buffer| {
            buffer.field == observation.field
                && buffer.scope == observation.scope
                && settings.allows_recording(config, &buffer.scope)
                && observation.text.is_char_boundary(observation.caret)
                && observation.text.is_char_boundary(observation.selection_end)
                && observation.selection_end >= observation.caret
        }) {
            let previous = emit(buffer, settings, observation.now)?;
            buffer.text = Zeroizing::new(observation.text.into());
            buffer.span = None;
            buffer.changed = observation.now;
            buffer.selection = (observation.caret, observation.selection_end);
            if observation.text.is_empty() {
                buffer.accepted = false;
                buffer.deferred.clear();
            }
            return Ok(previous.into_iter().collect());
        }
        self.observe_passive(config, settings, observation)
    }
    pub fn clear(&mut self) {
        self.buffer = None;
    }
    /// Only call after the insertion transaction confirms text, caret and focus.
    /// Key presses, posted insertion events and failed transactions do not qualify.
    pub fn confirm_acceptance(
        &mut self,
        config: &Config,
        settings: &Settings,
        field: u64,
        scope: &Scope,
        now: u64,
    ) -> Result<Vec<Record>, Error> {
        let Some(buffer) = self.buffer.as_mut().filter(|buffer| {
            buffer.field == field
                && &buffer.scope == scope
                && settings.allows_recording(config, &buffer.scope)
        }) else {
            return Ok(vec![]);
        };
        let latest = emit(buffer, settings, now)?;
        buffer.accepted = true;
        let mut records = std::mem::take(&mut buffer.deferred);
        records.extend(latest);
        Ok(records)
    }
    pub fn observe(
        &mut self,
        config: &Config,
        settings: &Settings,
        observation: Observation<'_>,
    ) -> Result<Vec<Record>, Error> {
        let Observation {
            field,
            scope,
            text,
            caret,
            selection_end,
            now,
        } = observation;
        if !settings.allows_recording(config, &scope)
            || field == 0
            || !text.is_char_boundary(caret)
            || !text.is_char_boundary(selection_end)
            || selection_end < caret
        {
            // Disabling recording drops uncommitted text. Changing focus to an
            // excluded field can finish only an earlier, still-permitted source.
            let old = self.finish(config, settings, now)?;
            return Ok(old.into_iter().collect());
        }
        let same = self
            .buffer
            .as_ref()
            .is_some_and(|b| b.field == field && b.scope == scope);
        if !same {
            let previous = self.finish(config, settings, now)?;
            self.buffer = Some(Buffer {
                field,
                scope,
                text: Zeroizing::new(text.into()),
                span: None,
                changed: now,
                selection: (caret, selection_end),
                accepted: false,
                deferred: vec![],
            });
            return Ok(previous.into_iter().collect());
        }
        let buffer = self.buffer.as_mut().unwrap();
        if buffer.text.as_str() == text {
            buffer.selection = (caret, selection_end);
            return Ok(vec![]);
        }
        let mut records = vec![];
        let edit = edit_range(&buffer.text, text);
        // A collapsed caret must confirm where the single edit ended. Ambiguous
        // repeated-text edits, undo of multiple regions, and bulk loads are skipped.
        let continuing = buffer
            .span
            .as_ref()
            .is_some_and(|span| edit.start >= span.start && edit.old_end <= span.end);
        let selected_replacement = buffer.selection.1 > buffer.selection.0
            && edit.start >= buffer.selection.0
            && edit.old_end <= buffer.selection.1;
        if edit.new_end != caret
            || selection_end != caret
            || text[edit.start..edit.new_end].chars().count() > MAX_EDIT_CHARS
            || (edit.old_end > edit.start && !continuing && !selected_replacement)
        {
            if let Some(record) = emit(buffer, settings, now)? {
                records.push(record);
            }
            buffer.span = None;
            buffer.text = Zeroizing::new(text.into());
            buffer.changed = now;
            buffer.selection = (caret, selection_end);
            if text.is_empty() {
                buffer.accepted = false;
                buffer.deferred.clear();
            }
            return Ok(records);
        }
        if continuing && !text.is_empty() {
            let span = buffer.span.as_mut().unwrap();
            span.end = span.end - (edit.old_end - edit.start) + (edit.new_end - edit.start);
        } else {
            if let Some(record) = emit(buffer, settings, now)? {
                records.push(record);
            }
            buffer.span = None;
            if edit.new_end > edit.start {
                buffer.span = Some(Span {
                    start: edit.start,
                    end: edit.new_end,
                    id: new_id()?,
                    created_at: now,
                    last_hash: None,
                });
            }
        }
        buffer.text = Zeroizing::new(text.into());
        buffer.changed = now;
        buffer.selection = (caret, selection_end);
        if text.is_empty() {
            buffer.accepted = false;
            buffer.deferred.clear();
        }
        // Long compositions become consecutive bounded records instead of
        // repeatedly truncating the same record or copying unrelated field text.
        while let Some(span) = buffer.span.as_ref() {
            let written = &buffer.text[span.start..span.end];
            let Some((offset, _)) = written.char_indices().nth(MAX_TEXT) else {
                break;
            };
            let split = span.start + offset;
            let end = span.end;
            buffer.span.as_mut().unwrap().end = split;
            if let Some(record) = emit(buffer, settings, now)? {
                records.push(record);
            }
            buffer.span = Some(Span {
                start: split,
                end,
                id: new_id()?,
                created_at: now,
                last_hash: None,
            });
        }
        Ok(records)
    }
    pub fn flush_idle(
        &mut self,
        config: &Config,
        settings: &Settings,
        now: u64,
        pause_seconds: u64,
    ) -> Result<Option<Record>, Error> {
        let Some(buffer) = self.buffer.as_mut() else {
            return Ok(None);
        };
        if !settings.allows_recording(config, &buffer.scope) {
            self.clear();
            return Ok(None);
        }
        if now.saturating_sub(buffer.changed) < pause_seconds {
            return Ok(None);
        }
        emit(buffer, settings, now)
    }
    pub fn finish(
        &mut self,
        config: &Config,
        settings: &Settings,
        now: u64,
    ) -> Result<Option<Record>, Error> {
        let Some(mut buffer) = self.buffer.take() else {
            return Ok(None);
        };
        if !settings.allows_recording(config, &buffer.scope) {
            return Ok(None);
        }
        emit(&mut buffer, settings, now)
    }
}
fn emit(buffer: &mut Buffer, settings: &Settings, now: u64) -> Result<Option<Record>, Error> {
    let Some(span) = buffer.span.as_mut() else {
        return Ok(None);
    };
    let text = &buffer.text[span.start..span.end];
    if text.trim().is_empty() {
        buffer.deferred.retain(|record| record.id != span.id);
        span.last_hash = None;
        return Ok(None);
    }
    let hash: [u8; 32] = digest(&SHA256, text.as_bytes())
        .as_ref()
        .try_into()
        .map_err(|_| Error::InvalidData)?;
    if span.last_hash == Some(hash) {
        return Ok(None);
    }
    let record = Record {
        id: span.id,
        scope: buffer.scope.clone(),
        created_at: span.created_at,
        updated_at: now.max(span.created_at),
        text: text.into(),
    };
    if !record.valid() {
        return Err(Error::InvalidData);
    }
    span.last_hash = Some(hash);
    if !settings.record_without_acceptance && !buffer.accepted {
        if let Some(existing) = buffer.deferred.iter_mut().find(|r| r.id == record.id) {
            *existing = record;
        } else {
            if buffer.deferred.len() == MAX_DEFERRED_RECORDS {
                buffer.deferred.remove(0);
            }
            buffer.deferred.push(record);
        }
        return Ok(None);
    }
    Ok(Some(record))
}
struct Edit {
    start: usize,
    old_end: usize,
    new_end: usize,
}
fn edit_range(before: &str, after: &str) -> Edit {
    let start = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = before[start..]
        .chars()
        .rev()
        .zip(after[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    Edit {
        start,
        old_end: before.len() - suffix,
        new_end: after.len() - suffix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            app: "com.apple.mail".into(),
            website: None,
        }
    }
    fn observation(text: &str, caret: usize, now: u64) -> Observation<'_> {
        Observation {
            field: 1,
            scope: scope(),
            text,
            caret,
            selection_end: caret,
            now,
        }
    }
    fn settings() -> Settings {
        Settings {
            recording: true,
            record_without_acceptance: true,
            ..Default::default()
        }
    }
    fn filtered_settings() -> Settings {
        Settings {
            recording: true,
            ..Default::default()
        }
    }
    fn type_text(tracker: &mut Tracker, config: &Config, settings: &Settings, text: &str) {
        tracker
            .observe(config, settings, observation("", 0, 1))
            .unwrap();
        assert!(tracker
            .observe(config, settings, observation(text, text.len(), 2))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn diagnostic_reports_ownership_without_exposing_text_or_scope() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        let text = "The Zephyr team will meet in Cedar Hall on Thursday morning.";
        type_text(&mut tracker, &config, &settings, text);
        tracker.flush_idle(&config, &settings, 10, 2).unwrap();
        let status = tracker.diagnostic();
        assert_eq!(
            status["owned_nonwhitespace_chars"],
            text.chars().filter(|c| !c.is_whitespace()).count()
        );
        assert_eq!(status["deferred_records"], 1);
        assert_eq!(status["accepted"], false);
        assert!(!status.to_string().contains("Zephyr"));
        assert!(!status.to_string().contains(&scope().app));
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 11)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(tracker.diagnostic()["accepted"], true);
        tracker.clear();
        assert_eq!(
            tracker.diagnostic(),
            serde_json::json!({
                "baseline": false, "owned_nonwhitespace_chars": 0,
                "deferred_records": 0, "accepted": false,
            })
        );
    }

    #[test]
    fn acceptance_filter_defaults_and_roundtrips_independently_of_recording() {
        let legacy: Settings = serde_json::from_str(r#"{"recording":true}"#).unwrap();
        assert!(legacy.recording);
        assert!(!legacy.record_without_acceptance);
        let defaults = Settings::default();
        assert!(!defaults.recording && !defaults.record_without_acceptance);
        let settings = Settings {
            record_without_acceptance: true,
            ..defaults
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.record_without_acceptance);
        assert!(!restored.recording);
    }
    #[test]
    fn idle_and_failed_insertions_do_not_qualify_a_session() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        type_text(&mut tracker, &config, &settings, "Human writing");
        assert!(tracker
            .flush_idle(&config, &settings, 10, 2)
            .unwrap()
            .is_none());
        // Posting an event and observing unchanged text is not verified acceptance.
        assert!(tracker
            .observe_generated(&config, &settings, observation("Human writing", 13, 11))
            .unwrap()
            .is_empty());
        assert!(tracker.finish(&config, &settings, 12).unwrap().is_none());
        assert!(tracker.buffer.is_none());
    }
    #[test]
    fn confirmed_acceptance_releases_human_spans_and_excludes_generated_words() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        type_text(&mut tracker, &config, &settings, "Human writing");
        assert!(tracker
            .flush_idle(&config, &settings, 10, 2)
            .unwrap()
            .is_none());
        let generated = "Human writing generated words";
        assert!(tracker
            .observe_generated(
                &config,
                &settings,
                observation(generated, generated.len(), 11)
            )
            .unwrap()
            .is_empty());
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 12)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "Human writing");
        assert!(tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 13)
            .unwrap()
            .is_empty());
        let continued = "Human writing generated words and more human writing";
        assert!(tracker
            .observe(
                &config,
                &settings,
                observation(continued, continued.len(), 14)
            )
            .unwrap()
            .is_empty());
        let record = tracker.finish(&config, &settings, 18).unwrap().unwrap();
        assert_eq!(record.text, " and more human writing");
    }
    #[test]
    fn deferred_updates_replace_prior_versions_and_are_bounded() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        type_text(&mut tracker, &config, &settings, "Early draft");
        tracker.flush_idle(&config, &settings, 5, 2).unwrap();
        tracker
            .observe(&config, &settings, observation("Final draft", 5, 6))
            .unwrap();
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 7)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "Final draft");
        tracker.clear();
        tracker
            .observe(&config, &settings, observation("", 0, 10))
            .unwrap();
        let mut text = String::new();
        for index in 0..MAX_DEFERRED_RECORDS + 5 {
            text.push_str(&format!(" human {index}"));
            assert!(tracker
                .observe(&config, &settings, observation(&text, text.len(), 11))
                .unwrap()
                .is_empty());
            text.push_str(" generated");
            assert!(tracker
                .observe_generated(&config, &settings, observation(&text, text.len(), 12))
                .unwrap()
                .is_empty());
        }
        assert_eq!(
            tracker.buffer.as_ref().unwrap().deferred.len(),
            MAX_DEFERRED_RECORDS
        );
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 20)
            .unwrap();
        assert_eq!(records.len(), MAX_DEFERRED_RECORDS);
        assert_eq!(records.first().unwrap().text, " human 5");
        assert!(records.iter().all(|r| !r.text.contains("generated")));
    }
    #[test]
    fn qualification_is_scoped_to_field_source_and_current_policy() {
        let config = Config::default();
        let mut settings = filtered_settings();
        let mut tracker = Tracker::default();
        type_text(&mut tracker, &config, &settings, "Pending writing");
        assert!(tracker
            .confirm_acceptance(&config, &settings, 2, &scope(), 3)
            .unwrap()
            .is_empty());
        let mut other = scope();
        other.website = Some("other.example".into());
        assert!(tracker
            .confirm_acceptance(&config, &settings, 1, &other, 4)
            .unwrap()
            .is_empty());
        settings.excluded_apps.push(scope().app.clone());
        assert!(tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 5)
            .unwrap()
            .is_empty());
        assert!(tracker.finish(&config, &settings, 6).unwrap().is_none());
        settings.excluded_apps.clear();
        type_text(&mut tracker, &config, &settings, "Allowed writing");
        assert_eq!(
            tracker
                .confirm_acceptance(&config, &settings, 1, &scope(), 7)
                .unwrap()
                .len(),
            1
        );
        let mut next = observation("", 0, 8);
        next.field = 2;
        assert!(tracker
            .observe(&config, &settings, next)
            .unwrap()
            .is_empty());
        let mut edit = observation("Unqualified new field", 21, 9);
        edit.field = 2;
        tracker.observe(&config, &settings, edit).unwrap();
        assert!(tracker.finish(&config, &settings, 10).unwrap().is_none());
    }
    #[test]
    fn composer_reset_or_external_load_starts_an_unqualified_session() {
        for passive in [false, true] {
            let config = Config::default();
            let settings = filtered_settings();
            let mut tracker = Tracker::default();
            type_text(&mut tracker, &config, &settings, "Approved writing");
            assert_eq!(
                tracker
                    .confirm_acceptance(&config, &settings, 1, &scope(), 3)
                    .unwrap()
                    .len(),
                1
            );
            let baseline = if passive { "Loaded document" } else { "" };
            if passive {
                tracker
                    .observe_passive(&config, &settings, observation(baseline, baseline.len(), 4))
                    .unwrap();
            } else {
                tracker
                    .observe(&config, &settings, observation(baseline, baseline.len(), 4))
                    .unwrap();
            }
            let next = format!("{baseline}Unqualified writing");
            tracker
                .observe(&config, &settings, observation(&next, next.len(), 5))
                .unwrap();
            assert!(tracker.finish(&config, &settings, 10).unwrap().is_none());
        }
    }
    #[test]
    fn unaccepted_long_unicode_writing_is_deferred_and_deleted_spans_are_dropped() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        let text = "🙂".repeat(MAX_TEXT + 20);
        type_text(&mut tracker, &config, &settings, &text);
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 3)
            .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records.iter().map(|r| r.text.as_str()).collect::<String>(),
            text
        );
        tracker.clear();
        type_text(&mut tracker, &config, &settings, "Delete me");
        tracker.flush_idle(&config, &settings, 5, 2).unwrap();
        tracker
            .observe(&config, &settings, observation("", 0, 6))
            .unwrap();
        assert!(tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 7)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn retyping_a_deleted_deferred_span_is_not_mistaken_for_a_saved_duplicate() {
        let config = Config::default();
        let settings = filtered_settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("Context: ", 9, 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("Context: Human", 14, 2))
            .unwrap();
        assert!(tracker
            .flush_idle(&config, &settings, 5, 2)
            .unwrap()
            .is_none());
        assert_eq!(tracker.buffer.as_ref().unwrap().deferred.len(), 1);
        tracker
            .observe(&config, &settings, observation("Context: ", 9, 6))
            .unwrap();
        assert!(tracker
            .flush_idle(&config, &settings, 9, 2)
            .unwrap()
            .is_none());
        assert!(tracker.buffer.as_ref().unwrap().deferred.is_empty());
        tracker
            .observe(&config, &settings, observation("Context: Human", 14, 10))
            .unwrap();
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope(), 11)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "Human");
    }
    #[test]
    fn passive_changes_never_become_owned_writing() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("", 0, 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("Human", 5, 2))
            .unwrap();
        let baseline = "Human generated text";
        let prior = tracker
            .observe_passive(&config, &settings, observation(baseline, baseline.len(), 3))
            .unwrap();
        assert_eq!(prior.len(), 1);
        assert_eq!(prior[0].text, "Human");
        assert!(tracker
            .flush_idle(&config, &settings, 10, 2)
            .unwrap()
            .is_none());
        assert!(tracker.text_matches(baseline));
        let edited = "Human generated text plus my words";
        tracker
            .observe(&config, &settings, observation(edited, edited.len(), 11))
            .unwrap();
        let next = tracker.finish(&config, &settings, 15).unwrap().unwrap();
        assert_eq!(next.text, " plus my words");
        assert_ne!(next.id, prior[0].id);
    }
    #[test]
    fn independent_replacements_do_not_import_unchanged_private_middle() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        let initial = "A PRIVATE MIDDLE B";
        let changed = "X PRIVATE MIDDLE Y";
        tracker
            .observe(&config, &settings, observation(initial, initial.len(), 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation(changed, changed.len(), 2))
            .unwrap();
        assert!(tracker.finish(&config, &settings, 10).unwrap().is_none());
    }
    #[test]
    fn replacement_of_a_known_selection_records_only_the_new_text() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        let mut selected = observation("Hello OLD world", 6, 1);
        selected.selection_end = 9;
        tracker.observe(&config, &settings, selected).unwrap();
        tracker
            .observe(&config, &settings, observation("Hello NEW world", 9, 2))
            .unwrap();
        assert_eq!(
            tracker.finish(&config, &settings, 6).unwrap().unwrap().text,
            "NEW"
        );
    }

    #[test]
    fn baseline_and_unchanged_surroundings_never_become_history() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(
                &config,
                &settings,
                observation("PRIVATE BEFORE  PRIVATE AFTER", 15, 1),
            )
            .unwrap();
        assert!(tracker
            .flush_idle(&config, &settings, 5, 3)
            .unwrap()
            .is_none());
        let changed = "PRIVATE BEFORE Newly typed 🙂 PRIVATE AFTER";
        let caret = "PRIVATE BEFORE Newly typed 🙂".len();
        tracker
            .observe(&config, &settings, observation(changed, caret, 6))
            .unwrap();
        let record = tracker
            .flush_idle(&config, &settings, 10, 3)
            .unwrap()
            .unwrap();
        assert_eq!(record.text, "Newly typed 🙂");
        assert!(tracker
            .flush_idle(&config, &settings, 20, 3)
            .unwrap()
            .is_none());
        let edited = "PRIVATE BEFORE Newly written 🙂 PRIVATE AFTER";
        let caret = "PRIVATE BEFORE Newly written".len();
        tracker
            .observe(&config, &settings, observation(edited, caret, 21))
            .unwrap();
        let update = tracker
            .flush_idle(&config, &settings, 25, 3)
            .unwrap()
            .unwrap();
        assert_eq!(update.id, record.id);
        assert_eq!(update.text, "Newly written 🙂");
    }
    #[test]
    fn off_or_newly_excluded_discards_uncommitted_writing() {
        let config = Config::default();
        let mut settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("", 0, 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("Some new words", 14, 2))
            .unwrap();
        settings.recording = false;
        assert!(tracker
            .flush_idle(&config, &settings, 10, 3)
            .unwrap()
            .is_none());
        settings.recording = true;
        tracker
            .observe(&config, &settings, observation("", 0, 11))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("More new words", 14, 12))
            .unwrap();
        settings.excluded_apps.push("com.apple.mail".into());
        assert!(tracker.finish(&config, &settings, 20).unwrap().is_none());
    }
    #[test]
    fn clearing_a_composer_finishes_only_the_new_span() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("quoted material: ", 17, 1))
            .unwrap();
        tracker
            .observe(
                &config,
                &settings,
                observation("quoted material: my reply", 25, 2),
            )
            .unwrap();
        let records = tracker
            .observe(&config, &settings, observation("", 0, 3))
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "my reply");
    }
    #[test]
    fn changing_fields_flushes_permitted_source_without_importing_destination() {
        let config = Config::default();
        let mut settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("", 0, 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("A written sample", 16, 2))
            .unwrap();
        settings.excluded_apps.push("private.app".into());
        let records = tracker
            .observe(
                &config,
                &settings,
                Observation {
                    field: 2,
                    scope: Scope {
                        app: "private.app".into(),
                        website: None,
                    },
                    text: "EXISTING PRIVATE CONTENT",
                    caret: 24,
                    selection_end: 24,
                    now: 3,
                },
            )
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "A written sample");
        assert!(tracker
            .flush_idle(&config, &settings, 20, 3)
            .unwrap()
            .is_none());
    }
    #[test]
    fn long_unicode_compositions_split_at_scalar_boundaries() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("", 0, 1))
            .unwrap();
        let text = "🙂".repeat(MAX_TEXT + 20);
        let mut records = tracker
            .observe(&config, &settings, observation(&text, text.len(), 2))
            .unwrap();
        records.extend(tracker.finish(&config, &settings, 3).unwrap());
        assert_eq!(records.len(), 2);
        assert_eq!(
            records.iter().map(|r| r.text.clone()).collect::<String>(),
            text
        );
        assert_ne!(records[0].id, records[1].id);
    }
    #[test]
    fn caret_mismatch_and_multiple_region_changes_are_not_attributed() {
        let config = Config::default();
        let settings = settings();
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, observation("aaaa", 2, 1))
            .unwrap();
        tracker
            .observe(&config, &settings, observation("aaaaa", 3, 2))
            .unwrap();
        assert!(tracker.finish(&config, &settings, 3).unwrap().is_none());
    }
}
