// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Focus-scoped manual activation for code-editor controls.
use super::*;
use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};

struct Lease {
    pid: i32,
    focused: OwnedCf,
    element: OwnedCf,
    labels: Vec<String>,
    until: Instant,
}
thread_local! {
    static MANUAL: RefCell<Option<Lease>> = const { RefCell::new(None) };
}
pub(super) fn revoke() {
    MANUAL.with(|s| *s.borrow_mut() = None);
}

pub(super) struct Policy {
    pub allowed: bool,
    pub viewport: Option<NSRect>,
}

unsafe fn bounds(element: &OwnedCf) -> Option<NSRect> {
    let position = attr(element.0, "AXPosition")?;
    let size = attr(element.0, "AXSize")?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    (ax_value(position.0, 1, &mut rect.origin)
        && ax_value(size.0, 2, &mut rect.size)
        && valid_rect(rect))
    .then_some(rect)
}

struct EditorPath {
    classes: Vec<Vec<String>>,
    viewport: Option<NSRect>,
    extension_chat: bool,
}

unsafe fn class_path(element: &OwnedCf, pid: i32) -> EditorPath {
    let mut path = Vec::new();
    let mut viewport = None;
    let mut extension_chat = false;
    let mut node = element.clone();
    let mut seen: Vec<OwnedCf> = Vec::new();
    let start = Instant::now();
    for _ in 0..12 {
        let mut owner = 0;
        if start.elapsed() > Duration::from_millis(20)
            || AXUIElementGetPid(node.0, &mut owner) != 0
            || owner != pid
            || seen.iter().any(|old| CFEqual(old.0, node.0))
        {
            break;
        }
        let mut classes = Vec::new();
        if let Some(array) = attr(node.0, "AXDOMClassList") {
            if CFGetTypeID(array.0) != CFArrayGetTypeID() {
                break;
            }
            let count = CFArrayGetCount(array.0.cast());
            if !(0..=64).contains(&count) {
                break;
            }
            for index in 0..count {
                if let Some(class) = string(CFArrayGetValueAtIndex(array.0.cast(), index)) {
                    if class.len() <= 128 {
                        classes.push(class);
                    }
                }
            }
        }
        // Only an ancestor of the focused input may supply display space.
        // Prefer the inner clipping container over a larger editor/workbench.
        if viewport.is_none()
            && classes
                .iter()
                .any(|class| class == "overflow-guard" || class == "monaco-editor")
        {
            viewport = bounds(&node);
        }
        path.push(classes);
        let role = attr(node.0, "AXRole").and_then(|v| string(v.0));
        if role.as_deref() == Some("AXWebArea") {
            let url = attr(node.0, "AXURL").and_then(|v| {
                if CFGetTypeID(v.0) == CFURLGetTypeID() {
                    string(CFURLGetString(v.0))
                } else {
                    string(v.0)
                }
            });
            extension_chat = url
                .as_deref()
                .is_some_and(|url| crate::code_editor::extension_chat(&path, url));
            // A nested document cannot borrow its parent's extension identity.
            break;
        }
        if crate::code_editor::classify_path(&[], &path) != crate::code_editor::InputKind::Other {
            break;
        }
        let Some(parent) = attr(node.0, "AXParent") else {
            break;
        };
        seen.push(node);
        node = parent;
    }
    EditorPath {
        classes: path,
        viewport,
        extension_chat,
    }
}

pub(super) unsafe fn policy(
    config: &Config,
    app: &str,
    pid: i32,
    focused: &OwnedCf,
    element: &OwnedCf,
    manual: bool,
) -> Policy {
    if !crate::code_editor::supported(app) {
        revoke();
        return Policy {
            allowed: true,
            viewport: None,
        };
    }
    let labels: Vec<String> = ["AXDescription", "AXTitle", "AXHelp", "AXIdentifier"]
        .iter()
        .map(|name| {
            attr(element.0, name)
                .and_then(|v| string(v.0))
                .unwrap_or_default()
        })
        .map(|s| if s.len() <= 2048 { s } else { String::new() })
        .collect();
    let path = class_path(element, pid);
    let kind = if path.extension_chat {
        crate::code_editor::InputKind::Chat
    } else {
        crate::code_editor::classify_path(&labels, &path.classes)
    };
    CAPTURE_DETAIL.with(|d| {
        d.borrow_mut().code_editor_kind = Some(match kind {
            crate::code_editor::InputKind::Chat if path.extension_chat => "extension chat",
            crate::code_editor::InputKind::Chat => "chat",
            crate::code_editor::InputKind::Source => "source",
            crate::code_editor::InputKind::Other => "other",
        })
    });
    let allowed = MANUAL.with(|state| {
        let mut lease = state.borrow_mut();
        let matches = lease.as_ref().is_some_and(|old| {
            old.pid == pid
                && CFEqual(old.focused.0, focused.0)
                && CFEqual(old.element.0, element.0)
                && old.labels == labels
                && Instant::now() < old.until
        });
        if !matches {
            *lease = None;
        }
        if manual {
            *lease = Some(Lease {
                pid,
                focused: focused.clone(),
                element: element.clone(),
                labels: labels.clone(),
                until: Instant::now() + Duration::from_secs(30),
            });
        }
        if manual || matches {
            return true;
        }
        crate::code_editor::automatic(kind, config.complete_in_code(app))
    });
    Policy {
        allowed,
        viewport: if allowed && kind != crate::code_editor::InputKind::Other {
            path.viewport
        } else {
            None
        },
    }
}

/// Expand only the line's horizontal drawing space, keeping both the focused
/// field's vertical limits and the owning editor's clipping bounds.
pub(super) fn display_bounds(field: NSRect, caret: NSRect, viewport: NSRect) -> Option<NSRect> {
    if !valid_rect(field) || !valid_rect(viewport) || !valid_rect(caret) {
        return None;
    }
    let contains_caret = |rect: NSRect| {
        caret.origin.x >= rect.origin.x
            && caret.origin.x <= rect.origin.x + rect.size.width
            && caret.origin.y >= rect.origin.y
            && caret.origin.y + caret.size.height <= rect.origin.y + rect.size.height
    };
    if !contains_caret(field) || !contains_caret(viewport) {
        return None;
    }
    let left = field.origin.x.max(viewport.origin.x);
    let top = field.origin.y.max(viewport.origin.y);
    let right = viewport.origin.x + viewport.size.width;
    let bottom = (field.origin.y + field.size.height).min(viewport.origin.y + viewport.size.height);
    let rect = NSRect::new(
        NSPoint::new(left, top),
        NSSize::new(right - left, bottom - top),
    );
    valid_rect(rect).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }
    #[test]
    fn editor_bounds_supply_room_without_reviving_clipped_or_hidden_carets() {
        // Observed VSCodium input ends exactly at the caret. The owning editor
        // provides horizontal room without moving the caret or inventing a row.
        let field = rect(234., 100., 138., 18.);
        let caret = rect(372., 102., 0., 14.);
        let viewport = rect(167., 92., 1050., 700.);
        let expanded = display_bounds(field, caret, viewport).unwrap();
        assert_eq!(rect_coords(expanded), (234., 100., 983., 18.));
        let screen = rect(0., 0., 1440., 900.);
        assert!(ghost_frame(caret, field, screen, 900.).is_none());
        assert!(ghost_frame(caret, expanded, screen, 900.).is_some());
        for clipped in [
            rect(400., 92., 600., 700.),   // neighboring pane
            rect(167., 120., 1050., 700.), // scrolled row
            rect(167., 92., 1050., 20.),   // partially clipped caret
            rect(f64::NAN, 92., 1050., 700.),
        ] {
            assert!(display_bounds(field, caret, clipped).is_none());
        }
        assert!(display_bounds(rect(234., -10000., 138., 18.), caret, viewport).is_none());
        // A narrower visible pane clips the old field instead of borrowing room
        // from an adjacent pane. Field height and baseline stay unchanged.
        let clipped = display_bounds(
            rect(234., 100., 1000., 18.),
            caret,
            rect(167., 92., 400., 700.),
        )
        .unwrap();
        assert_eq!(rect_coords(clipped), (234., 100., 333., 18.));
    }
}
