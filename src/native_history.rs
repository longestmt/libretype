// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Opt-in editor observation and native controls. Disk, Keychain and inference
//! stay on the history workers; the event tap only marks possible user edits.
use super::*;
use crate::history::{
    self,
    tracker::{Observation as WritingObservation, Tracker},
    worker::{Engine, Snapshot, Status},
    Profile, Record, Scope, Selector, Settings, Strength,
};

fn seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn stamp(config: &Config) -> Vec<u8> {
    serde_json::to_vec(&(
        history::profile::source_policy(config),
        config.enabled,
        config.history.recording,
        config.history.record_without_acceptance,
        config.history.strength,
        config.history.auto_refresh,
        &config.model,
    ))
    .unwrap_or_default()
}
fn edit_key(key: i64, flags: u64) -> bool {
    if flags & crate::shortcuts::CONTROL != 0 {
        return false;
    }
    if flags & crate::shortcuts::COMMAND != 0 {
        return matches!(key, 6 | 7 | 9 | 51 | 117);
    }
    matches!(key, 0..=51 | 65 | 67 | 69 | 75 | 76 | 78 | 81..=92 | 95 | 117)
}
// AXValue and AXSelectedTextRange are separate IPC reads. While typing,
// either may reflect the next edit before the other catches up. Let a short
// typing burst settle before consuming its ownership marker; otherwise a torn
// snapshot splits a human span and the following update appears passive.
const EDIT_SETTLE: Duration = Duration::from_millis(200);
const EDIT_WINDOW: Duration = Duration::from_millis(1000);
#[derive(Debug, PartialEq, Eq)]
enum EditPhase {
    Settling,
    Human,
    Passive,
}
fn edit_phase(age: Option<Duration>, before_insertion: bool) -> EditPhase {
    match age {
        Some(age) if age < EDIT_SETTLE && !before_insertion => EditPhase::Settling,
        Some(age) if age <= EDIT_WINDOW => EditPhase::Human,
        _ => EditPhase::Passive,
    }
}
struct Field {
    pid: i32,
    element: OwnedCf,
    focused: OwnedCf,
    scope: Scope,
}
pub(super) struct EmptyBaseline {
    pub pid: i32,
    pub element: OwnedCf,
    pub focused: OwnedCf,
    pub app: String,
    pub website: Option<String>,
}
impl Field {
    unsafe fn matches(&self, ctx: &Context) -> bool {
        self.pid == ctx.pid
            && self.scope.app == ctx.app
            && self.scope.website == ctx.website
            && CFEqual(self.element.0, ctx.element.0)
            && CFEqual(self.focused.0, ctx.focused.0)
    }
}
// Counts and fixed reason codes only: never input characters, key codes, app
// identities or writing. Lifetime counters retain transient failures between
// the once-per-second health snapshots.
#[derive(Default, serde::Serialize)]
struct CaptureCounters {
    edit_markers: u64,
    human_changes: u64,
    settling_observations: u64,
    passive_changes: u64,
    new_fields: u64,
    empty_baselines: u64,
    missing_context: u64,
    resets_with_field: u64,
    acceptance_matches: u64,
    acceptance_misses: u64,
    queued_records: u64,
    qualifying_records: u64,
    last_boundary: &'static str,
}
pub(super) struct Runtime {
    counters: CaptureCounters,
    worker: Engine,
    ui: HistoryUi,
    tracker: Tracker,
    field: Option<Field>,
    token: u64,
    input: Option<(i32, Instant)>,
    refresh: history::refresh::Schedule,
    config_stamp: Vec<u8>,
    snapshot: Snapshot,
    preview_epoch: Arc<AtomicU64>,
    preview_pending: Option<u64>,
    previews: mpsc::SyncSender<(u64, String)>,
    preview_results: mpsc::Receiver<(u64, Result<String, String>)>,
}
impl Runtime {
    pub(super) unsafe fn show_count(&self, control: id, selector: &Selector) {
        let message = match self.snapshot.record_count(selector, seconds()) {
            Some(count) => format!(
                "History: {count} stored record{}",
                if count == 1 { "" } else { "s" }
            ),
            None if matches!(self.snapshot.status, Status::Locked) => {
                "History: locked — unlock in History…".into()
            }
            None if matches!(self.snapshot.status, Status::Failed(_)) => {
                "History: unavailable — check History…".into()
            }
            None => "History: updating…".into(),
        };
        if ns_text(msg_send![control, stringValue]) != message {
            set_text(control, &message);
        }
    }
    pub(super) fn recording_active(&self, config: &Config) -> bool {
        config.history.recording && self.snapshot.unlocked
    }
    pub(super) unsafe fn new(target: id, config: &Config) -> Result<Self, String> {
        let path = Config::path()?
            .parent()
            .ok_or("Missing settings directory")?
            .join("history")
            .join("writing.lt");
        let worker =
            Engine::new(path, history::keychain::MacKeychain).map_err(|e| e.to_string())?;
        let preview_epoch = Arc::new(AtomicU64::new(1));
        let epoch = preview_epoch.clone();
        let (previews, requests) = mpsc::sync_channel::<(u64, String)>(1);
        let (results, preview_results) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((id, model)) = requests.recv() {
                if epoch.load(Ordering::Acquire) != id {
                    continue;
                }
                let result = synthetic_preview(&model, id, &epoch);
                if epoch.load(Ordering::Acquire) == id {
                    let _ = results.send((id, result));
                }
            }
        });
        Ok(Self {
            counters: CaptureCounters::default(),
            worker,
            ui: HistoryUi::new(target, &config.history),
            tracker: Tracker::default(),
            field: None,
            token: 0,
            input: None,
            refresh: history::refresh::Schedule::new(Instant::now()),
            config_stamp: stamp(config),
            snapshot: Snapshot::default(),
            preview_epoch,
            preview_pending: None,
            previews,
            preview_results,
        })
    }
    pub(super) fn diagnostic(&self, config: &Config) -> serde_json::Value {
        serde_json::json!({
            "recording": config.history.recording,
            "unlocked": self.snapshot.unlocked,
            "stored_records": self.snapshot.records,
            "status": format!("{:?}", self.snapshot.status),
            "revision": self.snapshot.revision,
            "profile_loaded": self.snapshot.profile.is_some(),
            "profile_revision": self.snapshot.profile.as_ref().map(|p| p.revision),
            "strength": config.history.strength,
            "field_active": self.field.is_some(),
            "tracker": self.tracker.diagnostic(),
            "counters": self.counters,
        })
    }
    pub(super) fn forget(&mut self) {
        if self.field.is_some() {
            self.counters.resets_with_field += 1;
        }
        self.tracker.clear();
        self.field = None;
        self.input = None;
    }
    pub(super) fn mark_input(&mut self, config: &Config, pid: i32, key: i64, flags: u64) {
        // Using an existing profile does not require recording more writing.
        // Navigation also counts as activity, so automatic analysis waits while
        // the user is interacting with an editor even when capture is disabled.
        self.refresh.note_input(Instant::now());
        if config.enabled
            && config.history.recording
            && self.snapshot.unlocked
            && edit_key(key, flags)
        {
            self.counters.edit_markers += 1;
            self.input = Some((pid, Instant::now()));
        }
    }
    fn enqueue(&mut self, config: &Config, records: Vec<Record>) {
        if records.is_empty() {
            return;
        }
        self.counters.queued_records += records.len() as u64;
        self.counters.qualifying_records +=
            records.iter().filter(|r| r.has_enough_writing()).count() as u64;
        if let Err(error) = self.worker.append(config, records, seconds()) {
            self.ui.message = error.to_string();
            self.forget();
        }
    }
    pub(super) fn finish(&mut self, config: &Config) {
        match self.tracker.finish(config, &config.history, seconds()) {
            Ok(record) => self.enqueue(config, record.into_iter().collect()),
            Err(error) => self.ui.message = error.to_string(),
        }
        self.forget();
    }
    pub(super) unsafe fn confirm_acceptance(&mut self, config: &Config, ctx: &Context) {
        if !config.enabled || !self.recording_active(config) || IsSecureEventInputEnabled() {
            self.forget();
            return;
        }
        let Some(field) = self.field.as_ref().filter(|field| field.matches(ctx)) else {
            self.counters.acceptance_misses += 1;
            return;
        };
        self.counters.acceptance_matches += 1;
        match self.tracker.confirm_acceptance(
            config,
            &config.history,
            self.token,
            &field.scope,
            seconds(),
        ) {
            Ok(records) => self.enqueue(config, records),
            Err(error) => {
                self.ui.message = error.to_string();
                self.forget();
            }
        }
    }
    pub(super) unsafe fn observe(
        &mut self,
        config: &Config,
        ctx: Option<&Context>,
        generated: bool,
    ) {
        self.observe_inner(config, ctx, generated, false);
    }
    pub(super) unsafe fn observe_empty(&mut self, config: &Config, empty: EmptyBaseline) {
        let scope = Scope { app: empty.app, website: empty.website };
        if !config.enabled || !config.history.recording || !self.snapshot.unlocked || IsSecureEventInputEnabled() {
            self.forget();
            return;
        }
        if !config.history.allows_recording(config, &scope) {
            self.finish(config);
            return;
        }
        let same = self.field.as_ref().is_some_and(|field|
            field.pid == empty.pid && field.scope == scope
                && CFEqual(field.element.0, empty.element.0)
                && CFEqual(field.focused.0, empty.focused.0));
        if !same {
            self.finish(config);
            self.counters.new_fields += 1;
            self.counters.empty_baselines += 1;
            self.token = self.token.wrapping_add(1).max(1);
            self.field = Some(Field { pid: empty.pid, element: empty.element,
                focused: empty.focused, scope: scope.clone() });
        }
        // No text is learned here. Keep the verified field identity and an empty
        // baseline, so a later same-field human paste can own its inserted span.
        let result = self.tracker.observe_passive(config, &config.history,
            WritingObservation { field: self.token, scope, text: "", caret: 0,
                selection_end: 0, now: seconds() });
        match result {
            Ok(records) => self.enqueue(config, records),
            Err(error) => { self.ui.message = error.to_string(); self.forget(); }
        }
    }
    /// Acceptance has revalidated this exact text, caret and field. Flush human
    /// ownership before our generated insertion even if the user accepted fast.
    pub(super) unsafe fn before_insertion(&mut self, config: &Config, ctx: &Context) {
        self.observe_inner(config, Some(ctx), false, true);
    }
    unsafe fn observe_inner(
        &mut self,
        config: &Config,
        ctx: Option<&Context>,
        generated: bool,
        before_insertion: bool,
    ) {
        if !config.enabled
            || !config.history.recording
            || !self.snapshot.unlocked
            || IsSecureEventInputEnabled()
        {
            self.forget();
            return;
        }
        let Some(ctx) = ctx else {
            if self.field.is_some() {
                self.counters.missing_context += 1;
                self.counters.last_boundary = CAPTURE_STATUS.with(Cell::get);
            }
            self.finish(config);
            return;
        };
        let scope = Scope {
            app: ctx.app.clone(),
            website: ctx.website.clone(),
        };
        if !config.history.allows_recording(config, &scope) {
            self.counters.last_boundary = "History scope excluded";
            self.finish(config);
            return;
        }
        // Keep the extra learning baseline bounded even in very large documents.
        if ctx.terminal.is_some() || ctx.text.len() > 256 * 1024 {
            self.finish(config);
            return;
        }
        let same_field = self.field.as_ref().is_some_and(|field| field.matches(ctx));
        if !same_field {
            self.counters.new_fields += 1;
            self.token = self.token.wrapping_add(1).max(1);
            self.field = Some(Field {
                pid: ctx.pid,
                element: ctx.element.clone(),
                focused: ctx.focused.clone(),
                scope: scope.clone(),
            });
        }
        let changed = !self.tracker.text_matches(&ctx.text);
        let phase = edit_phase(
            self.input
                .filter(|(pid, _)| *pid == ctx.pid)
                .map(|(_, time)| time.elapsed()),
            before_insertion,
        );
        if same_field && !generated && phase == EditPhase::Settling {
            self.counters.settling_observations += 1;
            return;
        }
        let human_edit = same_field && !generated && phase == EditPhase::Human;
        if changed && same_field && !generated {
            if human_edit {
                self.counters.human_changes += 1;
            } else {
                self.counters.passive_changes += 1;
            }
        }
        if changed || !same_field || generated {
            self.input = None;
        }
        let observation = WritingObservation {
            field: self.token,
            scope,
            text: &ctx.text,
            caret: ctx.prefix.len(),
            selection_end: ctx.text.len() - ctx.suffix.len(),
            now: seconds(),
        };
        let result = if generated {
            self.tracker
                .observe_generated(config, &config.history, observation)
        } else if same_field && changed && !human_edit {
            self.tracker
                .observe_passive(config, &config.history, observation)
        } else {
            self.tracker.observe(config, &config.history, observation)
        };
        match result {
            Ok(records) => self.enqueue(config, records),
            Err(error) => {
                self.ui.message = error.to_string();
                self.forget();
            }
        }
        if !generated {
            match self
                .tracker
                .flush_idle(config, &config.history, seconds(), 2)
            {
                Ok(record) => self.enqueue(config, record.into_iter().collect()),
                Err(error) => {
                    self.ui.message = error.to_string();
                    self.forget();
                }
            }
        }
    }
    pub(super) fn profile(&self, config: &Config, ctx: &Context) -> Option<Profile> {
        if self
            .snapshot
            .next_expiration
            .is_some_and(|deadline| seconds() >= deadline)
        {
            return None;
        }
        self.snapshot
            .profile
            .as_ref()
            .filter(|p| {
                p.instruction_for(config, &ctx.app, ctx.website.as_deref())
                    .is_some()
            })
            .cloned()
    }
    /// Returns true only for policy changes/retention expiry that must cancel
    /// suggestions immediately. Ordinary profile refresh affects future requests.
    pub(super) unsafe fn poll(&mut self, config: &Config) -> bool {
        let changed = self.config_stamp != stamp(config);
        let expired = self.snapshot.unlocked
            && self
                .snapshot
                .next_expiration
                .is_some_and(|expiry| seconds() >= expiry)
            && !matches!(self.snapshot.status, Status::Updating | Status::Unlocking);
        if changed || expired {
            self.counters.last_boundary = "History policy or retention changed";
            self.forget();
            self.config_stamp = stamp(config);
            self.snapshot.profile = None;
            if let Err(error) = self.worker.configure(config, seconds()) {
                self.ui.message = error.to_string();
            }
        }
        self.snapshot = self.worker.snapshot();
        if self.refresh.take_due(config, &self.snapshot, Instant::now()) {
            if let Err(error) = self.worker.build(config, seconds()) {
                self.ui.message = error.to_string();
            }
            self.snapshot = self.worker.snapshot();
        }
        while let Ok((id, result)) = self.preview_results.try_recv() {
            if self.preview_pending == Some(id) {
                self.preview_pending = None;
                self.ui.demo = result.unwrap_or_else(|e| format!("Sample preview failed: {e}"));
            }
        }
        self.ui
            .render(&self.snapshot, self.preview_pending.is_some());
        let kind: isize = msg_send![self.ui.scope, indexOfSelectedItem];
        let source = ns_text(msg_send![self.ui.source, stringValue]);
        let selector = match kind {
            0 => Some(Selector::App(source.trim().into())),
            1 => crate::website::canonical_host(source.trim())
                .ok()
                .map(Selector::Website),
            2 => Some(Selector::All),
            _ => None,
        };
        if let Some(selector) = selector.filter(Selector::valid) {
            self.show_count(self.ui.count, &selector);
        } else {
            set_text(
                self.ui.count,
                "Enter an app or exact website to see its record count.",
            );
        }
        changed || expired
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.preview_epoch.fetch_add(1, Ordering::SeqCst);
    }
}

fn synthetic_preview(model: &str, id: u64, epoch: &AtomicU64) -> Result<String, String> {
    let mut config = Config {
        model: model.into(),
        ..Config::default()
    };
    let mut data = history::Data::default();
    for (index, text) in history::profile::SYNTHETIC_VOCABULARY_SAMPLES
        .iter()
        .enumerate()
    {
        data.upsert(Record {
            id: [index as u8 + 1; 16],
            scope: Scope {
                app: "org.libretypist.preview".into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text: (*text).into(),
        })
        .map_err(|e| e.to_string())?;
    }
    let build =
        history::profile::BuildRequest::new(&data, &config, id, 30).map_err(|e| e.to_string())?;
    let profile = build.generate(&config, epoch).map_err(|e| e.to_string())?;
    config.history.strength = Strength::Medium;
    let request = CompletionRequest {
        id,
        prefix: "Please send the photonic lattice report to ".into(),
        suffix: String::new(),
        app_id: "org.libretypist.preview".into(),
        website: None,
        background: crate::background::Background::default(),
        profile: Some(profile.clone()),
    };
    let completion = model::generate(&config, &request, epoch).map_err(|e| e.to_string())?;
    Ok(format!(
        "Synthetic profile: {}\n\nSample completion: Please send the photonic lattice report to {}",
        profile.display_text(),
        completion
    ))
}

struct HistoryUi {
    window: id,
    recording: id,
    record_without_acceptance: id,
    strength: id,
    strength_label: id,
    automatic: id,
    apps: id,
    sites: id,
    retention: id,
    capacity: id,
    state: id,
    profile: id,
    scope: id,
    source: id,
    count: id,
    status: id,
    preview: id,
    demo: String,
    message: String,
}
unsafe fn set_text(control: id, text: &str) {
    let text = ns(text);
    let _: () = msg_send![control,setStringValue:text];
    let _: () = msg_send![text, release];
}
fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn popup(view: id, name: &str, items: &[&str], rect: NSRect) -> id {
    let control: id = msg_send![class!(NSPopUpButton), alloc];
    let control: id = msg_send![control,initWithFrame:rect pullsDown:NO];
    let value = ns(name);
    let _: () = msg_send![control,setAccessibilityLabel:value];
    let _: () = msg_send![value, release];
    for item in items {
        let value = ns(item);
        let _: () = msg_send![control,addItemWithTitle:value];
        let _: () = msg_send![value, release];
    }
    let _: () = msg_send![view,addSubview:control];
    let _: () = msg_send![control, release];
    control
}
unsafe fn checkbox(view: id, title: &str, y: f64) -> id {
    let control: id = msg_send![class!(NSButton), alloc];
    let control: id = msg_send![control,initWithFrame:frame(24.,y,610.,26.)];
    let _: () = msg_send![control,setButtonType:3usize];
    set_title(control, title);
    let _: () = msg_send![view,addSubview:control];
    let _: () = msg_send![control, release];
    control
}
impl HistoryUi {
    unsafe fn new(target: id, settings: &Settings) -> Self {
        let window: id = msg_send![class!(NSWindow), alloc];
        let window: id = msg_send![window,initWithContentRect:frame(0.,0.,680.,785.) styleMask:3usize backing:2usize defer:NO];
        set_title(window, "LibreType — Writing history & style");
        let _: () = msg_send![window,setReleasedWhenClosed:NO];
        let _: () = msg_send![window, center];
        let view: id = msg_send![window, contentView];
        label(
            view,
            "Writing history & style",
            frame(24., 730., 440., 36.),
            24.,
        );
        button(
            view,
            "Revert to saved",
            frame(480., 730., 178., 32.),
            target,
            sel!(revertHistory:),
        );
        label(view,"Optional learning from your writing. History is encrypted on this Mac.\nRecording and style influence start off. Unlock explicitly each time you launch.\nNew records need at least 40 non-whitespace characters.",frame(24.,680.,630.,45.),12.);
        let recording = checkbox(view, "Record new writing in eligible fields", 648.);
        let record_without_acceptance = checkbox(
            view,
            "Also keep sessions where no suggestion was accepted",
            613.,
        );
        label(view, "Style & vocabulary", frame(24., 577., 120., 26.), 13.);
        let strength: id = msg_send![class!(NSSlider), alloc];
        let strength: id = msg_send![strength, initWithFrame:frame(148.,572.,350.,36.)];
        let _: () = msg_send![strength,setMinValue:0f64];
        let _: () = msg_send![strength,setMaxValue:5f64];
        let _: () = msg_send![strength,setNumberOfTickMarks:6isize];
        let _: () = msg_send![strength,setAllowsTickMarkValuesOnly:YES];
        let _: () = msg_send![strength,setContinuous:YES];
        let _: () = msg_send![strength,setTarget:target];
        let _: () = msg_send![strength,setAction:sel!(historyStrength:)];
        let name = ns("Style and vocabulary influence: Off to Strong");
        let _: () = msg_send![strength,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![view,addSubview:strength];
        let _: () = msg_send![strength, release];
        let strength_label = label(view, "Off", frame(510., 577., 145., 26.), 13.);
        let automatic = checkbox(view, "Refresh the profile automatically after pauses", 543.);
        label(
            view,
            "Exclude apps (bundle IDs, comma-separated)",
            frame(24., 511., 630., 24.),
            12.,
        );
        let apps = text_input(
            view,
            "",
            "History excluded apps",
            frame(24., 482., 630., 27.),
        );
        label(
            view,
            "Exclude websites (exact domains, comma-separated)",
            frame(24., 451., 630., 24.),
            12.,
        );
        let sites = text_input(
            view,
            "",
            "History excluded websites",
            frame(24., 422., 630., 27.),
        );
        label(view, "Keep days", frame(24., 385., 80., 26.), 12.);
        let retention = text_input(
            view,
            "90",
            "History retention days",
            frame(105., 385., 75., 27.),
        );
        label(view, "Max records", frame(204., 385., 90., 26.), 12.);
        let capacity = text_input(
            view,
            "2000",
            "History record limit",
            frame(297., 385., 85., 27.),
        );
        button(
            view,
            "Save preferences",
            frame(480., 382., 178., 32.),
            target,
            sel!(saveHistory:),
        );
        let state = label(
            view,
            "History locked. No Keychain access on startup.",
            frame(24., 329., 365., 42.),
            12.,
        );
        button(
            view,
            "Unlock…",
            frame(400., 334., 120., 32.),
            target,
            sel!(unlockHistory:),
        );
        button(
            view,
            "Lock",
            frame(530., 334., 124., 32.),
            target,
            sel!(lockHistory:),
        );
        button(
            view,
            "Build / update profile",
            frame(20., 286., 220., 32.),
            target,
            sel!(buildHistory:),
        );
        label(
            view,
            "Needs 3 distinct samples and 300 characters.",
            frame(255., 289., 400., 26.),
            12.,
        );
        let scroll: id = msg_send![class!(NSScrollView), alloc];
        let scroll: id = msg_send![scroll,initWithFrame:frame(24.,200.,630.,83.)];
        let _: () = msg_send![scroll,setHasVerticalScroller:YES];
        let _: () = msg_send![scroll,setBorderType:1usize];
        let profile: id = msg_send![class!(NSTextView), alloc];
        let profile: id = msg_send![profile,initWithFrame:frame(0.,0.,610.,83.)];
        let _: () = msg_send![profile,setEditable:NO];
        let _: () = msg_send![profile,setSelectable:YES];
        let _: () = msg_send![profile,setRichText:NO];
        let font: id = msg_send![class!(NSFont),systemFontOfSize:12f64];
        let _: () = msg_send![profile,setFont:font];
        let _: () = msg_send![profile,setVerticallyResizable:YES];
        let _: () = msg_send![profile,setHorizontallyResizable:NO];
        let _: () = msg_send![profile,setAutoresizingMask:2usize];
        let container: id = msg_send![profile, textContainer];
        let _: () = msg_send![container,setWidthTracksTextView:YES];
        let name = ns("Learned style or synthetic preview");
        let _: () = msg_send![profile,setAccessibilityLabel:name];
        let _: () = msg_send![name, release];
        let _: () = msg_send![scroll,setDocumentView:profile];
        let _: () = msg_send![profile, release];
        let _: () = msg_send![view,addSubview:scroll];
        let _: () = msg_send![scroll, release];
        let count = label(
            view,
            "Enter an app or exact website to see its record count.",
            frame(24., 167., 630., 18.),
            12.,
        );
        let scope = popup(
            view,
            "History deletion scope",
            &["App bundle ID", "Exact website", "All history"],
            frame(20., 135., 172., 30.),
        );
        let source = text_input(
            view,
            "",
            "History deletion target",
            frame(200., 136., 313., 27.),
        );
        button(
            view,
            "Delete…",
            frame(526., 133., 132., 32.),
            target,
            sel!(deleteHistory:),
        );
        let preview: id = msg_send![class!(NSButton), alloc];
        let preview: id = msg_send![preview,initWithFrame:frame(20.,90.,250.,32.)];
        set_title(preview, "Try a synthetic profile");
        let _: () = msg_send![preview,setBezelStyle:1usize];
        let _: () = msg_send![preview,setTarget:target];
        let _: () = msg_send![preview,setAction:sel!(previewHistory:)];
        let _: () = msg_send![view,addSubview:preview];
        let _: () = msg_send![preview, release];
        label(
            view,
            "Demo uses no real writing or Keychain.",
            frame(288., 93., 365., 26.),
            12.,
        );
        let status = label(
            view,
            "Save settings, then unlock to record or use a saved profile.",
            frame(24., 14., 630., 67.),
            12.,
        );
        let ui = Self {
            window,
            recording,
            record_without_acceptance,
            strength,
            strength_label,
            automatic,
            apps,
            sites,
            retention,
            capacity,
            state,
            profile,
            scope,
            source,
            count,
            status,
            preview,
            demo: String::new(),
            message: String::new(),
        };
        ui.load(settings);
        ui
    }
    unsafe fn load(&self, settings: &Settings) {
        let _: () = msg_send![self.recording,setState:if settings.recording {1isize} else {0isize}];
        let _: () = msg_send![self.record_without_acceptance,setState:if settings.record_without_acceptance {1isize} else {0isize}];
        let _: () =
            msg_send![self.automatic,setState:if settings.auto_refresh {1isize} else {0isize}];
        let _: () = msg_send![self.strength,setDoubleValue:settings.strength.position() as f64];
        self.refresh_strength_label();
        set_text(self.apps, &settings.excluded_apps.join(", "));
        set_text(self.sites, &settings.excluded_websites.join(", "));
        set_text(self.retention, &settings.retention_days.to_string());
        set_text(self.capacity, &settings.max_records.to_string());
    }
    unsafe fn refresh_strength_label(&self) {
        let value: f64 = msg_send![self.strength, doubleValue];
        set_text(
            self.strength_label,
            Strength::from_position(value).map_or("Choose a level", Strength::label),
        );
    }
    unsafe fn draft(&self) -> Result<Settings, String> {
        let recording: isize = msg_send![self.recording, state];
        let record_without_acceptance: isize = msg_send![self.record_without_acceptance, state];
        let automatic: isize = msg_send![self.automatic, state];
        let strength: f64 = msg_send![self.strength, doubleValue];
        let apps = ns_text(msg_send![self.apps, stringValue]);
        let sites = ns_text(msg_send![self.sites, stringValue]);
        let mut settings = Settings {
            recording: recording == 1,
            record_without_acceptance: record_without_acceptance == 1,
            auto_refresh: automatic == 1,
            strength: Strength::from_position(strength)
                .ok_or("Choose one of the six influence levels.")?,
            excluded_apps: apps
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            excluded_websites: sites
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(crate::website::canonical_host)
                .collect::<Result<_, _>>()?,
            retention_days: ns_text(msg_send![self.retention, stringValue])
                .trim()
                .parse()
                .map_err(|_| "Use a whole number for retention days.")?,
            max_records: ns_text(msg_send![self.capacity, stringValue])
                .trim()
                .parse()
                .map_err(|_| "Use a whole number for the record limit.")?,
        };
        settings.excluded_apps.sort();
        settings.excluded_apps.dedup();
        settings.excluded_websites.sort();
        settings.excluded_websites.dedup();
        settings.validate()?;
        Ok(settings)
    }
    unsafe fn render(&self, snapshot: &Snapshot, previewing: bool) {
        let state = match snapshot.status {
            Status::Locked => "Locked".into(),
            Status::Unlocking => "Unlocking… macOS may request access".into(),
            Status::Ready => format!("Unlocked · {} records", snapshot.records),
            Status::Building => "Building a style profile locally…".into(),
            Status::Updating => "Updating encrypted history…".into(),
            Status::Failed(error) => error.to_string(),
        };
        set_text(self.state, &state);
        let guidance = zeroize::Zeroizing::new(if !self.demo.is_empty() {
            self.demo.clone()
        } else {
            snapshot
                .profile
                .as_ref()
                .map(Profile::display_text)
                .unwrap_or_else(|| {
                    "No learned profile loaded. The synthetic preview does not save a profile."
                        .into()
                })
        });
        // Preserve selection and scroll position across the regular UI poll.
        if ns_text(msg_send![self.profile, string]) != guidance.as_str() {
            let text = ns(&guidance);
            let _: () = msg_send![self.profile,setString:text];
            let _: () = msg_send![text, release];
        }
        let message = if previewing {
            format!(
                "Building a synthetic profile and example completion…\n{}",
                self.message
            )
        } else if self.message.is_empty() {
            "Excluded apps/sites do not record or use the learned profile. Recording also requires Accessibility access.".into()
        } else {
            self.message.clone()
        };
        set_text(self.status, &message);
        let _: () = msg_send![self.preview,setEnabled:if previewing {NO} else {YES}];
    }
}

extern "C" fn strength_changed(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow().as_ref() {
            unsafe {
                s.history.ui.refresh_strength_label();
            }
        }
    });
}
extern "C" fn show(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.history.finish(&s.config);
                s.clear();
                // The window stays alive when closed. Reopening must preserve
                // unfinished preferences, including invalid numeric/domain input.
                // Only Save or the explicit Revert action replaces these controls.
                s.history.ui.demo.clear();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.history.ui.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn revert(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.history.ui.load(&s.config.history);
                s.history.ui.message = "Restored saved preferences. No settings changed.".into();
            }
        }
    });
}
extern "C" fn save(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut config = s.config.clone();
                match s.history.ui.draft().and_then(|settings| {
                    config.history = settings;
                    config.save()
                }) {
                    Ok(()) => {
                        s.history.forget();
                        s.clear();
                        s.config = config;
                        s.history.ui.load(&s.config.history);
                        s.history.poll(&s.config);
                        s.history.ui.message =
                            "Saved. Unlock history to record writing or use a stored profile."
                                .into();
                    }
                    Err(error) => s.history.ui.message = format!("Not saved: {error}"),
                }
            }
        }
    });
}
extern "C" fn unlock(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.history.forget();
                s.clear();
                s.history.ui.demo.clear();
                s.history.ui.message = match s.history.worker.unlock(&s.config, seconds()) {
                    Ok(()) => "Unlock requested. macOS may ask for Keychain access.".into(),
                    Err(error) => error.to_string(),
                };
            }
        }
    });
}
extern "C" fn lock(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.history.forget();
                s.clear();
                s.history.snapshot.profile = None;
                s.history.ui.demo.clear();
                s.history.ui.message = match s.history.worker.lock() {
                    Ok(()) => {
                        "History locked. Recording and learned style are inactive until unlocked."
                            .into()
                    }
                    Err(error) => error.to_string(),
                };
            }
        }
    });
}
extern "C" fn build(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                s.history.finish(&s.config);
                s.clear();
                s.history.ui.demo.clear();
                s.history.refresh.note_attempt(Instant::now());
                s.history.ui.message = match s.history.worker.build(&s.config, seconds()) {
                    Ok(()) => "Building from permitted stored writing, locally.".into(),
                    Err(error) => error.to_string(),
                };
            }
        }
    });
}
extern "C" fn delete(_: &Object, _: Sel, _: id) {
    STATE.with(|state|{if let Some(s)=state.borrow_mut().as_mut(){unsafe{
        let kind:isize=msg_send![s.history.ui.scope,indexOfSelectedItem];let source=ns_text(msg_send![s.history.ui.source,stringValue]);
        let selector=match kind{0=>Selector::App(source.trim().into()),1=>match crate::website::canonical_host(source.trim()){Ok(host)=>Selector::Website(host),Err(e)=>{s.history.ui.message=e;return;}},2=>Selector::All,_=>return};
        if !selector.valid(){s.history.ui.message="Enter a valid app bundle ID or exact website.".into();return;}
        if !s.history.worker.snapshot().unlocked {s.history.ui.message="Unlock history before deleting records.".into();return;}
        let description=match &selector{Selector::All=>"all recorded writing".into(),Selector::App(app)=>format!("writing recorded in {app}"),Selector::Website(site)=>format!("writing recorded on {site}")};
        let alert:id=msg_send![class!(NSAlert),new];let title=ns("Delete writing history?");let detail=ns(&format!("Delete {description} and clear the learned profile? This cannot be undone in LibreType."));
        let _:()=msg_send![alert,setMessageText:title];let _:()=msg_send![alert,setInformativeText:detail];for text in ["Cancel","Delete"]{let text=ns(text);let _:id=msg_send![alert,addButtonWithTitle:text];let _:()=msg_send![text,release];}
        let answer:isize=msg_send![alert,runModal];let _:()=msg_send![alert,release];let _:()=msg_send![title,release];let _:()=msg_send![detail,release];
        if answer==1001{s.history.forget();s.clear();s.history.snapshot.profile=None;s.history.ui.demo.clear();s.history.ui.message=match s.history.worker.delete(&s.config,selector,seconds()){Ok(())=>"Deletion requested; the history status shows when it finishes.".into(),Err(error)=>error.to_string()};}
    }}});
}
extern "C" fn preview(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                if s.history.preview_pending.is_some() {
                    return;
                }
                s.history.finish(&s.config);
                s.clear();
                let id = s.history.preview_epoch.fetch_add(1, Ordering::SeqCst) + 1;
                s.history.ui.demo.clear();
                s.history.ui.message =
                    "Demo does not record, save a profile, or access Keychain.".into();
                if s.history
                    .previews
                    .try_send((id, s.config.model.clone()))
                    .is_ok()
                {
                    s.history.preview_pending = Some(id);
                } else {
                    s.history.ui.message = "Sample worker is busy; try again shortly.".into();
                }
            }
        }
    });
}
pub(super) unsafe fn register_actions(decl: &mut ClassDecl) {
    for (selector, action) in [
        (sel!(history:), show as extern "C" fn(&Object, Sel, id)),
        (sel!(historyStrength:), strength_changed),
        (sel!(saveHistory:), save),
        (sel!(revertHistory:), revert),
        (sel!(unlockHistory:), unlock),
        (sel!(lockHistory:), lock),
        (sel!(buildHistory:), build),
        (sel!(deleteHistory:), delete),
        (sel!(previewHistory:), preview),
    ] {
        decl.add_method(selector, action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_empty_baseline_distinguishes_human_paste_from_programmatic_load() {
        let config = Config::default();
        let settings = Settings { recording: true, ..Settings::default() };
        let scope = Scope { app: "com.google.Chrome".into(), website: Some("127.0.0.1".into()) };
        let sample = |text: &'static str, now| WritingObservation {
            field: 1, scope: scope.clone(), text, caret: text.len(), selection_end: text.len(), now,
        };
        const PASTED: &str = "Hi Ariadne, thanks for reviewing the photonic lattice. The prototype is ready for another calibration session tomorrow. Keep the photons dancing. Thank you for your ";
        let mut tracker = Tracker::default();
        tracker.observe_passive(&config, &settings, sample("", 1)).unwrap();
        tracker.observe_passive(&config, &settings, sample("", 2)).unwrap();
        assert_eq!(edit_phase(Some(Duration::from_millis(250)), false), EditPhase::Human);
        tracker.observe(&config, &settings, sample(PASTED, 3)).unwrap();
        assert!(tracker.flush_idle(&config, &settings, 5, 2).unwrap().is_none());
        let records = tracker.confirm_acceptance(&config, &settings, 1, &scope, 6).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, PASTED);
        tracker.clear();
        tracker.observe_passive(&config, &settings, sample("", 7)).unwrap();
        tracker.observe_passive(&config, &settings, sample(PASTED, 8)).unwrap();
        assert!(tracker.confirm_acceptance(&config, &settings, 1, &scope, 9).unwrap().is_empty());
    }

    #[test]
    fn typing_burst_keeps_complete_human_span_despite_torn_ax_samples() {
        let config = Config::default();
        let settings = Settings {
            recording: true,
            ..Settings::default()
        };
        let scope = Scope {
            app: "com.apple.TextEdit".into(),
            website: None,
        };
        let sample = |text: &'static str, caret, now| WritingObservation {
            field: 1,
            scope: scope.clone(),
            text,
            caret,
            selection_end: caret,
            now,
        };
        let mut tracker = Tracker::default();
        tracker
            .observe(&config, &settings, sample("", 0, 1))
            .unwrap();
        // These snapshots represent separate value/range reads during a burst.
        // Do not let a value ahead of its caret fragment the owned writing.
        for (text, caret) in [("The", 2), ("The Zephyr team", 12)] {
            let phase = edit_phase(Some(Duration::from_millis(50)), false);
            if phase != EditPhase::Settling {
                tracker
                    .observe(&config, &settings, sample(text, caret, 2))
                    .unwrap();
            }
        }
        const HUMAN: &str =
            "The Zephyr team will meet in Cedar Hall on Thursday morning. Thank you for your ";
        assert_eq!(
            edit_phase(Some(Duration::from_millis(250)), false),
            EditPhase::Human
        );
        tracker
            .observe(&config, &settings, sample(HUMAN, HUMAN.len(), 3))
            .unwrap();
        tracker.flush_idle(&config, &settings, 5, 2).unwrap();
        const COMPLETE: &str =
            "The Zephyr team will meet in Cedar Hall on Thursday morning. Thank you for your help.";
        tracker
            .observe_generated(&config, &settings, sample(COMPLETE, COMPLETE.len(), 6))
            .unwrap();
        let records = tracker
            .confirm_acceptance(&config, &settings, 1, &scope, 7)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, HUMAN);
        assert!(records[0].has_enough_writing());
        // Once consumed, an unrelated programmatic load is still passive.
        assert_eq!(edit_phase(None, false), EditPhase::Passive);
        assert_eq!(
            edit_phase(Some(Duration::from_millis(1001)), false),
            EditPhase::Passive
        );
        tracker
            .observe_passive(&config, &settings, sample("Loaded document", 15, 8))
            .unwrap();
        assert!(tracker
            .confirm_acceptance(&config, &settings, 1, &scope, 9)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn verified_fast_acceptance_flushes_human_writing_before_generated_text() {
        assert_eq!(
            edit_phase(Some(Duration::from_millis(50)), true),
            EditPhase::Human
        );
        assert_eq!(edit_phase(None, true), EditPhase::Passive);
        assert_eq!(
            edit_phase(Some(Duration::from_secs(2)), true),
            EditPhase::Passive
        );
    }

    #[test]
    fn edit_markers_skip_navigation_and_non_editing_commands() {
        use crate::shortcuts::{COMMAND, CONTROL, SHIFT};
        for key in [0, 36, 48, 49, 51, 117] {
            assert!(edit_key(key, 0));
            assert!(edit_key(key, SHIFT));
        }
        for key in [53, 115, 116, 119, 121, 123, 124, 125, 126] {
            assert!(!edit_key(key, 0));
        }
        for key in [6, 7, 9, 51, 117] {
            assert!(edit_key(key, COMMAND));
        }
        for key in [0, 8, 12, 48] {
            assert!(!edit_key(key, COMMAND));
        }
        assert!(!edit_key(9, COMMAND | CONTROL));
    }
}
