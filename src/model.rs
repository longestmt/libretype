// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
use crate::{
    config::Config,
    core::{self, CompletionFormat, CompletionRequest},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fmt,
    io::{BufRead, BufReader, Read},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const BASE: &str = "http://127.0.0.1:11434";

fn preferences<'a>(
    config: &'a Config,
    app: &str,
    website: Option<&str>,
    profile: Option<&crate::history::Profile>,
) -> (std::borrow::Cow<'a, str>, bool) {
    let explicit = config.instructions_for_context(app, website);
    match profile.and_then(|profile| profile.instruction_for(config, app, website)) {
        Some(style) => (
            std::borrow::Cow::Owned(format!(
                "{style}\nExplicit writing preferences (higher priority): {explicit}"
            )),
            true,
        ),
        None => (explicit, false),
    }
}

#[derive(Debug)]
pub enum ModelError {
    Stale,
    InvalidConfig(String),
    Unavailable(String),
    InvalidResponse(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => write!(f, "Completion was superseded by newer typing"),
            Self::InvalidConfig(s) | Self::Unavailable(s) | Self::InvalidResponse(s) => {
                f.write_str(s)
            }
        }
    }
}
impl std::error::Error for ModelError {}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(5))
        .build()
}

fn http_error(error: ureq::Error) -> ModelError {
    // Never include response bodies or prompts in diagnostics.
    match error {
        ureq::Error::Status(code, _) => ModelError::Unavailable(format!(
            "Local Ollama returned HTTP {code}; check the selected model is installed"
        )),
        ureq::Error::Transport(_) => ModelError::Unavailable(
            "Cannot reach local Ollama at 127.0.0.1:11434; start Ollama and retry".into(),
        ),
    }
}

fn ensure_current(request_id: u64, current: &AtomicU64) -> Result<(), ModelError> {
    if current.load(Ordering::Acquire) == request_id {
        Ok(())
    } else {
        Err(ModelError::Stale)
    }
}

fn local_error(error: std::io::Error) -> ModelError {
    match error.kind() {
        std::io::ErrorKind::ConnectionAborted => ModelError::Stale,
        std::io::ErrorKind::InvalidData => ModelError::InvalidResponse(error.to_string()),
        _ => ModelError::Unavailable(error.to_string()),
    }
}
fn json_body(value: &impl serde::Serialize) -> Result<zeroize::Zeroizing<Vec<u8>>, ModelError> {
    serde_json::to_vec(value)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| {
            ModelError::InvalidResponse("Could not prepare the local model request".into())
        })
}
fn local_metadata(
    base: &str,
    model: &str,
    id: u64,
    current: &AtomicU64,
) -> Result<Value, ModelError> {
    let mut response = crate::local_http::Response::post(
        base,
        "/api/show",
        json_body(&json!({"model": model}))?,
        id,
        current,
        1024 * 1024,
    )
    .map_err(local_error)?;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    response.read_to_end(&mut bytes).map_err(local_error)?;
    ensure_current(id, current)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| ModelError::InvalidResponse("Ollama returned invalid model metadata".into()))
}

fn is_local_model(metadata: &Value) -> bool {
    let remote = ["remote_host", "remote_model"].iter().any(|key| {
        metadata
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    });
    !remote && metadata.pointer("/details/format").and_then(Value::as_str) == Some("gguf")
}

pub fn generate(
    config: &Config,
    request: &CompletionRequest,
    current: &AtomicU64,
) -> Result<String, ModelError> {
    generate_at(BASE, config, request, current)
}
fn generate_at(
    base: &str,
    config: &Config,
    request: &CompletionRequest,
    current: &AtomicU64,
) -> Result<String, ModelError> {
    ensure_current(request.id, current)?;
    config.validate().map_err(ModelError::InvalidConfig)?;
    if !config.allows_context(&request.app_id, request.website.as_deref()) {
        return Ok(String::new());
    }
    // Preferences must apply to every result, so bypass the bare raw-text fast
    // path when they are configured rather than applying them only on failure.
    let (instructions, personalized) = preferences(
        config,
        &request.app_id,
        request.website.as_deref(),
        request.profile.as_ref(),
    );
    let background = request
        .background
        .prompt(config, &request.app_id, request.website.as_deref());
    let midline = !request.suffix.trim().is_empty();
    if midline && !config.midline {
        return Ok(String::new());
    }
    let prepared = if let Some(reference) = &background {
        core::prepare_contextual(
            &request.prefix,
            &request.suffix,
            config.context_chars,
            &instructions,
            reference,
        )
    } else if personalized || !instructions.trim().is_empty() {
        core::prepare_structured(
            &request.prefix,
            &request.suffix,
            config.context_chars,
            &instructions,
        )
    } else if midline {
        core::prepare_midline(
            &request.prefix,
            &request.suffix,
            config.context_chars,
            &instructions,
        )
    } else if instructions.trim().is_empty() && background.is_none() {
        core::prepare(&request.prefix, &request.suffix, config.context_chars)
    } else {
        None
    };
    let fallback =
        if midline || background.is_some() || personalized || !instructions.trim().is_empty() {
            None
        } else {
            core::prepare_instructed(
                &request.prefix,
                &request.suffix,
                config.context_chars,
                &instructions,
            )
        };
    if prepared.is_none() && fallback.is_none() {
        return Ok(String::new());
    }
    // Verify downloaded local metadata before sending any writing.
    let metadata = local_metadata(base, &config.model, request.id, current)?;
    if !is_local_model(&metadata) {
        return Err(ModelError::InvalidConfig(
            "Only downloaded local GGUF models are supported; cloud inference is disabled".into(),
        ));
    }
    ensure_current(request.id, current)?;
    let thinking = metadata
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|caps| caps.iter().any(|cap| cap.as_str() == Some("thinking")));
    if let Some(prepared) = prepared {
        let output = generate_prepared(config, request, current, base, &prepared, thinking)?;
        if !output.is_empty() {
            return Ok(output);
        }
    }
    match fallback {
        Some(prepared) => generate_prepared(config, request, current, base, &prepared, thinking),
        None => Ok(String::new()),
    }
}

fn generate_prepared(
    config: &Config,
    request: &CompletionRequest,
    current: &AtomicU64,
    base: &str,
    prepared: &core::PreparedCompletion,
    thinking: bool,
) -> Result<String, ModelError> {
    ensure_current(request.id, current)?;
    let (instructions, personalized) = preferences(
        config,
        &request.app_id,
        request.website.as_deref(),
        request.profile.as_ref(),
    );
    // Personalization must not suppress the final explicit-preference reminder.
    // Retain the learned entries; their relevance does not override instructions.
    let explicit = config.instructions_for_context(&request.app_id, request.website.as_deref());
    let prompt = if explicit.trim().is_empty() {
        prepared.prompt.clone()
    } else if personalized {
        format!("{}\nComplete the text in the fewest words that satisfy the explicit writing preferences: {}\nUse learned style and vocabulary only where compatible with those preferences and necessary for this sentence. Add no optional elaboration. Preserve the existing text. Return only the requested output.", prepared.prompt, explicit.trim())
    } else {
        format!(
            "{}\nComplete the text in the fewest words that satisfy all applicable writing preferences. Add no optional elaboration. The insertion must fit the existing sentence grammatically and use at most {} words. Preserve the existing text. Return only the requested output.",
            prepared.prompt, config.max_words
        )
    };
    let mut payload = json!({
        "model": config.model,
        "prompt": prompt,
        // Writing preferences need the model's instruction template even at a
        // word boundary. Keep raw continuation only for unconfigured writing.
        "raw": !matches!(prepared.format, CompletionFormat::ReconstructedWord | CompletionFormat::CompletedPassage)
            && request.background.prompt(config, &request.app_id, request.website.as_deref()).is_none()
            && instructions.trim().is_empty()
            && !personalized,
        "stream": true,
        "keep_alive": "15m",
        "options": {"temperature": 0, "seed": 42, "num_ctx": 2048,
                    "num_predict": (config.max_words * 4).clamp(24, 128),
                    "stop": ["\n", "<|", "<think>"]}
    });
    if thinking {
        payload["think"] = Value::Bool(false);
    }
    if prepared.format != CompletionFormat::Text {
        let field = match prepared.format {
            CompletionFormat::Infill => "insert",
            CompletionFormat::ReconstructedWord => "continuation",
            CompletionFormat::CompletedPassage => "completed",
            CompletionFormat::Text => unreachable!(),
        };
        payload["format"] = json!({"type":"object","properties":{(field):{"type":"string"}},"required":[field],"additionalProperties":false});
        payload["options"]["num_predict"] = json!((config.max_words * 6 + 48).clamp(64, 256));
        if let Some(context) = &prepared.passage {
            // Allow the bounded echo plus JSON escaping and the insertion.
            payload["options"]["num_predict"] = json!((context.before.chars().count()
                + context.after.chars().count()
                + config.max_words * 6
                + 64)
                .clamp(256, 512));
        }
        payload["options"]["stop"] = json!(["<|", "<think>"]);
    }
    let response = crate::local_http::Response::post(
        base,
        "/api/generate",
        json_body(&payload)?,
        request.id,
        current,
        128 * 1024,
    )
    .map_err(local_error)?;
    let mut reader = BufReader::new(response);
    let mut raw = String::new();
    let mut line = String::new();
    loop {
        ensure_current(request.id, current)?;
        line.clear();
        let count = reader.read_line(&mut line).map_err(local_error)?;
        ensure_current(request.id, current)?;
        if count == 0 {
            return Err(ModelError::InvalidResponse(
                "Ollama ended the response before completion".into(),
            ));
        }
        let event: GenerationEvent = serde_json::from_str(&line).map_err(|_| {
            ModelError::InvalidResponse("Ollama returned an invalid generation response".into())
        })?;
        if event.error.is_some() {
            return Err(ModelError::Unavailable(
                "Ollama could not generate a completion; check the selected local model".into(),
            ));
        }
        // Thinking is never displayed; this mode only accepts visible continuation tokens.
        raw.push_str(&event.response);
        // Once a generated prefix disagrees, no later token can repair it. Stop wasted inference.
        if prepared.format == CompletionFormat::Text
            && !prepared.fragment.starts_with(&raw)
            && !raw.starts_with(&prepared.fragment)
        {
            return Ok(String::new());
        }
        if raw.len() > 16 * 1024 {
            return Err(ModelError::InvalidResponse(
                "Ollama completion exceeded the size limit".into(),
            ));
        }
        if event.done {
            if prepared.format == CompletionFormat::Text
                && event.done_reason.as_deref() == Some("length")
            {
                raw.truncate(core::without_partial_tail(&raw).len());
            }
            break;
        }
        let complete = match prepared.format {
            CompletionFormat::Text => core::has_complete_limit(prepared, &raw, config.max_words),
            CompletionFormat::Infill => core::midline_response_complete(&raw),
            CompletionFormat::CompletedPassage => core::passage_response_complete(&raw),
            CompletionFormat::ReconstructedWord => core::reconstructed_response_complete(&raw),
        };
        if complete {
            break;
        }
    }
    ensure_current(request.id, current)?;
    Ok(match prepared.format {
        CompletionFormat::CompletedPassage => {
            core::finish_passage(prepared, &raw, config.max_words)
        }
        CompletionFormat::Infill => core::finish_midline(
            prepared,
            &raw,
            &request.prefix,
            &request.suffix,
            config.max_words,
        ),
        CompletionFormat::ReconstructedWord => {
            core::finish_reconstructed(prepared, &raw, config.max_words)
        }
        CompletionFormat::Text => core::finish(prepared, &raw, config.max_words),
    })
}

/// Explicit word-choice generation uses the same local-only model verification
/// and cancellation epoch as ordinary completions. Prompts/results are transient.
pub fn generate_choices(
    config: &Config,
    request: &crate::alternatives::Request,
    current: &AtomicU64,
) -> Result<Vec<crate::alternatives::Choice>, ModelError> {
    ensure_current(request.id, current)?;
    config.validate().map_err(ModelError::InvalidConfig)?;
    if !config.allows_context(&request.app_id, request.website.as_deref())
        || request.mode(&config.choices) == crate::alternatives::RevealMode::Off
    {
        return Ok(vec![]);
    }
    let (instructions, _) = preferences(
        config,
        &request.app_id,
        request.website.as_deref(),
        request.profile.as_ref(),
    );
    let Some(mut prompt) = request.prompt(config.context_chars, &instructions) else {
        return Ok(vec![]);
    };
    if let Some(reference) =
        request
            .background
            .prompt(config, &request.app_id, request.website.as_deref())
    {
        prompt.insert_str(0, &reference);
    }
    if !request.is_synonym() && !request.started_word().is_empty() && request.candidates.is_empty()
    {
        return Ok(vec![]);
    }
    let metadata = local_metadata(BASE, &config.model, request.id, current)?;
    if !is_local_model(&metadata) {
        return Err(ModelError::InvalidConfig(
            "Only downloaded local GGUF models are supported; cloud inference is disabled".into(),
        ));
    }
    ensure_current(request.id, current)?;
    let mut payload = json!({"model":config.model,"prompt":prompt,"raw":false,"stream":true,"keep_alive":"15m",
        "format":{"type":"object","properties":{"choices":{"type":"array","items":{"type":"string","minLength":1,"maxLength":80},"minItems":3,"maxItems":5}},"required":["choices"],"additionalProperties":false},
        "options":{"temperature":0.4,"seed":42,"num_ctx":2048,"num_predict":256,"stop":["<|","<think>"]}});
    if !request.candidates.is_empty() {
        payload["format"]["properties"]["choices"]["items"] =
            json!({"type":"string","enum":request.candidates});
        payload["format"]["properties"]["choices"]["minItems"] = json!(1);
    }
    if metadata
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|caps| caps.iter().any(|c| c.as_str() == Some("thinking")))
    {
        payload["think"] = Value::Bool(false);
    }
    let response = crate::local_http::Response::post(
        BASE,
        "/api/generate",
        json_body(&payload)?,
        request.id,
        current,
        128 * 1024,
    )
    .map_err(local_error)?;
    let mut reader = BufReader::new(response);
    let mut raw = String::new();
    let mut line = String::new();
    loop {
        ensure_current(request.id, current)?;
        line.clear();
        let count = reader.read_line(&mut line).map_err(local_error)?;
        ensure_current(request.id, current)?;
        if count == 0 {
            return Err(ModelError::InvalidResponse(
                "Ollama ended word choices before completion".into(),
            ));
        }
        let event: GenerationEvent = serde_json::from_str(&line).map_err(|_| {
            ModelError::InvalidResponse("Ollama returned invalid word-choice data".into())
        })?;
        if event.error.is_some() {
            return Err(ModelError::Unavailable(
                "Local model could not generate word choices".into(),
            ));
        }
        raw.push_str(&event.response);
        if raw.len() > 8192 {
            return Err(ModelError::InvalidResponse(
                "Word choices exceeded the size limit".into(),
            ));
        }
        if event.done || serde_json::from_str::<Value>(&raw).is_ok() {
            break;
        }
    }
    ensure_current(request.id, current)?;
    Ok(request.finish(&raw))
}

/// Analyze explicit samples locally. This does not load history, enable recording,
/// save a profile, or mutate settings. The vault owner must commit against revision.
pub fn generate_profile(
    config: &Config,
    request: &crate::history::profile::BuildRequest,
    current: &AtomicU64,
) -> Result<crate::history::Profile, ModelError> {
    ensure_current(request.id, current)?;
    config.validate().map_err(ModelError::InvalidConfig)?;
    if !request.matches_policy(config) {
        return Err(ModelError::Stale);
    }
    let metadata = local_metadata(BASE, &config.model, request.id, current)?;
    if !is_local_model(&metadata) {
        return Err(ModelError::InvalidConfig(
            "Only downloaded local GGUF models are supported; cloud inference is disabled".into(),
        ));
    }
    ensure_current(request.id, current)?;
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        model: &'a str,
        prompt: &'a str,
        raw: bool,
        stream: bool,
        keep_alive: &'a str,
        format: Value,
        options: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        think: Option<bool>,
    }
    let prompt = request.prompt();
    let payload = zeroize::Zeroizing::new(serde_json::to_vec(&Payload {
        model: &config.model, prompt: &prompt, raw: false, stream: true, keep_alive: "15m",
        format: json!({"type":"object","properties":{"guidance":{"type":"string","minLength":1,"maxLength":1000},"vocabulary":{"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":64}}},"required":["guidance","vocabulary"],"additionalProperties":false}),
        options: json!({"temperature":0,"seed":42,"num_ctx":8192,"num_predict":640,"stop":["<|","<think>"]}),
        think: metadata.get("capabilities").and_then(Value::as_array)
            .is_some_and(|caps| caps.iter().any(|cap| cap.as_str()==Some("thinking"))).then_some(false),
    }).map_err(|_| ModelError::InvalidResponse("Could not prepare local style analysis".into()))?);
    let response = crate::local_http::Response::post(
        BASE,
        "/api/generate",
        payload,
        request.id,
        current,
        128 * 1024,
    )
    .map_err(local_error)?;
    let mut reader = BufReader::new(response);
    let raw = read_profile_stream(&mut reader, request.id, current)?;
    ensure_current(request.id, current)?;
    request.finish(config, &raw).map_err(|_| {
        ModelError::InvalidResponse(
            "Local model returned an invalid or outdated style profile".into(),
        )
    })
}

fn read_profile_stream(
    reader: &mut impl BufRead,
    id: u64,
    current: &AtomicU64,
) -> Result<zeroize::Zeroizing<String>, ModelError> {
    let mut raw = zeroize::Zeroizing::new(String::new());
    let mut line = zeroize::Zeroizing::new(String::new());
    loop {
        ensure_current(id, current)?;
        zeroize::Zeroize::zeroize(&mut *line);
        // Bound each line independently, including in unit tests with arbitrary readers.
        let count = reader
            .by_ref()
            .take(16 * 1024)
            .read_line(&mut line)
            .map_err(local_error)?;
        ensure_current(id, current)?;
        if count == 0 {
            return Err(ModelError::InvalidResponse(
                "Ollama ended style analysis before completion".into(),
            ));
        }
        if count >= 16 * 1024 {
            return Err(ModelError::InvalidResponse(
                "Style analysis exceeded the size limit".into(),
            ));
        }
        let event: GenerationEvent = serde_json::from_str(&line).map_err(|_| {
            ModelError::InvalidResponse("Ollama returned invalid style-analysis data".into())
        })?;
        if event.error.is_some() {
            return Err(ModelError::Unavailable(
                "Local model could not analyze writing style".into(),
            ));
        }
        raw.push_str(&event.response);
        if raw.len() > 8192 {
            return Err(ModelError::InvalidResponse(
                "Style analysis exceeded the size limit".into(),
            ));
        }
        if event.done {
            if event.done_reason.as_deref() == Some("length") {
                return Err(ModelError::InvalidResponse(
                    "Style analysis exceeded the token limit".into(),
                ));
            }
            return Ok(raw);
        }
    }
}

#[derive(Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct GenerationEvent {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    done_reason: Option<String>,
}

pub fn list_models() -> Result<Vec<String>, ModelError> {
    let value: Value = agent()
        .get(&format!("{BASE}/api/tags"))
        .call()
        .map_err(http_error)?
        .into_json()
        .map_err(|_| ModelError::InvalidResponse("Ollama returned an invalid model list".into()))?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ModelError::InvalidResponse("Ollama response has no model list".into()))?;
    let mut names: Vec<String> = models
        .iter()
        .filter(|m| is_local_model(m))
        .filter_map(|m| m.get("name").and_then(Value::as_str))
        .filter(|name| !name.to_ascii_lowercase().contains("cloud"))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        sync::mpsc,
        time::Instant,
    };

    fn wire_request(socket: &mut TcpStream) -> (String, Value) {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut size = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty());
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    size = value.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0; size];
        reader.read_exact(&mut body).unwrap();
        (first, serde_json::from_slice(&body).unwrap())
    }
    fn wire_reply(socket: &mut TcpStream, body: &str) {
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        socket.flush().unwrap();
    }
    fn synthetic_request(id: u64) -> CompletionRequest {
        CompletionRequest {
            id,
            prefix: "Thank you for your ".into(),
            suffix: String::new(),
            app_id: "local.libretypist.synthetic-probe".into(),
            website: None,
            profile: None,
            background: Default::default(),
        }
    }
    #[test]
    fn explicit_preferences_use_templated_insertion_and_resolved_scope() {
        for (scope, learned) in (0..5).flat_map(|scope| [false, true].map(|learned| (scope, learned))) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let mut config = Config::default();
            let mut request = synthetic_request(1);
            if scope > 0 {
                config.writing_instructions = "Mention review.".into();
            }
            if scope >= 2 {
                config.set_app_instructions(&request.app_id, "Mention feedback.".into());
                config.set_app_append_instructions(&request.app_id, scope >= 3);
            }
            if scope >= 3 {
                request.website = Some("instructions.example".into());
                config.website_rules.insert(
                    "instructions.example".into(),
                    crate::config::WebsiteRule {
                        writing_instructions: "Mention patience.".into(),
                        append_instructions: scope == 3,
                        ..Default::default()
                    },
                );
            }
            if learned {
                config.history.strength = crate::history::Strength::Strong;
                request.profile = Some(crate::history::Profile {
                    revision: 1, created_at: 1,
                    source_policy: crate::history::profile::source_policy(&config),
                    guidance: "Concise writing.".into(),
                    vocabulary: vec!["Ariadne".into()],
                });
            }
            std::thread::scope(|threads| {
                let server = threads.spawn(|| {
                    let (mut socket, _) = listener.accept().unwrap();
                    let (path, _) = wire_request(&mut socket);
                    assert!(path.starts_with("POST /api/show "));
                    wire_reply(&mut socket, r#"{"details":{"format":"gguf"}}"#);
                    let (mut socket, _) = listener.accept().unwrap();
                    let (path, body) = wire_request(&mut socket);
                    assert!(path.starts_with("POST /api/generate "));
                    assert_eq!(body["raw"], scope == 0 && !learned);
                    let prompt = body["prompt"].as_str().unwrap();
                    assert_eq!(prompt.contains("Mention review."), scope == 1 || scope == 3);
                    assert_eq!(
                        prompt.contains("Mention feedback."),
                        scope == 2 || scope == 3
                    );
                    assert_eq!(prompt.contains("Mention patience."), scope >= 3);
                    assert_eq!(prompt.contains("Ariadne"), learned);
                    assert_eq!(prompt.contains("fewest words"), scope > 0);
                    assert_eq!(prompt.contains("satisfy the explicit writing preferences"), learned && scope > 0);
                    let response = if scope == 0 && !learned {
                        assert!(body.get("format").is_none());
                        assert_eq!(prompt, "Thank you for your");
                        " help.".to_owned()
                    } else {
                        assert_eq!(body["format"]["required"], json!(["insert"]));
                        if !learned {
                            assert!(prompt.contains("at most 8 words"));
                        }
                        json!({"insert":"help."}).to_string()
                    };
                    wire_reply(
                        &mut socket,
                        &format!("{}\n", json!({"response":response,"done":true})),
                    );
                });
                let output = generate_at(&base, &config, &request, &AtomicU64::new(1)).unwrap();
                assert_eq!(output, "help.");
                server.join().unwrap();
            });
        }
    }
    #[test]
    fn newer_completion_runs_after_cancelling_silent_metadata_or_generation() {
        for stall_at in [0, 1] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let epoch = AtomicU64::new(1);
            let (ready, waiting) = mpsc::channel();
            std::thread::scope(|scope| {
                let server = scope.spawn(|| {
                    for i in 0..stall_at + 3 {
                        let (mut socket, _) = listener.accept().unwrap();
                        let (path, body) = wire_request(&mut socket);
                        let is_show = path.starts_with("POST /api/show ");
                        if is_show {
                            assert_eq!(body.as_object().unwrap().len(), 1);
                            assert!(body.get("model").is_some());
                        } else {
                            assert!(path.starts_with("POST /api/generate "));
                            assert_eq!(body["prompt"], "Thank you for your");
                        }
                        if i == stall_at {
                            ready.send(()).unwrap();
                            let closed = socket.read(&mut [0]);
                            assert!(
                                matches!(closed, Ok(0))
                                    || closed.is_err_and(
                                        |e| e.kind() == std::io::ErrorKind::ConnectionReset
                                    )
                            );
                        } else if is_show {
                            wire_reply(&mut socket, r#"{"details":{"format":"gguf"}}"#);
                        } else {
                            wire_reply(&mut socket, "{\"response\":\" help.\",\"done\":true}\n");
                        }
                    }
                });
                let worker = scope.spawn(|| {
                    let config = Config::default();
                    assert!(matches!(
                        generate_at(&base, &config, &synthetic_request(1), &epoch),
                        Err(ModelError::Stale)
                    ));
                    assert_eq!(
                        generate_at(&base, &config, &synthetic_request(2), &epoch).unwrap(),
                        "help."
                    );
                });
                waiting.recv_timeout(Duration::from_secs(2)).unwrap();
                let start = Instant::now();
                epoch.store(2, Ordering::Release);
                worker.join().unwrap();
                server.join().unwrap();
                assert!(start.elapsed() < Duration::from_secs(1));
                println!(
                    "new completion after cancelling stage {stall_at}: {:.3} ms",
                    start.elapsed().as_secs_f64() * 1000.
                );
            });
        }
    }
    #[test]
    fn nonlocal_model_metadata_prevents_any_writing_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::scope(|scope| {
            let server = scope.spawn(|| {
                let (mut socket, _) = listener.accept().unwrap();
                let (path, body) = wire_request(&mut socket);
                assert!(path.starts_with("POST /api/show "));
                assert_eq!(body.as_object().unwrap().len(), 1);
                wire_reply(
                    &mut socket,
                    r#"{"details":{"format":"gguf"},"remote_host":"example.invalid"}"#,
                );
            });
            assert!(matches!(
                generate_at(
                    &base,
                    &Config::default(),
                    &synthetic_request(1),
                    &AtomicU64::new(1)
                ),
                Err(ModelError::InvalidConfig(_))
            ));
            server.join().unwrap();
        });
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn profile_stream_requires_a_complete_bounded_current_result() {
        let epoch = AtomicU64::new(1);
        let event =
            |response: &str, done: bool| format!("{}\n", json!({"response":response,"done":done}));
        let raw = format!(
            "{}{}",
            event(r#"{"guidance":"Brief "#, false),
            event(r#"sentences."}"#, true)
        );
        let result = read_profile_stream(&mut raw.as_bytes(), 1, &epoch).unwrap();
        assert_eq!(&*result, r#"{"guidance":"Brief sentences."}"#);
        for invalid in [
            event(r#"{"guidance":"Incomplete stream."}"#, false),
            format!(
                "{}\n",
                json!({"response":"NEVER_PRINT","done":true,"done_reason":"length"})
            ),
            format!("{}\n", json!({"error":"NEVER_PRINT"})),
            event(&"x".repeat(8193), true),
            "x".repeat(16 * 1024),
        ] {
            let err = read_profile_stream(&mut invalid.as_bytes(), 1, &epoch).unwrap_err();
            assert!(!err.to_string().contains("NEVER_PRINT"));
        }
        assert!(matches!(
            read_profile_stream(&mut raw.as_bytes(), 2, &epoch),
            Err(ModelError::Stale)
        ));
    }

    #[test]
    fn learned_preferences_are_transient_and_do_not_replace_explicit_instructions() {
        let mut config = Config {
            writing_instructions: "Formal style.".into(),
            ..Config::default()
        };
        let profile = crate::history::Profile {
            revision: 1,
            created_at: 1,
            source_policy: crate::history::profile::source_policy(&config),
            guidance: "Brief sentences.".into(),
            vocabulary: vec!["photonic lattice".into()],
        };
        let before = serde_json::to_string(&config).unwrap();
        let (off, active) = preferences(&config, "com.apple.TextEdit", None, Some(&profile));
        assert!(!active);
        assert_eq!(off, "Formal style.");
        config.history.strength = crate::history::Strength::Medium;
        let (on, active) = preferences(&config, "com.apple.TextEdit", None, Some(&profile));
        assert!(active);
        assert!(on.contains("Brief sentences."));
        assert!(on.contains("photonic lattice"));
        assert!(on.contains("higher priority): Formal style."));
        assert!(!serde_json::to_string(&config)
            .unwrap()
            .contains("Brief sentences."));
        assert!(!serde_json::to_string(&config)
            .unwrap()
            .contains("photonic lattice"));
        config.history.strength = crate::history::Strength::Off;
        assert_eq!(serde_json::to_string(&config).unwrap(), before);
    }

    #[test]
    #[ignore = "requires a downloaded local model; synthetic writing only, no history/Keychain access"]
    fn style_profile_live() {
        let mut config = Config::default();
        let mut data = crate::history::profile::tests::synthetic_data();
        let request = crate::history::profile::BuildRequest::new(&data, &config, 1, 30).unwrap();
        let epoch = AtomicU64::new(1);
        let profile = generate_profile(&config, &request, &epoch).unwrap();
        assert!(profile.valid());
        println!("Synthetic style analysis: {}", profile.guidance);
        data.set_profile(profile.clone()).unwrap();
        config.history.strength = crate::history::Strength::Medium;
        for (prefix, suffix) in [
            ("Thank you for your ", ""),
            ("Thank you for your hel", ""),
            ("Thank you for your ", "."),
        ] {
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: suffix.into(),
                app_id: "org.libretypist.test".into(),
                website: None,
                background: crate::background::Background::default(),
                profile: Some(profile.clone()),
            };
            let output = generate(&config, &request, &epoch).unwrap();
            println!("Synthetic personalized continuation: {prefix:?} | {suffix:?} => {output:?}");
            assert!(!output.trim().is_empty());
            assert!(!output.contains("guidance"));
            assert!(!output.contains("Personal writing style"));
            if prefix.ends_with("hel") {
                assert!(format!("{prefix}{output}").contains("help"));
            }
        }
        let request = crate::alternatives::Request {
            id: 1,
            prefix: "That is a ".into(),
            selected: "great".into(),
            suffix: " idea.".into(),
            app_id: "org.libretypist.test".into(),
            website: None,
            background: crate::background::Background::default(),
            profile: Some(profile),
            candidates: vec![],
        };
        let choices = generate_choices(&config, &request, &epoch).unwrap();
        assert!(!choices.is_empty());
        println!(
            "Synthetic personalized synonyms: {}",
            choices
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(!config.history.recording);
    }

    #[test]
    fn stale_profile_requests_stop_before_network_access() {
        let mut config = Config {
            model: "missing-model-test".into(),
            ..Config::default()
        };
        let request = crate::history::profile::BuildRequest::new(
            &crate::history::profile::tests::synthetic_data(),
            &config,
            1,
            30,
        )
        .unwrap();
        assert!(matches!(
            request.generate(&config, &AtomicU64::new(2)),
            Err(ModelError::Stale)
        ));
        config
            .history
            .excluded_apps
            .push("org.libretypist.test".into());
        assert!(matches!(
            request.generate(&config, &AtomicU64::new(1)),
            Err(ModelError::Stale)
        ));
    }

    #[test]
    #[ignore = "requires a downloaded local model; synthetic reference only"]
    fn clipboard_context_live() {
        let mut config = Config::default();
        config.background.clipboard = true;
        let epoch = AtomicU64::new(1);
        for (prefix, suffix) in [
            ("The meeting is at ", ""),
            ("The meeting is at ", " on Thursday."),
            ("The meeting is at Red", ""),
        ] {
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: suffix.into(),
                app_id: "org.libretypist.test".into(),
                website: None,
                profile: None,
                background: crate::background::Background::from_sample(
                    "The meeting is on Thursday at the Redwood Cafe.",
                ),
            };
            let output = generate(&config, &request, &epoch).unwrap();
            println!("{prefix:?} | {suffix:?} => {output:?}");
            assert!(
                format!("{prefix}{output}").contains("Redwood"),
                "synthetic venue was not used"
            );
            assert!(!output.contains("Background reference"));
        }
    }

    #[test]
    fn word_choice_exclusions_and_cancellation_precede_network() {
        let mut config = Config {
            model: "missing-model-test".into(),
            ..Config::default()
        };
        let mut request = crate::alternatives::Request {
            id: 1,
            prefix: "Thanks for your ".into(),
            selected: "help".into(),
            suffix: ".".into(),
            app_id: "test".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
            candidates: vec![],
        };
        let epoch = AtomicU64::new(1);
        config.set_app_enabled("test", false);
        assert!(generate_choices(&config, &request, &epoch)
            .unwrap()
            .is_empty());
        config.set_app_enabled("test", true);
        config.choices.synonyms = crate::alternatives::RevealMode::Off;
        assert!(generate_choices(&config, &request, &epoch)
            .unwrap()
            .is_empty());
        config.choices.synonyms = crate::alternatives::RevealMode::Automatic;
        request.selected = "bad\nselection".into();
        assert!(generate_choices(&config, &request, &epoch)
            .unwrap()
            .is_empty());
        epoch.store(2, Ordering::SeqCst);
        assert!(matches!(
            generate_choices(&config, &request, &epoch),
            Err(ModelError::Stale)
        ));
    }
    #[test]
    #[ignore = "requires local Ollama and qwen3:1.7b; synthetic word-choice contexts only"]
    fn word_choices_live() {
        let mut config = Config::default();
        config.choices.alternatives = crate::alternatives::RevealMode::Shortcut;
        let epoch = AtomicU64::new(1);
        for (prefix, selected, suffix) in [
            ("Thank you for your ", "", ""),
            ("The proposal is ", "great", "."),
            ("We should sche", "", ""),
        ] {
            let request = crate::alternatives::Request {
                id: 1,
                prefix: prefix.into(),
                selected: selected.into(),
                suffix: suffix.into(),
                app_id: "test".into(),
                website: None,
                profile: None,
                background: crate::background::Background::default(),
                candidates: if prefix.ends_with("sche") {
                    vec!["schedule".into(), "scheme".into(), "schematic".into()]
                } else {
                    vec![]
                },
            };
            let start = std::time::Instant::now();
            let choices = generate_choices(&config, &request, &epoch).unwrap();
            eprintln!(
                "synthetic {prefix:?}|{selected:?}|{suffix:?}: {:?} ms={}",
                choices.iter().map(|c| &c.label).collect::<Vec<_>>(),
                start.elapsed().as_millis()
            );
            assert!(!choices.is_empty());
            if prefix.ends_with("sche") {
                assert_eq!(choices[0].label, "schedule");
            }
        }
    }
    #[test]
    fn excluded_website_returns_before_contacting_ollama() {
        let mut config = Config {
            model: "missing-test-model".into(),
            ..Config::default()
        };
        config.website_rules.insert(
            "private.example".into(),
            crate::config::WebsiteRule {
                enabled: false,
                ..Default::default()
            },
        );
        let request = CompletionRequest {
            id: 1,
            prefix: "Synthetic test text".into(),
            suffix: String::new(),
            app_id: "browser".into(),
            website: Some("private.example".into()),
            profile: None,
            background: crate::background::Background::default(),
        };
        assert_eq!(generate(&config, &request, &AtomicU64::new(1)).unwrap(), "");
    }

    #[test]
    #[ignore = "requires local Ollama and downloaded qwen3:1.7b; uses synthetic text only"]
    fn website_instructions_live() {
        let mut config = Config::default();
        config.set_app_instructions(
            "browser",
            "When expressing thanks, specifically mention reviewing the proposal.".into(),
        );
        config.website_rules.insert(
            "mail.example".into(),
            crate::config::WebsiteRule {
                writing_instructions:
                    "When expressing thanks, specifically mention helping with the presentation."
                        .into(),
                ..Default::default()
            },
        );
        let epoch = AtomicU64::new(1);
        let mut request = CompletionRequest {
            id: 1,
            prefix: "Thank you for your ".into(),
            suffix: String::new(),
            app_id: "browser".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
        };
        let app = generate(&config, &request, &epoch).unwrap();
        request.website = Some("mail.example".into());
        let site = generate(&config, &request, &epoch).unwrap();
        eprintln!("synthetic app={app:?}; website={site:?}");
        assert!(app.contains("proposal"));
        assert!(site.contains("presentation"));
        assert_ne!(site, app);
        request.website = Some("other.example".into());
        assert_eq!(generate(&config, &request, &epoch).unwrap(), app);
        config.website_rules.remove("mail.example");
        request.website = Some("mail.example".into());
        assert_eq!(generate(&config, &request, &epoch).unwrap(), app);
    }

    #[test]
    fn stale_request_never_contacts_model() {
        let request = CompletionRequest {
            id: 1,
            prefix: "Please send ".into(),
            suffix: "".into(),
            app_id: "com.apple.TextEdit".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
        };
        assert!(matches!(
            generate(&Config::default(), &request, &AtomicU64::new(2)),
            Err(ModelError::Stale)
        ));
    }

    #[test]
    fn rejects_cloud_aliases_and_unknown_formats() {
        assert!(is_local_model(&json!({"details":{"format":"gguf"}})));
        assert!(!is_local_model(
            &json!({"details":{"format":"gguf"}, "remote_model":"qwen3"})
        ));
        assert!(!is_local_model(
            &json!({"details":{"format":"gguf"}, "remote_host":"https://ollama.com"})
        ));
        assert!(!is_local_model(&json!({"details":{}})));
    }

    #[test]
    fn blocked_apps_return_before_network_access() {
        let request = CompletionRequest {
            id: 1,
            prefix: "Please send ".into(),
            suffix: "".into(),
            app_id: "com.apple.Passwords".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
        };
        assert_eq!(
            generate(&Config::default(), &request, &AtomicU64::new(1)).unwrap(),
            ""
        );
    }

    #[test]
    #[ignore = "requires local Ollama and downloaded qwen3:1.7b; uses synthetic text only"]
    fn writing_instructions_live() {
        let request = CompletionRequest {
            id: 1,
            prefix: "Thank you for your ".into(),
            suffix: String::new(),
            app_id: "local.libretypist.test".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
        };
        let epoch = AtomicU64::new(1);
        let baseline = generate(&Config::default(), &request, &epoch).unwrap();
        let config = Config {
            writing_instructions:
                "When expressing thanks, specifically mention reviewing the proposal.".into(),
            ..Config::default()
        };
        let output = generate(&config, &request, &epoch).unwrap();
        eprintln!("synthetic baseline={baseline:?}; with instructions={output:?}");
        assert!(output.contains("proposal"));
        assert_ne!(baseline, output);
        assert!(output.split_whitespace().count() <= config.max_words);
        let mut per_app = Config::default();
        per_app.set_app_instructions(&request.app_id, config.writing_instructions.clone());
        let app_output = generate(&per_app, &request, &epoch).unwrap();
        assert_eq!(
            app_output, output,
            "app override must reach the same generation path"
        );
        let partial = CompletionRequest {
            prefix: "Thank you for your hel".into(),
            ..request.clone()
        };
        let partial_output = generate(&config, &partial, &epoch).unwrap();
        eprintln!("synthetic partial with instructions={partial_output:?}");
        assert!(format!("hel{partial_output}").starts_with("help"));
        assert!(partial_output.contains("proposal"));
        assert_eq!(
            generate(&per_app, &partial, &epoch).unwrap(),
            partial_output
        );
        let other_app = CompletionRequest {
            app_id: "local.libretypist.other".into(),
            website: None,
            ..request
        };
        assert_eq!(
            generate(&per_app, &other_app, &epoch).unwrap(),
            baseline,
            "app preferences must not leak into another app"
        );
    }

    #[test]
    fn disabled_midline_returns_before_network_access() {
        let config = Config {
            midline: false,
            model: "missing-model-for-test".into(),
            ..Config::default()
        };
        let request = CompletionRequest {
            id: 1,
            prefix: "We should ".into(),
            suffix: "tomorrow.".into(),
            app_id: "local.libretypist.test".into(),
            website: None,
            profile: None,
            background: crate::background::Background::default(),
        };
        assert_eq!(generate(&config, &request, &AtomicU64::new(1)).unwrap(), "");
    }

    #[test]
    #[ignore = "requires local Ollama and downloaded qwen3:1.7b; uses synthetic text only"]
    fn midline_live() {
        let config = Config::default();
        let epoch = AtomicU64::new(1);
        for (prefix, suffix, expected) in [
            ("Thank you for your underst", " with this issue.", "anding"),
            ("She sent the invo", " yesterday.", "ice"),
            ("The trans", "ation worked.", "form"),
            ("Put the book", " the shelf.", " on"),
            ("I enjoyed the con", "versation yesterday.", ""),
        ] {
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: suffix.into(),
                app_id: "local.libretypist.test".into(),
                website: None,
                profile: None,
                background: crate::background::Background::default(),
            };
            let start = std::time::Instant::now();
            let output = generate(&config, &request, &epoch).unwrap();
            eprintln!(
                "synthetic midline={prefix:?}|{suffix:?} insertion={output:?} elapsed_ms={}",
                start.elapsed().as_millis()
            );
            assert_eq!(output, expected);
        }
    }

    #[test]
    #[ignore = "requires local Ollama and downloaded qwen3:1.7b; uses synthetic text only"]
    fn partial_word_quality_live() {
        let config = Config::default();
        let epoch = AtomicU64::new(1);
        for (prefix, word_start, completed_word) in [
            ("Thank you for your hel", "hel", "help"),
            ("Thank you for your pat", "pat", "patience"),
            ("We should sched", "sched", "schedule"),
            ("Please send the docu", "docu", "document"),
            ("Gracias por su ayu", "ayu", "ayuda"),
            ("Merci pour votre ai", "ai", "aide"),
            ("Vielen Dank für Ihre Unterstü", "Unterstü", "Unterstützung"),
            ("We need to resch", "resch", "reschedule"),
            ("I will call you tomor", "tomor", "tomorrow"),
            ("The document is confiden", "confiden", "confidential"),
            ("The payment has been recei", "recei", "received"),
            ("Nous avons reçu votre mess", "mess", "message"),
        ] {
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: String::new(),
                app_id: "local.libretypist.test".into(),
                website: None,
                profile: None,
                background: crate::background::Background::default(),
            };
            let start = std::time::Instant::now();
            let output = generate(&config, &request, &epoch).unwrap();
            eprintln!(
                "synthetic partial prefix={prefix:?} insertion={output:?} elapsed_ms={}",
                start.elapsed().as_millis()
            );
            let completed = format!("{word_start}{output}");
            let tail = completed
                .strip_prefix(completed_word)
                .unwrap_or_else(|| panic!("Expected {completed_word:?}, got {completed:?}"));
            assert!(
                tail.is_empty() || !tail.starts_with(char::is_alphanumeric),
                "Unexpected longer word: {completed:?}"
            );
            assert!(!output.is_empty());
            assert!(output.split_whitespace().count() <= config.max_words);
        }
    }

    #[test]
    #[ignore = "requires local Ollama and downloaded qwen3:1.7b; uses synthetic text only"]
    fn local_model_smoke() {
        let config = Config::default();
        let epoch = AtomicU64::new(1);
        let mut nonempty = 0;
        for prefix in [
            "Thank you for your ",
            "Thank you for your hel",
            "Thank you for your tim",
            "Thank you for your pat",
            "Thank you for your underst",
            "I hope you have a",
            "Please send me the ",
        ] {
            let request = CompletionRequest {
                id: 1,
                prefix: prefix.into(),
                suffix: "".into(),
                app_id: "com.apple.TextEdit".into(),
                website: None,
                profile: None,
                background: crate::background::Background::default(),
            };
            let start = std::time::Instant::now();
            let output = generate(&config, &request, &epoch).unwrap();
            eprintln!(
                "synthetic prefix={prefix:?} insertion={output:?} elapsed_ms={}",
                start.elapsed().as_millis()
            );
            if !output.is_empty() {
                nonempty += 1;
            }
            assert!(output.split_whitespace().count() <= config.max_words);
            assert!(!output.contains('\n'));
        }
        assert!(nonempty > 0, "all synthetic completions were suppressed");
    }
}
