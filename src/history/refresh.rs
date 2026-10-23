// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Automatic analysis waits for input to stop, independently of recording.
use super::{
    worker::{Snapshot, Status},
    Strength,
};
use crate::config::Config;
use std::time::{Duration, Instant};

pub struct Schedule {
    activity: Instant,
    last_attempt: Option<Instant>,
}
impl Schedule {
    pub fn new(now: Instant) -> Self {
        Self {
            activity: now,
            last_attempt: None,
        }
    }
    /// A timestamp only; this does not retain keys, text or source identities.
    pub fn note_input(&mut self, now: Instant) {
        self.activity = now;
    }
    pub fn note_attempt(&mut self, now: Instant) {
        self.last_attempt = Some(now);
    }
    pub fn take_due(&mut self, config: &Config, snapshot: &Snapshot, now: Instant) -> bool {
        let due = config.enabled
            && config.history.auto_refresh
            && config.history.strength != Strength::Off
            && snapshot.unlocked
            && snapshot.records >= 3
            && snapshot.status == Status::Ready
            && snapshot
                .profile
                .as_ref()
                .is_none_or(|p| p.revision < snapshot.revision)
            && now.saturating_duration_since(self.activity) >= Duration::from_secs(5)
            && self
                .last_attempt
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(60));
        if due {
            self.last_attempt = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> (Config, Snapshot) {
        let mut config = Config::default();
        config.history.strength = Strength::Strong;
        config.history.auto_refresh = true;
        let mut snapshot = Snapshot::default();
        snapshot.unlocked = true;
        snapshot.records = 3;
        snapshot.revision = 3;
        snapshot.status = Status::Ready;
        (config, snapshot)
    }
    #[test]
    fn recording_off_still_waits_for_typing_pause_and_limits_retries() {
        let (config, snapshot) = ready();
        assert!(!config.history.recording);
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        for second in 1..=10 {
            let now = start + Duration::from_secs(second);
            schedule.note_input(now);
            assert!(!schedule.take_due(&config, &snapshot, now));
        }
        assert!(!schedule.take_due(&config, &snapshot, start + Duration::from_secs(14)));
        assert!(schedule.take_due(&config, &snapshot, start + Duration::from_secs(15)));
        assert!(!schedule.take_due(&config, &snapshot, start + Duration::from_secs(74)));
        assert!(schedule.take_due(&config, &snapshot, start + Duration::from_secs(75)));
    }
    #[test]
    fn pause_opt_out_lock_and_current_profile_prevent_automatic_analysis() {
        let (mut config, mut snapshot) = ready();
        let start = Instant::now();
        let now = start + Duration::from_secs(5);
        let mut schedule = Schedule::new(start);
        config.enabled = false;
        assert!(!schedule.take_due(&config, &snapshot, now));
        config.enabled = true;
        config.history.auto_refresh = false;
        assert!(!schedule.take_due(&config, &snapshot, now));
        config.history.auto_refresh = true;
        config.history.strength = Strength::Off;
        assert!(!schedule.take_due(&config, &snapshot, now));
        config.history.strength = Strength::Strong;
        snapshot.unlocked = false;
        assert!(!schedule.take_due(&config, &snapshot, now));
        snapshot.unlocked = true;
        snapshot.records = 2;
        assert!(!schedule.take_due(&config, &snapshot, now));
        snapshot.records = 3;
        for status in [
            Status::Building,
            Status::Updating,
            Status::Unlocking,
            Status::Locked,
            Status::Failed(super::super::Error::Analysis),
        ] {
            snapshot.status = status;
            assert!(!schedule.take_due(&config, &snapshot, now));
        }
        snapshot.status = Status::Ready;
        snapshot.profile = Some(super::super::Profile {
            revision: 3,
            created_at: 1,
            source_policy: super::super::profile::source_policy(&config),
            guidance: "Concise sentences.".into(),
            vocabulary: vec![],
        });
        assert!(!schedule.take_due(&config, &snapshot, now));
        snapshot.revision = 4;
        assert!(schedule.take_due(&config, &snapshot, now));
    }
}
