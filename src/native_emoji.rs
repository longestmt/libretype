// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Native preferences and local preview for the bundled emoji catalog.
use super::*;
use crate::emoji::{Choices, Query};

pub(super) struct EmojiUi {
    window: id,
    enabled: id,
    before: id,
    after: id,
    picker: id,
    output: id,
    status: id,
    choices: Option<Choices>,
    preview_context: (String, String),
}
impl EmojiUi {
    pub(super) unsafe fn new(target: id, enabled: bool) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,440.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Emoji");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Emoji",
            NSRect::new(NSPoint::new(24., 384.), NSSize::new(570., 36.)),
            24.,
        );
        label(view,"Type : and a name to filter emoji. Use ↑/↓ to choose, then accept.\nYour word or full shortcut replaces the shortcode. Esc dismisses the preview.",NSRect::new(NSPoint::new(24.,330.),NSSize::new(570.,46.)),12.);
        let checkbox: id = msg_send![class!(NSButton), alloc];
        let checkbox: id = msg_send![checkbox,initWithFrame:NSRect::new(NSPoint::new(24.,292.),NSSize::new(570.,28.))];
        let _: () = msg_send![checkbox,setButtonType:3usize];
        set_title(checkbox, "Suggest emoji from shortcodes");
        let _: () = msg_send![checkbox,setState:if enabled {1isize} else {0isize}];
        let _: () = msg_send![view,addSubview:checkbox];
        let _: () = msg_send![checkbox, release];
        label(
            view,
            "Preview: text before cursor | text after cursor",
            NSRect::new(NSPoint::new(24., 256.), NSSize::new(570., 26.)),
            12.,
        );
        let before = text_input(
            view,
            "Thanks :smi",
            "Emoji preview before cursor",
            NSRect::new(NSPoint::new(24., 220.), NSSize::new(280., 28.)),
        );
        let after = text_input(
            view,
            " for your help.",
            "Emoji preview after cursor",
            NSRect::new(NSPoint::new(314., 220.), NSSize::new(150., 28.)),
        );
        button(
            view,
            "Find",
            NSRect::new(NSPoint::new(480., 217.), NSSize::new(114., 32.)),
            target,
            sel!(findEmoji:),
        );
        let picker: id = msg_send![class!(NSPopUpButton), alloc];
        let picker: id = msg_send![picker,initWithFrame:NSRect::new(NSPoint::new(22.,170.),NSSize::new(572.,32.)) pullsDown:NO];
        let name = ns("Matching emoji");
        let _: () = msg_send![picker,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![picker,setTarget:target];
        let _: () = msg_send![picker,setAction:sel!(chooseEmojiPreview:)];
        let _: () = msg_send![picker,setEnabled:NO];
        let _: () = msg_send![view,addSubview:picker];
        let _: () = msg_send![picker, release];
        let output = label(
            view,
            "Find previews the bundled catalog without changing any document.",
            NSRect::new(NSPoint::new(24., 76.), NSSize::new(570., 82.)),
            13.,
        );
        let status = label(
            view,
            "Emoji lookup stays on this Mac. No model is needed.",
            NSRect::new(NSPoint::new(24., 18.), NSSize::new(440., 48.)),
            12.,
        );
        button(
            view,
            "Save",
            NSRect::new(NSPoint::new(480., 22.), NSSize::new(114., 32.)),
            target,
            sel!(saveEmoji:),
        );
        Self {
            window,
            enabled: checkbox,
            before,
            after,
            picker,
            output,
            status,
            choices: None,
            preview_context: (String::new(), String::new()),
        }
    }
    unsafe fn message(&self, value: &str) {
        let text = ns(value);
        let _: () = msg_send![self.status,setStringValue:text];
        let _: () = msg_send![text, release];
    }
    unsafe fn find(&mut self) {
        let before = ns_text(msg_send![self.before, stringValue]);
        let after = ns_text(msg_send![self.after, stringValue]);
        let enabled: isize = msg_send![self.enabled, state];
        self.choices = if enabled == 1 {
            Query::parse(&before, &after).map(Choices::new)
        } else {
            None
        };
        self.preview_context = (before, after);
        let _: () = msg_send![self.picker, removeAllItems];
        if let Some(choices) = &self.choices {
            for choice in &choices.matches {
                let title = ns(&format!("{} :{}:", choice.emoji, choice.alias));
                let _: () = msg_send![self.picker,addItemWithTitle:title];
                let _: () = msg_send![title, release];
            }
        }
        let available = self.choices.as_ref().is_some_and(|c| !c.matches.is_empty());
        let _: () = msg_send![self.picker,setEnabled:if available { YES } else { NO }];
        self.preview();
    }
    unsafe fn preview(&self) {
        let enabled: isize = msg_send![self.enabled, state];
        let before = ns_text(msg_send![self.before, stringValue]);
        let after = ns_text(msg_send![self.after, stringValue]);
        let result = if enabled != 1 {
            "Emoji suggestions are disabled in this draft.".into()
        } else if self.preview_context != (before, after) {
            "Text changed. Click Find to refresh the choices.".into()
        } else if let Some(choices) = &self.choices {
            choices
                .plan(&self.preview_context.0, &self.preview_context.1)
                .map(|plan| format!("{}\nResult: {}", choices.display(), plan.preview()))
                .unwrap_or_else(|| "No matching emoji. Try a shorter name.".into())
        } else {
            "Start a shortcode after a space, such as :smile or :thumbsup:.".into()
        };
        let text = ns(&result);
        let _: () = msg_send![self.output,setStringValue:text];
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
                let _: () = msg_send![s.emoji.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn find(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.emoji.find();
            }
        }
    });
}
extern "C" fn choose(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![s.emoji.picker, indexOfSelectedItem];
                if let Some(choices) = s.emoji.choices.as_mut() {
                    if index >= 0 && (index as usize) < choices.matches.len() {
                        choices.selected = index as usize;
                    }
                }
                s.emoji.preview();
            }
        }
    });
}
extern "C" fn save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                let enabled: isize = msg_send![s.emoji.enabled, state];
                config.emoji = enabled == 1;
                match config.save() {
                    Ok(()) => {
                        s.clear();
                        s.config = config;
                        s.emoji
                            .message("Saved. Emoji preferences apply to new suggestions.");
                    }
                    Err(e) => s.emoji.message(&format!("Not saved: {e}")),
                }
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(emoji:), show as extern "C" fn(&Object, Sel, id));
    decl.add_method(sel!(findEmoji:), find as extern "C" fn(&Object, Sel, id));
    decl.add_method(
        sel!(chooseEmojiPreview:),
        choose as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(sel!(saveEmoji:), save as extern "C" fn(&Object, Sel, id));
}
