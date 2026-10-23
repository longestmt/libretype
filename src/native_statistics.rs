// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Read-only daily totals. The recorder receives only a date and verified count.
use super::*;
use crate::statistics::{local_day, Recorder};

pub(super) struct Runtime {
    recorder: Recorder,
    window: id,
    today: id,
    list: id,
    status: id,
    menu_item: id,
    rendered: String,
}
impl Runtime {
    pub(super) unsafe fn new(target: id, menu: id) -> Result<Self, String> {
        let path = Config::path()?
            .parent()
            .ok_or("Missing settings directory")?
            .join("statistics")
            .join("daily.json");
        let recorder = Recorder::new(path).map_err(|e| e.to_string())?;
        let menu_item = menu_item(menu, "Words accepted today: …", sel!(statistics:), target);
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(540.,560.)) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Daily statistics");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Words accepted today",
            rect(24., 503., 490., 36.),
            24.,
        );
        let today = label(view, "…", rect(24., 440., 490., 60.), 40.);
        label(
            view,
            "Daily totals · most recent 365 active dates",
            rect(24., 400., 490., 28.),
            13.,
        );
        let scroll: id = msg_send![class!(NSScrollView), alloc];
        let scroll: id = msg_send![scroll,initWithFrame:rect(24.,173.,490.,220.)];
        let _: () = msg_send![scroll,setHasVerticalScroller:YES];
        let _: () = msg_send![scroll,setBorderType:1usize];
        let list: id = msg_send![class!(NSTextView), alloc];
        let list: id = msg_send![list,initWithFrame:rect(0.,0.,470.,220.)];
        let _: () = msg_send![list,setEditable:NO];
        let _: () = msg_send![list,setSelectable:YES];
        let _: () = msg_send![list,setRichText:NO];
        let font: id = msg_send![class!(NSFont),monospacedSystemFontOfSize:13f64 weight:0f64];
        let _: () = msg_send![list,setFont:font];
        let _: () = msg_send![list,setVerticallyResizable:YES];
        let _: () = msg_send![list,setHorizontallyResizable:NO];
        let _: () = msg_send![list,setAutoresizingMask:2usize];
        let container: id = msg_send![list, textContainer];
        let _: () = msg_send![container,setWidthTracksTextView:YES];
        let name = ns("Daily accepted-word totals");
        let _: () = msg_send![list,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![scroll,setDocumentView:list];
        let _: () = msg_send![list, release];
        let _: () = msg_send![view,addSubview:scroll];
        let _: () = msg_send![scroll, release];
        label(view,"Counts confirmed insertions, including completed words, corrections and synonyms. A word is a space-separated token with letters or digits. Emoji alone do not count.\nDates use your Mac’s time zone. Only dates and totals are saved locally; no writing or app names.",rect(24.,63.,490.,100.),12.);
        let status = label(view, "Loading totals…", rect(24., 12., 490., 46.), 12.);
        Ok(Self {
            recorder,
            window,
            today,
            list,
            status,
            menu_item,
            rendered: String::new(),
        })
    }
    pub(super) fn record(&mut self, words: u64) {
        if let Some(day) = local_day() {
            self.recorder.record(day, words);
        }
    }
    pub(super) fn finish(&mut self) {
        self.recorder.finish(Duration::from_millis(300));
    }
    pub(super) unsafe fn poll(&mut self) {
        self.recorder.poll();
        let snapshot = self.recorder.snapshot();
        let day = local_day();
        let count = day.as_deref().map(|d| snapshot.totals.count(d));
        let status = if let Some(error) = snapshot.error {
            error.to_string()
        } else if day.is_none() {
            "Local date unavailable; statistics are not being recorded.".into()
        } else if !snapshot.loaded {
            "Loading totals…".into()
        } else {
            "Totals are saved after the editor confirms insertion.".into()
        };
        let title = if snapshot.error.is_some() {
            "Statistics unavailable…".into()
        } else if snapshot.loaded {
            format!("Words accepted today: {}", count.unwrap_or(0))
        } else {
            "Words accepted today: …".into()
        };
        let mut lines = String::new();
        for (date, words) in snapshot.totals.days.iter().rev() {
            lines.push_str(&format!("{date}    {words} words\n"));
        }
        if lines.is_empty() {
            lines = "No confirmed words recorded yet.".into();
        }
        let fingerprint = format!("{title}\n{status}\n{lines}");
        if self.rendered == fingerprint {
            return;
        }
        self.rendered = fingerprint;
        set_title(self.menu_item, &title);
        set_value(
            self.today,
            &if snapshot.loaded {
                count.unwrap_or(0).to_string()
            } else {
                "…".into()
            },
        );
        set_value(self.status, &status);
        let text = ns(&lines);
        let _: () = msg_send![self.list,setString:text];
        let _: () = msg_send![text, release];
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn set_value(control: id, text: &str) {
    let text = ns(text);
    let _: () = msg_send![control,setStringValue:text];
    let _: () = msg_send![text, release];
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                s.statistics.poll();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.statistics.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    decl.add_method(sel!(statistics:), show as extern "C" fn(&Object, Sel, id));
}
