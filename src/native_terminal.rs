// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Focused terminal cursor adapter. No shell commands, clipboard, or Return events.
use super::*;

struct Lease {
    pid: i32,
    focused: OwnedCf,
    scope: crate::terminal::Scope,
}
thread_local! {
    static MANUAL: RefCell<Option<Lease>> = const { RefCell::new(None) };
}

pub(super) fn revoke() {
    MANUAL.with(|m| *m.borrow_mut() = None);
}

pub(super) fn front_changed(pid: i32, app: &str) {
    MANUAL.with(|m| {
        if m.borrow()
            .as_ref()
            .is_some_and(|lease| lease.pid != pid || !crate::terminal::supported(app))
        {
            *m.borrow_mut() = None;
        }
    });
}

unsafe fn number(value: CF) -> Option<i64> {
    let mut n = -1i64;
    (CFGetTypeID(value) == CFNumberGetTypeID()
        && CFNumberGetValue(value, 4, &mut n as *mut i64 as *mut c_void)
        && n >= 0)
        .then_some(n)
}

unsafe fn range(element: AX) -> Option<Range> {
    let selected = attr(element, "AXSelectedTextRange")?;
    let mut range = Range {
        location: -1,
        length: -1,
    };
    (ax_value(selected.0, 4, &mut range) && range.location >= 0 && range.length == 0)
        .then_some(range)
}

unsafe fn cursor_line(element: AX, cursor: Range) -> Option<i64> {
    let insertion = number(attr(element, "AXInsertionPointLineNumber")?.0)?;
    let actual = line_for_index(element, usize::try_from(cursor.location).ok()?)?;
    (actual == insertion).then_some(insertion)
}

unsafe fn line_for_index(element: AX, offset: usize) -> Option<i64> {
    let index: id = msg_send![class!(NSNumber), numberWithLongLong:i64::try_from(offset).ok()?];
    number(
        parameterized(element, "AXLineForIndex", index as CF)
            .ok()?
            .0,
    )
}

unsafe fn line_range(element: AX, line: i64) -> Result<crate::terminal::TextRange, i32> {
    let index: id = msg_send![class!(NSNumber), numberWithLongLong:line];
    let value = parameterized(element, "AXRangeForLine", index as CF)?;
    let mut range = Range {
        location: -1,
        length: -1,
    };
    if !ax_value(value.0, 4, &mut range)
        || range.location < 0
        || range.length < 0
        || range.location == isize::MAX
        || range.location.checked_add(range.length).is_none()
    {
        return Err(-25201);
    }
    Ok(crate::terminal::TextRange {
        start: range.location as usize,
        length: range.length as usize,
    })
}

unsafe fn limited_text(value: CF) -> Option<String> {
    if value.is_null()
        || CFGetTypeID(value) != CFStringGetTypeID()
        || !(0..=256 * 1024).contains(&CFStringGetLength(value))
    {
        return None;
    }
    let text = string(value)?;
    (text.len() <= 256 * 1024).then_some(text)
}

/// Range queries can refresh the terminal's own accessibility cache. The client
/// receives only a bounded window; this does not promise lower server-side CPU.
unsafe fn snapshot(element: AX, cursor: Range, line: i64) -> Option<crate::terminal::Snapshot> {
    let plan = crate::terminal::plan_window(
        cursor.location as usize,
        usize::try_from(line).ok()?,
        |number| line_range(element, i64::try_from(number).map_err(|_| -25201)?),
    )?;
    let window = match plan {
        crate::terminal::WindowPlan::Range(window) => window,
        crate::terminal::WindowPlan::Legacy => {
            // Check the count before requesting a potentially huge AXValue.
            let count = number(attr(element, "AXNumberOfCharacters")?.0)?;
            if count > 256 * 1024 {
                return None;
            }
            let text = limited_text(attr(element, "AXValue")?.0)?;
            let window = crate::terminal::TextRange {
                start: 0,
                length: count as usize,
            };
            if text.encode_utf16().count() != window.length
                || !window.contains(cursor.location as usize)
            {
                return None;
            }
            return Some(crate::terminal::Snapshot { window, text });
        }
    };
    let requested = Range {
        location: isize::try_from(window.start).ok()?,
        length: isize::try_from(window.length).ok()?,
    };
    let parameter = OwnedCf(AXValueCreate(
        4,
        &requested as *const Range as *const c_void,
    ));
    if parameter.0.is_null() {
        return None;
    }
    let value = parameterized(element, "AXStringForRange", parameter.0).ok()?;
    let text = limited_text(value.0)?;
    if text.encode_utf16().count() != window.length {
        return None;
    }
    Some(crate::terminal::Snapshot { window, text })
}

unsafe fn border_bounds(element: AX, offset: usize) -> Option<NSRect> {
    let range = Range {
        location: isize::try_from(offset).ok()?,
        length: 1,
    };
    let parameter = OwnedCf(AXValueCreate(4, &range as *const Range as *const c_void));
    let bounds = parameterized(element, "AXBoundsForRange", parameter.0).ok()?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    (ax_value(bounds.0, 3, &mut rect) && valid_rect(rect) && rect.size.width > 0.).then_some(rect)
}

/// Every candidate border is independently verified at the same screen X.
/// Display is bounded to the contiguous rows whose border geometry was checked.
fn pane_bounds(
    field: NSRect,
    caret: NSRect,
    rows: &[(Option<NSRect>, Option<NSRect>)],
) -> Option<NSRect> {
    let Some((left, right)) = rows.first() else {
        return Some(field);
    };
    let aligned = |a: NSRect, b: NSRect| {
        (a.origin.x - b.origin.x).abs() <= 0.5 && (a.size.width - b.size.width).abs() <= 0.5
    };
    let mut top = caret.origin.y;
    let mut bottom = top + caret.size.height;
    let mut row_positions = Vec::new();
    for (index, (l, r)) in rows.iter().enumerate() {
        let row = l.or(*r)?;
        if (row.size.height - caret.size.height).abs() > 1. {
            return None;
        }
        row_positions.push(row.origin.y);
        for (expected, actual) in [(*left, *l), (*right, *r)] {
            match (expected, actual) {
                (None, None) => {}
                (Some(a), Some(b)) if valid_rect(b) && aligned(a, b) => {
                    if index == 0 && (b.origin.y - caret.origin.y).abs() > 1. {
                        return None;
                    }
                    top = top.min(b.origin.y);
                    bottom = bottom.max(b.origin.y + b.size.height);
                }
                _ => return None,
            }
        }
        if let (Some(l), Some(r)) = (l, r) {
            if (l.origin.y - r.origin.y).abs() > 1. || (l.size.height - r.size.height).abs() > 1. {
                return None;
            }
        }
    }
    row_positions.sort_by(f64::total_cmp);
    if row_positions.windows(2).any(|pair| {
        let step = pair[1] - pair[0];
        step < 1. || (step - caret.size.height).abs() > 1.
    }) {
        return None;
    }
    let x = left
        .map_or(field.origin.x, |r| r.origin.x + r.size.width)
        .max(field.origin.x);
    let end = right
        .map_or(field.origin.x + field.size.width, |r| r.origin.x)
        .min(field.origin.x + field.size.width);
    top = top.max(field.origin.y);
    bottom = bottom.min(field.origin.y + field.size.height);
    (end > x
        && bottom > top
        && caret.origin.x >= x
        && caret.origin.x < end
        && caret.origin.y >= top
        && caret.origin.y + caret.size.height <= bottom)
        .then(|| NSRect::new(NSPoint::new(x, top), NSSize::new(end - x, bottom - top)))
}

pub(super) fn preview_frame(
    caret: NSRect,
    field: NSRect,
    screen: NSRect,
    primary_top: f64,
) -> Option<NSRect> {
    let mut frame = midline_frame(caret, field, screen, primary_top)?;
    let left = field.origin.x.max(screen.origin.x) + 4.;
    let right = (field.origin.x + field.size.width).min(screen.origin.x + screen.size.width) - 4.;
    frame.size.width = frame.size.width.min(right - left);
    if frame.size.width < 16. {
        return None;
    }
    frame.origin.x = caret.origin.x.clamp(left, right - frame.size.width);
    let bottom = (primary_top - field.origin.y - field.size.height).max(screen.origin.y) + 2.;
    let top = (primary_top - field.origin.y).min(screen.origin.y + screen.size.height) - 2.;
    if frame.origin.y < bottom || frame.origin.y + frame.size.height > top {
        frame.origin.y = primary_top - caret.origin.y - caret.size.height - frame.size.height - 4.;
    }
    (frame.origin.y >= bottom && frame.origin.y + frame.size.height <= top).then_some(frame)
}

pub(super) unsafe fn capture(
    pid: i32,
    app: String,
    focused: OwnedCf,
    manual_request: bool,
) -> Option<Context> {
    let result = capture_inner(pid, app, focused, manual_request);
    if result.is_none() {
        revoke();
    }
    result
}

unsafe fn capture_inner(
    pid: i32,
    app: String,
    focused: OwnedCf,
    manual_request: bool,
) -> Option<Context> {
    capture_status("Terminal cursor unavailable");
    // A focused native terminal text area identifies an iTerm split independently.
    // Never walk a terminal's children, read another pane, or infer a cursor from
    // its last output character.
    let role = string(attr(focused.0, "AXRole")?.0)?;
    CAPTURE_DETAIL.with(|d| d.borrow_mut().role = role.clone());
    if role != "AXTextArea" || bool_attr(focused.0, "AXEnabled") == Some(false) {
        return None;
    }
    match attr_checked(focused.0, "AXSubrole") {
        Ok(v) if string(v.0).is_none_or(|s| s == "AXSecureTextField") => return None,
        Err(e) if !matches!(e, -25205 | -25212) => return None,
        _ => {}
    }
    let cursor = range(focused.0)?;
    let line = cursor_line(focused.0, cursor)?;
    capture_status("Terminal text window unavailable");
    let observed = snapshot(focused.0, cursor, line)?;
    capture_status("Terminal prompt layout not supported");
    let prompt = observed.prompt(cursor.location as usize, |offset| {
        usize::try_from(line_for_index(focused.0, offset)?).ok()
    })?;
    let leased = MANUAL.with(|m| {
        m.borrow().as_ref().is_some_and(|lease| {
            lease.pid == pid && lease.scope == prompt.scope && CFEqual(lease.focused.0, focused.0)
        })
    });
    capture_status("Terminal prompt requires manual activation");
    if !prompt.automatic && !manual_request && !leased {
        return None;
    }
    // Require the cursor's real rectangle. A block cursor's width is not an
    // insertion offset; ghost text begins at its leading edge.
    capture_status("Terminal caret bounds unavailable");
    let parameter = OwnedCf(AXValueCreate(4, &cursor as *const Range as *const c_void));
    let bounds = parameterized(focused.0, "AXBoundsForRange", parameter.0).ok()?;
    let mut caret = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    if !ax_value(bounds.0, 3, &mut caret) || !valid_rect(caret) {
        return None;
    }
    caret.size.width = 0.;
    let mut field_bounds = caret;
    if !ax_value(
        attr(focused.0, "AXPosition")?.0,
        1,
        &mut field_bounds.origin,
    ) || !ax_value(attr(focused.0, "AXSize")?.0, 2, &mut field_bounds.size)
        || !valid_rect(field_bounds)
        || caret.origin.x < field_bounds.origin.x
        || caret.origin.y < field_bounds.origin.y
        || caret.origin.x >= field_bounds.origin.x + field_bounds.size.width
        || caret.origin.y + caret.size.height > field_bounds.origin.y + field_bounds.size.height
    {
        return None;
    }
    capture_status("Terminal pane geometry unavailable");
    let mut rows = Vec::new();
    for edge in &prompt.edges {
        let left = match edge.left_utf16 {
            Some(i) => Some(border_bounds(focused.0, i)?),
            None => None,
        };
        let right = match edge.right_utf16 {
            Some(i) => Some(border_bounds(focused.0, i)?),
            None => None,
        };
        rows.push((left, right));
    }
    field_bounds = pane_bounds(field_bounds, caret, &rows)?;
    capture_status("Terminal changed during capture");
    let system = OwnedCf(AXUIElementCreateSystemWide());
    // Selection and cursor line are separate in terminals. Verify both twice;
    // reject a redraw or focus/pane switch between cross-process AX messages.
    if snapshot(focused.0, cursor, line)? != observed
        || range(focused.0) != Some(cursor)
        || cursor_line(focused.0, cursor) != Some(line)
        || front_identity() != (pid, app.clone())
        || IsSecureEventInputEnabled()
        || !CFEqual(attr(system.0, "AXFocusedUIElement")?.0, focused.0)
    {
        return None;
    }
    if manual_request {
        MANUAL.with(|m| {
            *m.borrow_mut() = Some(Lease {
                pid,
                focused: focused.clone(),
                scope: prompt.scope.clone(),
            })
        });
    }
    CAPTURE_DETAIL.with(|d| d.borrow_mut().caret_source = "terminal cursor");
    capture_status(if prompt.automatic {
        "AI terminal prompt ready"
    } else {
        "Manual terminal prompt ready"
    });
    Some(Context {
        element: focused.clone(),
        focused,
        insertion: InsertionMode::UnicodeEvent,
        pid,
        app,
        website: None,
        text: format!("{}{}", prompt.prefix, prompt.suffix),
        prefix: prompt.prefix,
        suffix: prompt.suffix,
        range: cursor,
        caret,
        field_bounds,
        docs_geometry: false,
        terminal: Some(prompt.scope),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }
    #[test]
    fn context_identity_ignores_neighbor_offsets_but_preserves_focus_and_editor_guards() {
        unsafe {
            let focus = cfstr("synthetic focused field identity");
            let make = |neighbor: &str| {
                let row = |right: &str| {
                    format!(
                        "{neighbor}{}│{right}",
                        " ".repeat(20 - unicode_width::UnicodeWidthStr::width(neighbor))
                    )
                };
                let raw = [
                    row("active output"),
                    row("› Explain this "),
                    row("90% context left"),
                ]
                .join("\n");
                let byte = raw.find("› Explain this ").unwrap() + "› Explain this ".len();
                let location = raw[..byte].encode_utf16().count();
                let p = crate::terminal::extract(&raw, location).unwrap();
                Context {
                    element: focus.clone(),
                    focused: focus.clone(),
                    insertion: InsertionMode::UnicodeEvent,
                    pid: 99,
                    app: "com.apple.Terminal".into(),
                    website: None,
                    text: format!("{}{}", p.prefix, p.suffix),
                    prefix: p.prefix,
                    suffix: p.suffix,
                    range: Range {
                        location: location as isize,
                        length: 0,
                    },
                    caret: rect(400., 200., 0., 20.),
                    field_bounds: rect(300., 100., 300., 300.),
                    docs_geometry: false,
                    terminal: Some(p.scope),
                }
            };
            let a = make("👩‍💻");
            let mut b = make("different output");
            assert_ne!(a.range, b.range);
            assert!(a.same(&b));
            b.focused = cfstr("different focused field");
            assert!(!a.same(&b));
            b.focused = focus.clone();
            b.prefix.push('x');
            assert!(!a.same(&b));
            let mut a = make("same");
            b = a.clone();
            a.terminal = None;
            b.terminal = None;
            assert!(a.same(&b));
            b.range.location += 1;
            assert!(!a.same(&b));
            b.range = a.range;
            b.text.push('x');
            assert!(!a.same(&b));
        }
    }
    #[test]
    fn pane_geometry_rejects_misalignment_gaps_and_wrong_cursor() {
        let field = rect(100., 100., 600., 500.);
        let caret = rect(420., 200., 0., 20.);
        let rows = vec![
            (Some(rect(400., 200., 10., 20.)), None),
            (Some(rect(400., 180., 10., 20.)), None),
            (Some(rect(400., 220., 10., 20.)), None),
        ];
        let pane = pane_bounds(field, caret, &rows).unwrap();
        assert_eq!(rect_coords(pane), (410., 180., 290., 60.));
        let mut changed = rows.clone();
        changed[1].0.as_mut().unwrap().origin.x += 10.;
        assert!(pane_bounds(field, caret, &changed).is_none());
        changed = rows.clone();
        changed[1].0.as_mut().unwrap().origin.y -= 20.;
        assert!(pane_bounds(field, caret, &changed).is_none());
        assert!(pane_bounds(field, rect(390., 200., 0., 20.), &rows).is_none());
        changed = rows.clone();
        changed[0].0.as_mut().unwrap().origin.y += 20.;
        assert!(pane_bounds(field, caret, &changed).is_none());
    }
    #[test]
    fn terminal_preview_stays_inside_even_a_narrow_pane() {
        let field = rect(400., 100., 90., 200.);
        let screen = rect(0., 0., 1200., 900.);
        let caret = rect(470., 180., 0., 20.);
        let p = preview_frame(caret, field, screen, 900.).unwrap();
        assert!(p.origin.x >= 404. && p.origin.x + p.size.width <= 486.);
        assert!(p.origin.y >= 602. && p.origin.y + p.size.height <= 798.);
        let below = preview_frame(rect(440., 100., 0., 20.), field, screen, 900.).unwrap();
        assert!(below.origin.y + below.size.height < 780.);
        assert!(preview_frame(caret, rect(400., 180., 90., 20.), screen, 900.).is_none());
    }
}
