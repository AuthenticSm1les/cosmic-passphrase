use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn binary_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_pinentry_cosmic") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("target");
    path.push("debug");
    path.push("pinentry-cosmic");
    path
}

fn run_pinentry(input: &str) -> String {
    let mut cmd = Command::new(binary_path());
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn pinentry-cosmic");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

// ── D-Bus Secret Service helpers ─────────────────────────────────────

fn has_dbus_name_owner(name: &str) -> bool {
    // Use dbus-send instead of busctl because busctl's NameHasOwner
    // returns wrong results for services running under PrivateUsers=yes
    // + PrivateNetwork=yes namespace isolation.
    std::process::Command::new("dbus-send")
        .arg("--session")
        .arg("--print-reply")
        .arg("--dest=org.freedesktop.DBus")
        .arg("/org/freedesktop/DBus")
        .arg("org.freedesktop.DBus.NameHasOwner")
        .arg(format!("string:{}", name))
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).contains("true"))
            } else {
                None
            }
        })
        .unwrap_or(false)
}

fn dbus_secret_service_available() -> bool {
    // Fast check: see if the D-Bus name has an owner.
    // This avoids the 30s default timeout on oo7 method calls when
    // no secret service daemon is running.
    if !has_dbus_name_owner("org.freedesktop.secrets") {
        return false;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async { get_unlocked_collection().await.is_some() })
}

async fn get_unlocked_collection() -> Option<oo7::dbus::Collection> {
    // Bypass default_collection() (which calls ReadAlias) because the
    // oo7-daemon's read_alias returns "/" for every alias due to a bug.
    // Instead, iterate the available collections and return the first
    // unlocked one (the session collection).
    let svc = oo7::dbus::Service::new().await.ok()?;
    let collections = svc.collections().await.ok()?;
    for coll in collections {
        if !coll.is_locked().await.ok()? {
            return Some(coll);
        }
    }
    None
}

async fn dbus_store(key: &str, value: &str) -> bool {
    match get_unlocked_collection().await {
        Some(coll) => {
            coll.create_item(
                "cosmic-passphrase test",
                &[("application", "cosmic-passphrase"), ("key", key)],
                value,
                true,
                None,
            )
            .await
            .is_ok()
        }
        None => false,
    }
}

async fn dbus_delete(key: &str) {
    let coll = get_unlocked_collection().await;
    if let Some(coll) = coll
        && let Ok(items) = coll
            .search_items(&[("application", "cosmic-passphrase"), ("key", key)])
            .await
    {
        for item in &items {
            let _ = item.delete(None).await;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn test_dbus_cache_gpg_getpin_from_cache_requires_consent_fails_closed_without_display() {
    // A cache hit no longer silently returns the passphrase — it first
    // shows a dedicated Allow/Deny consent dialog ("gpg-agent wants to
    // access the saved passphrase..."). Without a display (as in this test
    // harness), that dialog can't be shown or approved, so the passphrase
    // must NOT leak over Assuan, and the cache entry must survive (a
    // failed/declined consent is not evidence it's wrong).
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "TESTDBUS1234";
    let cache_key = format!("gpg:{}", keygrip);
    let passphrase = "my_dbus_gpg_pass";
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, passphrase)),
        "failed to store passphrase in D-Bus"
    );

    let input = format!(
        "SETKEYINFO {}\n\
         OPTION allow-external-password-cache\n\
         OPTION timeout=1\n\
         GETPIN\n\
         BYE\n",
        keygrip
    );
    let output = run_pinentry(&input);

    assert!(
        !output.contains(&format!("D {}", passphrase)),
        "the cached passphrase must never leak without consent: {:?}",
        output
    );

    let items = rt.block_on(async {
        let coll = get_unlocked_collection().await?;
        coll.search_items(&[("application", "cosmic-passphrase"), ("key", &cache_key)])
            .await
            .ok()
    });
    assert!(
        items.is_none_or(|v| !v.is_empty()),
        "cache entry should survive a failed/declined consent"
    );

    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_gpg_getpin_from_cache_with_real_gpg_agent_keyinfo_format() {
    // Real gpg-agent does NOT send a bare keygrip in SETKEYINFO: it sends
    // "<flag>/<keygrip>" (e.g. "n/<keygrip>" or "c/<keygrip>", the flag
    // reflecting gpg-agent's own unrelated in-memory cache state). This is
    // a regression test — verified live against a real gpg-agent + gpg
    // sign operation — for a bug where pinentry-cosmic used the *raw*
    // SETKEYINFO string (flag included) as the oo7 cache key, so a cache
    // entry stored under the bare keygrip was never found, and worse, the
    // effective cache key silently changed depending on gpg-agent's flag.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "4CB13907FA13F63A8CE699C494B5774EB96A9CC7";
    let cache_key = format!("gpg:{}", keygrip);
    let passphrase = "real_wire_format_pass";
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, passphrase)),
        "failed to store passphrase in D-Bus"
    );

    for flag in ["n", "c"] {
        let input = format!(
            "SETKEYINFO {flag}/{keygrip}\n\
             OPTION allow-external-password-cache\n\
             OPTION timeout=1\n\
             GETPIN\n\
             BYE\n"
        );
        let output = run_pinentry(&input);

        // Cache hits now require consent (see the test above); without a
        // display the passphrase still must not leak. The point of *this*
        // test — that the SETKEYINFO flag prefix is stripped so the
        // right cache entry is found *at all* — is exercised by the fact
        // that D-Bus lookups happen the same way for both flags; a wrong
        // key derivation would be invisible either way here, so this is
        // paired with `keyinfo_cache_id`'s direct unit tests in
        // `src/main.rs` for the actual stripping logic.
        assert!(
            !output.contains(&format!("D {}", passphrase)),
            "flag {flag:?}: must not leak without consent: {:?}",
            output
        );
    }

    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_gpg_getpin_touches_file_even_when_cache_hit_declines_consent() {
    // gpg-agent relies on OPTION touch-file being touched on every completed
    // request. Originally a regression test for a bug where the touch-file
    // write was skipped specifically on the cache-hit early-return path;
    // since a cache hit now also requires consent (which can't be
    // shown/approved without a display, as here), the early-return path
    // itself can't be automated-tested anymore — see docs/TESTING.md. What
    // *is* still verified here: `touch_file_if_needed` is called on the
    // fallthrough path too (declined/failed consent -> manual dialog ->
    // also fails without a display -> still touches the file), so this
    // continues to guard against the touch-file call being dropped
    // entirely from the cache-hit branch of `GETPIN`.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "TOUCHFILECACHEHIT";
    let cache_key = format!("gpg:{}", keygrip);
    let passphrase = "touch_file_cache_pass";
    let rt = tokio::runtime::Runtime::new().unwrap();

    let touch_path = std::env::temp_dir().join("pinentry-cosmic-test-touch-cachehit");
    let _ = std::fs::remove_file(&touch_path);

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, passphrase)),
        "failed to store passphrase in D-Bus"
    );

    let input = format!(
        "OPTION touch-file={}\n\
         SETKEYINFO {}\n\
         OPTION allow-external-password-cache\n\
         OPTION timeout=1\n\
         GETPIN\n\
         BYE\n",
        touch_path.to_str().unwrap(),
        keygrip
    );
    let output = run_pinentry(&input);

    assert!(
        !output.contains(&format!("D {}", passphrase)),
        "must not leak without consent: {:?}",
        output
    );
    assert!(
        touch_path.exists(),
        "touch-file should still be created even though the cache hit's consent failed closed"
    );

    let _ = std::fs::remove_file(&touch_path);
    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_gpg_cache_miss_falls_through() {
    // When no cached passphrase exists and no display is available,
    // the dialog returns empty. The process must not crash or hang.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "NOCACHEKEYGRIP";
    let cache_key = format!("gpg:{}", keygrip);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));

    let input = format!(
        "SETKEYINFO {}\n\
         OPTION allow-external-password-cache\n\
         OPTION timeout=1\n\
         GETPIN\n\
         BYE\n",
        keygrip
    );
    let output = run_pinentry(&input);

    // Without display, the dialog cannot show, so the result should
    // be a cancellation (ERR) or empty data — but never a crash/hang.
    // The output should NOT contain our phantom passphrase.
    assert!(
        !output.contains("D test_pass"),
        "should not return a cached passphrase that doesn't exist"
    );
}

#[test]
fn test_dbus_cache_gpg_with_error_skips_cache_and_clears_it() {
    // When a cached passphrase is wrong, gpg-agent sends SETERROR before
    // the next GETPIN. The cache entry must be invalidated. This is
    // unaffected by the consent gate — eviction happens purely based on
    // `state.error`, before any dialog (consent or otherwise) is
    // attempted — so it doesn't need a prior successful cache-hit return
    // to set up (unlike before the consent dialog existed).
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "ERRRECOVERYKEY";
    let cache_key = format!("gpg:{}", keygrip);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, "wrong_passphrase")),
        "failed to store wrong passphrase in D-Bus"
    );

    // SETERROR + GETPIN in the same session as if a previous (unrelated
    // to this test) attempt with this cached value had just failed.
    let input = format!(
        "SETKEYINFO {}\n\
         OPTION allow-external-password-cache\n\
         SETERROR Bad passphrase\n\
         OPTION timeout=1\n\
         GETPIN\n\
         BYE\n",
        keygrip
    );
    let output = run_pinentry(&input);
    assert!(
        !output.contains("D wrong_passphrase"),
        "should NOT return cached wrong passphrase after SETERROR: {:?}",
        output
    );

    // Verify the cache entry was actually deleted from D-Bus
    let items = rt.block_on(async {
        let coll = get_unlocked_collection().await?;
        coll.search_items(&[("application", "cosmic-passphrase"), ("key", &cache_key)])
            .await
            .ok()
    });
    assert!(
        items.is_none_or(|v| v.is_empty()),
        "cache entry should be deleted from D-Bus after SETERROR"
    );
}

#[test]
fn test_dbus_cache_gpg_multiple_getpin_without_error_keeps_cache() {
    // Multiple GETPIN calls without SETERROR must never evict the cache
    // entry — only a SETERROR-flagged retry does that. Since a cache hit
    // now requires consent to actually be *returned* (untestable here
    // without a display — see the dedicated consent test above), this
    // verifies the still-automatable half of the claim: the entry survives
    // repeated hits with no error in between, i.e. nothing here ever calls
    // `cache.delete()` outside the SETERROR path.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "MULTIGETPINKEY";
    let cache_key = format!("gpg:{}", keygrip);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, "cached_value")),
        "failed to store in D-Bus"
    );

    for i in 0..2 {
        let input = format!(
            "SETKEYINFO {}\n\
             OPTION allow-external-password-cache\n\
             OPTION timeout=1\n\
             GETPIN\n\
             BYE\n",
            keygrip
        );
        let output = run_pinentry(&input);
        assert!(
            !output.contains("D cached_value"),
            "attempt {i}: must not leak without consent: {:?}",
            output
        );
    }

    let items = rt.block_on(async {
        let coll = get_unlocked_collection().await?;
        coll.search_items(&[("application", "cosmic-passphrase"), ("key", &cache_key)])
            .await
            .ok()
    });
    assert!(
        items.is_none_or(|v| !v.is_empty()),
        "cache entry should survive repeated hits with no SETERROR in between"
    );

    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_gpg_without_allow_external_skips_cache() {
    // When allow-external-password-cache is NOT set, GETPIN must not
    // consult the D-Bus cache, even if an item exists.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let keygrip = "SKIPCACHEKEY";
    let cache_key = format!("gpg:{}", keygrip);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, "should_not_be_returned")),
        "failed to store in D-Bus"
    );

    // Note: no OPTION allow-external-password-cache
    let input = format!(
        "SETKEYINFO {}\n\
         OPTION timeout=1\n\
         GETPIN\n\
         BYE\n",
        keygrip
    );
    let output = run_pinentry(&input);

    // Must NOT have used the cache
    assert!(
        !output.contains("D should_not_be_returned"),
        "should NOT read from D-Bus cache without allow-external-password-cache"
    );

    rt.block_on(dbus_delete(&cache_key));
}
