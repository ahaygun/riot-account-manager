//! Encryption of everything sensitive we keep on disk.
//!
//! Two layers:
//!
//! 1. Always: Windows DPAPI (see `dpapi`). Files can only be opened by the
//!    same Windows user on the same machine.
//! 2. Optional app lock: a key derived from a master password with
//!    Argon2id, used with XChaCha20-Poly1305. With the lock on, a program
//!    running as the same Windows user still cannot read saved sessions or
//!    passwords without the master password.
//!
//! The key lives only in memory, and only while the app is unlocked.

use crate::dpapi;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use zeroize::Zeroizing;

pub type MasterKey = Zeroizing<[u8; 32]>;

const PLAIN: u8 = 0;
const LOCKED: u8 = 1;
const NONCE_LEN: usize = 24;
/// Encrypted with the key and stored beside the salt, so a password can be
/// checked without touching real data.
const CHECK: &[u8] = b"quickswitch-vault-check-v1";
pub const MIN_PASSWORD_LEN: usize = 8;

/// Argon2id cost. The default takes a fraction of a second on a desktop,
/// which is what makes guessing a stolen file slow. Tests use tiny values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub passes: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams { memory_kib: 64 * 1024, passes: 3 }
    }
}

/// `vault.json`: everything needed to turn the password into the key,
/// and nothing that reveals either.
#[derive(Serialize, Deserialize)]
pub struct VaultFile {
    salt: String,
    kdf: KdfParams,
    check: String,
}

#[derive(Debug)]
pub enum VaultError {
    /// The lock is on and the app has not been unlocked.
    Locked,
    Io(io::Error),
}

impl From<io::Error> for VaultError {
    fn from(e: io::Error) -> Self {
        VaultError::Io(e)
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> io::Result<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return Err(bad("corrupt vault file"));
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| bad("corrupt vault file"))).collect()
}

fn derive(password: &str, salt: &[u8], kdf: KdfParams) -> io::Result<MasterKey> {
    let params = argon2::Params::new(kdf.memory_kib, kdf.passes, 1, Some(32)).map_err(|e| bad(&e.to_string()))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon.hash_password_into(password.as_bytes(), salt, key.as_mut()).map_err(|e| bad(&e.to_string()))?;
    Ok(key)
}

fn encrypt(key: &MasterKey, plain: &[u8]) -> io::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
    // A fresh random nonce per file; 24 bytes makes a repeat implausible.
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, plain).map_err(|_| bad("encryption failed"))?);
    Ok(out)
}

fn decrypt(key: &MasterKey, data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < NONCE_LEN {
        return Err(bad("corrupt encrypted file"));
    }
    let (nonce, body) = data.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
    // Fails on a wrong key and on any modified byte.
    cipher.decrypt(XNonce::from_slice(nonce), body).map_err(|_| bad("wrong key or modified file"))
}

/// Makes a new lock for `password`: what to write to `vault.json`, and the key.
pub fn new_lock(password: &str, kdf: KdfParams) -> io::Result<(VaultFile, MasterKey)> {
    let salt: [u8; 16] = rand_bytes();
    let key = derive(password, &salt, kdf)?;
    let check = encrypt(&key, CHECK)?;
    Ok((VaultFile { salt: hex(&salt), kdf, check: hex(&check) }, key))
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    use chacha20poly1305::aead::rand_core::RngCore;
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

/// Seals `plain` for disk with an explicit key choice (used while changing
/// the lock, when old and new keys both exist).
pub fn seal_with(key: Option<&MasterKey>, plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut inner = Zeroizing::new(Vec::with_capacity(plain.len() + 64));
    match key {
        Some(key) => {
            inner.push(LOCKED);
            inner.extend(encrypt(key, plain)?);
        }
        None => {
            inner.push(PLAIN);
            inner.extend_from_slice(plain);
        }
    }
    dpapi::protect(&inner)
}

pub struct Vault {
    file: PathBuf,
    key: Mutex<Option<MasterKey>>,
}

impl Vault {
    pub fn new(file: PathBuf) -> Self {
        Vault { file, key: Mutex::new(None) }
    }

    fn key(&self) -> std::sync::MutexGuard<'_, Option<MasterKey>> {
        self.key.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The app lock is turned on.
    pub fn enabled(&self) -> bool {
        self.file.is_file()
    }

    /// Sensitive data can be read right now.
    pub fn unlocked(&self) -> bool {
        !self.enabled() || self.key().is_some()
    }

    fn load(&self) -> io::Result<VaultFile> {
        serde_json::from_slice(&std::fs::read(&self.file)?).map_err(|_| bad("corrupt vault file"))
    }

    /// Derives the key from `password` and checks it. `Ok(None)` means the
    /// password is wrong.
    pub fn check(&self, password: &str) -> io::Result<Option<MasterKey>> {
        let file = self.load()?;
        let key = derive(password, &unhex(&file.salt)?, file.kdf)?;
        Ok(decrypt(&key, &unhex(&file.check)?).is_ok_and(|c| c == CHECK).then_some(key))
    }

    pub fn unlock(&self, password: &str) -> io::Result<bool> {
        match self.check(password)? {
            Some(key) => {
                *self.key() = Some(key);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Forgets the key. Has no effect when the lock is off.
    pub fn lock(&self) {
        *self.key() = None;
    }

    /// Installs a new lock (or none). The caller has already re-sealed the
    /// data with the same key.
    pub fn install(&self, lock: Option<(VaultFile, MasterKey)>) -> io::Result<()> {
        match lock {
            Some((file, key)) => {
                let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
                crate::store::write_atomic(&self.file, &json)?;
                *self.key() = Some(key);
            }
            None => {
                match std::fs::remove_file(&self.file) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                *self.key() = None;
            }
        }
        Ok(())
    }

    /// Whether `blob` is sealed the way this vault seals right now: with
    /// the current key when the lock is on, without one when it is off.
    /// `None` when that cannot be told, because the lock is on and closed.
    pub fn sealed_for_now(&self, blob: &[u8]) -> Option<bool> {
        let Ok(inner) = dpapi::unprotect(blob) else { return Some(false) };
        let inner = Zeroizing::new(inner);
        match (inner.split_first(), self.enabled()) {
            (Some((&PLAIN, _)), enabled) => Some(!enabled),
            (Some((&LOCKED, _)), false) => Some(false),
            (Some((&LOCKED, rest)), true) => self.key().as_ref().map(|key| decrypt(key, rest).is_ok()),
            _ => Some(false),
        }
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        if !self.enabled() {
            return Ok(seal_with(None, plain)?);
        }
        match self.key().as_ref() {
            Some(key) => Ok(seal_with(Some(key), plain)?),
            None => Err(VaultError::Locked),
        }
    }

    pub fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        let inner = Zeroizing::new(dpapi::unprotect(blob)?);
        match inner.split_first() {
            Some((&PLAIN, rest)) => Ok(Zeroizing::new(rest.to_vec())),
            Some((&LOCKED, rest)) => match self.key().as_ref() {
                Some(key) => Ok(Zeroizing::new(decrypt(key, rest)?)),
                None => Err(VaultError::Locked),
            },
            _ => Err(bad("corrupt encrypted file").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = KdfParams { memory_kib: 64, passes: 1 };

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qsw-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("vault.json")
    }

    #[test]
    fn without_a_lock_data_round_trips() {
        let v = Vault::new(temp("off"));
        assert!(!v.enabled() && v.unlocked());
        let blob = v.seal(b"hello").unwrap();
        assert_eq!(&**v.open(&blob).unwrap(), b"hello");
    }

    #[test]
    fn locked_data_needs_the_password() {
        let v = Vault::new(temp("on"));
        v.install(Some(new_lock("correct horse", FAST).unwrap())).unwrap();
        assert!(v.enabled() && v.unlocked());
        let blob = v.seal(b"session-token").unwrap();
        assert!(!blob.windows(13).any(|w| w == b"session-token"), "not stored in the clear");

        v.lock();
        assert!(!v.unlocked());
        assert!(matches!(v.open(&blob), Err(VaultError::Locked)));
        assert!(matches!(v.seal(b"x"), Err(VaultError::Locked)));

        assert!(!v.unlock("wrong horse").unwrap());
        assert!(!v.unlocked());
        assert!(v.unlock("correct horse").unwrap());
        assert_eq!(&**v.open(&blob).unwrap(), b"session-token");
    }

    #[test]
    fn a_fresh_start_stays_locked_until_unlocked() {
        let file = temp("restart");
        let v = Vault::new(file.clone());
        v.install(Some(new_lock("correct horse", FAST).unwrap())).unwrap();
        let blob = v.seal(b"data").unwrap();
        // The app restarts: the key was only ever in memory.
        let again = Vault::new(file.clone());
        assert!(again.enabled() && !again.unlocked());
        assert!(matches!(again.open(&blob), Err(VaultError::Locked)));
        assert!(again.unlock("correct horse").unwrap());
        assert_eq!(&**again.open(&blob).unwrap(), b"data");
        // The vault file holds neither the password nor the key.
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("correct horse"));
    }

    #[test]
    fn modified_files_are_rejected() {
        let v = Vault::new(temp("tamper"));
        v.install(Some(new_lock("correct horse", FAST).unwrap())).unwrap();
        let mut blob = v.seal(b"data").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(matches!(v.open(&blob), Err(VaultError::Io(_))));
    }

    #[test]
    fn the_same_data_never_encrypts_the_same_way_twice() {
        let v = Vault::new(temp("nonce"));
        v.install(Some(new_lock("correct horse", FAST).unwrap())).unwrap();
        assert_ne!(v.seal(b"data").unwrap(), v.seal(b"data").unwrap());
    }

    #[test]
    fn removing_the_lock() {
        let v = Vault::new(temp("remove"));
        v.install(Some(new_lock("correct horse", FAST).unwrap())).unwrap();
        v.install(None).unwrap();
        assert!(!v.enabled() && v.unlocked());
    }
}
