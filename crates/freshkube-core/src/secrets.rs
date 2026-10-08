//! Where keys the user asked Freshkube to remember are kept: the system's
//! credential store, which the desktop app implements, or memory in tests.
//! Nothing secret goes into the preferences folder; when no store is
//! available, a key stays in memory only.
//!
//! Every call can block, or make the system ask the user for permission, so
//! callers run them off the UI thread. No type here implements `Debug` or
//! `Display`, so no key reaches a log or an error by formatting.
use std::sync::Arc;

pub trait SecretStore: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forgetting a key that isn't there succeeds.
    fn forget(&self, account: &str) -> Result<(), String>;
}

pub type Secrets = Arc<dyn SecretStore>;

/// What the store is called on this platform, for labels and errors.
pub const STORE_NAME: &str = if cfg!(target_os = "macos") {
    "Keychain"
} else if cfg!(windows) {
    "Credential Manager"
} else {
    "system keyring"
};

/// A store in memory, so tests never touch the user's.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct MemoryStore {
    pub keys: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    pub unavailable: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "testing"))]
impl MemoryStore {
    fn check(&self) -> Result<(), String> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            Err(format!("No {STORE_NAME} is available"))
        } else {
            Ok(())
        }
    }
}

#[cfg(any(test, feature = "testing"))]
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    /// Fails to build if `T` implements `Debug` or `Display`: with either,
    /// `check` has two candidates and the call is ambiguous.
    trait Unformatted<A> {
        fn check() {}
    }
    impl<T: ?Sized> Unformatted<()> for T {}
    struct ByDebug;
    impl<T: ?Sized + std::fmt::Debug> Unformatted<ByDebug> for T {}
    struct ByDisplay;
    impl<T: ?Sized + std::fmt::Display> Unformatted<ByDisplay> for T {}

    #[test]
    fn no_store_can_be_formatted() {
        <dyn SecretStore as Unformatted<_>>::check();
        <Secrets as Unformatted<_>>::check();
        <MemoryStore as Unformatted<_>>::check();
    }

    #[test]
    fn a_memory_store_keeps_reads_and_forgets_a_key() {
        let store = MemoryStore::default();
        assert_eq!(store.read("Coroot https://coroot.example"), Ok(None));
        store
            .write("Coroot https://coroot.example", "s3cret")
            .unwrap();
        assert_eq!(
            store.read("Coroot https://coroot.example"),
            Ok(Some("s3cret".into()))
        );
        store.forget("Coroot https://coroot.example").unwrap();
        assert_eq!(store.read("Coroot https://coroot.example"), Ok(None));
        // Forgetting a key that isn't there succeeds.
        assert_eq!(store.forget("Coroot https://coroot.example"), Ok(()));
    }

    #[test]
    fn an_unavailable_memory_store_names_the_store_and_never_the_key() {
        let store = MemoryStore::default();
        store.write("account", "s3cret").unwrap();
        store.unavailable.store(true, Ordering::SeqCst);
        let expected = Err(format!("No {STORE_NAME} is available"));
        assert_eq!(store.read("account"), expected);
        assert_eq!(store.write("account", "other").map(|_| None), expected);
        assert_eq!(store.forget("account").map(|_| None), expected);
        assert_eq!(
            store
                .keys
                .lock()
                .unwrap()
                .get("account")
                .map(String::as_str),
            Some("s3cret")
        );
    }
}
