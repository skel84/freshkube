//! Where keys the user asked Freshkube to remember are kept: `keys.json`
//! beside the preferences, with a one-time read from the desktop's system
//! credential store for migration, or memory in tests.
//!
//! Every call can block, or make the system ask the user for permission, so
//! callers run them off the UI thread. No type here implements `Debug` or
//! `Display`, so no key reaches a log or an error by formatting.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

// String exception: key-store errors are displayed, never classified as cluster failures.
pub trait SecretStore: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forgetting a key that isn't there succeeds.
    fn forget(&self, account: &str) -> Result<(), String>;
}

pub type Secrets = Arc<dyn SecretStore>;

/// The file's name, for labels and errors.
pub const STORE_NAME: &str = "keys.json";

/// Remembered keys beside `preferences.json`. Calls reload the file before
/// changing it, and an unreadable or invalid file is never overwritten.
pub struct FileStore {
    file: PathBuf,
    fallback: Option<Secrets>,
    /// Also serializes this store's calls. Keep attempted fallback reads in
    /// memory until saved, so a failed save cannot prompt for the key again.
    migration: Mutex<BTreeMap<String, Result<Option<String>, String>>>,
}

/// `{ "keys": { account: key }, "migrated": [account, ...] }`.
/// Accounts retain their existing names. A migrated account stays here after
/// its key is forgotten, including when the fallback had no key to migrate.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    keys: BTreeMap<String, String>,
    migrated: BTreeSet<String>,
}

impl FileStore {
    /// Construction does no I/O. The fallback is read only, never written
    /// or asked to forget a key.
    pub fn new(preferences: &Path, fallback: Option<Secrets>) -> Self {
        Self {
            file: preferences.with_file_name(STORE_NAME),
            fallback,
            migration: Mutex::new(BTreeMap::new()),
        }
    }

    fn load(&self) -> Result<KeyFile, String> {
        let bytes = match std::fs::read(&self.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(KeyFile::default());
            }
            Err(_) => return Err(format!("Could not read {STORE_NAME}")),
        };
        // A parser's own error can repeat a key from the input.
        serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "Could not parse {STORE_NAME} at line {}, column {}",
                error.line(),
                error.column()
            )
        })
    }

    fn save(&self, value: &KeyFile) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(value)
            .map_err(|_| format!("Could not serialize {STORE_NAME}"))?;
        text.push('\n');
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| format!("Could not create the folder for {STORE_NAME}"))?;
        }
        // The temporary file is created 0600 before writing any key bytes;
        // replacing an existing file does not copy its old permissions.
        crate::workspace::write_whole(&self.file, &text)
            .map_err(|_| format!("Could not save {STORE_NAME}"))
    }
}

impl SecretStore for FileStore {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        let mut migration = self
            .migration
            .lock()
            .map_err(|_| format!("{STORE_NAME} is unavailable"))?;
        let mut value = self.load()?;
        if value.keys.contains_key(account) || value.migrated.contains(account) {
            return Ok(value.keys.get(account).cloned());
        }
        let Some(fallback) = &self.fallback else {
            return Ok(None);
        };
        let secret = migration
            .entry(account.to_owned())
            .or_insert_with(|| {
                fallback
                    .read(account)
                    .map_err(|_| "Could not read the previous credential store".to_owned())
            })
            .clone()?;
        if let Some(secret) = &secret {
            value.keys.insert(account.to_owned(), secret.clone());
        }
        value.migrated.insert(account.to_owned());
        self.save(&value)?;
        migration.remove(account);
        Ok(secret)
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), String> {
        let mut migration = self
            .migration
            .lock()
            .map_err(|_| format!("{STORE_NAME} is unavailable"))?;
        let mut value = self.load()?;
        value.keys.insert(account.to_owned(), secret.to_owned());
        value.migrated.insert(account.to_owned());
        self.save(&value)?;
        migration.remove(account);
        Ok(())
    }

    fn forget(&self, account: &str) -> Result<(), String> {
        let mut migration = self
            .migration
            .lock()
            .map_err(|_| format!("{STORE_NAME} is unavailable"))?;
        let mut value = self.load()?;
        value.keys.remove(account);
        value.migrated.insert(account.to_owned());
        self.save(&value)?;
        migration.remove(account);
        Ok(())
    }
}

/// A store in memory, so tests never touch the user's.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct MemoryStore {
    pub keys: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    pub unavailable: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "testing"))]
impl MemoryStore {
    // String exception: the test store preserves the credential-store message contract.
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
        <FileStore as Unformatted<_>>::check();
        <KeyFile as Unformatted<_>>::check();
        <Fallback as Unformatted<_>>::check();
    }

    #[test]
    fn a_memory_store_keeps_reads_and_forgets_a_key() {
        let store = MemoryStore::default();
        assert!(
            store
                .read("Coroot https://coroot.example")
                .unwrap()
                .is_none()
        );
        store
            .write("Coroot https://coroot.example", "s3cret")
            .unwrap();
        assert_key(&store, "Coroot https://coroot.example", "s3cret");
        store.forget("Coroot https://coroot.example").unwrap();
        assert!(
            store
                .read("Coroot https://coroot.example")
                .unwrap()
                .is_none()
        );
        // Forgetting a key that isn't there succeeds.
        assert_eq!(store.forget("Coroot https://coroot.example"), Ok(()));
    }

    #[test]
    fn an_unavailable_memory_store_names_the_store_and_never_the_key() {
        let store = MemoryStore::default();
        store.write("account", "s3cret").unwrap();
        store.unavailable.store(true, Ordering::SeqCst);
        let expected = Err(format!("No {STORE_NAME} is available"));
        assert!(store.read("account") == expected, "unexpected store error");
        assert_eq!(store.write("account", "other").map(|_| None), expected);
        assert_eq!(store.forget("account").map(|_| None), expected);
        let keys = store.keys.lock().unwrap();
        let expected = "s3cret";
        assert!(
            keys.get("account").map(String::as_str) == Some(expected),
            "the unavailable store changed its key"
        );
    }

    fn assert_key(store: &dyn SecretStore, account: &str, expected: &str) {
        let value = store.read(account).expect("key read failed");
        assert!(value.as_deref() == Some(expected), "stored key differs");
    }

    #[derive(Default)]
    struct Fallback {
        memory: MemoryStore,
        reads: std::sync::atomic::AtomicUsize,
    }

    impl SecretStore for Fallback {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.memory.read(account)
        }

        fn write(&self, _: &str, _: &str) -> Result<(), String> {
            panic!("the fallback must never be written")
        }

        fn forget(&self, _: &str) -> Result<(), String> {
            panic!("the fallback must never be asked to forget")
        }
    }

    #[test]
    fn file_keys_round_trip_beside_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("nested/preferences.json");
        let store = FileStore::new(&preferences, None);
        assert!(store.read("coroot").unwrap().is_none());
        assert!(!store.file.exists());
        store.write("coroot", "first-test-key").unwrap();
        store.write("metrics", "second-test-key").unwrap();
        assert!(preferences.with_file_name("keys.json").is_file());
        let reopened = FileStore::new(&preferences, None);
        assert_key(&reopened, "coroot", "first-test-key");
        assert_key(&reopened, "metrics", "second-test-key");
        reopened.write("coroot", "replacement-test-key").unwrap();
        assert_key(&store, "coroot", "replacement-test-key");
        reopened.forget("coroot").unwrap();
        reopened.forget("coroot").unwrap();
        assert!(store.read("coroot").unwrap().is_none());
        assert_key(&store, "metrics", "second-test-key");
    }

    #[cfg(unix)]
    #[test]
    fn file_keys_are_private_on_creation_and_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let store = FileStore::new(&directory.path().join("preferences.json"), None);
        store.write("account", "test-key").unwrap();
        let mode = || std::fs::metadata(&store.file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(), 0o600);
        std::fs::set_permissions(&store.file, std::fs::Permissions::from_mode(0o644)).unwrap();
        store.write("account", "replacement-test-key").unwrap();
        assert_eq!(mode(), 0o600);
        store.forget("account").unwrap();
        assert_eq!(mode(), 0o600);
    }

    #[test]
    fn migration_is_once_per_account_and_forgetting_survives_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        let fallback = Arc::new(Fallback::default());
        fallback.memory.write("coroot", "old-test-key").unwrap();
        let store = FileStore::new(&preferences, Some(fallback.clone()));
        assert_key(&store, "coroot", "old-test-key");
        assert_key(&store, "coroot", "old-test-key");
        assert!(store.read("metrics").unwrap().is_none());
        fallback.memory.write("metrics", "late-test-key").unwrap();
        assert!(store.read("metrics").unwrap().is_none());
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 2);
        let reopened = FileStore::new(&preferences, Some(fallback.clone()));
        assert_key(&reopened, "coroot", "old-test-key");
        reopened.forget("coroot").unwrap();
        let reopened = FileStore::new(&preferences, Some(fallback.clone()));
        assert!(reopened.read("coroot").unwrap().is_none());
        assert!(reopened.read("metrics").unwrap().is_none());
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 2);
        assert_key(&fallback.memory, "coroot", "old-test-key");
    }

    #[test]
    fn writes_and_forgets_before_a_read_do_not_migrate() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        let fallback = Arc::new(Fallback::default());
        fallback.memory.write("forgotten", "old-test-key").unwrap();
        let store = FileStore::new(&preferences, Some(fallback.clone()));
        store.write("written", "new-test-key").unwrap();
        store.forget("forgotten").unwrap();
        let reopened = FileStore::new(&preferences, Some(fallback.clone()));
        assert_key(&reopened, "written", "new-test-key");
        assert!(reopened.read("forgotten").unwrap().is_none());
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 0);
        assert_key(&fallback.memory, "forgotten", "old-test-key");
    }

    #[test]
    fn a_fallback_failure_is_not_asked_twice() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = Arc::new(Fallback::default());
        fallback.memory.unavailable.store(true, Ordering::SeqCst);
        let store = FileStore::new(
            &directory.path().join("preferences.json"),
            Some(fallback.clone()),
        );
        assert!(store.read("account").is_err());
        assert!(store.read("account").is_err());
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 1);
        assert!(!store.file.exists());
        store.write("account", "new-test-key").unwrap();
        assert_key(&store, "account", "new-test-key");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_migration_save_retries_without_reading_the_fallback_again() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let fallback = Arc::new(Fallback::default());
        fallback.memory.write("account", "old-test-key").unwrap();
        let store = FileStore::new(
            &directory.path().join("preferences.json"),
            Some(fallback.clone()),
        );
        store.write("other", "other-test-key").unwrap();
        let before = std::fs::read(&store.file).unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let first = store.read("account");
        let second = store.read("account");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(first.is_err());
        assert!(second.is_err());
        assert!(
            std::fs::read(&store.file).unwrap() == before,
            "file changed"
        );
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 1);
        assert_key(&store, "account", "old-test-key");
        assert_key(&store, "other", "other-test-key");
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_reads_migrate_an_account_once() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = Arc::new(Fallback::default());
        fallback.memory.write("account", "old-test-key").unwrap();
        let store = Arc::new(FileStore::new(
            &directory.path().join("preferences.json"),
            Some(fallback.clone()),
        ));
        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    barrier.wait();
                    assert_key(store.as_ref(), "account", "old-test-key");
                });
            }
        });
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn broken_files_return_errors_and_stay_whole() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = Arc::new(Fallback::default());
        let store = FileStore::new(
            &directory.path().join("preferences.json"),
            Some(fallback.clone()),
        );
        for bytes in [
            b"{\"keys\":{\"account\":\"test-key\"}".as_slice(),
            br#"{"keys":{"account":"test-key"},"migrated":"test-key"}"#,
        ] {
            std::fs::write(&store.file, bytes).unwrap();
            let Err(error) = store.read("account") else {
                panic!("broken file was read");
            };
            let key = "test-key";
            assert!(!error.contains(key), "parse error exposed the key");
            assert!(store.write("account", "replacement-test-key").is_err());
            assert!(store.forget("account").is_err());
            assert!(std::fs::read(&store.file).unwrap() == bytes, "file changed");
        }
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_return_errors_and_stay_whole() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let fallback = Arc::new(Fallback::default());
        let store = FileStore::new(
            &directory.path().join("preferences.json"),
            Some(fallback.clone()),
        );
        store.write("account", "test-key").unwrap();
        let before = std::fs::read(&store.file).unwrap();
        std::fs::set_permissions(&store.file, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(store.read("account").is_err());
        assert!(store.write("account", "replacement-test-key").is_err());
        assert!(store.forget("account").is_err());
        let mode = std::fs::metadata(&store.file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o000);
        std::fs::set_permissions(&store.file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            std::fs::read(&store.file).unwrap() == before,
            "file changed"
        );
        assert_eq!(fallback.reads.load(Ordering::SeqCst), 0);
    }
}
