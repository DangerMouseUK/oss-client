//! Remembers a player's password in the operating system's own credential store.
//!
//! **Where the password goes:** Windows Credential Manager, the macOS Keychain,
//! or on Linux whatever keyring the desktop provides through the Secret Service
//! (GNOME Keyring, KWallet). Nowhere else -- not the settings file, not
//! `login.ini`, not a file of our own -- and on a machine with none of those it
//! is not stored at all, and the client offers no tickbox. Using the system's
//! own vault means a player can always see and delete the entry with tools they
//! already have: it is listed under the name [`SERVICE`].
//!
//! **What never happens:** a password is never logged, printed or put in an
//! error message. Every type here that could hold one writes its own `Debug`.
//! The vault crates underneath log their own activity at debug level, so the
//! client turns their logging off entirely ([`QUIET_MODULES`]) rather than
//! trusting what each of them chooses to print.
//!
//! The screen code talks to [`PasswordStore`], so it can be tested against
//! [`MemoryStore`] without touching a real vault.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// The name every entry is filed under in the system's vault, which is what a
/// player looks for to find or delete it.
pub const SERVICE: &str = "Open SubSpace";

/// Modules whose logging the client switches off, whatever the log level asks
/// for: the vault crates log the credentials they handle, and what each one
/// prints is theirs to change in any release.
pub const QUIET_MODULES: &[&str] = &[
    "keyring_core",
    "windows_native_keyring_store",
    "apple_native_keyring_store",
    "zbus_secret_service_keyring_store",
    "secret_service",
    "zbus",
];

/// The entry name for a player in a zone: `name@host:port`, the name in lower
/// case because SubSpace names are not case-sensitive.
pub fn account(name: &str, zone: &str) -> String {
    format!("{}@{}", name.trim().to_lowercase(), zone.trim())
}

/// Why a vault could not do what was asked. It carries a description and never
/// the password: it is built only from the kind of failure, and from the
/// operating system's own message where there is one.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreError(String);

impl StoreError {
    /// The vault is there but would not open: locked, or the player said no to
    /// its prompt. Unlike "no vault", that is the player's choice, so the tickbox
    /// stays on screen.
    pub fn is_locked(&self) -> bool {
        self.0.starts_with(LOCKED)
    }
}

const LOCKED: &str = "the password store is locked or refused access";

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StoreError({:?})", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<keyring_core::Error> for StoreError {
    /// Each kind of failure in words. Deliberately not `{e:?}`: one kind
    /// (`BadEncoding`) carries the stored bytes themselves.
    fn from(e: keyring_core::Error) -> Self {
        use keyring_core::Error as E;
        StoreError(match e {
            E::NoStorageAccess(why) => format!("{LOCKED}: {why}"),
            E::PlatformFailure(why) => format!("the password store failed: {why}"),
            E::NoEntry => "no saved password".to_string(),
            E::BadEncoding(_) | E::BadDataFormat(..) => {
                "the saved password could not be read".to_string()
            }
            E::NoDefaultStore => "there is no password store".to_string(),
            E::TooLong(what, limit) => format!("{what} is longer than the store allows ({limit})"),
            _ => "the password store refused the request".to_string(),
        })
    }
}

/// Somewhere a password can be kept for a player in a zone.
pub trait PasswordStore: Send + Sync {
    /// Keeps `password` for `account`, replacing any already kept.
    fn save(&self, account: &str, password: &str) -> Result<(), StoreError>;
    /// The password kept for `account`, or `None` when there is none.
    fn load(&self, account: &str) -> Result<Option<String>, StoreError>;
    /// Removes any password kept for `account`. Removing one that is not there is
    /// not an error.
    fn forget(&self, account: &str) -> Result<(), StoreError>;
}

/// The operating system's own vault.
pub struct OsStore {
    store: Arc<keyring_core::CredentialStore>,
}

impl fmt::Debug for OsStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OsStore")
    }
}

impl OsStore {
    /// Opens this machine's vault, or `Ok(None)` when it has none -- which is
    /// when the client draws no tickbox at all.
    ///
    /// On Linux this can block while the desktop's keyring is asked whether it
    /// is there, so the client calls it away from the thread that draws. It
    /// first checks, without asking anyone, whether there is a session message
    /// bus to ask on: with none, there is certainly no vault.
    pub fn open() -> Result<Option<Self>, StoreError> {
        if !could_have_a_vault() {
            return Ok(None);
        }
        match open_platform_store() {
            Ok(Some(store)) => Ok(Some(Self { store })),
            Ok(None) => Ok(None),
            // A locked vault is still a vault: the tickbox stays, and the
            // vault asks the player when it is used.
            Err(e) if matches!(e, keyring_core::Error::NoStorageAccess(_)) => Err(e.into()),
            // Anything else at start-up means there is nothing usable here.
            Err(e) => {
                log::info!("no password store: {}", StoreError::from(e));
                Ok(None)
            }
        }
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, StoreError> {
        Ok(self.store.build(SERVICE, account, None)?)
    }
}

impl PasswordStore for OsStore {
    fn save(&self, account: &str, password: &str) -> Result<(), StoreError> {
        self.entry(account)?.set_password(password)?;
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, StoreError> {
        match self.entry(account)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn forget(&self, account: &str) -> Result<(), StoreError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(target_os = "windows")]
fn open_platform_store() -> keyring_core::Result<Option<Arc<keyring_core::CredentialStore>>> {
    Ok(Some(windows_native_keyring_store::Store::new()?))
}

#[cfg(target_os = "macos")]
fn open_platform_store() -> keyring_core::Result<Option<Arc<keyring_core::CredentialStore>>> {
    Ok(Some(apple_native_keyring_store::keychain::Store::new()?))
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn open_platform_store() -> keyring_core::Result<Option<Arc<keyring_core::CredentialStore>>> {
    Ok(Some(zbus_secret_service_keyring_store::Store::new()?))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
fn open_platform_store() -> keyring_core::Result<Option<Arc<keyring_core::CredentialStore>>> {
    Ok(None)
}

/// Whether this machine could have a vault at all, answered without asking one.
///
/// Only Linux and the other Unixes can say no here: their vault is reached over
/// the session's message bus, and with no bus there is nothing to reach. Both
/// places the bus is announced are checked -- the environment variable, and the
/// socket in the user's runtime folder -- before deciding there is none.
fn could_have_a_vault() -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty()) {
        return true;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| own_uid().map(|uid| format!("/run/user/{uid}").into()));
    runtime.is_some_and(|dir| dir.join("bus").exists())
}

/// This process's user number, read from `/proc` rather than through a system
/// call, which would need code this crate does not allow.
fn own_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// A store that keeps passwords in memory, for tests. It can be told to behave
/// as a locked vault.
#[derive(Default)]
pub struct MemoryStore {
    kept: Mutex<HashMap<String, String>>,
    locked: Mutex<bool>,
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let accounts: Vec<String> = self
            .kept
            .lock()
            .map(|k| k.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("MemoryStore")
            .field("accounts", &accounts)
            .finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// Every call fails as a locked vault does, until unlocked.
    pub fn lock(&self, locked: bool) {
        if let Ok(mut l) = self.locked.lock() {
            *l = locked;
        }
    }

    fn check(&self) -> Result<(), StoreError> {
        if self.locked.lock().is_ok_and(|l| *l) {
            return Err(StoreError(LOCKED.to_string()));
        }
        Ok(())
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PasswordStore for MemoryStore {
    fn save(&self, account: &str, password: &str) -> Result<(), StoreError> {
        self.check()?;
        self.kept()
            .insert(account.to_string(), password.to_string());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, StoreError> {
        self.check()?;
        Ok(self.kept().get(account).cloned())
    }

    fn forget(&self, account: &str) -> Result<(), StoreError> {
        self.check()?;
        self.kept().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "hunter2-correct-horse";

    #[test]
    fn the_entry_name_is_the_name_and_the_zone() {
        assert_eq!(
            account(" Pilot One ", "zone.example:5000"),
            "pilot one@zone.example:5000"
        );
        assert_eq!(account("PILOT", "z:1"), account("pilot", "z:1"));
    }

    #[test]
    fn save_load_forget() {
        let s = MemoryStore::default();
        assert_eq!(s.load("a@z:1"), Ok(None));
        s.save("a@z:1", SECRET).expect("saved");
        assert_eq!(s.load("a@z:1").expect("read").as_deref(), Some(SECRET));
        s.forget("a@z:1").expect("forgot");
        s.forget("a@z:1").expect("forgetting twice is fine");
        assert_eq!(s.load("a@z:1"), Ok(None));
        s.lock(true);
        assert!(s.load("a@z:1").expect_err("locked").is_locked());
    }

    /// Nothing here prints the password: not the stores, and not the error made
    /// from a failure that carries the stored bytes themselves.
    #[test]
    fn nothing_here_can_print_the_password() {
        let s = MemoryStore::default();
        s.save("pilot@zone:1", SECRET).expect("saved");
        let shown = format!("{s:?}");
        assert!(!shown.contains(SECRET), "{shown}");
        assert!(
            shown.contains("pilot@zone:1"),
            "the account still shows: {shown}"
        );

        let e: StoreError = keyring_core::Error::BadEncoding(SECRET.as_bytes().to_vec()).into();
        for text in [format!("{e}"), format!("{e:?}")] {
            assert!(!text.contains(SECRET), "{text}");
            assert!(!text.contains("104"), "not even as bytes: {text}");
        }
    }

    /// The real vault, on the machine running the test: save, read back and
    /// delete a throwaway entry.
    ///
    /// Ignored by default because it writes to the vault of whoever runs it --
    /// on a developer's own machine, their own Credential Manager or Keychain.
    /// Run it with `cargo test -p oss-credstore -- --ignored` on a machine
    /// where that is wanted, such as a CI runner.
    #[test]
    #[ignore = "writes a throwaway entry to this machine's real password store"]
    fn the_real_vault_round_trips_a_throwaway_entry() {
        let Some(store) = OsStore::open().expect("vault opens") else {
            eprintln!("no password store on this machine; nothing to test");
            return;
        };
        let who = account("oss-credstore-selftest", "localhost:0");
        store.save(&who, SECRET).expect("saved");
        assert_eq!(store.load(&who).expect("read").as_deref(), Some(SECRET));
        store.forget(&who).expect("deleted");
        assert_eq!(store.load(&who).expect("read"), None);
    }
}
