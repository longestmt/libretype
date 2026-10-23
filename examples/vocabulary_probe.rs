// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! End-to-end synthetic personalization probe. No config, vault or Keychain reads.
use libretypist::{
    config::Config,
    core::CompletionRequest,
    history::{self, Data, Record, Scope, Strength},
    model,
};
use std::{io::Write, sync::atomic::AtomicU64, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 || args.get(2).is_some_and(|s| s != "--steady") {
        return Err("Usage: vocabulary_probe MODEL [--steady] (synthetic writing only)".into());
    }
    let steady = args.len() == 3;
    let mut config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    let mut data = Data::default();
    for (index, text) in history::profile::SYNTHETIC_VOCABULARY_SAMPLES
        .iter()
        .enumerate()
    {
        data.upsert(Record {
            id: [index as u8 + 1; 16],
            scope: Scope {
                app: "org.libretypist.synthetic-vocabulary".into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text: (*text).into(),
        })?;
    }
    let build = history::profile::BuildRequest::new(&data, &config, 1, 30)?;
    let epoch = AtomicU64::new(1);
    let start = Instant::now();
    let profile = build.generate(&config, &epoch)?;
    println!(
        "{}",
        serde_json::json!({"event":"profile","model":config.model,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"profile":profile})
    );
    std::io::stdout().flush()?;
    let cases = [
        (
            "name",
            "Please send the photonic lattice report to ",
            "",
            "",
        ),
        ("specialist_term", "The prototype uses a photonic ", "", ""),
        ("recurring_phrase", "Our team's usual sign-off is: ", "", ""),
        (
            "midline_name",
            "Please send the photonic lattice report to ",
            " before Friday.",
            "",
        ),
        (
            "explicit_override",
            "Please send the photonic lattice report to ",
            "",
            "The recipient for this report is Morgan. Use Morgan as the recipient.",
        ),
        ("unrelated_context", "For dinner, I will make ", "", ""),
    ];
    let runs: Vec<_> = if steady {
        Strength::ALL
            .into_iter()
            .flat_map(|strength| cases.into_iter().map(move |case| (case, strength)))
            .collect()
    } else {
        cases
            .into_iter()
            .flat_map(|case| {
                [Strength::Off, Strength::Medium, Strength::Strong]
                    .into_iter()
                    .map(move |strength| (case, strength))
            })
            .collect()
    };
    for ((name, prefix, suffix, explicit), strength) in runs {
        config.history.strength = strength;
        config.writing_instructions = explicit.into();
        let request = CompletionRequest {
            id: 1,
            prefix: prefix.into(),
            suffix: suffix.into(),
            app_id: "org.libretypist.synthetic-vocabulary".into(),
            website: None,
            profile: Some(profile.clone()),
            background: Default::default(),
        };
        let start = Instant::now();
        let result = model::generate(&config, &request, &epoch);
        let (output, error) = match result {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        };
        println!(
            "{}",
            serde_json::json!({"event":"completion","steady_strength":steady,"case":name,"strength":strength,"prefix":prefix,"suffix":suffix,"explicit":explicit,"output":output,"error":error,"elapsed_ms":start.elapsed().as_secs_f64()*1000.})
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}
