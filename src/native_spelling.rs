// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! macOS spelling service. No learning, response recording or user dictionary edits.
use super::*;
use crate::{
    config::SpellingSettings,
    spelling::{Correction, WordCandidate},
};

pub(super) unsafe fn complete_words(fragment: &str, language: &str) -> Vec<String> {
    if fragment.is_empty() || fragment.chars().count() > 48 {
        return vec![];
    }
    let pool: id = msg_send![class!(NSAutoreleasePool), new];
    let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
    let text = ns(fragment);
    let lang = if language.is_empty() {
        let current: id = msg_send![checker, language];
        ns(&ns_text(current))
    } else {
        ns(language)
    };
    let tag: isize = msg_send![class!(NSSpellChecker), uniqueSpellDocumentTag];
    let range = Range {
        location: 0,
        length: fragment.encode_utf16().count() as isize,
    };
    let words: id = msg_send![checker,completionsForPartialWordRange:range inString:text language:lang inSpellDocumentWithTag:tag];
    let count: usize = msg_send![words, count];
    let mut result = Vec::new();
    for i in 0..count.min(64) {
        let word: id = msg_send![words,objectAtIndex:i];
        let word = ns_text(word);
        if word.chars().count() <= 80
            && word.chars().count() > fragment.chars().count()
            && word.starts_with(fragment)
            && word.chars().all(|c| {
                c.is_alphabetic()
                    || unicode_normalization::char::is_combining_mark(c)
                    || matches!(c, '\'' | '’' | '-')
            })
        {
            result.push(word);
        }
    }
    let _: () = msg_send![checker,closeSpellDocumentWithTag:tag];
    let _: () = msg_send![lang, release];
    let _: () = msg_send![text, release];
    let _: () = msg_send![pool, drain];
    result
}

pub(super) unsafe fn suggest(prefix: &str, suffix: &str, language: &str) -> Option<Correction> {
    let candidate = WordCandidate::new(prefix, suffix)?;
    // At an unfinished word, a valid dictionary continuation takes precedence
    // over a shorter spelling guess (for example, "forw" -> "for"). Once the
    // user types a separator, the completed word can be checked normally.
    if !prefix.ends_with(char::is_whitespace)
        && !complete_words(&candidate.word, language).is_empty()
    {
        return None;
    }
    let pool: id = msg_send![class!(NSAutoreleasePool), new];
    let result = (|| {
        let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
        if checker == nil {
            return None;
        }
        let lang = if language.is_empty() {
            let current: id = msg_send![checker, language];
            ns(&ns_text(current))
        } else {
            ns(language)
        };
        let available: id = msg_send![checker, availableLanguages];
        let supported: BOOL = msg_send![available,containsObject:lang];
        if supported != YES {
            let _: () = msg_send![lang, release];
            return None;
        }
        let text = ns(&candidate.context);
        let tag: isize = msg_send![class!(NSSpellChecker), uniqueSpellDocumentTag];
        let range = Range {
            location: candidate.context_start_utf16 as isize,
            length: candidate.word_utf16 as isize,
        };
        // This API returns a correction only when the specified word is
        // misspelled; it does not alter editor text or show a panel.
        let proposed: id = msg_send![checker,correctionForWordRange:range inString:text language:lang inSpellDocumentWithTag:tag];
        let correction = candidate.corrected(prefix, suffix, &ns_text(proposed));
        let _: () = msg_send![checker,closeSpellDocumentWithTag:tag];
        let _: () = msg_send![text, release];
        let _: () = msg_send![lang, release];
        correction
    })();
    let _: () = msg_send![pool, drain];
    result
}

pub(super) struct SpellingUi {
    window: id,
    enabled: id,
    fixes: id,
    language: id,
    languages: Vec<String>,
    input: id,
    suffix: id,
    output: id,
    status: id,
}
impl SpellingUi {
    pub(super) unsafe fn new(target: id, settings: &SpellingSettings) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,440.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Spelling");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Spelling",
            NSRect::new(NSPoint::new(24., 384.), NSSize::new(570., 36.)),
            24.,
        );
        label(view,"Suggested fixes replace only the identified word after you accept them.\nUses the macOS spelling service; no dictionary learning or history is recorded.",NSRect::new(NSPoint::new(24.,332.),NSSize::new(570.,44.)),12.);
        let mut boxes = Vec::new();
        for (y, title, checked) in [
            (
                290.,
                "Hold back completions when a likely typo is found",
                settings.enabled,
            ),
            (
                256.,
                "Show the suggested fix (accept with your word or full shortcut)",
                settings.show_fixes,
            ),
        ] {
            let b: id = msg_send![class!(NSButton), alloc];
            let b: id =
                msg_send![b,initWithFrame:NSRect::new(NSPoint::new(24.,y),NSSize::new(570.,26.))];
            let _: () = msg_send![b,setButtonType:3usize];
            set_title(b, title);
            let _: () = msg_send![b,setState:if checked{1isize}else{0isize}];
            let _: () = msg_send![view,addSubview:b];
            let _: () = msg_send![b, release];
            boxes.push(b);
        }
        label(
            view,
            "Spelling language",
            NSRect::new(NSPoint::new(24., 218.), NSSize::new(180., 26.)),
            13.,
        );
        let language: id = msg_send![class!(NSPopUpButton), alloc];
        let language: id = msg_send![language,initWithFrame:NSRect::new(NSPoint::new(210.,214.),NSSize::new(382.,30.)) pullsDown:NO];
        let name = ns("Spelling language");
        let _: () = msg_send![language,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let title = ns("System spelling language");
        let _: () = msg_send![language,addItemWithTitle:title];
        let _: () = msg_send![title, release];
        let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
        let available: id = msg_send![checker, availableLanguages];
        let count: usize = msg_send![available, count];
        let mut languages = vec![String::new()];
        let locale: id = msg_send![class!(NSLocale), currentLocale];
        for i in 0..count {
            let value: id = msg_send![available,objectAtIndex:i];
            languages.push(ns_text(value));
            let display: id = msg_send![locale,localizedStringForLocaleIdentifier:value];
            let _: () =
                msg_send![language,addItemWithTitle:if display == nil { value } else { display }];
        }
        let index = languages
            .iter()
            .position(|s| s == &settings.language)
            .unwrap_or(0);
        let _: () = msg_send![language,selectItemAtIndex:index as isize];
        let _: () = msg_send![view,addSubview:language];
        let _: () = msg_send![language, release];
        label(
            view,
            "Preview: text before cursor | text after cursor",
            NSRect::new(NSPoint::new(24., 182.), NSSize::new(570., 24.)),
            12.,
        );
        let input = text_input(
            view,
            "Please recieve",
            "Spelling preview before cursor",
            NSRect::new(NSPoint::new(24., 146.), NSSize::new(265., 28.)),
        );
        let suffix = text_input(
            view,
            " the file.",
            "Spelling preview after cursor",
            NSRect::new(NSPoint::new(299., 146.), NSSize::new(170., 28.)),
        );
        button(
            view,
            "Check",
            NSRect::new(NSPoint::new(480., 143.), NSSize::new(114., 32.)),
            target,
            sel!(checkSpellingPreview:),
        );
        let output = label(
            view,
            "Preview does not change any document.",
            NSRect::new(NSPoint::new(24., 82.), NSSize::new(570., 58.)),
            13.,
        );
        let status = label(
            view,
            "Changes apply after saving.",
            NSRect::new(NSPoint::new(24., 17.), NSSize::new(410., 54.)),
            12.,
        );
        button(
            view,
            "Save",
            NSRect::new(NSPoint::new(480., 22.), NSSize::new(114., 32.)),
            target,
            sel!(saveSpelling:),
        );
        Self {
            window,
            enabled: boxes[0],
            fixes: boxes[1],
            language,
            languages,
            input,
            suffix,
            output,
            status,
        }
    }
    unsafe fn draft(&self) -> SpellingSettings {
        let enabled: isize = msg_send![self.enabled, state];
        let fixes: isize = msg_send![self.fixes, state];
        let language: isize = msg_send![self.language, indexOfSelectedItem];
        SpellingSettings {
            enabled: enabled == 1,
            show_fixes: fixes == 1,
            language: self
                .languages
                .get(language as usize)
                .cloned()
                .unwrap_or_default(),
        }
    }
    unsafe fn message(&self, value: &str) {
        let text = ns(value);
        let _: () = msg_send![self.status,setStringValue:text];
        let _: () = msg_send![text, release];
    }
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.spelling.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                config.spelling = s.spelling.draft();
                match config.save() {
                    Ok(()) => {
                        s.clear();
                        s.config = config;
                        s.spelling
                            .message("Saved. Spelling preferences apply to new suggestions.");
                    }
                    Err(e) => s.spelling.message(&format!("Not saved: {e}")),
                }
            }
        }
    });
}
extern "C" fn check(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let settings = s.spelling.draft();
                let before = ns_text(msg_send![s.spelling.input, stringValue]);
                let after = ns_text(msg_send![s.spelling.suffix, stringValue]);
                let result = if !settings.enabled {
                    "Spelling assistance is disabled in this draft.".into()
                } else {
                    match suggest(&before, &after, &settings.language) {
                        Some(c) if settings.show_fixes => {
                            format!("{} → {}\nResult: {}", c.word, c.corrected, c.plan.preview())
                        }
                        Some(_) => {
                            "Likely typo: completions would pause; suggested fixes are hidden."
                                .into()
                        }
                        None => {
                            "No confident correction. Normal completions remain available.".into()
                        }
                    }
                };
                let text = ns(&result);
                let _: () = msg_send![s.spelling.output,setStringValue:text];
                let _: () = msg_send![text, release];
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(spelling:), show as extern "C" fn(&Object, Sel, id));
    decl.add_method(sel!(saveSpelling:), save as extern "C" fn(&Object, Sel, id));
    decl.add_method(
        sel!(checkSpellingPreview:),
        check as extern "C" fn(&Object, Sel, id),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "uses the installed macOS spelling service on synthetic words only"]
    fn unfinished_word_completion_takes_priority_over_spelling_live() {
        unsafe {
            for language in ["en", ""] {
                let words = complete_words("forw", language);
                assert!(words.iter().any(|word| word == "forward"), "{words:?}");
                let correction = suggest("I look forw", "", language);
                assert!(
                    correction.is_none(),
                    "An unfinished dictionary prefix must not become a typo replacement: {:?}",
                    correction.map(|c| c.corrected)
                );
                assert_eq!(
                    suggest("Please recieve ", "the file.", language)
                        .unwrap()
                        .plan
                        .preview(),
                    "Please receive the file."
                );
            }
        }
    }
    #[test]
    #[ignore = "uses the installed macOS spelling service on synthetic words only"]
    fn system_spelling_live() {
        unsafe {
            let words = complete_words("sche", "en");
            eprintln!("synthetic dictionary candidates={words:?}");
            assert!(words.iter().any(|w| w == "schedule"));
            for word in ["recieve", "seperate", "definately"] {
                let start = Instant::now();
                let correction = suggest(&format!("Please {word}"), "", "en");
                eprintln!(
                    "synthetic word={word:?} correction={:?} ms={}",
                    correction.as_ref().map(|c| &c.corrected),
                    start.elapsed().as_millis()
                );
                assert!(correction.is_some());
            }
            assert!(suggest("We should sched", "", "en").is_none());
            assert!(suggest("Please receive", "", "en").is_none());
            assert_eq!(
                suggest("Please recieve  ", "the file.", "en")
                    .unwrap()
                    .plan
                    .preview(),
                "Please receive  the file."
            );
            assert!(suggest("Please recieve", "", "").is_some());
        }
    }
}
