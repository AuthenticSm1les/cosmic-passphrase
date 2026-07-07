use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn binary_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_cosmic_ssh_askpass") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("target");
    path.push("debug");
    path.push("cosmic-ssh-askpass");
    path
}

fn run_askpass(
    read_file: Option<&str>,
    write_file: Option<&str>,
    args: &[&str],
) -> std::process::Output {
    let mut cmd = Command::new(binary_path());
    cmd.args(args);
    cmd.env_remove("OO7_PASSPHRASE_READ_FILE");
    cmd.env_remove("OO7_PASSPHRASE_WRITE_FILE");
    // Unset display variables so the dialog fails immediately instead of
    // showing on the user's screen and waiting for user interaction.
    cmd.env("DISPLAY", "");
    cmd.env("WAYLAND_DISPLAY", "");
    if let Some(path) = read_file {
        cmd.env("OO7_PASSPHRASE_READ_FILE", path);
    }
    if let Some(path) = write_file {
        cmd.env("OO7_PASSPHRASE_WRITE_FILE", path);
    }
    let output = cmd.output().unwrap();
    if !output.stderr.is_empty() {
        eprintln!("askpass stderr: {}", String::from_utf8_lossy(&output.stderr));
    }
    output
}

// ── oo7 read cache tests ───────────────────────────────────────────
// Simulates oo7-ssh-agent writing a cached passphrase to a temp file,
// then invoking cosmic-ssh-askpass with OO7_PASSPHRASE_READ_FILE set.

#[test]
fn test_oo7_read_cache_flow() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-cache-read");

    // oo7-ssh-agent writes cached passphrase to file
    std::fs::write(&read_path, "my_cached_passphrase").unwrap();

    // oo7-ssh-agent invokes askpass with read file path
    let output = run_askpass(
        Some(read_path.to_str().unwrap()),
        None,
        &["Enter passphrase for key /home/user/.ssh/id_rsa:"],
    );

    assert!(
        output.status.success(),
        "askpass should exit 0 when reading from cache"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "my_cached_passphrase",
        "should output the cached passphrase"
    );

    let _ = std::fs::remove_file(&read_path);
}

#[test]
fn test_oo7_cache_with_special_chars() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-special");

    std::fs::write(&read_path, "pässwörd with spaces and !@#$%^&*()").unwrap();

    let output = run_askpass(Some(read_path.to_str().unwrap()), None, &[]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "pässwörd with spaces and !@#$%^&*()");

    let _ = std::fs::remove_file(&read_path);
}

#[test]
fn test_oo7_cache_multiple_consecutive_calls() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-multi-cache");

    // Simulate multiple sequential ssh-agent invocations
    for i in 0..3 {
        std::fs::write(&read_path, format!("passphrase_{}", i)).unwrap();

        let output = run_askpass(Some(read_path.to_str().unwrap()), None, &[]);

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), format!("passphrase_{}", i));
    }

    let _ = std::fs::remove_file(&read_path);
}

#[test]
fn test_oo7_cache_empty_cache_falls_through() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-empty-cache");

    // oo7-ssh-agent writes empty cache (passphrase not found)
    std::fs::write(&read_path, "").unwrap();

    let _output = run_askpass(Some(read_path.to_str().unwrap()), None, &[]);

    // Falls through to dialog (which fails without display)
    // The important thing is the process finishes without hanging
    let _ = std::fs::remove_file(&read_path);
}

#[test]
fn test_oo7_cache_missing_file_falls_through() {
    let read_path = "/tmp/oo7-nonexistent-cache-123456789";

    let _output = run_askpass(Some(read_path), None, &[]);

    // Falls through to dialog (which fails without display)
    // The important thing is the process finishes without hanging
}

// ── oo7 write cache tests ──────────────────────────────────────────
// Simulates oo7-ssh-agent setting OO7_PASSPHRASE_WRITE_FILE so that
// cosmic-ssh-askpass writes the passphrase there when "Remember" is checked.
//
// Note: these tests require GUI interaction (checking "Remember" then
// clicking "Unlock"), so they can't be fully automated. We verify
// the binary doesn't crash and the write file is NOT written
// (because no GUI interaction occurs to trigger the write).

#[test]
fn test_oo7_write_file_not_written_without_interaction() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-write-read");
    let write_path = dir.join("oo7-test-write-out");

    std::fs::write(&read_path, "").unwrap();

    let _output = run_askpass(
        Some(read_path.to_str().unwrap()),
        Some(write_path.to_str().unwrap()),
        &[],
    );

    // No interaction, so write file should NOT exist
    assert!(
        !write_path.exists(),
        "write file should not be created without user interaction"
    );

    let _ = std::fs::remove_file(&read_path);
}

// ── oo7 full round-trip simulation ─────────────────────────────────
// Simulates the oo7-ssh-agent workflow:
// 1. Agent caches passphrase to a temp file
// 2. Invokes askpass with OO7_PASSPHRASE_READ_FILE
// 3. Askpass reads and prints the passphrase
// 4. Agent reads stdout to get the passphrase

#[test]
fn test_oo7_full_round_trip() {
    let dir = std::env::temp_dir();
    let cache_path = dir.join("oo7-test-roundtrip");

    // Step 1: Agent caches passphrase
    let expected = "my-ssh-key-passphrase-2024";
    std::fs::write(&cache_path, expected).unwrap();

    // Step 2-3: Agent invokes askpass
    let output = run_askpass(
        Some(cache_path.to_str().unwrap()),
        None,
        &["Enter passphrase for key /home/user/.ssh/id_ed25519:"],
    );

    // Step 4: Agent reads stdout
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        expected,
        "agent should receive the cached passphrase via stdout"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "stderr should be empty"
    );

    let _ = std::fs::remove_file(&cache_path);
}

// ── Trailing newline in cached passphrase ───────────────────────────
// ssh-agent may write the passphrase with a trailing newline.
// The binary should output it verbatim (print! doesn't add extra).

#[test]
fn test_trailing_newline_preserved() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-trailing-nl");
    std::fs::write(&path, "passphrase_with_newline\n").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    // stdout should be exactly "passphrase_with_newline\n" (no extra newline added)
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "passphrase_with_newline\n"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_multiple_trailing_newlines() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-multi-nl");
    std::fs::write(&path, "pass\n\n\n").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pass\n\n\n");
    let _ = std::fs::remove_file(&path);
}

// ── Leading/trailing whitespace ─────────────────────────────────────
// The binary preserves whitespace exactly as written in the cache file.

#[test]
fn test_leading_whitespace_preserved() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-leading-ws");
    std::fs::write(&path, "  leading-spaces").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "  leading-spaces"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_trailing_whitespace_preserved() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-trailing-ws");
    std::fs::write(&path, "trailing-spaces  ").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "trailing-spaces  "
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_tabs_preserved() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-tabs");
    std::fs::write(&path, "pass\tword").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pass\tword");
    let _ = std::fs::remove_file(&path);
}

// ── Binary / null bytes ─────────────────────────────────────────────
// Cache files could theoretically contain binary data.

#[test]
fn test_null_byte_in_cache() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-null");
    std::fs::write(&path, b"pass\0word").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    let stdout = output.stdout;
    assert_eq!(&stdout, b"pass\0word");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_non_utf8_cache_falls_through() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-nonutf8");
    // Byte 0xFF is never valid in any UTF-8 sequence
    std::fs::write(&path, b"\xff\xfe\x00").unwrap();

    let _output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    // Falls through to dialog (which fails without display)
    // Process must not hang or crash with signal
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_utf8_with_invalid_continuation_falls_through() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-badutf8");
    // 0x80 alone is an invalid continuation byte
    let mut data: Vec<u8> = b"valid_ascii".to_vec();
    data.push(0x80);
    std::fs::write(&path, &data).unwrap();

    let _output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    let _ = std::fs::remove_file(&path);
}

// ── Very large passphrases ──────────────────────────────────────────

#[test]
fn test_large_passphrase_4kb() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-4kb");
    let passphrase = "x".repeat(4 * 1024);
    std::fs::write(&path, &passphrase).unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), passphrase);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_large_passphrase_64kb() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-64kb");
    let passphrase = "y".repeat(64 * 1024);
    std::fs::write(&path, &passphrase).unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), passphrase);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_large_passphrase_1mb() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-1mb");
    let passphrase = "z".repeat(1024 * 1024);
    std::fs::write(&path, &passphrase).unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), passphrase);
    let _ = std::fs::remove_file(&path);
}

// ── Cache file is a directory ───────────────────────────────────────
// OO7_PASSPHRASE_READ_FILE pointing to a directory should not crash.

#[test]
fn test_cache_path_is_directory() {
    let dir = std::env::temp_dir();
    let dir_path = dir.join("oo7-test-is-dir");
    std::fs::create_dir_all(&dir_path).unwrap();

    let _output = run_askpass(Some(dir_path.to_str().unwrap()), None, &[]);
    // Falls through to dialog; process must not crash
    std::fs::remove_dir_all(&dir_path).unwrap();
}

// ── Multiple cache files (OO7_PASSPHRASE_READ_FILE/WRITE_FILE) ──────
// OO7_PASSPHRASE_WRITE_FILE pointing to a directory.

#[test]
fn test_write_path_is_directory() {
    let dir = std::env::temp_dir();
    let read_path = dir.join("oo7-test-write-dir-read");
    let write_dir = dir.join("oo7-test-write-dir-target");
    std::fs::write(&read_path, "pass").unwrap();
    std::fs::create_dir_all(&write_dir).unwrap();

    let _output = run_askpass(
        Some(read_path.to_str().unwrap()),
        Some(write_dir.to_str().unwrap()),
        &[],
    );
    // Process must not crash
    let _ = std::fs::remove_file(&read_path);
    std::fs::remove_dir_all(&write_dir).unwrap();
}

// ── Unreadable cache file (permissions) ─────────────────────────────

#[test]
fn test_cache_file_no_permissions() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-no-perm");
    // A previous run of this test could have panicked (or been killed)
    // between chmod-ing this file to 0o000 and restoring it, leaving a
    // permission-locked file behind that a plain write can no longer
    // overwrite. Removing it first (permitted regardless of the file's own
    // mode, since it only requires write access to the containing
    // directory) makes the test self-healing instead of permanently red.
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "secret").unwrap();

    // Make file unreadable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    }

    let _output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    // Falls through to dialog; process must not crash

    #[cfg(unix)]
    {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let _ = std::fs::remove_file(&path);
}

// ── Concurrent cache reads ──────────────────────────────────────────
// Two askpass processes reading the same cache file simultaneously.

#[test]
fn test_concurrent_cache_reads() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-concurrent");
    std::fs::write(&path, "concurrent_pass").unwrap();

    let path_str = path.to_str().unwrap().to_string();

    let handles: Vec<_> = (0..5)
        .map(|_| {
            let p = path_str.clone();
            std::thread::spawn(move || {
                let output = run_askpass(Some(&p), None, &[]);
                assert!(output.status.success());
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout).trim(),
                    "concurrent_pass"
                );
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let _ = std::fs::remove_file(&path);
}

// ── Cache with only whitespace ──────────────────────────────────────
// A cache file containing only whitespace is not empty, so it's not
// treated as "cache miss" — it would be returned as-is.

#[test]
fn test_cache_whitespace_only() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-whitespace-only");
    std::fs::write(&path, "   ").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "   ");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_cache_newlines_only() {
    let dir = std::env::temp_dir();
    let path = dir.join("oo7-test-newlines-only");
    std::fs::write(&path, "\n\n\n").unwrap();

    let output = run_askpass(Some(path.to_str().unwrap()), None, &[]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "\n\n\n");
    let _ = std::fs::remove_file(&path);
}

// ── D-Bus Secret Service tests ────────────────────────────────────
// These tests verify the oo7 D-Bus backend stores and retrieves
// passphrases. They require an unlocked D-Bus Secret Service daemon
// (gnome-keyring-daemon, oo7-daemon, etc.) running on the session bus.
// If unavailable, they skip gracefully.
//
// IMPORTANT: These tests share the same D-Bus session collection and
// MUST run serially (--test-threads=1). The session collection is
// cleaned between tests via dbus_delete in each test's setup, but
// parallel execution would still cause attribute-based search races.

use cosmic_passphrase_core::cache::CacheBackend;
use cosmic_ssh_askpass::cache_key_for_prompt;

/// Path of the on-disk retry-counter file for a given cache key, mirroring
/// `cosmic-ssh-askpass`'s own `retry_counter_path`. Every test that drives a
/// cache *hit* through `run_askpass` causes one of these to be written under
/// `$XDG_RUNTIME_DIR` (or `/tmp` as a fallback) — tests must remove it
/// afterward so a leftover count doesn't affect a later run reusing the same
/// cache key.
fn retry_file_path(cache_key: &str) -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("cosmic-passphrase-retry")
        .join(cache_key)
}

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
    let coll = match get_unlocked_collection().await {
        Some(c) => c,
        None => return false,
    };
    // Delete first to avoid item accumulation (the oo7-daemon's
    // CreateItem with replace=true may not find existing items
    // via SearchItems, similar to the read_alias bug).
    dbus_delete_with_collection(&coll, key).await;
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

async fn dbus_delete_with_collection(coll: &oo7::dbus::Collection, key: &str) {
    if let Ok(items) = coll
        .search_items(&[("application", "cosmic-passphrase"), ("key", key)])
        .await
    {
        for item in &items {
            let _ = item.delete(None).await;
        }
    }
}

async fn dbus_delete(key: &str) {
    if let Some(coll) = get_unlocked_collection().await {
        dbus_delete_with_collection(&coll, key).await;
    }
}

#[test]
fn test_dbus_cache_read_ssh_passphrase() {
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt = "Enter passphrase for test key:";
    let cache_key = cache_key_for_prompt(prompt);
    let retry_file = retry_file_path(&cache_key);
    let _ = std::fs::remove_file(&retry_file);

    tokio::runtime::Runtime::new().unwrap().block_on(dbus_delete(&cache_key));
    assert!(
        tokio::runtime::Runtime::new().unwrap().block_on(dbus_store(&cache_key, "dbus_ssh_pass")),
        "failed to store in D-Bus"
    );

    let output = run_askpass(None, None, &[prompt]);
    assert!(
        output.status.success(),
        "askpass should exit 0 when reading from D-Bus cache"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "dbus_ssh_pass",
        "should output the passphrase stored in D-Bus"
    );

    let _ = std::fs::remove_file(&retry_file);
    tokio::runtime::Runtime::new().unwrap().block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_ssh_retry_counter_accumulates_and_clears_after_limit() {
    // After MAX_CACHE_RETRIES (3) cache hits, the entry is deleted from
    // D-Bus and the binary falls through to the dialog.
    // This test verifies the first 3 hits, counter file, and cache
    // state. Exhausting the limit triggers the dialog (needs display).
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt = "Enter passphrase for retry-limit-test:";
    let cache_key = cache_key_for_prompt(prompt);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    let retry_file = retry_file_path(&cache_key);
    let _ = std::fs::remove_file(&retry_file);

    assert!(
        rt.block_on(dbus_store(&cache_key, "retry_limit_pass")),
        "failed to store in D-Bus"
    );

    for i in 1..=3 {
        let output = run_askpass(None, None, &[prompt]);
        assert!(output.status.success(), "attempt {} failed", i);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "retry_limit_pass",
            "attempt {} should return cached", i
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
        "cache should still exist during retry window"
    );
    assert!(
        retry_file.exists(),
        "retry counter file should exist after 3 hits"
    );

    // 4th call exceeds limit → deletes cache, shows dialog
    // Without display, dialog hangs — so we set the counter past the
    // limit and spawn with a 2s timeout. The binary should delete
    // the cache entry and retry file before attempting the dialog.
    std::fs::write(&retry_file, "99").unwrap();
    let mut child = std::process::Command::new(binary_path())
        .args([prompt])
        .env_remove("OO7_PASSPHRASE_READ_FILE")
        .env_remove("OO7_PASSPHRASE_WRITE_FILE")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn askpass");
    // Give it a brief moment to process cache deletion before dialog
    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = child.kill();
    let _ = child.wait();

    let items = rt.block_on(async {
        let coll = get_unlocked_collection().await?;
        coll.search_items(&[("application", "cosmic-passphrase"), ("key", &cache_key)])
            .await
            .ok()
    });
    assert!(
        items.is_none_or(|v| v.is_empty()),
        "cache should be deleted after retry limit"
    );
    assert!(
        !retry_file.exists(),
        "retry counter file should be cleaned up"
    );

    let _ = std::fs::remove_file(&retry_file);
    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_ssh_repeated_calls_same_prompt_return_same_value() {
    // SSH agent asks the same prompt multiple times (e.g., after wrong
    // passphrase, or just across separate unrelated sessions reusing a
    // still-correct passphrase). Within MAX_CACHE_RETRIES (3) rapid hits,
    // the cached value is returned every time — there is no SETERROR-style
    // feedback in SSH to signal the passphrase was ever wrong, so eviction
    // is a last resort, not proof of failure.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt = "Enter passphrase for repeated-key:";
    let cache_key = cache_key_for_prompt(prompt);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    // Reset any retry-counter state left over from a previous run of this
    // test (or any other test using this exact cache key) — otherwise a
    // leftover count >= MAX_CACHE_RETRIES makes attempt 0 below evict
    // immediately and fall through to the (display-less) dialog.
    let retry_file = retry_file_path(&cache_key);
    let _ = std::fs::remove_file(&retry_file);

    assert!(
        rt.block_on(dbus_store(&cache_key, "repeated_wrong_pass")),
        "failed to store in D-Bus"
    );

    for i in 0..3 {
        let output = run_askpass(None, None, &[prompt]);
        assert!(
            output.status.success(),
            "attempt {} should succeed", i
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "repeated_wrong_pass",
            "attempt {} should return cached passphrase", i
        );
    }

    let _ = std::fs::remove_file(&retry_file);
    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_ssh_stale_retry_count_is_not_evicted() {
    // A retry count sitting at/above MAX_CACHE_RETRIES from a stale,
    // long-past window (e.g. yesterday) must NOT evict a still-valid cached
    // passphrase — only a *rapid* run of repeated calls is treated as
    // evidence the passphrase might be wrong. Regression test for the fix
    // to the eviction heuristic (previously any 4th use of a correct cached
    // passphrase, no matter how spread out in time, was silently evicted).
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt = "Enter passphrase for stale-retry-test:";
    let cache_key = cache_key_for_prompt(prompt);
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(dbus_delete(&cache_key));
    assert!(
        rt.block_on(dbus_store(&cache_key, "still_valid_pass")),
        "failed to store in D-Bus"
    );

    let retry_file = retry_file_path(&cache_key);
    if let Some(parent) = retry_file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    // Count already at the limit, but timestamped a full day in the past.
    let one_day_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(24 * 60 * 60);
    std::fs::write(&retry_file, format!("99 {one_day_ago}")).unwrap();

    let output = run_askpass(None, None, &[prompt]);
    assert!(output.status.success(), "should still return the cached value");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "still_valid_pass",
        "a stale retry count must not evict a still-correct cached passphrase"
    );

    let _ = std::fs::remove_file(&retry_file);
    rt.block_on(dbus_delete(&cache_key));
}

#[test]
fn test_dbus_cache_ssh_different_prompts_independent_cache_keys() {
    // Different prompts must yield different cache keys and not interfere.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt_a = "Enter passphrase for key alpha:";
    let prompt_b = "Enter passphrase for key beta:";
    let key_a = cache_key_for_prompt(prompt_a);
    let key_b = cache_key_for_prompt(prompt_b);
    let retry_file_a = retry_file_path(&key_a);
    let retry_file_b = retry_file_path(&key_b);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let _ = std::fs::remove_file(&retry_file_a);
    let _ = std::fs::remove_file(&retry_file_b);
    rt.block_on(dbus_delete(&key_a));
    rt.block_on(dbus_delete(&key_b));

    assert!(rt.block_on(dbus_store(&key_a, "pass_for_alpha")));
    assert!(rt.block_on(dbus_store(&key_b, "pass_for_beta")));

    let out_a = run_askpass(None, None, &[prompt_a]);
    assert_eq!(
        String::from_utf8_lossy(&out_a.stdout).trim(),
        "pass_for_alpha"
    );

    let out_b = run_askpass(None, None, &[prompt_b]);
    assert_eq!(
        String::from_utf8_lossy(&out_b.stdout).trim(),
        "pass_for_beta"
    );

    let _ = std::fs::remove_file(&retry_file_a);
    let _ = std::fs::remove_file(&retry_file_b);
    rt.block_on(dbus_delete(&key_a));
    rt.block_on(dbus_delete(&key_b));
}

#[test]
fn test_dbus_cache_ssh_falls_through_when_missing() {
    // When no passphrase is in D-Bus, the binary falls through to the
    // dialog. Without a display, the dialog call will fail / return
    // empty. The test just verifies the process doesn't crash or hang.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no unlocked D-Bus Secret Service");
        return;
    }

    let prompt = "Enter passphrase for nonexistent key:";
    let cache_key = cache_key_for_prompt(prompt);

    // Ensure no item exists
    tokio::runtime::Runtime::new().unwrap().block_on(dbus_delete(&cache_key));

    let output = run_askpass(None, None, &[prompt]);
    // Falls through to dialog -> fails without display -> process exits
    // (no crash, no hang)
    let _ = output;
}


#[test]
fn test_dbus_backend_direct_read_write() {
    // This test uses DbusBackend directly (no subprocess) to verify
    // that the cache backend works correctly.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no D-Bus secret service");
        return;
    }

    let backend = cosmic_passphrase_core::cache::DbusBackend::new();
    let key = "ssh:test_direct_key";

    // Delete first
    backend.delete(key);

    // Store
    backend.store(key, "direct_test_value", "test label", None);

    // Read back
    let result = backend.read(key);
    eprintln!("Direct read result: {:?}", result);
    assert_eq!(
        result.as_deref().map(|s| s.as_str()),
        Some("direct_test_value"),
        "should read back directly stored value"
    );

    // Now read via a SECOND DbusBackend instance (like subprocess would)
    let backend2 = cosmic_passphrase_core::cache::DbusBackend::new();
    let result2 = backend2.read(key);
    eprintln!("Second backend read result: {:?}", result2);
    assert_eq!(
        result2.as_deref().map(|s| s.as_str()),
        Some("direct_test_value"),
        "second backend should also read the value"
    );

    // Clean up
    backend.delete(key);
}

#[test]
fn test_dbus_backend_stores_in_persistent_collection() {
    // Regression test for the hardening in DbusBackend::get_or_init_collection:
    // it must not just pick "whichever collection enumerates first" (which
    // could be a transient, in-memory-only collection like the freedesktop
    // "session" collection), but must prefer a collection labeled "Login"
    // (or "login"/"Default"/"default") when one exists and is unlocked,
    // since that's the disk-backed keyring that survives logout/reboot.
    if !dbus_secret_service_available() {
        eprintln!("SKIP: no D-Bus secret service");
        return;
    }

    const PERSISTENT_LABELS: &[&str] = &["Login", "login", "Default", "default"];

    let rt = tokio::runtime::Runtime::new().unwrap();
    let has_persistent_collection = rt.block_on(async {
        let Ok(svc) = oo7::dbus::Service::new().await else {
            return false;
        };
        let Ok(collections) = svc.collections().await else {
            return false;
        };
        for coll in collections {
            let Ok(label) = coll.label().await else { continue };
            if PERSISTENT_LABELS.contains(&label.as_str())
                && matches!(coll.is_locked().await, Ok(false))
            {
                return true;
            }
        }
        false
    });
    if !has_persistent_collection {
        eprintln!("SKIP: no unlocked collection labeled one of {PERSISTENT_LABELS:?} on this system");
        return;
    }

    let key = "ssh:test_persistent_collection_key";
    let backend = cosmic_passphrase_core::cache::DbusBackend::new();
    backend.delete(key);
    backend.store(key, "persistent_test_value", "test label", None);

    // Find which collection actually holds the item, independent of
    // DbusBackend, by searching every collection directly.
    let found_in_persistent = rt.block_on(async {
        let svc = oo7::dbus::Service::new().await.unwrap();
        for coll in svc.collections().await.unwrap() {
            let label = coll.label().await.unwrap_or_default();
            if !PERSISTENT_LABELS.contains(&label.as_str()) {
                continue;
            }
            let items = coll
                .search_items(&[("application", "cosmic-passphrase"), ("key", key)])
                .await
                .unwrap_or_default();
            if !items.is_empty() {
                return true;
            }
        }
        false
    });

    backend.delete(key);

    assert!(
        found_in_persistent,
        "item should have been stored in a collection labeled one of {PERSISTENT_LABELS:?}, not a transient one"
    );
}
