// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Cancellable HTTP for local inference. libcurl handles HTTP framing while this
//! worker drives socket readiness, checking the typing epoch every 25 ms.
use curl::{
    easy::{Easy2, Handler, List, ReadError, WriteError},
    multi::{Easy2Handle, Multi},
};
use std::{
    io::{self, Read},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

struct Body {
    upload: Zeroizing<Vec<u8>>,
    uploaded: usize,
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
    received: usize,
    headers: usize,
    limit: usize,
    exceeded: bool,
    activity: Instant,
}
impl Handler for Body {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, ReadError> {
        let count = output.len().min(self.upload.len() - self.uploaded);
        output[..count].copy_from_slice(&self.upload[self.uploaded..self.uploaded + count]);
        self.uploaded += count;
        self.activity = Instant::now();
        Ok(count)
    }
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.received = self.received.saturating_add(data.len());
        if self.received > self.limit {
            self.exceeded = true;
            return Ok(0);
        }
        self.bytes.extend_from_slice(data);
        self.activity = Instant::now();
        Ok(data.len())
    }
    fn header(&mut self, data: &[u8]) -> bool {
        self.headers = self.headers.saturating_add(data.len());
        self.activity = Instant::now();
        self.exceeded |= self.headers > 64 * 1024;
        !self.exceeded
    }
}

pub(crate) struct Response<'a> {
    multi: Multi,
    handle: Option<Easy2Handle<Body>>,
    current: &'a AtomicU64,
    id: u64,
    finished: bool,
    failed: bool,
}
fn interrupted() -> io::Error {
    // BufRead retries ErrorKind::Interrupted. A terminal cancellation must
    // escape read_line/read_to_end instead of spinning on the obsolete epoch.
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "Completion superseded by newer typing",
    )
}
fn unavailable(_: impl std::fmt::Debug) -> io::Error {
    // libcurl errors can include request URLs. Keep prompts and response data out.
    io::Error::other("Local Ollama request failed or timed out")
}
impl<'a> Response<'a> {
    pub(crate) fn post(
        base: &str,
        path: &str,
        payload: Zeroizing<Vec<u8>>,
        id: u64,
        current: &'a AtomicU64,
        limit: usize,
    ) -> io::Result<Self> {
        if current.load(Ordering::Acquire) != id {
            return Err(interrupted());
        }
        let parsed = url::Url::parse(base).map_err(unavailable)?;
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.path() != "/"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(path, "/api/show" | "/api/generate")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Inference requires the local Ollama endpoint",
            ));
        }
        let size = payload.len() as u64;
        let mut easy = Easy2::new(Body {
            upload: payload,
            uploaded: 0,
            bytes: Zeroizing::new(Vec::new()),
            offset: 0,
            received: 0,
            headers: 0,
            limit,
            exceeded: false,
            activity: Instant::now(),
        });
        easy.url(&format!("{}{path}", base.trim_end_matches('/')))
            .map_err(unavailable)?;
        easy.proxy("").map_err(unavailable)?;
        easy.follow_location(false).map_err(unavailable)?;
        easy.signal(false).map_err(unavailable)?;
        easy.connect_timeout(Duration::from_secs(2))
            .map_err(unavailable)?;
        easy.timeout(Duration::from_secs(120))
            .map_err(unavailable)?;
        easy.post(true).map_err(unavailable)?;
        easy.post_field_size(size).map_err(unavailable)?;
        easy.forbid_reuse(true).map_err(unavailable)?;
        let mut headers = List::new();
        headers
            .append("Content-Type: application/json")
            .map_err(unavailable)?;
        headers.append("Expect:").map_err(unavailable)?;
        easy.http_headers(headers).map_err(unavailable)?;
        let multi = Multi::new();
        let handle = multi.add2(easy).map_err(unavailable)?;
        Ok(Self {
            multi,
            handle: Some(handle),
            current,
            id,
            finished: false,
            failed: false,
        })
    }
}
impl Read for Response<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.current.load(Ordering::Acquire) != self.id {
                return Err(interrupted());
            }
            let handle = self.handle.as_mut().expect("live response");
            if handle.get_ref().exceeded {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Local Ollama response exceeded its size limit",
                ));
            }
            let status = handle.response_code().map_err(unavailable)?;
            if status == 101 || status > 200 {
                return Err(io::Error::other(format!(
                    "Local Ollama returned HTTP {status}"
                )));
            }
            if self.failed {
                return Err(unavailable(()));
            }
            let body = handle.get_mut();
            if body.offset < body.bytes.len() {
                let count = output.len().min(body.bytes.len() - body.offset);
                output[..count].copy_from_slice(&body.bytes[body.offset..body.offset + count]);
                body.offset += count;
                if body.offset == body.bytes.len() {
                    body.bytes.zeroize();
                    body.offset = 0;
                }
                return Ok(count);
            }
            if self.finished {
                return Ok(0);
            }
            if body.activity.elapsed() >= Duration::from_secs(30) {
                return Err(unavailable(()));
            }
            self.multi.perform().map_err(unavailable)?;
            self.multi.messages(|message| {
                if let Some(result) = message.result_for2(self.handle.as_ref().unwrap()) {
                    self.finished = true;
                    self.failed = result.is_err();
                }
            });
            // Inspect received data before waiting. With no sockets, wait() can
            // return immediately; avoid a busy loop while libcurl schedules work.
            if self.finished || !self.handle.as_ref().unwrap().get_ref().bytes.is_empty() {
                continue;
            }
            let start = Instant::now();
            let ready = self
                .multi
                .wait(&mut [], Duration::from_millis(25))
                .map_err(unavailable)?;
            if ready == 0 && start.elapsed() < Duration::from_millis(1) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Drop for Response<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            // Removing a live transfer cancels it. Dropping this private multi
            // session also closes any remaining connection cache.
            let _ = self.multi.remove2(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
    };

    pub(super) fn request(socket: &mut TcpStream) -> Vec<u8> {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut size = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty());
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    size = value.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0; size];
        reader.read_exact(&mut body).unwrap();
        body
    }
    #[test]
    fn obsolete_requests_close_silent_headers_and_partial_streams_promptly() {
        for prefix in [
            "",
            "HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n{\"response\":\"part",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let epoch = AtomicU64::new(1);
            let (ready, observed) = mpsc::channel();
            std::thread::scope(|scope| {
                let server = scope.spawn(|| {
                    let (mut socket, _) = listener.accept().unwrap();
                    assert_eq!(request(&mut socket), b"{}");
                    socket.write_all(prefix.as_bytes()).unwrap();
                    socket.flush().unwrap();
                    ready.send(()).unwrap();
                    let mut byte = [0];
                    let closed = socket.read(&mut byte);
                    assert!(
                        matches!(closed, Ok(0))
                            || closed.is_err_and(|e| e.kind() == io::ErrorKind::ConnectionReset),
                        "obsolete request must close its socket"
                    );
                });
                let worker = scope.spawn(|| {
                    let response = Response::post(
                        &base,
                        "/api/generate",
                        Zeroizing::new(b"{}".to_vec()),
                        1,
                        &epoch,
                        128 * 1024,
                    )
                    .unwrap();
                    let error = BufReader::new(response)
                        .read_line(&mut String::new())
                        .unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
                });
                observed.recv_timeout(Duration::from_secs(2)).unwrap();
                let start = Instant::now();
                epoch.store(2, Ordering::Release);
                worker.join().unwrap();
                server.join().unwrap();
                assert!(start.elapsed() < Duration::from_millis(500));
                println!(
                    "cancelled held response ({} header/body bytes) in {:.3} ms",
                    prefix.len(),
                    start.elapsed().as_secs_f64() * 1000.
                );
            });
        }
    }
    fn receive(reply: Vec<u8>, limit: usize) -> io::Result<Vec<u8>> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            request(&mut socket);
            let _ = socket.write_all(&reply);
        });
        let epoch = AtomicU64::new(1);
        let mut response = Response::post(
            &base,
            "/api/show",
            Zeroizing::new(b"{}".to_vec()),
            1,
            &epoch,
            limit,
        )
        .unwrap();
        let mut output = Vec::new();
        let result = response.read_to_end(&mut output).map(|_| output);
        server.join().unwrap();
        result
    }
    #[test]
    fn http_framing_preserves_unicode_and_rejects_truncation_and_oversized_data() {
        let body = "{\"response\":\"café 🦀\"}\n";
        let fixed = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(receive(fixed.into_bytes(), 1024).unwrap(), body.as_bytes());
        let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for byte in body.bytes() {
            chunked.extend_from_slice(&[b'1', b'\r', b'\n', byte, b'\r', b'\n']);
        }
        chunked.extend_from_slice(b"0\r\n\r\n");
        assert_eq!(receive(chunked.clone(), 1024).unwrap(), body.as_bytes());
        assert_eq!(
            receive(chunked, 5).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(receive(
            b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\n{}".to_vec(),
            1024
        )
        .is_err());
    }
    #[test]
    fn redirects_are_not_followed_and_nonlocal_targets_are_rejected() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let reply = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}/private\r\nContent-Length: 0\r\n\r\n",
            target.local_addr().unwrap()
        );
        assert!(receive(reply.into_bytes(), 1024)
            .unwrap_err()
            .to_string()
            .contains("HTTP 302"));
        assert_eq!(
            target.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let epoch = AtomicU64::new(1);
        for base in [
            "https://127.0.0.1:11434",
            "http://localhost:11434",
            "http://127.0.0.2:11434",
            "http://user@127.0.0.1:11434",
            "http://127.0.0.1:11434/extra",
        ] {
            assert!(
                Response::post(base, "/api/show", Zeroizing::new(vec![]), 1, &epoch, 1024).is_err()
            );
        }
        assert!(Response::post(
            "http://127.0.0.1:1",
            "/api/show",
            Zeroizing::new(vec![]),
            2,
            &epoch,
            1024
        )
        .is_err());
    }
}
