// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! Synthetic checks for ordinary-word adaptation versus exact names and phrases.
//! Never loads user settings, history or Keychain.
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
        return Err("Usage: vocabulary_priority_probe MODEL [fresh]".into());
    }
    let mut config = Config {
        model: args[1].clone(),
        ..Default::default()
    };
    config.history.strength = Strength::Strong;
    let profile = Profile {
        revision: 3,
        created_at: 30,
        source_policy: history::profile::source_policy(&config),
        guidance: "Friendly, concise writing. Uses British English spelling.".into(),
        vocabulary: [
            "Harbour Centre",
            "Colour Lab",
            "TheatreKit",
            "colourScale",
            "Keep the colours flying.",
            "neighbours",
            "favourite",
            "travelling",
            "organise",
            "Leonie",
        ]
        .map(str::to_owned)
        .to_vec(),
    };
    let cases = if args.len() == 3 {
        vec![
            (
                "new_color_us",
                "The wall col",
                " matches the curtains.",
                "Use American English spelling.",
                "or",
            ),
            (
                "new_center_us",
                "The shopping cen",
                " opens next month.",
                "Use American English spelling.",
                "ter",
            ),
            (
                "new_neighbor_us",
                "Our neigh",
                " is a retired teacher.",
                "Use American English spelling.",
                "bor",
            ),
            (
                "new_travel_british",
                "I prefer trav",
                " by bus.",
                "Use British English spelling.",
                "elling",
            ),
            (
                "new_name",
                "The conference venue is named ",
                ".",
                "Use American English spelling.",
                "Harbour Centre",
            ),
            (
                "new_person",
                "The report should be sent to ",
                ".",
                "Keep it concise. Use American English spelling.",
                "Leonie",
            ),
            (
                "new_override",
                "The report should be sent to ",
                ".",
                "Send the report to Mateo. Use American English spelling.",
                "Mateo",
            ),
            (
                "new_signoff",
                "Our usual sign-off is: ",
                "",
                "Use British English spelling. Keep our sign-off verbatim.",
                "Keep the colours flying.",
            ),
        ]
    } else {
        vec![
            (
                "ordinary_favourite_us",
                "This is my fav",
                " book.",
                "Use American English spelling.",
                "orite",
            ),
            (
                "ordinary_neighbour_us",
                "She thanked her neigh",
                " for feeding the dog.",
                "Use American English spelling.",
                "bor",
            ),
            (
                "ordinary_travel_us",
                "I enjoy trav",
                " by train.",
                "Use American English spelling.",
                "eling",
            ),
            (
                "ordinary_organize_us",
                "We should org",
                " the meeting.",
                "Use American English spelling.",
                "anize",
            ),
            (
                "proper_harbour",
                "The venue is named ",
                ".",
                "Use American English spelling.",
                "Harbour Centre",
            ),
            (
                "proper_colour",
                "The laboratory is called ",
                ".",
                "Use American English spelling.",
                "Colour Lab",
            ),
            (
                "proper_toolkit",
                "The toolkit is named ",
                ".",
                "Use American English spelling.",
                "TheatreKit",
            ),
            (
                "identifier",
                "The variable is named ",
                ".",
                "Use American English spelling.",
                "colourScale",
            ),
            (
                "compatible_person",
                "Please send the report to ",
                ".",
                "Use American English spelling.",
                "Leonie",
            ),
            (
                "fixed_signoff",
                "Our usual sign-off is: ",
                "",
                "Use American English spelling. Keep our sign-off verbatim.",
                "Keep the colours flying.",
            ),
            (
                "explicit_name",
                "The venue is named ",
                ".",
                "Use American English spelling. Name the venue Orchard House.",
                "Orchard House",
            ),
            (
                "british_preserved",
                "She thanked her neigh",
                " for feeding the dog.",
                "Use British English spelling.",
                "bour",
            ),
        ]
    };
    for (case, prefix, suffix, explicit, expected) in cases {
        config.writing_instructions = explicit.into();
        let request = CompletionRequest {
            id: 1,
            prefix: prefix.into(),
            suffix: suffix.into(),
            app_id: "org.libretype.synthetic-vocabulary".into(),
            website: None,
            profile: Some(profile.clone()),
            background: Default::default(),
        };
        let started = Instant::now();
        let (output, error) = match model::generate(&config, &request, &AtomicU64::new(1)) {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e.to_string())),
        };
        println!(
            "{}",
            serde_json::json!({"case":case,"prefix":prefix,"suffix":suffix,"explicit":explicit,"expected":expected,"output":output,"error":error,"elapsed_ms":started.elapsed().as_secs_f64()*1000.})
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}
