// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! Compare foreground inference with idle, canceled, and continuing style analysis.
//! Uses invented text only; no app settings, history files, or Keychain access.
use libretypist::{
    config::Config,
    core::CompletionRequest,
    history::{self, profile::BuildRequest},
    model,
};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn analysis(config: &Config) -> Result<BuildRequest, Box<dyn std::error::Error>> {
    let mut data = history::Data::default();
    for (index, sample) in history::profile::SYNTHETIC_VOCABULARY_SAMPLES
        .iter()
        .enumerate()
    {
        data.upsert(history::Record {
            id: [index as u8 + 1; 16],
            scope: history::Scope {
                app: "local.libretype.synthetic-contention".into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text: (*sample).into(),
        })?;
    }
    Ok(BuildRequest::new(&data, config, 1, 30)?)
}
fn completion(config: &Config, prefix: &str) -> Result<String, model::ModelError> {
    model::generate(
        config,
        &CompletionRequest {
            id: 1,
            prefix: prefix.into(),
            suffix: String::new(),
            app_id: "local.libretype.synthetic-contention".into(),
            website: None,
            profile: None,
            background: Default::default(),
        },
        &AtomicU64::new(1),
    )
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("Usage: analysis_contention_probe MODEL (synthetic only)".into());
    }
    let config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    let prefixes = [
        "Thank you for your ",
        "The meeting agenda includes ",
        "Please let me know if you ",
    ];
    analysis(&config)?.generate(&config, &AtomicU64::new(1))?;
    // Warm each foreground context before the balanced trial order. No unloads.
    for prefix in prefixes {
        if completion(&config, prefix)?.trim().is_empty() {
            return Err("Warmup returned no insertion".into());
        }
    }
    let orders = [
        ["idle", "cancel", "continue"],
        ["continue", "idle", "cancel"],
        ["cancel", "continue", "idle"],
        ["idle", "continue", "cancel"],
        ["cancel", "idle", "continue"],
        ["continue", "cancel", "idle"],
    ];
    let study = Instant::now();
    for (round, order) in orders.iter().enumerate() {
        for mode in order {
            if study.elapsed() > Duration::from_secs(300) {
                return Err("Study time budget exceeded".into());
            }
            let epoch = AtomicU64::new(1);
            let request = analysis(&config)?;
            let (entered, started) = mpsc::channel();
            let (exited, ended) = mpsc::channel();
            let row = std::thread::scope(|scope| -> Result<_, Box<dyn std::error::Error>> {
                let background = if *mode == "idle" {
                    None
                } else {
                    Some(scope.spawn(|| {
                        let begin = Instant::now();
                        let _ = entered.send(());
                        let result = request.generate(&config, &epoch);
                        let finish = Instant::now();
                        let summary = (
                            finish,
                            begin.elapsed().as_secs_f64() * 1000.,
                            matches!(result, Err(model::ModelError::Stale)),
                            result.is_ok(),
                        );
                        let _ = exited.send(summary);
                    }))
                };
                if background.is_some() {
                    started.recv_timeout(Duration::from_secs(3))?;
                    std::thread::sleep(Duration::from_millis(300));
                    if ended.try_recv().is_ok() {
                        return Err("Analysis already finished; no active-contention trial".into());
                    }
                }
                let foreground = Instant::now();
                if *mode == "cancel" {
                    epoch.store(2, Ordering::Release);
                }
                // Start immediately: waiting for the canceled call to exit would
                // omit part of the delay that resumed typing actually experiences.
                let output = completion(&config, prefixes[round % prefixes.len()]);
                let foreground_ms = foreground.elapsed().as_secs_f64() * 1000.;
                let mut analysis_ms = None;
                let mut analysis_finish_relative_ms = None;
                let mut stale = None;
                let mut analysis_success = None;
                if let Some(background) = background {
                    let (finish, elapsed, canceled, success) =
                        ended.recv_timeout(Duration::from_secs(125))?;
                    background.join().map_err(|_| "Analysis thread panicked")?;
                    analysis_ms = Some(elapsed);
                    analysis_finish_relative_ms = Some(if finish >= foreground {
                        finish.duration_since(foreground).as_secs_f64() * 1000.
                    } else {
                        -foreground.duration_since(finish).as_secs_f64() * 1000.
                    });
                    stale = Some(canceled);
                    analysis_success = Some(success);
                    if (*mode == "cancel" && !canceled) || (*mode == "continue" && !success) {
                        return Err("Background analysis did not reach its expected outcome".into());
                    }
                }
                let output = output?;
                if output.trim().is_empty() {
                    return Err("Foreground returned no insertion".into());
                }
                Ok(
                    serde_json::json!({"round":round,"context":round%prefixes.len(),"mode":mode,
                    "foreground_ms":foreground_ms,"output":output,"analysis_ms":analysis_ms,
                    "analysis_finish_relative_ms":analysis_finish_relative_ms,"analysis_canceled":stale,
                    "analysis_success":analysis_success}),
                )
            })?;
            println!("{row}");
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}
