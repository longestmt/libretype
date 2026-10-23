// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Word-choice preferences, deferred inference, numbered previews and explicit edits.
use super::*;
use crate::alternatives::{Choice, ChoiceSettings, Choices, Request, RevealMode};

pub(super) struct Reply {
    pub id: u64,
    pub result: Result<Vec<Choice>, String>,
}
type Work = (Config, Request);
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) enum Progress {
    #[default]
    Untried,
    Pending,
    Ready,
    Unavailable,
}
impl Progress {
    fn begin(&mut self, request: &Request) -> bool {
        if !request.is_synonym()
            && !request.started_word().is_empty()
            && request.candidates.is_empty()
        {
            *self = Self::Unavailable;
            false
        } else {
            *self = Self::Pending;
            true
        }
    }
    fn finish(&mut self, has_choices: bool) {
        *self = if has_choices {
            Self::Ready
        } else {
            Self::Unavailable
        };
    }
    pub(super) fn blocks_completion(self, selected: bool) -> bool {
        selected || matches!(self, Self::Pending | Self::Ready)
    }
}
pub(super) fn worker(epoch: Arc<AtomicU64>) -> (mpsc::SyncSender<Work>, mpsc::Receiver<Reply>) {
    let (tx, rx) = mpsc::sync_channel::<Work>(1);
    let (out, replies) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(mut work) = rx.recv() {
            while let Ok(new) = rx.try_recv() {
                work = new;
            }
            let (config, request) = work;
            let result =
                model::generate_choices(&config, &request, &epoch).map_err(|e| e.to_string());
            let _ = out.send(Reply {
                id: request.id,
                result,
            });
        }
    });
    (tx, replies)
}
pub(super) fn number_choice(key: i64, flags: u64) -> Option<usize> {
    if flags & MODIFIERS != 0 {
        return None;
    }
    match key {
        18 | 83 => Some(0),
        19 | 84 => Some(1),
        20 | 85 => Some(2),
        21 | 86 => Some(3),
        23 | 87 => Some(4),
        _ => None,
    }
}
fn request(ctx: &Context, id: u64) -> Request {
    Request {
        id,
        prefix: ctx.prefix.clone(),
        selected: ctx.text[ctx.prefix.len()..ctx.text.len() - ctx.suffix.len()].into(),
        suffix: ctx.suffix.clone(),
        app_id: ctx.app.clone(),
        website: ctx.website.clone(),
        profile: None,
        background: crate::background::Background::default(),
        candidates: vec![],
    }
}
impl State {
    pub(super) unsafe fn request_word_choices(&mut self) -> bool {
        let Some(ctx) = &self.context else {
            return false;
        };
        if ctx.terminal.is_some() {
            return false;
        }
        let mut request = request(ctx, 0);
        request.profile = self.history.profile(&self.config, ctx);
        if !request.valid() || request.mode(&self.config.choices) == RevealMode::Off {
            return false;
        }
        request.candidates =
            spelling::complete_words(&request.started_word(), &self.config.spelling.language);
        // An unmatched fragment needs no inference. Keep any ordinary ghost
        // text/request, and remember the attempt until this context changes.
        if !self.choice_progress.begin(&request) {
            return false;
        }
        request.background = background::collect(&self.config, ctx);
        self.preserving_remainder = false;
        self.requested = true;
        self.suggestion.clear();
        self.replacement = None;
        self.emoji_choices = None;
        let _: () = msg_send![self.panel,orderOut:nil];
        request.id = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        if !self.submit_with_screen(screen::Work::Choices(request)) {
            self.choice_progress = Progress::Untried;
            self.requested = false;
            self.pipeline_status = "Word-choice worker busy";
            return false;
        }
        self.pipeline_status = if self.screen.waiting() {
            "Reading active-window text locally"
        } else {
            "Generating word choices locally"
        };
        true
    }
    pub(super) unsafe fn process_choice_replies(&mut self) {
        while let Ok(reply) = self.choice_replies.try_recv() {
            if reply.id != self.epoch.load(Ordering::SeqCst) {
                continue;
            }
            if self.preview_epoch == Some(reply.id) {
                self.preview_epoch = None;
                if let Some(request) = self.choice_preview_request.take() {
                    self.choice_ui.receive(request, reply.result);
                }
                continue;
            }
            let current = capture_selected(&self.config, 0.08, true);
            let valid = match (&self.context, &current) {
                (Some(a), Some(b)) => a.same(b),
                _ => false,
            };
            if !valid {
                self.clear();
                continue;
            }
            let has_choices = reply.result.as_ref().is_ok_and(|items| !items.is_empty());
            self.choice_progress.finish(has_choices);
            if !has_choices {
                // The timer must not repeat a failed choice request, but an
                // ordinary continuation may now proceed for this same caret.
                self.requested = false;
            }
            match reply.result {
                Ok(items) if !items.is_empty() => {
                    self.word_choices = Some(Choices {
                        request: request(current.as_ref().unwrap(), reply.id),
                        items,
                        selected: 0,
                    });
                    self.overlay_word_choices();
                }
                Ok(_) => self.pipeline_status = "No suitable word choices",
                Err(e) => {
                    self.status_text = e;
                    self.pipeline_status = "Word-choice generation failed";
                }
            }
        }
    }
    pub(super) unsafe fn move_word_choice(&mut self, next: bool) {
        let valid = match (&self.context, capture_selected(&self.config, 0.08, true)) {
            (Some(a), Some(b)) => a.same(&b),
            _ => false,
        };
        if !valid {
            self.clear();
            return;
        }
        if let Some(choices) = self.word_choices.as_mut() {
            choices.step(next);
        }
        self.overlay_word_choices();
    }
    pub(super) unsafe fn accept_word_choice(&mut self, index: usize) -> bool {
        let Some(choices) = self.word_choices.clone() else {
            return false;
        };
        let Some(choice) = choices.items.get(index) else {
            return false;
        };
        let valid = match (&self.context, capture_selected(&self.config, 0.08, true)) {
            (Some(a), Some(b)) => a.same(&b),
            _ => false,
        };
        if !valid {
            self.clear();
            return false;
        }
        self.word_choices = None;
        self.choice_progress = Progress::Untried;
        self.suggestion = choice.insertion.clone();
        self.replacement = choices.request.replacement(choice);
        self.replacement_actionable = true;
        self.accept(true)
    }
    pub(super) unsafe fn overlay_word_choices(&mut self) {
        let Some(ctx) = &self.context else {
            return;
        };
        let Some(choices) = self.word_choices.as_mut() else {
            return;
        };
        let screens: id = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        self.suggestion.clear();
        if count == 0 {
            let _: () = msg_send![self.panel,orderOut:nil];
            return;
        }
        let primary: id = msg_send![screens,objectAtIndex:0usize];
        let main: NSRect = msg_send![primary, frame];
        let font: id = msg_send![class!(NSFont),systemFontOfSize:14f64];
        let color: id = msg_send![class!(NSColor), labelColor];
        let attributes: id = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![attributes,setObject:font forKey:NSFontAttributeName];
        let _: () = msg_send![attributes,setObject:color forKey:NSForegroundColorAttributeName];
        let measure = |s: &str| {
            let text = ns(s);
            let size: NSSize = msg_send![text,sizeWithAttributes:attributes];
            let _: () = msg_send![text, release];
            size
        };
        let row_height = measure("Ag").height.ceil();
        let mut selected_frame = None;
        for i in 0..count {
            let screen: id = msg_send![screens,objectAtIndex:i];
            let screen: NSRect = msg_send![screen, frame];
            if let Some((frame, rows)) = list_frame(
                ctx.caret,
                ctx.field_bounds,
                screen,
                main.origin.y + main.size.height,
                row_height,
                choices.items.len(),
            ) {
                choices.items.retain(|choice| {
                    measure(&format!("› 5  {}", choice.label)).width <= frame.size.width - 8.
                });
                choices.items.truncate(rows);
                if choices.items.is_empty() {
                    break;
                }
                choices.selected = choices.selected.min(choices.items.len() - 1);
                selected_frame = Some(frame);
                break;
            }
        }
        let Some(mut frame) = selected_frame else {
            let _: () = msg_send![self.panel,orderOut:nil];
            self.pipeline_status = "No room for word choices";
            return;
        };
        self.suggestion = choices.lines();
        let size = measure(&self.suggestion);
        frame.size.width = (size.width.ceil() + 8.).min(frame.size.width);
        let background: id = msg_send![class!(NSColor), windowBackgroundColor];
        let _: () = msg_send![self.panel,setBackgroundColor:background];
        (*self.ghost).set_ivar("ghostInset", 4f64);
        let string = ns(&self.suggestion);
        let text: id = msg_send![class!(NSAttributedString), alloc];
        let text: id = msg_send![text,initWithString:string attributes:attributes];
        let _: () = msg_send![string, release];
        let previous: id = *(*self.ghost).get_ivar("ghostText");
        (*self.ghost).set_ivar("ghostText", text);
        let _: () = msg_send![previous, release];
        let _: () = msg_send![self.ghost,setFrame:NSRect::new(NSPoint::new(0.,0.),frame.size)];
        let _: () = msg_send![self.ghost,setNeedsDisplay:YES];
        let _: () = msg_send![self.panel,setFrame:frame display:YES];
        let _: () = msg_send![self.panel, orderFrontRegardless];
        self.pipeline_status = "Numbered word choices visible";
    }
}
fn list_frame(
    caret: NSRect,
    field: NSRect,
    screen: NSRect,
    primary_top: f64,
    row_height: f64,
    rows: usize,
) -> Option<(NSRect, usize)> {
    if row_height <= 0. || !row_height.is_finite() || rows == 0 {
        return None;
    }
    let mut frame = midline_frame(caret, field, screen, primary_top)?;
    let bottom = primary_top - caret.origin.y - caret.size.height;
    let below = (bottom - screen.origin.y - 8.).max(0.);
    let above = (screen.origin.y + screen.size.height - bottom - caret.size.height - 8.).max(0.);
    let desired = row_height * rows as f64 + 8.;
    let use_above = below < desired && above > below;
    let room = if use_above { above } else { below };
    let count = rows.min(((room - 8.).max(0.) / row_height).floor() as usize);
    if count == 0 {
        return None;
    }
    frame.size.height = row_height * count as f64 + 8.;
    frame.origin.y = if use_above {
        bottom + caret.size.height + 4.
    } else {
        bottom - 4. - frame.size.height
    };
    Some((frame, count))
}

pub(super) struct ChoiceUi {
    window: id,
    alternatives: id,
    synonyms: id,
    delay: id,
    before: id,
    selected: id,
    after: id,
    picker: id,
    output: id,
    status: id,
    choices: Option<Choices>,
}
unsafe fn mode_picker(view: id, target: id, title: &str, y: f64, mode: RevealMode) -> id {
    label(
        view,
        title,
        NSRect::new(NSPoint::new(24., y + 3.), NSSize::new(250., 26.)),
        13.,
    );
    let picker: id = msg_send![class!(NSPopUpButton), alloc];
    let picker: id = msg_send![picker,initWithFrame:NSRect::new(NSPoint::new(285.,y),NSSize::new(310.,30.)) pullsDown:NO];
    for title in ["Off", "After a pause", "Only on shortcut"] {
        let title = ns(title);
        let _: () = msg_send![picker,addItemWithTitle:title];
        let _: () = msg_send![title, release];
    }
    let name = ns(title);
    let _: () = msg_send![picker,setAccessibilityLabel:name];
    let _: () = msg_send![name, release];
    let _: () = msg_send![picker,selectItemAtIndex:match mode {RevealMode::Off=>0isize,RevealMode::Automatic=>1,RevealMode::Shortcut=>2}];
    let _: () = msg_send![picker,setTarget:target];
    let _: () = msg_send![view,addSubview:picker];
    let _: () = msg_send![picker, release];
    picker
}
impl ChoiceUi {
    pub(super) unsafe fn new(target: id, settings: &ChoiceSettings) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,540.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Word choices");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Word choices",
            NSRect::new(NSPoint::new(24., 484.), NSSize::new(570., 36.)),
            24.,
        );
        label(view,"Alternatives finish the word you are typing. Synonyms replace selected text.\nPress a number (1–5), or use ↑/↓ and your acceptance shortcut to choose.\nThe reveal shortcut defaults to Control-Option-A; customize it in Keyboard shortcuts.",NSRect::new(NSPoint::new(24.,413.),NSSize::new(570.,64.)),12.);
        let alternatives = mode_picker(
            view,
            target,
            "Next-word alternatives",
            371.,
            settings.alternatives,
        );
        let synonyms = mode_picker(
            view,
            target,
            "Synonyms for selected text",
            331.,
            settings.synonyms,
        );
        label(
            view,
            "Automatic delay in ms (250–10000)",
            NSRect::new(NSPoint::new(24., 289.), NSSize::new(310., 26.)),
            13.,
        );
        let delay = text_input(
            view,
            &settings.delay_ms.to_string(),
            "Word-choice delay",
            NSRect::new(NSPoint::new(354., 289.), NSSize::new(115., 26.)),
        );
        label(
            view,
            "Preview: before | selected text (blank for alternatives) | after",
            NSRect::new(NSPoint::new(24., 252.), NSSize::new(570., 26.)),
            12.,
        );
        let before = text_input(
            view,
            "The proposal is ",
            "Word-choice text before",
            NSRect::new(NSPoint::new(24., 216.), NSSize::new(200., 28.)),
        );
        let selected = text_input(
            view,
            "great",
            "Selected word or phrase",
            NSRect::new(NSPoint::new(234., 216.), NSSize::new(160., 28.)),
        );
        let after = text_input(
            view,
            ".",
            "Word-choice text after",
            NSRect::new(NSPoint::new(404., 216.), NSSize::new(190., 28.)),
        );
        button(
            view,
            "Generate",
            NSRect::new(NSPoint::new(480., 175.), NSSize::new(114., 32.)),
            target,
            sel!(previewChoices:),
        );
        let picker: id = msg_send![class!(NSPopUpButton), alloc];
        let picker: id = msg_send![picker,initWithFrame:NSRect::new(NSPoint::new(22.,133.),NSSize::new(572.,32.)) pullsDown:NO];
        let name = ns("Generated word choices");
        let _: () = msg_send![picker,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![picker,setTarget:target];
        let _: () = msg_send![picker,setAction:sel!(chooseWordPreview:)];
        let _: () = msg_send![picker,setEnabled:NO];
        let _: () = msg_send![view,addSubview:picker];
        let _: () = msg_send![picker, release];
        let output = label(
            view,
            "Preview uses the local model without changing another document.",
            NSRect::new(NSPoint::new(24., 70.), NSSize::new(570., 60.)),
            13.,
        );
        let status = label(
            view,
            "Save to apply modes and delay in other apps.",
            NSRect::new(NSPoint::new(24., 15.), NSSize::new(440., 48.)),
            12.,
        );
        button(
            view,
            "Save",
            NSRect::new(NSPoint::new(480., 22.), NSSize::new(114., 32.)),
            target,
            sel!(saveChoices:),
        );
        Self {
            window,
            alternatives,
            synonyms,
            delay,
            before,
            selected,
            after,
            picker,
            output,
            status,
            choices: None,
        }
    }
    unsafe fn draft(&self) -> Result<ChoiceSettings, String> {
        let mode = |picker: id| {
            let i: isize = msg_send![picker, indexOfSelectedItem];
            match i {
                1 => RevealMode::Automatic,
                2 => RevealMode::Shortcut,
                _ => RevealMode::Off,
            }
        };
        Ok(ChoiceSettings {
            alternatives: mode(self.alternatives),
            synonyms: mode(self.synonyms),
            delay_ms: ns_text(msg_send![self.delay, stringValue])
                .trim()
                .parse()
                .map_err(|_| "Enter a numeric delay.".to_string())?,
        })
    }
    unsafe fn message(&self, value: &str) {
        set_text(self.status, value);
    }
    unsafe fn receive(&mut self, request: Request, result: Result<Vec<Choice>, String>) {
        let _: () = msg_send![self.picker, removeAllItems];
        self.choices = None;
        match result {
            Ok(items) if !items.is_empty() => {
                for (i, choice) in items.iter().enumerate() {
                    let title = ns(&format!("{}  {}", i + 1, choice.label));
                    let _: () = msg_send![self.picker,addItemWithTitle:title];
                    let _: () = msg_send![title, release];
                }
                self.choices = Some(Choices {
                    request,
                    items,
                    selected: 0,
                });
                self.preview_selected();
                self.message("Preview ready. Save to apply these settings.");
            }
            Ok(_) => set_text(
                self.output,
                "No suitable choices. Try another word or more context.",
            ),
            Err(e) => set_text(self.output, &e),
        }
        let _: () = msg_send![self.picker,setEnabled:if self.choices.is_some(){YES}else{NO}];
    }
    unsafe fn preview_selected(&self) {
        if let Some(choices) = &self.choices {
            let choice = &choices.items[choices.selected];
            let result = choices
                .request
                .replacement(choice)
                .map(|p| p.preview())
                .unwrap_or_else(|| {
                    format!(
                        "{}{}{}",
                        choices.request.prefix, choice.insertion, choices.request.suffix
                    )
                });
            set_text(
                self.output,
                &format!("Choose: {}\nResult: {result}", choice.label),
            );
        }
    }
}
unsafe fn set_text(field: id, value: &str) {
    let text = ns(value);
    let _: () = msg_send![field,setStringValue:text];
    let _: () = msg_send![text, release];
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.choice_ui.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                match s.choice_ui.draft() {
                    Ok(draft) => config.choices = draft,
                    Err(e) => {
                        s.choice_ui.message(&e);
                        return;
                    }
                }
                match config.save() {
                    Ok(()) => {
                        let pending_preview = s.choice_preview_request.is_some();
                        s.clear();
                        if pending_preview {
                            set_text(
                                s.choice_ui.output,
                                "Settings saved. Generate again to preview.",
                            );
                        }
                        s.config = config;
                        s.choice_ui
                            .message("Saved. Word-choice preferences apply in other apps.");
                    }
                    Err(e) => s.choice_ui.message(&format!("Not saved: {e}")),
                }
            }
        }
    });
}
extern "C" fn preview(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                match s.choice_ui.draft() {
                    Ok(draft) => config.choices = draft,
                    Err(e) => {
                        s.choice_ui.message(&e);
                        return;
                    }
                }
                if let Err(e) = config.validate() {
                    s.choice_ui.message(&e);
                    return;
                }
                s.clear();
                let id = s.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                let mut request = Request {
                    id,
                    prefix: ns_text(msg_send![s.choice_ui.before, stringValue]),
                    selected: ns_text(msg_send![s.choice_ui.selected, stringValue]),
                    suffix: ns_text(msg_send![s.choice_ui.after, stringValue]),
                    app_id: "local.libretypist.preview".into(),
                    website: None,
                    profile: None,
                    background: crate::background::Background::default(),
                    candidates: vec![],
                };
                s.choice_ui.choices = None;
                let _: () = msg_send![s.choice_ui.picker, removeAllItems];
                let _: () = msg_send![s.choice_ui.picker,setEnabled:NO];
                if request.mode(&config.choices) == RevealMode::Off {
                    set_text(
                        s.choice_ui.output,
                        "This type of word choice is disabled in the draft.",
                    );
                    return;
                }
                if !request.valid() {
                    set_text(
                        s.choice_ui.output,
                        "Select a word or short phrase, or provide text before the cursor.",
                    );
                    return;
                }
                s.choice_ui.choices = None;
                let _: () = msg_send![s.choice_ui.picker, removeAllItems];
                let _: () = msg_send![s.choice_ui.picker,setEnabled:NO];
                request.candidates =
                    spelling::complete_words(&request.started_word(), &config.spelling.language);
                match s.choice_requests.try_send((config, request.clone())) {
                    Ok(()) => {
                        s.preview_epoch = Some(id);
                        s.choice_preview_request = Some(request);
                        set_text(s.choice_ui.output, "Generating choices locally…");
                    }
                    Err(_) => set_text(
                        s.choice_ui.output,
                        "Model worker is busy. Try again shortly.",
                    ),
                }
            }
        }
    });
}
extern "C" fn choose(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![s.choice_ui.picker, indexOfSelectedItem];
                if let Some(choices) = s.choice_ui.choices.as_mut() {
                    if index >= 0 && (index as usize) < choices.items.len() {
                        choices.selected = index as usize;
                    }
                }
                s.choice_ui.preview_selected();
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(wordChoices:), show as extern "C" fn(&Object, Sel, id));
    decl.add_method(sel!(saveChoices:), save as extern "C" fn(&Object, Sel, id));
    decl.add_method(
        sel!(previewChoices:),
        preview as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(
        sel!(chooseWordPreview:),
        choose as extern "C" fn(&Object, Sel, id),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "uses macOS dictionary and installed Gemma4 E4B on synthetic text only"]
    fn partial_word_dictionary_to_model_live() {
        let mut config = Config {
            model: "gemma4:e4b".into(),
            ..Config::default()
        };
        config.choices.alternatives = RevealMode::Automatic;
        let request = Request {
            id: 1,
            prefix: "I look forw".into(),
            selected: String::new(),
            suffix: String::new(),
            app_id: "local.libretype.synthetic".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
            candidates: unsafe { spelling::complete_words("forw", "") },
        };
        let epoch = AtomicU64::new(1);
        let choices = model::generate_choices(&config, &request, &epoch).unwrap();
        eprintln!("Synthetic partial-word choices: {choices:?}");
        assert!(choices
            .iter()
            .any(|choice| choice.label == "forward" && choice.insertion == "ard"));
        assert!(choices
            .iter()
            .all(|choice| choice.label.starts_with("forw")));
    }
    #[test]
    fn empty_choices_release_completion_without_repeating_the_automatic_request() {
        let mut request = Request {
            id: 1,
            prefix: "Discuss Zeph".into(),
            selected: String::new(),
            suffix: String::new(),
            app_id: "test".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
            candidates: vec![],
        };
        assert!(request.valid());
        let mut progress = Progress::Untried;
        // No dictionary matches: do not submit or suppress ordinary completion.
        assert!(!progress.begin(&request));
        assert!(!progress.blocks_completion(false));
        assert_ne!(progress, Progress::Untried);

        // A new caret at a word boundary can generate choices without a dictionary.
        request.prefix = "We should ".into();
        assert!(progress.begin(&request));
        assert!(progress.blocks_completion(false));
        for raw in [r#"{"choices":[]}"#, r#"{"choices":["<invalid>"]}"#] {
            let choices = request.finish(raw);
            progress.finish(!choices.is_empty());
            assert!(!progress.blocks_completion(false));
            assert_ne!(progress, Progress::Untried); // no timer retry loop
            assert!(progress.blocks_completion(true)); // never replace a selection with prose
        }

        // Explicit reveal can retry; a visible list still owns acceptance.
        assert!(progress.begin(&request));
        let choices = request.finish(r#"{"choices":["meet","leave"]}"#);
        assert_eq!(choices.len(), 2);
        progress.finish(!choices.is_empty());
        assert!(progress.blocks_completion(false));

        // Synonyms do not require unfinished-word dictionary candidates.
        request.prefix = "This is ".into();
        request.selected = "great".into();
        assert!(progress.begin(&request));
        progress.finish(false); // also used for a model error
        assert!(progress.blocks_completion(true));
    }
    #[test]
    fn numbered_choice_keys_pass_modified_input_and_other_digits() {
        for (key, index) in [(18, 0), (19, 1), (20, 2), (21, 3), (23, 4), (83, 0)] {
            assert_eq!(number_choice(key, 0), Some(index));
            assert_eq!(number_choice(key, SHIFT), None);
            assert_eq!(number_choice(key, OPTION), None);
        }
        assert_eq!(number_choice(22, 0), None);
    }
    #[test]
    fn word_choice_list_fits_below_or_above_on_secondary_screens() {
        let screen = NSRect::new(NSPoint::new(-1440., 0.), NSSize::new(1440., 900.));
        let field = NSRect::new(NSPoint::new(-1400., 100.), NSSize::new(1000., 700.));
        for y in [110., 500., 780.] {
            let caret = NSRect::new(NSPoint::new(-1300., y), NSSize::new(1., 18.));
            let (frame, rows) = list_frame(caret, field, screen, 900., 18., 5).unwrap();
            assert_eq!(rows, 5);
            assert!(frame.origin.y >= 0. && frame.origin.y + frame.size.height <= 900.);
            let bottom = 900. - y - 18.;
            assert!(frame.origin.y + frame.size.height <= bottom || frame.origin.y >= bottom + 18.);
        }
    }
}
