//! Encrypted password vault (`vault.bin` in the config dir), protected by the master
//! password.
//!
//! File layout (all integers little endian):
//!
//! ```text
//! "RSCPVLT1"  magic (8)
//! u8          format version (1)
//! u8          algorithm (1 = Argon2id v19 + XChaCha20-Poly1305)
//! u32 u32 u32 Argon2 memory (KiB), passes, lanes
//! [16]        salt (random, new for every master password)
//! [24] [16]   check nonce + tag: AEAD of an empty message, tells a wrong password
//!             apart from a damaged file
//! [24]        nonce of the payload (new for every write)
//! [..]        AEAD(payload), associated data = everything above
//! ```
//!
//! The master password and the derived key are never written anywhere. The key is
//! kept in memory (in a [`SecretBox`]) only while the vault is unlocked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rolid_text::{txt, Text};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretString};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"RSCPVLT1";
const FORMAT: u8 = 1;
const ALG: u8 = 1;
const SALT: usize = 16;
const NONCE: usize = 24;
const TAG: usize = 16;
/// Header up to (and including) the salt.
const PREFIX: usize = 8 + 1 + 1 + 12 + SALT;
/// Whole header, i.e. associated data of the payload.
const HEADER: usize = PREFIX + NONCE + TAG + NONCE;

/// Argon2id parameters, stored in the file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// Minimum: 19 MiB of memory, 2 passes, 1 lane (OWASP recommendation).
    pub const MIN: KdfParams = KdfParams { m_kib: 19 * 1024, t: 2, p: 1 };
    /// Upper bounds accepted when reading a file.
    const MAX_M_KIB: u32 = 2 * 1024 * 1024;
    const MAX_T: u32 = 64;
    const MAX_P: u32 = 16;
    /// Memory used at most by [`KdfParams::calibrate`].
    const CALIBRATE_MAX_M_KIB: u32 = 256 * 1024;

    /// The minimum, or more memory if deriving the key stays within about `target`
    /// (0.5 s) on this machine.
    pub fn calibrate(target: Duration) -> KdfParams {
        let base = KdfParams::MIN;
        let t0 = Instant::now();
        let mut key = Zeroizing::new([0u8; 32]);
        if derive_into(b"calibration", &[0u8; SALT], base, &mut key).is_err() {
            return base;
        }
        let took = t0.elapsed().max(Duration::from_millis(1));
        let factor = (target.as_secs_f64() / took.as_secs_f64()).floor();
        if factor < 2.0 {
            return base;
        }
        let m = (base.m_kib as f64 * factor).min(Self::CALIBRATE_MAX_M_KIB as f64) as u32;
        KdfParams { m_kib: m / 1024 * 1024, ..base }
    }

    fn valid(&self) -> bool {
        (Self::MIN.m_kib..=Self::MAX_M_KIB).contains(&self.m_kib)
            && (Self::MIN.t..=Self::MAX_T).contains(&self.t)
            && (1..=Self::MAX_P).contains(&self.p)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// The master password does not open the vault.
    WrongPassword,
    /// The file is damaged or not a vault (the reason for the user).
    Corrupt(Text),
    /// Read/write error (the system's text).
    Io(String),
}

impl VaultError {
    /// The message for the user (English template, translated by the UI).
    pub fn text(&self) -> Text {
        match self {
            VaultError::WrongPassword => txt!("Wrong master password."),
            VaultError::Corrupt(why) => txt!(
                "The file with saved passwords is damaged ({why}). The passwords cannot be read from it; you can delete it and start again.",
                why = why
            ),
            VaultError::Io(e) => txt!("File with saved passwords: {error}", error = e),
        }
    }
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text())
    }
}

impl std::error::Error for VaultError {}

fn corrupt(why: Text) -> VaultError {
    VaultError::Corrupt(why)
}

fn io(path: &Path, e: std::io::Error) -> VaultError {
    VaultError::Io(format!("{}: {e}", path.display()))
}

fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| VaultError::Io(format!("random numbers: {e}")))?;
    Ok(b)
}

fn derive_into(
    password: &[u8],
    salt: &[u8],
    p: KdfParams,
    out: &mut [u8; 32],
) -> Result<(), VaultError> {
    let params = argon2::Params::new(p.m_kib, p.t, p.p, Some(32))
        .map_err(|e| corrupt(txt!("Argon2 parameters: {error}", error = e.to_string())))?;
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(password, salt, out)
        .map_err(|e| VaultError::Io(format!("Argon2: {e}")))
}

fn derive(master: &SecretString, salt: &[u8; SALT], p: KdfParams) -> Result<SecretBox<[u8; 32]>, VaultError> {
    let mut key = SecretBox::new(Box::new([0u8; 32]));
    derive_into(master.expose_secret().as_bytes(), salt, p, key.expose_secret_mut())?;
    Ok(key)
}

fn cipher(key: &SecretBox<[u8; 32]>) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.expose_secret().into())
}

/// Header fields before the salt (magic, version, algorithm, Argon2 parameters, salt).
fn prefix(p: KdfParams, salt: &[u8; SALT]) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER);
    h.extend_from_slice(MAGIC);
    h.push(FORMAT);
    h.push(ALG);
    for v in [p.m_kib, p.t, p.p] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.extend_from_slice(salt);
    h
}

/// Payload encoding: count, then (id, password) pairs, each as u32 length + bytes.
fn encode(entries: &BTreeMap<String, SecretString>) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::new());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (id, pw) in entries {
        for s in [id.as_bytes(), pw.expose_secret().as_bytes()] {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s);
        }
    }
    out
}

fn decode(mut b: &[u8]) -> Result<BTreeMap<String, SecretString>, VaultError> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], VaultError> {
        if b.len() < n {
            return Err(corrupt(txt!("truncated data")));
        }
        let (h, t) = b.split_at(n);
        *b = t;
        Ok(h)
    }
    fn len(b: &mut &[u8]) -> Result<usize, VaultError> {
        Ok(u32::from_le_bytes(take(b, 4)?.try_into().unwrap()) as usize)
    }
    let mut map = BTreeMap::new();
    for _ in 0..len(&mut b)? {
        let n = len(&mut b)?;
        let id = std::str::from_utf8(take(&mut b, n)?).map_err(|_| corrupt(txt!("invalid text")))?.to_owned();
        let n = len(&mut b)?;
        let pw = std::str::from_utf8(take(&mut b, n)?).map_err(|_| corrupt(txt!("invalid text")))?;
        map.insert(id, SecretString::from(pw));
    }
    if !b.is_empty() {
        return Err(corrupt(txt!("extra data")));
    }
    Ok(map)
}

struct Parsed<'a> {
    params: KdfParams,
    salt: [u8; SALT],
    check_nonce: [u8; NONCE],
    check_tag: [u8; TAG],
    nonce: [u8; NONCE],
    header: &'a [u8],
    body: &'a [u8],
}

fn parse(data: &[u8]) -> Result<Parsed<'_>, VaultError> {
    if data.len() < 8 || &data[..8] != MAGIC {
        return Err(corrupt(txt!("it is not a RolidSCP password file")));
    }
    if data.len() < HEADER + TAG {
        return Err(corrupt(txt!("the file is truncated")));
    }
    if data[8] != FORMAT || data[9] != ALG {
        return Err(corrupt(txt!("unknown format version – was it created by a newer version of RolidSCP?")));
    }
    let u = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
    let params = KdfParams { m_kib: u(10), t: u(14), p: u(18) };
    if !params.valid() {
        return Err(corrupt(txt!("invalid Argon2 parameters")));
    }
    let arr = |from: usize, len: usize| &data[from..from + len];
    Ok(Parsed {
        params,
        salt: arr(22, SALT).try_into().unwrap(),
        check_nonce: arr(PREFIX, NONCE).try_into().unwrap(),
        check_tag: arr(PREFIX + NONCE, TAG).try_into().unwrap(),
        nonce: arr(PREFIX + NONCE + TAG, NONCE).try_into().unwrap(),
        header: &data[..HEADER],
        body: &data[HEADER..],
    })
}

/// Unlocked vault: the derived key and the decrypted passwords (by profile id).
pub struct Vault {
    path: PathBuf,
    params: KdfParams,
    salt: [u8; SALT],
    key: SecretBox<[u8; 32]>,
    check_nonce: [u8; NONCE],
    check_tag: [u8; TAG],
    entries: BTreeMap<String, SecretString>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault").field("path", &self.path).field("entries", &self.entries.len()).finish()
    }
}

impl Vault {
    /// `vault.bin` in the config dir.
    pub fn default_path() -> PathBuf {
        crate::config_dir().join("vault.bin")
    }

    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    /// Delete the vault file ("forgot the master password, start over").
    pub fn delete_file(path: &Path) -> Result<(), VaultError> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(path, e)),
        }
    }

    fn new_key(master: &SecretString, params: KdfParams) -> Result<KeyParts, VaultError> {
        let salt = random::<SALT>()?;
        let key = derive(master, &salt, params)?;
        let check_nonce = random::<NONCE>()?;
        let tag = cipher(&key)
            .encrypt(&XNonce::from(check_nonce), Payload { msg: b"", aad: &prefix(params, &salt) })
            .map_err(|_| corrupt(txt!("encryption")))?;
        Ok(KeyParts { params, salt, key, check_nonce, check_tag: tag.try_into().unwrap() })
    }

    /// Create a new, empty vault and write it to `path` (overwrites an existing file).
    pub fn create(path: &Path, master: &SecretString, params: KdfParams) -> Result<Vault, VaultError> {
        let k = Self::new_key(master, params)?;
        let v = Vault {
            path: path.to_owned(),
            params: k.params,
            salt: k.salt,
            key: k.key,
            check_nonce: k.check_nonce,
            check_tag: k.check_tag,
            entries: BTreeMap::new(),
        };
        v.save()?;
        Ok(v)
    }

    /// Open (unlock) the vault at `path` with the master password.
    pub fn open(path: &Path, master: &SecretString) -> Result<Vault, VaultError> {
        let data = std::fs::read(path).map_err(|e| io(path, e))?;
        let p = parse(&data)?;
        let key = derive(master, &p.salt, p.params)?;
        let c = cipher(&key);
        let aad = prefix(p.params, &p.salt);
        let mut check = Vec::with_capacity(TAG);
        check.extend_from_slice(&p.check_tag);
        if c.decrypt(&XNonce::from(p.check_nonce), Payload { msg: &check, aad: &aad }).is_err() {
            return Err(VaultError::WrongPassword);
        }
        let plain = Zeroizing::new(
            c.decrypt(&XNonce::from(p.nonce), Payload { msg: p.body, aad: p.header })
                .map_err(|_| corrupt(txt!("the data failed the authenticity check")))?,
        );
        Ok(Vault {
            path: path.to_owned(),
            params: p.params,
            salt: p.salt,
            check_nonce: p.check_nonce,
            check_tag: p.check_tag,
            entries: decode(&plain)?,
            key,
        })
    }

    /// Encrypt with a fresh nonce and write atomically (temp file + rename, 0600).
    pub fn save(&self) -> Result<(), VaultError> {
        let nonce = random::<NONCE>()?;
        let mut header = prefix(self.params, &self.salt);
        header.extend_from_slice(&self.check_nonce);
        header.extend_from_slice(&self.check_tag);
        header.extend_from_slice(&nonce);
        let plain = encode(&self.entries);
        let body = cipher(&self.key)
            .encrypt(&XNonce::from(nonce), Payload { msg: &plain, aad: &header })
            .map_err(|_| corrupt(txt!("encryption")))?;
        header.extend_from_slice(&body);
        crate::write_atomic(&self.path, &header).map_err(VaultError::Io)
    }

    /// Read the file back with the key in memory and compare it with what we hold.
    /// Used before deleting data from elsewhere (keyring migration).
    pub fn verify_on_disk(&self) -> Result<(), VaultError> {
        let data = std::fs::read(&self.path).map_err(|e| io(&self.path, e))?;
        let p = parse(&data)?;
        if p.salt != self.salt || p.params != self.params {
            return Err(corrupt(txt!("the file on the disk has a different key")));
        }
        let plain = Zeroizing::new(
            cipher(&self.key)
                .decrypt(&XNonce::from(p.nonce), Payload { msg: p.body, aad: p.header })
                .map_err(|_| corrupt(txt!("the data failed the authenticity check")))?,
        );
        let back = decode(&plain)?;
        let same = back.len() == self.entries.len()
            && back.iter().zip(&self.entries).all(|((a, x), (b, y))| {
                a == b && x.expose_secret() == y.expose_secret()
            });
        if same { Ok(()) } else { Err(corrupt(txt!("the content read back does not match"))) }
    }

    /// New master password: new salt and key, the file is rewritten atomically.
    /// On error the vault (and the file) stay as they were.
    pub fn change_password(&mut self, new_master: &SecretString, params: KdfParams) -> Result<(), VaultError> {
        let k = Self::new_key(new_master, params)?;
        let entries = std::mem::take(&mut self.entries);
        let fresh = Vault {
            path: self.path.clone(),
            params: k.params,
            salt: k.salt,
            key: k.key,
            check_nonce: k.check_nonce,
            check_tag: k.check_tag,
            entries,
        };
        let old = std::mem::replace(self, fresh);
        if let Err(e) = self.save() {
            let entries = std::mem::take(&mut self.entries);
            *self = Vault { entries, ..old };
            return Err(e);
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn params(&self) -> KdfParams {
        self.params
    }

    pub fn get(&self, id: &str) -> Option<&SecretString> {
        self.entries.get(id)
    }

    /// Set a password (call [`Vault::save`] afterwards).
    pub fn set(&mut self, id: &str, password: SecretString) {
        self.entries.insert(id.to_owned(), password);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.entries.remove(id).is_some()
    }

    /// Keep only the entries for which `keep(id)` is true. Returns whether anything was removed.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) -> bool {
        let n = self.entries.len();
        self.entries.retain(|id, _| keep(id));
        n != self.entries.len()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

struct KeyParts {
    params: KdfParams,
    salt: [u8; SALT],
    key: SecretBox<[u8; 32]>,
    check_nonce: [u8; NONCE],
    check_tag: [u8; TAG],
}

/// Delay before the next unlock attempt after `failures` wrong passwords in a row:
/// 0, 1, 2, 4, 8, 16, 30, 30… seconds. Nothing is ever deleted or blocked for good.
pub fn backoff(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        n => Duration::from_secs((1u64 << (n - 1).min(5)).min(30)),
    }
}

// ------------------------------------------------------------------ migration

/// Where 0.3/0.4 stored server passwords (the system keyring); a trait so tests can
/// use an isolated in-memory keyring.
pub trait LegacyStore {
    fn get(&self, profile_id: &str) -> Result<Option<String>, String>;
    fn delete(&self, profile_id: &str) -> Result<(), String>;
}

/// The real system keyring.
pub struct SystemKeyring;

impl LegacyStore for SystemKeyring {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        crate::secrets::get(id)
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        crate::secrets::delete(id)
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MigrationReport {
    /// Moved into the vault and deleted from the keyring.
    pub moved: Vec<String>,
    /// No password in the keyring for this profile.
    pub missing: Vec<String>,
    /// Could not be read; still in the keyring (id, error).
    pub failed: Vec<(String, String)>,
    /// Moved, but deleting from the keyring failed (id, error).
    pub not_deleted: Vec<(String, String)>,
}

/// Move the passwords of `ids` from the keyring into the vault. Keyring entries are
/// deleted only after the vault was written and read back successfully.
pub fn migrate_from_keyring(
    vault: &mut Vault,
    ids: &[String],
    store: &dyn LegacyStore,
) -> Result<MigrationReport, VaultError> {
    let mut report = MigrationReport::default();
    let mut read = Vec::new();
    for id in ids {
        match store.get(id) {
            Ok(Some(pw)) => {
                let pw = Zeroizing::new(pw);
                vault.set(id, SecretString::from(pw.as_str()));
                read.push(id.clone());
            }
            Ok(None) => report.missing.push(id.clone()),
            Err(e) => report.failed.push((id.clone(), e)),
        }
    }
    if !read.is_empty() {
        vault.save()?;
        vault.verify_on_disk()?;
    }
    for id in read {
        if let Err(e) = store.delete(&id) {
            report.not_deleted.push((id.clone(), e));
        }
        report.moved.push(id);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    const MASTER: &str = "Modrý Kůň skáče 7x";

    #[test]
    fn create_save_open_roundtrip() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        v.set("p1", pw(" Žluťoučký kůň – пароль 1 "));
        v.set("p2", pw(""));
        v.save().unwrap();
        v.verify_on_disk().unwrap();

        let back = Vault::open(&path, &pw(MASTER)).unwrap();
        assert_eq!(back.get("p1").unwrap().expose_secret(), " Žluťoučký kůň – пароль 1 ");
        assert_eq!(back.get("p2").unwrap().expose_secret(), "");
        assert_eq!(back.len(), 2);
        assert_eq!(back.params(), KdfParams::MIN);

        // The file holds neither the passwords nor the master password.
        let raw = std::fs::read(&path).unwrap();
        for needle in [" Žluťoučký kůň", MASTER] {
            assert!(!raw.windows(needle.len()).any(|w| w == needle.as_bytes()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // No temp file left behind.
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn every_write_uses_a_new_nonce() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        let a = std::fs::read(&path).unwrap();
        v.save().unwrap();
        let b = std::fs::read(&path).unwrap();
        assert_eq!(a[..PREFIX + NONCE + TAG], b[..PREFIX + NONCE + TAG], "same key/salt");
        assert_ne!(a[PREFIX + NONCE + TAG..HEADER], b[PREFIX + NONCE + TAG..HEADER], "new nonce");
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_password() {
        let d = dir();
        let path = d.path().join("vault.bin");
        Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        assert_eq!(Vault::open(&path, &pw("Modrý Kůň skáče 7X")).unwrap_err(), VaultError::WrongPassword);
        assert_eq!(Vault::open(&path, &pw("")).unwrap_err(), VaultError::WrongPassword);
        // The file is untouched and still opens.
        Vault::open(&path, &pw(MASTER)).unwrap();
    }

    #[test]
    fn damaged_file() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        v.set("p1", pw("secret"));
        v.save().unwrap();
        let good = std::fs::read(&path).unwrap();

        // Flipped bit in the encrypted payload: right password, damaged data.
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &bad).unwrap();
        assert!(matches!(Vault::open(&path, &pw(MASTER)), Err(VaultError::Corrupt(_))));

        // Truncated file.
        std::fs::write(&path, &good[..HEADER - 3]).unwrap();
        assert!(matches!(Vault::open(&path, &pw(MASTER)), Err(VaultError::Corrupt(_))));

        // Not a vault at all.
        std::fs::write(&path, b"hello world, this is not a vault file at all....................").unwrap();
        assert!(matches!(Vault::open(&path, &pw(MASTER)), Err(VaultError::Corrupt(_))));

        // Argon2 parameters in the header changed: the key differs -> rejected.
        let mut bad = good.clone();
        bad[14] = 3;
        std::fs::write(&path, &bad).unwrap();
        assert!(Vault::open(&path, &pw(MASTER)).is_err());

        // Missing file is an I/O error, not "wrong password".
        assert!(matches!(Vault::open(&d.path().join("nope"), &pw(MASTER)), Err(VaultError::Io(_))));
    }

    #[test]
    fn change_password() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        v.set("p1", pw("secret"));
        v.save().unwrap();
        let before = std::fs::read(&path).unwrap();
        v.change_password(&pw("New Vault password 9"), KdfParams::MIN).unwrap();
        let after = std::fs::read(&path).unwrap();
        assert_ne!(before[22..22 + SALT], after[22..22 + SALT], "new salt");
        assert_eq!(Vault::open(&path, &pw(MASTER)).unwrap_err(), VaultError::WrongPassword);
        let back = Vault::open(&path, &pw("New Vault password 9")).unwrap();
        assert_eq!(back.get("p1").unwrap().expose_secret(), "secret");
        // The vault in memory keeps working with the new key.
        v.set("p2", pw("x"));
        v.save().unwrap();
        v.verify_on_disk().unwrap();
    }

    #[test]
    fn failed_change_keeps_the_old_password() {
        let d = dir();
        let path = d.path().join("sub").join("vault.bin");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        v.set("p1", pw("secret"));
        v.save().unwrap();
        // Make the directory read-only so the atomic write fails.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dirp = path.parent().unwrap();
            std::fs::set_permissions(dirp, std::fs::Permissions::from_mode(0o500)).unwrap();
            let r = v.change_password(&pw("New Vault password 9"), KdfParams::MIN);
            std::fs::set_permissions(dirp, std::fs::Permissions::from_mode(0o700)).unwrap();
            if r.is_err() {
                // Old key still in use, file unchanged.
                v.verify_on_disk().unwrap();
                assert_eq!(v.get("p1").unwrap().expose_secret(), "secret");
                Vault::open(&path, &pw(MASTER)).unwrap();
            } // (running as root: the write succeeds, nothing to check)
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(0), Duration::ZERO);
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(3), Duration::from_secs(4));
        assert_eq!(backoff(6), Duration::from_secs(30));
        assert_eq!(backoff(1000), Duration::from_secs(30));
    }

    #[test]
    fn calibration_respects_the_minimum() {
        let p = KdfParams::calibrate(Duration::from_millis(1));
        assert_eq!(p, KdfParams::MIN);
        let p = KdfParams::calibrate(Duration::from_millis(500));
        assert!(p.m_kib >= KdfParams::MIN.m_kib && p.t >= 2 && p.valid());
    }

    /// Isolated test keyring.
    #[derive(Default)]
    struct FakeKeyring {
        items: RefCell<HashMap<String, String>>,
        broken: Vec<String>,
    }

    impl LegacyStore for FakeKeyring {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            if self.broken.iter().any(|b| b == id) {
                return Err("locked".into());
            }
            Ok(self.items.borrow().get(id).cloned())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.items.borrow_mut().remove(id);
            Ok(())
        }
    }

    #[test]
    fn migration_from_keyring() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        let kr = FakeKeyring { broken: vec!["c".into()], ..Default::default() };
        kr.items.borrow_mut().insert("a".into(), " password A ".into());
        kr.items.borrow_mut().insert("c".into(), "password C".into());
        kr.items.borrow_mut().insert("other".into(), "foreign".into());
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let r = migrate_from_keyring(&mut v, &ids, &kr).unwrap();
        assert_eq!(r.moved, vec!["a"]);
        assert_eq!(r.missing, vec!["b"]);
        assert_eq!(r.failed.len(), 1);
        // Moved entry deleted from the keyring, unreadable and unrelated ones kept.
        assert!(!kr.items.borrow().contains_key("a"));
        assert!(kr.items.borrow().contains_key("c"));
        assert!(kr.items.borrow().contains_key("other"));
        let back = Vault::open(&path, &pw(MASTER)).unwrap();
        assert_eq!(back.get("a").unwrap().expose_secret(), " password A ");
    }

    #[test]
    fn migration_keeps_the_keyring_when_the_vault_cannot_be_written() {
        let d = dir();
        let path = d.path().join("vault.bin");
        let mut v = Vault::create(&path, &pw(MASTER), KdfParams::MIN).unwrap();
        // Replace the file's location with a directory: the atomic rename fails.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("x"), b"x").unwrap();
        let kr = FakeKeyring::default();
        kr.items.borrow_mut().insert("a".into(), "password A".into());
        assert!(migrate_from_keyring(&mut v, &["a".to_string()], &kr).is_err());
        assert!(kr.items.borrow().contains_key("a"), "keyring entry must stay");
    }
}
