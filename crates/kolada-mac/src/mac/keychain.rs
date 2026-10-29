//! Quick Send's keys in the login Keychain (`kolada_app::services::KeyStore`):
//! one generic password item, readable by this app on this Mac only, never
//! in a backup that leaves it (`AfterFirstUnlockThisDeviceOnly`). A test
//! instance keeps its own item. Security.framework is called directly, as
//! `hotkey.rs` calls Carbon.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSData, NSString};

use kolada_app::services::KeyStore;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: *const c_void;
    static kSecClassGenericPassword: *const c_void;
    static kSecAttrService: *const c_void;
    static kSecAttrAccount: *const c_void;
    static kSecAttrAccessible: *const c_void;
    static kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly: *const c_void;
    static kSecValueData: *const c_void;
    static kSecReturnData: *const c_void;
    static kSecMatchLimit: *const c_void;
    static kSecMatchLimitOne: *const c_void;
    fn SecItemCopyMatching(query: *const c_void, result: *mut *const c_void) -> i32;
    fn SecItemAdd(attributes: *const c_void, result: *mut *const c_void) -> i32;
    fn SecItemUpdate(query: *const c_void, attributes: *const c_void) -> i32;
    fn SecItemDelete(query: *const c_void) -> i32;
}

/// errSecItemNotFound.
const NOT_FOUND: i32 = -25300;

pub struct MacKeys;

fn service() -> &'static str {
    if kolada_core::testing() { "app.kolada.quick-send.test" } else { "app.kolada.quick-send" }
}

/// A Security constant as the object it is (a CFString, toll-free an NSString).
fn k(key: *const c_void) -> &'static AnyObject {
    unsafe { &*(key as *const AnyObject) }
}

/// A dictionary of `pairs`, toll-free a CFDictionary.
fn dict(pairs: &[(&AnyObject, &AnyObject)]) -> Retained<AnyObject> {
    unsafe {
        let d: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];
        for (key, value) in pairs {
            let _: () = msg_send![&*d, setObject: *value, forKey: *key];
        }
        d
    }
}

/// The item, as a query finds it.
fn item(extra: &[(&AnyObject, &AnyObject)]) -> Retained<AnyObject> {
    let service = NSString::from_str(service());
    let account = NSString::from_str("keys");
    let mut pairs: Vec<(&AnyObject, &AnyObject)> = vec![(k(unsafe { kSecClass }), k(unsafe { kSecClassGenericPassword })), (k(unsafe { kSecAttrService }), &service), (k(unsafe { kSecAttrAccount }), &account)];
    pairs.extend_from_slice(extra);
    dict(&pairs)
}

impl KeyStore for MacKeys {
    fn load(&self) -> Option<Vec<u8>> {
        let yes: Retained<AnyObject> = unsafe { msg_send![class!(NSNumber), numberWithBool: true] };
        let query = item(&[(k(unsafe { kSecReturnData }), &yes), (k(unsafe { kSecMatchLimit }), k(unsafe { kSecMatchLimitOne }))]);
        let mut out: *const c_void = std::ptr::null();
        let found = unsafe { SecItemCopyMatching(Retained::as_ptr(&query) as *const c_void, &mut out) };
        if found != 0 || out.is_null() {
            return None;
        }
        // A CFData handed over owned: toll-free an NSData.
        let data = unsafe { Retained::from_raw(out as *mut NSData) }?;
        Some(data.to_vec())
    }

    fn save(&self, keys: &[u8]) -> bool {
        let data = NSData::with_bytes(keys);
        let query = item(&[]);
        let change = dict(&[(k(unsafe { kSecValueData }), &data)]);
        let updated = unsafe { SecItemUpdate(Retained::as_ptr(&query) as *const c_void, Retained::as_ptr(&change) as *const c_void) };
        if updated == 0 {
            return true;
        }
        if updated != NOT_FOUND {
            unsafe { SecItemDelete(Retained::as_ptr(&query) as *const c_void) };
        }
        let new = item(&[(k(unsafe { kSecValueData }), &data), (k(unsafe { kSecAttrAccessible }), k(unsafe { kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly }))]);
        unsafe { SecItemAdd(Retained::as_ptr(&new) as *const c_void, std::ptr::null_mut()) == 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the test instance's item (never the app's) in the login
    /// Keychain, reads it back, replaces it, and removes it.
    #[test]
    #[ignore = "writes to the login Keychain: run by hand in a logged-in session"]
    fn keys_are_kept_read_back_and_replaced() {
        // SAFETY: set before any thread reads it; this test runs alone.
        unsafe { std::env::set_var("KOLADA_TEST_INSTANCE", "1") };
        assert_eq!(service(), "app.kolada.quick-send.test");
        assert!(MacKeys.save(b"first"));
        assert_eq!(MacKeys.load().as_deref(), Some(&b"first"[..]));
        assert!(MacKeys.save(b"second"));
        assert_eq!(MacKeys.load().as_deref(), Some(&b"second"[..]));
        unsafe { SecItemDelete(Retained::as_ptr(&item(&[])) as *const c_void) };
        assert_eq!(MacKeys.load(), None);
    }
}
