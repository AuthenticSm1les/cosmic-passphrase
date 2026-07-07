//! Library portion of `cosmic-ssh-askpass`.
//!
//! Split out from `main.rs` so the oo7 cache-key derivation can be
//! unit-tested directly (no subprocess spawn needed) and shared verbatim
//! with integration tests, instead of every test file hand-duplicating the
//! same `format!("ssh:{}", hash_key(prompt))` expression production code
//! uses — which is exactly how a previous version of this logic and its
//! tests silently drifted apart.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cosmic_passphrase_core::cache::hash_key;

/// How many rapid reuses of a cached passphrase are tolerated before it's
/// evicted. SSH gives askpass no success/failure signal, so repeated calls
/// for the same prompt are the only clue a cached passphrase might be
/// wrong — see [`decide_retry`].
pub const MAX_CACHE_RETRIES: usize = 3;

/// If more than this many seconds have passed since a cache entry's retry
/// counter was last bumped, the count is treated as reset rather than
/// carried forward — see [`decide_retry`].
pub const RETRY_WINDOW_SECS: u64 = 30;

pub fn retry_counter_path(cache_key: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("cosmic-passphrase-retry").join(cache_key)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads `(retry count, last-bumped timestamp)`. The timestamp is missing
/// for a freshly-created file (or a legacy plain-count file); that's
/// treated as "unknown age" and does not reset the count, preserving the
/// original eviction behavior when age can't be determined.
pub fn read_retry_state(cache_key: &str) -> (usize, Option<u64>) {
    let path = retry_counter_path(cache_key);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (0, None);
    };
    let mut parts = content.split_whitespace();
    let count = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let last_used = parts.next().and_then(|p| p.parse().ok());
    (count, last_used)
}

pub fn write_retry_state(cache_key: &str, count: usize, now: u64) {
    let path = retry_counter_path(cache_key);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{count} {now}"));
}

pub fn clear_retry_count(cache_key: &str) {
    let path = retry_counter_path(cache_key);
    let _ = std::fs::remove_file(&path);
}

/// What to do with a cache hit, given its retry bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Serve the cached value; persist this new count afterward.
    Serve { new_count: usize },
    /// Too many rapid reuses without a fresh entry — evict instead of
    /// serving, and let the caller fall through to a fresh dialog.
    Evict,
}

/// Pure decision logic for whether a cache hit should be served or evicted,
/// kept separate from *how* a hit is served (which now also requires user
/// consent via an Allow/Deny dialog — see `ask_use_cached` in `main.rs`) so
/// it can be unit-tested directly, without spawning a subprocess or a
/// dialog.
///
/// If more than `RETRY_WINDOW_SECS` have passed since `last_used`, `retries`
/// is treated as reset to 0 first — rapid reuse is the only available signal
/// that a passphrase might be wrong, so reuse spread out over time (across
/// separate, unrelated sessions) must not count against it.
pub fn decide_retry(retries: usize, last_used: Option<u64>, now: u64) -> RetryDecision {
    let stale = last_used.is_some_and(|last| now.saturating_sub(last) > RETRY_WINDOW_SECS);
    let effective = if stale { 0 } else { retries };
    if effective >= MAX_CACHE_RETRIES {
        RetryDecision::Evict
    } else {
        RetryDecision::Serve { new_count: effective + 1 }
    }
}

/// Derives the oo7 cache key used to store/fetch a passphrase for a given
/// `$SSH_ASKPASS` prompt string.
///
/// OpenSSH gives askpass helpers only a free-text prompt, no stable key
/// identifier — typically embedding the key's file path, e.g.
/// `"Enter passphrase for /home/user/.ssh/id_ed25519: "` (confirmed live
/// against a real `ssh-add`) or, in other OpenSSH versions/code paths,
/// `"Enter passphrase for key '/home/user/.ssh/id_ed25519': "`. Hashing the
/// *entire* prompt sentence means any change to the surrounding wording — a
/// new OpenSSH release, a different locale — silently orphans every
/// previously-cached passphrase, even though the key itself hasn't moved.
/// This extracts just the path-like substring first, so the cache key
/// stays stable as long as the key file's path doesn't change.
pub fn cache_key_for_prompt(prompt: &str) -> String {
    format!("ssh:{}", hash_key(stable_prompt_id(prompt)))
}

/// A short, human-readable label for the Secret Service item storing this
/// prompt's passphrase — what someone browsing `seahorse`/`secret-tool`
/// sees, instead of an opaque hash. Uses the same extracted identifier as
/// [`cache_key_for_prompt`], so it shows the key's path when one can be
/// found in the prompt.
pub fn label_for_prompt(prompt: &str) -> String {
    format!("SSH key passphrase ({})", stable_prompt_id(prompt))
}

/// Extracts the most likely stable identifier from an askpass prompt: the
/// key's file path, if one can be found in it. Falls back to the whole,
/// unmodified prompt when no path-like substring is found, so prompts with
/// no discernible path (including the synthetic, path-free prompts this
/// crate's own tests use) hash exactly as they always have.
///
/// Public so `main.rs`'s Allow/Deny consent prompt (`ask_use_cached`) can
/// show just the bare path, rather than the fuller `label_for_prompt`
/// wrapper text — the consent dialog wants to be short.
pub fn stable_prompt_id(prompt: &str) -> &str {
    // Prefer content inside single quotes, e.g. "Enter passphrase for key
    // '/home/user/.ssh/id_ed25519': " — the quotes unambiguously delimit
    // the path even if it happens to contain spaces.
    if let Some(start) = prompt.find('\'')
        && let Some(end) = prompt[start + 1..].find('\'')
    {
        return &prompt[start + 1..start + 1 + end];
    }

    // Otherwise, take from the first '/' onward and trim the trailing
    // punctuation OpenSSH tends to append, e.g.
    // "Enter passphrase for /home/user/.ssh/id_ed25519: ".
    if let Some(start) = prompt.find('/') {
        return prompt[start..].trim_end_matches([':', ' ', '\t']);
    }

    // No path-like substring found; hash the whole prompt, as before.
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stable_prompt_id_extracts_plain_path() {
        assert_eq!(
            stable_prompt_id("Enter passphrase for /home/user/.ssh/id_ed25519: "),
            "/home/user/.ssh/id_ed25519"
        );
    }

    #[test]
    fn test_stable_prompt_id_extracts_quoted_path() {
        assert_eq!(
            stable_prompt_id("Enter passphrase for key '/home/user/.ssh/id_rsa': "),
            "/home/user/.ssh/id_rsa"
        );
    }

    #[test]
    fn test_stable_prompt_id_quoted_path_with_space() {
        // Quotes unambiguously delimit the path even if it contains spaces.
        assert_eq!(
            stable_prompt_id("Enter passphrase for key '/home/user/my keys/id_rsa': "),
            "/home/user/my keys/id_rsa"
        );
    }

    #[test]
    fn test_stable_prompt_id_falls_back_without_path() {
        assert_eq!(
            stable_prompt_id("Enter passphrase for test key:"),
            "Enter passphrase for test key:"
        );
    }

    #[test]
    fn test_cache_key_for_prompt_stable_across_wording_changes() {
        // Same underlying key path, different surrounding wording (as if
        // it changed between OpenSSH releases) — must hash identically.
        let a = cache_key_for_prompt("Enter passphrase for /home/user/.ssh/id_ed25519: ");
        let b = cache_key_for_prompt("Enter passphrase for key '/home/user/.ssh/id_ed25519': ");
        assert_eq!(
            a, b,
            "cache key should be stable across prompt wording changes for the same key path"
        );
    }

    #[test]
    fn test_cache_key_for_prompt_differs_for_different_paths() {
        let a = cache_key_for_prompt("Enter passphrase for /home/user/.ssh/id_ed25519: ");
        let b = cache_key_for_prompt("Enter passphrase for /home/user/.ssh/id_rsa: ");
        assert_ne!(a, b);
    }

    #[test]
    fn test_cache_key_for_prompt_no_path_hashes_whole_prompt() {
        let direct = format!("ssh:{}", hash_key("Enter passphrase for test key:"));
        assert_eq!(
            cache_key_for_prompt("Enter passphrase for test key:"),
            direct
        );
    }

    #[test]
    fn test_cache_key_for_prompt_has_ssh_prefix() {
        assert!(cache_key_for_prompt("anything").starts_with("ssh:"));
    }

    #[test]
    fn test_label_for_prompt_includes_path() {
        assert_eq!(
            label_for_prompt("Enter passphrase for /home/user/.ssh/id_ed25519: "),
            "SSH key passphrase (/home/user/.ssh/id_ed25519)"
        );
    }

    #[test]
    fn test_label_for_prompt_falls_back_to_whole_prompt() {
        assert_eq!(
            label_for_prompt("Enter passphrase for test key:"),
            "SSH key passphrase (Enter passphrase for test key:)"
        );
    }

    // ── decide_retry ────────────────────────────────────────────────

    #[test]
    fn test_decide_retry_fresh_serves_and_bumps_to_one() {
        assert_eq!(
            decide_retry(0, None, 1000),
            RetryDecision::Serve { new_count: 1 }
        );
    }

    #[test]
    fn test_decide_retry_below_limit_serves() {
        assert_eq!(
            decide_retry(MAX_CACHE_RETRIES - 1, Some(1000), 1000),
            RetryDecision::Serve { new_count: MAX_CACHE_RETRIES }
        );
    }

    #[test]
    fn test_decide_retry_at_limit_within_window_evicts() {
        assert_eq!(
            decide_retry(MAX_CACHE_RETRIES, Some(1000), 1000 + RETRY_WINDOW_SECS),
            RetryDecision::Evict
        );
    }

    #[test]
    fn test_decide_retry_at_limit_but_stale_resets_and_serves() {
        // Same count that would evict within the window, but enough time
        // has passed since the last hit that it's treated as a fresh,
        // unrelated reuse rather than a rapid failing retry.
        let now = 1000 + RETRY_WINDOW_SECS + 1;
        assert_eq!(
            decide_retry(MAX_CACHE_RETRIES, Some(1000), now),
            RetryDecision::Serve { new_count: 1 }
        );
    }

    #[test]
    fn test_decide_retry_exactly_at_window_boundary_still_counts() {
        // Exactly RETRY_WINDOW_SECS elapsed is not yet "stale" (> is used,
        // not >=), so the count still carries forward.
        let now = 1000 + RETRY_WINDOW_SECS;
        assert_eq!(
            decide_retry(MAX_CACHE_RETRIES, Some(1000), now),
            RetryDecision::Evict
        );
    }

    #[test]
    fn test_decide_retry_no_timestamp_never_resets() {
        // Missing timestamp (fresh/legacy file) means "unknown age" — never
        // treated as stale, so a count already at the limit still evicts.
        assert_eq!(decide_retry(MAX_CACHE_RETRIES, None, 999_999), RetryDecision::Evict);
    }

    #[test]
    fn test_decide_retry_never_evicts_a_correct_passphrase_reused_across_sessions() {
        // The scenario this whole mechanism exists to protect: a correct
        // passphrase, reused once per (well-spaced-out) session, must never
        // reach the eviction threshold no matter how many times it's used,
        // as long as each reuse is outside the rapid-retry window.
        let mut retries = 0usize;
        let mut last_used = None;
        let mut now = 0u64;
        for _ in 0..50 {
            now += RETRY_WINDOW_SECS + 1;
            match decide_retry(retries, last_used, now) {
                RetryDecision::Serve { new_count } => {
                    retries = new_count;
                    last_used = Some(now);
                }
                RetryDecision::Evict => panic!("a correctly-reused passphrase must never be evicted just for being reused many times, spaced out"),
            }
        }
    }
}
