// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Synthetic regional-spelling holdout. Never reads settings, history, or Keychain.
use libretypist::{
    config::Config,
    core::CompletionRequest,
    history::{self, Data, Profile, Record, Scope, Strength},
    model,
};
use std::{io::Write, sync::atomic::AtomicU64, time::Instant};

const APP: &str = "org.libretypist.synthetic-style";
const BRITISH: [&str; 3] = [
    "Our neighbours enjoyed the theatre programme yesterday. The grey curtains looked lovely, and travelling home afterwards was easy. I would gladly attend another performance next month.",
    "I am travelling to the theatre with our neighbours on Saturday. The programme includes a short discussion about the grey stage design. We will have time for a quiet meal afterwards.",
    "The neighbours shared a theatre programme with me while we were travelling. Its cover showed a grey landscape, and the notes offered a thoughtful defence of the new staging.",
];

fn build_profile(
    config: &Config,
    american: bool,
    epoch: &AtomicU64,
) -> Result<Profile, Box<dyn std::error::Error>> {
    let mut data = Data::default();
    for (i, sample) in BRITISH.iter().enumerate() {
        let text = if american {
            sample
                .replace("neighbours", "neighbors")
                .replace("theatre", "theater")
                .replace("programme", "program")
                .replace("grey", "gray")
                .replace("travelling", "traveling")
                .replace("afterwards", "afterward")
                .replace("defence", "defense")
        } else {
            (*sample).into()
        };
        data.upsert(Record {
            id: [i as u8 + 1; 16],
            scope: Scope {
                app: APP.into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text,
        })?;
    }
    let request = history::profile::BuildRequest::new(&data, config, 1, 30)?;
    let started = Instant::now();
    let profile = request.generate(config, epoch)?;
    println!(
        "{}",
        serde_json::json!({
            "event":"profile", "american":american, "profile":profile,
            "elapsed_ms":started.elapsed().as_secs_f64()*1000.
        })
    );
    std::io::stdout().flush()?;
    Ok(profile)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("Usage: style_probe MODEL (synthetic only)".into());
    }
    let mut config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    let epoch = AtomicU64::new(1);
    let british = build_profile(&config, false, &epoch)?;
    let american = build_profile(&config, true, &epoch)?;
    // Diagnostic control separates profile extraction from completion behavior.
    let mut direct_british = british.clone();
    direct_british.guidance = "Uses British English spelling consistently.".into();
    direct_british.vocabulary.clear();
    let cases = [
        ("colour", "The paint col", " is blue."),
        ("favourite", "This is my fav", " route through the park."),
        (
            "labour",
            "The report lists the cost of labo",
            " for the renovation.",
        ),
        ("cancelled", "The flight was cancel", " because of snow."),
        ("fresh_honour", "It was an hon", " to participate."),
        ("fresh_cheque", "The che", " was paid into the account."),
        ("unrelated", "For dinner, I will make ", ""),
    ];
    for (case, prefix, suffix) in cases {
        for (variant, profile, strength, explicit) in [
            ("off", &british, Strength::Off, ""),
            ("learned_british", &british, Strength::Strong, ""),
            ("learned_american", &american, Strength::Strong, ""),
            ("direct_british", &direct_british, Strength::Strong, ""),
            (
                "explicit_american_override",
                &british,
                Strength::Strong,
                "Use American English spelling.",
            ),
        ] {
            config.history.strength = strength;
            config.writing_instructions = explicit.into();
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: suffix.into(),
                app_id: APP.into(),
                website: None,
                profile: Some(profile.clone()),
                background: Default::default(),
            };
            let started = Instant::now();
            let result = model::generate(&config, &request, &epoch);
            let (output, error) = match result {
                Ok(text) => (Some(text), None),
                Err(error) => (None, Some(error.to_string())),
            };
            println!(
                "{}",
                serde_json::json!({
                    "event":"completion", "model":config.model, "case":case,
                    "variant":variant,"prefix":prefix,"suffix":suffix,
                    "output":output,"error":error,
                    "elapsed_ms":started.elapsed().as_secs_f64()*1000.
                })
            );
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}
