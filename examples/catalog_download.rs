// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Explicit catalog download through the production worker. No app settings access.
use libretypist::model_setup::{Engine, Error, Phase, CHOICES};
use serde_json::json;
use std::{
    io::Write,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(2..=3).contains(&args.len()) || args.get(2).is_some_and(|s| s != "--exercise-cancel") {
        return Err("Usage: catalog_download CATALOG_TAG [--exercise-cancel] (downloads weights; does not activate model)".into());
    }
    let choice = CHOICES
        .iter()
        .position(|c| c.tag == args[1])
        .ok_or("Unknown catalog tag")?;
    let mut engine = Engine::new().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut reported = Instant::now() - Duration::from_secs(10);
    let mut previous = Phase::Idle;
    let mut cancelled = false;
    let mut retried = false;
    engine.start(choice).map_err(|e| e.to_string())?;
    loop {
        if started.elapsed() >= Duration::from_secs(1200) {
            engine.cancel();
            return Err("Download probe exceeded 20-minute budget; cancellation requested".into());
        }
        let snapshot = engine.snapshot();
        if reported.elapsed() >= Duration::from_secs(3)
            || snapshot.phase != previous && !matches!(snapshot.phase, Phase::Layer { .. })
        {
            println!(
                "{}",
                json!({"event":"progress","model":args[1],"job":snapshot.job,"phase":format!("{:?}",snapshot.phase),"elapsed_ms":started.elapsed().as_millis()})
            );
            std::io::stdout().flush()?;
            reported = Instant::now();
        }
        previous = snapshot.phase.clone();
        if args.len() == 3
            && !cancelled
            && matches!(snapshot.phase, Phase::Layer { done, total:Some(total) } if done>=16*1024*1024 && done<total)
        {
            cancelled = true;
            engine.cancel();
            println!(
                "{}",
                json!({"event":"cancel_requested","elapsed_ms":started.elapsed().as_millis()})
            );
            std::io::stdout().flush()?;
        } else if snapshot.phase == Phase::Failed(Error::Cancelled) && cancelled && !retried {
            retried = true;
            println!(
                "{}",
                json!({"event":"cancel_confirmed_retrying","elapsed_ms":started.elapsed().as_millis()})
            );
            std::io::stdout().flush()?;
            engine.start(choice).map_err(|e| e.to_string())?;
        } else if !snapshot.phase.busy() {
            if snapshot.phase != Phase::Ready {
                return Err(snapshot.phase.message().into());
            }
            if args.len() == 3 && !retried {
                return Err(
                    "Model completed without exercising cancellation; may have been cached".into(),
                );
            }
            println!(
                "{}",
                json!({"event":"verified_local","model":args[1],"cancel_retry_exercised":retried,"elapsed_ms":started.elapsed().as_millis()})
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
