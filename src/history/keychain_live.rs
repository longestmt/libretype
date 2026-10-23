// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Explicit native integration check. Uses only a new, disposable file keychain.
//! Never queries password items in the user's login/default keychain.
use super::*;
use crate::history::{store::Vault, Record, Scope};
use core_foundation::base::CFEqual;
use std::{ffi::CString, fs, os::unix::fs::DirBuilderExt, path::PathBuf};
use zeroize::Zeroizing;

extern "C" {
    fn SecKeychainCreate(
        path: *const libc::c_char,
        length: u32,
        password: *const libc::c_void,
        prompt: u8,
        access: CFTypeRef,
        keychain: *mut CFTypeRef,
    ) -> i32;
    fn SecKeychainDelete(keychain: CFTypeRef) -> i32;
    fn SecKeychainOpen(path: *const libc::c_char, keychain: *mut CFTypeRef) -> i32;
    fn SecKeychainLock(keychain: CFTypeRef) -> i32;
    fn SecKeychainUnlock(
        keychain: CFTypeRef,
        length: u32,
        password: *const libc::c_void,
        use_password: u8,
    ) -> i32;
    fn SecKeychainCopySearchList(list: *mut CFTypeRef) -> i32;
    fn SecKeychainGetUserInteractionAllowed(allowed: *mut u8) -> i32;
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
    fn SecItemDelete(query: CFTypeRef) -> i32;
}

struct NoDialogs(u8);
impl NoDialogs {
    unsafe fn new() -> Self {
        let mut previous = 0;
        assert_eq!(SecKeychainGetUserInteractionAllowed(&mut previous), 0);
        assert_eq!(SecKeychainSetUserInteractionAllowed(0), 0);
        Self(previous)
    }
}
impl Drop for NoDialogs {
    fn drop(&mut self) {
        unsafe {
            let status = SecKeychainSetUserInteractionAllowed(self.0);
            if status != 0 {
                eprintln!("Could not restore test-process Keychain interaction flag: {status}");
            }
        }
    }
}

struct Fixture {
    directory: PathBuf,
    keychain: Option<CFType>,
    password: Zeroizing<Vec<u8>>,
}
impl Fixture {
    unsafe fn new() -> Self {
        let random = || u128::from_be_bytes(crate::history::new_id().unwrap());
        let directory = std::env::temp_dir().join(format!("libretype-keychain-{:032x}", random()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let mut fixture = Self {
            directory,
            keychain: None,
            password: Zeroizing::new(format!("{:032x}{:032x}", random(), random()).into_bytes()),
        };
        let mut reference = ptr::null();
        let status = SecKeychainCreate(
            fixture.path().as_ptr(),
            fixture.password.len() as u32,
            fixture.password.as_ptr().cast(),
            0,
            ptr::null(),
            &mut reference,
        );
        // Retain any returned object before asserting, so unwinding cleans it up.
        fixture.keychain =
            (!reference.is_null()).then(|| CFType::wrap_under_create_rule(reference));
        assert_eq!(status, 0, "create isolated test keychain");
        assert!(fixture.keychain.is_some());
        fixture
    }
    fn path(&self) -> CString {
        CString::new(
            self.directory
                .join("fixture.keychain")
                .as_os_str()
                .as_encoded_bytes(),
        )
        .unwrap()
    }
    fn keychain(&self) -> &CFType {
        self.keychain.as_ref().unwrap()
    }
    unsafe fn reopen(&self) -> CFType {
        let mut reference = ptr::null();
        let status = SecKeychainOpen(self.path().as_ptr(), &mut reference);
        let owned = (!reference.is_null()).then(|| CFType::wrap_under_create_rule(reference));
        assert_eq!(status, 0, "reopen isolated test keychain");
        owned.unwrap()
    }
    unsafe fn unlock(&self) {
        assert_eq!(
            SecKeychainUnlock(
                self.keychain().as_CFTypeRef(),
                self.password.len() as u32,
                self.password.as_ptr().cast(),
                1,
            ),
            0,
            "unlock only the isolated test keychain"
        );
    }
    unsafe fn cleanup(&mut self) -> i32 {
        let status = self
            .keychain
            .as_ref()
            .map_or(0, |k| SecKeychainDelete(k.as_CFTypeRef()));
        if status == 0 {
            self.keychain = None;
        }
        status
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            let status = self.cleanup();
            if status == 0 {
                let _ = fs::remove_dir_all(&self.directory);
            } else {
                eprintln!(
                    "Isolated Keychain cleanup failed ({status}): {}",
                    self.directory.display()
                );
            }
        }
    }
}

unsafe fn ambient_references() -> (CFType, CFType) {
    let mut default = ptr::null();
    assert_eq!(SecKeychainCopyDefault(&mut default), 0);
    let default = CFType::wrap_under_create_rule(default);
    let mut list = ptr::null();
    assert_eq!(SecKeychainCopySearchList(&mut list), 0);
    (default, CFType::wrap_under_create_rule(list))
}

fn same_key(a: &MasterKey, b: &MasterKey) {
    // Never include raw key bytes in assertion output, even on failure.
    assert!(
        a.as_bytes() == b.as_bytes(),
        "Keychain returned a different key"
    );
}

#[test]
#[ignore = "creates/deletes isolated native keychains; synthetic data only, dialogs disabled; run alone"]
fn isolated_keychain_roundtrip_live() {
    unsafe {
        let _dialogs = NoDialogs::new();
        let ambient_before = ambient_references();
        let mut fixture = Fixture::new();
        assert!(matches!(
            load_from(fixture.keychain(), false),
            Err(Error::MissingKey)
        ));
        let created = load_from(fixture.keychain(), true).unwrap();
        same_key(&created, &load_from(fixture.keychain(), false).unwrap());
        same_key(&created, &load_from(fixture.keychain(), true).unwrap());

        // A second keychain with the identical service/account must not collide
        // or satisfy an absent key through the ambient search list.
        let mut neighbor = Fixture::new();
        assert!(matches!(
            load_from(neighbor.keychain(), false),
            Err(Error::MissingKey)
        ));
        let other = load_from(neighbor.keychain(), true).unwrap();
        assert!(created.as_bytes() != other.as_bytes());
        assert_eq!(neighbor.cleanup(), 0);
        drop(neighbor);

        let vault_path = fixture.directory.join("vault/writing.lt");
        let mut vault = Vault::open(
            vault_path.clone(),
            load_from(fixture.keychain(), false).unwrap(),
        )
        .unwrap();
        vault
            .transaction(|data| {
                data.upsert(Record {
                    id: [1; 16],
                    scope: Scope {
                        app: "org.libretype.synthetic".into(),
                        website: None,
                    },
                    created_at: 1,
                    updated_at: 2,
                    text: "Synthetic Zephyr project note: meet Thursday at Cedar Hall.".into(),
                })
            })
            .unwrap();
        drop(vault);
        let encrypted = fs::read(&vault_path).unwrap();
        assert!(!encrypted
            .windows(b"Synthetic Zephyr".len())
            .any(|s| s == b"Synthetic Zephyr"));

        assert_eq!(SecKeychainLock(fixture.keychain().as_CFTypeRef()), 0);
        assert!(matches!(
            load_from(fixture.keychain(), false),
            Err(Error::Keychain)
        ));
        assert!(matches!(
            load_from(fixture.keychain(), true),
            Err(Error::Keychain)
        ));
        fixture.unlock();
        let reopened = fixture.reopen();
        let recovered = load_from(&reopened, false).unwrap();
        same_key(&created, &recovered);
        let vault = Vault::open(vault_path, recovered).unwrap();
        assert_eq!(vault.data().records.len(), 1);
        assert!(vault.data().records[0].text.contains("Cedar Hall"));
        drop(vault);

        // Malformed key material must be reported, never replaced with a fresh
        // key that would strand an existing vault. Delete only our fixture item.
        let mut pairs = base_query();
        pairs.push((
            constant(kSecMatchSearchList),
            CFArray::from_CFTypes(&[fixture.keychain().clone()]).as_CFType(),
        ));
        let query = CFDictionary::from_CFType_pairs(&pairs);
        assert_eq!(SecItemDelete(query.as_CFTypeRef()), 0);
        let mut pairs = base_query();
        pairs.push((constant(kSecUseKeychain), fixture.keychain().clone()));
        pairs.push((
            constant(kSecValueData),
            CFData::from_buffer(&[7; 31]).as_CFType(),
        ));
        assert_eq!(
            SecItemAdd(
                CFDictionary::from_CFType_pairs(&pairs).as_CFTypeRef(),
                ptr::null_mut()
            ),
            0
        );
        assert!(matches!(
            load_from(fixture.keychain(), false),
            Err(Error::Keychain)
        ));
        assert!(matches!(
            load_from(fixture.keychain(), true),
            Err(Error::Keychain)
        ));

        drop(reopened);
        assert_eq!(fixture.cleanup(), 0);
        let directory = fixture.directory.clone();
        drop(fixture);
        assert!(!directory.exists());
        let ambient_after = ambient_references();
        assert!(
            CFEqual(
                ambient_before.0.as_CFTypeRef(),
                ambient_after.0.as_CFTypeRef()
            ) != 0
        );
        assert!(
            CFEqual(
                ambient_before.1.as_CFTypeRef(),
                ambient_after.1.as_CFTypeRef()
            ) != 0
        );
    }
}
