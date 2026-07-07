# Testing

## Running

```sh
just test
# or directly:
cargo test --workspace -- --test-threads=1
```

**`--test-threads=1` is required, not optional.** The D-Bus-backed tests in
`pinentry-cosmic/tests/oo7_caching.rs` and `cosmic-ssh-askpass/tests/oo7_caching.rs`
all share the same Secret Service collection on the session bus. Running them
in parallel causes attribute-search races between tests that clean up and
recreate items under similar keys — this was a real, reproduced flake during
development, not a theoretical concern.

## Suite layout

| Location | What | Needs a live Secret Service? |
|---|---|---|
| `cosmic-passphrase-core/src/cache.rs` (`#[cfg(test)] mod tests`) | `NullBackend`, `hash_key` | No |
| `cosmic-passphrase-core/src/{config,output}.rs` | Pure data type tests | No |
| `cosmic-ssh-askpass/src/lib.rs` (`#[cfg(test)] mod tests`) | `cache_key_for_prompt`/`stable_prompt_id`/`label_for_prompt` derivation | No |
| `pinentry-cosmic/src/{assuan,main}.rs` (`#[cfg(test)] mod tests`) | Protocol parsing, state machine, `keyinfo_cache_id` | No |
| `pinentry-cosmic/tests/assuan_protocol.rs` | Full subprocess sessions over stdin/stdout | No (spawns the real binary, but no D-Bus) |
| `cosmic-ssh-askpass/tests/integration.rs` | `OO7_PASSPHRASE_READ_FILE`/`WRITE_FILE` env-var test hooks | No |
| `pinentry-cosmic/tests/oo7_caching.rs`, `cosmic-ssh-askpass/tests/oo7_caching.rs` | Real D-Bus Secret Service round-trips | **Yes** |

The D-Bus tests check for an unlocked collection up front
(`dbus_secret_service_available()`) and skip with a message rather than
hanging on the ~30s default D-Bus timeout when none is available — CI (see
`.github/workflows/ci.yml`) runs on a bare runner with no session Secret
Service, so these skip there and only run meaningfully on a real desktop
session.

## What automated tests *can't* cover, and how it was verified instead

The COSMIC GUI dialog itself (`cosmic-passphrase-dialog`) cannot be driven
by an automated test in this environment — there's no virtual display/input
injection set up, and clicking through a real dialog requires an actual
human or a much heavier headless-compositor setup. Instead:

- Every dialog-independent code path (cache read/store/delete, Assuan
  parsing, retry-eviction logic) has direct unit/integration test coverage.
- The full pipeline — real `gpg-agent` + real installed `pinentry-cosmic` +
  real `oo7-daemon`, and real `ssh-agent`/`ssh-add` + real installed
  `cosmic-ssh-askpass` + real `oo7-daemon` — was exercised end-to-end
  manually multiple times during development: an ephemeral GPG key was
  generated, its passphrase pre-seeded into oo7 (simulating what "Remember"
  would have stored), and a real `gpg -s` sign operation was confirmed to
  succeed silently (no dialog shown, since none is possible without a
  display) and produce a verifiable signature. The equivalent was done for
  SSH with `ssh-add` against a freshly generated passphrase-protected key.
  A negative control (clearing the cache first) was run each time to
  confirm failure without the cache, ruling out a check being silently
  bypassed.
- This means the *store-via-GUI-checkbox* interaction specifically (a user
  actually clicking "Remember") is verified by code review of
  `cosmic-passphrase-dialog`'s `update()` function, not by an automated or
  manual end-to-end click-through — flagged explicitly rather than silently
  assumed.

## Regression tests worth knowing the history of

- `test_dbus_cache_gpg_getpin_from_cache_with_real_gpg_agent_keyinfo_format` —
  locks in the `SETKEYINFO n/<keygrip>` fix (see `ARCHITECTURE.md`) using
  the real wire format, not a bare keygrip.
- `test_dbus_cache_gpg_getpin_from_cache_touches_file` — locks in a fix
  where `OPTION touch-file` was silently skipped on the `GETPIN` cache-hit
  path (every other completed-request path called it; the early-return
  cache hit didn't).
- `test_dbus_cache_ssh_stale_retry_count_is_not_evicted` — locks in the
  retry-eviction time-window fix; without it, this test reproduces the bug
  where a still-correct passphrase gets evicted after exactly 3 uses no
  matter how far apart in time.
- `test_dbus_backend_stores_in_persistent_collection` — verifies,
  independently of `DbusBackend`'s own return value, that a stored item
  actually lands in a collection labeled `Login`/`login`/`Default`/
  `default` rather than relying on undocumented D-Bus enumeration order.

## Test hygiene notes

A few tests share on-disk state (`/tmp` files, `$XDG_RUNTIME_DIR/cosmic-passphrase-retry/`)
across runs. Two were previously found to poison themselves this way — a
permissions test that left a `0o000` file behind after a prior failed run,
and an eviction test that left a stale retry counter behind — both now reset
their own state defensively at the *start* of the test rather than relying
on cleanup at the end succeeding.
