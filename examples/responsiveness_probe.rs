// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Synthetic cancellation/replacement probe; never loads user settings or text.
use libretypist::{config::Config, core::CompletionRequest, model};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn request(id: u64, midline: bool) -> CompletionRequest {
    CompletionRequest {
        id,
        prefix: if midline { "Hi Alex,\nThank you for your thoughtful feedback on the project. I will send the revised report" } else { "Thank you for your " }.into(),
        suffix: if midline { " before Friday.\nBest,\nSam" } else { "" }.into(),
        app_id: "local.libretypist.synthetic-probe".into(), website: None, profile: None,
        background: Default::default(),
    }
}
fn main() {
    let selected = std::env::args()
        .nth(1)
        .expect("Usage: responsiveness_probe MODEL");
    let config = Config {
        model: selected.clone(),
        ..Config::default()
    };
    for trial in 0..3 {
        let epoch = AtomicU64::new(1);
        let (events, replies) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let first = model::generate(&config, &request(1, true), &epoch);
                let ended = Instant::now();
                let stale = matches!(first, Err(model::ModelError::Stale));
                events.send((ended, stale)).unwrap();
                // The native worker also processes its replacement sequentially.
                while epoch.load(Ordering::Acquire) == 1 {
                    std::thread::yield_now();
                }
                let start = Instant::now();
                let next = model::generate(&config, &request(2, false), &epoch);
                println!(
                    "{}",
                    serde_json::json!({"trial":trial,"event":"replacement","model":selected,
                    "elapsed_ms":start.elapsed().as_secs_f64()*1000.,"output":next.as_ref().ok(),
                    "error":next.as_ref().err().map(ToString::to_string)})
                );
                assert!(next.is_ok());
            });
            std::thread::sleep(Duration::from_millis(150));
            let superseded = Instant::now();
            epoch.store(2, Ordering::Release);
            let (ended, stale) = replies
                .recv_timeout(Duration::from_secs(2))
                .expect("obsolete request blocked replacement");
            println!(
                "{}",
                serde_json::json!({"trial":trial,"event":"cancellation","model":selected,
                "cancelled_while_active":stale,"elapsed_ms":ended.saturating_duration_since(superseded).as_secs_f64()*1000.})
            );
            assert!(stale, "the first request completed before cancellation; this trial does not prove cancellation");
        });
    }
}
