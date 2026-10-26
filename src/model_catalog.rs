// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
//! One background model-list worker, with bounded retries and coalesced refreshes.
//! Never starts Ollama, downloads a model, or changes the user's selection.
use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

type Inventory = Result<Vec<String>, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Checking,
    Available,
    Unavailable,
    Stopped,
}

pub struct Discovery {
    requests: SyncSender<()>,
    replies: Receiver<Inventory>,
    status: Status,
    in_flight: bool,
    followup: bool,
    next_check: Instant,
    failures: u32,
}

impl Discovery {
    pub fn new() -> Result<Self, String> {
        Self::with_fetch(|| crate::model::list_models().map_err(|e| e.to_string()))
    }

    fn with_fetch(fetch: impl FnMut() -> Inventory + Send + 'static) -> Result<Self, String> {
        let (requests, work) = mpsc::sync_channel(1);
        let (results, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("libretype-model-list".into())
            .spawn(move || {
                let mut fetch = fetch;
                while work.recv().is_ok() {
                    if results.send(fetch()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|_| "Could not start the local model checker".to_owned())?;
        Ok(Self {
            requests,
            replies,
            status: Status::Checking,
            in_flight: false,
            followup: false,
            next_check: Instant::now(),
            failures: 0,
        })
    }

    /// A burst of refresh clicks during a request needs at most one fresh lookup.
    /// The follow-up matters when a download finishes after the lookup started.
    pub fn refresh(&mut self) {
        if self.in_flight {
            self.followup = true;
        } else {
            self.next_check = Instant::now();
        }
    }

    pub fn poll(&mut self, now: Instant) -> Option<Inventory> {
        if self.status == Status::Stopped {
            return None;
        }
        let update = match self.replies.try_recv() {
            Ok(result) => {
                self.in_flight = false;
                let delay = if result.is_ok() {
                    self.status = Status::Available;
                    self.failures = 0;
                    Duration::from_secs(60)
                } else {
                    self.status = Status::Unavailable;
                    self.failures = self.failures.saturating_add(1);
                    retry_delay(self.failures)
                };
                self.next_check = if self.followup { now } else { now + delay };
                self.followup = false;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => return self.stopped(),
        };
        if !self.in_flight && now >= self.next_check {
            if self.requests.try_send(()).is_err() {
                return self.stopped();
            }
            self.in_flight = true;
        }
        update
    }

    fn stopped(&mut self) -> Option<Inventory> {
        self.status = Status::Stopped;
        self.in_flight = false;
        Some(Err("Local model checker stopped; reopen LibreType.".into()))
    }

    /// Availability is independent of the last generation/settings action.
    /// Finding a tag is not proof that its model can generate a completion.
    pub fn availability_message(&self) -> Option<&'static str> {
        match self.status {
            Status::Checking => Some("Checking local Ollama…"),
            Status::Available => None,
            Status::Unavailable => {
                Some("Cannot read local models. Open Ollama; LibreType will retry.")
            }
            Status::Stopped => Some("Local model checker stopped; reopen LibreType."),
        }
    }

    pub fn health(&self, selected: &str, models: &[String]) -> Option<&'static str> {
        self.availability_message().or_else(|| {
            if models.is_empty() {
                Some("No local models installed. Open Local models…")
            } else if !models.iter().any(|model| model == selected) {
                Some("Selected model is unavailable. Open Local models…")
            } else {
                None
            }
        })
    }

    pub fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "state": match self.status {
                Status::Checking => "checking",
                Status::Available => "available",
                Status::Unavailable => "unavailable",
                Status::Stopped => "stopped",
            },
            "in_flight": self.in_flight,
            "consecutive_failures": self.failures,
        })
    }
}

fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1_u64 << failures.min(5)).min(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
    };

    fn accept(listener: &TcpListener) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "catalog request did not arrive");
                    std::thread::yield_now();
                }
                Err(error) => panic!("{error}"),
            }
        }
    }

    fn read_request(stream: &TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "GET /api/tags HTTP/1.1\r\n");
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty());
            if line == "\r\n" {
                break;
            }
        }
    }

    fn test_listener() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        (listener, base)
    }

    fn controlled() -> (Discovery, Receiver<()>, SyncSender<Inventory>) {
        let (started, starts) = mpsc::sync_channel(1);
        let (finish, finishes) = mpsc::sync_channel(1);
        let discovery = Discovery::with_fetch(move || {
            started.send(()).unwrap();
            finishes.recv().unwrap()
        })
        .unwrap();
        (discovery, starts, finish)
    }

    fn receive(discovery: &mut Discovery, now: Instant) -> Inventory {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = discovery.poll(now) {
                return result;
            }
            assert!(Instant::now() < deadline, "worker did not return");
            std::thread::yield_now();
        }
    }

    #[test]
    fn unavailable_start_recovers_and_healthy_inventory_refreshes() {
        let (mut discovery, starts, finish) = controlled();
        let mut now = Instant::now();
        assert!(discovery.poll(now).is_none());
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        finish.send(Err("unavailable".into())).unwrap();
        assert!(receive(&mut discovery, now).is_err());
        assert!(discovery.availability_message().unwrap().contains("retry"));
        assert!(discovery.poll(now + Duration::from_secs(1)).is_none());
        assert!(starts.try_recv().is_err());
        now += Duration::from_secs(2);
        discovery.poll(now);
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        finish.send(Ok(vec!["synthetic:local".into()])).unwrap();
        let models = receive(&mut discovery, now).unwrap();
        assert_eq!(discovery.health("synthetic:local", &models), None);
        assert!(discovery.health("missing:local", &models).is_some());
        assert!(discovery.poll(now + Duration::from_secs(59)).is_none());
        assert!(starts.try_recv().is_err());
        now += Duration::from_secs(60);
        discovery.poll(now);
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        finish.send(Ok(vec![])).unwrap();
        assert!(receive(&mut discovery, now).unwrap().is_empty());
        assert!(discovery
            .health("synthetic:local", &[])
            .unwrap()
            .contains("No local models"));
    }

    #[test]
    fn refresh_bursts_coalesce_without_overlapping_requests() {
        let (mut discovery, starts, finish) = controlled();
        let now = Instant::now();
        discovery.poll(now);
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..100 {
            discovery.refresh();
            assert!(discovery.poll(now).is_none());
        }
        assert!(starts.try_recv().is_err());
        finish.send(Ok(vec![])).unwrap();
        assert!(receive(&mut discovery, now).is_ok());
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        finish
            .send(Ok(vec!["newly-downloaded:local".into()]))
            .unwrap();
        assert_eq!(
            receive(&mut discovery, now).unwrap(),
            ["newly-downloaded:local"]
        );
        assert!(!discovery.in_flight);
        assert!(starts.try_recv().is_err());
    }

    #[test]
    fn failed_retries_back_off_reset_and_manual_refresh_bypasses_wait() {
        let (mut discovery, starts, finish) = controlled();
        let mut now = Instant::now();
        for seconds in [2, 4, 8, 16, 30, 30] {
            discovery.poll(now);
            starts.recv_timeout(Duration::from_secs(2)).unwrap();
            finish.send(Err("unavailable".into())).unwrap();
            assert!(receive(&mut discovery, now).is_err());
            assert_eq!(discovery.next_check - now, Duration::from_secs(seconds));
            now = discovery.next_check;
        }
        discovery.refresh();
        let now = Instant::now();
        discovery.poll(now);
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        finish.send(Ok(vec![])).unwrap();
        receive(&mut discovery, now).unwrap();
        assert_eq!(discovery.failures, 0);
        assert_eq!(discovery.next_check - now, Duration::from_secs(60));
    }

    #[test]
    fn failed_worker_reports_once_and_never_spins_or_leaks_identifiers() {
        let mut discovery = Discovery::with_fetch(|| panic!("test worker stopped")).unwrap();
        let now = Instant::now();
        discovery.poll(now);
        assert!(receive(&mut discovery, now).is_err());
        assert_eq!(discovery.status, Status::Stopped);
        discovery.refresh();
        assert!(discovery.poll(now + Duration::from_secs(120)).is_none());
        assert_eq!(discovery.diagnostic()["state"], "stopped");
        assert!(!discovery.in_flight);
    }

    #[test]
    fn local_http_recovery_filters_inventory_and_recovers_from_malformed_reply() {
        let (listener, base) = test_listener();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                (503, "starting"),
                (
                    200,
                    r#"{"models":[{"name":"z:local","details":{"format":"gguf"}},{"name":"a:local","details":{"format":"gguf"}},{"name":"a:local","details":{"format":"gguf"}},{"name":"remote","details":{"format":"gguf"},"remote_host":"https://example.invalid"},{"name":"example-cloud","details":{"format":"gguf"}},{"name":"wrong-format","details":{"format":"mlx"}}]}"#,
                ),
                (200, "not JSON"),
                (200, r#"{"models":[]}"#),
            ] {
                let mut stream = accept(&listener);
                read_request(&stream);
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let mut discovery = Discovery::with_fetch(move || {
            crate::model::list_models_at(&base, Duration::from_secs(1)).map_err(|e| e.to_string())
        })
        .unwrap();
        let mut now = Instant::now();
        discovery.poll(now);
        assert!(receive(&mut discovery, now).is_err());
        now = discovery.next_check;
        discovery.poll(now);
        assert_eq!(
            receive(&mut discovery, now).unwrap(),
            ["a:local", "z:local"]
        );
        assert_eq!(discovery.status, Status::Available);
        now = discovery.next_check;
        discovery.poll(now);
        assert!(receive(&mut discovery, now).is_err());
        assert_eq!(discovery.status, Status::Unavailable);
        now = discovery.next_check;
        discovery.poll(now);
        assert!(receive(&mut discovery, now).unwrap().is_empty());
        assert_eq!(discovery.status, Status::Available);
        server.join().unwrap();
    }

    #[test]
    fn catalog_response_size_and_whole_request_deadline_are_bounded() {
        let (listener, base) = test_listener();
        let server = std::thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&stream);
            let body = " ".repeat(1024 * 1024 + 2);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        });
        let error = crate::model::list_models_at(&base, Duration::from_secs(1)).unwrap_err();
        assert!(error.to_string().contains("size limit"));
        server.join().unwrap();

        let (listener, base) = test_listener();
        let (release, wait) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .unwrap();
            // Keep the connection open until the bounded client has returned.
            let _ = wait.recv_timeout(Duration::from_secs(2));
        });
        let began = Instant::now();
        let result = crate::model::list_models_at(&base, Duration::from_millis(100));
        let elapsed = began.elapsed();
        let _ = release.send(());
        server.join().unwrap();
        assert!(result.is_err());
        assert!(
            elapsed < Duration::from_secs(1),
            "whole-request deadline was not applied: {elapsed:?}"
        );
    }
}
