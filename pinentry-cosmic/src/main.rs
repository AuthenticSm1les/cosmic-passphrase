use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::process;

use cosmic_passphrase_core::cache::{CacheBackend, DbusBackend};
use cosmic_passphrase_core::config::{DialogConfig, DialogMode, ExtraContent};
use cosmic_passphrase_core::output::DialogOutput;
use cosmic_passphrase_dialog::run_dialog;
use zeroize::Zeroizing;

mod assuan;
mod error;

use assuan::{parse_command, write_data, write_err, write_ok, write_status, Command};
use error::gpg;

const TIMEOUT_SECS: u64 = 120;

fn main() {
    // Must run before anything else: if this process was spawned by
    // run_dialog() as a dialog-only child (see cosmic-passphrase-dialog's
    // module docs — winit only allows one event loop per process, so a
    // pinentry session needing a second dialog, e.g. a retry after
    // SETERROR, delegates it to a fresh child process instead), this
    // shows that one dialog and exits, never reaching the Assuan loop
    // below at all.
    cosmic_passphrase_dialog::maybe_run_as_dialog_child();

    let cache = DbusBackend::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    let _ = writeln!(writer, "OK pleased to meet you -- pinentry-cosmic");
    let _ = writer.flush();

    let mut state = PinentryState::new();

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // stdin closed with no BYE and no SETERROR for a pending
                // passphrase in between — the best signal of success this
                // protocol gives us. Commit it before exiting.
                commit_pending(&mut state, &cache);
                break;
            }
            Ok(_) => {
                let cmd = parse_command(&line);
                handle_command(&mut writer, &mut state, cmd, &cache);
            }
            Err(e) => {
                let _ = write_err(&mut writer, gpg::ASS_GENERAL, &format!("read error: {}", e));
                let _ = writer.flush();
                process::exit(1);
            }
        }
        let _ = writer.flush();
    }
}

struct PinentryState {
    title: Option<String>,
    description: Option<String>,
    error: Option<String>,
    prompt: Option<String>,
    ok_label: Option<String>,
    cancel_label: Option<String>,
    notok_label: Option<String>,
    repeat_passphrase: bool,
    repeat_error: Option<String>,
    repeat_ok: Option<String>,
    quality_bar_label: Option<String>,
    quality_bar_tooltip: Option<String>,
    genpin_label: Option<String>,
    genpin_tooltip: Option<String>,
    keyinfo: Option<String>,
    timeout: Option<u64>,
    grab_keyboard: bool,
    allow_external_password_cache: bool,
    parent_wid: Option<String>,
    display: Option<String>,
    ttyname: Option<String>,
    ttytype: Option<String>,
    lc_ctype: Option<String>,
    lc_messages: Option<String>,
    constraints_enforce: bool,
    constraints_hint_short: Option<String>,
    constraints_hint_long: Option<String>,
    constraints_error_title: Option<String>,
    touch_file: Option<String>,
    /// A passphrase the user just entered and asked to remember, held here
    /// rather than written to the persistent cache immediately — we have no
    /// way to know it's actually correct until gpg-agent's *next* move (see
    /// `commit_pending`/the top of `Command::GetPin`). Deliberately *not*
    /// cleared by `reset_for_request`: it must survive until the following
    /// request resolves it.
    pending_cache_key: Option<String>,
    pending_passphrase: Option<Zeroizing<String>>,
}

impl PinentryState {
    fn new() -> Self {
        Self {
            title: None,
            description: None,
            error: None,
            prompt: None,
            ok_label: None,
            cancel_label: None,
            notok_label: None,
            repeat_passphrase: false,
            repeat_error: None,
            repeat_ok: None,
            quality_bar_label: None,
            quality_bar_tooltip: None,
            genpin_label: None,
            genpin_tooltip: None,
            keyinfo: None,
            timeout: None,
            grab_keyboard: false,
            allow_external_password_cache: false,
            parent_wid: None,
            display: None,
            ttyname: None,
            ttytype: None,
            lc_ctype: None,
            lc_messages: None,
            constraints_enforce: false,
            constraints_hint_short: None,
            constraints_hint_long: None,
            constraints_error_title: None,
            touch_file: None,
            pending_cache_key: None,
            pending_passphrase: None,
        }
    }

    fn reset_for_request(&mut self) {
        self.description = None;
        self.error = None;
        self.prompt = None;
        self.ok_label = None;
        self.cancel_label = None;
        self.notok_label = None;
        self.repeat_passphrase = false;
        self.repeat_error = None;
        self.repeat_ok = None;
        self.quality_bar_label = None;
        self.quality_bar_tooltip = None;
        self.genpin_label = None;
        self.genpin_tooltip = None;
        self.keyinfo = None;
    }

    fn full_reset(&mut self) {
        *self = Self::new();
    }
}

fn handle_command(writer: &mut impl Write, state: &mut PinentryState, cmd: Command, cache: &impl CacheBackend) {
    match cmd {
        Command::Option_(name, value) => {
            apply_option(state, &name, value.as_deref());
            let _ = write_ok(writer, "");
        }
        Command::SetTitle(text) => {
            state.title = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetDesc(text) => {
            state.description = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetPrompt(text) => {
            state.prompt = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetError(text) => {
            state.error = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetOk(text) => {
            state.ok_label = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetCancel(text) => {
            state.cancel_label = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetNotOk(text) => {
            state.notok_label = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetRepeat => {
            state.repeat_passphrase = true;
            let _ = write_ok(writer, "");
        }
        Command::SetRepeatError(text) => {
            state.repeat_error = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetRepeatOk(text) => {
            state.repeat_ok = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetQualityBar(text) => {
            state.quality_bar_label = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetQualityBarTt(text) => {
            state.quality_bar_tooltip = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetGenPin(text) => {
            state.genpin_label = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetGenPinTt(text) => {
            state.genpin_tooltip = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::SetKeyInfo(text) => {
            state.keyinfo = none_if_empty(text);
            let _ = write_ok(writer, "");
        }
        Command::GetPin => {
            // Resolve whatever we were still holding from a previous
            // GETPIN before doing anything else. gpg-agent resends
            // SETKEYINFO/SETERROR/etc. before every retry, so by the time
            // we're here `state.error` tells us whether the passphrase we
            // tentatively held last time actually worked.
            if state.error.is_some() {
                // It was wrong. It was never written to the persistent
                // cache, so a wrong passphrase never touches the keyring
                // at all — drop it. (Zeroizing scrubs the memory on drop.)
                state.pending_cache_key = None;
                state.pending_passphrase = None;
            } else {
                // Nothing complained about it — gpg-agent must have
                // accepted it (or moved on to something unrelated).
                commit_pending(state, cache);
            }

            // Look up a cached entry (if any) but don't act on it yet — see
            // `DialogConfig::offer_cached`'s docs: it's offered as a choice
            // within the *one* dialog shown below ("Use Saved Passphrase"),
            // not auto-returned or gated behind a separate confirm dialog
            // shown first, since a second `run_dialog` call in this same
            // process would panic (winit permits only one event loop per
            // process — see cosmic-passphrase-dialog's module docs).
            let mut cached_passphrase = None;
            if let Some(keygrip) = state.keyinfo.as_deref().map(keyinfo_cache_id)
                .filter(|_| state.allow_external_password_cache && state.error.is_none())
            {
                let cache_key = format!("gpg:{}", keygrip);
                cached_passphrase = cache.read(&cache_key);
            }
            if let Some(keygrip) = state.keyinfo.as_deref().map(keyinfo_cache_id)
                .filter(|_| state.allow_external_password_cache && state.error.is_some())
            {
                let cache_key = format!("gpg:{}", keygrip);
                cache.delete(&cache_key);
            }

            let mut config = build_config(state, cache.is_available());
            config.offer_cached = cached_passphrase.is_some();
            let result = run_dialog(config);

            if result.use_cached
                && let Some(cached) = cached_passphrase
            {
                let _ = write_data(writer, cached.as_str());
                let _ = write_ok(writer, "");
                touch_file_if_needed(state);
                state.reset_for_request();
                return;
            }

            if let Some(keygrip) = state.keyinfo.as_deref().map(keyinfo_cache_id)
                .filter(|_| state.allow_external_password_cache && result.remember)
                && let Some(ref p) = result.passphrase
                && !p.is_empty()
            {
                // Not cached yet — see the pending-resolution block at the
                // top of this handler and `commit_pending`.
                state.pending_cache_key = Some(format!("gpg:{}", keygrip));
                state.pending_passphrase = Some(Zeroizing::new(p.as_str().to_string()));
            }
            handle_passphrase_result(writer, &result);
            touch_file_if_needed(state);
            state.reset_for_request();
        }
        Command::Confirm { one_button } => {
            let mut config = build_config(state, cache.is_available());
            config.mode = DialogMode::Confirm;
            if one_button {
                config.notok_label = None;
            }
            let result = run_dialog(config);
            handle_confirm_result(writer, &result);
            touch_file_if_needed(state);
            state.reset_for_request();
        }
        Command::Message(text) => {
            let mut config = build_config(state, cache.is_available());
            config.mode = DialogMode::Message;
            config.description = Some(text);
            config.prompt = String::new();
            let _ = run_dialog(config);
            let _ = write_ok(writer, "");
            touch_file_if_needed(state);
            state.reset_for_request();
        }
        Command::Reset => {
            state.full_reset();
            let _ = write_ok(writer, "");
        }
        Command::Bye => {
            // Session ending with no SETERROR for a pending passphrase in
            // between — commit it (see `commit_pending`).
            commit_pending(state, cache);
            let _ = write_ok(writer, "closing connection");
            let _ = writer.flush();
            process::exit(0);
        }
        Command::Nop => {
            let _ = write_ok(writer, "");
        }
        Command::Unknown(_cmd) => {
            let _ = write_ok(writer, "");
        }
        Command::End | Command::Cancel | Command::Data(_) => {
            let _ = write_err(writer, gpg::ASS_GENERAL, "unexpected command in current state");
        }
    }
}

/// Commits a still-pending (not-yet-confirmed) passphrase to the cache, if
/// there is one. Called when we have reason to believe it worked: a fresh
/// `GETPIN` with no `SETERROR` in between, or the session ending (`BYE` /
/// stdin EOF) without one.
fn commit_pending(state: &mut PinentryState, cache: &impl CacheBackend) {
    let (Some(pending_key), Some(pending_pass)) =
        (state.pending_cache_key.take(), state.pending_passphrase.take())
    else {
        return;
    };
    let label = format!(
        "GPG key passphrase ({})",
        pending_key.strip_prefix("gpg:").unwrap_or(&pending_key)
    );
    cache.store(&pending_key, pending_pass.as_str(), &label, None);
}

/// Text shown in place of the "Remember passphrase" checkbox when caching
/// would otherwise apply but the keyring backend isn't currently usable
/// (e.g. the Secret Service collection is locked) — so "Remember" doesn't
/// just silently do nothing with no explanation.
const CACHE_UNAVAILABLE_HINT: &str =
    "Note: the system keyring is currently locked, so this passphrase can't be remembered right now.";

fn build_config(state: &PinentryState, cache_available: bool) -> DialogConfig {
    let extra = if state.repeat_passphrase {
        ExtraContent::Repeat
    } else if state.allow_external_password_cache && cache_available {
        ExtraContent::Remember
    } else {
        ExtraContent::None
    };

    let mut description = state.description.clone();
    if state.allow_external_password_cache && !cache_available && matches!(extra, ExtraContent::None) {
        description = Some(match description {
            Some(d) if !d.is_empty() => format!("{d}\n\n{CACHE_UNAVAILABLE_HINT}"),
            _ => CACHE_UNAVAILABLE_HINT.to_string(),
        });
    }

    DialogConfig {
        title: state.title.clone().unwrap_or_else(|| String::from("Passphrase Required")),
        description,
        error: state.error.clone(),
        prompt: state.prompt.clone().unwrap_or_else(|| String::from("Passphrase:")),
        ok_label: state.ok_label.clone().unwrap_or_else(|| String::from("OK")),
        cancel_label: state.cancel_label.clone().unwrap_or_else(|| String::from("Cancel")),
        notok_label: state.notok_label.clone(),
        mode: DialogMode::Passphrase,
        extra,
        timeout: state.timeout.map(|s| std::time::Duration::from_secs(s.min(TIMEOUT_SECS))),
        // Set by the GETPIN handler itself once it knows whether there's a
        // cached entry to offer; every other caller of build_config
        // (CONFIRM, MESSAGE) has no use for it.
        offer_cached: false,
    }
}

fn apply_option(state: &mut PinentryState, name: &str, value: Option<&str>) {
    match name {
        "timeout" => {
            state.timeout = value.and_then(|v| v.parse::<u64>().ok());
        }
        "grab" => state.grab_keyboard = true,
        "no-grab" => state.grab_keyboard = false,
        "allow-external-password-cache" => {
            state.allow_external_password_cache = true;
        }
        "parent-wid" => state.parent_wid = value.map(String::from),
        "display" => state.display = value.map(String::from),
        "ttyname" => state.ttyname = value.map(String::from),
        "ttytype" => state.ttytype = value.map(String::from),
        "ttyalert" => {}
        "lc-ctype" => state.lc_ctype = value.map(String::from),
        "lc-messages" => state.lc_messages = value.map(String::from),
        "constraints-enforce" => state.constraints_enforce = true,
        "constraints-hint-short" => {
            state.constraints_hint_short = value.map(String::from);
        }
        "constraints-hint-long" => {
            state.constraints_hint_long = value.map(String::from);
        }
        "constraints-error-title" => {
            state.constraints_error_title = value.map(String::from);
        }
        "touch-file" => state.touch_file = value.map(String::from),
        "default-ok" => {
            if state.ok_label.is_none() {
                state.ok_label = value.map(String::from);
            }
        }
        "default-cancel" => {
            if state.cancel_label.is_none() {
                state.cancel_label = value.map(String::from);
            }
        }
        "default-prompt" => {
            if state.prompt.is_none() {
                state.prompt = value.map(String::from);
            }
        }
        "default-pwmngr" => {}
        "default-cf-visi" => {}
        "default-tt-visi" => {}
        "default-tt-hide" => {}
        "default-capshint" => {}
        "invisible-char" => {}
        "flavor" => {
            let _ = write_status(
                &mut io::stdout().lock(),
                "FLAVOR",
                "cosmic",
            );
        }
        _other => {}
    }
}

fn none_if_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

/// Extracts the stable cache identifier from a `SETKEYINFO` argument.
///
/// Real gpg-agent does not send a bare keygrip: it sends `<flag>/<cacheid>`,
/// e.g. `n/4CB13907FA13F63A8CE699C494B5774EB96A9CC7`, where the single-letter
/// flag reflects gpg-agent's own (unrelated) in-memory cache state for that
/// key and can change between calls. Using the raw string as the cache key
/// meant the oo7-backed cache key changed depending on that flag and never
/// matched what was actually stored — GETPIN cache hits never fired against
/// real gpg-agent. Only the part after the first `/` is a stable identifier;
/// if there's no `/` at all (e.g. a bare keygrip, as used in this crate's own
/// tests), the whole string is used as-is.
fn keyinfo_cache_id(raw: &str) -> &str {
    raw.split_once('/').map_or(raw, |(_, id)| id)
}

fn handle_passphrase_result(writer: &mut impl Write, result: &DialogOutput) {
    match result {
        r if r.confirmed => {
            if let Some(ref passphrase) = r.passphrase {
                let _ = write_data(writer, passphrase.as_str());
            }
            let _ = write_ok(writer, "");
        }
        r if r.cancelled => {
            let _ = write_err(writer, gpg::CANCELED, "canceled");
        }
        _ => {
            let _ = write_err(writer, gpg::CANCELED, "canceled");
        }
    }
}

fn handle_confirm_result(writer: &mut impl Write, result: &DialogOutput) {
    match result {
        r if r.confirmed => {
            let _ = write_ok(writer, "");
        }
        r if !r.confirmed && !r.cancelled => {
            let _ = write_err(writer, gpg::NOT_CONFIRMED, "not confirmed");
        }
        _ => {
            let _ = write_err(writer, gpg::CANCELED, "canceled");
        }
    }
}

fn touch_file_if_needed(state: &PinentryState) {
    if let Some(ref path) = state.touch_file {
        let _ = fs::write(path, "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_passphrase_core::cache::NullBackend;

    // ── none_if_empty ────────────────────────────────────────────────

    #[test]
    fn test_none_if_empty_returns_none() {
        assert!(none_if_empty(String::new()).is_none());
    }

    // ── keyinfo_cache_id ────────────────────────────────────────────

    #[test]
    fn test_keyinfo_cache_id_strips_real_gpg_agent_flag_prefix() {
        // Real gpg-agent sends "<flag>/<keygrip>", e.g. "n/<keygrip>" when
        // it has nothing cached, or "c/<keygrip>" when it does. Only the
        // part after the slash is the stable, cacheable identifier.
        assert_eq!(
            keyinfo_cache_id("n/4CB13907FA13F63A8CE699C494B5774EB96A9CC7"),
            "4CB13907FA13F63A8CE699C494B5774EB96A9CC7"
        );
        assert_eq!(
            keyinfo_cache_id("c/4CB13907FA13F63A8CE699C494B5774EB96A9CC7"),
            "4CB13907FA13F63A8CE699C494B5774EB96A9CC7"
        );
    }

    #[test]
    fn test_keyinfo_cache_id_falls_back_to_whole_string_without_slash() {
        // Defensive fallback for a bare keygrip with no flag prefix.
        assert_eq!(keyinfo_cache_id("ABCD1234"), "ABCD1234");
    }

    #[test]
    fn test_none_if_empty_returns_some() {
        assert_eq!(none_if_empty("hello".into()), Some("hello".into()));
    }

    #[test]
    fn test_none_if_empty_whitespace_not_empty() {
        assert_eq!(none_if_empty(" ".into()), Some(" ".into()));
    }

    // ── apply_option ─────────────────────────────────────────────────

    #[test]
    fn test_apply_option_timeout() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "timeout", Some("30"));
        assert_eq!(state.timeout, Some(30));
    }

    #[test]
    fn test_apply_option_timeout_invalid() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "timeout", Some("not-a-number"));
        assert!(state.timeout.is_none());
    }

    #[test]
    fn test_apply_option_timeout_no_value() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "timeout", None);
        assert!(state.timeout.is_none());
    }

    #[test]
    fn test_apply_option_grab() {
        let mut state = PinentryState::new();
        assert!(!state.grab_keyboard);
        apply_option(&mut state, "grab", None);
        assert!(state.grab_keyboard);
    }

    #[test]
    fn test_apply_option_no_grab() {
        let mut state = PinentryState::new();
        state.grab_keyboard = true;
        apply_option(&mut state, "no-grab", None);
        assert!(!state.grab_keyboard);
    }

    #[test]
    fn test_apply_option_allow_external_password_cache() {
        let mut state = PinentryState::new();
        assert!(!state.allow_external_password_cache);
        apply_option(&mut state, "allow-external-password-cache", None);
        assert!(state.allow_external_password_cache);
    }

    #[test]
    fn test_apply_option_touch_file() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "touch-file", Some("/tmp/test-touch"));
        assert_eq!(state.touch_file, Some("/tmp/test-touch".into()));
    }

    #[test]
    fn test_apply_option_parent_wid() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "parent-wid", Some("0x12345"));
        assert_eq!(state.parent_wid, Some("0x12345".into()));
    }

    #[test]
    fn test_apply_option_display() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "display", Some(":0"));
        assert_eq!(state.display, Some(":0".into()));
    }

    #[test]
    fn test_apply_option_ttyname_ttytype() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "ttyname", Some("/dev/pts/1"));
        apply_option(&mut state, "ttytype", Some("xterm-256color"));
        assert_eq!(state.ttyname, Some("/dev/pts/1".into()));
        assert_eq!(state.ttytype, Some("xterm-256color".into()));
    }

    #[test]
    fn test_apply_option_locale() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "lc-ctype", Some("en_US.UTF-8"));
        apply_option(&mut state, "lc-messages", Some("de_DE.UTF-8"));
        assert_eq!(state.lc_ctype, Some("en_US.UTF-8".into()));
        assert_eq!(state.lc_messages, Some("de_DE.UTF-8".into()));
    }

    #[test]
    fn test_apply_option_constraints() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "constraints-enforce", None);
        apply_option(&mut state, "constraints-hint-short", Some("min 8 chars"));
        apply_option(&mut state, "constraints-hint-long", Some("must contain uppercase"));
        apply_option(&mut state, "constraints-error-title", Some("Weak Passphrase"));
        assert!(state.constraints_enforce);
        assert_eq!(state.constraints_hint_short, Some("min 8 chars".into()));
        assert_eq!(state.constraints_hint_long, Some("must contain uppercase".into()));
        assert_eq!(state.constraints_error_title, Some("Weak Passphrase".into()));
    }

    #[test]
    fn test_apply_option_default_ok_sets_when_empty() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "default-ok", Some("Continue"));
        assert_eq!(state.ok_label, Some("Continue".into()));
    }

    #[test]
    fn test_apply_option_default_ok_does_not_override() {
        let mut state = PinentryState::new();
        state.ok_label = Some("Unlock".into());
        apply_option(&mut state, "default-ok", Some("Continue"));
        assert_eq!(state.ok_label, Some("Unlock".into()));
    }

    #[test]
    fn test_apply_option_default_cancel() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "default-cancel", Some("Skip"));
        assert_eq!(state.cancel_label, Some("Skip".into()));
    }

    #[test]
    fn test_apply_option_default_prompt() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "default-prompt", Some("Enter PIN:"));
        assert_eq!(state.prompt, Some("Enter PIN:".into()));
    }

    #[test]
    fn test_apply_option_unknown_is_silently_ignored() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "nonexistent-option", Some("value"));
        // state should be unchanged
        assert_eq!(state.title, None);
    }

    #[test]
    fn test_apply_option_ttyalert_ignored() {
        let mut state = PinentryState::new();
        apply_option(&mut state, "ttyalert", Some("any"));
        // no crash, no state change
    }

    // ── build_config ─────────────────────────────────────────────────

    #[test]
    fn test_build_config_default() {
        let state = PinentryState::new();
        let config = build_config(&state, true);
        assert_eq!(config.title, "Passphrase Required");
        assert_eq!(config.prompt, "Passphrase:");
        assert_eq!(config.ok_label, "OK");
        assert_eq!(config.cancel_label, "Cancel");
        assert_eq!(config.mode, DialogMode::Passphrase);
        assert_eq!(config.extra, ExtraContent::None);
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_build_config_title() {
        let mut state = PinentryState::new();
        state.title = Some("Unlock Key".into());
        let config = build_config(&state, true);
        assert_eq!(config.title, "Unlock Key");
    }

    #[test]
    fn test_build_config_description() {
        let mut state = PinentryState::new();
        state.description = Some("Enter passphrase for key".into());
        let config = build_config(&state, true);
        assert_eq!(config.description.as_deref(), Some("Enter passphrase for key"));
    }

    #[test]
    fn test_build_config_error() {
        let mut state = PinentryState::new();
        state.error = Some("Previous attempt failed".into());
        let config = build_config(&state, true);
        assert_eq!(config.error.as_deref(), Some("Previous attempt failed"));
    }

    #[test]
    fn test_build_config_prompt() {
        let mut state = PinentryState::new();
        state.prompt = Some("PIN:".into());
        let config = build_config(&state, true);
        assert_eq!(config.prompt, "PIN:");
    }

    #[test]
    fn test_build_config_labels() {
        let mut state = PinentryState::new();
        state.ok_label = Some("Yes".into());
        state.cancel_label = Some("No".into());
        state.notok_label = Some("Maybe".into());
        let config = build_config(&state, true);
        assert_eq!(config.ok_label, "Yes");
        assert_eq!(config.cancel_label, "No");
        assert_eq!(config.notok_label, Some("Maybe".into()));
    }

    #[test]
    fn test_build_config_repeat_extra() {
        let mut state = PinentryState::new();
        state.repeat_passphrase = true;
        let config = build_config(&state, true);
        assert_eq!(config.extra, ExtraContent::Repeat);
    }

    #[test]
    fn test_build_config_remember_extra() {
        let mut state = PinentryState::new();
        state.allow_external_password_cache = true;
        let config = build_config(&state, true);
        assert_eq!(config.extra, ExtraContent::Remember);
    }

    #[test]
    fn test_build_config_repeat_overrides_remember() {
        let mut state = PinentryState::new();
        state.repeat_passphrase = true;
        state.allow_external_password_cache = true;
        let config = build_config(&state, true);
        assert_eq!(config.extra, ExtraContent::Repeat, "repeat takes priority");
    }

    #[test]
    fn test_build_config_timeout_capped() {
        let mut state = PinentryState::new();
        state.timeout = Some(9999);
        let config = build_config(&state, true);
        assert_eq!(config.timeout, Some(std::time::Duration::from_secs(TIMEOUT_SECS)));
    }

    #[test]
    fn test_build_config_timeout_normal() {
        let mut state = PinentryState::new();
        state.timeout = Some(30);
        let config = build_config(&state, true);
        assert_eq!(config.timeout, Some(std::time::Duration::from_secs(30)));
    }

    #[test]
    fn test_build_config_timeout_none() {
        let state = PinentryState::new();
        let config = build_config(&state, true);
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_build_config_keyinfo() {
        let mut state = PinentryState::new();
        state.keyinfo = Some("ABCD1234".into());
        let config = build_config(&state, true);
        assert!(config.description.is_none());
    }

    // ── handle_passphrase_result ─────────────────────────────────────

    #[test]
    fn test_handle_passphrase_result_ok_with_passphrase() {
        let mut buf = Vec::new();
        let result = DialogOutput::ok(Zeroizing::new("secret".into()));
        handle_passphrase_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("D secret"));
        assert!(output.contains("OK"));
    }

    #[test]
    fn test_handle_passphrase_result_confirmed_no_passphrase() {
        let mut buf = Vec::new();
        let result = DialogOutput::confirmed();
        handle_passphrase_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("OK"));
    }

    #[test]
    fn test_handle_passphrase_result_cancelled() {
        let mut buf = Vec::new();
        let result = DialogOutput::cancelled();
        handle_passphrase_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains(&format!("ERR {}", gpg::CANCELED)));
        assert!(output.contains("canceled"));
    }

    #[test]
    fn test_handle_passphrase_result_not_confirmed() {
        let mut buf = Vec::new();
        let result = DialogOutput::not_confirmed();
        handle_passphrase_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains(&format!("ERR {}", gpg::CANCELED)));
    }

    #[test]
    fn test_handle_passphrase_result_ok_with_remember() {
        let mut buf = Vec::new();
        let result = DialogOutput::ok_remember(Zeroizing::new("p4ss".into()), true);
        handle_passphrase_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("D p4ss"));
        assert!(output.contains("OK"));
    }

    // ── handle_confirm_result ────────────────────────────────────────

    #[test]
    fn test_handle_confirm_result_ok() {
        let mut buf = Vec::new();
        let result = DialogOutput::confirmed();
        handle_confirm_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output.trim(), "OK");
    }

    #[test]
    fn test_handle_confirm_result_not_confirmed() {
        let mut buf = Vec::new();
        let result = DialogOutput::not_confirmed();
        handle_confirm_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains(&format!("ERR {}", gpg::NOT_CONFIRMED)));
    }

    #[test]
    fn test_handle_confirm_result_cancelled() {
        let mut buf = Vec::new();
        let result = DialogOutput::cancelled();
        handle_confirm_result(&mut buf, &result);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains(&format!("ERR {}", gpg::CANCELED)));
    }

    // ── touch_file_if_needed ─────────────────────────────────────────

    #[test]
    fn test_touch_file_if_needed_creates_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("pinentry-test-touch");
        let path_str = path.to_str().unwrap().to_string();

        let mut state = PinentryState::new();
        state.touch_file = Some(path_str);
        touch_file_if_needed(&state);

        assert!(path.exists(), "touch file should be created");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_touch_file_if_needed_none_does_nothing() {
        let state = PinentryState::new();
        touch_file_if_needed(&state);
    }

    // ── PinentryState reset ──────────────────────────────────────────

    #[test]
    fn test_pinentry_state_reset_for_request_clears_per_request() {
        let mut state = PinentryState::new();
        state.description = Some("desc".into());
        state.error = Some("err".into());
        state.prompt = Some("prompt".into());
        state.ok_label = Some("ok".into());
        state.cancel_label = Some("cancel".into());
        state.notok_label = Some("notok".into());
        state.repeat_passphrase = true;
        state.repeat_error = Some("mismatch".into());
        state.repeat_ok = Some("matched".into());
        state.keyinfo = Some("key".into());
        // options that should survive
        state.timeout = Some(30);
        state.grab_keyboard = true;
        state.touch_file = Some("/tmp/test".into());

        state.reset_for_request();

        assert!(state.description.is_none());
        assert!(state.error.is_none());
        assert!(state.prompt.is_none());
        assert!(state.ok_label.is_none());
        assert!(state.cancel_label.is_none());
        assert!(state.notok_label.is_none());
        assert!(!state.repeat_passphrase);
        assert!(state.repeat_error.is_none());
        assert!(state.repeat_ok.is_none());
        assert!(state.keyinfo.is_none());

        // options preserved
        assert_eq!(state.timeout, Some(30));
        assert!(state.grab_keyboard);
        assert_eq!(state.touch_file, Some("/tmp/test".into()));
    }

    #[test]
    fn test_pinentry_state_full_reset_clears_everything() {
        let mut state = PinentryState::new();
        state.title = Some("title".into());
        state.timeout = Some(60);
        state.grab_keyboard = true;
        state.allow_external_password_cache = true;
        state.touch_file = Some("/tmp/t".into());

        state.full_reset();

        assert!(state.title.is_none());
        assert!(state.timeout.is_none());
        assert!(!state.grab_keyboard);
        assert!(!state.allow_external_password_cache);
        assert!(state.touch_file.is_none());
    }

    #[test]
    fn test_pinentry_state_new_sets_defaults() {
        let state = PinentryState::new();
        assert!(state.title.is_none());
        assert!(state.description.is_none());
        assert!(!state.repeat_passphrase);
        assert!(!state.grab_keyboard);
        assert!(!state.allow_external_password_cache);
        assert!(state.timeout.is_none());
        assert!(state.touch_file.is_none());
    }

    // ── handle_command ───────────────────────────────────────────────

    #[test]
    fn test_handle_command_settitle() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::SetTitle("My Title".into()), &NullBackend);
        assert_eq!(state.title, Some("My Title".into()));
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output.trim(), "OK");
    }

    #[test]
    fn test_handle_command_setdesc() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::SetDesc("Description".into()), &NullBackend);
        assert_eq!(state.description, Some("Description".into()));
    }

    #[test]
    fn test_handle_command_setprompt() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::SetPrompt("PIN:".into()), &NullBackend);
        assert_eq!(state.prompt, Some("PIN:".into()));
    }

    #[test]
    fn test_handle_command_nop() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::Nop, &NullBackend);
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output.trim(), "OK");
    }

    #[test]
    fn test_handle_command_unknown() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::Unknown("FAKECMD".into()), &NullBackend);
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output.trim(), "OK");
    }

    #[test]
    fn test_handle_command_end_is_error() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::End, &NullBackend);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("ERR"));
    }

    #[test]
    fn test_handle_command_cancel_is_error() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::Cancel, &NullBackend);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("ERR"));
    }

    #[test]
    fn test_handle_command_data_is_error() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::Data("test".into()), &NullBackend);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("ERR"));
    }

    #[test]
    fn test_handle_command_reset() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        state.title = Some("persistent".into());
        state.description = Some("will be cleared".into());
        handle_command(&mut buf, &mut state, Command::Reset, &NullBackend);
        assert!(state.title.is_none());
        assert!(state.description.is_none());
    }

    #[test]
    fn test_handle_command_option_timeout() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(
            &mut buf,
            &mut state,
            Command::Option_("timeout".into(), Some("45".into())),
            &NullBackend,
        );
        assert_eq!(state.timeout, Some(45));
    }

    #[test]
    fn test_handle_command_setrepeat() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::SetRepeat, &NullBackend);
        assert!(state.repeat_passphrase);
    }

    #[test]
    fn test_handle_command_setkeyinfo() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(&mut buf, &mut state, Command::SetKeyInfo("KEY123".into()), &NullBackend);
        assert_eq!(state.keyinfo, Some("KEY123".into()));
    }

    #[test]
    fn test_handle_command_option_parent_wid() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        handle_command(
            &mut buf,
            &mut state,
            Command::Option_("parent-wid".into(), Some("0x123".into())),
            &NullBackend,
        );
        assert_eq!(state.parent_wid, Some("0x123".into()));
    }

    #[test]
    fn test_handle_command_empty_settitle_clears() {
        let mut buf = Vec::new();
        let mut state = PinentryState::new();
        state.title = Some("Old".into());
        handle_command(&mut buf, &mut state, Command::SetTitle(String::new()), &NullBackend);
        assert!(state.title.is_none());
    }
}
