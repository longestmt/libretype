// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Local model setup with explicit download and activation actions.
use super::*;
use crate::model_setup::{self, Engine, Phase, CHOICES};

pub(super) struct ModelUi {
    window: id,
    choices: id,
    detail: id,
    active: id,
    status: id,
    download: id,
    cancel: id,
    use_model: id,
    engine: Engine,
    last_ready: u64,
    message: String,
    tags: Vec<String>,
    verification: Option<(u64, String)>,
    sequence: u64,
    verify: mpsc::SyncSender<(u64, String)>,
    replies: mpsc::Receiver<(u64, String, Result<(), model_setup::Error>)>,
}
impl ModelUi {
    pub(super) unsafe fn new(target: id) -> Result<Self, String> {
        let engine = Engine::new().map_err(|e| e.to_string())?;
        let (verify, requests) = mpsc::sync_channel::<(u64, String)>(1);
        let (results, replies) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((id, choice)) = requests.recv() {
                let result = model_setup::verify_named(&choice);
                if results.send((id, choice, result)).is_err() {
                    break;
                }
            }
        });
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:rect(0.,0.,620.,640.) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Local models");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(view, "Local models", rect(24., 585., 570., 36.), 26.);
        let info: id = msg_send![class!(NSProcessInfo), processInfo];
        let memory: u64 = msg_send![info, physicalMemory];
        let suggested = model_setup::recommended((memory > 0).then_some(memory));
        label(view,&format!("This Mac: {:.0} GB memory. Starting recommendation: {}.\nBased on small M1 tests; available memory and writing quality vary.",memory as f64/1073741824.,CHOICES[suggested].title),rect(24.,525.,570.,50.),13.);
        let choices: id = msg_send![class!(NSPopUpButton), alloc];
        let choices: id = msg_send![choices,initWithFrame:rect(20.,480.,575.,32.) pullsDown:NO];
        for choice in &CHOICES {
            let title = ns(&format!("{} — {}", choice.title, choice.tag));
            let _: () = msg_send![choices,addItemWithTitle:title];
            let _: () = msg_send![title, release];
        }
        let name = ns("Model to download or use");
        let _: () = msg_send![choices,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![choices,selectItemAtIndex:suggested as isize];
        let _: () = msg_send![choices,setTarget:target];
        let _: () = msg_send![choices,setAction:sel!(modelSetupChoice:)];
        let _: () = msg_send![view,addSubview:choices];
        let _: () = msg_send![choices, release];
        let detail = label(view, "", rect(24., 315., 570., 158.), 13.);
        let active = label(view, "", rect(24., 280., 345., 28.), 13.);
        button(
            view,
            "Model terms…",
            rect(380., 280., 215., 28.),
            target,
            sel!(modelTerms:),
        );
        let make_button = |text: &str, x: f64, w: f64, action: Sel| -> id {
            let item: id = msg_send![class!(NSButton), alloc];
            let item: id = msg_send![item,initWithFrame:rect(x,235.,w,32.)];
            set_title(item, text);
            let _: () = msg_send![item,setBezelStyle:1usize];
            let _: () = msg_send![item,setTarget:target];
            let _: () = msg_send![item,setAction:action];
            let _: () = msg_send![view,addSubview:item];
            let _: () = msg_send![item, release];
            item
        };
        let download = make_button("Download / update", 20., 190., sel!(downloadModel:));
        let cancel = make_button("Cancel", 215., 110., sel!(cancelModelDownload:));
        let use_model = make_button("Use selected model", 330., 265., sel!(useSetupModel:));
        let status = label(view, "", rect(24., 140., 570., 88.), 13.);
        label(view,"Downloads need internet; suggestions run on this Mac. Download size is not a memory estimate; leave room for other apps. Updates may replace a model version. Partial downloads can resume.",rect(24.,75.,570.,60.),12.);
        button(
            view,
            "Open Ollama",
            rect(20., 20., 170., 32.),
            target,
            sel!(openOllama:),
        );
        button(
            view,
            "Get Ollama…",
            rect(200., 20., 170., 32.),
            target,
            sel!(getOllama:),
        );
        button(
            view,
            "Refresh installed",
            rect(380., 20., 215., 32.),
            target,
            sel!(refreshModels:),
        );
        Ok(Self {
            window,
            choices,
            detail,
            active,
            status,
            download,
            cancel,
            use_model,
            engine,
            last_ready: 0,
            message: String::new(),
            tags: CHOICES.iter().map(|choice| choice.tag.to_owned()).collect(),
            verification: None,
            sequence: 0,
            verify,
            replies,
        })
    }
    unsafe fn selected(&self) -> Option<usize> {
        let index: isize = msg_send![self.choices, indexOfSelectedItem];
        usize::try_from(index).ok().filter(|i| *i < self.tags.len())
    }
    pub(super) unsafe fn poll(
        &mut self,
        config: &Config,
        installed: &[String],
        availability: Option<&str>,
    ) -> (Option<String>, bool) {
        let mut selected = None;
        while let Ok((id, choice, result)) = self.replies.try_recv() {
            if self.verification.as_ref() == Some(&(id, choice.clone())) {
                self.verification = None;
                if self.selected().is_some_and(|i| self.tags[i] == choice) {
                    match result {
                        Ok(()) => selected = Some(choice),
                        Err(e) => self.message = e.to_string(),
                    }
                }
            }
        }
        let snapshot = self.engine.snapshot();
        let ready = snapshot.phase == Phase::Ready && snapshot.job != self.last_ready;
        if ready {
            self.last_ready = snapshot.job;
        }
        if !snapshot.phase.busy() && self.verification.is_none() {
            let mut tags: Vec<String> =
                CHOICES.iter().map(|choice| choice.tag.to_owned()).collect();
            for tag in installed {
                if !tags.contains(tag) {
                    tags.push(tag.clone());
                }
            }
            if tags != self.tags {
                let previous = self.selected().map(|i| self.tags[i].clone());
                let _: () = msg_send![self.choices, removeAllItems];
                for (i, tag) in tags.iter().enumerate() {
                    let title = ns(&if i < CHOICES.len() {
                        format!("{} — {}", CHOICES[i].title, tag)
                    } else {
                        format!("{tag} — installed")
                    });
                    let _: () = msg_send![self.choices,addItemWithTitle:title];
                    let _: () = msg_send![title, release];
                }
                let index = previous
                    .and_then(|tag| tags.iter().position(|t| *t == tag))
                    .unwrap_or(0);
                let _: () = msg_send![self.choices,selectItemAtIndex:index as isize];
                self.tags = tags;
            }
        }
        let choice = self.selected().unwrap_or(0);
        let present = installed.iter().any(|tag| tag == &self.tags[choice]);
        set_text(
            self.detail,
            &format!(
                "{}\n{}\n{}",
                CHOICES.get(choice).map(|c|c.description).unwrap_or("Installed through Ollama. Memory needs and quality depend on the model. Use Ollama to download or update models outside this catalog."),
                CHOICES.get(choice).map(|c| c.license_label()).unwrap_or("Separately installed model: check its publisher’s license."),
                availability.unwrap_or(if present {
                    "Installed locally."
                } else {
                    "Not in the installed model list; download or refresh."
                })
            ),
        );
        set_text(self.active, &format!("Active model: {}", config.model));
        let text = if self.verification.is_some() {
            "Verifying selected model locally…".into()
        } else if snapshot.phase.busy() || self.message.is_empty() {
            format!(
                "{}{}",
                if snapshot.job > 0 {
                    format!("{}: ", CHOICES[snapshot.choice].title)
                } else {
                    String::new()
                },
                snapshot.phase.message()
            )
        } else {
            self.message.clone()
        };
        set_text(self.status, &text);
        let busy = snapshot.phase.busy() || self.verification.is_some();
        let _: () =
            msg_send![self.download,setEnabled:if busy || choice>=CHOICES.len(){NO}else{YES}];
        let _: () = msg_send![self.cancel,setEnabled:if snapshot.phase.busy() && snapshot.phase!=Phase::Cancelling{YES}else{NO}];
        let _: () = msg_send![self.use_model,setEnabled:if !busy && availability.is_none() && (present || (snapshot.phase==Phase::Ready && snapshot.choice==choice)){YES}else{NO}];
        let _: () = msg_send![self.choices,setEnabled:if busy{NO}else{YES}];
        (selected, ready)
    }
    pub(super) fn saved(&mut self, message: String) {
        self.message = message;
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn set_text(control: id, text: &str) {
    let text = ns(text);
    let _: () = msg_send![control,setStringValue:text];
    let _: () = msg_send![text, release];
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.model_setup.message.clear();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.model_setup.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn choice(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            s.model_setup.message.clear();
        }
    });
}
extern "C" fn download(_: &Object, _: Sel, _: id) {
    // NSAlert runs a nested AppKit loop: never hold the State RefCell borrow.
    let selected = STATE.with(|state| {
        state
            .borrow()
            .as_ref()
            .and_then(|s| unsafe { s.model_setup.selected() })
    });
    let Some(selected) = selected else {
        return;
    };
    if CHOICES.get(selected).is_some_and(|c| c.restricted_terms()) {
        unsafe {
            let alert: id = msg_send![class!(NSAlert), new];
            let title = ns("Optional model with separate terms");
            let detail = ns("Gemma 3 uses Google's Gemma Terms of Use, including use restrictions. These apply to the model, not LibreType's GPL application code. Review them using Model terms before choosing Download. Qwen3 is available under Apache-2.0.");
            let _: () = msg_send![alert,setMessageText:title];
            let _: () = msg_send![alert,setInformativeText:detail];
            for text in ["Cancel", "Download"] {
                let name = ns(text);
                let _: id = msg_send![alert,addButtonWithTitle:name];
                let _: () = msg_send![name, release];
            }
            let response: isize = msg_send![alert, runModal];
            let _: () = msg_send![title, release];
            let _: () = msg_send![detail, release];
            let _: () = msg_send![alert, release];
            if response != 1001 {
                return;
            }
        }
    }
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            if unsafe { s.model_setup.selected() } != Some(selected) {
                return;
            }
            s.model_setup.message.clear();
            if let Err(e) = s.model_setup.engine.start(selected) {
                s.model_setup.message = e.to_string();
            }
        }
    });
}
extern "C" fn cancel(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            s.model_setup.message.clear();
            s.model_setup.engine.cancel();
        }
    });
}
extern "C" fn use_model(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                if s.model_setup.verification.is_some()
                    || s.model_setup.engine.snapshot().phase.busy()
                {
                    return;
                }
                if let Some(choice) = s.model_setup.selected() {
                    s.model_setup.sequence += 1;
                    let id = s.model_setup.sequence;
                    let tag = s.model_setup.tags[choice].clone();
                    match s.model_setup.verify.try_send((id, tag.clone())) {
                        Ok(()) => s.model_setup.verification = Some((id, tag)),
                        Err(_) => {
                            s.model_setup.message =
                                "Model checker is busy; try again shortly.".into()
                        }
                    }
                }
            }
        }
    });
}
extern "C" fn open_ollama(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
                let name = ns("Ollama");
                let opened: BOOL = msg_send![workspace,launchApplication:name];
                let _: () = msg_send![name, release];
                s.model_setup.message = if opened == YES {
                    s.model_catalog.refresh();
                    "Ollama opened. Checking installed models automatically…".into()
                } else {
                    "Ollama was not found. Use Get Ollama to install the Mac app, then open it."
                        .into()
                };
            }
        }
    });
}
extern "C" fn get_ollama(_: &Object, _: Sel, _: id) {
    unsafe {
        let text = ns("https://ollama.com/download/mac");
        let url: id = msg_send![class!(NSURL),URLWithString:text];
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let _: BOOL = msg_send![workspace,openURL:url];
        let _: () = msg_send![text, release];
    }
}
extern "C" fn model_terms(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow().as_ref() {
            unsafe {
                if let Some(choice) = s.model_setup.selected().and_then(|i| CHOICES.get(i)) {
                    let text = ns(choice.license_url());
                    let url: id = msg_send![class!(NSURL),URLWithString:text];
                    let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
                    let _: BOOL = msg_send![workspace,openURL:url];
                    let _: () = msg_send![text, release];
                }
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    for (selector, action) in [
        (sel!(modelSetup:), show as extern "C" fn(&Object, Sel, id)),
        (sel!(modelSetupChoice:), choice),
        (sel!(downloadModel:), download),
        (sel!(modelTerms:), model_terms),
        (sel!(cancelModelDownload:), cancel),
        (sel!(useSetupModel:), use_model),
        (sel!(openOllama:), open_ollama),
        (sel!(getOllama:), get_ollama),
    ] {
        decl.add_method(selector, action);
    }
}
