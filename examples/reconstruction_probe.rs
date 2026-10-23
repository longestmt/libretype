// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Compare raw versus templated whole-word prompts on explicit synthetic cases.
use libretypist::core;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Write,
    time::{Duration, Instant},
};
use unicode_normalization::UnicodeNormalization;

#[derive(Deserialize)]
struct Case {
    id: String,
    prefix: String,
    #[serde(default)]
    suffix: String,
    #[serde(default)]
    instructions: String,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(3..=4).contains(&args.len())
        || args.get(3).is_some_and(|s| {
            !["--grammar", "--sentence-context", "--sentence-echo"].contains(&s.as_str())
        })
    {
        return Err("Usage: reconstruction_probe MODEL SYNTHETIC_CASES.json [--grammar|--sentence-context|--sentence-echo]".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let client = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(Duration::from_secs(45))
        .build();
    let metadata: Value = client
        .post("http://127.0.0.1:11434/api/show")
        .send_json(json!({"model":args[1]}))?
        .into_json()?;
    if metadata.pointer("/details/format").and_then(Value::as_str) != Some("gguf")
        || ["remote_host", "remote_model"].iter().any(|k| {
            metadata
                .get(k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        })
    {
        return Err("Expected a downloaded local GGUF model".into());
    }
    let thinking = metadata["capabilities"]
        .as_array()
        .is_some_and(|v| v.iter().any(|v| v == "thinking"));
    for case in cases {
        let prepared = if case.suffix.trim().is_empty() {
            core::prepare_reconstructed(&case.prefix, &case.suffix, 1200, &case.instructions)
        } else {
            core::prepare_midline(&case.prefix, &case.suffix, 1200, &case.instructions)
        };
        let Some(mut prepared) = prepared else {
            continue;
        };
        if args.get(3).is_some_and(|s| s == "--sentence-echo") {
            if !case.suffix.is_empty() || case.prefix.chars().count() > 128 {
                continue;
            }
            let before: String = case.prefix.nfc().collect();
            prepared = core::PreparedCompletion {
                prompt: format!("Complete this unfinished sentence naturally in the same language. Keep every existing character unchanged, finish the last word if needed, and add at most eight words. Return JSON with completed containing the full sentence, including the existing text. Match the grammatical form required by the sentence.\nExample: {{\"text\":\"She has already complet\",\"completed\":\"She has already completed the assignment.\"}}\nWriting preferences: {}\nText: {}\n", json!(case.instructions), json!(before)),
                fragment: String::new(),
                format: core::CompletionFormat::CompletedPassage,
                passage: Some(core::PassageContext { before, after: String::new() }),
            };
        }
        let field = if prepared.format == core::CompletionFormat::CompletedPassage {
            "completed"
        } else if prepared.format == core::CompletionFormat::Infill {
            "insert"
        } else {
            "continuation"
        };
        let variants: &[(bool, &str)] = if args.get(3).is_some_and(|s| s == "--sentence-echo") {
            &[(false, "sentence_echo")]
        } else if args.get(3).is_some_and(|s| s == "--sentence-context") {
            &[(false, "production"), (false, "sentence_context")]
        } else if args.len() == 4 {
            &[
                (false, "baseline"),
                (false, "grammar"),
                (false, "full_context"),
            ]
        } else {
            &[(true, "raw"), (false, "templated")]
        };
        for &(raw, variant) in variants {
            // Keep the historical comparison reproducible after the grammar prompt
            // becomes the production default.
            let mut prompt = if args.get(3).is_some_and(|s| s == "--grammar") {
                prepared.prompt.replacen("Finish that word using the grammatical form required by Context (including part of speech, tense, number and gender), then add a short continuation.",
                    "First finish spelling that word correctly, then add a short continuation.", 1)
            } else {
                prepared.prompt.clone()
            };
            if variant == "grammar" {
                prompt = prompt.replacen("First finish spelling that word correctly, then add a short continuation.",
                    "Finish that word using the grammatical form required by Context (including part of speech, tense, number and gender), then add a short continuation.",1);
            } else if variant == "full_context" || variant == "sentence_context" {
                prompt.push_str(&format!("Full text so far: {}\nChoose a completed word that fits this sentence grammatically. Return only the full last word and following words in continuation.\n",json!(case.prefix)));
            }
            let mut payload = json!({"model":args[1],"prompt":prompt,"raw":raw,"stream":false,"keep_alive":"15m",
                "format":{"type":"object","properties":{(field):{"type":"string"}},"required":[field],"additionalProperties":false},
                "options":{"temperature":0,"seed":42,"num_ctx":2048,"num_predict":96,"stop":["<|","<think>"]}});
            if let Some(context) = &prepared.passage {
                payload["options"]["num_predict"] = json!((context.before.chars().count()
                    + context.after.chars().count()
                    + 8 * 6
                    + 64)
                    .clamp(256, 512));
            }
            if thinking {
                payload["think"] = json!(false);
            }
            let start = Instant::now();
            let response: Value = client
                .post("http://127.0.0.1:11434/api/generate")
                .send_json(payload)?
                .into_json()?;
            let text = response["response"].as_str().unwrap_or("");
            println!(
                "{}",
                json!({"id":case.id,"model":args[1],"raw":raw,"variant":variant,"prefix":case.prefix,"response":text,"output":if prepared.format == core::CompletionFormat::CompletedPassage { core::finish_passage(&prepared,text,8) } else if prepared.format == core::CompletionFormat::Infill { core::finish_midline(&prepared,text,&case.prefix,&case.suffix,8) } else { core::finish_reconstructed(&prepared,text,8) },"elapsed_ms":start.elapsed().as_secs_f64()*1000.})
            );
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}
