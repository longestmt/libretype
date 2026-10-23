// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Local daily totals. The recorder accepts dates and counts, never editor text.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

const MAX_DAYS: usize = 365;
const MAX_FILE: u64 = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Io,
    Busy,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Statistics file is invalid; existing data was preserved.",
            Self::Io => "Statistics could not be saved; check the local storage and restart.",
            Self::Busy => "Statistics are in use by another process; restart after it closes.",
        })
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Totals {
    pub days: BTreeMap<String, u64>,
}
fn valid_day(day: &str) -> bool {
    let b = day.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = day[..4].parse().unwrap_or(0);
    let month: usize = day[5..7].parse().unwrap_or(0);
    let date: u32 = day[8..].parse().unwrap_or(0);
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=lengths[month - 1]).contains(&date)
}
impl Totals {
    fn validate(&self) -> Result<(), Error> {
        if self.days.len() > MAX_DAYS || self.days.iter().any(|(d, n)| !valid_day(d) || *n == 0) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn add(&mut self, counts: &BTreeMap<String, u64>) -> Result<(), Error> {
        if counts.keys().any(|d| !valid_day(d)) {
            return Err(Error::Invalid);
        }
        for (day, count) in counts {
            if *count > 0 {
                let total = self.days.entry(day.clone()).or_default();
                *total = total.checked_add(*count).ok_or(Error::Invalid)?;
            }
        }
        while self.days.len() > MAX_DAYS {
            self.days.pop_first();
        }
        self.validate()
    }
    pub fn count(&self, day: &str) -> u64 {
        self.days.get(day).copied().unwrap_or(0)
    }
}

/// Date at verification time in the Mac's current time zone. Store only this
/// date, never the time, offset, app identity, or accepted writing.
pub fn local_day() -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let now: libc::time_t = now.try_into().ok()?;
    let mut output = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        if libc::localtime_r(&now, output.as_mut_ptr()).is_null() {
            return None;
        }
        let date = output.assume_init();
        let day = format!(
            "{:04}-{:02}-{:02}",
            date.tm_year + 1900,
            date.tm_mon + 1,
            date.tm_mday
        );
        valid_day(&day).then_some(day)
    }
}

fn read(path: &Path) -> Result<Totals, Error> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Totals::default()),
        Err(_) => return Err(Error::Io),
    };
    let metadata = file.metadata().map_err(|_| Error::Io)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::Invalid);
    }
    let totals: Totals = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    totals.validate()?;
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "libretype-counts-{}-{nonce:x}-{id}",
                std::process::id()
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> PathBuf {
            self.0.join("statistics/daily.json")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn counts(day: &str, n: u64) -> BTreeMap<String, u64> {
        BTreeMap::from([(day.into(), n)])
    }
    #[test]
    fn daily_totals_validate_calendar_and_bound_retention_without_storing_text() {
        for day in ["2024-02-29", "2026-10-22", "2000-02-29"] {
            assert!(valid_day(day));
        }
        for day in [
            "2026-02-29",
            "2100-02-29",
            "2026-13-01",
            "2026-04-31",
            "2026-1-01",
            "hello",
            "2026-00-01",
        ] {
            assert!(!valid_day(day));
        }
        let mut totals = Totals::default();
        for year in 1970..=2335 {
            totals.add(&counts(&format!("{year}-01-01"), 2)).unwrap();
        }
        assert_eq!(totals.days.len(), 365);
        assert_eq!(totals.count("1970-01-01"), 0);
        assert_eq!(totals.count("1971-01-01"), 2);
        totals.add(&counts("2335-01-01", 5)).unwrap();
        assert_eq!(totals.count("2335-01-01"), 7);
        let encoded = serde_json::to_value(&totals).unwrap();
        assert_eq!(encoded.as_object().unwrap().len(), 1);
        assert!(encoded["days"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v.is_u64()));
        assert!(local_day().is_some_and(|day| valid_day(&day)));
    }
    #[test]
    fn worker_coalesces_counts_persists_on_finish_and_reloads_exact_totals() {
        let dir = Directory::new();
        let path = dir.path();
        let mut worker = Recorder::new(path.clone()).unwrap();
        assert!(worker.finish(Duration::from_secs(2)));
        assert!(!path.parent().unwrap().exists()); // Merely starting collects nothing.
        for _ in 0..500 {
            worker.record("2026-10-22".into(), 1);
            worker.record("2026-10-23".into(), 2);
        }
        assert!(worker.finish(Duration::from_secs(5)));
        assert_eq!(worker.snapshot().totals.count("2026-10-22"), 500);
        assert_eq!(read(&path).unwrap().count("2026-10-23"), 1000);
        let mut reloaded = Recorder::new(path.clone()).unwrap();
        assert!(reloaded.finish(Duration::from_secs(2)));
        assert_eq!(reloaded.snapshot().totals.count("2026-10-22"), 500);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
    #[test]
    fn corrupted_counts_and_overflow_are_preserved_and_never_reset() {
        let dir = Directory::new();
        let path = dir.path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"damaged").unwrap();
        let mut worker = Recorder::new(path.clone()).unwrap();
        assert!(!worker.finish(Duration::from_secs(2)));
        worker.record("2026-10-22".into(), 3);
        assert_eq!(worker.snapshot().error, Some(Error::Invalid));
        assert_eq!(fs::read(&path).unwrap(), b"damaged");
        let mut store = Store {
            path: path.clone(),
            lock: None,
        };
        let original = serde_json::to_vec(&Totals {
            days: counts("2026-10-22", u64::MAX),
        })
        .unwrap();
        fs::write(&path, &original).unwrap();
        assert!(matches!(
            store.update(&counts("2026-10-22", 1)),
            Err(Error::Invalid)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn competing_writers_cannot_overwrite_counts() {
        let dir = Directory::new();
        let path = dir.path();
        let mut first = Store {
            path: path.clone(),
            lock: None,
        };
        let mut second = Store {
            path: path.clone(),
            lock: None,
        };
        first.update(&counts("2026-10-22", 7)).unwrap();
        assert!(matches!(
            second.update(&counts("2026-10-22", 2)),
            Err(Error::Busy)
        ));
        assert_eq!(read(&path).unwrap().count("2026-10-22"), 7);
        drop(first);
        assert_eq!(
            second
                .update(&counts("2026-10-22", 2))
                .unwrap()
                .count("2026-10-22"),
            9
        );
    }
}

/// Hold one lock for the worker lifetime. A second app instance cannot overwrite
/// newer counts. No statistics directory is created until the first acceptance.
struct Store {
    path: PathBuf,
    lock: Option<File>,
}
impl Store {
    fn update(&mut self, counts: &BTreeMap<String, u64>) -> Result<Totals, Error> {
        let parent = self.path.parent().ok_or(Error::Io)?;
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(parent).map_err(|_| Error::Io)?;
        if self.lock.is_none() {
            let mut opts = OpenOptions::new();
            opts.read(true).write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let file = opts
                .open(self.path.with_extension("lock"))
                .map_err(|_| Error::Io)?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    return Err(Error::Busy);
                }
            }
            self.lock = Some(file);
        }
        let mut totals = read(&self.path)?;
        totals.add(counts)?;
        let bytes = serde_json::to_vec(&totals).map_err(|_| Error::Invalid)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Io)?
            .as_nanos();
        let temp = self
            .path
            .with_extension(format!("{}.{nonce:x}.tmp", std::process::id()));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp).map_err(|_| Error::Io)?;
        let result = (|| {
            file.write_all(&bytes).map_err(|_| Error::Io)?;
            file.sync_all().map_err(|_| Error::Io)?;
            fs::rename(&temp, &self.path).map_err(|_| Error::Io)?;
            File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|_| Error::Io)?;
            Ok(totals)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub totals: Totals,
    pub loaded: bool,
    pub error: Option<Error>,
    completed: u64,
}

struct Batch {
    sequence: u64,
    counts: BTreeMap<String, u64>,
}

pub struct Recorder {
    sender: mpsc::SyncSender<Batch>,
    shared: Arc<Mutex<Snapshot>>,
    pending: BTreeMap<String, u64>,
    submitted: u64,
}
impl Recorder {
    pub fn new(path: PathBuf) -> Result<Self, Error> {
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let worker_state = shared.clone();
        let (sender, receiver) = mpsc::sync_channel::<Batch>(1);
        std::thread::Builder::new()
            .name("LibreType statistics".into())
            .spawn(move || {
                let initial = read(&path);
                let mut failed = initial.is_err();
                {
                    let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.loaded = true;
                    match initial {
                        Ok(data) => state.totals = data,
                        Err(e) => state.error = Some(e),
                    }
                }
                let mut store = Store { path, lock: None };
                while let Ok(batch) = receiver.recv() {
                    if failed {
                        continue;
                    }
                    let result = store.update(&batch.counts);
                    let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                    match result {
                        Ok(data) => {
                            state.totals = data;
                            state.completed = batch.sequence;
                        }
                        Err(error) => {
                            state.error = Some(error);
                            failed = true;
                        }
                    }
                }
            })
            .map_err(|_| Error::Io)?;
        Ok(Self {
            sender,
            shared,
            pending: BTreeMap::new(),
            submitted: 0,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn record(&mut self, day: String, words: u64) {
        if words == 0 {
            return;
        }
        if !valid_day(&day) {
            self.fail(Error::Invalid);
            return;
        }
        let Some(total) = self
            .pending
            .get(&day)
            .copied()
            .unwrap_or(0)
            .checked_add(words)
        else {
            self.fail(Error::Invalid);
            return;
        };
        self.pending.insert(day, total);
        if self.pending.len() > MAX_DAYS {
            self.fail(Error::Busy);
            return;
        }
        self.poll();
    }
    fn fail(&mut self, error: Error) {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).error = Some(error);
        self.pending.clear();
    }
    /// One in-flight batch plus bounded, coalesced daily deltas. A slow disk never
    /// blocks the event loop, and a full queue keeps counts pending for next tick.
    pub fn poll(&mut self) {
        if self.snapshot().error.is_some() {
            self.pending.clear();
            return;
        }
        if self.pending.is_empty() {
            return;
        }
        let Some(sequence) = self.submitted.checked_add(1) else {
            self.fail(Error::Invalid);
            return;
        };
        match self.sender.try_send(Batch {
            sequence,
            counts: std::mem::take(&mut self.pending),
        }) {
            Ok(()) => self.submitted = sequence,
            Err(mpsc::TrySendError::Full(batch)) => self.pending = batch.counts,
            Err(mpsc::TrySendError::Disconnected(_)) => self.fail(Error::Io),
        }
    }
    /// Used only on ordinary app quit, never in the event tap or insertion path.
    pub fn finish(&mut self, timeout: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            self.poll();
            let state = self.snapshot();
            if state.error.is_some() {
                return false;
            }
            if state.loaded && self.pending.is_empty() && state.completed == self.submitted {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
