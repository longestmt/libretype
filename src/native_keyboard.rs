// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Native shortcut recorder: events are received only by this window's first responder.
use super::*;
use crate::shortcuts::{Hotkey, Shortcuts, ACTIONS};

pub(super) struct KeyboardUi {
    window: id,
    recorder: id,
    buttons: Vec<id>,
    status: id,
    space: id,
    dismiss_behavior: id,
    dismiss_seconds: id,
    draft: Shortcuts,
    recording: Option<Action>,
}
impl KeyboardUi {
    pub(super) unsafe fn new(target: id, shortcuts: &Shortcuts) -> Self {
        let mut decl = ClassDecl::new("LibreTypistShortcutRecorder", class!(NSView)).unwrap();
        decl.add_method(
            sel!(acceptsFirstResponder),
            accepts_focus as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(sel!(keyDown:), key_down as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(performKeyEquivalent:),
            key_equivalent as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        let recorder: id = msg_send![decl.register(), alloc];
        let recorder: id = msg_send![recorder, initWithFrame:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,683.))];
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,683.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Keyboard shortcuts");
        let _: () = msg_send![window, setReleasedWhenClosed:NO];
        let _: () = msg_send![window, setContentView:recorder];
        let _: () = msg_send![window, center];
        let _: () = msg_send![recorder, release];
        label(
            recorder,
            "Keyboard shortcuts",
            NSRect::new(NSPoint::new(24., 630.), NSSize::new(570., 36.)),
            24.,
        );
        label(recorder,"Click a shortcut, then press the keys you want. Esc cancels recording.\nDelete clears a shortcut. Labels use physical US key positions.",NSRect::new(NSPoint::new(24.,583.),NSSize::new(570.,42.)),12.);
        let mut buttons = Vec::new();
        for (i, action) in ACTIONS.into_iter().enumerate() {
            let y = 535. - i as f64 * 43.;
            label(
                recorder,
                action.label(),
                NSRect::new(NSPoint::new(24., y + 4.), NSSize::new(340., 26.)),
                13.,
            );
            let button: id = msg_send![class!(NSButton), alloc];
            let button: id = msg_send![button, initWithFrame:NSRect::new(NSPoint::new(374.,y),NSSize::new(220.,32.))];
            let _: () = msg_send![button, setBezelStyle:1usize];
            let _: () = msg_send![button, setTarget:target];
            let _: () = msg_send![button, setAction:sel!(recordShortcut:)];
            let _: () = msg_send![button, setTag:i as isize];
            let name = ns(&format!(
                "Record {} shortcut",
                action.label().to_lowercase()
            ));
            let _: () = msg_send![button, setAccessibilityLabel:name];
            let _: () = msg_send![name, release];
            let _: () = msg_send![recorder, addSubview:button];
            let _: () = msg_send![button, release];
            buttons.push(button);
        }
        let space: id = msg_send![class!(NSButton), alloc];
        let space: id = msg_send![space, initWithFrame:NSRect::new(NSPoint::new(24.,237.),NSSize::new(570.,26.))];
        let _: () = msg_send![space, setButtonType:3usize];
        set_title(space, "Include a trailing space after accepting one word");
        let _: () =
            msg_send![space, setState:if shortcuts.trailing_space { 1isize } else { 0isize }];
        let _: () = msg_send![recorder, addSubview:space];
        let _: () = msg_send![space, release];
        label(
            recorder,
            "After dismissing a suggestion",
            NSRect::new(NSPoint::new(24., 194.), NSSize::new(260., 26.)),
            13.,
        );
        let dismiss_behavior: id = msg_send![class!(NSPopUpButton), alloc];
        let dismiss_behavior: id = msg_send![dismiss_behavior, initWithFrame:NSRect::new(NSPoint::new(284.,190.),NSSize::new(310.,30.)) pullsDown:NO];
        for title in [
            "Pause the current field briefly",
            "Pause until switching apps",
            "Pass the dismissal key to the app",
        ] {
            let title = ns(title);
            let _: () = msg_send![dismiss_behavior, addItemWithTitle:title];
            let _: () = msg_send![title, release];
        }
        let name = ns("Dismissal behavior");
        let _: () = msg_send![dismiss_behavior, setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![recorder, addSubview:dismiss_behavior];
        let _: () = msg_send![dismiss_behavior, release];
        label(
            recorder,
            "Brief pause in seconds (1–60)",
            NSRect::new(NSPoint::new(24., 150.), NSSize::new(260., 26.)),
            13.,
        );
        let dismiss_seconds = text_input(
            recorder,
            &shortcuts.dismiss_seconds.to_string(),
            "Brief dismissal pause in seconds",
            NSRect::new(NSPoint::new(290., 150.), NSSize::new(70., 26.)),
        );
        label(recorder,"Option-Tab moves to the next field even when a suggestion is showing.\nManual requests respect app exclusions, secure fields and global pause.",NSRect::new(NSPoint::new(24.,83.),NSSize::new(570.,42.)),12.);
        let status = label(
            recorder,
            "Changes apply after saving.",
            NSRect::new(NSPoint::new(24., 15.), NSSize::new(340., 58.)),
            12.,
        );
        button(
            recorder,
            "Defaults",
            NSRect::new(NSPoint::new(366., 24.), NSSize::new(100., 32.)),
            target,
            sel!(shortcutDefaults:),
        );
        button(
            recorder,
            "Save shortcuts",
            NSRect::new(NSPoint::new(470., 24.), NSSize::new(130., 32.)),
            target,
            sel!(saveShortcuts:),
        );
        let ui = Self {
            window,
            recorder,
            buttons,
            status,
            space,
            dismiss_behavior,
            dismiss_seconds,
            draft: shortcuts.clone(),
            recording: None,
        };
        ui.refresh_behavior();
        ui.refresh();
        ui
    }
    unsafe fn refresh_behavior(&self) {
        let index = match self.draft.dismiss_behavior {
            DismissBehavior::PauseField => 0isize,
            DismissBehavior::UntilAppSwitch => 1,
            DismissBehavior::PassThrough => 2,
        };
        let _: () = msg_send![self.dismiss_behavior, selectItemAtIndex:index];
        let text = ns(&self.draft.dismiss_seconds.to_string());
        let _: () = msg_send![self.dismiss_seconds, setStringValue:text];
        let _: () = msg_send![text, release];
    }
    unsafe fn message(&self, message: &str) {
        let text = ns(message);
        let _: () = msg_send![self.status, setStringValue:text];
        let _: () = msg_send![text, release];
    }
    unsafe fn refresh(&self) {
        for (i, action) in ACTIONS.into_iter().enumerate() {
            let text = if self.recording == Some(action) {
                "Press shortcut…".into()
            } else {
                self.draft
                    .get(action)
                    .map(Hotkey::label)
                    .unwrap_or_else(|| "Not assigned".into())
            };
            set_title(self.buttons[i], &text);
        }
    }
}

extern "C" fn accepts_focus(_: &Object, _: Sel) -> BOOL {
    YES
}
extern "C" fn key_down(_: &Object, _: Sel, event: id) {
    record_event(event);
}
extern "C" fn key_equivalent(_: &Object, _: Sel, event: id) -> BOOL {
    if record_event(event) {
        YES
    } else {
        NO
    }
}
fn record_event(event: id) -> bool {
    let mut handled = false;
    STATE.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut() {
            if let Some(s) = state.as_mut() {
                unsafe {
                    let ui = &mut s.keyboard;
                    let Some(action) = ui.recording else { return };
                    let responder: id = msg_send![ui.window, firstResponder];
                    if responder != ui.recorder {
                        return;
                    }
                    let key: u16 = msg_send![event, keyCode];
                    let flags: u64 = msg_send![event, modifierFlags];
                    let repeat: BOOL = msg_send![event, isARepeat];
                    handled = true;
                    if repeat == YES {
                        return;
                    }
                    if key == 53 && flags & MODIFIERS == 0 {
                        ui.recording = None;
                        ui.message("Recording cancelled. Existing shortcut kept.");
                    } else {
                        let binding = if key == 51 && flags & MODIFIERS == 0 {
                            None
                        } else {
                            Some(Hotkey::new(key, flags))
                        };
                        let mut draft = ui.draft.clone();
                        draft.set(action, binding);
                        match draft.validate() {
                            Ok(()) => {
                                ui.draft = draft;
                                ui.recording = None;
                                ui.message("Shortcut recorded. Save shortcuts to apply it.");
                            }
                            Err(error) => ui.message(&error),
                        }
                    }
                    ui.refresh();
                }
            }
        }
    });
    handled
}
extern "C" fn on_show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.keyboard.recording = None;
                s.keyboard.refresh();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.keyboard.window, makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn on_record(_: &Object, _: Sel, sender: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![sender, tag];
                if let Some(action) = ACTIONS.get(index as usize) {
                    s.keyboard.recording = Some(*action);
                    s.keyboard
                        .message("Press a shortcut. Esc cancels; Delete removes the assignment.");
                    s.keyboard.refresh();
                    let _: BOOL =
                        msg_send![s.keyboard.window, makeFirstResponder:s.keyboard.recorder];
                }
            }
        }
    });
}
extern "C" fn on_defaults(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.keyboard.draft = Shortcuts::default();
                s.keyboard.refresh_behavior();
                s.keyboard.recording = None;
                let _: () = msg_send![s.keyboard.space, setState:0isize];
                s.keyboard.refresh();
                s.keyboard
                    .message("Defaults restored in this window. Save shortcuts to apply.");
            }
        }
    });
}
extern "C" fn on_save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                config.shortcuts = s.keyboard.draft.clone();
                let checked: isize = msg_send![s.keyboard.space, state];
                config.shortcuts.trailing_space = checked == 1;
                let index: isize = msg_send![s.keyboard.dismiss_behavior, indexOfSelectedItem];
                config.shortcuts.dismiss_behavior = match index {
                    1 => DismissBehavior::UntilAppSwitch,
                    2 => DismissBehavior::PassThrough,
                    _ => DismissBehavior::PauseField,
                };
                let Ok(seconds) = ns_text(msg_send![s.keyboard.dismiss_seconds, stringValue])
                    .trim()
                    .parse()
                else {
                    s.keyboard
                        .message("Not saved: use a whole number for the pause in seconds.");
                    return;
                };
                config.shortcuts.dismiss_seconds = seconds;
                match config.save() {
                    Ok(()) => {
                        s.clear();
                        s.dismissal.reset();
                        s.dismissed_field = None;
                        s.config = config;
                        s.keyboard.draft = s.config.shortcuts.clone();
                        s.keyboard.recording = None;
                        s.keyboard.refresh();
                        s.keyboard.message("Saved. Shortcuts apply immediately.");
                    }
                    Err(error) => s.keyboard.message(&format!("Not saved: {error}")),
                }
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(keyboard:), on_show as extern "C" fn(&Object, Sel, id));
    decl.add_method(
        sel!(recordShortcut:),
        on_record as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(
        sel!(shortcutDefaults:),
        on_defaults as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(
        sel!(saveShortcuts:),
        on_save as extern "C" fn(&Object, Sel, id),
    );
}
