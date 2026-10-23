// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Native editor for exact-domain preferences. No website history is collected.
use super::*;
use crate::{config::WebsiteRule, website::canonical_host};

pub(super) struct WebsiteUi {
    window: id,
    saved: id,
    domain: id,
    enabled: id,
    word: id,
    instructions: id,
    append_instructions: id,
    preview: id,
    pub(super) output: id,
    status: id,
    history_count: id,
    domains: Vec<String>,
}
unsafe fn checkbox(view: id, title: &str, y: f64) -> id {
    let b: id = msg_send![class!(NSButton), alloc];
    let b: id = msg_send![b, initWithFrame:NSRect::new(NSPoint::new(24.,y),NSSize::new(552.,26.))];
    let _: () = msg_send![b, setButtonType:3usize];
    set_title(b, title);
    let _: () = msg_send![b, setState:1isize];
    let _: () = msg_send![view, addSubview:b];
    let _: () = msg_send![b, release];
    b
}
impl WebsiteUi {
    pub(super) unsafe fn new(target: id) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window, initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(620.,680.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Website preferences");
        let _: () = msg_send![window, setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Website preferences",
            NSRect::new(NSPoint::new(24., 626.), NSSize::new(570., 36.)),
            24.,
        );
        label(view,"Exact domains only: mail.google.com does not include other Google sites.\nApp exclusions still apply. Browser Accessibility must expose the active site.",NSRect::new(NSPoint::new(24.,576.),NSSize::new(575.,44.)),12.);
        let saved: id = msg_send![class!(NSPopUpButton), alloc];
        let saved: id = msg_send![saved, initWithFrame:NSRect::new(NSPoint::new(22.,535.),NSSize::new(440.,30.)) pullsDown:NO];
        let name = ns("Saved website rules");
        let _: () = msg_send![saved, setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![view, addSubview:saved];
        let _: () = msg_send![saved, release];
        button(
            view,
            "Load rule",
            NSRect::new(NSPoint::new(470., 533.), NSSize::new(125., 32.)),
            target,
            sel!(loadWebsite:),
        );
        let domain = text_input(
            view,
            "",
            "Website domain",
            NSRect::new(NSPoint::new(24., 493.), NSSize::new(570., 28.)),
        );
        let placeholder = ns("Domain, for example mail.google.com");
        let _: () = msg_send![domain, setPlaceholderString:placeholder];
        let _: () = msg_send![placeholder, release];
        let history_count = label(
            view,
            "Enter an exact website to see its history count.",
            NSRect::new(NSPoint::new(24., 462.), NSSize::new(570., 24.)),
            12.,
        );
        let enabled = checkbox(view, "Allow suggestions on this website", 424.);
        let word = checkbox(
            view,
            "Allow word acceptance here (also requires the app shortcut)",
            392.,
        );
        label(view,"Writing instructions (up to 1,000 characters)\nBlank inherits the app; unchecked replaces inherited instructions.",NSRect::new(NSPoint::new(24.,343.),NSSize::new(570.,44.)),12.);
        let append_instructions = checkbox(
            view,
            "Append these instructions to the app/global instructions",
            314.,
        );
        let _: () = msg_send![append_instructions, setState:0isize];
        let scroll: id = msg_send![class!(NSScrollView), alloc];
        let scroll: id = msg_send![scroll, initWithFrame:NSRect::new(NSPoint::new(24.,224.),NSSize::new(570.,88.))];
        let _: () = msg_send![scroll, setHasVerticalScroller:YES];
        let _: () = msg_send![scroll, setBorderType:1usize];
        let instructions: id = msg_send![class!(NSTextView), alloc];
        let instructions: id = msg_send![instructions, initWithFrame:NSRect::new(NSPoint::new(0.,0.),NSSize::new(550.,108.))];
        let _: () = msg_send![instructions, setRichText:NO];
        let font: id = msg_send![class!(NSFont), systemFontOfSize:13f64];
        let _: () = msg_send![instructions, setFont:font];
        let _: () = msg_send![instructions, setVerticallyResizable:YES];
        let _: () = msg_send![instructions, setHorizontallyResizable:NO];
        let _: () = msg_send![instructions, setAutoresizingMask:2usize];
        let container: id = msg_send![instructions, textContainer];
        let _: () = msg_send![container, setWidthTracksTextView:YES];
        let name = ns("Website writing instructions");
        let _: () = msg_send![instructions, setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![scroll, setDocumentView:instructions];
        let _: () = msg_send![instructions, release];
        let _: () = msg_send![view, addSubview:scroll];
        let _: () = msg_send![scroll, release];
        label(
            view,
            "Preview with saved global instructions (browser app rules are not used here)",
            NSRect::new(NSPoint::new(24., 194.), NSSize::new(570., 22.)),
            12.,
        );
        let preview = text_input(
            view,
            "Thank you for your ",
            "Website preview text",
            NSRect::new(NSPoint::new(24., 158.), NSSize::new(425., 28.)),
        );
        button(
            view,
            "Complete",
            NSRect::new(NSPoint::new(470., 155.), NSSize::new(125., 32.)),
            target,
            sel!(previewWebsite:),
        );
        let output = label(
            view,
            "Preview stays on this Mac.",
            NSRect::new(NSPoint::new(24., 105.), NSSize::new(570., 45.)),
            13.,
        );
        let status = label(
            view,
            "Load a saved rule or enter a domain. Save applies this rule.",
            NSRect::new(NSPoint::new(24., 59.), NSSize::new(570., 42.)),
            12.,
        );
        button(
            view,
            "Remove rule",
            NSRect::new(NSPoint::new(24., 19.), NSSize::new(130., 32.)),
            target,
            sel!(removeWebsite:),
        );
        button(
            view,
            "Save rule",
            NSRect::new(NSPoint::new(470., 19.), NSSize::new(125., 32.)),
            target,
            sel!(saveWebsite:),
        );
        Self {
            window,
            saved,
            domain,
            enabled,
            word,
            instructions,
            append_instructions,
            preview,
            output,
            status,
            history_count,
            domains: vec![],
        }
    }
    pub(super) unsafe fn refresh_history_count(&self, history: &history::Runtime) {
        let domain = ns_text(msg_send![self.domain, stringValue]);
        if let Ok(host) = canonical_host(&domain) {
            history.show_count(self.history_count, &crate::history::Selector::Website(host));
        } else {
            let text = ns("Enter an exact website to see its history count.");
            let _: () = msg_send![self.history_count, setStringValue:text];
            let _: () = msg_send![text, release];
        }
    }
    unsafe fn message(&self, value: &str) {
        let text = ns(value);
        let _: () = msg_send![self.status,setStringValue:text];
        let _: () = msg_send![text, release];
    }
    unsafe fn refresh(&mut self, config: &Config) {
        self.domains = config.website_rules.keys().cloned().collect();
        self.domains.sort();
        let _: () = msg_send![self.saved, removeAllItems];
        let title = ns("Choose a saved website");
        let _: () = msg_send![self.saved,addItemWithTitle:title];
        let _: () = msg_send![title, release];
        for domain in &self.domains {
            let title = ns(domain);
            let _: () = msg_send![self.saved,addItemWithTitle:title];
            let _: () = msg_send![title, release];
        }
    }
    unsafe fn draft(&self) -> Result<(String, WebsiteRule), String> {
        let host = canonical_host(&ns_text(msg_send![self.domain, stringValue]))?;
        let enabled: isize = msg_send![self.enabled, state];
        let word: isize = msg_send![self.word, state];
        let append: isize = msg_send![self.append_instructions, state];
        Ok((
            host,
            WebsiteRule {
                enabled: enabled == 1,
                append_instructions: append == 1,
                word_shortcut: word == 1,
                writing_instructions: ns_text(msg_send![self.instructions, string]),
            },
        ))
    }
    unsafe fn load(&self, host: &str, rule: &WebsiteRule) {
        self.output_message("Preview uses the displayed rule. Click Complete to try it.");
        let text = ns(host);
        let _: () = msg_send![self.domain,setStringValue:text];
        let _: () = msg_send![text, release];
        let text = ns(&rule.writing_instructions);
        let _: () = msg_send![self.instructions,setString:text];
        let _: () = msg_send![text, release];
        let _: () = msg_send![self.enabled,setState:if rule.enabled {1isize}else{0isize}];
        let _: () = msg_send![self.word,setState:if rule.word_shortcut {1isize}else{0isize}];
        let _: () = msg_send![self.append_instructions,setState:if rule.append_instructions {1isize}else{0isize}];
    }
    unsafe fn output_message(&self, value: &str) {
        let text = ns(value);
        let _: () = msg_send![self.output,setStringValue:text];
        let _: () = msg_send![text, release];
    }
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.websites.refresh(&s.config);
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.websites.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn load(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![s.websites.saved, indexOfSelectedItem];
                if let Some(host) = index
                    .checked_sub(1)
                    .and_then(|i| s.websites.domains.get(i as usize))
                    .cloned()
                {
                    if let Some(rule) = s.config.website_rules.get(&host).cloned() {
                        s.clear();
                        s.websites.load(&host, &rule);
                        s.websites
                            .message("Loaded saved rule. Changes apply after saving.");
                    }
                }
            }
        }
    });
}
extern "C" fn save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let result = (|| {
                    let (host, rule) = s.websites.draft()?;
                    let mut config = s.config.clone();
                    config.website_rules.insert(host.clone(), rule.clone());
                    config.save()?;
                    Ok::<_, String>((config, host, rule))
                })();
                match result {
                    Ok((config, host, rule)) => {
                        s.clear();
                        s.config = config;
                        s.websites.refresh(&s.config);
                        s.websites.load(&host, &rule);
                        s.websites
                            .message("Saved. Exact-domain preferences apply to new suggestions.");
                    }
                    Err(e) => s.websites.message(&format!("Not saved: {e}")),
                }
            }
        }
    });
}
extern "C" fn remove(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let result = (|| {
                    let host = canonical_host(&ns_text(msg_send![s.websites.domain, stringValue]))?;
                    let mut config = s.config.clone();
                    if config.website_rules.remove(&host).is_none() {
                        return Err("No saved rule for that domain.".into());
                    }
                    config.save()?;
                    Ok::<_, String>(config)
                })();
                match result {
                    Ok(config) => {
                        s.clear();
                        s.config = config;
                        s.websites.refresh(&s.config);
                        s.websites.load("", &WebsiteRule::default());
                        s.websites
                            .message("Removed. This website now inherits its app preferences.");
                    }
                    Err(e) => s.websites.message(&e),
                }
            }
        }
    });
}
extern "C" fn preview(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.websites
                    .output_message("Preview unavailable. Check the settings below.");
                let (host, rule) = match s.websites.draft() {
                    Ok(d) => d,
                    Err(e) => {
                        s.websites.message(&e);
                        return;
                    }
                };
                if !rule.enabled {
                    s.websites
                        .message("Suggestions are disabled in this draft. Enable them to preview.");
                    return;
                }
                let mut config = s.config.clone();
                config.enabled = true;
                config.set_app_enabled("local.libretypist.website-preview", true);
                config.website_rules.insert(host.clone(), rule);
                if let Err(e) = config.validate() {
                    s.websites.message(&e);
                    return;
                }
                s.websites
                    .message("Preview uses this draft. Save rule to apply it in websites.");
                s.website_preview = true;
                let prefix = ns_text(msg_send![s.websites.preview, stringValue]);
                let id = s.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                s.preview_epoch = Some(id);
                let text = ns("Generating locally…");
                let _: () = msg_send![s.websites.output,setStringValue:text];
                let _: () = msg_send![text, release];
                if s.requests
                    .try_send((
                        config,
                        CompletionRequest {
                            id,
                            prefix,
                            suffix: String::new(),
                            app_id: "local.libretypist.website-preview".into(),
                            website: Some(host),
                            profile: None,
                            background: crate::background::Background::default(),
                        },
                    ))
                    .is_err()
                {
                    s.preview_epoch = None;
                    s.websites.output_message("Preview could not start yet.");
                    s.websites
                        .message("The previous request is stopping. Try Complete again.");
                }
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(websites:), show as extern "C" fn(&Object, Sel, id));
    decl.add_method(sel!(loadWebsite:), load as extern "C" fn(&Object, Sel, id));
    decl.add_method(sel!(saveWebsite:), save as extern "C" fn(&Object, Sel, id));
    decl.add_method(
        sel!(removeWebsite:),
        remove as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(
        sel!(previewWebsite:),
        preview as extern "C" fn(&Object, Sel, id),
    );
}
