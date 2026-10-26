// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! One text-free timing sample, from an observed typing event to a visible draw.
use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) struct Input {
    pid: i32,
    at: Instant,
    captured: Option<Instant>,
}
struct Cycle {
    id: u64,
    input: Input,
    requested: Instant,
    reply: Option<(Instant, Instant, Instant)>,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct Sample {
    pub sequence: u64,
    pub event_to_capture_ms: f64,
    pub capture_to_request_ms: f64,
    pub request_to_worker_ms: f64,
    pub worker_ms: f64,
    pub worker_to_reply_ms: f64,
    pub reply_to_draw_ms: f64,
    pub event_to_draw_ms: f64,
}
#[derive(Default)]
pub(super) struct Tracker {
    input: Option<Input>,
    cycle: Option<Cycle>,
    last: Option<Sample>,
}
impl Tracker {
    pub fn cancel(&mut self) {
        self.input = None;
        self.cycle = None;
    }
    pub fn input(&mut self, pid: i32, at: Instant) {
        self.cancel();
        self.input = Some(Input {
            pid,
            at,
            captured: None,
        });
    }
    pub fn take_unbound(&mut self) -> Option<Input> {
        self.input.take().filter(|input| input.captured.is_none())
    }
    pub fn bind(&mut self, input: Option<Input>, pid: i32, at: Instant) {
        self.input = input
            .filter(|input| input.pid == pid && input.at <= at)
            .map(|input| Input {
                captured: Some(at),
                ..input
            });
    }
    pub fn request(&mut self, id: u64, at: Instant) {
        self.cycle = self
            .input
            .take()
            .filter(|input| input.captured.is_some_and(|c| c <= at))
            .map(|input| Cycle {
                id,
                input,
                requested: at,
                reply: None,
            });
    }
    pub fn reply(&mut self, id: u64, started: Instant, ended: Instant, received: Instant) {
        if let Some(cycle) = self.cycle.as_mut().filter(|c| c.id == id) {
            if cycle.requested <= started && started <= ended && ended <= received {
                cycle.reply = Some((started, ended, received));
            } else {
                self.cycle = None;
            }
        }
    }
    pub fn drawable_id(&self) -> Option<u64> {
        self.cycle
            .as_ref()
            .filter(|c| c.reply.is_some())
            .map(|c| c.id)
    }
    pub fn draw(&mut self, id: u64, pid: i32, at: Instant) {
        let Some(cycle) = self.cycle.as_ref().filter(|c| c.id == id) else {
            return;
        };
        if cycle.input.pid != pid {
            self.cycle = None;
            return;
        }
        let Some((started, ended, received)) =
            cycle.reply.filter(|(_, _, received)| *received <= at)
        else {
            return;
        };
        let captured = cycle
            .input
            .captured
            .expect("requested cycles have a capture");
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f64() * 1000.;
        self.last = Some(Sample {
            sequence: self.last.map_or(1, |s| s.sequence.saturating_add(1)),
            event_to_capture_ms: ms(cycle.input.at, captured),
            capture_to_request_ms: ms(captured, cycle.requested),
            request_to_worker_ms: ms(cycle.requested, started),
            worker_ms: ms(started, ended),
            worker_to_reply_ms: ms(ended, received),
            reply_to_draw_ms: ms(received, at),
            event_to_draw_ms: ms(cycle.input.at, at),
        });
        self.cycle = None;
    }
    pub fn last(&self) -> Option<Sample> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }
    fn requested(t: &mut Tracker, start: Instant, id: u64) {
        t.input(10, start);
        let input = t.take_unbound();
        t.cancel();
        t.bind(input, 10, at(start, 100));
        t.request(id, at(start, 400));
    }
    #[test]
    fn records_each_phase_once_and_keeps_only_the_last_numeric_sample() {
        let start = Instant::now();
        let mut t = Tracker::default();
        requested(&mut t, start, 7);
        assert!(t.drawable_id().is_none());
        t.reply(7, at(start, 450), at(start, 1450), at(start, 1500));
        t.draw(7, 10, at(start, 1525));
        let s = t.last().unwrap();
        assert_eq!(
            [
                s.event_to_capture_ms,
                s.capture_to_request_ms,
                s.request_to_worker_ms,
                s.worker_ms,
                s.worker_to_reply_ms,
                s.reply_to_draw_ms,
                s.event_to_draw_ms
            ],
            [100., 300., 50., 1000., 50., 25., 1525.]
        );
        t.draw(7, 10, at(start, 1600));
        t.cancel();
        assert_eq!(t.last().unwrap().sequence, 1);
        let data = serde_json::to_value(t.last()).unwrap();
        assert!(data.as_object().unwrap().values().all(|v| v.is_number()));
        assert!(!data.as_object().unwrap().contains_key("pid"));
    }
    #[test]
    fn stale_replies_draws_and_focus_changes_cannot_complete_a_new_cycle() {
        let start = Instant::now();
        let mut t = Tracker::default();
        requested(&mut t, start, 1);
        requested(&mut t, at(start, 500), 2);
        t.reply(1, at(start, 450), at(start, 1450), at(start, 1500));
        t.draw(1, 10, at(start, 1600));
        assert!(t.last().is_none());
        t.reply(2, at(start, 950), at(start, 1450), at(start, 1500));
        t.draw(1, 10, at(start, 1600));
        assert!(t.last().is_none());
        t.cancel();
        t.draw(2, 10, at(start, 1600));
        assert!(t.last().is_none());
        t.input(10, start);
        let input = t.take_unbound();
        t.bind(input, 11, at(start, 100));
        t.request(3, at(start, 400));
        t.reply(3, at(start, 450), at(start, 1450), at(start, 1500));
        t.draw(3, 10, at(start, 1600));
        assert!(t.last().is_none());
        requested(&mut t, start, 4);
        t.reply(4, at(start, 450), at(start, 1450), at(start, 1500));
        t.draw(4, 11, at(start, 1600));
        assert!(t.last().is_none());
    }
    #[test]
    fn missing_capture_reply_and_nonmonotonic_events_do_not_make_samples() {
        let start = Instant::now();
        let mut t = Tracker::default();
        t.input(10, start);
        t.request(1, at(start, 400));
        t.reply(1, at(start, 450), at(start, 1450), at(start, 1500));
        t.draw(1, 10, at(start, 1600));
        assert!(t.last().is_none());
        requested(&mut t, start, 2);
        t.draw(2, 10, at(start, 1600));
        assert!(t.last().is_none());
        t.reply(2, at(start, 399), at(start, 1450), at(start, 1500));
        assert!(t.drawable_id().is_none());
    }
}
