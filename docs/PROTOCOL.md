# Assuan pinentry protocol support (`pinentry-cosmic`)

Reflects the actual `Command` enum and `apply_option`/`handle_command`
matches in `pinentry-cosmic/src/assuan.rs` and `src/main.rs` — not aspirational,
this table only lists what's actually parsed today, and calls out
specifically what's parsed-but-inert vs. what actually changes behavior.

## Commands

| Command | Support | Notes |
|---|---|---|
| `SETTITLE` | Full | Window title |
| `SETDESC` | Full | Dialog description text |
| `SETPROMPT` | Full | Input prompt label |
| `SETERROR` | Full | Error message (shown in red); also triggers oo7 cache eviction on the next `GETPIN` |
| `SETOK` | Full | Custom OK button label |
| `SETCANCEL` | Full | Custom Cancel button label |
| `SETNOTOK` | Full | Alternative button label (for `CONFIRM`) |
| `SETREPEAT` | Full | Shows the repeat-passphrase field |
| `SETREPEATERROR` | Parsed, stored | Not yet rendered — the dialog shows its own fixed "Passphrases do not match." text instead |
| `SETREPEATOK` | Parsed, stored | Not yet rendered |
| `SETQUALITYBAR` / `_TT` | Parsed, stored | **Inert** — no quality-bar UI or `INQUIRE QUALITY` round-trip implemented |
| `SETGENPIN` / `_TT` | Parsed, stored | **Inert** — no PIN-generation UI or `INQUIRE GENPIN` round-trip implemented |
| `SETKEYINFO` | Full | Drives the oo7 cache key (`gpg:<keygrip>` after stripping gpg-agent's `<flag>/` prefix — see `ARCHITECTURE.md`) |
| `GETPIN` | Full | Cache check → dialog → optional cache store |
| `CONFIRM [--one-button]` | Full | |
| `MESSAGE` | Full | |
| `RESET` | Full | Clears per-request state; preserves session options (timeout, grab, touch-file, etc.) |
| `BYE` | Full | |
| `NOP` | Full | |
| `END` / `CAN` / `CANCEL` / `D` (data line) | Rejected with `ERR` outside an `INQUIRE` | This crate never issues an `INQUIRE`, so these are only ever protocol errors in practice |
| anything unrecognized | `OK` (silently ignored) | Required by the Assuan spec — pinentry must tolerate unknown commands |

## `OPTION` values

| Option | Support | Notes |
|---|---|---|
| `timeout=<secs>` | Full | Capped at 120s; dialog auto-cancels via a 500ms `Tick` subscription |
| `allow-external-password-cache` | Full | Gates all oo7 cache read/write/evict behavior |
| `touch-file=<path>` | Full | Touched (empty-written) after every completed request, including a cache hit |
| `default-ok` / `default-cancel` / `default-prompt` | Full | Only applied if not already set by `SETOK`/`SETCANCEL`/`SETPROMPT` |
| `flavor` | Full | Replies `S FLAVOR cosmic` |
| `grab` / `no-grab` | Parsed, stored | **Inert** — no keyboard grab implemented in the COSMIC dialog |
| `parent-wid` | Parsed, stored | **Inert** — no window-parenting implemented |
| `display` / `ttyname` / `ttytype` / `lc-ctype` / `lc-messages` | Parsed, stored | **Inert** — the COSMIC dialog doesn't forward to a specific display/tty or localize by these |
| `constraints-enforce` / `constraints-hint-short` / `constraints-hint-long` / `constraints-error-title` | Parsed, stored | **Inert** — no passphrase-quality constraint checking implemented |
| `ttyalert`, `default-pwmngr`, `default-cf-visi`, `default-tt-visi`, `default-tt-hide`, `default-capshint`, `invisible-char` | Accepted, no-op | Explicitly ignored — has no effect on a GUI dialog by design |

"Inert" above is a deliberate, documented gap (matches upstream pinentry
projects' own "Planned" feature sets for quality bar / genpin / window
grabbing), not oversight — see `SECURITY.md`'s note on this for why it isn't
treated as dead code to be deleted.

## GPG error codes used (`pinentry-cosmic/src/error.rs`)

| Code | Name | Used for |
|---|---|---|
| 99 | `CANCELED` | User cancelled, or dialog failed/timed out |
| 257 | `ASS_GENERAL` | Protocol errors (stdin read failure, command received out of sequence) |
| 48 | `NOT_CONFIRMED` | `CONFIRM`'s alternative ("not ok") button pressed |
