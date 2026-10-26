// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Geometry and bounded text selection, independent of any screen access.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub id: u32,
    pub pid: i32,
    pub bounds: [f64; 4],
    pub layer: i32,
    pub visible: bool,
}
impl Window {
    pub fn valid(&self) -> bool {
        self.id != 0
            && self.pid > 0
            && self.visible
            && self.layer == 0
            && self.bounds.iter().all(|v| v.is_finite())
            && (32.0..=20_000.0).contains(&self.bounds[2])
            && (32.0..=20_000.0).contains(&self.bounds[3])
    }
    pub fn normalized_caret(&self, caret: [f64; 2]) -> Option<[f64; 2]> {
        if !self.valid() || !caret.iter().all(|v| v.is_finite()) {
            return None;
        }
        let [x, y, w, h] = self.bounds;
        let position = [(caret[0] - x) / w, 1.0 - (caret[1] - y) / h];
        position
            .iter()
            .all(|v| (0.0..=1.0).contains(v))
            .then_some(position)
    }
    pub fn pixels(&self) -> (usize, usize) {
        let scale = 2.0_f64
            .min(4096.0 / self.bounds[2])
            .min(3072.0 / self.bounds[3]);
        (
            (self.bounds[2] * scale).round() as usize,
            (self.bounds[3] * scale).round() as usize,
        )
    }
}
/// Window lists arrive front to back. Do not fall through to a different window
/// of the app if the frontmost normal window doesn't contain the caret.
pub fn choose(windows: &[Window], pid: i32, caret: [f64; 2]) -> Option<Window> {
    let window = windows
        .iter()
        .find(|w| w.pid == pid && w.layer == 0 && w.visible)?;
    window.normalized_caret(caret).map(|_| *window)
}

/// An accessibility window anchors selection to the window containing the
/// focused editor. Auxiliary windows can precede it in the window-server list.
/// Never select a different window merely because it happens to cover the caret.
pub fn choose_focused(
    windows: &[Window],
    pid: i32,
    caret: [f64; 2],
    focused_bounds: [f64; 4],
) -> Option<Window> {
    if !focused_bounds.iter().all(|v| v.is_finite())
        || focused_bounds[2] < 32.0
        || focused_bounds[3] < 32.0
    {
        return None;
    }
    let mut matches = windows.iter().filter(|w| {
        w.pid == pid
            && w.valid()
            && w.bounds
                .iter()
                .zip(focused_bounds)
                .all(|(a, b)| (a - b).abs() <= 2.0)
            && w.normalized_caret(caret).is_some()
    });
    let window = *matches.next()?;
    // Geometry alone cannot distinguish coincident windows belonging to an app.
    matches.next().is_none().then_some(window)
}
pub struct Line {
    pub text: String,
    /// Vision coordinates: normalized, origin at bottom left.
    pub bounds: [f64; 4],
}
pub fn nearby_text(mut lines: Vec<Line>, caret: [f64; 2]) -> String {
    lines.retain(|line| !line.text.trim().is_empty() && line.bounds.iter().all(|v| v.is_finite()));
    let distance = |line: &Line| {
        let [x, y, w, h] = line.bounds;
        let dx = (x - caret[0]).max(0.0).max(caret[0] - x - w);
        let dy = (y - caret[1]).max(0.0).max(caret[1] - y - h);
        dx * dx + dy * dy
    };
    lines.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    let mut used = 0;
    lines.retain(|line| {
        let count = line.text.chars().count() + 1;
        if used + count > crate::background::MAX_CHARS {
            return false;
        }
        used += count;
        true
    });
    lines.sort_by(|a, b| {
        b.bounds[1]
            .total_cmp(&a.bounds[1])
            .then(a.bounds[0].total_cmp(&b.bounds[0]))
    });
    lines
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focused_window_anchor_skips_auxiliary_windows_without_guessing() {
        let focused = Window {
            id: 10,
            pid: 42,
            bounds: [-1200., -300., 1000., 700.],
            layer: 0,
            visible: true,
        };
        let auxiliary = Window {
            id: 11,
            bounds: [0., 0., 200., 100.],
            ..focused
        };
        let caret = [-700., 50.];
        assert_eq!(choose(&[auxiliary, focused], 42, caret), None);
        assert_eq!(
            choose_focused(&[auxiliary, focused], 42, caret, focused.bounds),
            Some(focused)
        );
        assert_eq!(choose_focused(&[focused], 43, caret, focused.bounds), None);
        assert_eq!(
            choose_focused(&[focused], 42, [500., 300.], focused.bounds),
            None
        );
        assert_eq!(
            choose_focused(&[focused], 42, caret, auxiliary.bounds),
            None
        );
        for invalid in [
            Window {
                visible: false,
                ..focused
            },
            Window {
                layer: 3,
                ..focused
            },
            Window {
                bounds: [-1203., -300., 1000., 700.],
                ..focused
            },
        ] {
            assert_eq!(choose_focused(&[invalid], 42, caret, focused.bounds), None);
        }
        let duplicate = Window { id: 12, ..focused };
        assert_eq!(
            choose_focused(&[focused, duplicate], 42, caret, focused.bounds),
            None
        );
        assert_eq!(
            choose_focused(&[focused], 42, caret, [f64::NAN, 0., 1000., 700.]),
            None
        );
    }
    #[test]
    fn window_selection_preserves_identity_on_secondary_displays() {
        let front = Window {
            id: 5,
            pid: 42,
            bounds: [-1200., -300., 1000., 700.],
            layer: 0,
            visible: true,
        };
        let behind = Window {
            id: 6,
            bounds: [0., 0., 1000., 700.],
            ..front
        };
        assert_eq!(choose(&[front, behind], 42, [-700., 50.]), Some(front));
        assert!(choose(&[front, behind], 42, [500., 300.]).is_none());
        assert!(choose(&[front], 43, [-700., 50.]).is_none());
        assert_eq!(front.normalized_caret([-700., 50.]), Some([0.5, 0.5]));
        assert!(Window {
            bounds: [0., 0., f64::NAN, 100.],
            ..front
        }
        .normalized_caret([0., 0.])
        .is_none());
        let huge = Window {
            bounds: [0., 0., 10000., 8000.],
            ..front
        };
        let (w, h) = huge.pixels();
        assert!(w <= 4096 && h <= 3072);
    }
    #[test]
    fn nearest_lines_are_bounded_and_return_to_reading_order() {
        let lines = (0..30)
            .map(|i| Line {
                text: format!("line {i} {}", "x".repeat(90)),
                bounds: [0.1, i as f64 / 30., 0.8, 0.02],
            })
            .collect();
        let text = nearby_text(lines, [0.5, 0.9]);
        assert!(text.contains("line 27 "));
        assert!(!text.contains("line 0 "));
        assert!(text.chars().count() <= crate::background::MAX_CHARS);
        let ids: Vec<u32> = text
            .lines()
            .map(|line| line.split_whitespace().nth(1).unwrap().parse().unwrap())
            .collect();
        assert!(ids.windows(2).all(|pair| pair[0] > pair[1]));
    }
}
