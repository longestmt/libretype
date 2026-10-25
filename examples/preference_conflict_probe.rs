// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Frozen synthetic profiles isolate instruction conflicts from profile extraction.
//! Does not load application settings, history, clipboard, screen or Keychain.
use libretypist::{
    config::Config,
    core::CompletionRequest,
    history::{self, Profile, Strength},
    model,
};
use std::{io::Write, sync::atomic::AtomicU64, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("Usage: preference_conflict_probe MODEL".into());
    }
    let mut config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    let spelling = Profile {
        revision:3, created_at:30, source_policy:history::profile::source_policy(&config),
        guidance:"Tone is consistently polite and descriptive. Sentence length varies, mixing moderate and longer constructions. Punctuation is standard. No recurring greetings or sign-offs observed.".into(),
        vocabulary:vec!["neighbours".into(),"theatre programme".into(),"grey".into(),"travelling".into(),"afterwards".into()],
    };
    let terms = Profile {
        guidance: "Friendly, concise messages with a recurring sign-off.".into(),
        vocabulary: vec![
            "Ariadne".into(),
            "photonic lattice".into(),
            "Keep the photons dancing.".into(),
        ],
        ..spelling.clone()
    };
    let cases = [
        (
            "spelling_conflict",
            "The che",
            " was paid into the account.",
            "Use American English spelling.",
            &spelling,
            Strength::Strong,
        ),
        (
            "spelling_off",
            "The che",
            " was paid into the account.",
            "Use American English spelling.",
            &spelling,
            Strength::Off,
        ),
        (
            "colour_conflict",
            "The paint col",
            " is blue.",
            "Use American English spelling.",
            &spelling,
            Strength::Strong,
        ),
        (
            "recipient_override",
            "Please send the photonic lattice report to ",
            "",
            "The recipient for this report is Morgan. Use Morgan as the recipient.",
            &terms,
            Strength::Strong,
        ),
        (
            "signoff_override",
            "Our team's usual sign-off is: ",
            "",
            "Use Kind regards as the sign-off.",
            &terms,
            Strength::Strong,
        ),
        (
            "compatible_name",
            "Please send the photonic lattice report to ",
            "",
            "Use concise wording.",
            &terms,
            Strength::Strong,
        ),
        (
            "profile_without_instructions",
            "Please send the photonic lattice report to ",
            "",
            "",
            &terms,
            Strength::Strong,
        ),
    ];
    for (case, prefix, suffix, explicit, profile, strength) in cases {
        config.history.strength = strength;
        config.writing_instructions = explicit.into();
        let request = CompletionRequest {
            id: 1,
            prefix: prefix.into(),
            suffix: suffix.into(),
            app_id: "org.libretypist.synthetic-conflict".into(),
            website: None,
            profile: Some(profile.clone()),
            background: Default::default(),
        };
        let started = Instant::now();
        let result = model::generate(&config, &request, &AtomicU64::new(1));
        let (output, error) = match result {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        };
        println!(
            "{}",
            serde_json::json!({"case":case,"model":config.model,"prefix":prefix,"suffix":suffix,"explicit":explicit,"strength":strength,"output":output,"error":error,"elapsed_ms":started.elapsed().as_secs_f64()*1000.})
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}
