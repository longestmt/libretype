// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Per-user Unix socket between the optional Chrome host and the menu-bar app.
use super::{read_frame, write_frame, Cache, Reply, Request};
use std::os::unix::{
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub fn socket_path() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory unavailable"))?;
    Ok(PathBuf::from(home).join("Library/Caches/LibreTypist/BrowserBridge/docs.sock"))
}
fn owned_directory(path: &Path) -> io::Result<()> {
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid private bridge directory",
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}
pub struct Server {
    pub cache: Arc<Cache>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    path: PathBuf,
    inode: u64,
}
impl Server {
    pub fn bind(path: PathBuf) -> io::Result<Self> {
        owned_directory(
            path.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "No parent directory")
            })?,
        )?;
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Bridge endpoint is not an owned socket",
                ));
            }
            match UnixStream::connect(&path) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "Bridge is already running",
                    ))
                }
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(&path)?,
                Err(e) => return Err(e),
            }
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let inode = fs::metadata(&path)?.ino();
        listener.set_nonblocking(true)?;
        let cache = Arc::new(Cache::default());
        let worker_cache = cache.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                        let reply = match read_frame(&mut stream)
                            .ok()
                            .flatten()
                            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok())
                        {
                            Some(request) => worker_cache.receive(request),
                            None => Reply {
                                enabled: false,
                                accepted: false,
                            },
                        };
                        let _ = write_frame(&mut stream, &reply);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            cache,
            stop,
            worker: Some(worker),
            path,
            inode,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.file_type().is_socket() && m.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub fn exchange(path: &Path, request: &Request) -> io::Result<Reply> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid bridge endpoint",
        ));
    }
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_millis(300)))?;
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    write_frame(&mut stream, request)?;
    let bytes = read_frame(&mut stream)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "Missing bridge reply"))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid bridge reply"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_lifecycle_is_private_optional_and_does_not_replace_live_or_unrelated_files() {
        let root = std::env::temp_dir().join(format!(
            "lt-docs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("bridge.sock");
        let server = Server::bind(path.clone()).unwrap();
        assert!(!exchange(&path, &Request::Ping {}).unwrap().enabled);
        assert!(Server::bind(path.clone()).is_err());
        server.cache.set_enabled(true);
        assert!(exchange(&path, &Request::Ping {}).unwrap().enabled);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        drop(server);
        assert!(!path.exists());
        fs::write(&path, b"owned unrelated file").unwrap();
        assert!(Server::bind(path.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"owned unrelated file");
        fs::remove_dir_all(root).unwrap();
    }
}
