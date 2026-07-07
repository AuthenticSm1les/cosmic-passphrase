# Security model

## What's protecting the cached passphrases

Passphrases are stored as ordinary freedesktop Secret Service items, in
whichever collection `DbusBackend` selects (see `ARCHITECTURE.md`) —
normally the persistent `Login` collection, encrypted at rest in
`~/.local/share/keyrings/v1/login.keyring` and unlocked automatically at
login via `oo7-daemon`'s PAM module (`pam_oo7.so`), using your login
password. The D-Bus session itself is encrypted (`oo7`/`libsecret` negotiate
an AES session key when opening a connection) — confirmed by inspecting a
live collection handle during development (`algorithm: Encrypted`).

Attribute values used to look up items (`application=cosmic-passphrase`,
`key=<cache key>`) are **not** encrypted — Secret Service attributes are the
searchable index, only the secret value itself is. Cache keys are either a
GPG keygrip or a hash of an SSH key's file path, neither of which is
sensitive on its own.

## No consent/prompter layer

GNOME's stack has `gcr-prompter` (registered as
`org.gnome.keyring.PrivatePrompter`/`SystemPrompter` on D-Bus) mediating
"app X wants to access this secret" consent dialogs. Checked directly on
this system: `oo7` registers no equivalent service. Every `store`/`read`/
`delete` this project does — and, by the same token, anything any other
local session application does — happens against an unlocked collection
with no per-app gating. This was confirmed empirically: none of the many
manual `secret-tool`/`DbusBackend` round-trips performed while developing
this project ever triggered a consent prompt.

This is not a novel weakening introduced by this project — it reflects how
an *unlocked* default keyring already treats same-session applications
under `gnome-keyring` too, for routine item access (the prompter is invoked
more for unlock/PKCS#11-trust decisions). It's documented here as a known,
accepted property of the design rather than a silent gap: **any application
running as your user, on your session bus, can read every passphrase this
project has cached**, with no additional barrier beyond your session being
unlocked at all.

## Cache-key stability vs. correctness

- **GPG** (`gpg:<keygrip>`): the keygrip is gpg-agent's own stable
  identifier for a specific secret key. A cache hit is exactly as trustworthy
  as gpg-agent's own key management.
- **SSH** (`ssh:<hash of extracted path>`): derived from free-text prompt
  text OpenSSH provides askpass helpers, since there is no stable identifier
  available at all in that protocol. If two different keys ever produced
  the same extracted path string (they can't, in practice — the path *is*
  the identifier), or if OpenSSH ever stopped including the path in the
  prompt at all (it does today, confirmed live), the cache key would degrade
  back to hashing arbitrary prose, which is stable only as long as the
  prose doesn't change. This is a structural limitation of the
  `$SSH_ASKPASS` protocol, not something this project can fully close
  without becoming a full `ssh-agent` replacement instead of a helper
  alongside one (a design tradeoff explicitly considered — see
  `ARCHITECTURE.md` and the now-removed `oo7-ssh-agent` sibling project,
  which took that heavier approach and was abandoned in favor of this one).
- **SSH retry eviction is a heuristic, not a security control.** It exists
  only to stop repeatedly feeding a *wrong* passphrase to `ssh-add` forever;
  it cannot distinguish "wrong" from "right but reused a 4th time" without
  an actual signal, which OpenSSH's askpass protocol does not provide.

## Known fragility already mitigated

- A transient D-Bus error on the very first call used to permanently
  disable caching for that process's lifetime (fixed — only successful
  connections are cached now).
- The Secret Service collection could silently be selected by unspecified
  enumeration order rather than by which one is actually persistent (fixed
  — explicit label preference, with a logged fallback warning if it can't
  be determined).
- `hash_key` previously used `std`'s `DefaultHasher`, whose docs explicitly
  disclaim stability across Rust releases — a toolchain upgrade could have
  silently orphaned every cached SSH passphrase. Replaced with a
  from-scratch, permanently stable FNV-1a implementation.

## Things a future contributor should not assume

- That gpg-agent sends a bare keygrip in `SETKEYINFO` — it doesn't (see
  `ARCHITECTURE.md`). Any change touching GPG cache-key derivation should be
  verified against a real `gpg-agent`, not just this crate's own synthetic
  Assuan-session tests, which is exactly how this bug went undetected for as
  long as it did.
- That the Secret Service backend is `oo7-daemon` specifically — the code
  deliberately only depends on the generic D-Bus Secret Service protocol,
  and should keep working unmodified against `gnome-keyring-daemon` or any
  other implementation.
- That every `OPTION`/`SET*` Assuan command `pinentry-cosmic` parses is
  wired into dialog behavior. Several (`SETQUALITYBAR`, `SETGENPIN`,
  `OPTION grab`, `OPTION parent-wid`, tty/locale options) are accepted and
  stored in `PinentryState` per the Assuan spec, but not yet acted on by the
  COSMIC dialog — see `PROTOCOL.md` for the full, up-to-date list of what's
  wired up vs. deliberately inert. This is intentional, documented upstream
  scope, not dead code.
