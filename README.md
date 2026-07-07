# cosmic-passphrase

Native COSMIC desktop integration for caching GPG and SSH key passphrases in
the system keyring (via [oo7](https://docs.rs/oo7)/the freedesktop Secret
Service), so you unlock a key once and aren't asked again until the cached
entry is invalidated.

A cached passphrase is never handed back silently — a dedicated Allow/Deny
dialog always shows first, naming what's being accessed (e.g. *"gpg-agent
wants to access the saved passphrase..."*), so reusing a stored value is
always an explicit choice, not something gpg-agent/ssh-agent gets the moment
the keyring happens to be unlocked. If the keyring is locked or otherwise
unreachable, the "Remember" option is replaced with a note explaining why,
instead of silently doing nothing. See `docs/ARCHITECTURE.md` for the
caching flow in detail, including why a submitted GPG passphrase is held in
memory rather than written to the keyring until gpg-agent confirms it was
correct.

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

## Installation

### From the AUR

```sh
paru -S cosmic-passphrase-git
# or: yay -S cosmic-passphrase-git
```

This installs `/usr/bin/pinentry-cosmic` and `/usr/lib/cosmic-ssh-askpass`.
Installing the package does not wire anything up by itself — see "Usage"
below to point gpg-agent/ssh-agent at them.

### From source

```sh
git clone https://github.com/AuthenticSm1les/cosmic-passphrase.git
cd cosmic-passphrase
just install-all   # release build + install to /usr/local/bin and /usr/lib (sudo)
```

See "Building" further down for the other `just` targets (debug builds,
tests, lint) if you're setting up a dev environment instead of just
installing.

## Usage

### GPG passphrases

Point gpg-agent at `pinentry-cosmic`:

```
# ~/.gnupg/gpg-agent.conf
pinentry-program /usr/bin/pinentry-cosmic
```

(`/usr/local/bin/pinentry-cosmic` instead, if you installed from source via
`just install-all` rather than the AUR package.) Then:

```sh
gpg-connect-agent reloadagent /bye
```

`allow-external-cache` is on by default in gpg-agent (there's only a
`--no-allow-external-cache` to turn it off), so nothing else is needed for
oo7-backed caching to activate. The next passphrase prompt shows the normal
COSMIC dialog with a "Remember passphrase" checkbox; check it, and the
passphrase is cached once gpg-agent confirms it was actually correct (see
`docs/ARCHITECTURE.md` for why that's not immediate). The *next* request for
that same key shows an Allow/Deny dialog instead of a blank prompt — using
the cached value is always an explicit choice, never automatic.

### SSH passphrases

There are two independent ways to get SSH key passphrases cached here —
pick one (see "Which SSH integration should I use?" below for the
tradeoffs):

**Option A — via gpg-agent**, if you already run `enable-ssh-support`:

```
# ~/.gnupg/gpg-agent.conf
enable-ssh-support
pinentry-program /usr/bin/pinentry-cosmic
```

```sh
export SSH_AUTH_SOCK="$(gpgconf --list-dirs agent-ssh-socket)"
```

`cosmic-ssh-askpass` isn't involved in this path at all — gpg-agent calls
`pinentry-cosmic` directly, using the SSH key's keygrip as a stable cache
identifier, same as for GPG keys.

**Option B — a plain OpenSSH `ssh-agent`** with `cosmic-ssh-askpass`, for
anyone who'd rather not have gpg-agent acting as their SSH agent:

```sh
eval "$(ssh-agent -s)"
export SSH_ASKPASS=/usr/lib/cosmic-ssh-askpass
export SSH_ASKPASS_REQUIRE=force
ssh-add ~/.ssh/id_ed25519
```

Put the `ssh-agent`/`SSH_ASKPASS*` lines in your shell profile or session
startup so they're set for every new session — `ssh-add` itself only needs
re-running when the agent restarts (e.g. after a reboot), not every login.

The two options cache independently (different Secret Service attributes),
so switching between them means re-entering a given key's passphrase once.

### Checking what's cached

Cached passphrases are ordinary, labeled Secret Service items:

```sh
secret-tool search application cosmic-passphrase
```

or browse the `Login` collection in a GUI keyring manager like `seahorse`.

### Clearing a cached passphrase

```sh
secret-tool search application cosmic-passphrase   # find the key
secret-tool clear application cosmic-passphrase key '<key>'
```

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

See "Usage" above for configuring gpg-agent/ssh-agent to actually use what
you just built.

## Contributing

Bug reports, fixes, and small focused PRs are welcome.

### Before you start

- Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) first. It covers the
  crate split and both frontends' caching flows, and documents several
  non-obvious constraints that are easy to accidentally regress without
  knowing about them going in — e.g. the real (not bare-keygrip)
  `SETKEYINFO` wire format, and winit's one-event-loop-per-process limit
  and the child-process workaround it forced.
- Skim [`docs/SECURITY.md`](docs/SECURITY.md) before touching anything
  cache- or consent-related — it documents the trust model and what *not*
  to assume.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md) lists exactly which Assuan
  commands/options are wired up vs. deliberately inert, if you're working
  on `pinentry-cosmic`.

### Workflow

```sh
just check   # cargo check
just lint    # cargo clippy --all-targets -- -D warnings — must be clean
just test    # cargo test --workspace -- --test-threads=1 (serial; see docs/TESTING.md)
just fmt     # cargo fmt
```

All four should pass before opening a PR. CI (`.github/workflows/ci.yml`)
runs `check`, `lint`, and `test` on every push; the D-Bus-backed tests skip
gracefully on CI's bare runner (no session Secret Service there) and only
run meaningfully on a real desktop session — if you're changing anything
cache-related, also run `just test` locally on a real session before
opening a PR, not just relying on CI.

### Conventions

- No comments explaining *what* code does — names should already do that.
  Comments are for *why*: a non-obvious constraint, a workaround, a
  subtlety that would surprise a reader. The existing code has plenty of
  examples of this if you want the calibration.
- Prefer a pure, directly-unit-testable function over logic embedded in an
  I/O-heavy caller, where reasonable — e.g. `decide_retry` in
  `cosmic-ssh-askpass`, `keyinfo_cache_id` in `pinentry-cosmic`. Most of
  this project's trickiest bugs were caught by tests that needed neither a
  display nor a live D-Bus session.
- Anything touching GPG/SSH cache-key derivation, the Assuan wire format,
  or the consent flow should be tested against something closer to the
  real protocol than a hand-written synthetic session where practical —
  see `docs/ARCHITECTURE.md`'s note on the `SETKEYINFO` bug for exactly
  why that matters here specifically.
- Keep `cosmic-passphrase-core` free of GUI dependencies. An `iced`/
  `libcosmic` import there almost certainly belongs in
  `cosmic-passphrase-dialog` instead.

### Reporting issues

Include: what you expected vs. what happened, your gpg-agent/ssh-agent
setup (which SSH integration option, if relevant — see "Which SSH
integration should I use?" above), and whether your Secret Service backend
is `oo7-daemon` or `gnome-keyring-daemon`. For anything cache-related,
`secret-tool search application cosmic-passphrase` output (attributes/labels
only — redact any secret values) is usually the fastest way to show what's
actually cached.
