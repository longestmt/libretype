// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Pure continuation handling. All offsets are UTF-8 boundaries; no text is persisted.
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug)]
pub struct CompletionRequest {
    pub id: u64,
    pub prefix: String,
    pub suffix: String,
    pub app_id: String,
    pub website: Option<String>,
    pub profile: Option<crate::history::Profile>,
    pub background: crate::background::Background,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCompletion {
    pub prompt: String,
    pub fragment: String,
    pub format: CompletionFormat,
    pub passage: Option<PassageContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassageContext {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionFormat {
    Text,
    Infill,
    ReconstructedWord,
    CompletedPassage,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '\'' | '’')
        || matches!(c as u32, 0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f)
}

// Han/kana characters and Hangul syllables are already complete written units;
// backtracking an entire unspaced clause asks the model to echo that clause.
fn syllabic_or_ideographic(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30ff | 0x31f0..=0x31ff | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af
        | 0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af)
}

pub fn prepare(prefix: &str, suffix: &str, context_chars: usize) -> Option<PreparedCompletion> {
    // Raw continuation has no suffix awareness. Do not offer edits in front of existing text.
    if !suffix.trim().is_empty() || prefix.trim().is_empty() || context_chars == 0 {
        return None;
    }
    if prefix
        .chars()
        .next_back()
        .is_some_and(syllabic_or_ideographic)
    {
        let start = prefix
            .char_indices()
            .nth(prefix.chars().count().saturating_sub(context_chars))
            .map_or(0, |(i, _)| i);
        return Some(PreparedCompletion {
            prompt: prefix[start..].into(),
            fragment: String::new(),
            format: CompletionFormat::Text,
            passage: None,
        });
    }
    let split = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| !word_char(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let before = prefix[..split].trim_end_matches([' ', '\t']);
    // Include removed whitespace in the exact prefix that the model must reconstruct.
    // Trailing spaces in a tokenizer prompt otherwise encourage malformed word continuations.
    let fragment = &prefix[before.len()..];
    // With no preceding context there is no useful full-word prediction to verify.
    if before.trim().is_empty() {
        return None;
    }
    let count = before.chars().count();
    let start = before
        .char_indices()
        .nth(count.saturating_sub(context_chars))
        .map_or(0, |(i, _)| i);
    Some(PreparedCompletion {
        prompt: before[start..].to_owned(),
        fragment: fragment.to_owned(),
        format: CompletionFormat::Text,
        passage: None,
    })
}

/// Fallback for short text or a completed word that the raw model failed to
/// reconstruct. Few-shot examples ask for a suffix rather than a replacement.
/// Existing editor whitespace is consumed logically, never rewritten.
pub fn prepare_fallback(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
) -> Option<PreparedCompletion> {
    if !suffix.trim().is_empty() || prefix.trim().chars().count() < 3 || context_chars == 0 {
        return None;
    }
    let normalized = prefix.replace(['\u{a0}', '\u{202f}'], " ");
    let before = normalized.trim_end_matches([' ', '\t']);
    let fragment = if before.len() < normalized.len() {
        " "
    } else {
        ""
    };
    let count = before.chars().count();
    let start = before
        .char_indices()
        .nth(count.saturating_sub(context_chars))
        .map_or(0, |(i, _)| i);
    Some(PreparedCompletion {
        prompt: format!("Continue the text with a few words.\n\nText: Hello\nContinuation: there, how are you?\n\nText: Thank you for your\nContinuation: help with this.\n\nText: I hope your mee\nContinuation:ting goes well.\n\nText: {}\nContinuation:", &before[start..]),
        fragment: fragment.into(),
        format: CompletionFormat::Text,
        passage: None,
    })
}

/// Place preferences next to the current text, after format examples. Small
/// completion models otherwise tend to follow the examples and ignore style.
pub fn prepare_instructed(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if prefix
        .chars()
        .next_back()
        .is_some_and(syllabic_or_ideographic)
    {
        return prepare_insertion(prefix, suffix, context_chars, instructions);
    }
    if prefix.ends_with(word_char) {
        return prepare_sentence_end(prefix, suffix, context_chars, instructions)
            .or_else(|| prepare_reconstructed(prefix, suffix, context_chars, instructions));
    }
    let mut prepared = prepare_fallback(prefix, suffix, context_chars)?;
    if !instructions.trim().is_empty() {
        let normalized = prefix.replace(['\u{a0}', '\u{202f}'], " ");
        let before = normalized.trim_end_matches([' ', '\t']);
        let count = before.chars().count();
        let start = before
            .char_indices()
            .nth(count.saturating_sub(context_chars))
            .map_or(0, |(i, _)| i);
        prepared.prompt = format!("Continue the text with a few words. Output only the continuation.\n\nText: Good morning\nContinuation: everyone, how are you?\n\nText: I hope the mee\nContinuation:ting goes well.\n\nWriting preferences: {}\nText: {}\nContinuation:", instructions.trim(), &before[start..]);
    }
    Some(prepared)
}

/// Ask for a whole word instead of asking the tokenizer to spell a missing
/// suffix. Strip only the verified, already-typed fragment before insertion.
pub fn prepare_reconstructed(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if !suffix.trim().is_empty()
        || prefix.trim().chars().count() < 3
        || !prefix.ends_with(word_char)
        || prefix
            .chars()
            .next_back()
            .is_some_and(syllabic_or_ideographic)
    {
        return None;
    }
    // Normalize the model's copy, never the editor's original text. This avoids
    // treating canonically equivalent accented spellings as different words.
    let normalized: String = prefix.nfc().collect();
    let split = normalized
        .char_indices()
        .rev()
        .find(|(_, c)| !word_char(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let fragment = &normalized[split..];
    let fragment_chars = fragment.chars().count();
    if fragment_chars == 0 || fragment_chars > context_chars {
        return None;
    }
    let context = &normalized[..split];
    let count = context.chars().count();
    let start = context
        .char_indices()
        .nth(count.saturating_sub(context_chars - fragment_chars))
        .map_or(context.len(), |(i, _)| i);
    let mut prompt = String::from("Complete the writing in the same language. Context is everything before the last word. Started word contains letters already typed. Finish that word using the grammatical form required by Context (including part of speech, tense, number and gender), then add a short continuation. The word may already be complete. Never repeat Context or change Started word. Return JSON with continuation: the full last word followed by a few more words.\n\n");
    for (context, fragment, completion) in [
        ("I enjoyed the ", "conver", "conversation with you."),
        ("Have a ", "great", "great weekend."),
        ("", "Welcome", "Welcome to the team."),
    ] {
        prompt.push_str(&format!(
            "Context: {}\nStarted word: {}\n{{\"continuation\": {}}}\n\n",
            serde_json::json!(context),
            serde_json::json!(fragment),
            serde_json::json!(completion)
        ));
    }
    if !instructions.trim().is_empty() {
        prompt.push_str(&format!("Writing preferences: {}\n", instructions.trim()));
    }
    prompt.push_str(&format!(
        "Context: {}\nStarted word: {}\n",
        serde_json::json!(&context[start..]),
        serde_json::json!(fragment)
    ));
    Some(PreparedCompletion {
        prompt,
        fragment: fragment.into(),
        format: CompletionFormat::ReconstructedWord,
        passage: None,
    })
}

/// Echo a bounded sentence fragment so grammatical context stays next to the
/// unfinished word. Only the verified new suffix is ever inserted into an editor.
pub fn prepare_sentence_end(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if !suffix.trim().is_empty()
        || prefix.trim().chars().count() < 3
        || !prefix.ends_with(word_char)
        || prefix
            .chars()
            .next_back()
            .is_some_and(syllabic_or_ideographic)
        || context_chars < 3
        || prefix.chars().rev().take_while(|c| word_char(*c)).count() > context_chars.min(128)
    {
        return None;
    }
    prepare_passage(prefix, "", context_chars, instructions)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconstructedResponse {
    continuation: String,
}

pub fn reconstructed_response_complete(raw: &str) -> bool {
    serde_json::from_str::<ReconstructedResponse>(raw).is_ok()
}

pub fn finish_reconstructed(prepared: &PreparedCompletion, raw: &str, max_words: usize) -> String {
    let Ok(response) = serde_json::from_str::<ReconstructedResponse>(raw) else {
        return String::new();
    };
    if response.continuation.chars().any(char::is_control) {
        return String::new();
    }
    let full: String = response.continuation.nfc().collect();
    // A model may lowercase a sentence-initial word (Testing -> testing).
    // Match its letters without replacing the user's capitalization. Compare
    // scalar by scalar so case expansion cannot shift the insertion boundary.
    let mut generated = full.char_indices();
    let mut end = 0;
    for typed in prepared.fragment.chars() {
        let Some((i, c)) = generated.next() else {
            return String::new();
        };
        if !typed.to_lowercase().eq(c.to_lowercase()) {
            return String::new();
        }
        end = i + c.len_utf8();
    }
    finish(
        prepared,
        &format!("{}{}", prepared.fragment, &full[end..]),
        max_words,
    )
}

/// A structured insertion, conditioned on both sides of the caret. Quoted text
/// makes existing spaces and partial words explicit without rewriting either side.
pub fn prepare_midline(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if prefix.trim().chars().count() < 3 || suffix.trim().is_empty() || context_chars < 3 {
        return None;
    }
    prepare_passage(prefix, suffix, context_chars, instructions)
}

/// Contextual requests use a structured instruction rather than raw continuation,
/// so the model can distinguish reference material from the text being completed.
pub fn prepare_structured(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if !suffix.trim().is_empty() {
        prepare_midline(prefix, suffix, context_chars, instructions)
    } else if prefix.ends_with(word_char)
        && !prefix
            .chars()
            .next_back()
            .is_some_and(syllabic_or_ideographic)
    {
        prepare_sentence_end(prefix, suffix, context_chars, instructions)
            .or_else(|| prepare_reconstructed(prefix, suffix, context_chars, instructions))
    } else {
        prepare_insertion(prefix, suffix, context_chars, instructions)
    }
}

pub fn prepare_contextual(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
    reference: &str,
) -> Option<PreparedCompletion> {
    let mut prepared = prepare_structured(prefix, suffix, context_chars, instructions)?;
    if prepared.format == CompletionFormat::CompletedPassage {
        prepared.prompt = format!(
            "Use relevant reference facts while preserving the existing text.\n{reference}\n{}",
            prepared.prompt
        );
        return Some(prepared);
    }
    let before: String = prefix
        .chars()
        .rev()
        .take(context_chars * 2 / 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = suffix.chars().take(context_chars / 3).collect();
    let task = if prepared.format == CompletionFormat::ReconstructedWord {
        format!("Return JSON with continuation: finish the last word {} and optionally add a few words. Start continuation with that ENTIRE last word, including its already typed letters. Do not repeat earlier words.",serde_json::json!(prepared.fragment))
    } else {
        r#"Fill [GAP] in the sentence using relevant reference facts. Return JSON with insert containing ONLY the missing characters, not the complete sentence. Preserve surrounding text and include necessary spaces. Example: sentence "We meet at [GAP] tomorrow." with reference "Venue: River Park" gives {"insert":"River Park"}."#.into()
    };
    let text = if prepared.format == CompletionFormat::ReconstructedWord {
        let normalized: String = before.nfc().collect();
        let context = normalized
            .strip_suffix(&prepared.fragment)
            .unwrap_or(&normalized);
        format!(
            "Context: {}\nStarted word: {}",
            serde_json::json!(context),
            serde_json::json!(prepared.fragment)
        )
    } else {
        format!(
            "Sentence: {}",
            serde_json::json!(format!("{before}[GAP]{after}"))
        )
    };
    prepared.prompt = format!("Use relevant facts in the supplied reference to complete the writing naturally. Preserve its language.\n{task}\n{reference}\nWriting preferences: {}\n{text}\nOutput only the requested JSON.",serde_json::json!(instructions));
    Some(prepared)
}

/// Reconstruct a short passage, then extract only the verified gap. Bounding
/// the echoed text keeps generation latency/token use independent of document size.
fn prepare_passage(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    let sentence_end = suffix.is_empty();
    let before_budget = if sentence_end {
        context_chars
    } else {
        context_chars.saturating_mul(2) / 3
    };
    let before_source: String = prefix
        .chars()
        .rev()
        .take(before_budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .nfc()
        .collect();
    let after_source: String = suffix
        .chars()
        .take(context_chars.saturating_sub(before_budget))
        .collect::<String>()
        .nfc()
        .collect();
    let mut before_start = before_source
        .char_indices()
        .nth(before_source.chars().count().saturating_sub(128))
        .map_or(0, |(i, _)| i);
    let mut after_end = after_source
        .char_indices()
        .nth(96)
        .map_or(after_source.len(), |(i, _)| i);
    // Echo complete nearby sentence fragments instead of arbitrary long slices.
    // Older/later material remains reference context below. This avoids asking a
    // model to reproduce clipped words or several unrelated sentences exactly.
    let window_start = before_start;
    for (i, c) in before_source[window_start..].char_indices() {
        let boundary = window_start + i + c.len_utf8();
        let rest = &before_source[boundary..];
        if passage_boundary(c, rest) && !rest.trim().is_empty() {
            before_start = boundary + rest.len() - rest.trim_start().len();
        }
    }
    // If the context window starts inside a Latin word, omit only that remote
    // fragment when a later word boundary is available.
    if before_start > 0
        && before_source[..before_start].ends_with(word_char)
        && before_source[before_start..].starts_with(word_char)
    {
        if let Some((i, c)) = before_source[before_start..]
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
        {
            if before_start + i + c.len_utf8() < before_source.len() {
                before_start += i + c.len_utf8();
            }
        }
    }
    for (i, c) in after_source[..after_end].char_indices() {
        let boundary = i + c.len_utf8();
        if passage_boundary(c, &after_source[boundary..]) {
            after_end = boundary;
            break;
        }
    }
    let before = before_source[before_start..].to_owned();
    let after = after_source[..after_end].to_owned();
    let mut prompt = if sentence_end {
        String::from("Complete this unfinished sentence naturally in the same language. Keep every existing character unchanged, finish the last word if needed, and add a few words. Return JSON with completed containing the full sentence, including the existing text. Match the grammatical form required by the sentence.\nExample: {\"text\":\"She has already complet\",\"completed\":\"She has already completed the assignment.\"}\n")
    } else {
        String::from("Fill the gap between Before and After with a few words in the same language. Return JSON with completed containing the ENTIRE completed passage: Before + your insertion + After. Copy Before and After exactly, including their spaces and punctuation. Only add characters at the gap. If no insertion is needed, return Before + After unchanged. Never rewrite existing words.\n\n")
    };
    let examples = [
        ("She will ret", " tomorrow.", "She will return tomorrow."),
        (
            "I will come",
            " after lunch.",
            "I will come back after lunch.",
        ),
        ("We should ", "tomorrow.", "We should meet tomorrow."),
        ("Turn on the mic", "rophone.", "Turn on the microphone."),
    ];
    for (before, after, completed) in examples.into_iter().filter(|_| !sentence_end) {
        prompt.push_str(&format!(
            "Before: {}\nAfter: {}\n{}\n\n",
            serde_json::json!(before),
            serde_json::json!(after),
            serde_json::json!({"completed":completed})
        ));
    }
    if before_start > 0 || after_end < after_source.len() {
        prompt.push_str(&format!(
            "Additional context outside the passage (do not echo):\nEarlier: {}\nLater: {}\n",
            serde_json::json!(&before_source[..before_start]),
            serde_json::json!(&after_source[after_end..])
        ));
    }
    if !instructions.trim().is_empty() {
        prompt.push_str(&format!("Writing preferences: {}\n", instructions.trim()));
    }
    if sentence_end {
        prompt.push_str(&format!("Text: {}\n", serde_json::json!(before)));
    } else {
        prompt.push_str(&format!(
            "Before: {}\nAfter: {}\n",
            serde_json::json!(before),
            serde_json::json!(after)
        ));
    }
    Some(PreparedCompletion {
        prompt,
        fragment: String::new(),
        format: CompletionFormat::CompletedPassage,
        passage: Some(PassageContext { before, after }),
    })
}

fn passage_boundary(c: char, after: &str) -> bool {
    matches!(c, '\n' | '\r' | '。' | '！' | '？')
        || (matches!(c, '.' | '!' | '?')
            && (after.is_empty() || after.starts_with(char::is_whitespace)))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PassageResponse {
    completed: String,
}

pub fn passage_response_complete(raw: &str) -> bool {
    serde_json::from_str::<PassageResponse>(raw).is_ok()
}

pub fn finish_passage(prepared: &PreparedCompletion, raw: &str, max_words: usize) -> String {
    let Some(context) = prepared.passage.as_ref() else {
        return String::new();
    };
    let Ok(response) = serde_json::from_str::<PassageResponse>(raw) else {
        return String::new();
    };
    // Normalize working copies only; never replace any editor text. Remove the
    // prefix first so overlapping copies cannot masquerade as a valid insertion.
    let full: String = response.completed.nfc().collect();
    let Some(insert) = full
        .strip_prefix(&context.before)
        .and_then(|rest| rest.strip_suffix(&context.after))
    else {
        return String::new();
    };
    if insert.chars().any(char::is_control)
        || insert.trim().is_empty()
        || invalid_word_gap(&context.before, &context.after, insert)
    {
        return String::new();
    }
    // Truncating an insertion can destroy its join with the fixed right side.
    // Enforce limits by rejecting an overlong bridge, not by changing its words.
    if context.after.is_empty() {
        return finish(prepared, insert, max_words);
    }
    if finish(prepared, insert, max_words) != insert.trim_end() {
        return String::new();
    }
    insert.to_owned()
}

fn invalid_word_gap(prefix: &str, suffix: &str, insert: &str) -> bool {
    let inside_word = prefix
        .chars()
        .next_back()
        .is_some_and(|c| word_char(c) && !syllabic_or_ideographic(c))
        && suffix
            .chars()
            .next()
            .is_some_and(|c| word_char(c) && !syllabic_or_ideographic(c));
    inside_word && insert.chars().any(|c| !word_char(c))
}

fn prepare_insertion(
    prefix: &str,
    suffix: &str,
    context_chars: usize,
    instructions: &str,
) -> Option<PreparedCompletion> {
    if prefix.trim().chars().count() < 3 || context_chars < 3 {
        return None;
    }
    let before_limit = context_chars * 2 / 3;
    let before: String = prefix
        .chars()
        .rev()
        .take(before_limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = suffix.chars().take(context_chars - before_limit).collect();
    let mut prompt = String::from("Fill the gap between Before and After. Return JSON with only insert: the exact characters to insert. Preserve both surrounding texts. Include needed spaces in insert. Do not repeat Before or After.\n\n");
    for (before, after, insert) in [
        ("Thank you for your ", ".", "help"),
        ("We should ", "tomorrow.", "meet "),
        ("I hope your mee", " goes well.", "ting"),
        ("Hello ", "world!", "beautiful "),
        ("I enjoy read", " books.", "ing"),
    ] {
        prompt.push_str(&format!(
            "Before: {}\nAfter: {}\n{}\n\n",
            serde_json::json!(before),
            serde_json::json!(after),
            serde_json::json!({"insert":insert})
        ));
    }
    if !instructions.trim().is_empty() {
        prompt.push_str(&format!("Writing preferences: {}\n", instructions.trim()));
    }
    prompt.push_str(&format!(
        "Before: {}\nAfter: {}\n",
        serde_json::json!(before),
        serde_json::json!(after)
    ));
    Some(PreparedCompletion {
        prompt,
        fragment: String::new(),
        format: CompletionFormat::Infill,
        passage: None,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InfillResponse {
    insert: String,
}

pub fn midline_response_complete(raw: &str) -> bool {
    serde_json::from_str::<InfillResponse>(raw).is_ok()
}

pub fn finish_midline(
    prepared: &PreparedCompletion,
    raw: &str,
    prefix: &str,
    suffix: &str,
    max_words: usize,
) -> String {
    let Ok(response) = serde_json::from_str::<InfillResponse>(raw) else {
        return String::new();
    };
    if response.insert.chars().any(char::is_control) {
        return String::new();
    }
    let original = response.insert.as_str();
    // In a space-delimited word, a model must not turn the cursor gap into a
    // phrase boundary (e.g. con|versation -> contact versation). CJK clauses
    // use different word boundaries and are not subject to this restriction.
    if invalid_word_gap(prefix, suffix, original) {
        return String::new();
    }
    let mut output = finish(prepared, original, max_words);
    if output.is_empty() {
        return output;
    }
    let had_separator = original.ends_with(char::is_whitespace)
        || original
            .strip_prefix(&output)
            .is_some_and(|tail| tail.starts_with(char::is_whitespace));
    if prefix.ends_with(char::is_whitespace) {
        output = output.trim_start().to_owned();
    }
    // A response that copies the whole nearby prefix is not an insertion.
    let before = prefix.trim();
    if before.chars().count() >= 8 && output.trim_start().starts_with(before) {
        return String::new();
    }
    // Remove an echoed suffix at whole-word boundaries, never a coincidentally
    // matching letter in a partially completed word. Original suffix stays intact.
    let after = suffix.trim_start();
    for (start, _) in output.char_indices() {
        let candidate = &output[start..];
        let boundary = start == 0 || output[..start].ends_with(|c: char| !word_char(c));
        if boundary
            && candidate.chars().count() >= 3
            && after.starts_with(candidate)
            && after[candidate.len()..]
                .chars()
                .next()
                .is_none_or(|c| !word_char(c))
        {
            output.truncate(start);
            break;
        }
    }
    if let Some(punctuation) = after
        .chars()
        .next()
        .filter(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '。' | '！' | '？'))
    {
        if output.ends_with(punctuation) {
            output.pop();
        }
    }
    if suffix.starts_with(char::is_whitespace)
        || after.starts_with(['.', ',', '!', '?', ';', ':', '。', '！', '？'])
    {
        output = output.trim_end().to_owned();
    } else if had_separator && !output.ends_with(char::is_whitespace) {
        output.push(' ');
    }
    if output.trim().is_empty() {
        String::new()
    } else {
        output
    }
}

/// True once enough *complete* output exists to close the model stream early.
pub fn has_complete_limit(prepared: &PreparedCompletion, raw: &str, max_words: usize) -> bool {
    let Some(continuation) = raw.strip_prefix(&prepared.fragment) else {
        return false;
    };
    let mut words = 0;
    let mut in_word = false;
    let mut previous = '\0';
    for c in continuation.chars() {
        if matches!(c, '。' | '！' | '？') {
            return true;
        }
        if c.is_whitespace() {
            if words >= max_words || matches!(previous, '.' | '!' | '?' | '。' | '！' | '？') {
                return true;
            }
            in_word = false;
        } else if !in_word {
            words += 1;
            in_word = true;
        }
        previous = c;
    }
    false
}

/// A token budget may cut a word in half. Only retain an established word boundary.
pub fn without_partial_tail(raw: &str) -> &str {
    if raw.ends_with(char::is_whitespace) || raw.ends_with(['.', '!', '?', '。', '！', '？']) {
        raw
    } else {
        raw.char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or("", |(i, c)| &raw[..i + c.len_utf8()])
    }
}

pub fn finish(prepared: &PreparedCompletion, raw: &str, max_words: usize) -> String {
    if max_words == 0
        || raw.is_empty()
        || raw
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r')
    {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    if [
        "<|",
        "|>",
        "<think",
        "</think",
        "[inst]",
        "[/inst]",
        "assistant:",
        "user:",
        "system:",
        "```",
        "/no_think",
    ]
    .iter()
    .any(|tag| lower.contains(tag))
    {
        return String::new();
    }
    // Never trim the beginning: it contains the precise whitespace to insert.
    let Some(continuation) = raw.strip_prefix(&prepared.fragment) else {
        return String::new();
    };
    let line = continuation.split(['\n', '\r']).next().unwrap_or("");
    let mut end = line.len();
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            words += 1;
            if words > max_words {
                end = i;
                break;
            }
            in_word = true;
        }
        if matches!(c, '.' | '!' | '?' | '。' | '！' | '？') {
            let next = &line[i + c.len_utf8()..];
            if matches!(c, '。' | '！' | '？')
                || next.is_empty()
                || next.starts_with(char::is_whitespace)
            {
                end = i + c.len_utf8();
                break;
            }
        }
    }
    let result = line[..end].trim_end();
    if result.trim().is_empty() {
        String::new()
    } else {
        result.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passage_extracts_only_the_gap_including_both_joining_spaces() {
        for (before, after, completed, expected) in [
            (
                "Please send the report",
                " before Friday.",
                "Please send the report to me before Friday.",
                " to me",
            ),
            (
                "Thank you for your underst",
                " with this issue.",
                "Thank you for your understanding with this issue.",
                "anding",
            ),
            (
                "We should ",
                "tomorrow.",
                "We should meet tomorrow.",
                "meet ",
            ),
            (
                "I look forward to ",
                " you next week.",
                "I look forward to seeing you next week.",
                "seeing",
            ),
            (
                "The trans",
                "ation worked.",
                "The transformation worked.",
                "form",
            ),
            (
                "Merci pour votre",
                " pendant cette période.",
                "Merci pour votre aide pendant cette période.",
                " aide",
            ),
            (
                "Le café",
                " est ouvert.",
                "Le café voisin est ouvert.",
                " voisin",
            ),
            (
                "確認しました",
                "連絡します。",
                "確認しました、後ほど連絡します。",
                "、後ほど",
            ),
            (
                "Note:\nPlease send the invo",
                " before noon.\nThanks.",
                "Please send the invoice before noon.",
                "ice",
            ),
        ] {
            let p = prepare_midline(before, after, 1200, "").unwrap();
            assert_eq!(p.format, CompletionFormat::CompletedPassage);
            let raw = serde_json::json!({"completed":completed}).to_string();
            assert!(passage_response_complete(&raw));
            assert_eq!(finish_passage(&p, &raw, 8), expected);
            // Original decomposed spelling, newline, and suffix bytes remain editor-owned.
            let joined = format!("{before}{expected}{after}");
            assert!(joined.starts_with(before) && joined.ends_with(after));
        }
    }

    #[test]
    fn passage_rejects_rewrites_overlapping_echoes_and_broken_bridges() {
        for (before, after, completed, limit) in [
            (
                "We need support",
                " with the move.",
                "We need support for the move.",
                8,
            ),
            ("Thanks", ".", "thanks for your help.", 8),
            ("abc", "bcd", "abcd", 8), // Prefix/suffix overlap, not an insertion.
            (
                "I enjoyed the con",
                "versation.",
                "I enjoyed the contact versation.",
                8,
            ),
            (
                "We should ",
                "tomorrow.",
                "We should meet with everyone tomorrow.",
                2,
            ),
            (
                "We should ",
                "tomorrow.",
                "We should meet\nagain tomorrow.",
                8,
            ),
            (
                "We should ",
                "tomorrow.",
                "We should <think>meet tomorrow.",
                8,
            ),
            (
                "We should ",
                "tomorrow.",
                "We should meet. See you tomorrow.",
                8,
            ),
            ("We should ", "tomorrow.", "We should meet tomorrow.", 0),
            ("The docu", "ment.", "The document.", 8), // Valid no-op.
        ] {
            let p = prepare_midline(before, after, 1200, "").unwrap();
            assert_eq!(
                finish_passage(
                    &p,
                    &serde_json::json!({"completed":completed}).to_string(),
                    limit
                ),
                "",
                "{completed}"
            );
        }
        let p = prepare_midline("We should ", "tomorrow.", 1200, "").unwrap();
        for raw in [
            r#"{"completed":"We should meet tomorrow.","replace":true}"#,
            r#"{"completed":42}"#,
            r#"{"insert":"meet "}"#,
            r#"{"completed":"We should meet"#,
        ] {
            assert!(!passage_response_complete(raw));
            assert_eq!(finish_passage(&p, raw, 8), "");
        }
        // Joining space survives when the insertion exactly meets the word limit.
        assert_eq!(
            finish_passage(&p, r#"{"completed":"We should meet tomorrow."}"#, 1),
            "meet "
        );
    }

    #[test]
    fn passage_echoes_nearby_sentences_and_preserves_remote_context_as_reference() {
        let prefix = format!(
            "{}Please send the invo",
            "Older notes about version 3.14. ".repeat(20)
        );
        let suffix = format!(
            " before noon. {}",
            "Later details about delivery. ".repeat(20)
        );
        let p = prepare_midline(&prefix, &suffix, 1200, "").unwrap();
        assert_eq!(p.passage.as_ref().unwrap().before, "Please send the invo");
        assert_eq!(p.passage.as_ref().unwrap().after, " before noon.");
        assert!(p.prompt.contains("Older notes about version 3.14."));
        assert!(p.prompt.contains("Later details about delivery."));
        assert_eq!(
            finish_passage(
                &p,
                r#"{"completed":"Please send the invoice before noon."}"#,
                8
            ),
            "ice"
        );
        let p = prepare_midline(
            "Version 3.14 needs a re",
            " before version 3.15 ships. More context.",
            1200,
            "",
        )
        .unwrap();
        assert_eq!(
            p.passage.as_ref().unwrap().before,
            "Version 3.14 needs a re"
        );
        assert_eq!(
            p.passage.as_ref().unwrap().after,
            " before version 3.15 ships."
        );
        let p = prepare_midline(
            "確認済みです。資料を確認した",
            "ご連絡します。よろしくお願いします。",
            1200,
            "",
        )
        .unwrap();
        assert_eq!(p.passage.as_ref().unwrap().before, "資料を確認した");
        assert_eq!(p.passage.as_ref().unwrap().after, "ご連絡します。");
    }

    #[test]
    fn passage_windows_bound_echo_cost_and_keep_style_and_reference_routing() {
        let before = format!("{} Please send the invo", "older context. ".repeat(100));
        let after = format!(" before noon. {}", "later context. ".repeat(100));
        for limit in [3, 30, 1200, usize::MAX] {
            let p = prepare_midline(&before, &after, limit, "Be concise.").unwrap();
            let context = p.passage.as_ref().unwrap();
            assert!(context.before.chars().count() <= 128);
            assert!(context.after.chars().count() <= 96);
            assert!(context.before.chars().count() + context.after.chars().count() <= limit);
            assert!(before.ends_with(&context.before) && after.starts_with(&context.after));
            let raw =
                serde_json::json!({"completed":format!("{}ice{}",context.before,context.after)})
                    .to_string();
            assert_eq!(finish_passage(&p, &raw, 8), "ice");
            assert!(p.prompt.contains("Writing preferences: Be concise."));
        }
        assert_eq!(
            prepare_structured("Send the invo", " tomorrow.", 1200, "Be concise.")
                .unwrap()
                .format,
            CompletionFormat::CompletedPassage
        );
        let contextual = prepare_contextual(
            "Meet at ",
            " tomorrow.",
            1200,
            "Be concise.",
            "Venue: River Park",
        )
        .unwrap();
        assert_eq!(contextual.format, CompletionFormat::CompletedPassage);
        assert!(contextual.prompt.contains("Venue: River Park"));
        assert_eq!(
            finish_passage(
                &contextual,
                r#"{"completed":"Meet at River Park tomorrow."}"#,
                8
            ),
            "River Park"
        );
    }

    #[test]
    fn contextual_edits_preserve_fragment_and_suffix() {
        let prepared = prepare_contextual(
            "Meet at CAFE\u{301}",
            "",
            1200,
            "",
            "Synthetic reference: Café Azure",
        )
        .unwrap();
        assert_eq!(prepared.format, CompletionFormat::CompletedPassage);
        assert_eq!(
            finish_passage(&prepared, r#"{"completed":"Meet at CAFÉ Azure."}"#, 8),
            " Azure."
        );
        assert!(finish_passage(&prepared, r#"{"completed":"Meet at River Park."}"#, 8).is_empty());
        let prepared = prepare_contextual(
            "Meet at ",
            " tomorrow.",
            1200,
            "",
            "Synthetic reference: Café Azure",
        )
        .unwrap();
        assert_eq!(
            finish_passage(
                &prepared,
                r#"{"completed":"Meet at Café Azure tomorrow."}"#,
                8
            ),
            "Café Azure"
        );
        assert!(prepare_contextual(" ", "", 1200, "", "reference").is_none());
    }

    #[test]
    fn sentence_end_extracts_only_new_text_and_rejects_rewrites() {
        for (prefix, full, expected) in [
            (
                "Nous sommes très reconnaiss",
                "Nous sommes très reconnaissants de votre aide.",
                "ants de votre aide.",
            ),
            (
                "Estamos muito agradec",
                "Estamos muito agradecidos por sua ajuda.",
                "idos por sua ajuda.",
            ),
            ("The cafe\u{301}", "The café is open.", " is open."),
            ("Hello", "Hello, how are you?", ", how are you?"),
        ] {
            let p = prepare_instructed(prefix, "", 1200, "").unwrap();
            let raw = serde_json::json!({"completed": full}).to_string();
            assert_eq!(finish_passage(&p, &raw, 8), expected);
            for invalid in [
                serde_json::json!({"completed": format!("Changed {full}")}),
                serde_json::json!({"completed": format!("{prefix}\nnew line")}),
                serde_json::json!({"completed": prefix}),
            ] {
                assert!(finish_passage(&p, &invalid.to_string(), 8).is_empty());
            }
        }
        let p = prepare_sentence_end("We should sched", "", 1200, "").unwrap();
        let raw = r#"{"completed":"We should schedule a meeting with the team tomorrow morning."}"#;
        assert_eq!(finish_passage(&p, raw, 3), "ule a meeting");
        assert_eq!(finish_passage(&p, raw, 0), "");
    }

    #[test]
    fn sentence_end_bounds_echo_without_losing_reference_or_preferences() {
        let prefix = format!(
            "{} Earlier facts matter. We should sched",
            "Long context. ".repeat(150)
        );
        let p = prepare_sentence_end(&prefix, "", 1200, "Use a formal tone.").unwrap();
        let passage = p.passage.as_ref().unwrap();
        assert_eq!(passage.before, "We should sched");
        assert!(passage.after.is_empty());
        assert!(p.prompt.contains("Earlier facts matter."));
        assert!(p.prompt.contains("Use a formal tone."));
        assert!(p.prompt.len() < 3000);
        for (prefix, suffix, budget) in [
            ("Hello", "there", 1200),
            ("Hello", "", 4),
            ("Hello", "", 0),
            ("Hi", "", 1200),
            ("Hello ", "", 1200),
            ("これは", "", 1200),
        ] {
            assert!(prepare_sentence_end(prefix, suffix, budget, "").is_none());
        }
        assert!(prepare_sentence_end(&"x".repeat(129), "", 1200, "").is_none());
    }

    #[test]
    fn reconstructed_words_verify_typed_letters_and_preserve_editor_text() {
        for (prefix, full, expected) in [
            ("Thank you for your hel", "help with this.", "p with this."),
            ("Thank you for your pat", "patience.", "ience."),
            ("We should sched", "schedule a meeting.", "ule a meeting."),
            ("Gracias por su ayu", "ayuda.", "da."),
            ("Merci pour votre ai", "aide.", "de."),
            ("The cafe\u{301}", "café is open.", " is open."),
            ("The café", "cafe\u{301} is open.", " is open."),
            ("I don’t", "don’t know.", " know."),
            ("Hello", "Hello, how are you?", ", how are you?"),
            ("Testing", "testing the system.", " the system."),
            ("My colleague Zoë", "zoë is here.", " is here."),
        ] {
            let prepared = prepare_reconstructed(prefix, "", 1200, "").unwrap();
            assert_eq!(prepared.format, CompletionFormat::ReconstructedWord);
            let raw = serde_json::json!({"continuation":full}).to_string();
            assert!(reconstructed_response_complete(&raw));
            let insertion = finish_reconstructed(&prepared, &raw, 8);
            assert_eq!(insertion, expected);
            assert!(format!("{prefix}{insertion}").starts_with(prefix));
        }
        let prepared = prepare_reconstructed("We should sched", "", 1200, "").unwrap();
        for raw in [
            r#"{"continuation":"a meeting."}"#,
            r#"{"continuation":"We should schedule."}"#,
            r#"{"continuation":"schedule\na meeting"}"#,
            r#"{"continuation":"schedule <think>now"}"#,
            r#"{"continuation":"schedule.","replace":true}"#,
            r#"{"continuation":12}"#,
            r#"{"continuation":"sched"}"#,
            r#"{"continuation":"schedule""#,
            "schedule",
        ] {
            assert_eq!(finish_reconstructed(&prepared, raw, 8), "", "{raw}");
        }
        let raw = r#"{"continuation":"schedule a meeting with the team tomorrow morning. Then continue."}"#;
        assert_eq!(finish_reconstructed(&prepared, raw, 3), "ule a meeting");
        assert_eq!(
            finish_reconstructed(&prepared, raw, 32),
            "ule a meeting with the team tomorrow morning."
        );
        assert_eq!(finish_reconstructed(&prepared, raw, 0), "");
        let dotted_i = prepare_reconstructed("İstanbul", "", 1200, "").unwrap();
        assert_eq!(
            finish_reconstructed(&dotted_i, r#"{"continuation":"istanbul is sunny."}"#, 8),
            ""
        );
    }

    #[test]
    fn reconstruction_bounds_context_without_splitting_words_or_overriding_suffix() {
        let prefix = format!("{} hel", "é".repeat(100));
        let prepared = prepare_reconstructed(&prefix, "", 10, "Use a friendly tone.").unwrap();
        assert_eq!(prepared.fragment, "hel");
        assert!(prepared
            .prompt
            .ends_with("Context: \"éééééé \"\nStarted word: \"hel\"\n"));
        assert!(prepared
            .prompt
            .contains("Writing preferences: Use a friendly tone."));
        let single = prepare_reconstructed("Hello", "", 5, "").unwrap();
        assert!(single
            .prompt
            .ends_with("Context: \"\"\nStarted word: \"Hello\"\n"));
        for (prefix, suffix, limit) in [
            ("Hello", " existing", 1200),
            ("Hello", "", 4),
            ("Hello", "", 0),
            ("hi", "", 1200),
            ("Hello ", "", 1200),
            ("Hello!", "", 1200),
        ] {
            assert!(prepare_reconstructed(prefix, suffix, limit, "").is_none());
        }
        assert_eq!(
            prepare_instructed("Hello ", "", 1200, "").unwrap().format,
            CompletionFormat::Text
        );
    }

    #[test]
    fn midline_keeps_joining_spaces_partial_words_and_existing_punctuation() {
        for (before, after, raw, expected) in [
            ("We should ", "tomorrow.", r#"{"insert":"meet "}"#, "meet "),
            (
                "I hope your mee",
                " goes well.",
                r#"{"insert":"ting"}"#,
                "ting",
            ),
            ("Thank you for your ", ".", r#"{"insert":" help."}"#, "help"),
            (
                "I look forward to ",
                " you next week.",
                r#"{"insert":"meeting "}"#,
                "meeting",
            ),
            (
                "We should ",
                "tomorrow.",
                r#"{"insert":"meet tomorrow"}"#,
                "meet ",
            ),
            ("Try ", "and see.", r#"{"insert":"a"}"#, "a"),
        ] {
            let p = prepare_insertion(before, after, 1200, "").unwrap();
            assert_eq!(p.format, CompletionFormat::Infill);
            assert_eq!(finish_midline(&p, raw, before, after, 8), expected);
        }
    }

    #[test]
    fn midline_bounds_context_and_rejects_non_insertions() {
        let before = "é".repeat(100);
        let after = "世".repeat(100);
        let p = prepare_insertion(&before, &after, 30, "Use a friendly tone.").unwrap();
        assert!(p.prompt.ends_with(&format!(
            "Before: {}\nAfter: {}\n",
            serde_json::json!("é".repeat(20)),
            serde_json::json!("世".repeat(10))
        )));
        assert!(p
            .prompt
            .contains("Writing preferences: Use a friendly tone."));
        for raw in [
            r#"{"insert":"hello\nworld"}"#,
            r#"{"insert":"<think>hi"}"#,
            r#"{"insert":12}"#,
            r#"{"insert":"hi","replace":true}"#,
            "hi",
            "{\"insert\":",
        ] {
            assert_eq!(finish_midline(&p, raw, &before, &after, 8), "");
        }
        assert!(!midline_response_complete("{\"insert\":"));
        assert!(midline_response_complete(r#"{"insert":"meet "}"#));
        assert_eq!(
            finish_midline(
                &p,
                r#"{"insert":"Please send the report"}"#,
                "Please send ",
                " by Friday",
                8
            ),
            ""
        );
        assert!(prepare_midline("Hi", "there", 1200, "").is_none());
        assert!(prepare_midline("Hello", "", 1200, "").is_none());
    }

    #[test]
    fn cjk_continuation_does_not_require_echoing_an_unspaced_clause() {
        for prefix in [
            "資料を確認してから",
            "我会在审核文件后",
            "문서를 확인한 후에",
        ] {
            let p = prepare(prefix, "", 5).unwrap();
            assert_eq!(
                p.prompt,
                prefix
                    .chars()
                    .rev()
                    .take(5)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
            );
            assert!(p.fragment.is_empty());
            assert_eq!(finish(&p, "再联系您。", 8), "再联系您。");
            assert!(prepare_reconstructed(prefix, "", 1200, "").is_none());
            assert_eq!(
                prepare_instructed(prefix, "", 1200, "Be concise.")
                    .unwrap()
                    .format,
                CompletionFormat::Infill
            );
            assert_eq!(
                prepare_structured(prefix, "", 1200, "Be concise.")
                    .unwrap()
                    .format,
                CompletionFormat::Infill
            );
        }
        // Latin fragments echo a bounded sentence and verify all existing text.
        assert_eq!(
            prepare_instructed("Thanks for your underst", "", 1200, "")
                .unwrap()
                .format,
            CompletionFormat::CompletedPassage
        );
    }

    #[test]
    fn an_insertion_inside_a_word_cannot_create_a_spurious_phrase() {
        let p = prepare_insertion("I enjoyed the con", "versation yesterday.", 1200, "").unwrap();
        for raw in [
            r#"{"insert":"tact "}"#,
            r#"{"insert":"! "}"#,
            r#"{"insert":" beautiful "}"#,
        ] {
            assert_eq!(
                finish_midline(&p, raw, "I enjoyed the con", "versation yesterday.", 8),
                ""
            );
        }
        let p = prepare_insertion("The trans", "ation worked.", 1200, "").unwrap();
        assert_eq!(
            finish_midline(&p, r#"{"insert":"form"}"#, "The trans", "ation worked.", 8),
            "form"
        );
        let p = prepare_insertion("確認しました", "連絡します。", 1200, "").unwrap();
        assert_eq!(
            finish_midline(
                &p,
                r#"{"insert":"、後ほど"}"#,
                "確認しました",
                "連絡します。",
                8
            ),
            "、後ほど"
        );
    }

    #[test]
    fn cjk_sentence_caps_do_not_require_spaces_between_sentences() {
        let p = prepare("資料を確認してから", "", 1200).unwrap();
        for (raw, expected) in [
            ("、連絡します。よろしくお願いします。", "、連絡します。"),
            ("可以吗？请告诉我。", "可以吗？"),
            ("ありがとうございます！また明日。", "ありがとうございます！"),
        ] {
            assert!(has_complete_limit(&p, raw, 8));
            assert_eq!(finish(&p, raw, 8), expected);
        }
        assert!(!has_complete_limit(&p, "、連絡します", 8));
        assert_eq!(
            finish(&p, " version 3.14 works.", 8),
            " version 3.14 works."
        );
    }

    #[test]
    fn reconstructs_partial_word_without_duplication() {
        let p = prepare("I hope your mee", "", 1200).unwrap();
        assert_eq!(p.prompt, "I hope your");
        assert_eq!(p.fragment, " mee");
        assert_eq!(finish(&p, " meeting goes well.", 12), "ting goes well.");
        assert_eq!(finish(&p, " week goes well.", 12), "");
        assert_eq!(finish(&p, " Meeting goes well.", 12), "");
    }

    #[test]
    fn preserves_insertion_whitespace() {
        let p = prepare("Here is a sentence.", "", 1200).unwrap();
        assert_eq!(finish(&p, " Another follows.", 12), " Another follows.");
        let p = prepare("I hope you have a ", "", 1200).unwrap();
        assert_eq!(finish(&p, " great day.", 12), "great day.");
        let p = prepare("I hope you have a", "", 1200).unwrap();
        assert_eq!(finish(&p, " a great day.", 12), " great day.");
        let p = prepare("I hope you have a  ", "", 1200).unwrap();
        assert_eq!(finish(&p, " great day.", 12), "");
    }

    #[test]
    fn unicode_fragment_and_context_are_valid() {
        let p = prepare("Je prends un caf", "", 10).unwrap();
        assert_eq!(p.prompt, " prends un");
        assert_eq!(finish(&p, " café demain.", 12), "é demain.");
        let p = prepare("Un cafe\u{301}", "", 5).unwrap();
        assert_eq!(p.fragment, " cafe\u{301}");
        assert_eq!(finish(&p, " cafe\u{301} délicieux", 12), " délicieux");
        assert!(prepare("你好 世界 ", "", 3)
            .unwrap()
            .prompt
            .is_char_boundary(0));
    }

    #[test]
    fn short_caps_and_untrusted_output() {
        let p = prepare("Please send ", "", 100).unwrap();
        assert_eq!(
            finish(&p, " the report before lunch tomorrow", 3),
            "the report before"
        );
        assert_eq!(finish(&p, " the report. Then wait.", 12), "the report.");
        assert_eq!(finish(&p, " the report\nAnother line", 12), "the report");
        assert_eq!(finish(&p, "<think>first reason</think>the report", 12), "");
        assert_eq!(finish(&p, "the\u{1b}report", 12), "");
        assert_eq!(finish(&p, "assistant: the report", 12), "");
    }

    #[test]
    fn stream_stops_only_at_complete_boundaries() {
        let p = prepare("The result is ", "", 100).unwrap();
        assert!(!has_complete_limit(&p, " 3.", 8));
        assert!(!has_complete_limit(&p, " 3.14", 8));
        assert!(has_complete_limit(&p, " 3.14. ", 8));
        assert!(!has_complete_limit(&p, " one two", 2));
        assert!(has_complete_limit(&p, " one two ", 2));
        assert_eq!(without_partial_tail(" one complica"), " one ");
        assert_eq!(without_partial_tail(" café déli"), " café ");
        assert_eq!(without_partial_tail(" done."), " done.");
    }

    #[test]
    fn suppresses_midline_and_empty_context() {
        assert!(prepare("I hope ", "existing text", 1200).is_none());
        assert!(prepare("Hello", "", 1200).is_none());
        assert!(prepare("  ", "", 1200).is_none());
        assert!(prepare("I hope ", " \n", 1200).is_some());
    }

    #[test]
    fn fallback_handles_short_text_word_endings_and_rich_editor_spaces() {
        let p = prepare_fallback("Thank you for your", "", 1200).unwrap();
        assert_eq!(
            finish(&p, " time and attention.", 8),
            " time and attention."
        );
        let p = prepare_fallback("Testing", "", 1200).unwrap();
        assert_eq!(finish(&p, " is going well.", 8), " is going well.");
        let p = prepare_fallback("I hope your mee", "", 1200).unwrap();
        assert_eq!(finish(&p, "ting goes well.", 8), "ting goes well.");
        for space in [" ", "  ", "\u{a0}", "\u{202f}"] {
            let p = prepare_fallback(&format!("Hello{space}"), "", 1200).unwrap();
            assert_eq!(finish(&p, " there, how are you?", 8), "there, how are you?");
        }
        assert!(prepare_fallback("Hello", "existing text", 1200).is_none());
        assert!(prepare_fallback(" \u{a0}", "", 1200).is_none());
    }

    #[test]
    fn writing_preferences_preserve_insertion_and_context_guards() {
        let p = prepare_instructed("Hola ", "", 1200, "Write in Spanish.").unwrap();
        assert!(p.prompt.contains("Write in Spanish."));
        assert!(p.prompt.ends_with("Text: Hola\nContinuation:"));
        assert_eq!(finish(&p, " ¿Cómo estás?", 8), "¿Cómo estás?");
        assert!(prepare_instructed("Hola", "existing", 1200, "Continue anyway").is_none());
        assert_eq!(
            prepare_instructed("Hello ", "", 1200, ""),
            prepare_fallback("Hello ", "", 1200)
        );
    }
}
