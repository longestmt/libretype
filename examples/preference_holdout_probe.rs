// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Pre-registered held-out synthetic preference checks.
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
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "fresh")) {
        return Err("Usage: preference_holdout_probe MODEL [fresh] (synthetic only)".into());
    }
    let fresh = args.len() == 3;
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
        vocabulary: if fresh {
            vec![
                "Inez".into(),
                "tidal sensor".into(),
                "Cobalt Gateway".into(),
                "With appreciation.".into(),
                "Monday".into(),
            ]
        } else {
            vec![
                "Maren".into(),
                "wetland survey".into(),
                "Lumen Bridge".into(),
                "Warmest wishes.".into(),
                "Thursday".into(),
            ]
        },
        ..spelling.clone()
    };
    let cases = if fresh {
        [
            (
                "flavour_us",
                "The soup has a rich flav",
                ".",
                "Use American English spelling.",
                &spelling,
                Strength::Strong,
            ),
            (
                "neighbour_us",
                "My next-door neigh",
                " feeds the cat.",
                "Use American English spelling.",
                &spelling,
                Strength::Strong,
            ),
            (
                "recipient_samira",
                "Please send the tidal sensor report to ",
                ".",
                "Address the report to Samira.",
                &terms,
                Strength::Strong,
            ),
            (
                "compatible_inez",
                "Please send the tidal sensor report to ",
                ".",
                "Keep the sentence concise.",
                &terms,
                Strength::Strong,
            ),
            (
                "signoff_best",
                "The email closes with: ",
                "",
                "Use Best wishes as the sign-off.",
                &terms,
                Strength::Strong,
            ),
            (
                "compatible_appreciation",
                "Our usual sign-off is: ",
                "",
                "Use concise wording.",
                &terms,
                Strength::Strong,
            ),
            (
                "gateway_override",
                "The gateway is named ",
                ".",
                "Name the gateway Amber Link.",
                &terms,
                Strength::Strong,
            ),
            (
                "day_override",
                "The appointment is on ",
                ".",
                "The appointment day is Wednesday.",
                &terms,
                Strength::Strong,
            ),
        ]
    } else {
        [
            (
                "centre_us",
                "The new community cen",
                " opens on Friday.",
                "Use American English spelling.",
                &spelling,
                Strength::Strong,
            ),
            (
                "behaviour_us",
                "Her usual behav",
                " is calm and patient.",
                "Use American English spelling.",
                &spelling,
                Strength::Strong,
            ),
            (
                "recipient_leila",
                "Please send the wetland survey to ",
                ".",
                "The recipient of this survey is Leila.",
                &terms,
                Strength::Strong,
            ),
            (
                "compatible_maren",
                "Please send the wetland survey to ",
                ".",
                "Use concise wording.",
                &terms,
                Strength::Strong,
            ),
            (
                "signoff_sincerely",
                "The note ends with: ",
                "",
                "Use Sincerely as the sign-off.",
                &terms,
                Strength::Strong,
            ),
            (
                "device_override",
                "The device is called the ",
                ".",
                "Call the device Aurora Relay.",
                &terms,
                Strength::Strong,
            ),
            (
                "compatible_signoff",
                "Our usual sign-off is: ",
                "",
                "Keep it short.",
                &terms,
                Strength::Strong,
            ),
            (
                "delivery_override",
                "The package will arrive on ",
                ".",
                "State the delivery day as Tuesday.",
                &terms,
                Strength::Strong,
            ),
        ]
    };
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
