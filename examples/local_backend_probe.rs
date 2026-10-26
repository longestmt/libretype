// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! Synthetic backend checks for alternatives and profile analysis. No settings,
//! history files, Keychain, clipboard, screen or editor access.
use libretypist::{alternatives, config::Config, history, model};
use std::{io::Write, sync::atomic::AtomicU64, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("Usage: local_backend_probe MODEL (synthetic data only)".into());
    }
    let config = Config {
        model: args[1].clone(),
        ..Default::default()
    };
    let epoch = AtomicU64::new(1);
    let request = alternatives::Request {
        id: 1,
        prefix: "The garden is ".into(),
        selected: "beautiful".into(),
        suffix: ".".into(),
        app_id: "org.libretype.synthetic-backend".into(),
        website: None,
        profile: None,
        background: Default::default(),
        candidates: vec![],
    };
    let start = Instant::now();
    let result = model::generate_choices(&config, &request, &epoch);
    let (choices, error) = match result {
        Ok(choices) => (
            Some(choices.into_iter().map(|v| v.insertion).collect::<Vec<_>>()),
            None,
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    println!(
        "{}",
        serde_json::json!({"event":"synonyms","choices":choices,"error":error,"elapsed_ms":start.elapsed().as_secs_f64()*1000.})
    );
    std::io::stdout().flush()?;
    let mut data = history::Data::default();
    for (i, text) in history::profile::SYNTHETIC_VOCABULARY_SAMPLES
        .iter()
        .enumerate()
    {
        data.upsert(history::Record {
            id: [i as u8 + 1; 16],
            scope: history::Scope {
                app: "org.libretype.synthetic-backend".into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text: (*text).into(),
        })?;
    }
    let request = history::profile::BuildRequest::new(&data, &config, 1, 30)?;
    let start = Instant::now();
    let (profile, error) = match request.generate(&config, &epoch) {
        Ok(profile) => (Some(profile), None),
        Err(error) => (None, Some(error.to_string())),
    };
    println!(
        "{}",
        serde_json::json!({"event":"profile","profile":profile,"error":error,"elapsed_ms":start.elapsed().as_secs_f64()*1000.})
    );
    Ok(())
}
