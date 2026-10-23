// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Font metadata for the verified editor only. Missing names must not discard
//! a reported size (Chromium commonly supplies AXFontSize without AXFontName).
use super::*;

#[derive(Debug, PartialEq)]
struct Metadata {
    name: Option<String>,
    family: Option<String>,
    points: f64,
    traits: usize,
}

unsafe fn metadata(value: CF) -> Option<Metadata> {
    if value.is_null()
        || CFGetTypeID(value) != CFAttributedStringGetTypeID()
        || !(1..=16).contains(&CFAttributedStringGetLength(value))
    {
        return None;
    }
    let index = CFAttributedStringGetLength(value) - 1;
    let info = CFAttributedStringGetAttribute(value, index, kAXFontTextAttribute, ptr::null_mut());
    if info.is_null() || CFGetTypeID(info) != CFDictionaryGetTypeID() {
        return None;
    }
    let size = CFDictionaryGetValue(info, kAXFontSizeKey);
    let mut points = 0f64;
    if size.is_null()
        || CFGetTypeID(size) != CFNumberGetTypeID()
        || !CFNumberGetValue(size, 13, &mut points as *mut f64 as *mut c_void)
        || !points.is_finite()
        || !(4. ..=200.).contains(&points)
    {
        return None;
    }
    let text = |key: CF| {
        string(CFDictionaryGetValue(info, key)).filter(|s| !s.is_empty() && s.len() <= 256)
    };
    let flag = |key: &str| {
        let key = cfstr(key);
        let value = CFDictionaryGetValue(info, key.0);
        !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value)
    };
    Some(Metadata {
        name: text(kAXFontNameKey),
        family: text(kAXFontFamilyKey),
        points,
        traits: usize::from(flag("AXFontItalic")) | (usize::from(flag("AXFontBold")) << 1),
    })
}

unsafe fn fallback(points: f64, monospaced: bool) -> id {
    if monospaced {
        msg_send![class!(NSFont), monospacedSystemFontOfSize:points weight:0f64]
    } else {
        msg_send![class!(NSFont), systemFontOfSize:points]
    }
}

unsafe fn resolve(info: &Metadata, monospaced: bool) -> id {
    let mut font = nil;
    if let Some(name) = &info.name {
        let name = ns(name);
        font = msg_send![class!(NSFont), fontWithName:name size:info.points];
        let _: () = msg_send![name, release];
    }
    let manager: id = msg_send![class!(NSFontManager), sharedFontManager];
    if font == nil {
        if let Some(family) = &info.family {
            let family = ns(family);
            font = msg_send![manager, fontWithFamily:family traits:info.traits weight:5isize size:info.points];
            let _: () = msg_send![family, release];
        }
    }
    if font == nil {
        font = fallback(info.points, monospaced);
    }
    if info.traits != 0 {
        let converted: id = msg_send![manager, convertFont:font toHaveTrait:info.traits];
        if converted != nil {
            font = converted;
        }
    }
    font
}

// These optional macOS text-marker functions are also used by Chromium. Resolve
// them at runtime so an unavailable SPI disables this fallback, not the app.
type MarkerType = unsafe extern "C" fn() -> usize;
type MarkerStart = unsafe extern "C" fn(CF) -> CF;
unsafe fn start_marker(range: CF) -> Option<OwnedCf> {
    static API: std::sync::OnceLock<Option<(MarkerType, MarkerStart)>> = std::sync::OnceLock::new();
    let (kind, start) = API
        .get_or_init(|| {
            let kind = libc::dlsym(libc::RTLD_DEFAULT, c"AXTextMarkerRangeGetTypeID".as_ptr());
            let start = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"AXTextMarkerRangeCopyStartMarker".as_ptr(),
            );
            if kind.is_null() || start.is_null() {
                None
            } else {
                Some((
                    std::mem::transmute::<*mut c_void, MarkerType>(kind),
                    std::mem::transmute::<*mut c_void, MarkerStart>(start),
                ))
            }
        })
        .as_ref()?;
    if range.is_null() || CFGetTypeID(range) != kind() {
        return None;
    }
    let marker = start(range);
    (!marker.is_null()).then_some(OwnedCf(marker))
}

unsafe fn selected_start(ctx: &Context) -> Option<OwnedCf> {
    let current = attr(ctx.element.0, "AXSelectedTextRange")?;
    let mut current_range = Range {
        location: -1,
        length: -1,
    };
    if !ax_value(current.0, 4, &mut current_range) || current_range != ctx.range {
        return None;
    }
    let selected = attr(ctx.element.0, "AXSelectedTextMarkerRange")?;
    let selected_length =
        parameterized(ctx.element.0, "AXLengthForTextMarkerRange", selected.0).ok()?;
    let mut selected_units = -1i64;
    if CFGetTypeID(selected_length.0) != CFNumberGetTypeID()
        || !CFNumberGetValue(
            selected_length.0,
            4,
            &mut selected_units as *mut i64 as *mut c_void,
        )
        || selected_units != ctx.range.length as i64
    {
        return None;
    }
    start_marker(selected.0)
}

unsafe fn marker_metadata(ctx: &Context) -> Option<Metadata> {
    if ctx.prefix.is_empty() {
        return None;
    }
    let end = selected_start(ctx)?;
    let before = parameterized(ctx.element.0, "AXPreviousTextMarkerForTextMarker", end.0).ok()?;
    let markers: id = msg_send![class!(NSArray), arrayWithObject:before.0 as id];
    let markers: id = msg_send![markers, arrayByAddingObject:end.0 as id];
    let range = parameterized(
        ctx.element.0,
        "AXTextMarkerRangeForUnorderedTextMarkers",
        markers as CF,
    )
    .ok()?;
    let length = parameterized(ctx.element.0, "AXLengthForTextMarkerRange", range.0).ok()?;
    let mut units = 0i64;
    if CFGetTypeID(length.0) != CFNumberGetTypeID()
        || !CFNumberGetValue(length.0, 4, &mut units as *mut i64 as *mut c_void)
        || !(1..=16).contains(&units)
    {
        return None;
    }
    let value = parameterized(
        ctx.element.0,
        "AXAttributedStringForTextMarkerRange",
        range.0,
    )
    .ok()?;
    if CFGetTypeID(value.0) != CFAttributedStringGetTypeID()
        || CFAttributedStringGetLength(value.0) != units as isize
    {
        return None;
    }
    // A marker must refer to text immediately before this editor's cursor,
    // never an unrelated page leaf. No marker bytes or text are logged.
    let text = string(CFAttributedStringGetString(value.0))?;
    if !matching_sample(&ctx.prefix, &text) {
        return None;
    }
    metadata(value.0)
}

// Browser font sizes can be unzoomed CSS values. Compare a bounded adjacent
// word with its actual screen width instead of interpreting line height as size.
// This also leaves extra CSS line spacing out of the size calculation.
unsafe fn calibrated_font(ctx: &Context, info: &Metadata, font: id) -> Option<(id, &'static str)> {
    if ctx.terminal.is_some() {
        return None;
    }
    let spaces = ctx.prefix.chars().rev().take_while(|c| *c == ' ').count();
    if spaces > 4 {
        return None;
    }
    let mut end = selected_start(ctx)?;
    for _ in 0..spaces {
        end = parameterized(ctx.element.0, "AXPreviousTextMarkerForTextMarker", end.0).ok()?;
    }
    let word = parameterized(
        ctx.element.0,
        "AXLeftWordTextMarkerRangeForTextMarker",
        end.0,
    )
    .ok()?;
    let length = parameterized(ctx.element.0, "AXLengthForTextMarkerRange", word.0).ok()?;
    let mut units = 0i64;
    if CFGetTypeID(length.0) != CFNumberGetTypeID()
        || !CFNumberGetValue(length.0, 4, &mut units as *mut i64 as *mut c_void)
        || !(3..=16).contains(&units)
    {
        return None;
    }
    let value = parameterized(
        ctx.element.0,
        "AXAttributedStringForTextMarkerRange",
        word.0,
    )
    .ok()?;
    let word_info = metadata(value.0)?;
    if word_info != *info || CFAttributedStringGetLength(value.0) != units as isize {
        return None;
    }
    let text = string(CFAttributedStringGetString(value.0))?;
    if !calibration_sample(&ctx.prefix, &text) {
        return None;
    }
    // Mixed-font words cannot calibrate one font reliably.
    let mut run = Range {
        location: 0,
        length: 0,
    };
    CFAttributedStringGetAttribute(value.0, 0, kAXFontTextAttribute, &mut run);
    if run.location != 0 || run.length != units as isize {
        return None;
    }
    let bounds = parameterized(ctx.element.0, "AXBoundsForTextMarkerRange", word.0).ok()?;
    let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
    if !ax_value(bounds.0, 3, &mut rect) {
        return None;
    }
    let attrs: id =
        msg_send![class!(NSDictionary), dictionaryWithObject:font forKey:NSFontAttributeName];
    let string = ns(&text);
    let measured: NSSize = msg_send![string, sizeWithAttributes:attrs];
    let _: () = msg_send![string, release];
    let (points, source) = calibrated_size(info.points, measured, rect, ctx.caret)?;
    let resized: id = msg_send![class!(NSFontManager), sharedFontManager];
    let resized: id = msg_send![resized, convertFont:font toSize:points];
    (resized != nil).then_some((resized, source))
}

fn calibration_sample(prefix: &str, sample: &str) -> bool {
    (3..=16).contains(&sample.len())
        && sample.bytes().all(|b| b.is_ascii_alphabetic())
        && prefix.trim_end_matches(' ').ends_with(sample)
}

fn calibrated_size(
    points: f64,
    measured: NSSize,
    rect: NSRect,
    caret: NSRect,
) -> Option<(f64, &'static str)> {
    if !valid_rect(rect)
        || !valid_rect(caret)
        || !measured.width.is_finite()
        || !measured.height.is_finite()
        || measured.width <= 0.
        || measured.height <= 0.
        || rect.size.width <= 0.
        || rect.origin.x + rect.size.width > caret.origin.x + 1.
        || rect.origin.y + rect.size.height < caret.origin.y
        || rect.origin.y > caret.origin.y + caret.size.height
        || rect.size.height > caret.size.height + 1.
    {
        return None;
    }
    let width_scale = rect.size.width / measured.width;
    let height_scale = rect.size.height / measured.height;
    if !(0.25..=4.).contains(&width_scale) || !(0.25..=4.).contains(&height_scale) {
        return None;
    }
    // A substitute font can be narrower than the editor's face, and some
    // mixed-style marker bounds include adjacent space. Neither is evidence
    // that the font should be enlarged. Require the word's actual height to
    // corroborate width scaling; the caret/line height is deliberately unused.
    let (scale, source) = if (measured.height * width_scale - rect.size.height).abs()
        <= (rect.size.height * 0.06).max(1.5)
    {
        (width_scale, "metadata with measured word width")
    } else if (rect.size.height - measured.height).abs() <= (measured.height * 0.08).max(2.) {
        (1., "metadata size (font metric mismatch)")
    } else if (width_scale - 1.) * (height_scale - 1.) > 0.
        && (0.7..=1.3).contains(&(height_scale / width_scale))
    {
        // Both axes indicate zoom, but different glyph advances distort the
        // width ratio. Use the bounded word height, not extra CSS line spacing.
        (height_scale, "metadata with measured word height")
    } else {
        return None;
    };
    let result = points * scale;
    (points.is_finite() && (4. ..=200.).contains(&result)).then_some((result, source))
}

fn matching_sample(prefix: &str, sample: &str) -> bool {
    !sample.is_empty()
        && sample.encode_utf16().count() <= 16
        && !sample.chars().any(char::is_control)
        && prefix.ends_with(sample)
}

pub(super) unsafe fn font(ctx: &Context) -> id {
    let length = ctx.prefix.chars().next_back().map_or(0, |c| c.len_utf16()) as isize;
    let range_info = if length > 0 && ctx.range.location >= length {
        let range = Range {
            location: ctx.range.location - length,
            length,
        };
        let parameter = OwnedCf(AXValueCreate(4, &range as *const Range as *const c_void));
        parameterized(ctx.element.0, "AXAttributedStringForRange", parameter.0)
            .ok()
            .and_then(|value| metadata(value.0))
    } else {
        None
    };
    let info = range_info
        .map(|info| (info, "range metadata"))
        .or_else(|| marker_metadata(ctx).map(|info| (info, "text marker metadata")));
    let (font, source) = if let Some((info, source)) = info {
        let font = resolve(&info, ctx.terminal.is_some());
        match calibrated_font(ctx, &info, font) {
            Some(calibrated) => calibrated,
            None => (font, source),
        }
    } else {
        (
            fallback(
                (ctx.caret.size.height * 0.78).clamp(8., 72.),
                ctx.terminal.is_some(),
            ),
            "caret estimate",
        )
    };
    let points: f64 = msg_send![font, pointSize];
    FONT_DIAGNOSTIC.with(|d| d.set((source, points)));
    font
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn sample(size: id, name: Option<&str>) -> OwnedCf {
        let dict: id = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![dict, setObject:size forKey:kAXFontSizeKey as id];
        if let Some(name) = name {
            let name = ns(name);
            let _: () = msg_send![dict, setObject:name forKey:kAXFontNameKey as id];
            let _: () = msg_send![name, release];
        }
        let attrs: id = msg_send![class!(NSDictionary), dictionaryWithObject:dict forKey:kAXFontTextAttribute as id];
        let text = ns("x");
        let result: id = msg_send![class!(NSAttributedString), alloc];
        let result: id = msg_send![result, initWithString:text attributes:attrs];
        let _: () = msg_send![text, release];
        OwnedCf(result as CF)
    }

    #[test]
    fn chromium_size_without_name_survives_and_invalid_sizes_are_rejected() {
        unsafe {
            let pool: id = msg_send![class!(NSAutoreleasePool), new];
            for points in [9., 13., 16., 24., 40.] {
                let size: id = msg_send![class!(NSNumber), numberWithDouble:points];
                let value = sample(size, None);
                let info = metadata(value.0).unwrap();
                assert_eq!(info.points, points);
                assert!(info.name.is_none());
                let resolved = resolve(&info, false);
                let actual: f64 = msg_send![resolved, pointSize];
                assert_eq!(actual, points);
            }
            for points in [f64::NAN, f64::INFINITY, -1., 0., 201.] {
                let size: id = msg_send![class!(NSNumber), numberWithDouble:points];
                assert!(metadata(sample(size, None).0).is_none());
            }
            let size = ns("16");
            assert!(metadata(sample(size, None).0).is_none());
            let _: () = msg_send![size, release];
            let size: id = msg_send![class!(NSNumber), numberWithDouble:17f64];
            let info = metadata(sample(size, Some("LibreTypeNonexistentTestFont")).0).unwrap();
            let font = resolve(&info, false);
            let actual: f64 = msg_send![font, pointSize];
            assert_eq!(actual, 17.);
            let _: () = msg_send![pool, drain];
        }
    }

    #[test]
    fn marker_sample_must_match_adjacent_text_and_remain_bounded() {
        assert!(matching_sample("Hello ", " "));
        assert!(matching_sample("Café", "é"));
        assert!(matching_sample("Cafe\u{301}", "e\u{301}"));
        assert!(matching_sample("Hello 👩‍💻", "👩‍💻"));
        for (prefix, sample) in [
            ("Hello", ""),
            ("Hello", "other"),
            ("", "x"),
            ("Hello\n", "\n"),
            ("12345678901234567", "12345678901234567"),
        ] {
            assert!(!matching_sample(prefix, sample));
        }
    }

    #[test]
    fn font_calibration_uses_word_width_not_line_spacing_and_rejects_wrapping() {
        let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        for scale in [0.75, 1., 1.25, 2.] {
            let word = rect(100., 100., 50. * scale, 20. * scale);
            for extra_spacing in [0., 10., 30.] {
                let caret = rect(100. + 50. * scale, 100., 0., 20. * scale + extra_spacing);
                assert_eq!(
                    calibrated_size(16., NSSize::new(50., 20.), word, caret).map(|v| v.0),
                    Some(16. * scale)
                );
            }
        }
        let caret = rect(160., 100., 0., 20.);
        for word in [
            rect(100., 100., 50., 40.),
            rect(100., 200., 50., 20.),
            rect(100., 100., 90., 20.),
            rect(100., 100., 0., 20.),
        ] {
            assert!(calibrated_size(16., NSSize::new(50., 20.), word, caret).is_none());
        }
        assert!(calibrated_size(
            16.,
            NSSize::new(f64::NAN, 20.),
            rect(100., 100., 50., 20.),
            caret
        )
        .is_none());
        assert!(
            calibrated_size(16., NSSize::new(1., 20.), rect(100., 100., 50., 20.), caret).is_none()
        );
        assert!(calibration_sample("Thank you for your ", "your"));
        assert!(!calibration_sample("Thank you for your ", "for"));
        assert!(!calibration_sample("Test 👩‍💻", "👩‍💻"));
        assert!(!calibration_sample("Example of ", "of"));
    }

    #[test]
    fn different_glyph_widths_do_not_masquerade_as_font_zoom() {
        let check = |points, measured, width, height| {
            calibrated_size(
                points,
                measured,
                NSRect::new(NSPoint::new(100., 100.), NSSize::new(width, height)),
                NSRect::new(
                    NSPoint::new(100. + width, 100.),
                    NSSize::new(0., height + 30.),
                ),
            )
        };
        // AppKit measurements and bounds inferred from the observed build-28
        // miscalibrations. Text height agrees with the original reported size.
        assert_eq!(
            check(24., NSSize::new(45.335615158081055, 28.), 51., 29.),
            Some((24., "metadata size (font metric mismatch)"))
        );
        assert_eq!(
            check(18., NSSize::new(35.45580339431763, 21.), 43., 21.),
            Some((18., "metadata size (font metric mismatch)"))
        );
        // The same width mismatch under actual zoom must still scale the font.
        for scale in [0.75, 1.5, 2.] {
            let result = check(
                18.,
                NSSize::new(35.45580339431763, 21.),
                43. * scale,
                21. * scale,
            )
            .unwrap();
            assert_eq!(result, (18. * scale, "metadata with measured word height"));
        }
        // Bad height or grossly inconsistent axes cannot justify calibration.
        for height in [0., f64::NAN, f64::INFINITY] {
            assert!(check(18., NSSize::new(35., height), 43., 21.).is_none());
        }
        assert!(check(18., NSSize::new(35., 21.), 43., 70.).is_none());
    }
}
