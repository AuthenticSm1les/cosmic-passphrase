# Architecture

## Crate layout

```
cosmic-passphrase-core     headless: CacheBackend trait + backends,
                            DialogConfig, DialogOutput. No GUI dependency.
        ^
        |  DialogConfig in, DialogOutput out — the only coupling
        |
cosmic-passphrase-dialog   the one crate depending on libcosmic/iced/wgpu.
                            run_dialog(DialogConfig) -> DialogOutput.
        ^
        |
   +----+----+
   |         |
pinentry-cosmic   cosmic-ssh-askpass
(bin only)        (bin + lib: cache_key_for_prompt, label_for_prompt
                   are unit-tested directly and shared with integration
                   tests, instead of being hand-duplicated per test file)
```

`cosmic-passphrase-core` and `cosmic-passphrase-dialog` are split
deliberately: anything that only needs the caching/config logic — unit
tests, a future headless tool, CI — builds against `cosmic-passphrase-core`
alone in seconds, without pulling in `iced`/`wgpu`/`winit`/X11/Wayland
client libraries. Only `cosmic-passphrase-dialog`, and anything that links
it, pays that build cost. This was verified directly: a clean build of
`cosmic-passphrase-core` alone touches zero GUI crates.

## The two frontends

### `pinentry-cosmic` — GPG

Implements the [Assuan pinentry protocol](https://www.gnupg.org/documentation/manuals/assuan/)
over stdin/stdout, invoked by `gpg-agent` as its `pinentry-program`. Runs a
simple synchronous loop: read a line, parse it (`assuan.rs`), mutate
`PinentryState`, respond.

**Passphrase caching flow (`GETPIN`):**

1. `gpg-agent` sends `OPTION allow-external-password-cache` (on by default —
   there's only a `--no-allow-external-cache` to turn it off) and
   `SETKEYINFO <flag>/<keygrip>` before `GETPIN`.
2. If caching is allowed and there's no pending `SETERROR`, `pinentry-cosmic`
   looks up `cache.read("gpg:<keygrip>")` *before* showing any dialog — but
   doesn't act on a hit yet. A hit is offered via a dedicated Allow/Deny
   consent dialog (`ask_use_cached()`) before anything is handed back; see
   "Consent before using a cached passphrase" below for why.
3. A miss (or a prior `SETERROR`, which explicitly evicts first — or Deny on
   the consent dialog) shows the normal COSMIC entry dialog. If the user
   checks "Remember" and submits a typed passphrase, it is *not* written to
   the keyring yet — see "Deferred GPG cache commit" below.

**Deferred GPG cache commit — don't cache a possibly-wrong passphrase:**

Assuan has no "the passphrase you gave me was correct" message. The only
signal is negative: if it was wrong, gpg-agent sends `SETERROR <message>`
before asking again with another `GETPIN`. Earlier, this crate cached
eagerly on submission and relied on `SETERROR` to evict — which meant a
*wrong* passphrase sat in the persistent keyring, readable by anything that
can talk to the Secret Service, for the (possibly long) window between
submission and the next `GETPIN`.

Instead, a freshly-typed, "Remember"-checked passphrase is held in memory
only (`PinentryState.pending_cache_key` / `pending_passphrase`, zeroized on
drop) and committed to the keyring (`commit_pending()`) only once
gpg-agent implicitly confirms it was right — i.e. the *next* `GETPIN`
arrives with no `SETERROR` pending, or the session ends (`BYE`/EOF) without
one. If `SETERROR` does arrive first, the pending passphrase is dropped
instead of committed. A wrong passphrase is now never written to the
persistent keyring at all, not even transiently.

**Consent before using a cached passphrase:**

A cache hit is no longer silently piped back to gpg-agent/ssh-agent — a
dedicated dialog (`DialogMode::Confirm`, titled "Passphrase Request",
`ok_label: "Allow"` / `cancel_label: "Deny"`) is shown first, naming what's
being accessed: e.g. `gpg-agent wants to access the saved passphrase: "GPG
key passphrase (<keygrip>)".` for GPG, `ssh-agent wants to access the saved
passphrase: "<label>".` for SSH. **Allow** hands the cached value straight
back, exactly as before. **Deny** falls through to the normal entry dialog
— a *second* `run_dialog()` call in the same process — without evicting the
cache entry; declining once isn't evidence it's wrong, only a confirmed
failure is (see the GPG/SSH-specific eviction rules above and below). This
two-dialog design is only safe because of the child-process delegation
described in "One event loop per process" below — an earlier version of
this feature embedded the choice as a button in the entry dialog itself
specifically to avoid a second `run_dialog()` call, before that mechanism
existed.

**The `SETKEYINFO` wire-format detail (a real bug found and fixed here):**
real gpg-agent does **not** send a bare keygrip. It sends `<flag>/<keygrip>`,
e.g. `n/4CB13907FA13F63A8CE699C494B5774EB96A9CC7` — a single-letter flag
reflecting gpg-agent's own, unrelated in-memory cache state, then a slash,
then the keygrip. `keyinfo_cache_id()` in `pinentry-cosmic/src/main.rs`
strips that prefix before it's used as a cache key. Before this fix, the
cache key silently included the flag, so a stored entry never matched a
lookup, and the effective key changed depending on gpg-agent's own state —
**the GPG-side oo7 caching never actually worked against a real gpg-agent**,
only against this crate's own synthetic tests (which fed a bare keygrip).
This is why testing against the real protocol, not just a hand-written
Assuan session, mattered.

### `cosmic-ssh-askpass` — SSH

An `$SSH_ASKPASS` helper (the same shape as GNOME's own `gcr-ssh-askpass`):
invoked by a real `ssh-agent`/`ssh-add`, not a replacement for one. OpenSSH
gives it only a free-text prompt on argv — no stable key identifier — so
the cache key has to be derived from that prompt.

**Cache key derivation (`cache_key_for_prompt` in `cosmic-ssh-askpass/src/lib.rs`):**

OpenSSH prompts typically embed the key's file path, e.g.
`"Enter passphrase for /home/user/.ssh/id_ed25519: "` (confirmed live
against a real `ssh-add`) or, in other versions, `"...for key
'/path': "`. `stable_prompt_id()` extracts just that path-like substring —
preferring quoted content, else everything from the first `/` onward,
trimming trailing punctuation — before hashing it with `hash_key` (a
from-scratch FNV-1a implementation, not `std`'s `DefaultHasher`, whose docs
explicitly disclaim algorithm stability across Rust releases). This means
the cache key survives OpenSSH wording/locale changes as long as the key's
path doesn't move. Prompts with no path at all (including this crate's own
synthetic test prompts) fall back to hashing the whole prompt, unchanged
from the original behavior.

**Retry/eviction heuristic:** SSH gives askpass no success/failure signal —
`ssh-add` just calls it again with the same prompt if the passphrase was
wrong. `MAX_CACHE_RETRIES` (3) rapid hits are tolerated before the entry is
evicted, but the retry counter is time-windowed (`RETRY_WINDOW_SECS = 30`,
stored alongside the count as `"<count> <unix-timestamp>"` in
`$XDG_RUNTIME_DIR/cosmic-passphrase-retry/<key>`): if more than 30s have
passed since the last hit, the count resets rather than carrying forward.
Without this, a *correct* cached passphrase reused across separate
sessions/reboots would get silently evicted on exactly its 4th use, with no
way to tell "wrong 3 times" from "right 3 times" — this was reproduced and
confirmed via a flaky-test investigation before being fixed. This
bookkeeping is a pure function, `decide_retry()` in
`cosmic-ssh-askpass/src/lib.rs`, unit-tested directly (including a
50-iteration stress test asserting a correctly-reused passphrase, spaced
out across the retry window, is never evicted) without needing a
subprocess or display at all.

Because SSH has no success/failure signal at all — unlike GPG's
`SETERROR` — a wrong passphrase can't be detected and evicted immediately;
`decide_retry`'s time-windowed retry counter (above) is the mitigation on
the eviction side, and requiring explicit Allow/Deny consent before a
cached passphrase is even offered (see `pinentry-cosmic`'s "Consent before
using a cached passphrase," which applies identically here via the same
`ask_use_cached()` pattern) is the mitigation on the "don't act on
stale/wrong data silently" side. **Deliberately not** implemented: deleting
the entry immediately the first time it's denied or the prompt reappears.
Unlike GPG, SSH has no way to tell "denied/reused because it was wrong"
from "asked again for an unrelated reason" (a second key, a fresh
connection, `ssh-add -c` re-confirming each use) — immediate eviction was
tried in an earlier design and reproduced exactly the false-eviction bug
`decide_retry`'s time window exists to prevent (see above). GPG gets
immediate, deterministic eviction on `SETERROR`; SSH gets the tolerant
heuristic instead, and that's a deliberate asymmetry, not an oversight.

## Keyring-locked UX

Both frontends call `CacheBackend::is_available()` (for `DbusBackend`, a
live check via `get_or_init_collection()`) before building the dialog. If
the keyring isn't currently reachable — e.g. the Secret Service collection
is locked — the "Remember" checkbox is omitted entirely rather than shown
and silently doing nothing, and a note is appended to the dialog
description explaining that the passphrase can't be remembered right now.

## One event loop per process

`cosmic::app::run()` (via `winit`) can only create one event loop per
process, ever: `winit`'s Wayland backend guards `EventLoop::new()` with a
process-wide `static AtomicBool`, and every call after the first returns
`EventLoopError::RecreationAttempt` — confirmed by reproducing the panic
against a real display, not just inferred from reading the source. This is
not a hypothetical edge case: it breaks the pre-existing GPG retry flow
(`SETERROR` → `GETPIN` again, i.e. a *second* dialog in the same
`pinentry-cosmic` process) just as much as it would break a naive
two-dialog consent flow.

`cosmic-passphrase-dialog::run_dialog()` handles this with a per-process
budget: the first call in a process runs the dialog directly
(`run_dialog_in_process`); every call after that re-execs
`std::env::current_exe()` as a child with a marker environment variable set
(`run_dialog_in_child`), sending the `DialogConfig` as JSON over the
child's stdin and reading a `DialogOutput` back as JSON over its stdout. A
fresh process has a fresh (unused) event-loop budget, so the child's own
first (and only) `run_dialog()` call runs in-process as normal.
`maybe_run_as_dialog_child()` — called as literally the first line of both
binaries' `main()` — detects the marker, runs that one dialog, writes the
result, and exits before reaching any Assuan/askpass logic, so a
dialog-child process never does anything else. The wire format is a
private implementation detail of `cosmic-passphrase-dialog` (not exposed
from `cosmic-passphrase-core`) and carries only what a dialog needs — never
argv or environment variables for the passphrase itself, since both are
readable via `/proc/<pid>/cmdline` and `/proc/<pid>/environ` by anything
with `ptrace` access to the user; the passphrase only ever crosses the pipe.

This is exactly what makes the Allow/Deny consent dialog above viable at
all: on Deny it falls through to a second, real `run_dialog()` call in the
same process to show normal entry — on every single cache hit, not just
the rare GPG retry-after-`SETERROR` case this mechanism was originally
built for. An earlier iteration of the consent feature avoided a second
`run_dialog()` call altogether by embedding the choice as an extra button
in the entry dialog itself, specifically to sidestep this limitation before
the child-process fallback existed; once the fallback existed and was
proven live, that workaround was no longer needed and was replaced with the
more direct two-dialog design described above.

**Child crashes are retried once, and fail safe.** Live testing (rapidly
showing six dialogs in a row through this mechanism) reproduced, once, a
child process dying with `SIGSEGV` right after a `Bad file descriptor`
Wayland I/O error — not reproduced again across a dozen further attempts,
consistent with a rare compositor-side race on reconnect rather than a
logic bug in this crate. `run_dialog_in_child` retries the spawn+run once
on a crash or spawn failure before giving up. Either way — first attempt,
retry, or final give-up — a failure is always surfaced as
`DialogOutput::cancelled()`, the same as the user explicitly closing the
window; it is never treated as confirmed, and the child's crash can't
leak or fabricate a passphrase, only fail to produce one.

## The shared cache backend (`cosmic-passphrase-core::cache`)

`DbusBackend` is the only real implementation of `CacheBackend`, talking to
whatever implements the freedesktop Secret Service D-Bus API
(`org.freedesktop.secrets`) — `oo7-daemon` on this system, but it would work
identically against `gnome-keyring-daemon`.

- **Connection**: bypasses `Service::default_collection()`/`with_alias()`,
  which call `ReadAlias` — confirmed to be broken in `oo7-daemon` (returns
  `/` for every alias, triggering a `CreateCollection` fallback that panics
  the daemon). Instead enumerates all collections directly.
- **Collection selection**: prefers a collection labeled `Login`/`login`/
  `Default`/`default` (`PERSISTENT_COLLECTION_LABELS`) over "whichever
  collection enumerates first" — the freedesktop `session` collection is
  typically transient/in-memory and doesn't survive logout or reboot, and
  relying on undocumented enumeration order to avoid landing there was a
  latent risk. Falls back to first-unlocked (with a logged warning) if no
  known-persistent label is found, rather than failing outright.
- **Failure caching**: only a *successful* collection lookup is cached in
  the backend's `OnceLock`; a failed attempt is not, so one transient D-Bus
  hiccup doesn't permanently disable caching for the rest of a process's
  (short) lifetime.
- **Item labels**: `store()` takes a human-readable label (e.g. `"SSH key
  passphrase (/home/user/.ssh/id_ed25519)"`), so items are recognizable in
  `seahorse`/`secret-tool` instead of an opaque hash.

## Persistence

Passphrases stored in the `Login` collection land in
`~/.local/share/keyrings/v1/login.keyring` — the same on-disk,
GNOME-Keyring-compatible file format (confirmed via its file header magic
bytes) that `oo7-daemon`'s PAM module (`pam_oo7.so`) auto-unlocks at login
using your login password, the same way `gnome-keyring`'s
`pam_gnome_keyring.so` always has. This is what makes the cache survive a
reboot without a separate "unlock keyring" step.
