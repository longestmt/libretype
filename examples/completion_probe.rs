// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Synthetic production-pipeline probe. Never reads the user's configuration or writing.
use libretypist::{config::Config, core::CompletionRequest, model};
use serde::Deserialize;
use std::{io::Write, sync::atomic::AtomicU64, time::Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    prefix: String,
    #[serde(default)]
    suffix: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    app_instructions: String,
    #[serde(default)]
    site_instructions: String,
    #[serde(default)]
    append_app: bool,
    #[serde(default)]
    append_site: bool,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("Usage: completion_probe MODEL SYNTHETIC_CASES.json".into());
    }
    let input = std::fs::read(&args[2])?;
    if input.len() > 128 * 1024 {
        return Err("Case file exceeds 128 KiB".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&input)?;
    if cases.len() > 100 {
        return Err("At most 100 cases per run".into());
    }
    for case in cases {
        let mut config = Config {
            model: args[1].clone(),
            writing_instructions: case.instructions.clone(),
            ..Config::default()
        };
        let app = "local.libretypist.synthetic-probe";
        config.set_app_instructions(app, case.app_instructions.clone());
        config.set_app_append_instructions(app, case.append_app);
        let website =
            (!case.site_instructions.is_empty()).then(|| "instructions.example".to_owned());
        if let Some(host) = &website {
            config.website_rules.insert(
                host.clone(),
                libretypist::config::WebsiteRule {
                    writing_instructions: case.site_instructions.clone(),
                    append_instructions: case.append_site,
                    ..Default::default()
                },
            );
        }
        let request = CompletionRequest {
            id: 1,
            prefix: case.prefix.clone(),
            suffix: case.suffix.clone(),
            app_id: "local.libretypist.synthetic-probe".into(),
            website,
            profile: None,
            background: Default::default(),
        };
        let start = Instant::now();
        let result = model::generate(&config, &request, &AtomicU64::new(1));
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
        let (output, error) = match result {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        };
        println!(
            "{}",
            serde_json::json!({"id":case.id,"model":args[1],"prefix":case.prefix,"suffix":case.suffix,
            "instructions":case.instructions,"app_instructions":case.app_instructions,"site_instructions":case.site_instructions,
            "append_app":case.append_app,"append_site":case.append_site,
            "effective_instructions":config.instructions_for_context(app, request.website.as_deref()),"max_words":config.max_words,"elapsed_ms":elapsed_ms,"output":output,"error":error})
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}
