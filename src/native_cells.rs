// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Keep navigation through compact table cells free of automatic suggestions.
use super::*;

#[derive(Default)]
struct Activation<K> {
    current: Option<(K, Instant)>,
}

impl<K: PartialEq> Activation<K> {
    fn allows(&mut self, cell: Option<K>, manual: bool, now: Instant) -> bool {
        let Some(cell) = cell else {
            self.current = None;
            return true;
        };
        let active = self
            .current
            .as_ref()
            .is_some_and(|(key, until)| *key == cell && now < *until);
        if manual {
            self.current = Some((cell, now + Duration::from_secs(30)));
            true
        } else {
            if !active {
                self.current = None;
            }
            active
        }
    }
}

struct CellKey {
    pid: i32,
    focused: OwnedCf,
    editor: OwnedCf,
    cell: OwnedCf,
    bounds: (f64, f64, f64, f64),
    row: Option<Range>,
    column: Option<Range>,
    spreadsheet_address: Option<String>,
}

impl PartialEq for CellKey {
    fn eq(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.bounds == other.bounds
            && self.row == other.row
            && self.column == other.column
            && self.spreadsheet_address == other.spreadsheet_address
            && unsafe {
                CFEqual(self.focused.0, other.focused.0)
                    && CFEqual(self.editor.0, other.editor.0)
                    && CFEqual(self.cell.0, other.cell.0)
            }
    }
}

thread_local! {
    static MANUAL: RefCell<Activation<CellKey>> = const {
        RefCell::new(Activation { current: None })
    };
}

pub(super) fn revoke() {
    MANUAL.with(|state| state.borrow_mut().current = None);
}

fn compact(field: NSRect, caret: NSRect) -> bool {
    // Scale with the exposed line height instead of assuming a fixed display
    // density or font size. A full-width formula bar is an ordinary editor.
    valid_rect(field)
        && valid_rect(caret)
        && field.size.width > 0.
        && field.size.width <= caret.size.height * 12.
        && field.size.height <= caret.size.height * 2.
}

fn spreadsheet_address(app: &str, dom_id: &str, label: &str) -> Option<String> {
    if !crate::website::is_chromium_browser(app) || dom_id != "waffle-rich-text-editor" {
        return None;
    }
    // A single A1-style cell, not arbitrary labels or ranges. The address is
    // held only in the activation key; it never enters diagnostics or prompts.
    let letters = label.bytes().take_while(u8::is_ascii_uppercase).count();
    let row = label.get(letters..)?;
    ((1..=3).contains(&letters)
        && (1..=7).contains(&row.len())
        && !row.starts_with('0')
        && row.bytes().all(|c| c.is_ascii_digit()))
    .then(|| label.to_owned())
}

pub(super) unsafe fn spreadsheet_editor(app: &str, element: &OwnedCf) -> Option<String> {
    if !crate::website::is_chromium_browser(app) {
        return None;
    }
    let dom_id = string(attr(element.0, "AXDOMIdentifier")?.0)?;
    if dom_id != "waffle-rich-text-editor" {
        return None;
    }
    ["AXDescription", "AXTitle"].into_iter().find_map(|name| {
        let label = string(attr(element.0, name)?.0)?;
        spreadsheet_address(app, &dom_id, &label)
    })
}

pub(super) fn scoped_spreadsheet_address<'a>(
    address: Option<&'a str>,
    host: Option<&str>,
) -> Option<&'a str> {
    address.filter(|_| host == Some("docs.google.com"))
}

fn spreadsheet_key(
    pid: i32,
    focused: &OwnedCf,
    editor: &OwnedCf,
    field: NSRect,
    address: &str,
) -> CellKey {
    CellKey {
        pid,
        focused: focused.clone(),
        editor: editor.clone(),
        cell: editor.clone(),
        // This floating editor grows horizontally as text is accepted. Keep
        // activation through that resize, but not a move or cell-address change.
        bounds: (field.origin.x, field.origin.y, 0., 0.),
        row: None,
        column: None,
        spreadsheet_address: Some(address.into()),
    }
}

unsafe fn index_range(cell: AX, name: &str) -> Option<Range> {
    let value = attr(cell, name)?;
    let mut range = Range {
        location: 0,
        length: 0,
    };
    (ax_value(value.0, 4, &mut range) && range.location >= 0 && range.length > 0).then_some(range)
}

unsafe fn owning_cell(element: &OwnedCf, pid: i32) -> Option<OwnedCf> {
    let mut node = element.clone();
    let mut seen = Vec::<OwnedCf>::new();
    let start = Instant::now();
    // Only follow the focused editor's parents. Never search rows, neighboring
    // cells, document contents or another window to infer a table association.
    for _ in 0..12 {
        let mut owner = 0;
        if start.elapsed() > Duration::from_millis(20)
            || AXUIElementGetPid(node.0, &mut owner) != 0
            || owner != pid
            || seen.iter().any(|old| CFEqual(old.0, node.0))
        {
            return None;
        }
        let role = string(attr(node.0, "AXRole")?.0)?;
        if role == "AXCell" {
            return Some(node);
        }
        if matches!(role.as_str(), "AXWebArea" | "AXWindow" | "AXApplication") {
            return None;
        }
        let parent = attr(node.0, "AXParent")?;
        seen.push(node);
        node = parent;
    }
    None
}

pub(super) unsafe fn allows(
    pid: i32,
    focused: &OwnedCf,
    element: &OwnedCf,
    field: NSRect,
    caret: NSRect,
    manual: bool,
    spreadsheet_address: Option<&str>,
) -> bool {
    let cell = if compact(field, caret) {
        if let Some(address) = spreadsheet_address {
            CAPTURE_DETAIL.with(|d| d.borrow_mut().cell_context = Some("compact spreadsheet cell"));
            Some(spreadsheet_key(pid, focused, element, field, address))
        } else {
            owning_cell(element, pid).map(|cell| {
                CAPTURE_DETAIL.with(|d| d.borrow_mut().cell_context = Some("compact table cell"));
                CellKey {
                    row: index_range(cell.0, "AXRowIndexRange"),
                    column: index_range(cell.0, "AXColumnIndexRange"),
                    pid,
                    focused: focused.clone(),
                    editor: element.clone(),
                    cell,
                    bounds: rect_coords(field),
                    spreadsheet_address: None,
                }
            })
        }
    } else {
        None
    };
    MANUAL.with(|state| state.borrow_mut().allows(cell, manual, Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(100., 100.), NSSize::new(width, height))
    }

    #[test]
    fn compact_cell_detection_scales_but_leaves_large_editors_automatic() {
        let caret = rect(0., 20.);
        assert!(compact(rect(213., 28.), caret));
        assert!(compact(rect(426., 56.), rect(0., 40.)));
        assert!(!compact(rect(620., 28.), caret)); // formula bar
        assert!(!compact(rect(213., 100.), caret)); // multiline editor
        for invalid in [rect(0., 28.), rect(f64::NAN, 28.), rect(213., -1.)] {
            assert!(!compact(invalid, caret));
        }
    }

    #[test]
    fn activation_stays_in_one_cell_and_does_not_extend_itself() {
        let mut gate = Activation::<u32>::default();
        let now = Instant::now();
        assert!(!gate.allows(Some(1), false, now));
        assert!(gate.allows(Some(1), true, now));
        assert!(gate.allows(Some(1), false, now + Duration::from_secs(29)));
        assert!(!gate.allows(Some(1), false, now + Duration::from_secs(30)));
        assert!(gate.allows(Some(1), true, now));
        assert!(!gate.allows(Some(2), false, now));
        assert!(!gate.allows(Some(1), false, now));
        assert!(gate.allows(Some(1), true, now));
        assert!(gate.allows(None, false, now)); // ordinary small field
        assert!(!gate.allows(Some(1), false, now));
    }

    #[test]
    fn reused_editor_cannot_carry_activation_to_another_cell_address() {
        // CF strings provide independently allocated, value-comparable objects
        // for this identity test; no accessibility calls or user data are read.
        let key = || unsafe {
            CellKey {
                pid: 7,
                focused: cfstr("focus"),
                editor: cfstr("editor"),
                cell: cfstr("cell"),
                bounds: (100., 100., 213., 28.),
                row: Some(Range {
                    location: 1,
                    length: 1,
                }),
                column: Some(Range {
                    location: 2,
                    length: 1,
                }),
                spreadsheet_address: None,
            }
        };
        let now = Instant::now();
        for changed in 0..6 {
            let mut gate = Activation { current: None };
            assert!(gate.allows(Some(key()), true, now));
            assert!(gate.allows(Some(key()), false, now));
            let mut other = key();
            match changed {
                0 => other.row.as_mut().unwrap().location += 1,
                1 => other.column.as_mut().unwrap().location += 1,
                2 => other.bounds.0 += 30.,
                3 => other.pid += 1,
                4 => other.focused = unsafe { cfstr("other-focus") },
                _ => other.cell = unsafe { cfstr("other-cell") },
            }
            assert!(!gate.allows(Some(other), false, now));
        }
    }

    #[test]
    fn floating_spreadsheet_identity_requires_exact_provider_id_address_and_host() {
        let app = "com.google.Chrome";
        let id = "waffle-rich-text-editor";
        for address in ["A1", "F2", "ZZZ1000000"] {
            assert_eq!(
                spreadsheet_address(app, id, address).as_deref(),
                Some(address)
            );
        }
        for label in [
            "",
            "F0",
            "F02",
            "f2",
            " F2",
            "F2 ",
            "F2:G3",
            "F2 notes",
            "AAAA1",
            "F10000000",
            "F٢",
        ] {
            assert!(spreadsheet_address(app, id, label).is_none());
        }
        assert!(spreadsheet_address("org.whispersystems.signal-desktop", id, "F2").is_none());
        assert!(spreadsheet_address(app, "formula-bar", "F2").is_none());
        assert_eq!(
            scoped_spreadsheet_address(Some("F2"), Some("docs.google.com")),
            Some("F2")
        );
        for host in [
            None,
            Some("mail.google.com"),
            Some("docs.google.com.example"),
        ] {
            assert_eq!(scoped_spreadsheet_address(Some("F2"), host), None);
        }
    }

    #[test]
    fn floating_cell_activation_survives_growth_but_not_cell_or_focus_changes() {
        let focused = unsafe { cfstr("focus") };
        let editor = unsafe { cfstr("editor") };
        let now = Instant::now();
        let key = |address, width| spreadsheet_key(7, &focused, &editor, rect(width, 19.), address);
        let mut gate = Activation { current: None };
        assert!(!gate.allows(Some(key("F2", 119.)), false, now));
        assert!(gate.allows(Some(key("F2", 119.)), true, now));
        assert!(gate.allows(Some(key("F2", 200.)), false, now));
        assert!(!gate.allows(Some(key("F3", 200.)), false, now));
        assert!(!gate.allows(Some(key("F2", 119.)), false, now));
        assert!(gate.allows(Some(key("F2", 119.)), true, now));
        let mut moved = key("F2", 119.);
        moved.bounds.1 += 20.;
        assert!(!gate.allows(Some(moved), false, now));
        assert!(gate.allows(Some(key("F2", 119.)), true, now));
        let mut other = key("F2", 119.);
        other.focused = unsafe { cfstr("other") };
        assert!(!gate.allows(Some(other), false, now));
    }
}
