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
    pub viewport: [f64; 2],
    pub field: Rect,
    pub caret: Rect,
    pub proxy_caret: Rect,
}
impl Snapshot {
    pub fn valid(&self) -> bool {
        let [width, height] = self.viewport;
        fingerprint(&self.page_key) && fingerprint(&self.text_hash)
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
    ) -> Option<(Rect, Rect)> {
        if !self.valid()
            || !native_proxy.valid()
            || !native_viewport.valid()
            || page_key(page).as_deref() != Some(self.page_key.as_str())
            || text.len() > 128 * 1024
            || hash(text) != self.text_hash
        {
            return None;
        }
        let xscale = native_viewport.2 / self.viewport[0];
        let yscale = native_viewport.3 / self.viewport[1];
        if !(0.25..=4.).contains(&xscale) || (xscale - yscale).abs() > 0.02 * xscale {
            return None;
        }
        let proxy = self.proxy_caret.mapped(native_viewport, xscale);
        if (proxy.0 - native_proxy.0).abs() > 2.
            || (proxy.1 - native_proxy.1).abs() > 2.
            || (proxy.3 - native_proxy.3).abs() > 3.
        {
            return None;
        }
        let caret = self.caret.mapped(native_viewport, xscale);
        let field = self.field.mapped(native_viewport, xscale);
        (native_viewport.contains(field) && field.contains(caret)).then_some((caret, field))
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
    fn sample() -> Snapshot {
        Snapshot {
            page_key: page_key("https://docs.google.com/document/d/synthetic_123/edit").unwrap(),
            text_hash: hash("Thank you for your\u{a0}"),
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
            s.resolve(page, "Thank you for your\u{a0}", proxy, viewport),
            Some((Rect(340., 280., 2., 18.), Rect(90., 200., 900., 680.)))
        );
        for wrong in [
            "https://docs.google.com.evil.example/document/d/synthetic_123/edit",
            "https://docs.google.com/spreadsheets/d/synthetic_123/edit",
            "https://docs.google.com/document/d/other_document/edit",
            "https://user@docs.google.com/document/d/synthetic_123/edit",
        ] {
            assert!(s
                .resolve(wrong, "Thank you for your\u{a0}", proxy, viewport)
                .is_none());
        }
        assert!(s
            .resolve(page, "Changed writing", proxy, viewport)
            .is_none());
        assert!(s
            .resolve(
                page,
                "Thank you for your\u{a0}",
                Rect(195., -9910., 0., 18.),
                viewport
            )
            .is_none());
        assert!(s
            .resolve(
                page,
                "Thank you for your\u{a0}",
                proxy,
                Rect(40., 80., 1000., 400.)
            )
            .is_none());
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
