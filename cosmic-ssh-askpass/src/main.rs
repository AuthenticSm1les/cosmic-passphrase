use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use cosmic_passphrase_core::cache::{CacheBackend, DbusBackend};
use cosmic_passphrase_core::config::{DialogConfig, ExtraContent};
use cosmic_passphrase_core::output::DialogOutput;
use cosmic_passphrase_dialog::run_dialog;
use cosmic_ssh_askpass::{cache_key_for_prompt, label_for_prompt};

const MAX_CACHE_RETRIES: usize = 3;

// SSH gives askpass no success/failure signal, so repeated calls for the
// same prompt are the only clue that a cached passphrase might be wrong.
// That signal is only meaningful when the calls happen in rapid succession
// (the same ssh-add/ssh retrying immediately after a bad passphrase).
// Spread out over minutes/hours/days, repeated calls just mean the cached
// passphrase is being reused across separate, unrelated sessions and is
// presumably still correct. If more than this many seconds have passed
// since the last cache hit, the retry count is treated as reset rather than
// carried forward — otherwise a perfectly valid passphrase gets silently
// evicted (and the user re-prompted) after exactly MAX_CACHE_RETRIES uses.
const RETRY_WINDOW_SECS: u64 = 30;

fn retry_counter_path(cache_key: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("cosmic-passphrase-retry").join(cache_key)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads `(retry count, last-bumped timestamp)`. The timestamp is missing
/// for a freshly-created file (or a legacy plain-count file); that's treated
/// as "unknown age" and does not reset the count, preserving the original
/// eviction behavior when age can't be determined.
fn read_retry_state(cache_key: &str) -> (usize, Option<u64>) {
    let path = retry_counter_path(cache_key);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (0, None);
    };
    let mut parts = content.split_whitespace();
    let count = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let last_used = parts.next().and_then(|p| p.parse().ok());
    (count, last_used)
}

fn write_retry_state(cache_key: &str, count: usize, now: u64) {
    let path = retry_counter_path(cache_key);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{count} {now}"));
}

fn clear_retry_count(cache_key: &str) {
    let path = retry_counter_path(cache_key);
    let _ = std::fs::remove_file(&path);
}

fn main() {
    let cache = DbusBackend::new();

    if let Ok(path) = std::env::var("OO7_PASSPHRASE_READ_FILE") {
        match std::fs::read_to_string(&path) {
            Ok(passphrase) if !passphrase.is_empty() => {
                print!("{}", passphrase);
                return;
            }
            _ => {}
        }
    }

    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Enter passphrase:".to_string());

    let cache_key = cache_key_for_prompt(&prompt);
    let label = label_for_prompt(&prompt);

    if let Some(cached) = cache.read(&cache_key) {
        let now = now_secs();
        let (mut retries, last_used) = read_retry_state(&cache_key);
        let stale = last_used.is_some_and(|last| now.saturating_sub(last) > RETRY_WINDOW_SECS);
        if stale {
            retries = 0;
        }
        if retries >= MAX_CACHE_RETRIES {
            cache.delete(&cache_key);
            clear_retry_count(&cache_key);
        } else {
            write_retry_state(&cache_key, retries + 1, now);
            print!("{}", cached.as_str());
            return;
        }
    }

    let config = DialogConfig {
        title: String::from("Unlock SSH Key"),
        prompt,
        ok_label: String::from("Unlock"),
        extra: ExtraContent::Remember,
        ..Default::default()
    };

    clear_retry_count(&cache_key);
    let result = run_dialog(config);

    match result {
        DialogOutput { passphrase: Some(ref p), .. } if !p.is_empty() => {
            print!("{}", p.as_str());

            if result.remember {
                cache.store(&cache_key, p.as_str(), &label, None);
            }

            if result.remember
                && let Ok(path) = std::env::var("OO7_PASSPHRASE_WRITE_FILE")
                && let Ok(mut f) = std::fs::File::create(&path)
            {
                use std::io::Write;
                let _ = f.write_all(p.as_bytes());
            }

            std::process::exit(0);
        }
        _ => {
            std::process::exit(1);
        }
    }
}
