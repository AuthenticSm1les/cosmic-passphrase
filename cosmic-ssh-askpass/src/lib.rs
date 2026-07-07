//! Library portion of `cosmic-ssh-askpass`.
//!
//! Split out from `main.rs` so the oo7 cache-key derivation can be
//! unit-tested directly (no subprocess spawn needed) and shared verbatim
//! with integration tests, instead of every test file hand-duplicating the
//! same `format!("ssh:{}", hash_key(prompt))` expression production code
//! uses — which is exactly how a previous version of this logic and its
//! tests silently drifted apart.

use cosmic_passphrase_core::cache::hash_key;

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
fn stable_prompt_id(prompt: &str) -> &str {
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
}
