# cosmic-passphrase

Native COSMIC desktop integration for caching GPG and SSH key passphrases in
the system keyring (via [oo7](https://docs.rs/oo7)/the freedesktop Secret
Service), so you unlock a key once and aren't asked again until the cached
entry is invalidated.

A cached passphrase is never handed back silently — the dialog always shows,
with a "Use Saved Passphrase" button next to normal entry, so reusing a
stored value is always an explicit choice. If the keyring is locked or
otherwise unreachable, the "Remember" option is replaced with a note
explaining why, instead of silently doing nothing. See `docs/ARCHITECTURE.md`
for the caching flow in detail, including why a submitted GPG passphrase is
held in memory rather than written to the keyring until gpg-agent confirms it
was correct.

**Full documentation:**

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — crate layout, both
  frontends' caching flows, the `SETKEYINFO` wire-format detail, collection
  selection, persistence.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md) — the full Assuan command/option
  support table: what's wired up vs. deliberately inert.
- [`docs/SECURITY.md`](docs/SECURITY.md) — trust model, no consent/prompter
  layer, cache-key stability tradeoffs, what not to assume.
- [`docs/TESTING.md`](docs/TESTING.md) — suite layout, what can't be
  automated (the GUI dialog) and how it was verified instead, regression
  test history.

This project supersedes three earlier, standalone prototypes (a
`pinentry-cosmic` with no oo7 caching, a `cosmic-ssh-askpass` pre-merge, and
an `oo7-ssh-agent` full-agent-replacement approach) — all removed, no
functionality lost; everything they did is covered here.

## Architecture

The workspace is split by *what each crate depends on*, not just by binary:

```
cosmic-passphrase-core     headless: CacheBackend + DbusBackend, DialogConfig,
                            DialogOutput. No GUI dependency.
        ^
        |  (DialogConfig in, DialogOutput out)
        |
cosmic-passphrase-dialog   the only crate depending on libcosmic/iced/wgpu.
                            run_dialog(DialogConfig) -> DialogOutput.
        ^
        |
   +----+----+
   |         |
pinentry-cosmic   cosmic-ssh-askpass
(GPG pinentry)    ($SSH_ASKPASS helper)
```

`cosmic-passphrase-core` and `cosmic-passphrase-dialog` are split deliberately:
anything that only needs the caching/config logic (unit tests, a future
headless tool, CI) builds in seconds against `cosmic-passphrase-core` alone,
without pulling in the GUI toolchain (`iced`, `wgpu`, `winit`, X11/Wayland
client libraries). Only `cosmic-passphrase-dialog` — and therefore anything
that links it — pays that build cost.

- **`pinentry-cosmic`** — a drop-in [pinentry](https://www.gnupg.org/documentation/manuals/assuan/)
  program for `gpg-agent`. Implements the same `SETKEYINFO` /
  `allow-external-password-cache` mechanism `pinentry-gnome3` uses against
  `gnome-keyring`, against oo7 instead. Cache key: `gpg:<keygrip>`, where the
  keygrip comes from gpg-agent itself — a stable, protocol-provided identifier
  (see "SETKEYINFO wire format" below for a wrinkle here).
- **`cosmic-ssh-askpass`** — an `$SSH_ASKPASS` helper (same shape as GNOME's
  own `gcr-ssh-askpass`): invoked by a real `ssh-agent`/`ssh-add`, not a
  replacement for one. Cache key: `ssh:<hash of the key's file path>`,
  extracted from the free-text prompt OpenSSH provides (see
  `cosmic-ssh-askpass`'s `cache_key_for_prompt` for details — this is
  inherently fuzzier than the GPG path, since askpass never receives a
  stable key identifier, only a sentence).

## Which SSH integration should I use?

There are two independent ways SSH key passphrases can end up cached here,
and they matter differently depending on your setup:

1. **`gpg-agent --enable-ssh-support`** — if your `gpg-agent.conf` has
   `enable-ssh-support`, gpg-agent itself acts as your `ssh-agent`
   (`gpg-agent-ssh.socket`) and calls `pinentry-cosmic` for SSH key
   passphrases too, using the exact same `SETKEYINFO`-based caching as GPG
   keys — including the stable keygrip identifier. If you use gpg-agent's
   SSH support, `cosmic-ssh-askpass` is not involved at all.
2. **A plain OpenSSH `ssh-agent`** with `SSH_ASKPASS=cosmic-ssh-askpass` and
   `SSH_ASKPASS_REQUIRE=force` — for people who deliberately avoid
   gpg-agent's SSH-agent emulation (it has its own long-standing UX quirks
   around adding/removing keys). This path is what actually exercises
   `cosmic-ssh-askpass` and its hashed-prompt cache key.

Both are legitimate depending on which agent you run — just be aware they
cache independently (different attribute keys), so switching between them
means re-entering passphrases once.

## Trust model

Items are stored as ordinary Secret Service items under the attribute
`application=cosmic-passphrase`, in whichever collection is unlocked and
labeled `Login`/`login`/`Default`/`default` (falling back to the first
unlocked collection otherwise — see `DbusBackend::get_or_init_collection`).
There is **no consent-prompt layer** here (the equivalent of GNOME's
`gcr-prompter`, which mediates "app X wants to access this secret"
dialogs) — any application on your session D-Bus can read, create, or
delete these items with no per-app gating. This mirrors how most local
session apps are already trusted equally by an unlocked default keyring;
it's noted here so it's a documented property of the design, not a silent
gap.

## Building

```sh
just build          # release build of pinentry-cosmic and cosmic-ssh-askpass
just build-all       # debug build of the whole workspace
just check           # cargo check
just test            # full test suite (needs an unlocked Secret Service on
                      # the session bus for the D-Bus-backed tests; they skip
                      # gracefully if one isn't available)
just lint             # cargo clippy -- -D warnings
just install-pinentry # build + install pinentry-cosmic to /usr/local/bin
just install-ssh      # build + install cosmic-ssh-askpass to /usr/lib
just install-all
```

### Testing notes

- Tests that talk to a real Secret Service (`tests/oo7_caching.rs` in both
  `pinentry-cosmic` and `cosmic-ssh-askpass`) share the same D-Bus session
  collection and **must run serially**: `cargo test -- --test-threads=1`.
  Running them in parallel causes attribute-search races between tests.
- These tests check for an unlocked Secret Service daemon
  (`oo7-daemon`, `gnome-keyring-daemon`, or anything else implementing
  `org.freedesktop.secrets`) up front and skip with a message if none is
  available, rather than hanging on the default 30s D-Bus timeout.

### Configuring gpg-agent

```
# ~/.gnupg/gpg-agent.conf
pinentry-program /usr/local/bin/pinentry-cosmic
```

Then `gpg-connect-agent reloadagent /bye`.

`allow-external-cache` is on by default in gpg-agent (there's only a
`--no-allow-external-cache` to turn it off), so no further gpg-agent
configuration is needed for the oo7-backed caching to activate.
