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
   calls `cache.read("gpg:<keygrip>")` *before* showing any dialog. A hit
   returns the passphrase over Assuan immediately — no GUI involved.
3. A miss (or a prior `SETERROR`, which explicitly evicts first) shows the
   COSMIC dialog. If the user checks "Remember," `cache.store()` runs after
   they submit, labeled `"GPG key passphrase (<keygrip>)"`.

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
confirmed via a flaky-test investigation before being fixed.

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
