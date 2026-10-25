// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Optional geometry correction for the verified off-screen Docs braille editor.
use super::*;
use crate::docs_bridge::{self, transport::Server, Cache, Rect};
use std::sync::OnceLock;

static CACHE: OnceLock<Arc<Cache>> = OnceLock::new();

pub(super) fn start(config: &Config) -> Option<Server> {
    let server = Server::bind(docs_bridge::transport::socket_path().ok()?).ok()?;
    server
        .cache
        .set_enabled(config.enabled && config.google_docs);
    CACHE.set(server.cache.clone()).ok()?;
    Some(server)
}
pub(super) fn sync_enabled(config: &Config) {
    if let Some(cache) = CACHE.get() {
        cache.set_enabled(config.enabled && config.google_docs);
    }
}
fn plain(rect: NSRect) -> Rect {
    Rect(
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}
fn native(rect: Rect) -> NSRect {
    NSRect::new(NSPoint::new(rect.0, rect.1), NSSize::new(rect.2, rect.3))
}

pub(super) unsafe fn awaiting_geometry(config: &Config, original: &Context, elapsed: Duration) -> bool {
    if !docs_bridge::may_wait_after_insertion(
        original.docs_geometry.is_some(),
        config.enabled && config.google_docs,
        elapsed,
        CAPTURE_STATUS.with(Cell::get),
    ) || original.app != "com.google.Chrome" || original.website.as_deref() != Some("docs.google.com") {
        return false;
    }
    let system = OwnedCf(AXUIElementCreateSystemWide());
    capture_focus_unchanged(system.0, &original.focused, original.pid, &original.app,
        true, &original.website, 0.08)
}

pub(super) unsafe fn geometry(
    config: &Config,
    app: &str,
    focused: &OwnedCf,
    pid: i32,
    text: &str,
    caret: (NSRect, usize),
    timeout: f32,
) -> Option<(NSRect, NSRect, f64)> {
    if !config.google_docs || app != "com.google.Chrome" {
        return None;
    }
    let diagnostic = |state| CAPTURE_DETAIL.with(|d| d.borrow_mut().docs_companion = Some(state));
    diagnostic("Waiting for companion geometry");
    let snapshot = CACHE.get()?.current()?;
    CAPTURE_DETAIL.with(|d| d.borrow_mut().docs_selection_utf16 = Some([caret.1, snapshot.selection_utf16]));
    diagnostic("Finding native Docs document");
    let started = Instant::now();
    let mut node = focused.clone();
    let mut seen = Vec::<OwnedCf>::new();
    for _ in 0..32 {
        if started.elapsed() > Duration::from_secs_f32(timeout.max(0.005))
            || seen.iter().any(|old| CFEqual(old.0, node.0))
        {
            return None;
        }
        let mut node_pid = 0;
        if AXUIElementGetPid(node.0, &mut node_pid) != 0 || node_pid != pid {
            return None;
        }
        let role = string(attr(node.0, "AXRole")?.0)?;
        if matches!(role.as_str(), "AXWindow" | "AXApplication") {
            return None;
        }
        if role == "AXWebArea" {
            if let Some(url) = attr(node.0, "AXURL").and_then(|v| {
                if CFGetTypeID(v.0) == CFURLGetTypeID() {
                    string(CFURLGetString(v.0))
                } else {
                    string(v.0)
                }
            }) {
                if docs_bridge::page_key(&url).is_some() {
                    let viewport = element_bounds(node.0)?;
                    let scale = viewport.size.width / snapshot.viewport[0];
                    CAPTURE_DETAIL.with(|d| d.borrow_mut().docs_proxy_delta = Some([
                        caret.0.origin.x - (viewport.origin.x + snapshot.proxy_caret.0 * scale),
                        caret.0.origin.y - (viewport.origin.y + snapshot.proxy_caret.1 * scale),
                        caret.0.size.height - snapshot.proxy_caret.3 * scale,
                    ]));
                    let resolved = snapshot.resolve_checked(&url, text, plain(caret.0), plain(viewport), caret.1);
                    diagnostic(match &resolved { Ok(_) => "Matched text and caret", Err(reason) => reason });
                    return resolved.ok().and_then(|(caret, field)| {
                            // Metadata from the braille field is in CSS units.
                            // The resolved caret is in native screen points,
                            // including both document and browser zoom.
                            docs_bridge::paired_caret_scale(caret.3, snapshot.proxy_caret.3)
                                .map(|scale| (native(caret), native(field), scale))
                        });
                }
            }
        }
        seen.push(node.clone());
        node = attr(node.0, "AXParent")?;
    }
    None
}
