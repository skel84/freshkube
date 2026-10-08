//! Keys the user asked Freshkube to remember, kept in the system's credential
//! store: the Keychain on macOS, Credential Manager on Windows and the Secret
//! Service on Linux. The trait and the in-memory store for tests are core's
//! (`freshkube_core::secrets`); the platform's store stays here, with the
//! `keyring` dependency.
pub(crate) use freshkube_core::secrets::{STORE_NAME, SecretStore, Secrets};

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
