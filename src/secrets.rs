//! The keys and tokens in the user's settings, kept in Windows Credential Manager instead of config.json.
//!
//! A plain-text config file leaks through backups, synced folders, a file shared in a bug report, or an agent told to
//! `cat ~/.scoutcode/config.json`. Credential Manager encrypts them for this Windows user. It doesn't stop a program
//! already running as that user from asking for them; nothing a desktop app can do does.
//!
//! `config::read_saved` and `config::save_config` go through here for the fields in [`FIELDS`], so the rest of the app
//! sees them exactly as before. Where the store can't be used (another OS, a test, a value too big for it) the field
//! stays in the file, so nothing stops working.

use serde_json::Value;

/// The settings that are secrets. `additionalProviders` is a list of linked providers with their keys.
pub const FIELDS: [&str; 9] = ["apiKey", "openrouterKeys", "openrouterKey", "nvidiaKey", "kiloKey", "copilotToken", "grokKey", "hermesKey", "additionalProviders"];

pub fn is_secret(field: &str) -> bool {
    FIELDS.contains(&field)
}

/// Each settings folder gets its own entries, so a SCOUT_HOME used for testing or a portable copy never reads or
/// overwrites the installed app's keys.
fn target(field: &str) -> String {
    match std::env::var("SCOUT_HOME") {
        Ok(h) if !h.is_empty() => {
            use sha2::{Digest, Sha256};
            let d = Sha256::digest(h.as_bytes());
            let tag: String = d.iter().take(6).map(|b| format!("{b:02x}")).collect();
            format!("ScoutCode/{tag}/{field}")
        }
        _ => format!("ScoutCode/{field}"),
    }
}

/// Whether the store is used. Tests keep to files, so running them never touches the user's own credentials.
fn available() -> bool {
    cfg!(windows) && !cfg!(test)
}

pub fn get(field: &str) -> Option<Value> {
    if !available() {
        return None;
    }
    let text = os::read(&target(field))?;
    serde_json::from_str(&text).ok()
}

/// Stores `value`. False when it couldn't be, and the caller keeps it in the file instead.
pub fn set(field: &str, value: &Value) -> bool {
    if !available() {
        return false;
    }
    os::write(&target(field), &value.to_string())
}

pub fn delete(field: &str) {
    if available() {
        os::delete(&target(field));
    }
}

/// advapi32's Credential Manager calls, declared here rather than through the `windows` crate: they are three stable
/// functions, and the crate's Win32_Security_Credentials feature would mean rebuilding all of it.
#[cfg(windows)]
mod os {
    use std::ffi::c_void;

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    /// Credential Manager's limit on a generic credential's secret.
    const MAX_BLOB: usize = 5 * 512;

    /// CREDENTIALW, as in wincred.h.
    #[repr(C)]
    struct Credential {
        flags: u32,
        kind: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: [u32; 2],
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredReadW(target: *const u16, kind: u32, flags: u32, cred: *mut *mut Credential) -> i32;
        fn CredWriteW(cred: *const Credential, flags: u32) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *const c_void);
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn read(target: &str) -> Option<String> {
        let name = wide(target);
        let mut cred: *mut Credential = std::ptr::null_mut();
        unsafe {
            if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
                return None;
            }
            let c = &*cred;
            let bytes = if c.blob.is_null() { vec![] } else { std::slice::from_raw_parts(c.blob, c.blob_size as usize).to_vec() };
            CredFree(cred as *const c_void);
            String::from_utf8(bytes).ok()
        }
    }

    pub fn write(target: &str, secret: &str) -> bool {
        if secret.len() > MAX_BLOB {
            return false;
        }
        let mut name = wide(target);
        let mut user = wide("Scout");
        let mut comment = wide("Saved by the Scout desktop app");
        let mut blob = secret.as_bytes().to_vec();
        let cred = Credential {
            flags: 0,
            kind: CRED_TYPE_GENERIC,
            target_name: name.as_mut_ptr(),
            comment: comment.as_mut_ptr(),
            last_written: [0, 0],
            blob_size: blob.len() as u32,
            blob: blob.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: std::ptr::null_mut(),
            target_alias: std::ptr::null_mut(),
            user_name: user.as_mut_ptr(),
        };
        unsafe { CredWriteW(&cred, 0) != 0 }
    }

    pub fn delete(target: &str) {
        let name = wide(target);
        unsafe {
            CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
    }
}

#[cfg(not(windows))]
mod os {
    pub fn read(_: &str) -> Option<String> {
        None
    }
    pub fn write(_: &str, _: &str) -> bool {
        false
    }
    pub fn delete(_: &str) {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// The store itself, under a name of its own that the test removes again.
    #[test]
    fn credential_manager_round_trip() {
        let t = format!("ScoutCode/test-{}/apiKey", std::process::id());
        assert!(os::write(&t, r#""scout-test-value""#));
        assert_eq!(os::read(&t).as_deref(), Some(r#""scout-test-value""#));
        os::delete(&t);
        assert_eq!(os::read(&t), None);
        assert!(!os::write(&t, &"x".repeat(3000)), "too big for the store, so it stays in the file");
    }

    #[test]
    fn the_secret_fields_are_the_keys() {
        assert!(is_secret("apiKey") && is_secret("openrouterKeys") && is_secret("additionalProviders"));
        assert!(!is_secret("baseUrl") && !is_secret("mode") && !is_secret("username"));
    }
}
