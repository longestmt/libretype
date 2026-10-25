// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Original macOS integration. No global keystroke history is collected.
#![allow(deprecated, unexpected_cfgs)]
use crate::{
    acceptance::{AcceptancePlan, Observation, ReplacementPlan},
    config::Config,
    core::CompletionRequest,
    model,
    shortcuts::{Action, DismissBehavior, Dismissal, MODIFIERS, OPTION, SHIFT},
};
use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::declare::ClassDecl;
use objc::runtime::{Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    ffi::{c_void, CStr},
    ptr,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[path = "native_background.rs"]
mod background;
#[path = "native_choices.rs"]
mod choices;
#[path = "native_cells.rs"]
mod cells;
#[path = "native_code_editor.rs"]
mod code_editor;
#[path = "native_emoji.rs"]
mod emoji;
#[path = "native_font.rs"]
mod font;
#[path = "native_history.rs"]
mod history;
#[path = "native_keyboard.rs"]
mod keyboard;
#[path = "native_models.rs"]
mod models;
#[path = "native_screen.rs"]
mod screen;
#[path = "native_spelling.rs"]
mod spelling;
#[path = "native_startup.rs"]
mod startup;
#[path = "native_statistics.rs"]
mod statistics;
#[path = "native_terminal.rs"]
mod terminal;
#[path = "native_websites.rs"]
mod websites;

type CF = *const c_void;
type AX = CF;
#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSFontAttributeName: id;
    static NSForegroundColorAttributeName: id;
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range {
    location: isize,
    length: isize,
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CF) -> bool;
    fn AXUIElementCreateSystemWide() -> AX;
    fn AXUIElementCreateApplication(pid: i32) -> AX;
    fn AXUIElementSetMessagingTimeout(element: AX, timeout: f32) -> i32;
    fn AXUIElementGetPid(element: AX, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(element: AX, attribute: CF, result: *mut CF) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AX,
        attribute: CF,
        parameter: CF,
        result: *mut CF,
    ) -> i32;
    fn AXUIElementIsAttributeSettable(element: AX, attribute: CF, result: *mut bool) -> i32;
    fn AXUIElementSetAttributeValue(element: AX, attribute: CF, value: CF) -> i32;
    fn AXValueGetValue(value: CF, value_type: i32, result: *mut c_void) -> bool;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetType(value: CF) -> i32;
    fn AXValueCreate(value_type: i32, value: *const c_void) -> CF;
    static kAXFontTextAttribute: CF;
    static kAXFontNameKey: CF;
    static kAXFontSizeKey: CF;
    static kAXFontFamilyKey: CF;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: extern "C" fn(CF, u32, CF, *mut c_void) -> CF,
        user: *mut c_void,
    ) -> CF;
    fn CGEventTapEnable(tap: CF, enabled: bool);
    fn CGEventGetIntegerValueField(event: CF, field: u32) -> i64;
    fn CGEventGetFlags(event: CF) -> u64;
    fn CGEventSourceCreate(state: i32) -> CF;
    fn CGEventCreateKeyboardEvent(source: CF, key: u16, down: bool) -> CF;
    fn CGEventKeyboardSetUnicodeString(event: CF, length: usize, text: *const u16);
    fn CGEventSetFlags(event: CF, flags: u64);
    fn CGEventSetIntegerValueField(event: CF, field: u32, value: i64);
    fn CGEventPostToPid(pid: i32, event: CF);
}
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: CF);
    fn CFRetain(value: CF) -> CF;
    fn CFEqual(a: CF, b: CF) -> bool;
    fn CFGetTypeID(value: CF) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFURLGetTypeID() -> usize;
    fn CFURLGetString(url: CF) -> CF;
    fn CFAttributedStringGetTypeID() -> usize;
    fn CFAttributedStringGetLength(value: CF) -> isize;
    fn CFAttributedStringGetString(value: CF) -> CF;
    fn CFAttributedStringGetAttribute(value: CF, index: isize, name: CF, range: *mut Range) -> CF;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(value: CF, key: CF) -> CF;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: CF, kind: i32, output: *mut c_void) -> bool;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: CF) -> bool;
    static kCFBooleanTrue: CF;
    fn CFStringCreateWithBytes(
        allocator: CF,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: bool,
    ) -> CF;
    fn CFStringGetLength(value: CF) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: CF, buffer: *mut i8, size: isize, encoding: u32) -> bool;
    fn CFMachPortCreateRunLoopSource(allocator: CF, port: CF, order: isize) -> CF;
    fn CFRunLoopGetMain() -> CF;
    fn CFRunLoopAddSource(loop_: CF, source: CF, mode: CF);
    static kCFRunLoopCommonModes: CF;
}

const UTF8: u32 = 0x08000100;
const INSERT_EVENT_TAG: i64 = 0x4c54_5950;
// The updater requests the same shutdown as the menu. The signal handler does
// no allocation, AppKit work or locking; the main timer performs termination.
static QUIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn request_termination(_: libc::c_int) {
    QUIT_REQUESTED.store(true, Ordering::Relaxed);
}
struct OwnedCf(CF);
impl Drop for OwnedCf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
impl Clone for OwnedCf {
    fn clone(&self) -> Self {
        Self(unsafe { CFRetain(self.0) })
    }
}
unsafe fn cfstr(s: &str) -> OwnedCf {
    OwnedCf(CFStringCreateWithBytes(
        ptr::null(),
        s.as_ptr(),
        s.len() as isize,
        UTF8,
        false,
    ))
}
unsafe fn string(value: CF) -> Option<String> {
    if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
        return None;
    }
    let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) + 1;
    if size <= 0 || size > 8_000_000 {
        return None;
    }
    let mut bytes = vec![0u8; size as usize];
    if !CFStringGetCString(value, bytes.as_mut_ptr() as *mut i8, size, UTF8) {
        return None;
    }
    Some(
        CStr::from_ptr(bytes.as_ptr() as *const i8)
            .to_string_lossy()
            .into_owned(),
    )
}
unsafe fn attr(element: AX, name: &str) -> Option<OwnedCf> {
    attr_checked(element, name).ok()
}
unsafe fn attr_checked(element: AX, name: &str) -> Result<OwnedCf, i32> {
    let name = cfstr(name);
    let mut out = ptr::null();
    let result = AXUIElementCopyAttributeValue(element, name.0, &mut out);
    if result == 0 && !out.is_null() {
        Ok(OwnedCf(out))
    } else {
        Err(result)
    }
}
unsafe fn parameterized(element: AX, name: &str, parameter: CF) -> Result<OwnedCf, i32> {
    let name = cfstr(name);
    let mut out = ptr::null();
    let result = AXUIElementCopyParameterizedAttributeValue(element, name.0, parameter, &mut out);
    if result == 0 && !out.is_null() {
        Ok(OwnedCf(out))
    } else {
        Err(result)
    }
}

// Chromium's frameForRange deliberately rejects non-atomic contenteditables.
// Their selected text markers preserve the DOM leaf and offset needed to obtain
// the real caret. Treat markers as opaque and query only the verified editor.
unsafe fn marker_caret(element: AX, selection_length: isize) -> Option<NSRect> {
    let marker = attr_checked(element, "AXSelectedTextMarkerRange");
    CAPTURE_DETAIL.with(|d| d.borrow_mut().marker_selection = marker.as_ref().err().copied());
    let marker = marker.ok()?;
    let length = parameterized(element, "AXLengthForTextMarkerRange", marker.0).ok()?;
    let mut count: i64 = -1;
    if CFGetTypeID(length.0) != CFNumberGetTypeID()
        || !CFNumberGetValue(length.0, 4, &mut count as *mut i64 as *mut c_void)
        || count != selection_length as i64
    {
        return None;
    }
    let bounds = parameterized(element, "AXBoundsForTextMarkerRange", marker.0);
    CAPTURE_DETAIL.with(|d| d.borrow_mut().marker_bounds = bounds.as_ref().err().copied());
    let bounds = bounds.ok()?;
    let mut caret = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    (ax_value(bounds.0, 3, &mut caret) && valid_rect(caret)).then_some(caret)
}
unsafe fn is_settable(element: AX, name: &str) -> bool {
    let name = cfstr(name);
    let mut writable = false;
    AXUIElementIsAttributeSettable(element, name.0, &mut writable) == 0 && writable
}
unsafe fn bool_attr(element: AX, name: &str) -> Option<bool> {
    let value = attr(element, name)?;
    (CFGetTypeID(value.0) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(value.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertionMode {
    SelectedText,
    UnicodeEvent,
}

fn insertion_mode(
    selected_text: bool,
    value: bool,
    enabled: bool,
    browser: bool,
) -> Option<InsertionMode> {
    if !enabled {
        None
    } else if browser && value {
        // Browser AX setters can report success without updating the editor,
        // including plain textareas with working AXBoundsForRange. Transport
        // must not depend on which API happened to provide caret geometry.
        Some(InsertionMode::UnicodeEvent)
    } else if selected_text {
        Some(InsertionMode::SelectedText)
    } else if value {
        Some(InsertionMode::UnicodeEvent)
    } else {
        None
    }
}

// One process-targeted event pair; no clipboard, shortcuts, or global posting.
// Use a conservative cap of 20 UTF-16 units per accepted event pair.
fn event_payload(text: &str) -> Option<Vec<u16>> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let units: Vec<_> = text.encode_utf16().collect();
    (units.len() <= 20).then_some(units)
}

unsafe fn insert_unicode(ctx: &Context, text: &str) -> bool {
    let Some(units) = event_payload(text) else {
        return false;
    };
    let source = OwnedCf(CGEventSourceCreate(-1));
    if source.0.is_null() {
        return false;
    }
    let down = OwnedCf(CGEventCreateKeyboardEvent(source.0, 0, true));
    let up = OwnedCf(CGEventCreateKeyboardEvent(source.0, 0, false));
    if down.0.is_null() || up.0.is_null() {
        return false;
    }
    for event in [&down, &up] {
        CGEventSetFlags(event.0, 0);
        CGEventSetIntegerValueField(event.0, 42, INSERT_EVENT_TAG);
        CGEventKeyboardSetUnicodeString(event.0, units.len(), units.as_ptr());
    }
    let system = OwnedCf(AXUIElementCreateSystemWide());
    let Some(focused) = attr(system.0, "AXFocusedUIElement") else {
        return false;
    };
    if !CFEqual(focused.0, ctx.focused.0)
        || IsSecureEventInputEnabled()
        || front_identity() != (ctx.pid, ctx.app.clone())
    {
        return false;
    }
    CGEventPostToPid(ctx.pid, down.0);
    CGEventPostToPid(ctx.pid, up.0);
    true
}
unsafe fn ax_value<T>(value: CF, kind: i32, output: &mut T) -> bool {
    !value.is_null()
        && CFGetTypeID(value) == AXValueGetTypeID()
        && AXValueGetType(value) == kind
        && AXValueGetValue(value, kind, output as *mut T as *mut c_void)
}
fn valid_rect(rect: NSRect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
        && rect.size.width >= 0.
        && rect.size.height > 0.
}
fn rect_coords(rect: NSRect) -> (f64, f64, f64, f64) {
    (
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}
unsafe fn ns(s: &str) -> id {
    let v: id = msg_send![class!(NSString), alloc];
    msg_send![v, initWithBytes:s.as_ptr() length:s.len() encoding:4usize]
}
unsafe fn ns_text(v: id) -> String {
    if v == nil {
        return String::new();
    }
    let p: *const i8 = msg_send![v, UTF8String];
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
unsafe fn set_title(item: id, title: &str) {
    // AppKit's status-bar button redraws even when setTitle receives the same
    // text. This helper also serves menu items and windows; leave unchanged
    // titles alone while still reflecting a changed title on the current tick.
    if ns_text(msg_send![item, title]) == title {
        return;
    }
    let t = ns(title);
    let _: () = msg_send![item,setTitle:t];
    let _: () = msg_send![t, release];
}

#[derive(Clone)]
struct Context {
    element: OwnedCf,
    focused: OwnedCf,
    insertion: InsertionMode,
    pid: i32,
    app: String,
    website: Option<String>,
    text: String,
    prefix: String,
    suffix: String,
    range: Range,
    caret: NSRect,
    field_bounds: NSRect,
    terminal: Option<crate::terminal::Scope>,
}
enum CapturedField {
    Editor(Box<Context>),
    EmptyHistory(history::EmptyBaseline),
}
impl Context {
    unsafe fn same(&self, other: &Self) -> bool {
        self.terminal == other.terminal
            && self.website == other.website
            && self.app == other.app
            && self.pid == other.pid
            && if self.terminal.is_some() {
                self.prefix == other.prefix && self.suffix == other.suffix
            } else {
                self.range == other.range && self.text == other.text
            }
            && self.insertion == other.insertion
            && CFEqual(self.focused.0, other.focused.0)
            && CFEqual(self.element.0, other.element.0)
    }
}
unsafe fn front_identity() -> (i32, String) {
    let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
    let app: id = msg_send![ws, frontmostApplication];
    let pid: i32 = msg_send![app, processIdentifier];
    if pid == std::process::id() as i32 {
        return (pid, String::new());
    }
    (pid, ns_text(msg_send![app, bundleIdentifier]))
}
unsafe fn front_app() -> String {
    front_identity().1
}
unsafe fn front_name() -> String {
    let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
    let app: id = msg_send![workspace, frontmostApplication];
    ns_text(msg_send![app, localizedName])
}
// Read only the focused field's ancestor chain. The outermost web area owns
// the website rule; never inspect browser history, address-bar text or other tabs.
unsafe fn website_for_element(
    focused: &OwnedCf,
    pid: i32,
    timeout: f32,
) -> Result<Option<String>, ()> {
    let started = Instant::now();
    let mut node = focused.clone();
    let mut seen: Vec<OwnedCf> = Vec::new();
    let mut scope = crate::website::DocumentScope::default();
    for _ in 0..32 {
        if started.elapsed() > Duration::from_secs_f32(timeout.max(0.005))
            || seen.iter().any(|old| CFEqual(old.0, node.0))
        {
            return Err(());
        }
        let mut node_pid = 0;
        if AXUIElementGetPid(node.0, &mut node_pid) != 0 || node_pid != pid {
            return Err(());
        }
        let role = attr(node.0, "AXRole").and_then(|v| string(v.0)).ok_or(())?;
        if matches!(role.as_str(), "AXWindow" | "AXApplication") {
            return scope.finish().map_err(|_| ());
        }
        if role == "AXWebArea" {
            let url = attr(node.0, "AXURL").and_then(|v| {
                if CFGetTypeID(v.0) == CFURLGetTypeID() {
                    string(CFURLGetString(v.0))
                } else {
                    string(v.0)
                }
            });
            scope.observe(url.as_deref());
        }
        seen.push(node.clone());
        node = attr(node.0, "AXParent").ok_or(())?;
    }
    Err(())
}

unsafe fn capture(config: &Config) -> Option<Context> {
    capture_with_timeout(config, 0.08)
}
thread_local! {
    static CAPTURE_STATUS: Cell<&'static str> = const { Cell::new("Starting") };
    static FONT_DIAGNOSTIC: Cell<(&'static str, f64)> = const { Cell::new(("not requested", 0.)) };
    static GHOST_DRAWS: Cell<u64> = const { Cell::new(0) };
    static CAPTURE_DETAIL: RefCell<CaptureDetail> = RefCell::new(CaptureDetail::default());
}
#[derive(Default, serde::Serialize)]
struct CaptureDetail {
    role: String,
    cell_context: Option<&'static str>,
    code_editor_kind: Option<&'static str>,
    caret_source: &'static str,
    range_bounds: Option<i32>,
    marker_selection: Option<i32>,
    marker_bounds: Option<i32>,
    code_editor_viewport: Option<[f64; 4]>,
}
fn capture_status(status: &'static str) {
    CAPTURE_STATUS.with(|s| s.set(status));
}
unsafe fn capture_with_timeout(config: &Config, timeout: f32) -> Option<Context> {
    capture_selected(config, timeout, false)
}
unsafe fn capture_selected(
    config: &Config,
    timeout: f32,
    allow_selection: bool,
) -> Option<Context> {
    capture_mode(config, timeout, allow_selection, false)
}
unsafe fn capture_mode(
    config: &Config,
    timeout: f32,
    allow_selection: bool,
    manual_request: bool,
) -> Option<Context> {
    capture_observation(config, timeout, allow_selection, manual_request, false).0
}
unsafe fn capture_observation(
    config: &Config,
    timeout: f32,
    allow_selection: bool,
    manual_request: bool,
    allow_empty_history: bool,
) -> (Option<Context>, Option<history::EmptyBaseline>) {
    let captured = capture_inner(config, timeout, allow_selection, manual_request, allow_empty_history);
    let mut empty = None;
    let result = match captured {
        Some(CapturedField::Editor(ctx)) => Some(*ctx),
        Some(CapturedField::EmptyHistory(baseline)) => { empty = Some(baseline); None },
        None => None,
    }.filter(|ctx| {
        // Some editors expose an off-screen input proxy with valid text and a
        // formally valid caret rectangle. Reject it before inference, optional
        // context collection, or insertion; overlay clipping alone is too late.
        // A fresh visible snapshot naturally resumes the normal debounce path.
        if !context_is_visible(ctx) {
            capture_status("Caret outside visible editor");
            return false;
        }
        true
    });
    if result.is_none() {
        code_editor::revoke();
        cells::revoke();
    }
    (result, empty)
}
unsafe fn capture_inner(
    config: &Config,
    timeout: f32,
    allow_selection: bool,
    manual_request: bool,
    allow_empty_history: bool,
) -> Option<CapturedField> {
    CAPTURE_DETAIL.with(|d| *d.borrow_mut() = CaptureDetail::default());
    capture_status("Suggestions paused");
    if !config.enabled {
        terminal::revoke();
        code_editor::revoke();
        cells::revoke();
        return None;
    }
    capture_status("Accessibility permission needed");
    if !AXIsProcessTrusted() {
        terminal::revoke();
        code_editor::revoke();
        cells::revoke();
        return None;
    }
    capture_status("Secure input active");
    if IsSecureEventInputEnabled() {
        terminal::revoke();
        code_editor::revoke();
        cells::revoke();
        return None;
    }
    capture_status("Current app excluded");
    let (pid, app) = front_identity();
    terminal::front_changed(pid, &app);
    if app.is_empty() || app == "org.libretypist.app" || !config.allows_app(&app) {
        terminal::revoke();
        code_editor::revoke();
        cells::revoke();
        return None;
    }
    let system = OwnedCf(AXUIElementCreateSystemWide());
    // The system-wide element sets the AX messaging timeout for this process.
    // Unresponsive editors must not block the event tap for the OS default timeout.
    AXUIElementSetMessagingTimeout(system.0, timeout);
    capture_status("Focused control unavailable");
    let focused = attr(system.0, "AXFocusedUIElement")?;
    let mut element = focused.clone();
    capture_status("Focus does not match foreground app");
    let mut element_pid = 0;
    if AXUIElementGetPid(element.0, &mut element_pid) != 0 || element_pid != pid {
        return None;
    }
    if crate::terminal::supported(&app) {
        code_editor::revoke();
        cells::revoke();
        return terminal::capture(pid, app, focused, manual_request).map(|ctx| CapturedField::Editor(Box::new(ctx)));
    }
    capture_status("Control role unavailable");
    let mut role = string(attr(element.0, "AXRole")?.0)?;
    CAPTURE_DETAIL.with(|d| d.borrow_mut().role = role.clone());
    if !matches!(
        role.as_str(),
        "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSecureTextField"
    ) {
        // Rich editors can focus a text leaf. Use only the declared editable
        // ancestor, never search unrelated children or conversation messages.
        capture_status("Editable ancestor unavailable");
        let editor = attr(element.0, "AXEditableAncestor")?;
        let mut editor_pid = 0;
        if AXUIElementGetPid(editor.0, &mut editor_pid) != 0 || editor_pid != pid {
            return None;
        }
        element = editor;
        role = string(attr(element.0, "AXRole")?.0)?;
        CAPTURE_DETAIL.with(|d| d.borrow_mut().role = role.clone());
    }
    capture_status("Control security metadata unavailable");
    let subrole = attr_checked(element.0, "AXSubrole")
        .and_then(|value| string(value.0).ok_or(-25200));
    // Chromium uses a nil subrole for ordinary inputs. Its password controls
    // expose AXSecureTextField and do not expose AXVisibleCharacterRange.
    // Require both that known provider and an unprotected range, never treating
    // a messaging error or an unsupported attribute as evidence of safety.
    let ordinary_chromium = subrole == Err(-25212) && role == "AXTextField"
        && crate::website::is_chromium_browser(&app)
        && unprotected_visible_range(element.0);
    match text_security(&role, subrole.as_deref().map_err(|e| *e), ordinary_chromium) {
        TextSecurity::Unknown => return None,
        TextSecurity::Protected => {
            capture_status("Secure text field");
            return None;
        }
        TextSecurity::Unprotected => {}
    }
    // Only explicit text-entry roles, never arbitrary window or web-page text.
    capture_status("Focus is not an editable text field");
    if !matches!(role.as_str(), "AXTextField" | "AXTextArea" | "AXComboBox") {
        return None;
    }
    capture_status("Code-editor field requires manual activation");
    let editor_policy = code_editor::policy(config, &app, pid, &focused, &element, manual_request);
    if !editor_policy.allowed {
        return None;
    }
    // Electron desktop apps can expose internal file:// web areas. Website
    // rules apply to recognized browsers, not to those desktop app internals.
    let spreadsheet_address = cells::spreadsheet_editor(&app, &element);
    let needs_website = (spreadsheet_address.is_some()
        || !config.website_rules.is_empty()
        || config.background.clipboard
        || config.background.screen
        || config.history.recording
        || config.history.strength != crate::history::Strength::Off)
        && crate::website::is_browser(&app);
    let website = if !needs_website {
        None
    } else {
        capture_status("Website identity unavailable");
        let host = website_for_element(&focused, pid, timeout).ok().flatten();
        if (!config.website_rules.is_empty() || spreadsheet_address.is_some()) && host.is_none() {
            return None;
        }
        host
    };
    capture_status("Current website excluded");
    if !config.allows_context(&app, website.as_deref()) {
        return None;
    }
    capture_status("Editor does not expose a writable text control");
    let value_writable = is_settable(element.0, "AXValue");
    let mut insertion = insertion_mode(
        is_settable(element.0, "AXSelectedText"),
        value_writable,
        bool_attr(element.0, "AXEnabled").unwrap_or(true),
        crate::website::is_browser(&app),
    )?;
    capture_status("Selection range unavailable");
    let selected = attr(element.0, "AXSelectedTextRange")?;
    let mut range = Range {
        location: 0,
        length: 0,
    };
    capture_status("Text is selected or selection is invalid");
    if !ax_value(selected.0, 4, &mut range)
        || range.location < 0
        || range.length < 0
        || (!allow_selection && range.length != 0)
    {
        return None;
    }
    capture_status("Editor text unavailable");
    let value = attr(element.0, "AXValue")?;
    let text = string(value.0)?;
    let units: Vec<u16> = text.encode_utf16().collect();
    let pos = range.location as usize;
    let selection_end = pos.checked_add(range.length as usize)?;
    if selection_end > units.len() {
        return None;
    }
    let prefix = String::from_utf16(&units[..pos]).ok()?;
    let suffix = String::from_utf16(&units[selection_end..]).ok()?;
    capture_status("Caret bounds unavailable");
    let name = cfstr("AXBoundsForRange");
    let mut caret = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    let mut exact_caret = false;
    let bounds = parameterized(element.0, "AXBoundsForRange", selected.0);
    CAPTURE_DETAIL.with(|d| d.borrow_mut().range_bounds = bounds.as_ref().err().copied());
    if let Ok(bounds) = bounds {
        exact_caret = ax_value(bounds.0, 3, &mut caret) && valid_rect(caret);
    }
    if exact_caret {
        CAPTURE_DETAIL.with(|d| d.borrow_mut().caret_source = "range");
    } else if let Some(rect) = marker_caret(element.0, range.length) {
        caret = rect;
        exact_caret = true;
        CAPTURE_DETAIL.with(|d| d.borrow_mut().caret_source = "text marker");
        // Chromium rich editors advertise AXSelectedText as settable, but
        // that setter can acknowledge without changing the DOM editor.
        // Use the already-validated writable control's text input path.
        if value_writable {
            insertion = InsertionMode::UnicodeEvent;
        }
    }
    // Ghost text must have a real insertion point; a field-level popup fallback
    // would obscure existing text and appear disconnected from the typed line.
    // Some standard editors do not return bounds for a zero-length range.
    // An ASCII character immediately before the caret provides its trailing
    // edge without guessing from the entire field or changing the selection.
    if !exact_caret && prefix.is_ascii() && !prefix.ends_with(['\n', '\r', '\t']) && pos > 0 {
        let previous = Range {
            location: range.location - 1,
            length: 1,
        };
        let parameter = OwnedCf(AXValueCreate(4, &previous as *const Range as *const c_void));
        let mut value = ptr::null();
        if AXUIElementCopyParameterizedAttributeValue(element.0, name.0, parameter.0, &mut value)
            == 0
            && !value.is_null()
        {
            let value = OwnedCf(value);
            if ax_value(value.0, 3, &mut caret) && valid_rect(caret) {
                caret.origin.x += caret.size.width;
                caret.size.width = 0.;
                exact_caret = true;
                CAPTURE_DETAIL.with(|d| d.borrow_mut().caret_source = "previous glyph");
            }
        }
    }
    if !exact_caret {
        // Empty Chromium controls may expose text/selection but no caret bounds.
        // Preserve only their verified empty baseline for history. This separate
        // type cannot enter completion, context collection, display or insertion.
        if allow_empty_history && crate::website::is_browser(&app) && text.is_empty() {
            let field = element_bounds(element.0)?;
            if empty_history_on_screen(&text, range, field)
                && capture_focus_unchanged(system.0, &focused, pid, &app, needs_website, &website, timeout)
            {
                capture_status("Empty editor history baseline; caret unavailable");
                return Some(CapturedField::EmptyHistory(history::EmptyBaseline {
                    pid, element, focused, app, website,
                }));
            }
        }
        return None;
    }
    capture_status("Text field bounds unavailable");
    let mut field_bounds = element_bounds(element.0)?;
    if let Some(viewport) = editor_policy.viewport {
        if let Some(expanded) = code_editor::display_bounds(field_bounds, caret, viewport) {
            field_bounds = expanded;
            CAPTURE_DETAIL.with(|d| {
                d.borrow_mut().code_editor_viewport = Some([
                    viewport.origin.x,
                    viewport.origin.y,
                    viewport.size.width,
                    viewport.size.height,
                ])
            });
        }
    }
    if app == "com.apple.TextEdit" {
        // TextEdit can return the previous line's Y for an empty range. Prefer
        // an adjacent glyph's line when its trailing edge agrees with the caret.
        let mut glyph = None;
        if prefix.is_ascii() && pos > 0 && !prefix.ends_with(['\n', '\r', '\t']) {
            let previous = Range {
                location: range.location - 1,
                length: 1,
            };
            let parameter = OwnedCf(AXValueCreate(4, &previous as *const Range as *const c_void));
            let mut value = ptr::null();
            if AXUIElementCopyParameterizedAttributeValue(
                element.0,
                name.0,
                parameter.0,
                &mut value,
            ) == 0
                && !value.is_null()
            {
                let value = OwnedCf(value);
                let mut rect = caret;
                if ax_value(value.0, 3, &mut rect) && valid_rect(rect) {
                    glyph = Some(rect);
                }
            }
        }
        caret = textedit_caret(caret, field_bounds, glyph, !prefix.contains(['\n', '\r']));
    }
    capture_status("Compact table cell requires manual activation");
    if !cells::allows(
        pid,
        &focused,
        &element,
        field_bounds,
        caret,
        manual_request,
        cells::scoped_spreadsheet_address(spreadsheet_address.as_deref(), website.as_deref()),
    ) {
        return None;
    }
    // Reading AX attributes crosses process boundaries. Reject a snapshot if focus
    // or secure-input state changed while those messages were in flight.
    capture_status("Focus changed during capture");
    if !capture_focus_unchanged(system.0, &focused, pid, &app, needs_website, &website, timeout) {
        return None;
    }
    capture_status("Ready");
    Some(CapturedField::Editor(Box::new(Context {
        element,
        focused,
        insertion,
        pid,
        app,
        website,
        text,
        prefix,
        suffix,
        range,
        caret,
        field_bounds,
        terminal: None,
    })))
}

#[derive(Debug, PartialEq, Eq)]
enum TextSecurity { Unprotected, Protected, Unknown }

fn text_security(role: &str, subrole: Result<&str, i32>, ordinary_chromium: bool) -> TextSecurity {
    if role == "AXSecureTextField" || subrole == Ok("AXSecureTextField") {
        return TextSecurity::Protected;
    }
    match subrole {
        Ok(_) => TextSecurity::Unprotected,
        Err(-25205 | -25212) if role == "AXTextArea" => TextSecurity::Unprotected,
        Err(-25212) if role == "AXTextField" && ordinary_chromium => TextSecurity::Unprotected,
        _ => TextSecurity::Unknown,
    }
}

unsafe fn unprotected_visible_range(element: AX) -> bool {
    let Some(value) = attr(element, "AXVisibleCharacterRange") else { return false; };
    let mut range = Range { location: 0, length: 0 };
    ax_value(value.0, 4, &mut range) && range.location >= 0 && range.length >= 0
        && range.location.checked_add(range.length).is_some()
}

unsafe fn element_bounds(element: AX) -> Option<NSRect> {
    let position = attr(element, "AXPosition")?;
    let size = attr(element, "AXSize")?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    (ax_value(position.0, 1, &mut rect.origin)
        && ax_value(size.0, 2, &mut rect.size)
        && valid_rect(rect)).then_some(rect)
}

unsafe fn capture_focus_unchanged(system: AX, focused: &OwnedCf, pid: i32, app: &str,
    needs_website: bool, website: &Option<String>, timeout: f32) -> bool {
    let Some(final_focus) = attr(system, "AXFocusedUIElement") else { return false };
    !IsSecureEventInputEnabled()
        && front_identity() == (pid, app.to_owned())
        && CFEqual(focused.0, final_focus.0)
        && (!needs_website || website_for_element(focused, pid, timeout).ok().flatten() == *website)
}

fn empty_history_visible(text: &str, range: Range, field: NSRect, screen: NSRect, primary_top: f64) -> bool {
    let bottom = primary_top - field.origin.y - field.size.height;
    text.is_empty() && range.location == 0 && range.length == 0
        && valid_rect(field) && field.size.width > 0. && valid_rect(screen)
        && primary_top.is_finite() && bottom.is_finite()
        && field.origin.x >= screen.origin.x
        && field.origin.x + field.size.width <= screen.origin.x + screen.size.width
        && bottom >= screen.origin.y
        && bottom + field.size.height <= screen.origin.y + screen.size.height
}

unsafe fn empty_history_on_screen(text: &str, range: Range, field: NSRect) -> bool {
    let screens: id = msg_send![class!(NSScreen), screens];
    let count: usize = msg_send![screens, count];
    if count == 0 { return false; }
    let primary: id = msg_send![screens, objectAtIndex:0usize];
    let primary: NSRect = msg_send![primary, frame];
    (0..count).any(|index| {
        let screen: id = msg_send![screens, objectAtIndex:index];
        let screen: NSRect = msg_send![screen, frame];
        empty_history_visible(text, range, field, screen, primary.origin.y + primary.size.height)
    })
}

struct Reply {
    id: u64,
    result: Result<String, String>,
}

unsafe fn set_selection(ctx: &Context, range: Range) -> bool {
    if IsSecureEventInputEnabled()
        || front_identity() != (ctx.pid, ctx.app.clone())
        || !is_settable(ctx.element.0, "AXSelectedTextRange")
    {
        return false;
    }
    let system = OwnedCf(AXUIElementCreateSystemWide());
    if attr(system.0, "AXFocusedUIElement").is_none_or(|focus| !CFEqual(focus.0, ctx.focused.0)) {
        return false;
    }
    let name = cfstr("AXSelectedTextRange");
    let value = OwnedCf(AXValueCreate(4, &range as *const Range as *const c_void));
    !value.0.is_null() && AXUIElementSetAttributeValue(ctx.element.0, name.0, value.0) == 0
}
// Only before posting replacement text, and only if our unchanged selection is
// still present. Never undo a user's changed selection or retry a posted edit.
unsafe fn restore_owned_selection(
    config: &Config,
    original: &Context,
    plan: &crate::acceptance::ReplacementPlan,
) {
    if let Some(ctx) = capture_selected(config, 0.012, true) {
        if ctx.pid == original.pid
            && ctx.app == original.app
            && ctx.website == original.website
            && CFEqual(ctx.element.0, original.element.0)
            && CFEqual(ctx.focused.0, original.focused.0)
            && plan.selection_matches(
                &ctx.text,
                ctx.range.location as usize,
                ctx.range.length as usize,
            )
        {
            set_selection(&ctx, original.range);
        }
    }
}

fn textedit_caret(
    mut caret: NSRect,
    field: NSRect,
    glyph: Option<NSRect>,
    first_line: bool,
) -> NSRect {
    if let Some(glyph) = glyph {
        let same_edge = (glyph.origin.x + glyph.size.width - caret.origin.x).abs() <= 2.;
        let nearby_line = (glyph.origin.y - caret.origin.y).abs() <= caret.size.height + 2.;
        let inside = glyph.origin.y >= field.origin.y
            && glyph.origin.y + glyph.size.height <= field.origin.y + field.size.height;
        if same_edge && nearby_line && inside {
            caret.origin.y = glyph.origin.y;
            caret.size.height = glyph.size.height;
            return caret;
        }
    }
    // Observed on TextEdit's first line: AX gives (y=150,h=14) while
    // AXPosition says the text view starts at y=164. Correct only that exact
    // one-line offset, never arbitrary offscreen/scrolled-out caret bounds.
    if first_line
        && caret.origin.y < field.origin.y
        && (caret.origin.y + caret.size.height - field.origin.y).abs() <= 1.
    {
        caret.origin.y = field.origin.y;
    }
    caret
}

// AX coordinates start at the primary display's top-left; AppKit starts at
// its bottom-left. Keep the caret anchor fixed, including on secondary displays.
fn caret_is_visible(caret: NSRect, field: NSRect, screen: NSRect, primary_top: f64) -> bool {
    let bottom = primary_top - caret.origin.y - caret.size.height;
    valid_rect(caret)
        && valid_rect(field)
        // A caret can be a zero-width insertion point, but a zero-width
        // editor is a hidden proxy, even when its coordinates are on-screen.
        && field.size.width > 0.
        && valid_rect(screen)
        && primary_top.is_finite()
        && bottom.is_finite()
        && caret.origin.x >= field.origin.x
        && caret.origin.x <= field.origin.x + field.size.width
        && caret.origin.y >= field.origin.y
        && caret.origin.y + caret.size.height <= field.origin.y + field.size.height
        && caret.origin.x >= screen.origin.x
        && caret.origin.x < screen.origin.x + screen.size.width
        && bottom >= screen.origin.y
        && bottom + caret.size.height <= screen.origin.y + screen.size.height
}

unsafe fn context_is_visible(ctx: &Context) -> bool {
    let screens: id = msg_send![class!(NSScreen), screens];
    let count: usize = msg_send![screens, count];
    if count == 0 {
        return false;
    }
    let primary: id = msg_send![screens, objectAtIndex:0usize];
    let primary: NSRect = msg_send![primary, frame];
    (0..count).any(|index| {
        let screen: id = msg_send![screens, objectAtIndex:index];
        // Use the physical frame, including the full-screen Dock reservation.
        let screen: NSRect = msg_send![screen, frame];
        caret_is_visible(
            ctx.caret,
            ctx.field_bounds,
            screen,
            primary.origin.y + primary.size.height,
        )
    })
}

fn ghost_frame(caret: NSRect, field: NSRect, screen: NSRect, primary_top: f64) -> Option<NSRect> {
    if !caret_is_visible(caret, field, screen, primary_top) {
        return None;
    }
    let x = caret.origin.x;
    let bottom = primary_top - caret.origin.y - caret.size.height;
    let field_right = field.origin.x + field.size.width;
    // Reserve a little trailing room instead of drawing over the editor border.
    let width = (field_right.min(screen.origin.x + screen.size.width) - x - 4.).min(1200.);
    (width >= 8.).then(|| {
        NSRect::new(
            NSPoint::new(x, bottom),
            NSSize::new(width, caret.size.height),
        )
    })
}

fn fitting_completion(text: &str, width: f64, measure: impl Fn(&str) -> f64) -> String {
    let text = text.trim_end();
    if measure(text) <= width {
        return text.to_owned();
    }
    // Offer only whole visible words. Shift-Tab must never insert a hidden tail.
    let mut end = 0;
    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() && !text[..i].trim().is_empty() {
            if measure(&text[..i]) > width {
                break;
            }
            end = i;
        }
    }
    text[..end].trim_end().to_owned()
}

fn completion_frame(caret: NSRect, field: NSRect, screen: NSRect, primary_top: f64,
    text: &str, measure: impl Fn(&str) -> f64) -> Option<(NSRect, bool)> {
    if let Some(frame) = ghost_frame(caret, field, screen, primary_top) {
        if !fitting_completion(text, frame.size.width, &measure).trim().is_empty() {
            return Some((frame, false));
        }
    }
    // A non-interactive preview preserves access to an otherwise invisible
    // completion at the right edge, without inventing the editor's wrap point.
    midline_frame(caret, field, screen, primary_top).map(|frame| (frame, true))
}

fn midline_frame(caret: NSRect, field: NSRect, screen: NSRect, primary_top: f64) -> Option<NSRect> {
    if !caret_is_visible(caret, field, screen, primary_top) {
        return None;
    }
    let bottom = primary_top - caret.origin.y - caret.size.height;
    let width = field
        .size
        .width
        .clamp(120., 480.)
        .min(screen.size.width - 16.);
    let height = caret.size.height + 8.;
    if width < 16. || height > screen.size.height - 8. {
        return None;
    }
    let x = caret.origin.x.clamp(
        screen.origin.x + 8.,
        screen.origin.x + screen.size.width - width - 8.,
    );
    let above = bottom + caret.size.height + 4.;
    let y = if above + height <= screen.origin.y + screen.size.height - 4. {
        above
    } else {
        bottom - height - 4.
    };
    if y < screen.origin.y + 4. {
        return None;
    }
    Some(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)))
}

fn fitting_midline(text: &str, width: f64, measure: impl Fn(&str) -> f64) -> String {
    let mut fitted = fitting_completion(text, width, measure);
    if !fitted.is_empty() {
        // Preserve the separator needed before the existing suffix. It is part
        // of the insertion even though the normal ghost fitter trims its ink.
        let separator: String = text[fitted.len()..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        fitted.push_str(&separator);
    }
    fitted
}

extern "C" fn draw_ghost(view: &Object, _: Sel, _: NSRect) {
    GHOST_DRAWS.with(|count| count.set(count.get() + 1));
    unsafe {
        let text: id = *view.get_ivar("ghostText");
        if text != nil {
            let bounds: NSRect = msg_send![view, bounds];
            let size: NSSize = msg_send![text, size];
            // No NSTextField padding, background, border, or shortcut footer.
            let inset: f64 = *view.get_ivar("ghostInset");
            let origin = NSPoint::new(inset, (bounds.size.height - size.height) / 2.);
            let _: () = msg_send![text, drawAtPoint:origin];
        }
    }
}
extern "C" fn drop_ghost(view: &mut Object, _: Sel) {
    unsafe {
        let text: id = *view.get_ivar("ghostText");
        let _: () = msg_send![text, release];
        let _: () = msg_send![super(view, class!(NSView)), dealloc];
    }
}
unsafe fn ghost_view() -> id {
    let mut decl = ClassDecl::new("LibreTypistGhostView", class!(NSView)).unwrap();
    decl.add_ivar::<id>("ghostText");
    decl.add_ivar::<f64>("ghostInset");
    decl.add_method(
        sel!(drawRect:),
        draw_ghost as extern "C" fn(&Object, Sel, NSRect),
    );
    decl.add_method(sel!(dealloc), drop_ghost as extern "C" fn(&mut Object, Sel));
    let view: id = msg_send![decl.register(), new];
    (*view).set_ivar("ghostText", nil);
    (*view).set_ivar("ghostInset", 0f64);
    view
}

struct State {
    config: Config,
    target: id,
    status: id,
    menu: id,
    pause_item: id,
    app_item: id,
    pause_app_item: id,
    health_item: id,
    panel: id,
    ghost: id,
    settings: SettingsUi,
    keyboard: keyboard::KeyboardUi,
    history: history::Runtime,
    model_setup: models::ModelUi,
    statistics: statistics::Runtime,
    startup: startup::StartupUi,
    websites: websites::WebsiteUi,
    website_preview: bool,
    background_ui: background::BackgroundUi,
    screen: screen::Engine,
    background_preview: bool,
    spelling: spelling::SpellingUi,
    replacement: Option<ReplacementPlan>,
    replacement_actionable: bool,
    emoji: emoji::EmojiUi,
    emoji_choices: Option<crate::emoji::Choices>,
    choice_ui: choices::ChoiceUi,
    word_choices: Option<crate::alternatives::Choices>,
    choice_progress: choices::Progress,
    preserving_remainder: bool,
    choice_preview_request: Option<crate::alternatives::Request>,
    choice_requests: mpsc::SyncSender<(Config, crate::alternatives::Request)>,
    choice_replies: mpsc::Receiver<choices::Reply>,
    paused_apps: HashMap<String, Instant>,
    dismissal: Dismissal,
    dismissed_field: Option<(i32, OwnedCf)>,
    word_shortcut_drafts: HashMap<String, bool>,
    code_editor_drafts: HashMap<String, bool>,
    append_instruction_drafts: HashMap<String, bool>,
    pending_control: Option<(Action, i32, String)>,
    settings_apps: Vec<String>,
    editing_app: String,
    instruction_drafts: HashMap<String, String>,
    context: Option<Context>,
    changed: Instant,
    requested: bool,
    request_profile_attached: Option<bool>,
    personalized_requests: u64,
    suggestion: String,
    epoch: Arc<AtomicU64>,
    requests: mpsc::SyncSender<(Config, CompletionRequest)>,
    replies: mpsc::Receiver<Reply>,
    tap: CF,
    suppressed_keys: u128,
    tab_bypassed: bool,
    models: Vec<String>,
    status_text: String,
    last_app: String,
    last_app_name: String,
    preview_epoch: Option<u64>,
    preview_context: Option<(String, String)>,
    preview_results: mpsc::Receiver<Result<Vec<String>, String>>,
    models_sender: mpsc::Sender<Result<Vec<String>, String>>,
    last_diagnostic: Instant,
    textedit_diagnostic: serde_json::Value,
    pipeline_status: &'static str,
    accessibility_pid: i32,
    accessibility_bridge: (i32, i32),
    signal_diagnostic: serde_json::Value,
    chrome_diagnostic: serde_json::Value,
    pending_input: VecDeque<PendingInput>,
    last_accept: String,
    insertion: Option<PendingInsertion>,
    replacement_selection: Option<PendingReplacementSelection>,
    last_accept_units: usize,
    last_retained_units: usize,
    chunks_verified: usize,
}
fn emoji_navigation(key: i64, flags: u64) -> Option<bool> {
    if flags & MODIFIERS != 0 {
        return None;
    }
    match key {
        125 => Some(true),
        126 => Some(false),
        _ => None,
    }
}
enum PendingInput {
    Accept(bool),
    EmojiMove(bool),
    Choose(usize),
}
struct PendingInsertion {
    original: Context,
    plan: AcceptancePlan,
    posted: Instant,
}
struct PendingReplacementSelection {
    original: Context,
    replacement: ReplacementPlan,
    plan: AcceptancePlan,
    requested: Instant,
}
thread_local! { static STATE:RefCell<Option<State>>=const {RefCell::new(None)}; }

impl State {
    unsafe fn try_emoji(&mut self) -> bool {
        if self
            .context
            .as_ref()
            .is_some_and(|ctx| ctx.terminal.is_some())
            || !self.config.emoji
        {
            return false;
        }
        let Some(ctx) = self.context.clone() else {
            return false;
        };
        let Some(query) = crate::emoji::Query::parse(&ctx.prefix, &ctx.suffix) else {
            return false;
        };
        // A recognized shortcode never goes to the language model, including
        // zero matches and editors without verifiable selection replacement.
        self.requested = true;
        self.emoji_choices = Some(crate::emoji::Choices::new(query));
        self.show_emoji();
        true
    }
    unsafe fn show_emoji(&mut self) {
        let Some(ctx) = &self.context else {
            return;
        };
        let Some(choices) = &self.emoji_choices else {
            return;
        };
        self.replacement = choices.plan(&ctx.prefix, &ctx.suffix);
        self.replacement_actionable =
            self.replacement.is_some() && is_settable(ctx.element.0, "AXSelectedTextRange");
        self.suggestion = if self.replacement_actionable {
            choices.display()
        } else {
            String::new()
        };
        if self.suggestion.is_empty() {
            self.pipeline_status = "No emoji match or selection replacement unavailable";
            let _: () = msg_send![self.panel,orderOut:nil];
        } else {
            self.overlay();
        }
    }
    unsafe fn move_emoji(&mut self, next: bool) {
        let valid = match (&self.context, capture(&self.config)) {
            (Some(previous), Some(current)) => previous.same(&current),
            _ => false,
        };
        if !valid {
            self.clear();
            return;
        }
        if let Some(choices) = self.emoji_choices.as_mut() {
            choices.step(next);
        }
        self.show_emoji();
    }
    unsafe fn try_spelling(&mut self) -> bool {
        if self
            .context
            .as_ref()
            .is_some_and(|ctx| ctx.terminal.is_some())
        {
            return false;
        }
        if !self.config.spelling.enabled {
            return false;
        }
        let Some(ctx) = self.context.clone() else {
            return false;
        };
        let Some(correction) =
            spelling::suggest(&ctx.prefix, &ctx.suffix, &self.config.spelling.language)
        else {
            return false;
        };
        if capture(&self.config).is_none_or(|current| !ctx.same(&current)) {
            self.clear();
            return true;
        }
        self.requested = true;
        self.replacement_actionable =
            self.config.spelling.show_fixes && is_settable(ctx.element.0, "AXSelectedTextRange");
        self.suggestion = if self.replacement_actionable {
            format!(" → {}", correction.corrected)
        } else {
            " typo?".into()
        };
        self.replacement = Some(correction.plan);
        self.overlay();
        true
    }
    unsafe fn accept_replacement(&mut self) -> bool {
        if !self.replacement_actionable || self.suggestion.is_empty() {
            return false;
        }
        let Some(replacement) = self.replacement.clone() else {
            return false;
        };
        let Some(current) = capture_selected(&self.config, 0.08, true) else {
            self.clear();
            return false;
        };
        if self.context.as_ref().is_none_or(|old| !old.same(&current)) {
            self.clear();
            return false;
        }
        let Some(plan) = replacement.insertion(current.insertion == InsertionMode::UnicodeEvent)
        else {
            return false;
        };
        let range = Range {
            location: replacement.start_utf16 as isize,
            length: replacement.length_utf16 as isize,
        };
        if current.range != range && !set_selection(&current, range) {
            self.clear();
            self.last_accept = "Replacement cancelled: selection unavailable".into();
            return false;
        }
        self.replacement_selection = Some(PendingReplacementSelection {
            original: current,
            replacement,
            plan,
            requested: Instant::now(),
        });
        self.last_accept_units = 0;
        self.last_retained_units = 0;
        self.chunks_verified = 0;
        self.emoji_choices = None;
        self.replacement = None;
        self.replacement_actionable = false;
        self.suggestion.clear();
        self.requested = true;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let _: () = msg_send![self.panel,orderOut:nil];
        self.last_accept = "Waiting for replacement selection".into();
        self.pipeline_status = "Waiting for replacement selection";
        true
    }
    unsafe fn advance_replacement_selection(&mut self, current: Option<&Context>) {
        let Some(mut pending) = self.replacement_selection.take() else {
            return;
        };
        let original = &pending.original;
        let selected = current.filter(|ctx| {
            ctx.pid == original.pid
                && ctx.app == original.app
                && ctx.insertion == original.insertion
                && ctx.website == original.website
                && CFEqual(ctx.element.0, original.element.0)
                && CFEqual(ctx.focused.0, original.focused.0)
        });
        let observation = selected.map_or(Observation::Diverged, |ctx| {
            pending.replacement.observe_selection(
                &ctx.text,
                (ctx.range.location as usize, ctx.range.length as usize),
                (
                    original.range.location as usize,
                    original.range.length as usize,
                ),
                pending.requested.elapsed() >= Duration::from_secs(1),
            )
        });
        match observation {
            Observation::Waiting => self.replacement_selection = Some(pending),
            Observation::Diverged => {
                restore_owned_selection(&self.config, original, &pending.replacement);
                self.clear();
                self.last_accept = "Replacement cancelled: selection changed or timed out".into();
            }
            Observation::Confirmed => {
                let selected = selected.unwrap();
                let Some(chunk) = pending.plan.next_chunk() else {
                    self.clear();
                    return;
                };
                if !self.post_chunk(selected, &chunk) {
                    restore_owned_selection(&self.config, original, &pending.replacement);
                    self.clear();
                    return;
                }
                self.last_accept_units = pending.plan.accepted_utf16_units;
                self.insertion = Some(PendingInsertion {
                    original: selected.clone(),
                    plan: pending.plan,
                    posted: Instant::now(),
                });
                self.last_accept = "Replacement posted; awaiting verification".into();
                self.pipeline_status = "Verifying replacement insertion";
            }
        }
    }
    fn toggle_app_pause(&mut self, app: &str) {
        if self.paused_apps.remove(app).is_none() {
            self.paused_apps
                .insert(app.into(), Instant::now() + Duration::from_secs(300));
        }
    }
    unsafe fn process_control(&mut self) {
        let Some((action, pid, app)) = self.pending_control.take() else {
            return;
        };
        if front_identity() != (pid, app.clone()) || IsSecureEventInputEnabled() {
            return;
        }
        match action {
            Action::ToggleGlobal => {
                self.config.enabled = !self.config.enabled;
                self.clear();
                self.save();
            }
            Action::PauseApp if self.config.allows_app(&app) => {
                self.toggle_app_pause(&app);
                self.clear();
            }
            Action::Choices if self.config.allows_app(&app) => {
                self.paused_apps.remove(&app);
                self.dismissal.reset();
                self.dismissed_field = None;
                self.prepare_accessibility();
                let context = capture_selected(&self.config, 0.08, true);
                self.clear();
                self.context = context;
                self.request_word_choices();
            }
            Action::Manual if self.config.allows_app(&app) => {
                self.paused_apps.remove(&app);
                self.dismissal.reset();
                self.dismissed_field = None;
                self.prepare_accessibility();
                let context = capture_mode(&self.config, 0.08, false, true);
                self.clear();
                self.context = context;
                self.changed = Instant::now() - Duration::from_millis(self.config.delay_ms);
                self.pipeline_status = "Manual completion requested";
            }
            _ => {}
        }
    }

    unsafe fn remember_instruction_draft(&mut self) {
        if !self.editing_app.is_empty() {
            let append: isize = msg_send![self.settings.append_instructions, state];
            self.append_instruction_drafts
                .insert(self.editing_app.clone(), append == 1);
            let checked: isize = msg_send![self.settings.word_shortcut, state];
            self.word_shortcut_drafts
                .insert(self.editing_app.clone(), checked == 1);
        }
        if crate::code_editor::supported(&self.editing_app) {
            let checked: isize = msg_send![self.settings.code_editor, state];
            self.code_editor_drafts
                .insert(self.editing_app.clone(), checked == 1);
        }
        self.instruction_drafts.insert(
            self.editing_app.clone(),
            ns_text(msg_send![self.settings.instructions, string]),
        );
    }

    unsafe fn refresh_word_shortcut(&self) {
        let app = &self.editing_app;
        let append = self
            .append_instruction_drafts
            .get(app)
            .copied()
            .unwrap_or_else(|| self.config.app_appends_instructions(app));
        let _: () = msg_send![self.settings.append_instructions, setEnabled:if app.is_empty() { NO } else { YES }];
        let _: () = msg_send![self.settings.append_instructions, setState:if !app.is_empty() && append { 1isize } else { 0isize }];
        let code_enabled = self
            .code_editor_drafts
            .get(app)
            .copied()
            .unwrap_or_else(|| self.config.complete_in_code(app));
        let _: () = msg_send![self.settings.code_editor, setEnabled:if crate::code_editor::supported(app) { YES } else { NO }];
        let _: () = msg_send![self.settings.code_editor, setState:if code_enabled { 1isize } else { 0isize }];
        let enabled = self
            .word_shortcut_drafts
            .get(app)
            .copied()
            .unwrap_or_else(|| self.config.word_shortcut_enabled(app));
        let _: () = msg_send![self.settings.word_shortcut, setEnabled:if app.is_empty() { NO } else { YES }];
        let _: () =
            msg_send![self.settings.word_shortcut, setState:if enabled { 1isize } else { 0isize }];
    }
    fn instruction_draft(&self, app: &str) -> String {
        self.instruction_drafts
            .get(app)
            .cloned()
            .unwrap_or_else(|| {
                if app.is_empty() {
                    self.config.writing_instructions.clone()
                } else {
                    self.config
                        .app_rules
                        .get(app)
                        .map(|rule| rule.writing_instructions.clone())
                        .unwrap_or_default()
                }
            })
    }

    unsafe fn preview_instruction_config(&self) -> Config {
        let mut config = self.config.clone();
        let text = ns_text(msg_send![self.settings.instructions, string]);
        config.writing_instructions = if self.editing_app.is_empty() {
            text.clone()
        } else {
            self.instruction_draft("")
        };
        config.set_app_instructions(
            "org.libretypist.preview",
            if self.editing_app.is_empty() {
                String::new()
            } else {
                text
            },
        );
        let append: isize = msg_send![self.settings.append_instructions, state];
        config.set_app_append_instructions(
            "org.libretypist.preview",
            !self.editing_app.is_empty() && append == 1,
        );
        config
    }

    unsafe fn refresh_settings_apps(&mut self) {
        self.remember_instruction_draft();
        let mut apps = BTreeMap::new();
        for app in self
            .config
            .app_rules
            .keys()
            .chain(self.instruction_drafts.keys())
        {
            if !app.is_empty() {
                apps.insert(app.clone(), app.clone());
            }
        }
        if !self.last_app.is_empty() {
            apps.insert(self.last_app.clone(), self.last_app_name.clone());
        }
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let running: id = msg_send![workspace, runningApplications];
        let count: usize = msg_send![running, count];
        for i in 0..count {
            let app: id = msg_send![running, objectAtIndex:i];
            let policy: isize = msg_send![app, activationPolicy];
            let bundle = ns_text(msg_send![app, bundleIdentifier]);
            if policy == 0 && !bundle.is_empty() && bundle != "org.libretypist.app" {
                let name = ns_text(msg_send![app, localizedName]);
                apps.insert(bundle, name);
            }
        }
        let mut apps: Vec<_> = apps.into_iter().collect();
        apps.sort_by_key(|(bundle, name)| (name.to_lowercase(), bundle.clone()));
        let _: () = msg_send![self.settings.scope, removeAllItems];
        let title = ns("All apps (default instructions)");
        let _: () = msg_send![self.settings.scope, addItemWithTitle:title];
        let _: () = msg_send![title, release];
        self.settings_apps = vec![String::new()];
        for (bundle, name) in apps {
            let title = ns(&format!("{name} — {bundle}"));
            let _: () = msg_send![self.settings.scope, addItemWithTitle:title];
            let _: () = msg_send![title, release];
            self.settings_apps.push(bundle);
        }
        let index = self
            .settings_apps
            .iter()
            .position(|app| app == &self.editing_app)
            .unwrap_or(0);
        let _: () = msg_send![self.settings.scope, selectItemAtIndex:index as isize];
    }

    unsafe fn prepare_accessibility(&mut self) {
        if !self.config.enabled || !AXIsProcessTrusted() || IsSecureEventInputEnabled() {
            return;
        }
        let (pid, app) = front_identity();
        if pid == self.accessibility_pid
            || pid <= 0
            || app.is_empty()
            || !self.config.allows_app(&app)
        {
            return;
        }
        self.accessibility_pid = pid;
        let target = OwnedCf(AXUIElementCreateApplication(pid));
        AXUIElementSetMessagingTimeout(target.0, 0.08);
        // Documented assistive-technology handshake for Electron. Unsupported
        // attributes are ignored by native apps. Never toggle them back off:
        // another accessibility client may also depend on the expanded tree.
        let manual = cfstr("AXManualAccessibility");
        let enhanced = cfstr("AXEnhancedUserInterface");
        let first = AXUIElementSetAttributeValue(target.0, manual.0, kCFBooleanTrue);
        let second = if is_settable(target.0, "AXEnhancedUserInterface") {
            AXUIElementSetAttributeValue(target.0, enhanced.0, kCFBooleanTrue)
        } else {
            -25205
        };
        self.accessibility_bridge = (first, second);
    }
    fn app_allowed(&self, app: &str) -> bool {
        let mut config = self.config.clone();
        config.enabled = true;
        config.allows_app(app)
    }
    unsafe fn clear(&mut self) {
        self.pending_control = None;
        self.pending_input.clear();
        if self.replacement_selection.take().is_some() {
            self.last_accept = "Replacement selection cancelled".into();
        }
        if self.insertion.take().is_some() {
            self.last_accept = "Remaining insertion cancelled".into();
        }
        self.pipeline_status = "Waiting for typing";
        self.suggestion.clear();
        self.replacement = None;
        self.replacement_actionable = false;
        self.emoji_choices = None;
        self.word_choices = None;
        self.choice_progress = choices::Progress::Untried;
        self.preserving_remainder = false;
        self.choice_preview_request = None;
        self.context = None;
        self.requested = false;
        self.request_profile_attached = None;
        self.preview_epoch = None;
        self.website_preview = false;
        self.background_preview = false;
        self.screen.clear();
        self.preview_context = None;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let _: () = msg_send![self.panel,orderOut:nil];
    }
    unsafe fn save(&mut self) {
        if let Err(e) = self.config.save() {
            self.status_text = format!("Settings not saved: {e}");
        }
    }
    unsafe fn refresh_menu(&mut self) {
        set_title(
            self.pause_item,
            if self.config.enabled {
                "Pause suggestions"
            } else {
                "Resume suggestions"
            },
        );
        let current = front_app();
        if !current.is_empty() && current != "org.libretypist.app" {
            self.last_app = current;
            self.last_app_name = front_name();
        }
        set_title(
            self.app_item,
            &format!(
                "{} {}",
                if self.app_allowed(&self.last_app) {
                    "Disable in"
                } else {
                    "Enable in"
                },
                self.last_app_name
            ),
        );
        let tooltip = ns(&self.last_app);
        let _: () = msg_send![self.app_item, setToolTip:tooltip];
        let _: () = msg_send![tooltip, release];
        let _: () = msg_send![self.app_item,setEnabled:if self.last_app.is_empty(){NO}else{YES}];
        set_title(
            self.pause_app_item,
            &if self.paused_apps.contains_key(&self.last_app) {
                format!("Resume in {}", self.last_app_name)
            } else {
                format!("Pause in {} for 5 minutes", self.last_app_name)
            },
        );
        let _: () = msg_send![self.pause_app_item, setEnabled:if self.app_allowed(&self.last_app) { YES } else { NO }];
        let health = if !AXIsProcessTrusted() {
            "Accessibility permission needed"
        } else if self.tap.is_null() {
            "Keyboard shortcut permission needed"
        } else {
            &self.status_text
        };
        set_title(self.health_item, health);
        let button: id = msg_send![self.status, button];
        set_title(button, if self.config.enabled { "LT" } else { "LTⅡ" });
    }
    unsafe fn overlay(&mut self) {
        if self.word_choices.is_some() {
            self.overlay_word_choices();
            return;
        }
        let Some(ctx) = &self.context else { return };
        let mut midline = !ctx.suffix.trim().is_empty() || self.emoji_choices.is_some();
        let font = font::font(ctx);
        let attributes: id = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![attributes, setObject:font forKey:NSFontAttributeName];
        let measure = |text: &str| {
            let text = ns(text);
            let size: NSSize = msg_send![text, sizeWithAttributes:attributes];
            let _: () = msg_send![text, release];
            size.width
        };
        self.pipeline_status = "No display available";
        let screens: id = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        if count == 0 {
            self.suggestion.clear();
            let _: () = msg_send![self.panel, orderOut:nil];
            return;
        }
        let primary: id = msg_send![screens,objectAtIndex:0usize];
        let main_frame: NSRect = msg_send![primary, frame];
        let mut frame = None;
        let mut preview_screen = None;
        for i in 0..count {
            let screen: id = msg_send![screens,objectAtIndex:i];
            // visibleFrame excludes the Dock/menu bar's desktop reservation,
            // even when a full-screen editor legitimately occupies that area.
            // Clip against the physical display and the actual editor bounds.
            let sf: NSRect = msg_send![screen, frame];
            frame = if midline && ctx.terminal.is_some() {
                terminal::preview_frame(
                    ctx.caret,
                    ctx.field_bounds,
                    sf,
                    main_frame.origin.y + main_frame.size.height,
                )
            } else if midline {
                midline_frame(
                    ctx.caret,
                    ctx.field_bounds,
                    sf,
                    main_frame.origin.y + main_frame.size.height,
                )
            } else if ctx.terminal.is_none() {
                completion_frame(ctx.caret, ctx.field_bounds, sf,
                    main_frame.origin.y + main_frame.size.height, &self.suggestion, measure)
                    .map(|(frame, preview)| { midline = preview; frame })
            } else {
                ghost_frame(
                    ctx.caret,
                    ctx.field_bounds,
                    sf,
                    main_frame.origin.y + main_frame.size.height,
                )
            };
            if frame.is_some() {
                preview_screen = Some(sf);
                break;
            }
        }
        let Some(mut frame) = frame else {
            self.pipeline_status = "Caret outside visible field or no room on line";
            self.suggestion.clear();
            let _: () = msg_send![self.panel, orderOut:nil];
            return;
        };
        let color: id = if midline {
            msg_send![class!(NSColor), secondaryLabelColor]
        } else {
            msg_send![class!(NSColor), colorWithCalibratedWhite:0.5f64 alpha:0.75f64]
        };
        let background: id = if midline {
            msg_send![class!(NSColor), windowBackgroundColor]
        } else {
            msg_send![class!(NSColor), clearColor]
        };
        let _: () = msg_send![self.panel, setBackgroundColor:background];
        (*self.ghost).set_ivar("ghostInset", if midline { 4f64 } else { 0f64 });
        let _: () = msg_send![attributes, setObject:color forKey:NSForegroundColorAttributeName];
        let full_correction = self
            .replacement
            .as_ref()
            .filter(|_| self.emoji_choices.is_none())
            .map(|_| self.suggestion.clone());
        self.suggestion = if let Some(choices) = &self.emoji_choices {
            choices.fitting_display(frame.size.width - 8., measure)
        } else if midline {
            fitting_midline(&self.suggestion, frame.size.width - 8., measure)
        } else {
            fitting_completion(&self.suggestion, frame.size.width, measure)
        };
        if full_correction.is_some_and(|full| full != self.suggestion) {
            self.suggestion.clear();
            self.replacement_actionable = false;
        }
        if self.suggestion.trim().is_empty() {
            self.pipeline_status = "No complete suggestion word fits on line";
            let _: () = msg_send![self.panel, orderOut:nil];
            return;
        }
        frame.size.width = (measure(&self.suggestion).ceil() + if midline { 8. } else { 1. })
            .min(frame.size.width);
        if let Some(screen) = preview_screen.filter(|_| midline && ctx.terminal.is_none()) {
            frame.origin.x = ctx.caret.origin.x.clamp(
                screen.origin.x + 8.,
                screen.origin.x + screen.size.width - frame.size.width - 8.,
            );
        }
        let string = ns(&self.suggestion);
        let text: id = msg_send![class!(NSAttributedString), alloc];
        let text: id = msg_send![text, initWithString:string attributes:attributes];
        let _: () = msg_send![string, release];
        let previous: id = *(*self.ghost).get_ivar("ghostText");
        (*self.ghost).set_ivar("ghostText", text);
        let _: () = msg_send![previous, release];
        let _: () = msg_send![self.ghost, setFrame:NSRect::new(NSPoint::new(0.,0.), frame.size)];
        let _: () = msg_send![self.ghost, setNeedsDisplay:YES];
        let _: () = msg_send![self.panel,setFrame:frame display:YES];
        let _: () = msg_send![self.panel, orderFrontRegardless];
        self.pipeline_status = if self.emoji_choices.is_some() {
            "Emoji choice visible"
        } else if self.replacement.is_some() {
            "Typo correction visible"
        } else if midline {
            "Mid-line preview visible"
        } else {
            "Ghost text visible"
        };
    }
    unsafe fn write_diagnostic(&mut self) {
        if self.last_diagnostic.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_diagnostic = Instant::now();
        // A single overwritten health snapshot, never typed text, responses,
        // clipboard content, window titles, or a history of foreground apps.
        let visible: BOOL = msg_send![self.panel, isVisible];
        let capture = CAPTURE_STATUS.with(Cell::get);
        let snapshot = serde_json::json!({
            "capture": capture,
            "history": self.history.diagnostic(&self.config),
            "request_profile_attached": self.request_profile_attached,
            "personalized_requests": self.personalized_requests,
            "pipeline": self.pipeline_status,
            "accessibility": AXIsProcessTrusted(),
            "website_identified": self.context.as_ref().is_some_and(|c| c.website.is_some()),
            "terminal_prompt": self.context.as_ref().is_some_and(|c| c.terminal.is_some()),
            "keyboard_shortcuts": !self.tap.is_null(),
            "overlay_visible": visible == YES,
            "draw_count": GHOST_DRAWS.with(Cell::get),
            "ghost_font": FONT_DIAGNOSTIC.with(|d| {let (source, points) = d.get(); serde_json::json!({"source":source,"points":points})}),
            "caret": self.context.as_ref().map(|c| rect_coords(c.caret)),
            "field": self.context.as_ref().map(|c| rect_coords(c.field_bounds)),
            "insertion": self.context.as_ref().map(|c| format!("{:?}", c.insertion)),
            "accessibility_bridge": self.accessibility_bridge,
            "acceptance": self.last_accept,
            "accepted_utf16_units": self.last_accept_units,
            "retained_utf16_units": self.last_retained_units,
            "chunks_verified": self.chunks_verified,
            "editor": CAPTURE_DETAIL.with(|d| serde_json::to_value(&*d.borrow()).ok()),
        });
        if front_app() == "com.apple.TextEdit" {
            self.textedit_diagnostic = snapshot.clone();
        }
        if front_app() == "org.whispersystems.signal-desktop" {
            self.signal_diagnostic = snapshot.clone();
        }
        if front_app() == "com.google.Chrome" {
            self.chrome_diagnostic = snapshot.clone();
        }
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let directory = std::path::PathBuf::from(home).join("Library/Caches/LibreTypist");
        let data = serde_json::json!({
            "pid": std::process::id(),
            "current": snapshot,
            "last_textedit": self.textedit_diagnostic,
            "last_signal": self.signal_diagnostic,
            "last_chrome": self.chrome_diagnostic,
        });
        if std::fs::create_dir_all(&directory).is_ok() {
            let temporary = directory.join("status.tmp");
            if let Ok(bytes) = serde_json::to_vec_pretty(&data) {
                if std::fs::write(&temporary, bytes).is_ok() {
                    let _ = std::fs::rename(temporary, directory.join("status.json"));
                }
            }
        }
    }
    unsafe fn tick(&mut self) {
        self.paused_apps.retain(|_, until| *until > Instant::now());
        self.process_control();
        self.statistics.poll();
        self.startup.poll();
        let (activate, refresh) = self.model_setup.poll(&self.config, &self.models);
        if refresh {
            let sender = self.models_sender.clone();
            std::thread::spawn(move || {
                let _ = sender.send(model::list_models().map_err(|e| e.to_string()));
            });
        }
        if let Some(tag) = activate {
            let mut config = self.config.clone();
            config.model = tag.clone();
            match config.save() {
                Ok(()) => {
                    self.clear();
                    self.config = config;
                    if !self.models.contains(&tag) {
                        self.models.push(tag.clone());
                        self.models.sort();
                    }
                    self.rebuild_models();
                    self.model_setup
                        .saved(format!("Using {tag}. Settings saved."));
                    self.status_text = format!("Model: {tag}");
                }
                Err(error) => self
                    .model_setup
                    .saved(format!("Model not changed: {error}")),
            }
        }
        if self.history.poll(&self.config) {
            self.clear();
        }
        let history_scope = if self.editing_app.is_empty() {
            crate::history::Selector::All
        } else {
            crate::history::Selector::App(self.editing_app.clone())
        };
        self.history
            .show_count(self.settings.history_count, &history_scope);
        self.websites.refresh_history_count(&self.history);
        self.dismissal.observe_app(&front_app(), Instant::now());
        if !self.dismissal.active() {
            self.dismissed_field = None;
        }
        if self.paused_apps.contains_key(&front_app()) {
            self.history.forget();
            self.clear();
            self.pipeline_status = "Current app temporarily paused";
            capture_status("Current app temporarily paused");
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        // Do cross-process accessibility work after returning from the event
        // tap. Browser UI threads can take longer than its tight old deadline.
        if self.insertion.is_none() && self.replacement_selection.is_none() {
            if let Some(input) = self.pending_input.pop_front() {
                match input {
                    PendingInput::Accept(all) => {
                        self.accept(all);
                    }
                    PendingInput::EmojiMove(next) => {
                        if self.word_choices.is_some() {
                            self.move_word_choice(next);
                        } else {
                            self.move_emoji(next);
                        }
                    }
                    PendingInput::Choose(index) => {
                        self.accept_word_choice(index);
                    }
                }
            }
        }
        self.prepare_accessibility();
        while let Ok(models) = self.preview_results.try_recv() {
            match models {
                Ok(models) => {
                    self.models = models;
                    self.rebuild_models();
                    self.status_text = format!("Model: {}", self.config.model);
                }
                Err(e) => self.status_text = e,
            }
        }
        self.process_screen_replies();
        self.process_choice_replies();
        while let Ok(reply) = self.replies.try_recv() {
            if reply.id != self.epoch.load(Ordering::SeqCst) {
                continue;
            }
            if self.preview_epoch == Some(reply.id) {
                self.preview_epoch = None;
                let preview_context = self.preview_context.take();
                let text = ns(&match reply.result {
                    Ok(s) if s.trim().is_empty() => {
                        "No matching suggestion. Try a few more words.".into()
                    }
                    Ok(s) => match preview_context {
                        Some((before, after)) if !after.trim().is_empty() => {
                            format!("Insert: {s:?}\nResult: {before}{s}{after}")
                        }
                        _ => s,
                    },
                    Err(e) => format!("Could not generate: {e}"),
                });
                let destination = if self.background_preview {
                    self.background_ui.output
                } else if self.website_preview {
                    self.websites.output
                } else {
                    self.settings.output
                };
                let _: () = msg_send![destination,setStringValue:text];
                let _: () = msg_send![text, release];
                continue;
            }
            match reply.result {
                Ok(s) => {
                    let valid = match (&self.context, capture(&self.config)) {
                        (Some(a), Some(b)) => a.same(&b),
                        _ => false,
                    };
                    if valid && !s.trim().is_empty() {
                        self.suggestion = s;
                        self.overlay();
                    } else if valid {
                        self.pipeline_status = "Model returned no matching continuation";
                    } else {
                        self.pipeline_status = "Focus or text changed before completion";
                    }
                }
                Err(e) => {
                    self.status_text = format!("Model error: {e}");
                    self.pipeline_status = "Model request failed";
                }
            }
        }
        if self.preview_epoch.is_some() {
            self.history.observe(&self.config, None, false);
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        if AXIsProcessTrusted() && self.tap.is_null() {
            self.tap = install_tap();
        }
        let selecting_replacement = self
            .insertion
            .as_ref()
            .is_some_and(|p| p.original.range.length > 0);
        let allow_selection = selecting_replacement
            || self.replacement_selection.is_some()
            || self.history.recording_active(&self.config)
            || self.config.choices.synonyms != crate::alternatives::RevealMode::Off
            || self.word_choices.is_some()
            || self.choice_progress != choices::Progress::Untried;
        let (now, empty) = capture_observation(&self.config, 0.08, allow_selection, false,
            self.history.recording_active(&self.config));
        if let Some(empty) = empty {
            self.history.observe_empty(&self.config, empty);
        } else {
            self.history.observe(
                &self.config,
                now.as_ref(),
                self.insertion.is_some() || self.replacement_selection.is_some(),
            );
        }
        if self.replacement_selection.is_some() {
            self.advance_replacement_selection(now.as_ref());
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        if self.insertion.is_some() {
            self.advance_insertion(now.as_ref());
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        let same = match (&self.context, &now) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.clear();
            self.context = now;
            self.changed = Instant::now();
        } else if let (Some(previous), Some(current)) = (&self.context, now) {
            let moved = rect_coords(previous.caret) != rect_coords(current.caret)
                || rect_coords(previous.field_bounds) != rect_coords(current.field_bounds);
            self.context = Some(current);
            if moved && !self.suggestion.is_empty() {
                self.overlay();
            }
        }
        let same_field = match (&self.dismissed_field, &self.context) {
            (Some((pid, field)), Some(ctx)) => *pid == ctx.pid && CFEqual(field.0, ctx.element.0),
            _ => false,
        };
        if self.dismissal.blocks(same_field) {
            self.pipeline_status = "Suggestions paused after dismissal";
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        let selected = self
            .context
            .as_ref()
            .is_some_and(|ctx| ctx.range.length > 0);
        if self.choice_progress == choices::Progress::Untried
            && self.config.choices.automatic_due(
                selected,
                self.changed.elapsed(),
                self.preserving_remainder,
            )
            && self.emoji_choices.is_none()
            && self.replacement.is_none()
        {
            self.request_word_choices();
        }
        if self.choice_progress.blocks_completion(selected) {
            self.refresh_menu();
            self.write_diagnostic();
            return;
        }
        if !self.requested && self.changed.elapsed() >= Duration::from_millis(self.config.delay_ms)
        {
            if self.try_emoji() || self.try_spelling() {
                self.refresh_menu();
                self.write_diagnostic();
                return;
            }
            if let Some(ctx) = &self.context {
                if ctx.prefix.trim().chars().count() >= 3
                    && (self.config.midline || ctx.suffix.trim().is_empty())
                {
                    let id = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                    let request = CompletionRequest {
                        id,
                        prefix: ctx.prefix.clone(),
                        suffix: ctx.suffix.clone(),
                        app_id: ctx.app.clone(),
                        website: ctx.website.clone(),
                        profile: self.history.profile(&self.config, ctx),
                        background: background::collect(&self.config, ctx),
                    };
                    // Keep at most one pending context while the current model call
                    // is cancelling. A full queue is retried on the next timer tick.
                    self.requested = self.submit_with_screen(screen::Work::Completion(request));
                    if self.requested {
                        self.pipeline_status = if self.screen.waiting() {
                            "Reading active-window text locally"
                        } else {
                            "Generating locally"
                        };
                    }
                }
            }
        }
        self.refresh_menu();
        self.write_diagnostic();
    }
    unsafe fn post_chunk(&mut self, ctx: &Context, text: &str) -> bool {
        let system = OwnedCf(AXUIElementCreateSystemWide());
        if IsSecureEventInputEnabled()
            || front_identity() != (ctx.pid, ctx.app.clone())
            || attr(system.0, "AXFocusedUIElement")
                .is_none_or(|focus| !CFEqual(focus.0, ctx.focused.0))
        {
            self.last_accept = "Insertion cancelled: focus changed".into();
            return false;
        }
        self.history.before_insertion(&self.config, ctx);
        if ctx.insertion == InsertionMode::SelectedText
            && is_settable(ctx.element.0, "AXSelectedText")
        {
            let name = cfstr("AXSelectedText");
            let value = cfstr(text);
            let error = AXUIElementSetAttributeValue(ctx.element.0, name.0, value.0);
            if error != 0 {
                self.last_accept = format!("Selected-text insertion failed: {error}; no retry");
                return false;
            }
        } else if ctx.insertion == InsertionMode::UnicodeEvent
            && (ctx.terminal.is_some() || is_settable(ctx.element.0, "AXValue"))
            && bool_attr(ctx.element.0, "AXEnabled") != Some(false)
        {
            if !insert_unicode(ctx, text) {
                self.last_accept = "Unicode insertion cancelled by final validation".into();
                return false;
            }
        } else {
            self.last_accept = "Editor no longer writable".into();
            return false;
        }
        true
    }

    unsafe fn accept(&mut self, all: bool) -> bool {
        if let Some(choices) = &self.word_choices {
            return self.accept_word_choice(choices.selected);
        }
        if self.replacement.is_some() {
            return self.accept_replacement();
        }
        if self.suggestion.is_empty() {
            return false;
        }
        let current = capture(&self.config);
        let valid = match (&self.context, &current) {
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        };
        if !valid {
            self.clear();
            self.last_accept = format!("Acceptance cancelled: {}", CAPTURE_STATUS.with(Cell::get));
            return false;
        }
        let ctx = current.unwrap();
        let Some(mut plan) = AcceptancePlan::new(
            &ctx.prefix,
            &ctx.suffix,
            &self.suggestion,
            all,
            ctx.insertion == InsertionMode::UnicodeEvent,
            self.config.shortcuts.trailing_space,
        ) else {
            self.clear();
            return false;
        };
        let Some(chunk) = plan.next_chunk() else {
            self.clear();
            return false;
        };
        self.last_accept_units = plan.accepted_utf16_units;
        self.last_retained_units = 0;
        self.chunks_verified = 0;
        if !self.post_chunk(&ctx, &chunk) {
            self.clear();
            return false;
        }
        // Keep the remaining suggestion in the transaction until the editor has
        // confirmed the accepted text and its new caret. No regeneration here.
        self.last_accept = format!("{:?} insertion requested", ctx.insertion);
        self.insertion = Some(PendingInsertion {
            original: ctx,
            plan,
            posted: Instant::now(),
        });
        self.suggestion.clear();
        self.requested = true;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let _: () = msg_send![self.panel, orderOut:nil];
        true
    }

    unsafe fn advance_insertion(&mut self, current: Option<&Context>) {
        let Some(mut pending) = self.insertion.take() else {
            return;
        };
        let Some(current) = current else {
            self.clear();
            self.last_accept = "Insertion stopped: editor unavailable; no retry".into();
            return;
        };
        let original = &pending.original;
        if current.pid != original.pid
            || current.app != original.app
            || current.website != original.website
            || current.terminal != original.terminal
            || current.insertion != original.insertion
            || !CFEqual(current.element.0, original.element.0)
            || !CFEqual(current.focused.0, original.focused.0)
        {
            self.clear();
            self.last_accept = "Insertion stopped: focus changed".into();
            return;
        }
        if current.range.length > 0 {
            if original.same(current) && pending.posted.elapsed() < Duration::from_secs(1) {
                self.insertion = Some(pending);
            } else {
                self.clear();
                self.last_accept =
                    "Replacement stopped: selection did not resolve; no retry".into();
            }
            return;
        }
        match pending.plan.observe(&current.prefix, &current.suffix) {
            Observation::Waiting if pending.posted.elapsed() < Duration::from_secs(1) => {
                self.insertion = Some(pending);
            }
            Observation::Waiting | Observation::Diverged => {
                let reason = pending.plan.unconfirmed_reason(&current.prefix, &current.suffix);
                self.clear();
                self.last_accept = format!("Insertion stopped: {reason}; no retry");
            }
            Observation::Confirmed => {
                self.chunks_verified += 1;
                self.history.confirm_acceptance(&self.config, current);
                self.statistics.record(pending.plan.take_verified_words());
                if let Some(chunk) = pending.plan.next_chunk() {
                    if self.post_chunk(current, &chunk) {
                        pending.posted = Instant::now();
                        self.insertion = Some(pending);
                        self.last_accept = "Chunk verified; inserting next chunk".into();
                    } else {
                        self.clear();
                    }
                } else {
                    self.last_accept = "Insertion verified in editor".into();
                    self.context = Some(current.clone());
                    self.suggestion = pending.plan.remainder;
                    self.last_retained_units = self.suggestion.encode_utf16().count();
                    self.requested = !self.suggestion.is_empty();
                    self.preserving_remainder = !self.suggestion.is_empty();
                    self.changed = Instant::now();
                    if self.suggestion.is_empty() {
                        self.pending_input.clear();
                    } else {
                        self.overlay();
                    }
                }
            }
        }
    }
    unsafe fn rebuild_models(&mut self) {
        let item: id = msg_send![self.menu,itemWithTag:10isize];
        let submenu: id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![submenu,setAutoenablesItems:NO];
        for (index, model) in self.models.iter().enumerate() {
            let it = menu_item(submenu, model, sel!(chooseModel:), self.target);
            let _: () = msg_send![it,setTag:(index as isize)];
            let _: () = msg_send![it,setState:if *model==self.config.model{1isize}else{0isize}];
        }
        if self.models.is_empty() {
            let it = menu_item(submenu, "No local models found", sel!(noop:), self.target);
            let _: () = msg_send![it,setEnabled:NO];
        }
        let _: () = msg_send![item,setSubmenu:submenu];
        let _: () = msg_send![submenu, release];
    }
}

extern "C" fn on_tick(target: &Object, selector: Sel, sender: id) {
    if QUIT_REQUESTED.swap(false, Ordering::Relaxed) {
        on_quit(target, selector, sender);
        return;
    }
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(s) = s.as_mut() {
                unsafe {
                    s.tick();
                }
            }
        }
    });
}
extern "C" fn on_pause(_: &Object, _: Sel, _: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            unsafe {
                s.config.enabled = !s.config.enabled;
                s.clear();
                s.save();
                s.refresh_menu();
            }
        }
    });
}
extern "C" fn on_app(_: &Object, _: Sel, _: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            unsafe {
                let app = s.last_app.clone();
                let enabled = !s.app_allowed(&app);
                s.config.set_app_enabled(&app, enabled);
                s.clear();
                s.save();
                s.refresh_menu();
            }
        }
    });
}
extern "C" fn on_pause_app(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            let app = s.last_app.clone();
            if s.app_allowed(&app) {
                s.toggle_app_pause(&app);
                unsafe {
                    s.clear();
                    s.refresh_menu();
                }
            }
        }
    });
}
extern "C" fn on_model(_: &Object, _: Sel, sender: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![sender, tag];
                if let Some(model) = s.models.get(index as usize) {
                    s.config.model = model.clone();
                    s.clear();
                    s.save();
                    s.rebuild_models();
                }
            }
        }
    });
}
extern "C" fn on_refresh(_: &Object, _: Sel, _: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            let tx = s.models_sender.clone();
            std::thread::spawn(move || {
                let _ = tx.send(model::list_models().map_err(|e| e.to_string()));
            });
        }
    });
}
extern "C" fn on_quit(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            s.statistics.finish();
        }
    });
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app,terminate:nil];
    }
}
extern "C" fn on_noop(_: &Object, _: Sel, _: id) {}
extern "C" fn on_setup(_: &Object, _: Sel, _: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            unsafe {
                s.refresh_settings_apps();
                let app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app,activateIgnoringOtherApps:YES];
                let _: () = msg_send![s.settings.window,makeKeyAndOrderFront:nil];
            }
        }
    });
}
extern "C" fn on_permission(_: &Object, _: Sel, _: id) {
    unsafe {
        let key = ns("AXTrustedCheckOptionPrompt");
        let yes: id = msg_send![class!(NSNumber),numberWithBool:YES];
        let opts: id = msg_send![class!(NSDictionary),dictionaryWithObject:yes forKey:key];
        AXIsProcessTrustedWithOptions(opts as CF);
        let _: () = msg_send![key, release];
        let url =
            ns("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
        let url: id = msg_send![class!(NSURL),URLWithString:url];
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let _: BOOL = msg_send![ws,openURL:url];
    }
}
extern "C" fn on_preview(_: &Object, _: Sel, _: id) {
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            unsafe {
                s.clear();
                let prefix = ns_text(msg_send![s.settings.input, stringValue]);
                let suffix = ns_text(msg_send![s.settings.suffix, stringValue]);
                let id = s.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                s.preview_epoch = Some(id);
                s.preview_context = Some((prefix.clone(), suffix.clone()));
                let text = ns("Generating locally…");
                let _: () = msg_send![s.settings.output,setStringValue:text];
                let _: () = msg_send![text, release];
                // Resolve the current app/global drafts through the same layering
                // path as production, without persisting or enabling the real app.
                let mut preview_config = s.preview_instruction_config();
                preview_config.enabled = true;
                let midline_enabled: isize = msg_send![s.settings.midline, state];
                preview_config.midline = midline_enabled == 1;
                preview_config.set_app_enabled("org.libretypist.preview", true);
                if s.requests
                    .try_send((
                        preview_config,
                        CompletionRequest {
                            id,
                            prefix,
                            suffix,
                            app_id: "org.libretypist.preview".into(),
                            website: None,
                            profile: None,
                            background: crate::background::Background::default(),
                        },
                    ))
                    .is_err()
                {
                    s.preview_epoch = None;
                    let text = ns("The previous request is stopping. Please try Complete again.");
                    let _: () = msg_send![s.settings.output, setStringValue:text];
                    let _: () = msg_send![text, release];
                }
            }
        }
    });
}
extern "C" fn on_instruction_scope(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let index: isize = msg_send![s.settings.scope, indexOfSelectedItem];
                let Some(app) = s.settings_apps.get(index as usize).cloned() else {
                    return;
                };
                s.remember_instruction_draft();
                s.editing_app = app;
                s.refresh_word_shortcut();
                let text = ns(&s.instruction_draft(&s.editing_app));
                let _: () = msg_send![s.settings.instructions, setString:text];
                let _: () = msg_send![text, release];
                s.clear();
                let text =
                    ns("Preview uses the selected instructions. Save to apply them in apps.");
                let _: () = msg_send![s.settings.output, setStringValue:text];
                let _: () = msg_send![text, release];
            }
        }
    });
}

extern "C" fn on_save_instructions(_: &Object, _: Sel, _: id) {
    STATE.with(|state| {
        if let Some(s) = state.borrow_mut().as_mut() {
            unsafe {
                let mut updated = s.config.clone();
                let midline: isize = msg_send![s.settings.midline, state];
                updated.midline = midline == 1;
                s.remember_instruction_draft();
                for (app, instructions) in &s.instruction_drafts {
                    if app.is_empty() {
                        updated.writing_instructions = instructions.clone();
                    } else {
                        updated.set_app_instructions(app, instructions.clone());
                    }
                }
                for (app, enabled) in &s.word_shortcut_drafts {
                    updated.set_word_shortcut(app, *enabled);
                }
                for (app, append) in &s.append_instruction_drafts {
                    updated.set_app_append_instructions(app, *append);
                }
                for (app, enabled) in &s.code_editor_drafts {
                    updated.set_complete_in_code(app, *enabled);
                }
                let words = ns_text(msg_send![s.settings.length, stringValue])
                    .trim()
                    .parse();
                let delay = ns_text(msg_send![s.settings.delay, stringValue])
                    .trim()
                    .parse();
                let result = match (words, delay) {
                    (Ok(words), Ok(delay)) => {
                        updated.max_words = words;
                        updated.delay_ms = delay;
                        updated.save()
                    }
                    _ => Err("Use whole numbers for maximum words and delay.".into()),
                };
                let message = match result {
                    Ok(()) => {
                        s.clear();
                        s.config = updated;
                        s.instruction_drafts.clear();
                        s.word_shortcut_drafts.clear();
                        s.code_editor_drafts.clear();
                        s.append_instruction_drafts.clear();
                        code_editor::revoke();
                        cells::revoke();
                        "Saved all settings. Applies to new suggestions.".to_owned()
                    }
                    Err(error) => format!("Not saved: {error}"),
                };
                let text = ns(&message);
                let _: () = msg_send![s.settings.status, setStringValue:text];
                let _: () = msg_send![text, release];
            }
        }
    });
}
extern "C" fn on_licenses(_: &Object, _: Sel, _: id) {
    unsafe {
        let alert: id = msg_send![class!(NSAlert), new];
        let title = ns("LibreType — License and source");
        let detail = ns("Copyright (c) 2026 LibreTypist contributors. Application code: GNU GPL version 3 only. You may modify and redistribute it under that license. No warranty, including merchantability or fitness for a particular purpose. Third-party components and downloaded models retain their own terms. Open the license files for the full license, dependency notices and source-release instructions. This development build has no public source URL configured.");
        let _: () = msg_send![alert,setMessageText:title];
        let _: () = msg_send![alert,setInformativeText:detail];
        for text in ["Open license files", "Close"] {
            let label = ns(text);
            let _: id = msg_send![alert,addButtonWithTitle:label];
            let _: () = msg_send![label, release];
        }
        let result: isize = msg_send![alert, runModal];
        if result == 1000 {
            let bundle: id = msg_send![class!(NSBundle), mainBundle];
            let resource: id = msg_send![bundle, resourcePath];
            let folder = ns("Licenses");
            let path: id = msg_send![resource,stringByAppendingPathComponent:folder];
            let url: id = msg_send![class!(NSURL),fileURLWithPath:path];
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let _: BOOL = msg_send![workspace,openURL:url];
            let _: () = msg_send![folder, release];
        }
        let _: () = msg_send![title, release];
        let _: () = msg_send![detail, release];
        let _: () = msg_send![alert, release];
    }
}
extern "C" fn event_callback(_: CF, kind: u32, event: CF, _: *mut c_void) -> CF {
    if matches!(kind, 10 | 11)
        && !event.is_null()
        && unsafe { CGEventGetIntegerValueField(event, 42) } == INSERT_EVENT_TAG
    {
        return event;
    }
    let mut consumed = false;
    STATE.with(|s| {
        if let Ok(mut state) = s.try_borrow_mut() {
            if let Some(s) = state.as_mut() {
                unsafe {
                    if kind == 0xffff_fffe || kind == 0xffff_ffff {
                        if !s.tap.is_null() {
                            CGEventTapEnable(s.tap, true);
                        }
                        terminal::revoke();
                        code_editor::revoke();
                        cells::revoke();
                        s.suppressed_keys = 0;
                        s.tab_bypassed = false;
                        s.history.forget();
                        s.clear();
                        return;
                    }
                    let key = CGEventGetIntegerValueField(event, 9);
                    if kind == 11 {
                        if key == 48 && s.tab_bypassed {
                            CGEventSetFlags(event, CGEventGetFlags(event) & !OPTION);
                            s.tab_bypassed = false;
                        }
                        if (0..128).contains(&key) && s.suppressed_keys & (1u128 << key) != 0 {
                            s.suppressed_keys &= !(1u128 << key);
                            consumed = true;
                        }
                        return;
                    }
                    if kind != 10 {
                        return;
                    }
                    if key == 48 && s.tab_bypassed {
                        CGEventSetFlags(event, CGEventGetFlags(event) & !OPTION);
                        return;
                    }
                    if (0..128).contains(&key) && s.suppressed_keys & (1u128 << key) != 0 {
                        // Swallow autorepeat and the matching key-up for accepted shortcuts.
                        consumed = true;
                        return;
                    }
                    // AppKit handles shortcut recording locally, even without Accessibility.
                    let (pid, app) = front_identity();
                    if pid == std::process::id() as i32 || s.preview_epoch.is_some() {
                        return;
                    }
                    if IsSecureEventInputEnabled() {
                        terminal::revoke();
                        code_editor::revoke();
                        cells::revoke();
                        s.history.forget();
                        s.clear();
                        return;
                    }
                    let flags = CGEventGetFlags(event);
                    let has_display = !s.suggestion.is_empty()
                        || s.insertion.is_some()
                        || s.replacement_selection.is_some();
                    let has_suggestion =
                        has_display && (s.replacement.is_none() || s.replacement_actionable);
                    if key == 48 && flags & (MODIFIERS & !SHIFT) == OPTION && has_display {
                        s.clear();
                        // A real Tab, rather than Option-Tab, reaches the target editor.
                        CGEventSetFlags(event, flags & !OPTION);
                        s.tab_bypassed = true;
                        return;
                    }
                    let action = u16::try_from(key)
                        .ok()
                        .and_then(|key| s.config.shortcut_action(&app, key, flags));
                    if !matches!(action, Some(Action::Word | Action::All))
                        && crate::terminal::cancels_manual(key, flags)
                    {
                        terminal::revoke();
                        code_editor::revoke();
                        cells::revoke();
                    }
                    match action {
                        None if has_suggestion
                            && s.word_choices.is_some()
                            && choices::number_choice(key, flags).is_some()
                            && s.context
                                .as_ref()
                                .is_some_and(|ctx| ctx.pid == pid && ctx.app == app) =>
                        {
                            let index = choices::number_choice(key, flags).unwrap();
                            if s.word_choices
                                .as_ref()
                                .is_some_and(|c| index < c.items.len())
                            {
                                if s.pending_input.len() < 32 {
                                    s.pending_input.push_back(PendingInput::Choose(index));
                                }
                                consumed = true;
                            } else {
                                s.clear();
                            }
                        }
                        None if has_suggestion
                            && (s.emoji_choices.is_some() || s.word_choices.is_some())
                            && s.context
                                .as_ref()
                                .is_some_and(|ctx| ctx.pid == pid && ctx.app == app)
                            && emoji_navigation(key, flags).is_some() =>
                        {
                            if s.pending_input.len() < 32 {
                                s.pending_input.push_back(PendingInput::EmojiMove(
                                    emoji_navigation(key, flags).unwrap(),
                                ));
                            }
                            consumed = true;
                        }
                        Some(Action::Word | Action::All)
                            if has_suggestion
                                && (action == Some(Action::All)
                                    || s.config.website_word_shortcut_enabled(
                                        s.context.as_ref().and_then(|c| c.website.as_deref()),
                                    )) =>
                        {
                            if s.replacement_selection.is_none() && s.pending_input.len() < 32 {
                                s.pending_input
                                    .push_back(PendingInput::Accept(action == Some(Action::All)));
                            }
                            if s.replacement_selection.is_none() {
                                s.last_accept = "Acceptance shortcut queued for validation".into();
                            }
                            consumed = true;
                        }
                        Some(Action::Dismiss) if has_display => {
                            let valid = match (
                                &s.context,
                                capture_selected(&s.config, 0.012, s.word_choices.is_some()),
                            ) {
                                (Some(a), Some(b)) => a.same(&b),
                                _ => false,
                            };
                            let context = s.context.clone();
                            s.clear();
                            if valid {
                                s.dismissed_field =
                                    context.as_ref().map(|ctx| (ctx.pid, ctx.element.clone()));
                                s.dismissal.start(
                                    s.config.shortcuts.dismiss_behavior,
                                    &app,
                                    s.config.shortcuts.dismiss_seconds,
                                    Instant::now(),
                                );
                                s.context = context;
                                s.changed = Instant::now();
                                consumed = true;
                            }
                        }
                        Some(Action::ToggleGlobal) => {
                            s.clear();
                            s.pending_control = Some((Action::ToggleGlobal, pid, app));
                            consumed = true;
                        }
                        Some(action @ (Action::Manual | Action::PauseApp | Action::Choices))
                            if s.config.allows_app(&app) =>
                        {
                            s.clear();
                            s.pending_control = Some((action, pid, app));
                            consumed = true;
                        }
                        _ => {
                            s.history.mark_input(&s.config, pid, key, flags);
                            s.clear();
                            s.changed = Instant::now();
                        }
                    }
                    if consumed && (0..128).contains(&key) {
                        s.suppressed_keys |= 1u128 << key;
                    }
                }
            }
        }
    });
    if consumed {
        ptr::null()
    } else {
        event
    }
}
unsafe fn install_tap() -> CF {
    let tap = CGEventTapCreate(
        1,
        0,
        0,
        (1 << 10) | (1 << 11),
        event_callback,
        ptr::null_mut(),
    );
    if !tap.is_null() {
        let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
        if !source.is_null() {
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            CFRelease(source);
            CGEventTapEnable(tap, true);
        }
    }
    tap
}
unsafe fn menu_item(menu: id, title: &str, action: Sel, target: id) -> id {
    let item: id = msg_send![class!(NSMenuItem), alloc];
    let title = ns(title);
    let key = ns("");
    let item: id = msg_send![item,initWithTitle:title action:action keyEquivalent:key];
    let _: () = msg_send![item,setTarget:target];
    let _: () = msg_send![menu,addItem:item];
    let _: () = msg_send![title, release];
    let _: () = msg_send![key, release];
    let _: () = msg_send![item, release];
    item
}
unsafe fn label(parent: id, text: &str, frame: NSRect, size: f64) -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field,initWithFrame:frame];
    let _: () = msg_send![field,setEditable:NO];
    let _: () = msg_send![field,setSelectable:NO];
    let _: () = msg_send![field,setBezeled:NO];
    let _: () = msg_send![field,setDrawsBackground:NO];
    let font: id = msg_send![class!(NSFont),systemFontOfSize:size];
    let _: () = msg_send![field,setFont:font];
    let text = ns(text);
    let _: () = msg_send![field,setStringValue:text];
    let _: () = msg_send![text, release];
    let _: () = msg_send![parent,addSubview:field];
    field
}
unsafe fn button(parent: id, title: &str, frame: NSRect, target: id, action: Sel) {
    let b: id = msg_send![class!(NSButton), alloc];
    let b: id = msg_send![b,initWithFrame:frame];
    set_title(b, title);
    let _: () = msg_send![b,setBezelStyle:1usize];
    let _: () = msg_send![b,setTarget:target];
    let _: () = msg_send![b,setAction:action];
    let _: () = msg_send![parent,addSubview:b];
    let _: () = msg_send![b, release];
}
struct SettingsUi {
    window: id,
    input: id,
    suffix: id,
    midline: id,
    word_shortcut: id,
    code_editor: id,
    append_instructions: id,
    output: id,
    instructions: id,
    status: id,
    history_count: id,
    scope: id,
    length: id,
    delay: id,
}

unsafe fn text_input(parent: id, text: &str, name: &str, frame: NSRect) -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, initWithFrame:frame];
    let text = ns(text);
    let _: () = msg_send![field, setStringValue:text];
    let _: () = msg_send![text, release];
    let name = ns(name);
    let _: () = msg_send![field, setAccessibilityLabel:name];
    let _: () = msg_send![name, release];
    let _: () = msg_send![parent, addSubview:field];
    let _: () = msg_send![field, release];
    field
}

unsafe fn setup_window(target: id, config: &Config) -> SettingsUi {
    let window: id = msg_send![class!(NSWindow), alloc];
    let window: id = msg_send![window,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(600.,734.)) styleMask:3usize backing:2usize defer:NO];
    let bundle: id = msg_send![class!(NSBundle), mainBundle];
    let bundle_text = |key: &str| {
        let key = ns(key);
        let value = ns_text(msg_send![bundle, objectForInfoDictionaryKey:key]);
        let _: () = msg_send![key, release];
        value
    };
    let build = bundle_text("CFBundleVersion");
    let title = if build.is_empty() {
        "LibreType — Settings & preview".into()
    } else {
        format!(
            "LibreType — Settings & preview ({}, build {build})",
            bundle_text("CFBundleShortVersionString")
        )
    };
    set_title(window, &title);
    let _: () = msg_send![window,setReleasedWhenClosed:NO];
    let _: () = msg_send![window, center];
    let view: id = msg_send![window, contentView];
    label(
        view,
        "LibreType",
        NSRect::new(NSPoint::new(24., 682.), NSSize::new(550., 36.)),
        28.,
    );
    button(
        view,
        "Emoji…",
        NSRect::new(NSPoint::new(245., 686.), NSSize::new(105., 30.)),
        target,
        sel!(emoji:),
    );
    button(
        view,
        "Startup…",
        NSRect::new(NSPoint::new(153., 686.), NSSize::new(90., 30.)),
        target,
        sel!(startup:),
    );
    button(
        view,
        "Keyboard shortcuts…",
        NSRect::new(NSPoint::new(355., 686.), NSSize::new(222., 30.)),
        target,
        sel!(keyboard:),
    );
    label(view, "Local writing suggestions. Keep Ollama running.\nChoose an installed model from the LT menu.", NSRect::new(NSPoint::new(24.,633.),NSSize::new(370.,44.)),14.);
    button(
        view,
        "License and source…",
        NSRect::new(NSPoint::new(400., 641.), NSSize::new(177., 32.)),
        target,
        sel!(licenses:),
    );
    label(view, "Default shortcuts: Tab accepts a word; Shift-Tab accepts all.\nSecure fields and excluded apps never request suggestions.", NSRect::new(NSPoint::new(24.,586.),NSSize::new(550.,40.)),13.);
    button(
        view,
        "Grant Accessibility Access",
        NSRect::new(NSPoint::new(20., 545.), NSSize::new(245., 32.)),
        target,
        sel!(permission:),
    );
    button(
        view,
        "Local models…",
        NSRect::new(NSPoint::new(267., 545.), NSSize::new(130., 32.)),
        target,
        sel!(modelSetup:),
    );
    button(
        view,
        "Websites…",
        NSRect::new(NSPoint::new(397., 545.), NSSize::new(98., 32.)),
        target,
        sel!(websites:),
    );
    button(
        view,
        "Spelling…",
        NSRect::new(NSPoint::new(495., 545.), NSSize::new(98., 32.)),
        target,
        sel!(spelling:),
    );
    label(
        view,
        "Writing instructions",
        NSRect::new(NSPoint::new(24., 460.), NSSize::new(200., 24.)),
        15.,
    );
    let history_count = label(
        view,
        "History: locked — unlock in History…",
        NSRect::new(NSPoint::new(230., 460.), NSSize::new(345., 24.)),
        11.,
    );
    button(
        view,
        "Word choices…",
        NSRect::new(NSPoint::new(288., 501.), NSSize::new(170., 28.)),
        target,
        sel!(wordChoices:),
    );
    button(
        view,
        "Context…",
        NSRect::new(NSPoint::new(462., 501.), NSSize::new(130., 28.)),
        target,
        sel!(background:),
    );
    button(
        view,
        "History…",
        NSRect::new(NSPoint::new(20., 501.), NSSize::new(130., 28.)),
        target,
        sel!(history:),
    );
    button(
        view,
        "Statistics…",
        NSRect::new(NSPoint::new(154., 501.), NSSize::new(130., 28.)),
        target,
        sel!(statistics:),
    );
    let scope: id = msg_send![class!(NSPopUpButton), alloc];
    let scope: id = msg_send![scope, initWithFrame:NSRect::new(NSPoint::new(22.,423.),NSSize::new(554.,30.)) pullsDown:NO];
    let _: () = msg_send![scope, setTarget:target];
    let _: () = msg_send![scope, setAction:sel!(instructionScope:)];
    let name = ns("Instructions for");
    let _: () = msg_send![scope, setAccessibilityLabel:name];
    let _: () = msg_send![name, release];
    let _: () = msg_send![view, addSubview:scope];
    let _: () = msg_send![scope, release];
    label(
        view,
        "Up to 1,000 characters per scope. Blank inherits. Save settings to apply.",
        NSRect::new(NSPoint::new(24., 399.), NSSize::new(550., 20.)),
        12.,
    );
    let append_instructions: id = msg_send![class!(NSButton), alloc];
    let append_instructions: id = msg_send![append_instructions, initWithFrame:NSRect::new(NSPoint::new(24.,374.),NSSize::new(550.,24.))];
    let _: () = msg_send![append_instructions, setButtonType:3usize];
    set_title(
        append_instructions,
        "Append these app instructions to the global instructions",
    );
    let _: () = msg_send![append_instructions, setState:0isize];
    let _: () = msg_send![append_instructions, setEnabled:NO];
    let _: () = msg_send![view, addSubview:append_instructions];
    let _: () = msg_send![append_instructions, release];
    let scroll: id = msg_send![class!(NSScrollView), alloc];
    let scroll: id =
        msg_send![scroll, initWithFrame:NSRect::new(NSPoint::new(24.,322.),NSSize::new(550.,48.))];
    let _: () = msg_send![scroll, setHasVerticalScroller:YES];
    let _: () = msg_send![scroll, setBorderType:1usize];
    let instructions: id = msg_send![class!(NSTextView), alloc];
    let instructions: id = msg_send![instructions, initWithFrame:NSRect::new(NSPoint::new(0.,0.),NSSize::new(530.,70.))];
    let _: () = msg_send![instructions, setRichText:NO];
    let font: id = msg_send![class!(NSFont),systemFontOfSize:13f64];
    let _: () = msg_send![instructions, setFont:font];
    let _: () = msg_send![instructions, setVerticallyResizable:YES];
    let _: () = msg_send![instructions, setHorizontallyResizable:NO];
    let _: () = msg_send![instructions, setAutoresizingMask:2usize];
    let container: id = msg_send![instructions, textContainer];
    let _: () = msg_send![container, setWidthTracksTextView:YES];
    let name = ns("Writing instructions");
    let _: () = msg_send![instructions, setAccessibilityLabel:name];
    let _: () = msg_send![name, release];
    let initial = ns(&config.writing_instructions);
    let _: () = msg_send![instructions, setString:initial];
    let _: () = msg_send![initial, release];
    let _: () = msg_send![scroll, setDocumentView:instructions];
    let _: () = msg_send![instructions, release];
    let _: () = msg_send![view, addSubview:scroll];
    let _: () = msg_send![scroll, release];
    let code_editor: id = msg_send![class!(NSButton), alloc];
    let code_editor: id = msg_send![code_editor, initWithFrame:NSRect::new(NSPoint::new(24.,294.),NSSize::new(550.,24.))];
    let _: () = msg_send![code_editor, setButtonType:3usize];
    set_title(
        code_editor,
        "Automatically suggest in source-code areas of this app",
    );
    let _: () = msg_send![code_editor, setState:0isize];
    let _: () = msg_send![code_editor, setEnabled:NO];
    let _: () = msg_send![view, addSubview:code_editor];
    let _: () = msg_send![code_editor, release];
    let word_shortcut: id = msg_send![class!(NSButton), alloc];
    let word_shortcut: id = msg_send![word_shortcut, initWithFrame:NSRect::new(NSPoint::new(24.,266.),NSSize::new(550.,26.))];
    let _: () = msg_send![word_shortcut, setButtonType:3usize];
    set_title(
        word_shortcut,
        "Accept one word with its shortcut in this app (default: Tab)",
    );
    let _: () = msg_send![word_shortcut, setState:1isize];
    let _: () = msg_send![word_shortcut, setEnabled:NO];
    let _: () = msg_send![view, addSubview:word_shortcut];
    let _: () = msg_send![word_shortcut, release];
    label(
        view,
        "Preview: text before cursor | text after cursor (optional)",
        NSRect::new(NSPoint::new(24., 233.), NSSize::new(550., 22.)),
        13.,
    );
    let input = text_input(
        view,
        "Thank you for your ",
        "Text before cursor",
        NSRect::new(NSPoint::new(24., 198.), NSSize::new(265., 28.)),
    );
    let suffix = text_input(
        view,
        "",
        "Text after cursor",
        NSRect::new(NSPoint::new(299., 198.), NSSize::new(155., 28.)),
    );
    button(
        view,
        "Complete",
        NSRect::new(NSPoint::new(463., 195.), NSSize::new(112., 32.)),
        target,
        sel!(preview:),
    );
    let output = label(
        view,
        "Your completion will appear here.",
        NSRect::new(NSPoint::new(24., 143.), NSSize::new(550., 48.)),
        14.,
    );
    let _: () = msg_send![output,setSelectable:YES];
    label(
        view,
        "Maximum words (1–32)",
        NSRect::new(NSPoint::new(24., 108.), NSSize::new(170., 22.)),
        12.,
    );
    let length = text_input(
        view,
        &config.max_words.to_string(),
        "Maximum words",
        NSRect::new(NSPoint::new(198., 106.), NSSize::new(55., 26.)),
    );
    label(
        view,
        "Delay in ms (100–10000)",
        NSRect::new(NSPoint::new(279., 108.), NSSize::new(210., 22.)),
        12.,
    );
    let delay = text_input(
        view,
        &config.delay_ms.to_string(),
        "Typing delay in milliseconds",
        NSRect::new(NSPoint::new(495., 106.), NSSize::new(79., 26.)),
    );
    let midline: id = msg_send![class!(NSButton), alloc];
    let midline: id =
        msg_send![midline, initWithFrame:NSRect::new(NSPoint::new(24.,77.),NSSize::new(550.,22.))];
    let _: () = msg_send![midline, setButtonType:3usize];
    set_title(
        midline,
        "Suggest in the middle of text (preview above the caret)",
    );
    let _: () = msg_send![midline, setState:if config.midline { 1isize } else { 0isize }];
    let _: () = msg_send![view, addSubview:midline];
    let _: () = msg_send![midline, release];
    let status = label(
        view,
        "Settings stay on this Mac.",
        NSRect::new(NSPoint::new(24., 12.), NSSize::new(405., 52.)),
        12.,
    );
    button(
        view,
        "Save settings",
        NSRect::new(NSPoint::new(440., 23.), NSSize::new(140., 32.)),
        target,
        sel!(saveInstructions:),
    );
    SettingsUi {
        window,
        input,
        suffix,
        midline,
        word_shortcut,
        code_editor,
        append_instructions,
        output,
        instructions,
        status,
        history_count,
        scope,
        length,
        delay,
    }
}

pub fn run() -> Result<(), String> {
    if unsafe {
        libc::signal(
            libc::SIGTERM,
            request_termination as *const () as libc::sighandler_t,
        )
    } == libc::SIG_ERR
    {
        return Err("Could not install the application shutdown handler".into());
    }
    let config = Config::load()?;
    let (tx, rx) = mpsc::sync_channel::<(Config, CompletionRequest)>(1);
    let (reply_tx, replies) = mpsc::channel();
    let epoch = Arc::new(AtomicU64::new(1));
    let worker_epoch = epoch.clone();
    std::thread::spawn(move || {
        while let Ok(mut work) = rx.recv() {
            while let Ok(new) = rx.try_recv() {
                work = new;
            }
            let (config, request) = work;
            let result =
                model::generate(&config, &request, &worker_epoch).map_err(|e| e.to_string());
            let _ = reply_tx.send(Reply {
                id: request.id,
                result,
            });
        }
    });
    let (models_sender, preview_results) = mpsc::channel();
    let initial_models = models_sender.clone();
    std::thread::spawn(move || {
        let _ = initial_models.send(model::list_models().map_err(|e| e.to_string()));
    });
    unsafe {
        let pool: id = msg_send![class!(NSAutoreleasePool), new];
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: BOOL = msg_send![app,setActivationPolicy:1isize];
        let mut decl = ClassDecl::new("LibreTypistController", class!(NSObject))
            .ok_or("Objective-C class already registered")?;
        decl.add_method(sel!(tick:), on_tick as extern "C" fn(&Object, Sel, id));
        decl.add_method(sel!(pause:), on_pause as extern "C" fn(&Object, Sel, id));
        decl.add_method(sel!(toggleApp:), on_app as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(pauseApp:),
            on_pause_app as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(chooseModel:),
            on_model as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(refreshModels:),
            on_refresh as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(sel!(quit:), on_quit as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(licenses:),
            on_licenses as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(sel!(noop:), on_noop as extern "C" fn(&Object, Sel, id));
        decl.add_method(sel!(setup:), on_setup as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(instructionScope:),
            on_instruction_scope as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(saveInstructions:),
            on_save_instructions as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(permission:),
            on_permission as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(preview:),
            on_preview as extern "C" fn(&Object, Sel, id),
        );
        keyboard::register_actions(&mut decl);
        history::register_actions(&mut decl);
        models::register_actions(&mut decl);
        statistics::register_actions(&mut decl);
        startup::register_actions(&mut decl);
        websites::register_actions(&mut decl);
        spelling::register_actions(&mut decl);
        emoji::register_actions(&mut decl);
        choices::register_actions(&mut decl);
        background::register_actions(&mut decl);
        let target: id = msg_send![decl.register(), new];
        let bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
        let status: id = msg_send![bar,statusItemWithLength:-1.0f64];
        let _: id = msg_send![status, retain];
        let menu: id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![menu,setAutoenablesItems:NO];
        let health_item = menu_item(menu, "Starting local model…", sel!(noop:), target);
        let _: () = msg_send![health_item,setEnabled:NO];
        let pause_item = menu_item(menu, "Pause suggestions", sel!(pause:), target);
        let app_item = menu_item(menu, "Enable current app", sel!(toggleApp:), target);
        let pause_app_item = menu_item(
            menu,
            "Pause current app for 5 minutes",
            sel!(pauseApp:),
            target,
        );
        let models = menu_item(menu, "Model", sel!(noop:), target);
        let _: () = msg_send![models,setTag:10isize];
        menu_item(
            menu,
            "Refresh installed models",
            sel!(refreshModels:),
            target,
        );
        menu_item(menu, "Settings & preview…", sel!(setup:), target);
        menu_item(menu, "Startup…", sel!(startup:), target);
        menu_item(
            menu,
            "Download / choose local model…",
            sel!(modelSetup:),
            target,
        );
        menu_item(menu, "Keyboard shortcuts…", sel!(keyboard:), target);
        menu_item(menu, "Writing history & style…", sel!(history:), target);
        menu_item(menu, "Website preferences…", sel!(websites:), target);
        menu_item(menu, "Spelling…", sel!(spelling:), target);
        menu_item(menu, "Emoji…", sel!(emoji:), target);
        menu_item(menu, "Word choices…", sel!(wordChoices:), target);
        menu_item(menu, "Additional context…", sel!(background:), target);
        let statistics = statistics::Runtime::new(target, menu)?;
        menu_item(
            menu,
            "Grant Accessibility access…",
            sel!(permission:),
            target,
        );
        let separator: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu,addItem:separator];
        menu_item(menu, "License and source…", sel!(licenses:), target);
        menu_item(menu, "Quit LibreType", sel!(quit:), target);
        let _: () = msg_send![status,setMenu:menu];
        let panel: id = msg_send![class!(NSPanel), alloc];
        let panel: id = msg_send![panel,initWithContentRect:NSRect::new(NSPoint::new(0.,0.),NSSize::new(460.,64.)) styleMask:128usize backing:2usize defer:NO];
        let _: () = msg_send![panel,setLevel:3isize];
        let _: () = msg_send![panel,setHidesOnDeactivate:NO];
        let _: () = msg_send![panel,setBecomesKeyOnlyIfNeeded:YES];
        let _: () = msg_send![panel,setIgnoresMouseEvents:YES];
        let _: () = msg_send![panel,setCollectionBehavior:((1usize<<0)|(1usize<<8))];
        let _: () = msg_send![panel,setHasShadow:NO];
        let _: () = msg_send![panel,setOpaque:NO];
        let clear: id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![panel,setBackgroundColor:clear];
        let ghost = ghost_view();
        let _: () = msg_send![panel,setContentView:ghost];
        let _: () = msg_send![ghost, release];
        let settings = setup_window(target, &config);
        let keyboard = keyboard::KeyboardUi::new(target, &config.shortcuts);
        let history = history::Runtime::new(target, &config)?;
        let model_setup = models::ModelUi::new(target)?;
        let startup = startup::StartupUi::new(target);
        let websites = websites::WebsiteUi::new(target);
        let spelling = spelling::SpellingUi::new(target, &config.spelling);
        let emoji = emoji::EmojiUi::new(target, config.emoji);
        let choice_ui = choices::ChoiceUi::new(target, &config.choices);
        let background_ui = background::BackgroundUi::new(target, &config.background);
        let screen = screen::Engine::new(epoch.clone());
        let (choice_requests, choice_replies) = choices::worker(epoch.clone());
        let tap = if AXIsProcessTrusted() {
            install_tap()
        } else {
            ptr::null()
        };
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                config,
                target,
                status,
                menu,
                pause_item,
                app_item,
                pause_app_item,
                health_item,
                panel,
                ghost,
                settings,
                keyboard,
                history,
                model_setup,
                statistics,
                startup,
                websites,
                website_preview: false,
                background_ui,
                screen,
                background_preview: false,
                spelling,
                replacement: None,
                replacement_actionable: false,
                emoji,
                emoji_choices: None,
                choice_ui,
                word_choices: None,
                choice_progress: choices::Progress::Untried,
                preserving_remainder: false,
                choice_preview_request: None,
                choice_requests,
                choice_replies,
                paused_apps: HashMap::new(),
                dismissal: Dismissal::default(),
                dismissed_field: None,
                word_shortcut_drafts: HashMap::new(),
                code_editor_drafts: HashMap::new(),
                append_instruction_drafts: HashMap::new(),
                pending_control: None,
                settings_apps: vec![],
                editing_app: String::new(),
                instruction_drafts: HashMap::new(),
                context: None,
                changed: Instant::now(),
                requested: false,
                request_profile_attached: None,
                personalized_requests: 0,
                suggestion: String::new(),
                epoch,
                requests: tx,
                replies,
                tap,
                suppressed_keys: 0,
                tab_bypassed: false,
                models: vec![],
                status_text: "Ready".into(),
                last_app: String::new(),
                last_app_name: String::new(),
                preview_epoch: None,
                preview_context: None,
                preview_results,
                models_sender,
                last_diagnostic: Instant::now(),
                textedit_diagnostic: serde_json::Value::Null,
                pipeline_status: "Starting",
                accessibility_pid: 0,
                accessibility_bridge: (-25205, -25205),
                signal_diagnostic: serde_json::Value::Null,
                chrome_diagnostic: serde_json::Value::Null,
                pending_input: VecDeque::new(),
                last_accept: "No acceptance attempted".into(),
                insertion: None,
                replacement_selection: None,
                last_accept_units: 0,
                last_retained_units: 0,
                chunks_verified: 0,
            })
        });
        let _: id = msg_send![class!(NSTimer),scheduledTimerWithTimeInterval:0.10f64 target:target selector:sel!(tick:) userInfo:nil repeats:YES];
        let _: () = msg_send![app,setDelegate:target];
        let _: () = msg_send![app, run];
        let _: () = msg_send![pool, drain];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chromium_nil_subrole_needs_positive_unprotected_range_evidence() {
        assert_eq!(text_security("AXTextField", Err(-25212), true), TextSecurity::Unprotected);
        assert_eq!(text_security("AXTextField", Err(-25212), false), TextSecurity::Unknown);
        for error in [0, -25200, -25202, -25204, -25205, -25211] {
            assert_eq!(text_security("AXTextField", Err(error), true), TextSecurity::Unknown);
        }
        assert_eq!(text_security("AXComboBox", Err(-25212), true), TextSecurity::Unknown);
        assert_eq!(text_security("AXTextArea", Err(-25212), false), TextSecurity::Unprotected);
        assert_eq!(text_security("AXTextArea", Err(-25204), true), TextSecurity::Unknown);
    }

    #[test]
    fn secure_roles_override_all_fallback_evidence() {
        for fallback in [false, true] {
            assert_eq!(text_security("AXTextField", Ok("AXSecureTextField"), fallback), TextSecurity::Protected);
            assert_eq!(text_security("AXTextArea", Ok("AXSecureTextField"), fallback), TextSecurity::Protected);
            assert_eq!(text_security("AXSecureTextField", Err(-25212), fallback), TextSecurity::Protected);
            assert_eq!(text_security("AXSecureTextField", Ok(""), fallback), TextSecurity::Protected);
        }
        assert_eq!(text_security("AXTextField", Ok("AXSearchField"), false), TextSecurity::Unprotected);
    }

    #[test]
    fn empty_history_baseline_requires_empty_selection_and_visible_field() {
        let screen = rect(0., 0., 1440., 900.);
        let field = rect(339., 661., 818., 145.);
        let empty = Range { location: 0, length: 0 };
        assert!(empty_history_visible("", empty, field, screen, 900.));
        assert!(!empty_history_visible("Existing document", empty, field, screen, 900.));
        assert!(!empty_history_visible("", Range { location: 1, length: 0 }, field, screen, 900.));
        assert!(!empty_history_visible("", Range { location: 0, length: 1 }, field, screen, 900.));
        assert!(!empty_history_visible("", empty, rect(100., -9952., 600., 1.), screen, 900.));
        assert!(!empty_history_visible("", empty, rect(100., 850., 600., 145.), screen, 900.));
        assert!(!empty_history_visible("", empty, rect(100., 100., 0., 145.), screen, 900.));
    }

    #[test]
    fn right_edge_completion_uses_preview_but_normal_text_stays_inline() {
        let screen = rect(0., 0., 1440., 900.);
        let field = rect(339., 200., 818., 145.);
        let measure = |text: &str| text.chars().count() as f64 * 10.;
        let (preview, is_preview) = completion_frame(rect(1150., 220., 0., 24.), field, screen, 900., "help and support.", measure).unwrap();
        assert!(is_preview);
        assert!(!fitting_midline("help and support.", preview.size.width - 8., measure).is_empty());
        assert!(preview.origin.y >= 900. - 220.);
        let (inline, is_preview) = completion_frame(rect(600., 220., 0., 24.), field, screen, 900., "help and support.", measure).unwrap();
        assert!(!is_preview);
        assert_eq!(inline.origin.x, 600.);
        assert!(completion_frame(rect(600., -100., 0., 24.), field, screen, 900., "help.", measure).is_none());
    }
    #[test]
    fn emoji_navigation_requires_unmodified_vertical_arrows() {
        assert_eq!(emoji_navigation(125, 0), Some(true));
        assert_eq!(emoji_navigation(126, 0), Some(false));
        assert_eq!(emoji_navigation(126, 1 << 16), Some(false)); // Caps Lock
        for flags in [
            SHIFT,
            OPTION,
            crate::shortcuts::COMMAND,
            crate::shortcuts::CONTROL,
        ] {
            assert_eq!(emoji_navigation(125, flags), None);
        }
        for key in [48, 53, 123, 124] {
            assert_eq!(emoji_navigation(key, 0), None);
        }
    }

    #[test]
    fn midline_preview_does_not_cover_the_caret_or_cross_the_screen_edge() {
        let screen = rect(0., 0., 1440., 900.);
        let field = rect(100., 50., 1300., 820.);
        let caret = rect(1350., 200., 0., 18.);
        let preview = midline_frame(caret, field, screen, 900.).unwrap();
        assert!(preview.origin.x + preview.size.width <= 1432.);
        assert_eq!(preview.origin.y, 704.); // above the caret's top at 700
        let top = midline_frame(
            rect(200., 2., 0., 18.),
            rect(100., 0., 600., 200.),
            screen,
            900.,
        )
        .unwrap();
        assert!(top.origin.y + top.size.height <= 876.); // below the caret at screen top
        assert!(midline_frame(rect(200., -20., 0., 18.), field, screen, 900.).is_none());
        let left = midline_frame(
            rect(-200., 200., 0., 18.),
            rect(-1200., 100., 1100., 500.),
            rect(-1280., 0., 1280., 800.),
            900.,
        )
        .unwrap();
        assert!(left.origin.x + left.size.width <= -8.);
    }

    #[test]
    fn midline_fitting_retains_required_separators_even_when_clipped() {
        let measure = |s: &str| s.chars().count() as f64;
        assert_eq!(fitting_midline("meet ", 30., measure), "meet ");
        assert_eq!(fitting_midline("meet and review ", 5., measure), "meet ");
        assert_eq!(fitting_midline("ting", 30., measure), "ting");
        assert_eq!(fitting_midline("meeting ", 2., measure), "");
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn capture_visibility_rejects_hidden_proxies_and_recovers_after_scroll() {
        let screen = rect(0., 0., 1440., 900.);
        // Observed off-screen editable proxy: reading succeeds, but inference
        // cannot produce a suggestion the user can see or safely accept.
        assert!(!caret_is_visible(
            rect(183., -9952., 0., 17.),
            rect(56., 56., 625., 1.),
            screen,
            900.
        ));
        // A fully off-screen field is still invalid even when its caret is
        // inside its own rectangle. Scrolling the same editor back restores it.
        let field = rect(100., -200., 600., 500.);
        assert!(!caret_is_visible(
            rect(200., -100., 0., 18.),
            field,
            screen,
            900.
        ));
        assert!(caret_is_visible(
            rect(200., 100., 0., 18.),
            field,
            screen,
            900.
        ));
        // A caret clipped above an editor's viewport is not visible merely
        // because it falls on a display. The field must contain it too.
        assert!(!caret_is_visible(
            rect(200., 90., 0., 18.),
            rect(100., 100., 600., 500.),
            screen,
            900.
        ));
        // Visibility is not a minimum space requirement: a right-edge caret
        // remains available to replacements and above-caret word choices.
        let field = rect(100., 100., 600., 500.);
        assert!(caret_is_visible(
            rect(700., 200., 0., 18.),
            field,
            screen,
            900.
        ));
        assert!(ghost_frame(rect(700., 200., 0., 18.), field, screen, 900.).is_none());
        assert!(midline_frame(rect(700., 200., 0., 18.), field, screen, 900.).is_some());
    }

    #[test]
    fn capture_visibility_rejects_zero_width_editors_but_preserves_insertion_carets() {
        let screen = rect(0., 0., 1440., 900.);
        // Observed after a spreadsheet check: both rectangles were on-screen,
        // but the editor had no visible width.
        let proxy = rect(60., 0., 0., 1.);
        assert!(!caret_is_visible(proxy, proxy, screen, 900.));
        assert!(ghost_frame(proxy, proxy, screen, 900.).is_none());
        assert!(midline_frame(proxy, proxy, screen, 900.).is_none());

        let caret = rect(200., 200., 0., 18.);
        let field = rect(100., 100., 600., 500.);
        assert!(caret_is_visible(caret, field, screen, 900.));
        assert!(ghost_frame(caret, field, screen, 900.).is_some());
        assert!(midline_frame(caret, field, screen, 900.).is_some());
    }

    #[test]
    fn capture_visibility_preserves_secondary_displays_and_fullscreen_fields() {
        for (caret, field, screen) in [
            (
                rect(-900., 200., 0., 18.),
                rect(-1200., 100., 1100., 500.),
                rect(-1280., 0., 1280., 800.),
            ),
            (
                rect(200., -500., 0., 18.),
                rect(100., -600., 600., 400.),
                rect(0., 900., 1440., 900.),
            ),
            (
                rect(371., 865., 0., 17.),
                rect(244., 863., 1129., 20.),
                rect(0., 0., 1440., 900.),
            ),
        ] {
            assert!(caret_is_visible(caret, field, screen, 900.));
            assert!(ghost_frame(caret, field, screen, 900.).is_some());
        }
        let field = rect(100., 100., 600., 500.);
        let screen = rect(0., 0., 1440., 900.);
        assert!(!caret_is_visible(
            rect(f64::NAN, 200., 0., 18.),
            field,
            screen,
            900.
        ));
        assert!(!caret_is_visible(
            rect(200., 200., 0., 18.),
            field,
            screen,
            f64::INFINITY
        ));
        assert!(!caret_is_visible(
            rect(200., 200., 0., 0.),
            field,
            screen,
            900.
        ));
    }

    #[test]
    fn ghost_stays_at_caret_and_clips_to_field() {
        let frame = ghost_frame(
            rect(200., 100., 0., 16.),
            rect(20., 70., 400., 200.),
            rect(0., 0., 1440., 875.),
            900.,
        )
        .unwrap();
        assert_eq!(rect_coords(frame), (200., 784., 216., 16.));
    }

    #[test]
    fn ghost_handles_left_display_and_hides_at_edge() {
        let screen = rect(-1280., 0., 1280., 800.);
        let field = rect(-1200., 100., 1100., 500.);
        let frame = ghost_frame(rect(-900., 200., 0., 18.), field, screen, 900.).unwrap();
        assert_eq!(rect_coords(frame), (-900., 682., 796., 18.));
        assert!(ghost_frame(rect(-104., 200., 0., 18.), field, screen, 900.).is_none());
        assert!(ghost_frame(rect(-900., 20., 0., 18.), field, screen, 900.).is_none());
    }

    #[test]
    fn fullscreen_signal_caret_can_occupy_the_dock_reservation() {
        let caret = rect(371., 865., 0., 17.);
        let field = rect(244., 863., 1129., 20.);
        // The observed full-screen composer is on-screen, although it lies
        // below the desktop's Dock-reserved usable rectangle.
        assert!(ghost_frame(caret, field, rect(0., 60., 1440., 815.), 900.).is_none());
        let frame = ghost_frame(caret, field, rect(0., 0., 1440., 900.), 900.).unwrap();
        assert_eq!(rect_coords(frame), (371., 18., 998., 17.));
        assert!(ghost_frame(
            rect(371., 895., 0., 17.),
            field,
            rect(0., 0., 1440., 900.),
            900.
        )
        .is_none());
    }

    #[test]
    fn fitting_text_never_accepts_hidden_or_partial_words() {
        let measure = |s: &str| s.chars().count() as f64;
        assert_eq!(
            fitting_completion(" time and help.", 9., measure),
            " time and"
        );
        assert_eq!(fitting_completion(" time and help.", 4., measure), "");
        assert_eq!(fitting_completion("e and help.", 1., measure), "e");
        assert_eq!(fitting_completion(" café bientôt", 6., measure), " café");
        assert_eq!(
            fitting_completion(" full text ", 40., measure),
            " full text"
        );
    }

    #[test]
    fn textedit_first_line_regression_uses_observed_coordinates() {
        let raw = rect(272.052734375, 150., 0., 14.);
        let field = rect(141., 164., 586., 382.);
        let screen = rect(0., 0., 1440., 875.);
        assert!(ghost_frame(raw, field, screen, 900.).is_none());
        let corrected = textedit_caret(raw, field, None, true);
        assert_eq!(corrected.origin.y, 164.);
        let frame = ghost_frame(corrected, field, screen, 900.).unwrap();
        assert_eq!(frame.origin.y, 722.);
        assert_eq!(frame.origin.x, raw.origin.x);
        assert!(frame.size.width > 400.);
        assert_eq!(textedit_caret(raw, field, None, false).origin.y, 150.);
        assert_eq!(
            textedit_caret(rect(272., 120., 0., 14.), field, None, true)
                .origin
                .y,
            120.
        );
    }

    #[test]
    fn textedit_glyph_alignment_requires_matching_horizontal_edge() {
        let field = rect(100., 100., 500., 500.);
        let raw = rect(250., 114., 0., 14.);
        let glyph = rect(244., 128., 6., 14.);
        assert_eq!(
            textedit_caret(raw, field, Some(glyph), false).origin.y,
            128.
        );
        let wrapped = rect(500., 128., 6., 14.);
        assert_eq!(
            textedit_caret(raw, field, Some(wrapped), false).origin.y,
            114.
        );
    }

    #[test]
    fn insertion_requires_an_enabled_writable_control() {
        assert_eq!(
            insertion_mode(true, true, true, false),
            Some(InsertionMode::SelectedText)
        );
        assert_eq!(
            insertion_mode(false, true, true, false),
            Some(InsertionMode::UnicodeEvent)
        );
        assert_eq!(insertion_mode(false, false, true, false), None);
        assert_eq!(insertion_mode(true, true, false, false), None);
    }

    #[test]
    fn browser_insertion_uses_text_input_without_weakening_writability_guards() {
        for selected in [false, true] {
            assert_eq!(
                insertion_mode(selected, true, true, true),
                Some(InsertionMode::UnicodeEvent)
            );
            assert_eq!(insertion_mode(selected, true, false, true), None);
            assert_eq!(insertion_mode(selected, false, false, true), None);
        }
        assert_eq!(insertion_mode(false, false, true, true), None);
        // A selected-text-only control cannot use the Unicode transport, which
        // revalidates AXValue writability immediately before posting each chunk.
        assert_eq!(
            insertion_mode(true, false, true, true),
            Some(InsertionMode::SelectedText)
        );
    }

    #[test]
    fn unicode_insertion_cannot_emit_send_keys_or_truncate_surrogates() {
        for text in ["", "\n", "hello\r", "hello\t", "hello\u{1b}"] {
            assert!(event_payload(text).is_none());
        }
        let text = "🙂".repeat(10);
        assert_eq!(event_payload(&text).unwrap().len(), 20);
        assert!(event_payload(&(text + "a")).is_none());
        let completion = fitting_completion(" hello 🙂 world", 10., |s| {
            s.encode_utf16().count() as f64
        });
        assert_eq!(completion, " hello 🙂");
        assert_eq!(
            String::from_utf16(&event_payload(&completion).unwrap()).unwrap(),
            completion
        );
    }
}
