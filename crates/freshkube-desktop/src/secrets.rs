//! The system credential store, kept as a read-only migration fallback for
//! core's file store. The platform's store stays here with `keyring`.
pub(crate) use freshkube_core::secrets::{FileStore, SecretStore, Secrets};

const SERVICE: &str = "Freshkube";
const SYSTEM_STORE_NAME: &str = match freshkube_ui::platform::Platform::current() {
    freshkube_ui::platform::Platform::MacOs => "Keychain",
    freshkube_ui::platform::Platform::Windows => "Credential Manager",
    freshkube_ui::platform::Platform::Linux => "system keyring",
};

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
        keyring::Error::NoDefaultStore => format!("No {SYSTEM_STORE_NAME} is available"),
        keyring::Error::NoStorageAccess(error) => {
            format!("The {SYSTEM_STORE_NAME} is not available: {error}")
        }
        keyring::Error::PlatformFailure(error) => {
            format!("The {SYSTEM_STORE_NAME} failed: {error}")
        }
        _ => format!("The {SYSTEM_STORE_NAME} refused the key"),
    }
}
