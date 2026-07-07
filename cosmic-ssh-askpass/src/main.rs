use cosmic_passphrase_core::cache::{CacheBackend, DbusBackend};
use cosmic_passphrase_core::config::{DialogConfig, ExtraContent};
use cosmic_passphrase_core::output::DialogOutput;
use cosmic_passphrase_dialog::run_dialog;
use cosmic_ssh_askpass::{
    cache_key_for_prompt, clear_retry_count, decide_retry, label_for_prompt, now_secs,
    read_retry_state, write_retry_state, RetryDecision,
};

/// Shown in place of the "Remember passphrase" checkbox when the keyring
/// backend isn't currently usable (e.g. the Secret Service collection is
/// locked) — so "Remember" doesn't just silently do nothing.
const CACHE_UNAVAILABLE_HINT: &str =
    "Note: the system keyring is currently locked, so this passphrase can't be remembered right now.";

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

    // Look up a cached entry (if any) but don't act on it yet — it's
    // offered as a choice within the one dialog shown below ("Use Saved
    // Passphrase"), rather than auto-returned silently or gated behind a
    // separate confirm dialog shown first (which would mean *two* dialogs
    // in this process — see DialogConfig::offer_cached's docs for why
    // that's not just a style choice).
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

    let cache_available = cache.is_available();
    let config = DialogConfig {
        title: String::from("Unlock SSH Key"),
        description: (!cache_available).then(|| CACHE_UNAVAILABLE_HINT.to_string()),
        prompt,
        ok_label: String::from("Unlock"),
        extra: if cache_available { ExtraContent::Remember } else { ExtraContent::None },
        offer_cached: cached_passphrase.is_some(),
        ..Default::default()
    };

    let result = run_dialog(config);

    if result.use_cached
        && let Some(cached) = cached_passphrase
    {
        print!("{}", cached.as_str());
        std::process::exit(0);
    }

    // Declined ("Unlock" with a typed passphrase, or Cancel), or there was
    // nothing cached to offer in the first place: any prior cache entry
    // shouldn't be treated as stale just because it wasn't picked this
    // time, so the retry counter is left as `decide_retry` set it above.
    clear_retry_count(&cache_key);

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
