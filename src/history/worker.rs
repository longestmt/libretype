// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Serialized vault ownership, bounded queues and a separately cancellable model
//! worker. Construction performs no filesystem, Keychain or model access.
use super::{
    profile::{source_policy, BuildRequest},
    store::{MasterKey, Vault},
    Error, Profile, Record, Scope, Selector,
};
use crate::config::Config;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

pub trait KeySource: Send + 'static {
    /// Called only for an explicit Unlock request. Never replace a missing key
    /// when `create` is false (an encrypted vault already exists).
    fn load(&mut self, create: bool) -> Result<MasterKey, Error>;
}
pub trait Analyzer: Send + 'static {
    fn generate(
        &mut self,
        config: &Config,
        request: &BuildRequest,
        epoch: &AtomicU64,
    ) -> Result<Profile, Error>;
}
pub struct LocalAnalyzer;
impl Analyzer for LocalAnalyzer {
    fn generate(
        &mut self,
        config: &Config,
        request: &BuildRequest,
        epoch: &AtomicU64,
    ) -> Result<Profile, Error> {
        request.generate(config, epoch).map_err(|_| Error::Analysis)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Locked,
    Unlocking,
    Ready,
    Building,
    Updating,
    Failed(Error),
}
#[derive(Clone, Debug)]
pub struct SourceCount {
    pub scope: Scope,
    pub records: usize,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub version: u64,
    pub last_job: u64,
    pub status: Status,
    pub unlocked: bool,
    pub records: usize,
    pub revision: u64,
    pub next_expiration: Option<u64>,
    pub profile: Option<Profile>,
    pub sources: Vec<SourceCount>,
    session: u64,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 0,
            last_job: 0,
            status: Status::Locked,
            unlocked: false,
            records: 0,
            revision: 0,
            next_expiration: None,
            profile: None,
            sources: vec![],
            session: 1,
        }
    }
}
impl Snapshot {
    /// None means unavailable, never zero: do not expose stale counts while
    /// locking, pruning, deleting, updating, or recovering from an error.
    pub fn record_count(&self, selector: &Selector, now: u64) -> Option<usize> {
        if !selector.valid()
            || !self.unlocked
            || !matches!(self.status, Status::Ready | Status::Building)
            || self.next_expiration.is_some_and(|deadline| now >= deadline)
        {
            return None;
        }
        Some(
            self.sources
                .iter()
                .filter(|s| selector.matches(&s.scope))
                .map(|s| s.records)
                .sum(),
        )
    }
}
enum Command {
    Unlock,
    Sync,
    Append(Vec<Record>),
    Delete(Selector),
    Build,
    CancelBuild,
    Lock,
}
struct Job {
    sequence: u64,
    session: u64,
    analysis: u64,
    config: Config,
    now: u64,
    command: Command,
}
struct AnalysisJob {
    session: u64,
    analysis: u64,
    config: Config,
    request: BuildRequest,
}
struct AnalysisReply {
    session: u64,
    analysis: u64,
    result: Result<Profile, Error>,
}

pub struct Engine {
    sender: mpsc::SyncSender<Job>,
    shared: Arc<Mutex<Snapshot>>,
    session: Arc<AtomicU64>,
    analysis: Arc<AtomicU64>,
    locked: Arc<AtomicBool>,
    sequence: u64,
    last_change: u64,
    background_analysis: Option<u64>,
}
impl Engine {
    pub fn new(path: PathBuf, keys: impl KeySource) -> Result<Self, Error> {
        Self::with_analyzer(path, keys, LocalAnalyzer)
    }
    pub fn with_analyzer(
        path: PathBuf,
        keys: impl KeySource,
        analyzer: impl Analyzer,
    ) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let session = Arc::new(AtomicU64::new(1));
        let analysis = Arc::new(AtomicU64::new(1));
        let locked = Arc::new(AtomicBool::new(true));
        let mut owner = Owner {
            path,
            keys: Box::new(keys),
            vault: None,
            config: Config::default(),
            shared: shared.clone(),
            session: session.clone(),
            analysis: analysis.clone(),
            locked: locked.clone(),
            last_job: 0,
            active_analysis: None,
        };
        std::thread::Builder::new()
            .name("LibreType history".into())
            .spawn(move || owner.run(receiver, analyzer))
            .map_err(|_| Error::Io)?;
        Ok(Self {
            sender,
            shared,
            session,
            analysis,
            locked,
            sequence: 0,
            last_change: 0,
            background_analysis: None,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if snapshot.last_job < self.last_change {
            snapshot.profile = None;
        }
        snapshot
    }
    /// Explicit user action; this is the only operation allowed to call KeySource.
    pub fn unlock(&mut self, config: &Config, now: u64) -> Result<(), Error> {
        self.submit(config, now, Command::Unlock, true)
    }
    /// Call after saving any source/retention settings. Clear native tracker and
    /// completion caches first; queued writes and model results are invalidated.
    pub fn configure(&mut self, config: &Config, now: u64) -> Result<(), Error> {
        self.submit(config, now, Command::Sync, true)
    }
    pub fn lock(&mut self) -> Result<(), Error> {
        self.submit(&Config::default(), 0, Command::Lock, true)
    }
    pub fn append(&mut self, config: &Config, records: Vec<Record>, now: u64) -> Result<(), Error> {
        if records.is_empty() {
            return Ok(());
        }
        if records.len() > 16 || records.iter().any(|r| !r.valid()) {
            return Err(Error::InvalidData);
        }
        if !self.snapshot().unlocked {
            return Err(Error::Locked);
        }
        let records: Vec<_> = records
            .into_iter()
            .filter(|r| config.history.allows_recording(config, &r.scope))
            .collect();
        if records.is_empty() {
            return Ok(());
        }
        self.submit(config, now, Command::Append(records), false)
    }
    pub fn delete(&mut self, config: &Config, selector: Selector, now: u64) -> Result<(), Error> {
        if !selector.valid() {
            return Err(Error::InvalidData);
        }
        if !self.snapshot().unlocked {
            return Err(Error::Locked);
        }
        self.submit(config, now, Command::Delete(selector), true)
    }
    pub fn build(&mut self, config: &Config, now: u64) -> Result<(), Error> {
        if !self.snapshot().unlocked {
            return Err(Error::Locked);
        }
        self.submit(config, now, Command::Build, false)
    }
    /// Automatic refresh may yield to keyboard activity; explicit builds do not.
    pub fn build_background(&mut self, config: &Config, now: u64) -> Result<(), Error> {
        self.build(config, now)?;
        self.background_analysis = Some(self.analysis.load(Ordering::Acquire));
        Ok(())
    }
    /// Invalidate inference immediately, then let the owner restore its ready
    /// snapshot without changing stored writing or the previously saved profile.
    pub fn interrupt_background(&mut self, config: &Config, now: u64) -> Result<bool, Error> {
        let Some(analysis) = self.background_analysis.take() else {
            return Ok(false);
        };
        if self.analysis.load(Ordering::Acquire) != analysis
            || self.snapshot().status != Status::Building
        {
            return Ok(false);
        }
        self.submit(config, now, Command::CancelBuild, false)?;
        Ok(true)
    }
    fn submit(
        &mut self,
        config: &Config,
        now: u64,
        command: Command,
        invalidate: bool,
    ) -> Result<(), Error> {
        config.validate().map_err(|_| Error::InvalidData)?;
        self.background_analysis = None;
        if matches!(command, Command::Lock) {
            self.locked.store(true, Ordering::Release);
        }
        if matches!(command, Command::Unlock) {
            self.locked.store(false, Ordering::Release);
        }
        self.sequence = self.sequence.checked_add(1).ok_or(Error::Capacity)?;
        self.last_change = self.sequence;
        let analysis = self.analysis.fetch_add(1, Ordering::SeqCst) + 1;
        let session = if invalidate {
            self.session.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.session.load(Ordering::Acquire)
        };
        {
            let mut snapshot = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            snapshot.session = session;
            snapshot.version += 1;
            snapshot.profile = None;
            snapshot.status = match command {
                Command::Unlock => Status::Unlocking,
                Command::Lock => Status::Locked,
                Command::Build => Status::Building,
                _ => Status::Updating,
            };
            if matches!(command, Command::Lock | Command::Unlock) {
                snapshot.unlocked = false;
                snapshot.records = 0;
                snapshot.sources.clear();
            }
        }
        let job = Job {
            sequence: self.sequence,
            session,
            analysis,
            config: config.clone(),
            now,
            command,
        };
        self.sender.try_send(job).map_err(|_| {
            let mut snapshot = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            snapshot.status = Status::Failed(Error::Busy);
            snapshot.version += 1;
            Error::Busy
        })
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.locked.store(true, Ordering::Release);
        self.session.fetch_add(1, Ordering::SeqCst);
        self.analysis.fetch_add(1, Ordering::SeqCst);
        // Sender closes after Drop. Never wait on a Keychain dialog/model on the UI thread.
    }
}
struct Owner {
    path: PathBuf,
    keys: Box<dyn KeySource>,
    vault: Option<Vault>,
    config: Config,
    shared: Arc<Mutex<Snapshot>>,
    session: Arc<AtomicU64>,
    analysis: Arc<AtomicU64>,
    locked: Arc<AtomicBool>,
    last_job: u64,
    active_analysis: Option<u64>,
}
impl Owner {
    fn publish(&self, session: u64, status: Status) {
        let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if self.session.load(Ordering::Acquire) != session {
            return;
        }
        let version = state.version + 1;
        let mut next = Snapshot {
            version,
            last_job: self.last_job,
            session,
            status,
            ..Snapshot::default()
        };
        if let Some(vault) = &self.vault {
            next.unlocked = true;
            next.records = vault.data().records.len();
            next.revision = vault.data().revision;
            next.next_expiration = vault
                .data()
                .records
                .iter()
                .map(|record| {
                    record
                        .updated_at
                        .saturating_add(self.config.history.retention_days.saturating_mul(86400))
                        .saturating_add(1)
                })
                .min();
            next.profile = vault
                .data()
                .profile
                .clone()
                .filter(|p| p.source_policy == source_policy(&self.config));
            let mut counts = std::collections::BTreeMap::new();
            for record in &vault.data().records {
                *counts
                    .entry((record.scope.app.clone(), record.scope.website.clone()))
                    .or_insert(0) += 1;
            }
            next.sources = counts
                .into_iter()
                .map(|((app, website), records)| SourceCount {
                    scope: Scope { app, website },
                    records,
                })
                .collect();
        }
        *state = next;
    }
    fn normalize(&mut self, now: u64) -> Result<(), Error> {
        let vault = self.vault.as_mut().ok_or(Error::Locked)?;
        let mut next = vault.data().clone();
        let before = next.revision;
        next.prune(&self.config.history, now)?;
        if next
            .profile
            .as_ref()
            .is_some_and(|p| p.source_policy != source_policy(&self.config))
        {
            next.invalidate_profile()?;
        }
        if next.revision != before {
            vault.transaction(|data| {
                *data = next;
                Ok(())
            })?;
        }
        Ok(())
    }
    fn run(&mut self, receiver: mpsc::Receiver<Job>, mut analyzer: impl Analyzer) {
        let (analysis_tx, analysis_rx) = mpsc::sync_channel::<AnalysisJob>(1);
        let (result_tx, result_rx) = mpsc::sync_channel::<AnalysisReply>(1);
        let epoch = self.analysis.clone();
        if std::thread::Builder::new()
            .name("LibreType style".into())
            .spawn(move || {
                while let Ok(work) = analysis_rx.recv() {
                    if epoch.load(Ordering::Acquire) != work.analysis {
                        continue;
                    }
                    let result = analyzer.generate(&work.config, &work.request, &epoch);
                    if epoch.load(Ordering::Acquire) == work.analysis
                        && result_tx
                            .send(AnalysisReply {
                                session: work.session,
                                analysis: work.analysis,
                                result,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            })
            .is_err()
        {
            self.publish(
                self.session.load(Ordering::Acquire),
                Status::Failed(Error::Io),
            );
            return;
        }
        loop {
            // A full command queue cannot prevent a requested lock from dropping
            // cached key material once an in-flight OS call/transaction returns.
            if self.locked.load(Ordering::Acquire) && self.vault.is_some() {
                self.vault = None;
                self.active_analysis = None;
                self.publish(self.session.load(Ordering::Acquire), Status::Locked);
            }
            match receiver.recv_timeout(Duration::from_millis(30)) {
                Ok(job) => {
                    if job.session != self.session.load(Ordering::Acquire) {
                        continue;
                    }
                    self.last_job = job.sequence;
                    self.active_analysis = None;
                    // An append producer may hold an older config snapshot. It
                    // cannot roll back source policy after an explicit configure.
                    if !matches!(&job.command, Command::Append(_) | Command::CancelBuild) {
                        self.config = job.config;
                    }
                    let result = self.execute(
                        job.command,
                        job.now,
                        job.session,
                        job.analysis,
                        &analysis_tx,
                    );
                    // A key dialog may finish after Lock/Delete/config changes.
                    if job.session != self.session.load(Ordering::Acquire) {
                        self.vault = None;
                        continue;
                    }
                    match result {
                        Ok(status) => self.publish(job.session, status),
                        Err(error) => {
                            // A partial durability error, conflict or corruption is never
                            // retried automatically. Reload explicitly before any more writes.
                            if !matches!(
                                error,
                                Error::InsufficientSamples
                                    | Error::StaleProfile
                                    | Error::Analysis
                                    | Error::Busy
                            ) {
                                self.vault = None;
                            }
                            self.publish(job.session, Status::Failed(error));
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(reply) = result_rx.try_recv() {
                if reply.session != self.session.load(Ordering::Acquire)
                    || reply.analysis != self.analysis.load(Ordering::Acquire)
                    || self.active_analysis != Some(reply.analysis)
                {
                    continue;
                }
                self.active_analysis = None;
                let result = reply.result.and_then(|profile| {
                    if profile.source_policy != source_policy(&self.config) {
                        return Err(Error::StaleProfile);
                    }
                    self.vault
                        .as_mut()
                        .ok_or(Error::Locked)?
                        .transaction(|data| data.set_profile(profile))
                });
                match result {
                    Ok(()) => self.publish(reply.session, Status::Ready),
                    Err(error) => {
                        if !matches!(error, Error::Analysis | Error::StaleProfile | Error::Busy) {
                            self.vault = None;
                        }
                        self.publish(reply.session, Status::Failed(error));
                    }
                }
            }
        }
        self.vault = None;
    }
    fn execute(
        &mut self,
        command: Command,
        now: u64,
        session: u64,
        analysis: u64,
        sender: &mpsc::SyncSender<AnalysisJob>,
    ) -> Result<Status, Error> {
        match command {
            Command::Lock => {
                self.vault = None;
                return Ok(Status::Locked);
            }
            Command::Unlock => {
                self.vault = None;
                let create = match std::fs::symlink_metadata(&self.path) {
                    Ok(_) => false,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                    Err(_) => return Err(Error::Io),
                };
                let key = self.keys.load(create)?;
                if session != self.session.load(Ordering::Acquire) {
                    return Err(Error::Locked);
                }
                self.vault = Some(Vault::open(self.path.clone(), key)?);
                self.normalize(now)?;
            }
            Command::Sync => {
                if self.vault.is_none() {
                    return Ok(Status::Locked);
                }
                self.normalize(now)?;
            }
            Command::Append(records) => {
                self.normalize(now)?;
                let allowed: Vec<_> = records
                    .into_iter()
                    .filter(|r| self.config.history.allows_recording(&self.config, &r.scope))
                    .collect();
                let vault = self.vault.as_mut().ok_or(Error::Locked)?;
                if allowed.iter().any(|record| {
                    record.has_enough_writing()
                        || vault.data().records.iter().any(|old| old.id == record.id)
                }) {
                    let config = &self.config;
                    vault.transaction(|data| {
                        for record in allowed {
                            // Make room under the configured cap before adding a new ID.
                            if record.has_enough_writing()
                                && data.records.len() >= config.history.max_records
                                && !data.records.iter().any(|r| r.id == record.id)
                            {
                                data.records.sort_by_key(|r| r.updated_at);
                                data.records.remove(0);
                                data.invalidate_profile()?;
                            }
                            data.record_writing(record)?;
                        }
                        data.prune(&config.history, now)?;
                        Ok(())
                    })?;
                }
            }
            Command::Delete(selector) => {
                let vault = self.vault.as_mut().ok_or(Error::Locked)?;
                if matches!(selector, Selector::All) {
                    vault.clear_all()?;
                } else {
                    vault.transaction(|data| data.remove(&selector))?;
                }
                self.normalize(now)?;
            }
            Command::Build => {
                self.normalize(now)?;
                let request = BuildRequest::new(
                    self.vault.as_ref().ok_or(Error::Locked)?.data(),
                    &self.config,
                    analysis,
                    now,
                )?;
                sender
                    .try_send(AnalysisJob {
                        session,
                        analysis,
                        config: self.config.clone(),
                        request,
                    })
                    .map_err(|_| Error::Busy)?;
                self.active_analysis = Some(analysis);
                return Ok(Status::Building);
            }
            Command::CancelBuild => {
                // submit() already revoked the analysis epoch. No transaction,
                // source-policy update or profile replacement is needed here.
                if self.vault.is_none() {
                    return Ok(Status::Locked);
                }
            }
        }
        Ok(Status::Ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "libretypist-worker-{:x}",
                u128::from_be_bytes(super::super::new_id().unwrap())
            )))
        }
        fn path(&self) -> PathBuf {
            self.0.join("writing.lt")
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct Keys {
        calls: Arc<AtomicUsize>,
        error: Option<Error>,
        create: Arc<Mutex<Vec<bool>>>,
    }
    impl KeySource for Keys {
        fn load(&mut self, create: bool) -> Result<MasterKey, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.create.lock().unwrap().push(create);
            match self.error {
                Some(error) => Err(error),
                None => Ok(MasterKey::from_bytes([42; 32])),
            }
        }
    }
    struct SyntheticAnalyzer;
    impl Analyzer for SyntheticAnalyzer {
        fn generate(
            &mut self,
            config: &Config,
            request: &BuildRequest,
            _: &AtomicU64,
        ) -> Result<Profile, Error> {
            request.finish(config, r#"{"guidance":"Short, courteous sentences."}"#)
        }
    }
    fn keys(error: Option<Error>) -> Keys {
        Keys {
            calls: Arc::new(AtomicUsize::new(0)),
            error,
            create: Arc::new(Mutex::new(vec![])),
        }
    }
    fn wait(engine: &Engine, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let start = std::time::Instant::now();
        loop {
            let snapshot = engine.snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "history worker did not acknowledge the synthetic request: {:?}",
                snapshot.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn settled(engine: &Engine) -> Snapshot {
        wait(engine, |s| {
            s.last_job == engine.sequence
                && !matches!(
                    s.status,
                    Status::Building | Status::Unlocking | Status::Updating
                )
        })
    }
    fn samples() -> Vec<Record> {
        super::super::profile::tests::synthetic_data().records
    }

    #[test]
    fn worker_is_inert_until_unlock_and_recording_remains_a_separate_opt_in() {
        let dir = Temp::new();
        let keys = keys(None);
        let calls = keys.calls.clone();
        let mut engine = Engine::with_analyzer(dir.path(), keys, SyntheticAnalyzer).unwrap();
        let mut config = Config::default();
        engine.configure(&config, 30).unwrap();
        assert_eq!(settled(&engine).status, Status::Locked);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!dir.0.exists());
        assert_eq!(engine.append(&config, samples(), 30), Err(Error::Locked));
        engine.unlock(&config, 30).unwrap();
        assert!(settled(&engine).unlocked);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        engine.append(&config, samples(), 30).unwrap();
        assert_eq!(settled(&engine).records, 0);
        assert!(!dir.0.exists());
        config.history.recording = true;
        engine.configure(&config, 30).unwrap();
        settled(&engine);
        engine.append(&config, samples(), 30).unwrap();
        assert_eq!(settled(&engine).records, 3);
        assert!(dir.path().exists());
        config.enabled = false;
        config.history.recording = false;
        engine.configure(&config, 30).unwrap();
        settled(&engine);
        engine.build(&config, 30).unwrap();
        let snapshot = settled(&engine);
        assert!(snapshot.profile.is_some());
        assert!(snapshot
            .profile
            .unwrap()
            .instruction_for(&config, "com.apple.TextEdit", None)
            .is_none());
        config.enabled = true;
        config.history.recording = true;
        config
            .history
            .excluded_apps
            .push("org.libretypist.test".into());
        engine.configure(&config, 30).unwrap();
        assert!(engine.snapshot().profile.is_none());
        let snapshot = settled(&engine);
        assert_eq!(snapshot.records, 3);
        assert!(snapshot.profile.is_none());
        let mut stale_config = config.clone();
        stale_config.history.excluded_apps.clear();
        let mut stale_records = samples();
        stale_records[0].text = "This stale producer must not roll back the new exclusion.".into();
        engine.append(&stale_config, stale_records, 31).unwrap();
        assert_eq!(settled(&engine).revision, snapshot.revision);
        engine.lock().unwrap();
        assert!(!settled(&engine).unlocked);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn short_writing_never_reaches_disk_in_either_recording_mode() {
        for record_without_acceptance in [false, true] {
            let dir = Temp::new();
            let mut config = Config::default();
            config.history.recording = true;
            config.history.record_without_acceptance = record_without_acceptance;
            let mut engine =
                Engine::with_analyzer(dir.path(), keys(None), SyntheticAnalyzer).unwrap();
            engine.unlock(&config, 30).unwrap();
            settled(&engine);
            let mut short = samples().remove(0);
            short.text = "synthetic short query".into();
            engine.append(&config, vec![short.clone()], 30).unwrap();
            assert_eq!(settled(&engine).record_count(&Selector::All, 30), Some(0));
            assert!(!dir.path().exists());
            short.text =
                "A longer synthetic writing sample that contains enough characters to retain."
                    .into();
            engine.append(&config, vec![short.clone()], 31).unwrap();
            assert_eq!(settled(&engine).records, 1);
            short.text = "now short".into();
            short.updated_at = 32;
            engine.append(&config, vec![short], 32).unwrap();
            assert_eq!(settled(&engine).records, 0);
            engine.lock().unwrap();
            settled(&engine);
            let vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
            assert!(vault.data().records.is_empty());
        }
    }
    #[test]
    fn ignored_short_writing_cannot_evict_records_at_capacity() {
        let dir = Temp::new();
        let mut vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
        vault
            .transaction(|data| {
                for id in 1..=100 {
                    let mut item = samples().remove(0);
                    item.id = [id; 16];
                    data.upsert(item)?;
                }
                Ok(())
            })
            .unwrap();
        drop(vault);
        let before = std::fs::read(dir.path()).unwrap();
        let mut config = Config::default();
        config.history.recording = true;
        config.history.max_records = 100;
        let mut engine = Engine::with_analyzer(dir.path(), keys(None), SyntheticAnalyzer).unwrap();
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        let mut short = samples().remove(0);
        short.id = [101; 16];
        short.text = "small".into();
        engine.append(&config, vec![short], 30).unwrap();
        assert_eq!(settled(&engine).records, 100);
        assert_eq!(std::fs::read(dir.path()).unwrap(), before);
    }
    #[test]
    fn source_counts_aggregate_exact_domains_across_browsers_and_follow_deletion_retention_and_lock(
    ) {
        let dir = Temp::new();
        let mut config = Config::default();
        config.history.recording = true;
        config.history.retention_days = 1;
        let mut engine = Engine::with_analyzer(dir.path(), keys(None), SyntheticAnalyzer).unwrap();
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        let sources = [
            ("com.google.Chrome", Some("mail.example")),
            ("com.google.Chrome", Some("other.example")),
            ("com.apple.Safari", Some("mail.example")),
            ("com.google.Chrome", Some("sub.mail.example")),
            ("org.libretype.synthetic", None),
        ];
        let records: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, (app, site))| {
                let mut item = samples().remove(0);
                item.id = [i as u8 + 1; 16];
                item.scope = Scope {
                    app: (*app).into(),
                    website: site.map(str::to_owned),
                };
                item
            })
            .collect();
        engine.append(&config, records.clone(), 30).unwrap();
        let snapshot = settled(&engine);
        assert_eq!(snapshot.record_count(&Selector::All, 30), Some(5));
        assert_eq!(
            snapshot.record_count(&Selector::App("com.google.Chrome".into()), 30),
            Some(3)
        );
        assert_eq!(
            snapshot.record_count(&Selector::Website("mail.example".into()), 30),
            Some(2)
        );
        assert_eq!(
            snapshot.record_count(&Selector::Website("sub.mail.example".into()), 30),
            Some(1)
        );
        assert_eq!(
            snapshot.record_count(&Selector::Website("example".into()), 30),
            Some(0)
        );
        let mut edit = records[0].clone();
        edit.text.push_str(" A synthetic revision.");
        edit.updated_at = 31;
        engine.append(&config, vec![edit], 31).unwrap();
        assert_eq!(settled(&engine).record_count(&Selector::All, 31), Some(5));
        engine
            .delete(&config, Selector::Website("mail.example".into()), 32)
            .unwrap();
        let snapshot = settled(&engine);
        assert_eq!(snapshot.record_count(&Selector::All, 32), Some(3));
        assert_eq!(
            snapshot.record_count(&Selector::Website("mail.example".into()), 32),
            Some(0)
        );
        assert_eq!(
            snapshot.record_count(&Selector::App("com.google.Chrome".into()), 32),
            Some(2)
        );
        assert_eq!(snapshot.record_count(&Selector::All, 86421), None);
        engine.configure(&config, 86421).unwrap();
        assert_eq!(
            settled(&engine).record_count(&Selector::All, 86421),
            Some(0)
        );
        engine.lock().unwrap();
        assert_eq!(engine.snapshot().record_count(&Selector::All, 86421), None);
        assert!(settled(&engine).sources.is_empty());
    }
    #[test]
    fn unavailable_counts_are_not_reported_as_zero_or_stale_totals() {
        let mut snapshot = Snapshot {
            unlocked: true,
            status: Status::Ready,
            sources: vec![SourceCount {
                scope: Scope {
                    app: "org.libretype.synthetic".into(),
                    website: None,
                },
                records: 7,
            }],
            ..Snapshot::default()
        };
        assert_eq!(snapshot.record_count(&Selector::All, 30), Some(7));
        for status in [
            Status::Locked,
            Status::Unlocking,
            Status::Updating,
            Status::Failed(Error::Authentication),
        ] {
            snapshot.status = status;
            assert_eq!(snapshot.record_count(&Selector::All, 30), None);
        }
        snapshot.status = Status::Building;
        assert_eq!(snapshot.record_count(&Selector::All, 30), Some(7));
        snapshot.next_expiration = Some(31);
        assert_eq!(snapshot.record_count(&Selector::All, 31), None);
        assert_eq!(
            snapshot.record_count(&Selector::App(String::new()), 30),
            None
        );
    }
    #[test]
    fn existing_vault_never_authorizes_replacement_of_a_missing_key() {
        let dir = Temp::new();
        let mut vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
        vault
            .transaction(|data| {
                for item in samples() {
                    data.upsert(item)?;
                }
                Ok(())
            })
            .unwrap();
        drop(vault);
        let before = std::fs::read(dir.path()).unwrap();
        let keys = keys(Some(Error::MissingKey));
        let create = keys.create.clone();
        let mut engine = Engine::with_analyzer(dir.path(), keys, SyntheticAnalyzer).unwrap();
        engine.unlock(&Config::default(), 30).unwrap();
        assert_eq!(settled(&engine).status, Status::Failed(Error::MissingKey));
        assert_eq!(*create.lock().unwrap(), vec![false]);
        assert_eq!(std::fs::read(dir.path()).unwrap(), before);
    }

    struct GatedKeys {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl KeySource for GatedKeys {
        fn load(&mut self, _: bool) -> Result<MasterKey, Error> {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(MasterKey::from_bytes([42; 32]))
        }
    }
    #[test]
    fn lock_cancels_an_unlock_that_returns_after_a_prompt() {
        let dir = Temp::new();
        let (entered, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut engine = Engine::with_analyzer(
            dir.path(),
            GatedKeys {
                entered,
                release: resume,
            },
            SyntheticAnalyzer,
        )
        .unwrap();
        engine.unlock(&Config::default(), 30).unwrap();
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        engine.lock().unwrap();
        release.send(()).unwrap();
        assert_eq!(settled(&engine).status, Status::Locked);
        assert!(!engine.snapshot().unlocked);
        assert!(!dir.0.exists());
    }

    struct GatedAnalyzer {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Analyzer for GatedAnalyzer {
        fn generate(
            &mut self,
            config: &Config,
            request: &BuildRequest,
            _: &AtomicU64,
        ) -> Result<Profile, Error> {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            // Deliberately ignores cancellation, like a late OS/service reply.
            request.finish(config, r#"{"guidance":"Old deleted style."}"#)
        }
    }
    #[test]
    fn keyboard_activity_cancels_only_background_analysis_and_preserves_saved_profile() {
        let dir = Temp::new();
        let config = Config::default();
        let mut data = super::super::profile::tests::synthetic_data();
        let profile = BuildRequest::new(&data, &config, 1, 30)
            .unwrap()
            .finish(&config, r#"{"guidance":"Previously saved style."}"#)
            .unwrap();
        data.set_profile(profile).unwrap();
        let mut vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
        vault
            .transaction(|stored| {
                *stored = data;
                Ok(())
            })
            .unwrap();
        drop(vault);
        let saved = std::fs::read(dir.path()).unwrap();
        let (entered, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut engine = Engine::with_analyzer(
            dir.path(),
            keys(None),
            GatedAnalyzer {
                entered,
                release: resume,
            },
        )
        .unwrap();
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        engine.build_background(&config, 30).unwrap();
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        let background_epoch = engine.analysis.load(Ordering::Acquire);
        assert!(engine.interrupt_background(&config, 30).unwrap());
        assert_ne!(engine.analysis.load(Ordering::Acquire), background_epoch);
        // The analyzer is still gated: cancellation never waits on inference.
        let snapshot = settled(&engine);
        assert_eq!(snapshot.status, Status::Ready);
        assert_eq!(snapshot.records, 3);
        assert_eq!(
            snapshot.profile.unwrap().guidance,
            "Previously saved style."
        );
        assert_eq!(std::fs::read(dir.path()).unwrap(), saved);
        assert!(!engine.interrupt_background(&config, 30).unwrap());

        // A subsequent manual build must survive further keyboard activity.
        engine.build(&config, 30).unwrap();
        let manual_epoch = engine.analysis.load(Ordering::Acquire);
        assert!(!engine.interrupt_background(&config, 30).unwrap());
        assert_eq!(engine.analysis.load(Ordering::Acquire), manual_epoch);
        release.send(()).unwrap(); // Late, canceled background result.
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(std::fs::read(dir.path()).unwrap(), saved);
        assert_eq!(engine.snapshot().status, Status::Building);
        release.send(()).unwrap(); // Deliberately requested manual result.
        let snapshot = settled(&engine);
        assert_eq!(snapshot.status, Status::Ready);
        assert_eq!(snapshot.profile.unwrap().guidance, "Old deleted style.");
        engine.lock().unwrap();
        settled(&engine);
    }

    #[test]
    fn keyboard_activity_does_not_cancel_completed_or_replaced_background_jobs() {
        let dir = Temp::new();
        let mut config = Config::default();
        config.history.recording = true;
        let mut engine = Engine::with_analyzer(dir.path(), keys(None), SyntheticAnalyzer).unwrap();
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        engine.append(&config, samples(), 30).unwrap();
        settled(&engine);
        engine.build_background(&config, 30).unwrap();
        let completed = settled(&engine);
        assert!(completed.profile.is_some());
        let epoch = engine.analysis.load(Ordering::Acquire);
        assert!(!engine.interrupt_background(&config, 30).unwrap());
        assert_eq!(engine.analysis.load(Ordering::Acquire), epoch);
        assert_eq!(engine.snapshot().version, completed.version);
        // Lock revokes an automatic request even if inference was just queued.
        engine.build_background(&config, 30).unwrap();
        engine.lock().unwrap();
        assert!(!engine.interrupt_background(&config, 30).unwrap());
        assert_eq!(settled(&engine).status, Status::Locked);
    }

    #[test]
    fn deletion_finishes_during_slow_analysis_and_late_profiles_cannot_return() {
        let dir = Temp::new();
        let (entered, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut engine = Engine::with_analyzer(
            dir.path(),
            keys(None),
            GatedAnalyzer {
                entered,
                release: resume,
            },
        )
        .unwrap();
        let mut config = Config::default();
        config.history.recording = true;
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        engine.append(&config, samples(), 30).unwrap();
        settled(&engine);
        engine.build(&config, 30).unwrap();
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        engine.delete(&config, Selector::All, 30).unwrap();
        let snapshot = settled(&engine);
        assert_eq!(snapshot.records, 0);
        assert!(snapshot.profile.is_none());
        // Deletion has already committed even though inference is still blocked.
        let vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
        assert!(vault.data().records.is_empty());
        release.send(()).unwrap();
        engine.lock().unwrap();
        assert_eq!(settled(&engine).status, Status::Locked);
        engine.unlock(&config, 30).unwrap();
        let snapshot = settled(&engine);
        assert_eq!(snapshot.records, 0);
        assert!(snapshot.profile.is_none());
    }

    #[test]
    fn source_deletion_is_exact_and_corrupt_reload_never_resets_history() {
        let dir = Temp::new();
        let mut engine = Engine::with_analyzer(dir.path(), keys(None), SyntheticAnalyzer).unwrap();
        let mut config = Config::default();
        config.history.recording = true;
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        let mut records = samples();
        records[0].scope = Scope {
            app: "com.google.Chrome".into(),
            website: Some("mail.example".into()),
        };
        records[1].scope = Scope {
            app: "com.google.Chrome".into(),
            website: Some("other.example".into()),
        };
        engine.append(&config, records, 30).unwrap();
        settled(&engine);
        engine
            .delete(&config, Selector::Website("mail.example".into()), 30)
            .unwrap();
        let snapshot = settled(&engine);
        assert_eq!(snapshot.records, 2);
        assert!(snapshot
            .sources
            .iter()
            .all(|s| s.scope.website.as_deref() != Some("mail.example")));
        engine.lock().unwrap();
        settled(&engine);
        let mut corrupt = std::fs::read(dir.path()).unwrap();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        std::fs::write(dir.path(), &corrupt).unwrap();
        engine.unlock(&config, 30).unwrap();
        assert_eq!(
            settled(&engine).status,
            Status::Failed(Error::Authentication)
        );
        assert_eq!(std::fs::read(dir.path()).unwrap(), corrupt);
    }

    #[test]
    #[ignore = "synthetic writing, temporary encrypted vault and local model; never accesses Keychain"]
    fn worker_profile_live() {
        let dir = Temp::new();
        let mut engine = Engine::new(dir.path(), keys(None)).unwrap();
        let mut config = Config::default();
        if let Ok(model) = std::env::var("LIBRETYPE_TEST_MODEL") {
            config.model = model;
        }
        config.history.recording = true;
        engine.unlock(&config, 30).unwrap();
        settled(&engine);
        engine.append(&config, samples(), 30).unwrap();
        settled(&engine);
        // Exercise the production automatic scheduler with recording disabled.
        // This loads no application settings and never touches its real vault.
        config.history.recording = false;
        config.history.strength = super::super::Strength::Strong;
        config.history.auto_refresh = true;
        engine.configure(&config, 30).unwrap();
        let before = settled(&engine);
        let start = std::time::Instant::now();
        let mut schedule = super::super::refresh::Schedule::new(start);
        schedule.note_input(start + Duration::from_secs(4));
        assert!(!schedule.take_due(&config, &before, start + Duration::from_secs(5)));
        assert!(schedule.take_due(&config, &before, start + Duration::from_secs(9)));
        engine.build(&config, 30).unwrap();
        let start = std::time::Instant::now();
        loop {
            let state = engine.snapshot();
            if state.last_job == engine.sequence
                && !matches!(state.status, Status::Building | Status::Updating)
            {
                assert_eq!(state.status, Status::Ready);
                assert!(state.profile.is_some());
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(50));
        }
        engine.lock().unwrap();
        settled(&engine);
        let vault = Vault::open(dir.path(), MasterKey::from_bytes([42; 32])).unwrap();
        assert_eq!(vault.data().records.len(), 3);
        assert!(vault.data().profile.as_ref().unwrap().valid());
        let encrypted = std::fs::read(dir.path()).unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains("Thanks for the update"));

        // Reopen through the worker, then send the persisted profile through
        // the same completion generator used for external editor requests.
        engine.unlock(&config, 30).unwrap();
        let restored = settled(&engine);
        assert_eq!(restored.records, 3);
        let profile = restored.profile.expect("persisted profile must reload");
        let instruction = profile
            .instruction_for(&config, "org.libretypist.test", None)
            .expect("recording off must not disable style influence");
        assert!(instruction.contains(&profile.guidance));
        let request = crate::core::CompletionRequest {
            id: 1,
            prefix: "Thank you for your ".into(),
            suffix: String::new(),
            app_id: "org.libretypist.test".into(),
            website: None,
            background: Default::default(),
            profile: Some(profile),
        };
        let output = crate::model::generate(&config, &request, &AtomicU64::new(1)).unwrap();
        assert!(!output.trim().is_empty());
        assert!(!output.contains("guidance"));
        println!("Synthetic lifecycle passed: model={}, encrypted samples=3, idle refresh, reload, recording-off profile completion ({} characters)", config.model, output.chars().count());
        engine.lock().unwrap();
        assert!(settled(&engine).profile.is_none());
    }
}
