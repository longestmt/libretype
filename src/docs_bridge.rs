// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Optional Google Docs geometry transport. Never transports document text.
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

pub const EXTENSION_ID: &str = include_str!("../companion/google-docs/extension-id.txt");
pub const HOST_NAME: &str = "org.libretype.docs";
pub const MAX_FRAME: usize = 8192;
pub const MAX_AGE: Duration = Duration::from_millis(800);

/// The matched visible and braille carets represent the same insertion point.
/// Visible height is in native points; proxy height is in browser CSS units.
/// Their ratio includes Docs and browser zoom, absent from braille font metadata.
/// This is not a font estimate from arbitrary line height.
pub fn paired_caret_scale(visible_height: f64, proxy_height: f64) -> Option<f64> {
    if !visible_height.is_finite() || !proxy_height.is_finite()
        || !(5. ..=600.).contains(&visible_height) || !(5. ..=600.).contains(&proxy_height) {
        return None;
    }
    let scale = visible_height / proxy_height;
    (0.25..=4.).contains(&scale).then_some(scale)
}

/// A posted edit invalidates the old text fingerprint before the next browser
/// observation. This permits waiting only; callers still validate live focus,
/// and no chunk may be confirmed or reposted without fresh native text/caret.
pub fn may_wait_after_insertion(used_companion: bool, enabled: bool, elapsed: Duration, reason: &str) -> bool {
    used_companion && enabled && elapsed < Duration::from_secs(1)
        && matches!(reason, "Caret outside visible editor" | "Caret bounds unavailable" | "Text field bounds unavailable")
}

pub fn hash(text: &str) -> String {
    digest(&SHA256, text.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn page_key(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("docs.google.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let path: Vec<_> = url.path_segments()?.collect();
    if path.len() != 4
        || path[0] != "document"
        || path[1] != "d"
        || path[3] != "edit"
        || !(10..=200).contains(&path[2].len())
        || !path[2]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return None;
    }
    Some(hash(&format!("https://docs.google.com{}", url.path())))
}
fn fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect(pub f64, pub f64, pub f64, pub f64);
impl Rect {
    fn valid(self) -> bool {
        [self.0, self.1, self.2, self.3]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 100_000.)
            && self.2 >= 0.
            && self.3 > 0.
    }
    fn contains(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && self.2 > 0.
            && other.0 >= self.0 - 1.
            && other.1 >= self.1 - 1.
            && other.0 + other.2 <= self.0 + self.2 + 1.
            && other.1 + other.3 <= self.1 + self.3 + 1.
    }
    fn mapped(self, origin: Self, scale: f64) -> Self {
        Self(
            origin.0 + self.0 * scale,
            origin.1 + self.1 * scale,
            self.2 * scale,
            self.3 * scale,
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub page_key: String,
    pub text_hash: String,
    pub selection_utf16: usize,
    pub viewport: [f64; 2],
    pub field: Rect,
    pub caret: Rect,
    pub proxy_caret: Rect,
}
impl Snapshot {
    pub fn valid(&self) -> bool {
        let [width, height] = self.viewport;
        fingerprint(&self.page_key) && fingerprint(&self.text_hash) && self.selection_utf16 <= 32768
            && width.is_finite() && height.is_finite()
            && (100. ..=20_000.).contains(&width) && (100. ..=20_000.).contains(&height)
            && Rect(0.,0.,width,height).contains(self.field) && self.field.contains(self.caret)
            && self.caret.2 <= 8. && (5. ..=150.).contains(&self.caret.3)
            && self.proxy_caret.valid() && self.proxy_caret.2 <= 8.
            && (5. ..=150.).contains(&self.proxy_caret.3)
            // Only the known off-screen Docs braille surface can be replaced.
            && self.proxy_caret.1 < -1000.
    }
    pub fn resolve(
        &self,
        page: &str,
        text: &str,
        native_proxy: Rect,
        native_viewport: Rect,
        native_selection: usize,
    ) -> Option<(Rect, Rect)> {
        self.resolve_checked(page, text, native_proxy, native_viewport, native_selection).ok()
    }
    pub fn resolve_checked(
        &self,
        page: &str,
        text: &str,
        native_proxy: Rect,
        native_viewport: Rect,
        native_selection: usize,
    ) -> Result<(Rect, Rect), &'static str> {
        if !self.valid()
            || !native_proxy.valid()
            || !native_viewport.valid()
        {
            return Err("Invalid companion or native geometry");
        }
        if page_key(page).as_deref() != Some(self.page_key.as_str()) {
            return Err("Document identity mismatch");
        }
        if text.len() > 128 * 1024 || hash(text) != self.text_hash {
            return Err("Accessible text fingerprint mismatch");
        }
        if self.selection_utf16 != native_selection || native_selection > text.encode_utf16().count() {
            return Err("Caret offset mismatch");
        }
        let xscale = native_viewport.2 / self.viewport[0];
        let yscale = native_viewport.3 / self.viewport[1];
        if !(0.25..=4.).contains(&xscale) || (xscale - yscale).abs() > 0.02 * xscale {
            return Err("Viewport scale mismatch");
        }
        let proxy = self.proxy_caret.mapped(native_viewport, xscale);
        // Chromium's off-screen braille coordinates can diverge from DOM
        // coordinates as Docs shifts its paragraph window. The exact text and
        // UTF-16 caret match above establish identity; retain the hidden-surface
        // and font-height checks instead of relying on those X/Y coordinates.
        if (native_proxy.1 - native_viewport.1) / xscale >= -1000.
            || (proxy.3 - native_proxy.3).abs() > 3.
        {
            return Err("Hidden caret bounds mismatch");
        }
        let caret = self.caret.mapped(native_viewport, xscale);
        let field = self.field.mapped(native_viewport, xscale);
        (native_viewport.contains(field) && field.contains(caret)).then_some((caret, field))
            .ok_or("Companion caret outside viewport")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Ping {},
    Clear {},
    Observe { snapshot: Snapshot },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub enabled: bool,
    pub accepted: bool,
}

#[derive(Default)]
pub struct Cache {
    enabled: AtomicBool,
    latest: Mutex<Option<(Instant, Snapshot)>>,
}
impl Cache {
    pub fn set_enabled(&self, enabled: bool) {
        if let Ok(mut latest) = self.latest.lock() {
            if self.enabled.swap(enabled, Ordering::AcqRel) != enabled || !enabled {
                *latest = None;
            }
        }
    }
    pub fn receive(&self, request: Request) -> Reply {
        let mut accepted = false;
        let mut enabled = false;
        if let Ok(mut latest) = self.latest.lock() {
            enabled = self.enabled.load(Ordering::Acquire);
            match request {
                Request::Ping {} => {}
                Request::Clear {} => {
                    *latest = None;
                    accepted = true;
                }
                Request::Observe { snapshot } => {
                    if enabled && snapshot.valid() {
                        *latest = Some((Instant::now(), snapshot));
                        accepted = true;
                    } else {
                        *latest = None;
                    }
                }
            }
        }
        Reply { enabled, accepted }
    }
    pub fn current(&self) -> Option<Snapshot> {
        if !self.enabled.load(Ordering::Acquire) {
            return None;
        }
        self.latest
            .lock()
            .ok()?
            .as_ref()
            .filter(|(time, _)| time.elapsed() <= MAX_AGE)
            .map(|(_, s)| s.clone())
    }
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0u8; 4];
    match reader.read(&mut header[..1])? {
        0 => return Ok(None),
        _ => reader.read_exact(&mut header[1..])?,
    }
    let length = u32::from_ne_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid bridge frame size",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}
pub fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Bridge response too large",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(unix)]
pub mod transport;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matched_carets_capture_document_zoom_without_using_absolute_line_height() {
        let normal = 16.859375;
        assert_eq!(paired_caret_scale(normal, normal), Some(1.));
        let measured = paired_caret_scale(25.296875, normal).unwrap();
        assert!((measured * (44. / 3.) - 22.).abs() < 0.02);
        // A different font or line height shared by both carets does not change
        // the ratio. Browser zoom is included in the mapped visible height.
        for base in [10., 20., 40.] {
            for zoom in [0.5, 1., 1.5, 2.] {
                assert_eq!(paired_caret_scale(base * zoom, base), Some(zoom));
            }
        }
        assert_eq!(paired_caret_scale(18. * 1.5 * 1.25, 18.), Some(1.875));
        for (visible, proxy) in [(0., normal), (normal, 0.), (f64::NAN, normal),
            (normal, f64::INFINITY), (100., 10.), (5., 100.)] {
            assert!(paired_caret_scale(visible, proxy).is_none());
        }
    }
    #[test]
    fn delayed_geometry_only_waits_for_known_docs_edits_and_never_reposts_a_chunk() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let delay = Duration::from_millis(250);
        assert!(may_wait_after_insertion(true, true, delay, "Caret outside visible editor"));
        assert!(!may_wait_after_insertion(false, true, delay, "Caret outside visible editor"));
        assert!(!may_wait_after_insertion(true, false, delay, "Caret outside visible editor"));
        assert!(!may_wait_after_insertion(true, true, Duration::from_secs(1), "Caret outside visible editor"));
        for reason in ["Secure input enabled", "Focus changed during capture", "Editable ancestor unavailable", "Application excluded"] {
            assert!(!may_wait_after_insertion(true, true, delay, reason));
        }
        let mut plan = AcceptancePlan::new("We should ", "tomorrow.",
            "meet in the small conference room ", true, true, false).unwrap();
        let first = plan.next_chunk().unwrap();
        // Several missing geometry observations cannot retry the posted chunk.
        for _ in 0..3 {
            assert!(may_wait_after_insertion(true, true, delay, "Caret bounds unavailable"));
            assert!(plan.next_chunk().is_none());
        }
        assert_eq!(plan.observe(&format!("We should {first}"), "tomorrow."), Observation::Confirmed);
        assert!(plan.next_chunk().is_some());
        assert_eq!(plan.observe("unexpected writing", "tomorrow."), Observation::Diverged);
    }
    fn sample() -> Snapshot {
        Snapshot {
            page_key: page_key("https://docs.google.com/document/d/synthetic_123/edit").unwrap(),
            text_hash: hash("Thank you for your\u{a0}"),
            selection_utf16: 19,
            viewport: [1000., 800.],
            field: Rect(50., 120., 900., 680.),
            caret: Rect(300., 200., 2., 18.),
            proxy_caret: Rect(150., -9990., 0., 18.),
        }
    }
    #[test]
    fn document_origin_path_text_and_proxy_caret_must_all_match() {
        let s = sample();
        let page = "https://docs.google.com/document/d/synthetic_123/edit?tab=t.0";
        let viewport = Rect(40., 80., 1000., 800.);
        let proxy = Rect(190., -9910., 0., 18.);
        assert_eq!(
            s.resolve(page, "Thank you for your\u{a0}", proxy, viewport, 19),
            Some((Rect(340., 280., 2., 18.), Rect(90., 200., 900., 680.)))
        );
        for wrong in [
            "https://docs.google.com.evil.example/document/d/synthetic_123/edit",
            "https://docs.google.com/spreadsheets/d/synthetic_123/edit",
            "https://docs.google.com/document/d/other_document/edit",
            "https://user@docs.google.com/document/d/synthetic_123/edit",
        ] {
            assert!(s
                .resolve(wrong, "Thank you for your\u{a0}", proxy, viewport, 19)
                .is_none());
        }
        assert!(s
            .resolve(page, "Changed writing", proxy, viewport, 19)
            .is_none());
        assert!(s
            .resolve(
                page,
                "Thank you for your\u{a0}",
                proxy,
                viewport,
                18
            )
            .is_none());
        assert!(s
            .resolve(
                page,
                "Thank you for your\u{a0}",
                proxy,
                Rect(40., 80., 1000., 400.),
                19
            )
            .is_none());
        // A shifted hidden paragraph window is safe only with exact current
        // text and caret identity; a visible native control must never match.
        assert!(s.resolve(page, "Thank you for your\u{a0}", Rect(250., -9500., 0., 18.), viewport, 19).is_some());
        assert!(s.resolve(page, "Thank you for your\u{a0}", Rect(250., 200., 0., 18.), viewport, 19).is_none());
    }
    #[test]
    fn off_stale_cleared_or_invalid_snapshots_never_override_native_geometry() {
        let cache = Cache::default();
        assert!(
            !cache
                .receive(Request::Observe { snapshot: sample() })
                .accepted
        );
        cache.set_enabled(true);
        assert!(
            cache
                .receive(Request::Observe { snapshot: sample() })
                .accepted
        );
        assert!(cache.current().is_some());
        cache.latest.lock().unwrap().as_mut().unwrap().0 =
            Instant::now() - MAX_AGE - Duration::from_millis(1);
        assert!(cache.current().is_none());
        cache.receive(Request::Observe { snapshot: sample() });
        cache.receive(Request::Clear {});
        assert!(cache.current().is_none());
        cache.receive(Request::Observe { snapshot: sample() });
        cache.set_enabled(false);
        cache.set_enabled(true);
        assert!(cache.current().is_none());
        let mut invalid = sample();
        invalid.caret.0 = 2000.;
        assert!(
            !cache
                .receive(Request::Observe { snapshot: invalid })
                .accepted
        );
    }
    #[test]
    fn bounded_native_frames_reject_truncation_oversize_and_extra_fields() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Request::Ping {}).unwrap();
        let result = read_frame(&mut bytes.as_slice()).unwrap().unwrap();
        assert!(matches!(
            serde_json::from_slice::<Request>(&result).unwrap(),
            Request::Ping {}
        ));
        assert!(read_frame(&mut &bytes[..3]).is_err());
        assert!(read_frame(&mut &bytes[..bytes.len() - 1]).is_err());
        assert!(read_frame(&mut ((MAX_FRAME + 1) as u32).to_ne_bytes().as_slice()).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"type":"ping","text":"not allowed"}"#).is_err()
        );
        assert!(read_frame(&mut &b""[..]).unwrap().is_none());
    }
}
