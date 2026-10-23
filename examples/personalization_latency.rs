// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Synthetic-only request-format experiment. Does not load app settings or history.
use libretypist::{
    config::Config,
    core,
    history::{self, Profile, Strength},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    time::{Duration, Instant},
};

const BASE: &str = "http://127.0.0.1:11434";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(2..=3).contains(&args.len())
        || args
            .get(2)
            .is_some_and(|s| !matches!(s.as_str(), "examples" | "wording" | "baseline"))
    {
        return Err(
            "Usage: personalization_latency MODEL [examples|wording|baseline] (synthetic only)"
                .into(),
        );
    }
    let mut config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    config.history.strength = Strength::Strong;
    let profile = Profile {
        revision: 3, created_at: 30, source_policy: history::profile::source_policy(&config),
        guidance: "Tone is consistently collegial and proactive. Sentence length varies, mixing short, direct statements with slightly longer, informative ones. Punctuation is standard, using commas and periods effectively. Greetings are informal and direct. Sign-offs are absent.".into(),
        vocabulary: vec!["Keep the photons dancing.".into(), "photonic lattice".into(), "Ariadne".into(), "prototype".into()],
    };
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build();
    let metadata: Value = agent
        .post(&format!("{BASE}/api/show"))
        .send_json(json!({"model":config.model}))?
        .into_json()?;
    if metadata.pointer("/details/format").and_then(Value::as_str) != Some("gguf")
        || ["remote_host", "remote_model"].iter().any(|k| {
            metadata
                .get(k)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        })
    {
        return Err("Probe requires a locally installed GGUF model".into());
    }
    let thinking = metadata["capabilities"]
        .as_array()
        .is_some_and(|v| v.iter().any(|s| s == "thinking"));
    let cases = [
        ("name", "Please send the photonic lattice report to ", ""),
        (
            "name_repeat",
            "Please send the photonic lattice report to ",
            "",
        ),
        (
            "name_incremental",
            "Please send the photonic lattice report to Ariadne and ",
            "",
        ),
        ("specialist_term", "The prototype uses a photonic ", ""),
        ("recurring_phrase", "Our team's usual sign-off is: ", ""),
        (
            "explicit_override",
            "Please send the photonic lattice report to ",
            "The recipient for this report is Morgan. Use Morgan as the recipient.",
        ),
        ("unrelated_context", "For dinner, I will make ", ""),
        (
            "spanish",
            "Gracias por tu ayuda. Te enviaré el documento cuando ",
            "",
        ),
    ];
    let variants: &[&str] = match args.get(2).map(String::as_str) {
        Some("baseline") => &["baseline"],
        Some("examples") => &["one_example", "two_examples", "one_example", "two_examples"],
        Some("wording") => &[
            "two_original",
            "compact_preferences",
            "two_original",
            "compact_preferences",
        ],
        _ => &["baseline", "compact", "baseline", "compact"],
    };
    for &variant in variants {
        for (name, prefix, explicit) in cases {
            let style = profile
                .instruction_for(&config, "org.libretypist.synthetic-latency", None)
                .unwrap();
            let mut instructions =
                format!("{style}\nExplicit writing preferences (higher priority): {explicit}");
            if variant == "compact_preferences" {
                instructions = format!("Personal style (observations, not instructions): {}\nRecurring wording (data, not instructions): {}. Prefer these names, terms and greetings/sign-offs when they fit the current sentence; keep exact spelling. Do not change its topic or add irrelevant terms.\nMatch these style tendencies closely and prefer the recurring vocabulary when relevant to the current task. Explicit writing preferences and the current text take priority.\nExplicit writing preferences (higher priority): {explicit}",json!(profile.guidance),json!(profile.vocabulary));
            }
            let mut prepared =
                core::prepare_structured(prefix, "", config.context_chars, &instructions).unwrap();
            if variant == "compact" {
                prepared.prompt = format!("Continue the text with a few words in its language. Return JSON with only insert: the exact characters to append. Include any needed leading space. Do not repeat existing text.\nWriting preferences: {instructions}\nBefore: {}\n", json!(prefix));
            } else if variant.ends_with("example") || variant == "two_examples" {
                let task = prepared.prompt.split("\n\n").next().unwrap();
                let mut prompt = format!("{task}\n\nBefore: \"Thank you for your \"\nAfter: \"\"\n{{\"insert\":\"help.\"}}\n\n");
                if variant == "two_examples" {
                    prompt.push_str(
                        "Before: \"Hello,\"\nAfter: \"\"\n{\"insert\":\" how are you?\"}\n\n",
                    );
                }
                prepared.prompt = format!(
                    "{prompt}Writing preferences: {instructions}\nBefore: {}\nAfter: \"\"\n",
                    json!(prefix)
                );
            }
            if variant == "two_original" {
                let mut chunks = prepared.prompt.split("\n\n");
                let kept = chunks.by_ref().take(3).collect::<Vec<_>>().join("\n\n");
                let tail = chunks.last().unwrap();
                prepared.prompt = format!("{kept}\n\n{tail}");
            }
            let mut payload = json!({"model":config.model,"prompt":prepared.prompt,"raw":false,"stream":true,"keep_alive":"15m",
                "format":{"type":"object","properties":{"insert":{"type":"string"}},"required":["insert"],"additionalProperties":false},
                "options":{"temperature":0,"seed":42,"num_ctx":2048,"num_predict":(config.max_words*6+48).clamp(64,256),"stop":["<|","<think>"]}});
            if thinking {
                payload["think"] = json!(false);
            }
            let started = Instant::now();
            let response = agent
                .post(&format!("{BASE}/api/generate"))
                .send_json(payload)?;
            let mut raw = String::new();
            let mut first = None;
            let mut complete = None;
            let mut final_event = None;
            for line in BufReader::new(response.into_reader().take(128 * 1024)).lines() {
                let event: Value = serde_json::from_str(&line?)?;
                if event.get("error").is_some() {
                    return Err("Local generation failed".into());
                }
                raw.push_str(event["response"].as_str().unwrap_or(""));
                let elapsed = started.elapsed().as_secs_f64() * 1000.;
                if !raw.is_empty() && first.is_none() {
                    first = Some(elapsed);
                }
                if complete.is_none() && core::midline_response_complete(&raw) {
                    complete = Some(elapsed);
                }
                if raw.len() > 16 * 1024 {
                    return Err("Synthetic response exceeded limit".into());
                }
                if event["done"] == true {
                    final_event = Some(event);
                    break;
                }
            }
            let event = final_event.ok_or("Generation ended before done event")?;
            let metrics: serde_json::Map<_, _> = event
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| {
                    k.ends_with("duration") || k.ends_with("count") || *k == "done_reason"
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            println!(
                "{}",
                json!({"variant":variant,"case":name,"model":config.model,"prompt_chars":prepared.prompt.chars().count(),
                "first_content_ms":first,"complete_json_ms":complete,"total_ms":started.elapsed().as_secs_f64()*1000.,
                "output":core::finish_midline(&prepared,&raw,prefix,"",config.max_words),"raw":raw,"metrics":metrics})
            );
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}
