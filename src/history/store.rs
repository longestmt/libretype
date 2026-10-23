// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Authenticated encrypted snapshots. Fresh per-save keys are derived from a
//! random 256-bit salt and a master key intended for a separate native Keychain adapter.
use super::{Data, Error};
use ring::{
    aead, hkdf,
    rand::{SecureRandom, SystemRandom},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
const MAGIC: &[u8; 8] = b"LTHIST01";
const HEADER: usize = 40;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const INFO: &[u8] = b"LibreTypist encrypted history snapshot v1";

pub struct MasterKey(Zeroizing<[u8; 32]>);
impl MasterKey {
    pub fn generate() -> Result<Self, Error> {
        let mut key = Zeroizing::new([0; 32]);
        SystemRandom::new()
            .fill(key.as_mut())
            .map_err(|_| Error::Random)?;
        Ok(Self(key))
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    /// Intended solely for the Keychain adapter; never serialize this key into a file.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
struct KeyLength;
impl hkdf::KeyType for KeyLength {
    fn len(&self) -> usize {
        32
    }
}
fn derive(master: &MasterKey, salt: &[u8]) -> Result<aead::LessSafeKey, Error> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, salt);
    let prk = salt.extract(master.as_bytes());
    let mut key = Zeroizing::new([0; 32]);
    prk.expand(&[INFO], KeyLength)
        .map_err(|_| Error::InvalidData)?
        .fill(key.as_mut())
        .map_err(|_| Error::InvalidData)?;
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key.as_ref()).map_err(|_| Error::InvalidData)?,
    ))
}
pub fn encrypt(data: &Data, master: &MasterKey) -> Result<Vec<u8>, Error> {
    data.validate()?;
    let mut body = Zeroizing::new(serde_json::to_vec(data).map_err(|_| Error::InvalidData)?);
    if body.len() as u64 > MAX_FILE - HEADER as u64 - 16 {
        return Err(Error::Capacity);
    }
    let mut envelope = vec![0u8; HEADER];
    envelope[..8].copy_from_slice(MAGIC);
    SystemRandom::new()
        .fill(&mut envelope[8..])
        .map_err(|_| Error::Random)?;
    let key = derive(master, &envelope[8..])?;
    // Each random salt derives a new key; this fixed nonce is used once per derived key.
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key([0; 12]),
        aead::Aad::from(envelope.as_slice()),
        &mut *body,
    )
    .map_err(|_| Error::InvalidData)?;
    envelope.extend_from_slice(&body);
    Ok(envelope)
}
pub fn decrypt(envelope: &[u8], master: &MasterKey) -> Result<Data, Error> {
    if envelope.len() < HEADER + 16 || envelope.len() as u64 > MAX_FILE || &envelope[..8] != MAGIC {
        return Err(Error::InvalidFile);
    }
    let key = derive(master, &envelope[8..HEADER])?;
    let mut body = Zeroizing::new(envelope[HEADER..].to_vec());
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key([0; 12]),
            aead::Aad::from(&envelope[..HEADER]),
            &mut body,
        )
        .map_err(|_| Error::Authentication)?;
    let data: Data = serde_json::from_slice(plain).map_err(|_| Error::InvalidData)?;
    data.validate()?;
    Ok(data)
}
fn check_file(path: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_FILE {
        return Err(Error::InvalidFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Permissions);
        }
    }
    Ok(())
}
fn fingerprint(bytes: &[u8]) -> [u8; 32] {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .try_into()
        .expect("SHA-256 length")
}
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::Io),
        Ok(_) => {}
    }
    check_file(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| Error::Io)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::InvalidFile);
    }
    Ok(Some(bytes))
}
struct FileLock(File);
impl FileLock {
    fn acquire(parent: &Path) -> Result<Self, Error> {
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            let file = options
                .open(parent.join(".history.lock"))
                .map_err(|_| Error::Io)?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(Error::Busy);
            }
            Ok(Self(file))
        }
        #[cfg(not(unix))]
        {
            let _ = options;
            let _ = parent;
            Err(Error::Permissions)
        }
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}
struct Commit {
    fingerprint: [u8; 32],
    durable: bool,
}
fn write(
    path: &Path,
    key: &MasterKey,
    data: &Data,
    expected: Option<[u8; 32]>,
) -> Result<Commit, Error> {
    let bytes = encrypt(data, key)?;
    let parent = path.parent().ok_or(Error::Io)?;
    if let Ok(metadata) = fs::symlink_metadata(parent) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Permissions);
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent).map_err(|_| Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| Error::Permissions)?;
    }
    let _lock = FileLock::acquire(parent)?;
    let current = read_bytes(path)?;
    if current.as_ref().map(|bytes| fingerprint(bytes)) != expected {
        return Err(Error::Conflict);
    }
    let id = super::new_id()?;
    let suffix = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let temp = parent.join(format!(".history-{suffix}.tmp"));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|_| Error::Io)?;
    let result = (|| {
        file.write_all(&bytes).map_err(|_| Error::Io)?;
        file.sync_all().map_err(|_| Error::Io)?;
        fs::rename(&temp, path).map_err(|_| Error::Io)?;
        // Once rename succeeds the visible state has changed. Report durability
        // separately so the caller cannot accidentally keep a stale in-memory view.
        Ok(Commit {
            fingerprint: fingerprint(&bytes),
            durable: File::open(parent).and_then(|dir| dir.sync_all()).is_ok(),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
/// Mutations are committed to an authenticated snapshot before replacing the
/// in-memory view. Opening a corrupt file fails; it never creates a replacement.
pub struct Vault {
    path: PathBuf,
    key: MasterKey,
    data: Data,
    expected: Option<[u8; 32]>,
}
impl Vault {
    pub fn open(path: PathBuf, key: MasterKey) -> Result<Self, Error> {
        let bytes = read_bytes(&path)?;
        let data = match &bytes {
            Some(bytes) => decrypt(bytes, &key)?,
            None => Data::default(),
        };
        let expected = bytes.as_ref().map(|bytes| fingerprint(bytes));
        Ok(Self {
            path,
            key,
            data,
            expected,
        })
    }
    pub fn data(&self) -> &Data {
        &self.data
    }
    /// Clear records/profile and remove this store's encrypted crash leftovers.
    /// The key is retained for future opt-in recording. This is not forensic erasure.
    pub fn clear_all(&mut self) -> Result<(), Error> {
        self.transaction(|data| data.remove(&super::Selector::All))?;
        let parent = self.path.parent().ok_or(Error::Io)?;
        let _lock = FileLock::acquire(parent)?;
        if read_bytes(&self.path)?
            .as_ref()
            .map(|bytes| fingerprint(bytes))
            != self.expected
        {
            return Err(Error::Conflict);
        }
        for entry in fs::read_dir(parent).map_err(|_| Error::Io)? {
            let entry = entry.map_err(|_| Error::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(suffix) = name
                .strip_prefix(".history-")
                .and_then(|s| s.strip_suffix(".tmp"))
            else {
                continue;
            };
            if suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                fs::remove_file(entry.path()).map_err(|_| Error::Io)?;
            }
        }
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Error::Durability)
    }
    pub fn transaction<T>(
        &mut self,
        change: impl FnOnce(&mut Data) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut next = self.data.clone();
        let result = change(&mut next)?;
        next.validate()?;
        let commit = write(&self.path, &self.key, &next, self.expected)?;
        self.data = next;
        self.expected = Some(commit.fingerprint);
        if !commit.durable {
            return Err(Error::Durability);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Profile, Record, Scope, Selector};
    #[test]
    fn clear_all_removes_owned_crash_snapshots_but_preserves_unrelated_files() {
        let directory = std::env::temp_dir().join(format!(
            "libretypist-clear-{:x}",
            u128::from_be_bytes(crate::history::new_id().unwrap())
        ));
        let path = directory.join("writing.lt");
        let mut vault = Vault::open(path.clone(), MasterKey::from_bytes([8; 32])).unwrap();
        vault
            .transaction(|data| {
                *data = fixture();
                Ok(())
            })
            .unwrap();
        let orphan = directory.join(".history-1234567890abcdef1234567890abcdef.tmp");
        fs::copy(&path, &orphan).unwrap();
        let unrelated = directory.join("keep.txt");
        fs::write(&unrelated, b"unrelated fixture").unwrap();
        let malformed = directory.join(".history-unrelated.tmp");
        fs::write(&malformed, b"unrelated fixture").unwrap();
        vault.clear_all().unwrap();
        assert!(!orphan.exists());
        assert!(unrelated.exists());
        assert!(malformed.exists());
        let reopened = Vault::open(path, MasterKey::from_bytes([8; 32])).unwrap();
        assert!(reopened.data().records.is_empty());
        assert!(reopened.data().profile.is_none());
        drop(reopened);
        drop(vault);
        fs::remove_dir_all(directory).unwrap();
    }
    fn fixture() -> Data {
        let mut data = Data::default();
        data.upsert(Record {
            id: [1; 16],
            scope: Scope {
                app: "private.app".into(),
                website: Some("private.example".into()),
            },
            created_at: 1,
            updated_at: 2,
            text: "Synthetic confidential sample 🌻".into(),
        })
        .unwrap();
        data.set_profile(Profile {
            revision: data.revision,
            created_at: 3,
            guidance: "Synthetic personal style.".into(),
            vocabulary: vec!["Synthetic terminology".into()],
            source_policy: crate::history::profile::source_policy(&crate::config::Config::default()),
        })
        .unwrap();
        data
    }
    #[test]
    fn stale_writers_and_failed_edits_cannot_resurrect_deleted_data() {
        let dir = std::env::temp_dir().join(format!(
            "libretypist-conflict-test-{:x}",
            u128::from_be_bytes(crate::history::new_id().unwrap())
        ));
        let path = dir.join("writing.lt");
        let key = [9; 32];
        let mut first = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        first
            .transaction(|data| {
                *data = fixture();
                Ok(())
            })
            .unwrap();
        let mut stale = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        first
            .transaction(|data| data.remove(&Selector::All))
            .unwrap();
        assert_eq!(
            stale
                .transaction(|data| {
                    data.profile = None;
                    Ok(())
                })
                .err(),
            Some(Error::Conflict)
        );
        let mut current = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        assert!(current.data().records.is_empty());
        assert_eq!(
            current
                .transaction::<()>(|data| {
                    *data = fixture();
                    Err(Error::InvalidData)
                })
                .err(),
            Some(Error::InvalidData)
        );
        assert!(current.data().records.is_empty());
        let held = FileLock::acquire(&dir).unwrap();
        assert_eq!(current.transaction(|_| Ok(())).err(), Some(Error::Busy));
        drop(held);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authenticated_round_trip_hides_text_scope_and_profile() {
        let key = MasterKey::generate().unwrap();
        let data = fixture();
        let bytes = encrypt(&data, &key).unwrap();
        let second = encrypt(&data, &key).unwrap();
        assert_ne!(bytes, second);
        for secret in [
            "confidential",
            "private.example",
            "private.app",
            "personal style",
            "Synthetic terminology",
        ] {
            assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        }
        let recovered = decrypt(&bytes, &key).unwrap();
        assert_eq!(recovered.records[0].text, data.records[0].text);
        assert_eq!(
            recovered.profile.as_ref().unwrap().vocabulary,
            ["Synthetic terminology"]
        );
        assert_eq!(
            recovered.profile.unwrap().guidance,
            "Synthetic personal style."
        );
        assert_eq!(
            decrypt(&bytes, &MasterKey::generate().unwrap()).err(),
            Some(Error::Authentication)
        );
        for at in [8, HEADER, bytes.len() - 1] {
            let mut damaged = bytes.clone();
            damaged[at] ^= 1;
            assert_eq!(decrypt(&damaged, &key).err(), Some(Error::Authentication));
        }
        assert_eq!(decrypt(&bytes[..20], &key).err(), Some(Error::InvalidFile));
    }
    #[test]
    fn private_atomic_vault_reloads_deletes_and_refuses_corruption() {
        let dir = std::env::temp_dir().join(format!(
            "libretypist-history-test-{:x}",
            u128::from_be_bytes(crate::history::new_id().unwrap())
        ));
        let path = dir.join("writing.lt");
        let key = [7; 32];
        let mut vault = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        assert!(!path.exists());
        vault
            .transaction(|data| {
                *data = fixture();
                Ok(())
            })
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let mut reopened = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        assert_eq!(reopened.data().records.len(), 1);
        reopened
            .transaction(|data| data.remove(&Selector::All))
            .unwrap();
        let empty = Vault::open(path.clone(), MasterKey::from_bytes(key)).unwrap();
        assert!(empty.data().records.is_empty() && empty.data().profile.is_none());
        let mut corrupted = fs::read(&path).unwrap();
        *corrupted.last_mut().unwrap() ^= 1;
        fs::write(&path, &corrupted).unwrap();
        assert!(Vault::open(path.clone(), MasterKey::from_bytes(key)).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupted);
        assert_eq!(
            reopened.transaction(|_| Ok(())).err(),
            Some(Error::Conflict)
        );
        assert_eq!(fs::read(&path).unwrap(), corrupted);
        fs::remove_dir_all(dir).unwrap();
    }
}
