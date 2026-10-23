// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Dedicated local master-key item for the current ad-hoc macOS app.
//! Legacy Keychain access can show an OS prompt, so load is ONLY called by an
//! explicit Unlock action on the history worker. No startup/background lookup.
use super::{store::MasterKey, worker::KeySource, Error};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    string::CFString,
};
use std::ptr;

const SERVICE: &str = "org.libretypist.history.master-key";
const ACCOUNT: &str = "local-v1";
const NOT_FOUND: i32 = -25300;
const DUPLICATE: i32 = -25299;
#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecKeychainCopyDefault(result: *mut CFTypeRef) -> i32;
    fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> i32;
    fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> i32;
    static kSecClass: CFTypeRef;
    static kSecClassGenericPassword: CFTypeRef;
    static kSecAttrService: CFTypeRef;
    static kSecAttrAccount: CFTypeRef;
    static kSecAttrLabel: CFTypeRef;
    static kSecReturnData: CFTypeRef;
    static kSecMatchLimit: CFTypeRef;
    static kSecMatchLimitOne: CFTypeRef;
    static kSecMatchSearchList: CFTypeRef;
    static kSecUseKeychain: CFTypeRef;
    static kSecValueData: CFTypeRef;
}
unsafe fn constant(value: CFTypeRef) -> CFType {
    CFType::wrap_under_get_rule(value)
}
unsafe fn base_query() -> Vec<(CFType, CFType)> {
    vec![
        (constant(kSecClass), constant(kSecClassGenericPassword)),
        (
            constant(kSecAttrService),
            CFString::new(SERVICE).as_CFType(),
        ),
        (
            constant(kSecAttrAccount),
            CFString::new(ACCOUNT).as_CFType(),
        ),
    ]
}
unsafe fn lookup_query(keychain: &CFType) -> CFDictionary<CFType, CFType> {
    let mut pairs = base_query();
    pairs.extend([
        (
            constant(kSecMatchSearchList),
            CFArray::from_CFTypes(std::slice::from_ref(keychain)).as_CFType(),
        ),
        (
            constant(kSecReturnData),
            CFBoolean::true_value().as_CFType(),
        ),
        (constant(kSecMatchLimit), constant(kSecMatchLimitOne)),
    ]);
    CFDictionary::from_CFType_pairs(&pairs)
}
unsafe fn read(keychain: &CFType) -> Result<Option<MasterKey>, Error> {
    let query = lookup_query(keychain);
    let mut result = ptr::null();
    let status = SecItemCopyMatching(query.as_CFTypeRef(), &mut result);
    let owned = (!result.is_null()).then(|| CFType::wrap_under_create_rule(result));
    if status == NOT_FOUND {
        return Ok(None);
    }
    if status != 0 {
        return Err(Error::Keychain);
    }
    let data = owned
        .and_then(|value| value.downcast::<CFData>())
        .ok_or(Error::Keychain)?;
    if data.len() != 32 {
        return Err(Error::Keychain);
    }
    let mut bytes = zeroize::Zeroizing::new([0; 32]);
    bytes.copy_from_slice(data.bytes());
    Ok(Some(MasterKey::from_bytes(*bytes)))
}

// The caller supplies the exact keychain; neither reads nor writes may fall back
// to a global search. This also permits isolated native integration checks.
unsafe fn load_from(keychain: &CFType, create: bool) -> Result<MasterKey, Error> {
    if let Some(key) = read(keychain)? {
        return Ok(key);
    }
    if !create {
        return Err(Error::MissingKey);
    }
    let key = MasterKey::generate()?;
    let mut pairs = base_query();
    pairs.extend([
        (constant(kSecUseKeychain), keychain.clone()),
        (
            constant(kSecAttrLabel),
            CFString::new("LibreType encrypted writing history").as_CFType(),
        ),
        (
            constant(kSecValueData),
            CFData::from_buffer(key.as_bytes()).as_CFType(),
        ),
    ]);
    let attributes = CFDictionary::from_CFType_pairs(&pairs);
    match SecItemAdd(attributes.as_CFTypeRef(), ptr::null_mut()) {
        0 => Ok(key),
        DUPLICATE => read(keychain)?.ok_or(Error::Keychain),
        _ => Err(Error::Keychain),
    }
}

pub struct MacKeychain;
impl KeySource for MacKeychain {
    fn load(&mut self, create: bool) -> Result<MasterKey, Error> {
        unsafe {
            let mut keychain = ptr::null();
            let status = SecKeychainCopyDefault(&mut keychain);
            let owned = (!keychain.is_null()).then(|| CFType::wrap_under_create_rule(keychain));
            if status != 0 {
                return Err(Error::Keychain);
            }
            let keychain = owned.ok_or(Error::Keychain)?;
            load_from(&keychain, create)
        }
    }
}

#[cfg(test)]
#[path = "keychain_live.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_is_exact_and_restricted_to_the_selected_local_keychain() {
        // Build/inspect a dictionary only. Never call any Keychain API in this test.
        unsafe {
            let fixture = CFString::new("synthetic keychain reference").as_CFType();
            let query = lookup_query(&fixture);
            assert_eq!(query.len(), 6);
            assert_eq!(
                query
                    .get(constant(kSecAttrService))
                    .downcast::<CFString>()
                    .unwrap()
                    .to_string(),
                SERVICE
            );
            assert_eq!(
                query
                    .get(constant(kSecAttrAccount))
                    .downcast::<CFString>()
                    .unwrap()
                    .to_string(),
                ACCOUNT
            );
            assert_eq!(
                *query.get(constant(kSecMatchLimit)),
                constant(kSecMatchLimitOne)
            );
            let list = query
                .get(constant(kSecMatchSearchList))
                .downcast::<CFArray>()
                .unwrap();
            assert_eq!(list.len(), 1);
        }
    }
}
