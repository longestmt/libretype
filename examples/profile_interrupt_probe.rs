// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! Synthetic profile cancellation through the production worker and local model.
//! Never reads app settings, recorded writing, or Keychain.
use libretypist::{
    config::Config,
    core::CompletionRequest,
    history::{
        self,
        profile::BuildRequest,
        store::{MasterKey, Vault},
        worker::{Analyzer, Engine, KeySource, Status},
        Error, Profile,
    },
    model,
};
use std::{
    io::Write,
    path::PathBuf,
    sync::{atomic::AtomicU64, mpsc},
    time::{Duration, Instant},
};

struct TemporaryVault(PathBuf);
impl Drop for TemporaryVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct SyntheticKey;
impl KeySource for SyntheticKey {
    fn load(&mut self, _: bool) -> Result<MasterKey, Error> {
        Ok(MasterKey::from_bytes([73; 32]))
    }
}
struct ObservedAnalyzer {
    entered: mpsc::Sender<()>,
    exited: mpsc::Sender<(Instant, bool)>,
}
impl Analyzer for ObservedAnalyzer {
    fn generate(
        &mut self,
        config: &Config,
        request: &BuildRequest,
        epoch: &AtomicU64,
    ) -> Result<Profile, Error> {
        let _ = self.entered.send(());
        let result = request.generate(config, epoch);
        let _ = self.exited.send((
            Instant::now(),
            matches!(result, Err(model::ModelError::Stale)),
        ));
        result.map_err(|_| Error::Analysis)
    }
}
fn wait_ready(engine: &Engine) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    loop {
        let state = engine.snapshot();
        if state.status == Status::Ready && state.unlocked {
            return Ok(());
        }
        if matches!(state.status, Status::Failed(_)) || start.elapsed() > Duration::from_secs(3) {
            return Err(
                format!("Synthetic worker did not become ready: {:?}", state.status).into(),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("Usage: profile_interrupt_probe MODEL (synthetic writing only)".into());
    }
    let config = Config {
        model: args[1].clone(),
        ..Config::default()
    };
    let temporary = TemporaryVault(std::env::temp_dir().join(format!(
        "libretype-profile-interrupt-{:x}",
        u128::from_be_bytes(history::new_id()?)
    )));
    let path = temporary.0.join("writing.lt");
    let mut data = history::Data::default();
    for (index, sample) in history::profile::SYNTHETIC_VOCABULARY_SAMPLES
        .iter()
        .enumerate()
    {
        data.upsert(history::Record {
            id: [index as u8 + 1; 16],
            scope: history::Scope {
                app: "local.libretype.synthetic-interrupt".into(),
                website: None,
            },
            created_at: 10,
            updated_at: 20,
            text: (*sample).into(),
        })?;
    }
    let profile = BuildRequest::new(&data, &config, 1, 30)?.finish(
        &config,
        r#"{"guidance":"Brief, friendly writing.","vocabulary":["Ariadne"]}"#,
    )?;
    data.set_profile(profile)?;
    let mut vault = Vault::open(path.clone(), MasterKey::from_bytes([73; 32]))?;
    vault.transaction(|stored| {
        *stored = data;
        Ok(())
    })?;
    drop(vault);
    let saved = std::fs::read(&path)?;
    let (entered, started) = mpsc::channel();
    let (exited, ended) = mpsc::channel();
    let mut engine = Engine::with_analyzer(
        path.clone(),
        SyntheticKey,
        ObservedAnalyzer { entered, exited },
    )?;
    engine.unlock(&config, 30)?;
    wait_ready(&engine)?;
    for (trial, delay_ms) in [150, 300, 600].into_iter().enumerate() {
        engine.build_background(&config, 30)?;
        started.recv_timeout(Duration::from_secs(3))?;
        std::thread::sleep(Duration::from_millis(delay_ms));
        let canceled = Instant::now();
        if !engine.interrupt_background(&config, 30)? {
            return Err(
                "Analysis was already complete: this trial cannot prove cancellation".into(),
            );
        }
        let (stopped, stale) = ended.recv_timeout(Duration::from_secs(2))?;
        if !stale || stopped < canceled {
            return Err("Analysis ended before interruption or did not report cancellation".into());
        }
        wait_ready(&engine)?;
        if std::fs::read(&path)? != saved {
            return Err("Canceled analysis changed the synthetic encrypted snapshot".into());
        }
        println!(
            "{}",
            serde_json::json!({"event":"cancellation","trial":trial,"delay_ms":delay_ms,
            "model":config.model,"call_exit_after_cancel_ms":stopped.duration_since(canceled).as_secs_f64()*1000.,
            "vault_unchanged":true})
        );
        std::io::stdout().flush()?;
        let next = Instant::now();
        let output = model::generate(
            &config,
            &CompletionRequest {
                id: 1,
                prefix: "Thank you for your ".into(),
                suffix: String::new(),
                app_id: "local.libretype.synthetic-interrupt".into(),
                website: None,
                profile: None,
                background: Default::default(),
            },
            &AtomicU64::new(1),
        )?;
        println!(
            "{}",
            serde_json::json!({"event":"following_completion","trial":trial,
            "elapsed_ms":next.elapsed().as_secs_f64()*1000.,"output":output})
        );
        std::io::stdout().flush()?;
        if output.trim().is_empty() {
            return Err("No usable following completion".into());
        }
    }
    let previous_job = engine.snapshot().last_job;
    engine.lock()?;
    let start = Instant::now();
    while engine.snapshot().status != Status::Locked
        || engine.snapshot().unlocked
        || engine.snapshot().last_job <= previous_job
    {
        if start.elapsed() > Duration::from_secs(3) {
            return Err("Lock timed out".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(engine);
    drop(temporary);
    Ok(())
}
