use cosmic_passphrase_core::cache::{CacheBackend, DbusBackend};
use cosmic_passphrase_core::config::{DialogConfig, DialogMode, ExtraContent};
use cosmic_passphrase_core::output::DialogOutput;
use cosmic_passphrase_dialog::run_dialog;
use cosmic_ssh_askpass::{
    cache_key_for_prompt, clear_retry_count, decide_retry, label_for_prompt, now_secs,
    read_retry_state, stable_prompt_id, write_retry_state, RetryDecision,
};

/// Shown in place of the "Remember passphrase" checkbox when the keyring
/// backend isn't currently usable (e.g. the Secret Service collection is
/// locked) — so "Remember" doesn't just silently do nothing.
const CACHE_UNAVAILABLE_HINT: &str =
    "Note: the system keyring is currently locked, so this passphrase can't be remembered right now.";

/// Asks the user, via a dedicated Allow/Deny dialog, whether to hand a
/// cached passphrase back to ssh-agent/ssh-add. A cache hit is never used
/// silently — this is always shown first, and returns `true` only if the
/// user explicitly picked Allow. Safe to call even when a dialog has
/// already been shown earlier in this process: see
/// `cosmic-passphrase-dialog`'s module docs for why a second `run_dialog`
/// call in the same process is not the winit hazard it looks like.
fn ask_use_cached(prompt: &str) -> bool {
    let config = DialogConfig {
        title: String::from("Passphrase Request"),
        description: Some(format!(
            "ssh-agent wants to use your saved passphrase for {}.",
            stable_prompt_id(prompt)
        )),
        prompt: String::new(),
        ok_label: String::from("Allow"),
        cancel_label: String::from("Deny"),
        mode: DialogMode::Confirm,
        ..Default::default()
    };
    run_dialog(config).confirmed
}

fn main() {
    // Must run before anything else — see cosmic-passphrase-dialog's module
    // docs for why a dialog-only child process can exist at all here.
    cosmic_passphrase_dialog::maybe_run_as_dialog_child();

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

    // OpenSSH's askpass protocol gives no success/failure signal at all —
    // unlike GPG's SETERROR, there's no deterministic way to tell "the
    // cached passphrase was wrong" from "ssh-add asked again for some
    // unrelated reason". `decide_retry`'s time-windowed tolerance (not an
    // immediate evict) exists specifically to avoid wrongly evicting a
    // *correct* passphrase that's just been reused — see
    // `cosmic-ssh-askpass::decide_retry`'s docs and tests. That heuristic
    // is unchanged here; only the presentation of a hit (Allow/Deny,
    // below) is new.
    let mut cached_passphrase = None;
    if let Some(cached) = cache.read(&cache_key) {
        let (retries, last_used) = read_retry_state(&cache_key);
        let now = now_secs();
        match decide_retry(retries, last_used, now) {
            RetryDecision::Evict => {
                cache.delete(&cache_key);
                clear_retry_count(&cache_key);
            }
            RetryDecision::Serve { new_count } => {
                write_retry_state(&cache_key, new_count, now);
                cached_passphrase = Some(cached);
            }
        }
    }

    // A cache hit is never handed back silently — ask first, via a
    // dedicated Allow/Deny dialog (safe as a second `run_dialog` call in
    // this process). Declining falls through to normal manual entry below
    // rather than evicting the entry — the retry counter above already
    // tracks repeated hits, so a single Deny isn't treated as "wrong".
    if let Some(cached) = cached_passphrase
        && ask_use_cached(&prompt)
    {
        print!("{}", cached.as_str());
        std::process::exit(0);
    }

    // No cache hit, or the user denied it: any prior cache entry shouldn't
    // be treated as stale just because it wasn't picked this time, so the
    // retry counter is left as `decide_retry` set it above until a manual
    // entry below actually clears it.
    clear_retry_count(&cache_key);

    let cache_available = cache.is_available();
    let config = DialogConfig {
        title: String::from("Unlock SSH Key"),
        description: (!cache_available).then(|| CACHE_UNAVAILABLE_HINT.to_string()),
        prompt,
        ok_label: String::from("Unlock"),
        extra: if cache_available { ExtraContent::Remember } else { ExtraContent::None },
        ..Default::default()
    };

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
