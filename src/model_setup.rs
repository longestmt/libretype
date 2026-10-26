// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Explicit model downloads through local Ollama. Requests contain no writing.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
const BASE: &str = "http://127.0.0.1:11434";
const MAX_LINE: u64 = 16 * 1024;
const MAX_STREAM: u64 = 64 * 1024 * 1024;

pub struct Choice {
    pub tag: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}
// Keep the first two entries stable for the isolated download regression probes.
// Gemma4 uses explicit GGUF variants: short aliases can now select MLX assets.
// Sizes are approximate registry download sizes, not runtime memory requirements.
// Memory guidance is conservative LibreType guidance, not a vendor minimum.
pub const CHOICES:[Choice;9]=[
    Choice{tag:"qwen3:1.7b",title:"Qwen3 1.7B",description:"Lightweight starting choice. About 1.4 GB download. Tested on a 16 GB M1; completion quality varies."},
    Choice{tag:"gemma3:1b",title:"Gemma3 1B",description:"Smallest download in this catalog: about 0.8 GB. Optional for memory-constrained Macs; separate restricted terms apply."},
    Choice{tag:"gemma4:e2b-it-q4_K_M",title:"Gemma4 E2B",description:"About 4.6 GB download. Prefer 16 GB or more memory. Faster in small M1 tests, but missed some suggestions and personalized poorly."},
    Choice{tag:"qwen3:4b",title:"Qwen3 4B",description:"About 2.5 GB download. Prefer 16 GB or more memory. Larger models may produce slower suggestions; quality testing is pending."},
    Choice{tag:"gemma3:4b",title:"Gemma3 4B",description:"About 3.4 GB download. Prefer 16 GB or more memory. Larger models may produce slower suggestions; quality testing is pending."},
    Choice{tag:"gemma4:e4b-it-q4_K_M",title:"Gemma4 E4B",description:"About 6.6 GB download. Prefer 16 GB or more memory. An earlier E4B version performed well in writing tests; this variant needs testing."},
    Choice{tag:"qwen3:8b",title:"Qwen3 8B",description:"About 5.2 GB download. Prefer 16 GB or more memory, with room for other apps. Completion-quality and speed testing are pending."},
    Choice{tag:"gemma4:26b-a4b-it-q4_K_M",title:"Gemma4 26B A4B",description:"About 18 GB download. Prefer 32 GB or more memory. Only part of this model is active per token, but the full model needs memory. Testing is pending."},
    Choice{tag:"qwen3:30b",title:"Qwen3 30B A3B",description:"About 19 GB download. Prefer 32 GB or more memory. Only part of this model is active per token, but the full model needs memory. Testing is pending."},
];
impl Choice {
    pub fn restricted_terms(&self) -> bool {
        self.tag.starts_with("gemma3:")
    }
    pub fn license_label(&self) -> &'static str {
        if self.restricted_terms() {
            "Optional model: Gemma Terms of Use (includes use restrictions). Separate from LibreType's GPL."
        } else {
            "Model license: Apache-2.0. Separate from LibreType's GPL."
        }
    }
    pub fn license_url(&self) -> &'static str {
        match self.tag {
            "gemma3:1b" | "gemma3:4b" => "https://ai.google.dev/gemma/terms/",
            "qwen3:1.7b" => "https://huggingface.co/Qwen/Qwen3-1.7B/blob/main/LICENSE",
            "qwen3:4b" => "https://huggingface.co/Qwen/Qwen3-4B/blob/main/LICENSE",
            "qwen3:8b" => "https://huggingface.co/Qwen/Qwen3-8B/blob/main/LICENSE",
            "qwen3:30b" => "https://huggingface.co/Qwen/Qwen3-30B-A3B/blob/main/LICENSE",
            _ => "https://ai.google.dev/gemma/apache_2",
        }
    }
}
pub fn recommended(_memory_bytes: Option<u64>) -> usize {
    // Keep a permissively licensed starting choice at every memory size.
    // Smaller restricted models remain explicit opt-ins; existing choices persist.
    0
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Unavailable,
    Response,
    Download,
    Stalled,
    NotLocal,
    Busy,
    UnknownModel,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
        Self::Cancelled=>"Download cancelled. Ollama may retain partial files for resume.",
        Self::Unavailable=>"Cannot reach local Ollama, or the connection stalled. Open Ollama and retry.",
        Self::Response=>"Ollama returned an incomplete or invalid download response. Retry to resume.",
        Self::Download=>"Ollama could not download this model. Check internet access and free disk space, then retry.",
        Self::Stalled=>"The model download stopped making progress. Check internet access and free disk space, then retry to resume.",
        Self::NotLocal=>"The downloaded model could not be verified as a supported local GGUF or MLX model.",
        Self::Busy=>"A download is already running or stopping.",
        Self::UnknownModel=>"Choose one of the available local models.",
    })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Connecting,
    Manifest,
    Layer { done: u64, total: Option<u64> },
    Verifying,
    Finishing,
    Checking,
    Cancelling,
    Ready,
    Failed(Error),
}
impl Phase {
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Connecting
                | Self::Manifest
                | Self::Layer { .. }
                | Self::Verifying
                | Self::Finishing
                | Self::Checking
                | Self::Cancelling
        )
    }
    pub fn message(&self) -> String {
        match self {
        Self::Idle=>"Choose a model. Downloads start only when you click Download.".into(),
        Self::Connecting=>"Connecting to local Ollama…".into(),
        Self::Manifest=>"Fetching model information…".into(),
        Self::Layer{done,total:Some(total)} if *total>0=>format!("Current layer: {:.1} / {:.1} MiB ({:.0}%)",*done as f64/1048576.,*total as f64/1048576.,*done as f64 / *total as f64 *100.),
        Self::Layer{..}=>"Downloading a model layer…".into(),
        Self::Verifying=>"Ollama is verifying the downloaded files…".into(),
        Self::Finishing=>"Ollama is finishing the installation…".into(),
        Self::Checking=>"Checking that the model runs locally…".into(),
        Self::Cancelling=>"Stopping the download request… a stalled connection can take up to 30 seconds to close.".into(),
        Self::Ready=>"Download complete. Local model verified; click Use selected model to activate it.".into(),
        Self::Failed(error)=>error.to_string(),
    }
    }
}
#[derive(Clone)]
pub struct Snapshot {
    pub job: u64,
    pub choice: usize,
    pub phase: Phase,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            job: 0,
            choice: 0,
            phase: Phase::Idle,
        }
    }
}
#[derive(Deserialize)]
struct Event {
    #[serde(default)]
    status: String,
    digest: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}
fn current(id: u64, epoch: &AtomicU64) -> Result<(), Error> {
    if epoch.load(Ordering::Acquire) == id {
        Ok(())
    } else {
        Err(Error::Cancelled)
    }
}
fn stream(
    reader: impl Read,
    id: u64,
    epoch: &AtomicU64,
    progress: impl FnMut(Phase),
) -> Result<(), Error> {
    let start = Instant::now();
    stream_timed(reader, id, epoch, progress, || start.elapsed())
}

/// A live HTTP stream is not proof of a progressing download: Ollama repeats
/// layer counters while its upstream requests retry. Only new stage/layer
/// evidence or a higher byte count renews this deadline.
#[derive(Default)]
struct DownloadProgress {
    layers: HashMap<String, u64>,
    stages: u8,
    last_advance: Duration,
    finishing: bool,
}
impl DownloadProgress {
    fn check(&self, now: Duration) -> Result<(), Error> {
        let limit = Duration::from_secs(if self.finishing { 300 } else { 120 });
        if now.saturating_sub(self.last_advance) >= limit {
            Err(Error::Stalled)
        } else {
            Ok(())
        }
    }
    fn observe(&mut self, event: &Event, phase: &Phase, now: Duration) -> Result<(), Error> {
        let advanced = if let Some(digest) = &event.digest {
            let done = event.completed.unwrap_or(0);
            if let Some(previous) = self.layers.get_mut(digest) {
                let advanced = done > *previous;
                *previous = (*previous).max(done);
                advanced
            } else {
                if self.layers.len() >= 128 {
                    return Err(Error::Response);
                }
                self.layers.insert(digest.clone(), done);
                true
            }
        } else {
            let bit = match phase {
                Phase::Manifest => 1,
                Phase::Verifying => 2,
                _ => 4,
            };
            let unseen = self.stages & bit == 0;
            self.stages |= bit;
            unseen
        };
        self.finishing = matches!(phase, Phase::Verifying | Phase::Finishing);
        if advanced {
            self.last_advance = now;
        }
        Ok(())
    }
}
fn stream_timed(
    reader: impl Read,
    id: u64,
    epoch: &AtomicU64,
    mut progress: impl FnMut(Phase),
    mut elapsed: impl FnMut() -> Duration,
) -> Result<(), Error> {
    let mut reader = BufReader::new(reader);
    let mut received = 0u64;
    let mut advances = DownloadProgress::default();
    loop {
        current(id, epoch)?;
        if elapsed() > Duration::from_secs(8 * 3600) {
            return Err(Error::Response);
        }
        let mut line = Vec::new();
        loop {
            current(id, epoch)?;
            advances.check(elapsed())?;
            let bytes = reader.fill_buf().map_err(|_| Error::Unavailable)?;
            current(id, epoch)?;
            advances.check(elapsed())?;
            if bytes.is_empty() {
                break;
            }
            let newline = bytes.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(bytes.len(), |index| index + 1);
            received += count as u64;
            if line.len() as u64 + count as u64 > MAX_LINE || received > MAX_STREAM {
                return Err(Error::Response);
            }
            line.extend_from_slice(&bytes[..count]);
            reader.consume(count);
            if newline.is_some() {
                break;
            }
        }
        if line.is_empty() {
            return Err(Error::Response);
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let event: Event = serde_json::from_slice(&line).map_err(|_| Error::Response)?;
        if event.error.is_some() {
            return Err(Error::Download);
        }
        if event.status.len() > 1024
            || event.digest.as_ref().is_some_and(|d| d.len() > 128)
            || matches!((event.completed,event.total),(Some(c),Some(t)) if c>t)
        {
            return Err(Error::Response);
        }
        if event.status == "success" {
            return Ok(());
        }
        let phase = if event.digest.is_some() {
            Phase::Layer {
                done: event.completed.unwrap_or(0),
                total: event.total,
            }
        } else if event.status == "pulling manifest" {
            Phase::Manifest
        } else if event.status.contains("verifying") {
            Phase::Verifying
        } else {
            Phase::Finishing
        };
        advances.observe(&event, &phase, elapsed())?;
        progress(phase);
    }
}
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(5))
        .build()
}
fn verify_at(client: &ureq::Agent, base: &str, tag: &str) -> Result<(), Error> {
    let response = client
        .post(&format!("{base}/api/show"))
        .timeout(Duration::from_secs(30))
        .send_json(json!({"model":tag}))
        .map_err(|_| Error::Unavailable)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::Response);
    }
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Response)?;
    if !crate::model::is_local_model(&metadata) {
        return Err(Error::NotLocal);
    }
    Ok(())
}
pub fn verify_local(choice: usize) -> Result<(), Error> {
    let item = CHOICES.get(choice).ok_or(Error::UnknownModel)?;
    verify_named(item.tag)
}
pub fn verify_named(tag: &str) -> Result<(), Error> {
    if tag.is_empty()
        || tag.len() > 255
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:-".contains(&b))
        || tag.to_ascii_lowercase().contains("cloud")
    {
        return Err(Error::UnknownModel);
    }
    verify_at(&agent(), BASE, tag)
}
fn download_at(
    base: &str,
    choice: usize,
    id: u64,
    epoch: &AtomicU64,
    mut progress: impl FnMut(Phase),
) -> Result<(), Error> {
    let item = CHOICES.get(choice).ok_or(Error::UnknownModel)?;
    current(id, epoch)?;
    let client = agent();
    let response = client
        .post(&format!("{base}/api/pull"))
        // ureq 2's overall deadline overrides its per-read socket timeout.
        // Keep the 30-second read timeout; stream() enforces the total budget.
        .send_json(json!({"model":item.tag,"stream":true,"insecure":false}))
        .map_err(|_| Error::Unavailable)?;
    current(id, epoch)?;
    stream(response.into_reader(), id, epoch, &mut progress)?;
    current(id, epoch)?;
    progress(Phase::Checking);
    verify_at(&client, base, item.tag)?;
    current(id, epoch)
}
pub struct Engine {
    sender: mpsc::SyncSender<(usize, u64)>,
    state: Arc<Mutex<Snapshot>>,
    epoch: Arc<AtomicU64>,
}
impl Engine {
    pub fn new() -> Result<Self, Error> {
        Self::with_base(BASE.to_owned())
    }
    fn with_base(base: String) -> Result<Self, Error> {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let epoch = Arc::new(AtomicU64::new(0));
        let shared = state.clone();
        let worker_epoch = epoch.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("LibreType model download".into())
            .spawn(move || {
                while let Ok((choice, id)) = receiver.recv() {
                    let result = download_at(&base, choice, id, &worker_epoch, |phase| {
                        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                        if state.job == id && worker_epoch.load(Ordering::Acquire) == id {
                            state.phase = phase;
                        }
                    });
                    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if state.job == id {
                        state.phase = if worker_epoch.load(Ordering::Acquire) != id {
                            Phase::Failed(Error::Cancelled)
                        } else {
                            match result {
                                Ok(()) => Phase::Ready,
                                Err(e) => Phase::Failed(e),
                            }
                        };
                    }
                }
            })
            .map_err(|_| Error::Unavailable)?;
        Ok(Self {
            sender,
            state,
            epoch,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn start(&mut self, choice: usize) -> Result<(), Error> {
        if choice >= CHOICES.len() {
            return Err(Error::UnknownModel);
        }
        if self.snapshot().phase.busy() {
            return Err(Error::Busy);
        }
        let id = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Snapshot {
            job: id,
            choice,
            phase: Phase::Connecting,
        };
        self.sender.try_send((choice, id)).map_err(|_| {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).phase = Phase::Failed(Error::Busy);
            Error::Busy
        })
    }
    pub fn cancel(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.phase.busy() {
            self.epoch.fetch_add(1, Ordering::SeqCst);
            state.phase = Phase::Cancelling;
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
    };
    /// Run only through scripts/check_fresh_model.py, which owns a separate
    /// loopback Ollama process with empty temporary model storage.
    #[test]
    #[ignore = "downloads Gemma3 1B into an isolated Ollama server"]
    fn fresh_download_cancel_and_retry_live() {
        let base =
            std::env::var("LIBRETYPE_TEST_OLLAMA_BASE").expect("isolated test endpoint required");
        let parsed = url::Url::parse(&base).unwrap();
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.port().is_some_and(|p| p != 0 && p != 11434));
        assert_eq!(parsed.path(), "/");
        assert!(
            parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none()
        );
        let client = agent();
        let tags: Value = client
            .get(&format!("{base}/api/tags"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(
            tags["models"].as_array().unwrap().len(),
            0,
            "fresh storage required"
        );
        let mut engine = Engine::with_base(base.clone()).unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(600);
        let report = |event: &str, phase: &Phase, job: u64| {
            println!(
                "{}",
                json!({"event":event,"phase":format!("{phase:?}"),"job":job,"elapsed_ms":started.elapsed().as_millis()})
            );
            std::io::stdout().flush().unwrap();
        };
        engine.start(1).unwrap();
        let mut last_report = started;
        let cancelled_bytes = loop {
            assert!(Instant::now() < deadline, "fresh download timed out");
            let state = engine.snapshot();
            if last_report.elapsed() >= Duration::from_secs(1) {
                report("progress_before_cancel", &state.phase, state.job);
                last_report = Instant::now();
            }
            if let Phase::Layer {
                done,
                total: Some(total),
            } = state.phase
            {
                if done >= 16 * 1024 * 1024 && done < total {
                    report("cancel_requested", &state.phase, state.job);
                    engine.cancel();
                    break done;
                }
            }
            assert!(
                state.phase.busy(),
                "download ended before cancellation: {:?}",
                state.phase
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let cancel_started = Instant::now();
        while engine.snapshot().phase.busy() {
            assert!(
                cancel_started.elapsed() < Duration::from_secs(35),
                "cancellation stalled"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(engine.snapshot().phase, Phase::Failed(Error::Cancelled));
        report(
            "cancel_confirmed",
            &engine.snapshot().phase,
            engine.snapshot().job,
        );
        assert!(
            verify_at(&client, &base, CHOICES[1].tag).is_err(),
            "cancelled partial download must not be ready"
        );
        engine.start(1).unwrap();
        let mut saw_retry_bytes = false;
        loop {
            assert!(Instant::now() < deadline, "retry timed out");
            let state = engine.snapshot();
            if matches!(state.phase, Phase::Layer { done, .. } if done >= cancelled_bytes) {
                saw_retry_bytes = true;
            }
            if last_report.elapsed() >= Duration::from_secs(1) || !state.phase.busy() {
                report("retry_progress", &state.phase, state.job);
                last_report = Instant::now();
            }
            if !state.phase.busy() {
                assert_eq!(state.phase, Phase::Ready);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_retry_bytes);
        assert_eq!(verify_at(&client, &base, CHOICES[1].tag), Ok(()));
        report(
            "verified_local",
            &engine.snapshot().phase,
            engine.snapshot().job,
        );
    }

    #[test]
    #[ignore = "downloads Gemma3 1B through an isolated outage relay"]
    fn download_network_outage_and_retry_live() {
        let endpoint = |name: &str| {
            let base = std::env::var(name).expect("isolated endpoint required");
            let parsed = url::Url::parse(&base).unwrap();
            assert_eq!(parsed.scheme(), "http");
            assert_eq!(parsed.host_str(), Some("127.0.0.1"));
            assert!(parsed.port().is_some_and(|p| p != 0 && p != 11434));
            assert_eq!(parsed.path(), "/");
            assert!(
                parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
            );
            base
        };
        let base = endpoint("LIBRETYPE_TEST_OLLAMA_BASE");
        let control = endpoint("LIBRETYPE_TEST_FAULT_CONTROL");
        let stall = std::env::var("LIBRETYPE_TEST_FAULT_MODE").is_ok_and(|s| s == "stall");
        let token = std::env::var("LIBRETYPE_TEST_FAULT_TOKEN").unwrap();
        let client = agent();
        let change_network = |mode: &str| -> Value {
            client
                .post(&format!("{control}/{mode}"))
                .set("X-LibreType-Test-Token", &token)
                .send_json(json!({}))
                .unwrap()
                .into_json()
                .unwrap()
        };
        let tags: Value = client
            .get(&format!("{base}/api/tags"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert!(tags["models"].as_array().unwrap().is_empty());
        let mut engine = Engine::with_base(base.clone()).unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(600);
        let report = |event: &str, phase: &Phase, detail: Value| {
            println!(
                "{}",
                json!({"event":event,"phase":format!("{phase:?}"),"detail":detail,"elapsed_ms":started.elapsed().as_millis()})
            );
            std::io::stdout().flush().unwrap();
        };
        engine.start(1).unwrap();
        loop {
            assert!(Instant::now() < deadline);
            let phase = engine.snapshot().phase;
            if matches!(phase, Phase::Layer { done, total:Some(total) } if done >= 16*1024*1024 && done < total)
            {
                let network = change_network(if stall { "stall" } else { "offline" });
                if stall {
                    assert_eq!(network["stalled"], true);
                    assert_eq!(network["cut_sockets"], 0);
                } else {
                    assert_eq!(network["offline"], true);
                    assert!(network["cut_sockets"].as_u64().unwrap() >= 2);
                }
                report("network_cut", &phase, network);
                break;
            }
            assert!(phase.busy(), "download ended before outage: {phase:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let offline_started = Instant::now();
        let mut last_report = Instant::now();
        loop {
            assert!(
                offline_started.elapsed() < Duration::from_secs(180),
                "outage did not produce an error"
            );
            let phase = engine.snapshot().phase;
            if !phase.busy() {
                assert!(
                    if stall {
                        phase == Phase::Failed(Error::Stalled)
                    } else {
                        matches!(phase, Phase::Failed(Error::Download | Error::Unavailable))
                    },
                    "false success during outage: {phase:?}"
                );
                report("network_failed", &phase, Value::Null);
                break;
            }
            if last_report.elapsed() > Duration::from_secs(5) {
                report("offline_progress", &phase, Value::Null);
                last_report = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(verify_at(&client, &base, CHOICES[1].tag).is_err());
        let network = change_network("online");
        assert_eq!(network["offline"], false);
        if stall {
            assert_eq!(network["stalled"], false);
            assert_eq!(network["cut_sockets"], 0);
            assert!(network["stall_waits"].as_u64().unwrap() > 0);
        } else {
            assert!(network["blocked_connections"].as_u64().unwrap() > 0);
        }
        report("network_restored", &engine.snapshot().phase, network);
        engine.start(1).unwrap();
        loop {
            assert!(Instant::now() < deadline);
            let phase = engine.snapshot().phase;
            if !phase.busy() {
                assert_eq!(phase, Phase::Ready);
                report("verified_local", &phase, Value::Null);
                break;
            }
            if last_report.elapsed() > Duration::from_secs(1) {
                report("retry_progress", &phase, Value::Null);
                last_report = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(verify_at(&client, &base, CHOICES[1].tag), Ok(()));
    }
    #[test]
    #[ignore = "checks two real 30-second silent socket deadlines on loopback"]
    fn silent_download_timeout_and_cancellation_live() {
        for cancel in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (release, hold) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let (line, _) = request(&mut socket);
                assert_eq!(line, "POST /api/pull HTTP/1.1\r\n");
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: 99999\r\n\r\n{\"status\":\"pulling manifest\"}\n").unwrap();
                socket.flush().unwrap();
                // Keep the incomplete body open until the client has timed out.
                let _ = hold.recv_timeout(Duration::from_secs(40));
            });
            let mut engine = Engine::with_base(base).unwrap();
            engine.start(1).unwrap();
            let started = Instant::now();
            while engine.snapshot().phase == Phase::Connecting {
                assert!(started.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(engine.snapshot().phase, Phase::Manifest);
            if cancel {
                engine.cancel();
            }
            while engine.snapshot().phase.busy() {
                assert!(
                    started.elapsed() < Duration::from_secs(35),
                    "silent socket did not close"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let expected = if cancel {
                Error::Cancelled
            } else {
                Error::Unavailable
            };
            assert_eq!(engine.snapshot().phase, Phase::Failed(expected));
            assert!(started.elapsed() >= Duration::from_secs(25));
            println!(
                "{}",
                json!({"scenario":if cancel {"silent_cancel"} else {"silent_timeout"},"elapsed_ms":started.elapsed().as_millis(),"phase":format!("{:?}",engine.snapshot().phase)})
            );
            std::io::stdout().flush().unwrap();
            release.send(()).unwrap();
            server.join().unwrap();
        }
    }
    fn request(stream: &mut TcpStream) -> (String, Value) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut size = 0;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header == "\r\n" {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    size = value.trim().parse::<usize>().unwrap();
                }
            }
        }
        assert!(size < 4096);
        let mut bytes = vec![0; size];
        reader.read_exact(&mut bytes).unwrap();
        (first, serde_json::from_slice(&bytes).unwrap())
    }
    fn response(stream: &mut TcpStream, body: &str) {
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    }
    fn server(metadata: Value) -> (String, std::thread::JoinHandle<()>) {
        server_for("qwen3:1.7b", metadata)
    }
    fn server_for(tag: &'static str, metadata: Value) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            for path in ["/api/pull", "/api/show"] {
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Expected request did not arrive");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                let (line, body) = request(&mut socket);
                assert_eq!(line, format!("POST {path} HTTP/1.1\r\n"));
                if path == "/api/pull" {
                    assert_eq!(body, json!({"model":tag,"stream":true,"insecure":false}));
                    response(
                        &mut socket,
                        "{\"status\":\"pulling manifest\"}\n{\"status\":\"success\"}\n",
                    );
                } else {
                    assert_eq!(body, json!({"model":tag}));
                    response(&mut socket, &metadata.to_string());
                }
            }
        });
        (base, handle)
    }
    #[test]
    fn bounded_stream_requires_success_and_tracks_each_layer_separately() {
        let epoch = AtomicU64::new(1);
        let mut phases = Vec::new();
        let input=b"{\"status\":\"pulling manifest\"}\n{\"status\":\"pulling layer\",\"digest\":\"a\",\"total\":100,\"completed\":50}\n{\"status\":\"pulling layer\",\"digest\":\"b\",\"total\":10,\"completed\":0}\n{\"status\":\"verifying sha256 digest\"}\n{\"status\":\"success\"}\n";
        stream(input.as_slice(), 1, &epoch, |p| phases.push(p)).unwrap();
        assert_eq!(
            phases,
            vec![
                Phase::Manifest,
                Phase::Layer {
                    done: 50,
                    total: Some(100)
                },
                Phase::Layer {
                    done: 0,
                    total: Some(10)
                },
                Phase::Verifying
            ]
        );
        for invalid in [
            b"".as_slice(),
            b"{}\n",
            b"{\"status\":\"pulling manifest\"}\n",
            b"{\"status\":\"x\",\"total\":1,\"completed\":2}\n",
            b"not JSON\n",
        ] {
            assert_eq!(stream(invalid, 1, &epoch, |_| {}), Err(Error::Response));
        }
        assert_eq!(
            stream(
                b"{\"error\":\"private path in server error\"}\n".as_slice(),
                1,
                &epoch,
                |_| {}
            ),
            Err(Error::Download)
        );
        assert_eq!(
            stream(
                vec![b'x'; MAX_LINE as usize + 1].as_slice(),
                1,
                &epoch,
                |_| {}
            ),
            Err(Error::Response)
        );
        assert_eq!(
            download_at("http://127.0.0.1:1", 0, 2, &epoch, |_| {}),
            Err(Error::Cancelled)
        );
        assert_eq!(
            download_at("http://127.0.0.1:1", 99, 1, &epoch, |_| {}),
            Err(Error::UnknownModel)
        );
        assert_eq!(recommended(Some(8 * 1024 * 1024 * 1024)), 0);
        assert_eq!(recommended(Some(16 * 1024 * 1024 * 1024)), 0);
    }
    #[test]
    fn cancellation_is_checked_between_partial_progress_reads() {
        struct Partial<'a>(&'a AtomicU64, bool);
        impl Read for Partial<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(!self.1, "Cancelled reader must not request more bytes");
                self.1 = true;
                let fragment = b"{\"status\":";
                output[..fragment.len()].copy_from_slice(fragment);
                self.0.store(2, Ordering::Release);
                Ok(fragment.len())
            }
        }
        let epoch = AtomicU64::new(1);
        assert_eq!(
            stream(Partial(&epoch, false), 1, &epoch, |_| {}),
            Err(Error::Cancelled)
        );
        for tag in ["", "model:cloud", "bad\nname", "name with spaces"] {
            assert_eq!(verify_named(tag), Err(Error::UnknownModel));
        }
    }
    fn timed_events(events: Vec<(u64, String)>) -> Result<(), Error> {
        use std::cell::Cell;
        struct Script<'a> {
            events: std::vec::IntoIter<(u64, String)>,
            seconds: &'a Cell<u64>,
        }
        impl Read for Script<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let Some((delay, text)) = self.events.next() else {
                    return Ok(0);
                };
                self.seconds.set(self.seconds.get() + delay);
                assert!(text.len() <= output.len());
                output[..text.len()].copy_from_slice(text.as_bytes());
                Ok(text.len())
            }
        }
        let seconds = Cell::new(0);
        stream_timed(
            Script {
                events: events.into_iter(),
                seconds: &seconds,
            },
            1,
            &AtomicU64::new(1),
            |_| {},
            || Duration::from_secs(seconds.get()),
        )
    }
    fn layer(digest: &str, done: u64) -> String {
        format!(
            "{}\n",
            json!({"status":"pulling layer","digest":digest,"total":100,"completed":done})
        )
    }
    #[test]
    fn repeated_regressing_and_partial_progress_cannot_extend_a_stalled_download() {
        for events in [
            vec![
                (0, layer("a", 10)),
                (60, layer("a", 9)),
                (60, layer("a", 10)),
            ],
            vec![
                (0, layer("a", 10)),
                (60, layer("b", 10)),
                (60, layer("a", 10)),
                (60, layer("b", 10)),
            ],
            vec![(0, layer("a", 10)), (60, " ".into()), (60, " ".into())],
            vec![
                (0, layer("a", 10)),
                (120, "{\"status\":\"success\"}\n".into()),
            ],
        ] {
            assert_eq!(timed_events(events), Err(Error::Stalled));
        }
    }
    #[test]
    fn slow_advancing_layers_and_file_verification_keep_their_progress_budget() {
        assert_eq!(
            timed_events(vec![
                (0, "{\"status\":\"pulling manifest\"}\n".into()),
                (119, layer("a", 0)),
                (119, layer("a", 1)),
                (119, layer("b", 0)),
                (119, layer("a", 2)),
                (119, "{\"status\":\"verifying sha256 digest\"}\n".into()),
                (298, "{\"status\":\"success\"}\n".into()),
            ]),
            Ok(())
        );
        assert_eq!(
            timed_events(vec![
                (0, "{\"status\":\"verifying sha256 digest\"}\n".into()),
                (150, "{\"status\":\"verifying sha256 digest\"}\n".into()),
                (150, "{\"status\":\"success\"}\n".into()),
            ]),
            Err(Error::Stalled)
        );
    }
    #[test]
    fn status_chatter_and_unbounded_new_layers_cannot_renew_forever() {
        let chatter = (0..=5)
            .map(|i| {
                (
                    if i == 0 { 0 } else { 60 },
                    format!("{}\n", json!({"status":format!("finishing step {i}")})),
                )
            })
            .collect();
        assert_eq!(timed_events(chatter), Err(Error::Stalled));
        let layers = (0..129).map(|i| (1, layer(&i.to_string(), 0))).collect();
        assert_eq!(timed_events(layers), Err(Error::Response));
    }
    #[test]
    fn success_requires_local_metadata_and_sends_only_download_fields() {
        for (metadata, result) in [
            (json!({"details":{"format":"gguf"}}), Ok(())),
            (
                json!({"details":{"format":"safetensors","runner":"mlx"},"capabilities":["completion"]}),
                Ok(()),
            ),
            (
                json!({"details":{"format":"safetensors","runner":"mlx"},"capabilities":["completion"],"remote_model":"remote"}),
                Err(Error::NotLocal),
            ),
            (
                json!({"details":{"format":"gguf"},"remote_host":false}),
                Err(Error::NotLocal),
            ),
            (
                json!({"details":{"format":"gguf"},"remote_host":"example.invalid"}),
                Err(Error::NotLocal),
            ),
            (json!({"details":{"format":"other"}}), Err(Error::NotLocal)),
        ] {
            let (base, server) = server(metadata);
            let epoch = AtomicU64::new(1);
            assert_eq!(download_at(&base, 0, 1, &epoch, |_| {}), result);
            server.join().unwrap();
        }
    }
    #[test]
    fn every_catalog_choice_downloads_and_verifies_the_selected_model() {
        let expected = [
            "qwen3:1.7b",
            "gemma3:1b",
            "gemma4:e2b-it-q4_K_M",
            "qwen3:4b",
            "gemma3:4b",
            "gemma4:e4b-it-q4_K_M",
            "qwen3:8b",
            "gemma4:26b-a4b-it-q4_K_M",
            "qwen3:30b",
        ];
        assert_eq!(CHOICES.map(|c| c.tag), expected);
        for (choice, tag) in expected.into_iter().enumerate() {
            let (base, server) = server_for(tag, json!({"details":{"format":"gguf"}}));
            let mut engine = Engine::with_base(base).unwrap();
            engine.start(choice).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let state = engine.snapshot();
                assert_eq!(state.choice, choice);
                assert_eq!(state.job, 1);
                if !state.phase.busy() {
                    assert_eq!(state.phase, Phase::Ready);
                    break;
                }
                assert!(Instant::now() < deadline, "Catalog download did not finish");
                std::thread::sleep(Duration::from_millis(5));
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn cancellation_prevents_a_late_success_from_becoming_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (started, waiting) = mpsc::sync_channel(1);
        let (release, finish) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            request(&mut socket);
            started.send(()).unwrap();
            finish.recv_timeout(Duration::from_secs(3)).unwrap();
            response(&mut socket, "{\"status\":\"success\"}\n");
        });
        let mut engine = Engine::with_base(base).unwrap();
        engine.start(0).unwrap();
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        engine.cancel();
        assert_eq!(engine.snapshot().phase, Phase::Cancelling);
        assert_eq!(engine.start(1), Err(Error::Busy));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while engine.snapshot().phase.busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(engine.snapshot().phase, Phase::Failed(Error::Cancelled));
        server.join().unwrap();
    }
}
