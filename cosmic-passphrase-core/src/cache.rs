use zeroize::Zeroizing;

pub trait CacheBackend {
    fn read(&self, key: &str) -> Option<Zeroizing<String>>;

    /// Stores `value` under `key`. `label` is a short, human-readable
    /// description of what's being cached (e.g. `"GPG key passphrase
    /// (<keygrip>)"` or `"SSH key passphrase — /home/user/.ssh/id_ed25519"`)
    /// — for backends that expose stored items to the user (like a Secret
    /// Service item browsed in `seahorse`/`secret-tool`), this is what they
    /// see instead of an opaque, unrecognizable cache key.
    fn store(&self, key: &str, value: &str, label: &str, ttl: Option<std::time::Duration>);

    fn delete(&self, key: &str);
}

#[derive(Default)]
pub struct NullBackend;

impl NullBackend {
    pub fn new() -> Self {
        Self
    }
}

impl CacheBackend for NullBackend {
    fn read(&self, _key: &str) -> Option<Zeroizing<String>> {
        None
    }

    fn store(&self, _key: &str, _value: &str, _label: &str, _ttl: Option<std::time::Duration>) {}

    fn delete(&self, _key: &str) {}
}

/// Collection labels, in priority order, known to correspond to the
/// persistent, disk-backed default keyring across common Secret Service
/// implementations (GNOME Keyring, oo7-daemon). Used to avoid landing
/// passphrases in a transient, in-memory-only collection (such as the
/// freedesktop "session" collection) purely because of D-Bus enumeration
/// order — see `DbusBackend::get_or_init_collection`.
const PERSISTENT_COLLECTION_LABELS: &[&str] = &["Login", "login", "Default", "default"];

pub struct DbusBackend {
    runtime: tokio::runtime::Runtime,
    // Only a *successful* connection is cached. A failed attempt is not
    // stored here, so the next call retries from scratch instead of
    // treating one transient D-Bus hiccup (daemon not ready yet, session
    // collection briefly locked at login, ...) as a permanent "no keyring
    // available" verdict for the rest of this process's lifetime.
    collection: std::sync::OnceLock<oo7::dbus::Collection>,
}

impl Default for DbusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DbusBackend {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        Self { runtime, collection: std::sync::OnceLock::new() }
    }

    fn get_or_init_collection(&self) -> Option<&oo7::dbus::Collection> {
        if let Some(coll) = self.collection.get() {
            return Some(coll);
        }
        let coll = self.runtime.block_on(async {
            // Bypass oo7::Keyring::new()/Service::default_collection(),
            // which call ReadAlias – the oo7-daemon's read_alias returns
            // "/" for every alias, which is treated as "no such
            // collection" and falls back to CreateCollection, which panics
            // the daemon. Service::with_label() below only enumerates
            // collections and checks their Label property, so it never
            // touches ReadAlias.
            let svc = oo7::dbus::Service::new().await.map_err(|e| {
                eprintln!("cosmic-passphrase: oo7 Service::new failed: {e}");
            }).ok()?;
            let collections = svc.collections().await.map_err(|e| {
                eprintln!("cosmic-passphrase: oo7 collections() failed: {e}");
            }).ok()?;

            // Label each collection once up front so both the preferred-label
            // pass and the fallback pass below can share the same D-Bus round
            // trips instead of re-querying per candidate label.
            let mut labeled = Vec::with_capacity(collections.len());
            for coll in collections {
                match coll.label().await {
                    Ok(label) => labeled.push((label, coll)),
                    Err(e) => eprintln!("cosmic-passphrase: oo7 label() failed: {e}"),
                }
            }

            // Prefer a collection known to be the persistent, disk-backed
            // keyring over whichever collection happens to enumerate first.
            // Secret Service implementations commonly label it "Login"
            // (GNOME Keyring / oo7-daemon) or "login"/"Default"; anything
            // else (notably the freedesktop "session" collection) is
            // typically transient/in-memory and does not survive logout or
            // reboot. Without this, a passphrase could silently land in a
            // collection that vanishes at the next reboot depending on
            // which collection the daemon happens to list first.
            for &candidate in PERSISTENT_COLLECTION_LABELS {
                let Some(pos) = labeled.iter().position(|(label, _)| label == candidate) else {
                    continue;
                };
                match labeled[pos].1.is_locked().await {
                    Ok(false) => return Some(labeled.swap_remove(pos).1),
                    Ok(true) => {
                        eprintln!("cosmic-passphrase: collection {candidate:?} is locked, skipping");
                    }
                    Err(e) => eprintln!(
                        "cosmic-passphrase: oo7 is_locked() failed for {candidate:?}: {e}"
                    ),
                }
            }

            eprintln!(
                "cosmic-passphrase: no unlocked collection labeled any of {PERSISTENT_COLLECTION_LABELS:?}; \
                 falling back to the first unlocked collection (it may not persist across reboot)"
            );
            for (_, coll) in labeled {
                match coll.is_locked().await {
                    Ok(false) => return Some(coll),
                    Ok(true) => continue,
                    Err(e) => {
                        eprintln!("cosmic-passphrase: oo7 is_locked() failed: {e}");
                        return None;
                    }
                }
            }
            eprintln!("cosmic-passphrase: no unlocked Secret Service collection found");
            None
        })?;
        // DbusBackend is only ever driven synchronously from a single
        // caller, so a lost race on `set` can't happen in practice; fall
        // back to whatever is present either way.
        let _ = self.collection.set(coll);
        self.collection.get()
    }

    fn attrs(key: &str) -> [(&'static str, &str); 2] {
        [("application", "cosmic-passphrase"), ("key", key)]
    }
}

impl CacheBackend for DbusBackend {
    fn read(&self, key: &str) -> Option<Zeroizing<String>> {
        let coll = self.get_or_init_collection()?;
        let attrs = Self::attrs(key);
        let items = self.runtime.block_on(coll.search_items(&attrs)).map_err(|e| {
            eprintln!("cosmic-passphrase: oo7 search_items failed: {e}");
        }).ok()?;
        let item = items.first()?;
        let secret = self.runtime.block_on(item.secret()).map_err(|e| {
            eprintln!("cosmic-passphrase: oo7 item.secret() failed: {e}");
        }).ok()?;
        match &secret {
            oo7::Secret::Text(s) => Some(Zeroizing::new(s.clone())),
            oo7::Secret::Blob(b) => String::from_utf8(b.clone()).ok().map(Zeroizing::new),
        }
    }

    fn store(&self, key: &str, value: &str, label: &str, _ttl: Option<std::time::Duration>) {
        let Some(coll) = self.get_or_init_collection() else {
            return;
        };
        let attrs = Self::attrs(key);
        if let Err(e) = self.runtime.block_on(
            coll.create_item(label, &attrs, value, true, None),
        ) {
            eprintln!("cosmic-passphrase: oo7 create_item failed: {e}");
        }
    }

    fn delete(&self, key: &str) {
        let Some(coll) = self.get_or_init_collection() else {
            return;
        };
        let attrs = Self::attrs(key);
        match self.runtime.block_on(coll.search_items(&attrs)) {
            Ok(items) => {
                for item in &items {
                    if let Err(e) = self.runtime.block_on(item.delete(None)) {
                        eprintln!("cosmic-passphrase: oo7 item.delete() failed: {e}");
                    }
                }
            }
            Err(e) => eprintln!("cosmic-passphrase: oo7 search_items failed: {e}"),
        }
    }
}

/// A small, stable (non-cryptographic) string hash used to derive fixed-length
/// D-Bus Secret Service attribute values from arbitrary text (e.g. an SSH
/// askpass prompt). Deliberately not `std::collections::hash_map::DefaultHasher`:
/// its docs explicitly disclaim algorithm stability across Rust releases,
/// which would silently orphan every previously-cached entry after a
/// toolchain upgrade. FNV-1a has a fixed, documented definition.
pub fn hash_key(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_backend_read_returns_none() {
        let backend = NullBackend;
        assert!(backend.read("any_key").is_none());
    }

    #[test]
    fn test_null_backend_store_does_nothing() {
        let backend = NullBackend;
        backend.store("any_key", "value", "label", None);
        assert!(backend.read("any_key").is_none());
    }

    #[test]
    fn test_null_backend_new_returns_default() {
        let _ = NullBackend::new();
    }

    #[test]
    fn test_hash_key_deterministic() {
        let a = hash_key("hello");
        let b = hash_key("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn test_hash_key_different() {
        let a = hash_key("hello");
        let b = hash_key("world");
        assert_ne!(a, b);
    }

    #[test]
    fn test_hash_key_format() {
        let h = hash_key("test");
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_null_backend_delete_does_nothing() {
        let backend = NullBackend;
        backend.delete("any_key");
    }
}
