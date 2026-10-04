//! Keys the user asked Freshkube to remember, kept in the system's credential
//! store: the Keychain on macOS, Credential Manager on Windows and the Secret
//! Service on Linux. Nothing secret goes into the preferences folder; when no
//! store is available, a key stays in memory only.
//!
//! Every call can block, or make the system ask the user for permission, so
//! callers run them off the UI thread.
use std::sync::Arc;

pub(crate) trait SecretStore: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forgetting a key that isn't there succeeds.
    fn forget(&self, account: &str) -> Result<(), String>;
}

pub(crate) type Secrets = Arc<dyn SecretStore>;

/// What the store is called on this platform, for labels and errors.
pub(crate) const STORE_NAME: &str = if cfg!(target_os = "macos") {
    "Keychain"
} else if cfg!(windows) {
    "Credential Manager"
} else {
    "system keyring"
};

const SERVICE: &str = "Freshkube";

/// The platform's credential store.
pub(crate) struct SystemStore;

impl SecretStore for SystemStore {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.get_password()) {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(describe(error)),
        }
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(describe)
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(describe(error)),
        }
    }
}

/// Some errors carry the stored bytes, so only the platform's own message is
/// ever shown.
fn describe(error: keyring::Error) -> String {
    match error {
        keyring::Error::NoDefaultStore => format!("No {STORE_NAME} is available"),
        keyring::Error::NoStorageAccess(error) => {
            format!("The {STORE_NAME} is not available: {error}")
        }
        keyring::Error::PlatformFailure(error) => format!("The {STORE_NAME} failed: {error}"),
        _ => format!("The {STORE_NAME} refused the key"),
    }
}

/// A store in memory, so tests never touch the user's.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemoryStore {
    pub(crate) keys: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    pub(crate) unavailable: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl MemoryStore {
    fn check(&self) -> Result<(), String> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            Err(format!("No {STORE_NAME} is available"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        self.check()?;
        Ok(self.keys.lock().unwrap().get(account).cloned())
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), String> {
        self.check()?;
        self.keys
            .lock()
            .unwrap()
            .insert(account.into(), secret.into());
        Ok(())
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        self.check()?;
        self.keys.lock().unwrap().remove(account);
        Ok(())
    }
}
