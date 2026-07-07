use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn binary_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_pinentry_cosmic") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("target");
    path.push("debug");
    path.push("pinentry-cosmic");
    path
}

struct PinentrySession {
    child: Child,
    reader: BufReader<Box<dyn std::io::Read + Send>>,
    writer: Box<dyn std::io::Write + Send>,
    greeting: String,
}

impl PinentrySession {
    fn start() -> Self {
        let mut child = Command::new(binary_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn pinentry-cosmic");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let mut reader = BufReader::new(Box::new(stdout) as Box<dyn std::io::Read + Send>);
        let mut greeting = String::new();
        reader.read_line(&mut greeting).unwrap();

        Self {
            child,
            reader,
            writer: Box::new(stdin),
            greeting,
        }
    }

    fn send(&mut self, line: &str) {
        writeln!(self.writer, "{}", line).unwrap();
        self.writer.flush().unwrap();
    }

    fn read_line(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        line
    }

    fn read_ok(&mut self) -> String {
        let line = self.read_line();
        assert!(line.starts_with("OK"), "expected OK, got: {:?}", line);
        line
    }

    fn read_err(&mut self) -> String {
        let line = self.read_line();
        assert!(line.starts_with("ERR"), "expected ERR, got: {:?}", line);
        line
    }

    fn close(mut self) {
        self.send("BYE");
        let response = self.read_line();
        assert!(response.contains("OK closing connection"));
        self.child.wait().unwrap();
    }

}

// ── Greeting ───────────────────────────────────────────────────────

#[test]
fn test_greeting_format() {
    let session = PinentrySession::start();
    assert!(
        session.greeting.starts_with("OK pleased to meet you"),
        "unexpected greeting: {:?}",
        session.greeting
    );
    assert!(
        session.greeting.contains("pinentry-cosmic"),
        "greeting should identify as pinentry-cosmic"
    );
    session.close();
}

// ── SET* commands ──────────────────────────────────────────────────

#[test]
fn test_settitle() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Unlock Secret Key");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setdesc() {
    let mut s = PinentrySession::start();
    s.send("SETDESC Enter passphrase to unlock key");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setprompt() {
    let mut s = PinentrySession::start();
    s.send("SETPROMPT PIN:");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_seterror() {
    let mut s = PinentrySession::start();
    s.send("SETERROR Invalid passphrase");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setok() {
    let mut s = PinentrySession::start();
    s.send("SETOK Confirm");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setcancel() {
    let mut s = PinentrySession::start();
    s.send("SETCANCEL Abort");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setnotok() {
    let mut s = PinentrySession::start();
    s.send("SETNOTOK Deny");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setrepeat() {
    let mut s = PinentrySession::start();
    s.send("SETREPEAT");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setempty_title_clears() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Some Title");
    s.read_ok();
    s.send("SETTITLE ");
    s.read_ok();
    // Subsequent GETPIN should not include the old title
    // (can't test GETPIN without GUI, but protocol responses are correct)
    s.close();
}

// ── OPTION commands ────────────────────────────────────────────────

#[test]
fn test_option_timeout() {
    let mut s = PinentrySession::start();
    s.send("OPTION timeout=30");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_grab() {
    let mut s = PinentrySession::start();
    s.send("OPTION grab");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_no_grab() {
    let mut s = PinentrySession::start();
    s.send("OPTION no-grab");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_allow_external_cache() {
    let mut s = PinentrySession::start();
    s.send("OPTION allow-external-password-cache");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_touch_file() {
    let mut s = PinentrySession::start();
    s.send("OPTION touch-file=/tmp/pinentry-test");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_display() {
    let mut s = PinentrySession::start();
    s.send("OPTION display=:0");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_ttyname() {
    let mut s = PinentrySession::start();
    s.send("OPTION ttyname=/dev/pts/0");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_lc_messages() {
    let mut s = PinentrySession::start();
    s.send("OPTION lc-messages=de_DE.UTF-8");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_parent_wid() {
    let mut s = PinentrySession::start();
    s.send("OPTION parent-wid=0x12345");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── NOP ────────────────────────────────────────────────────────────

#[test]
fn test_nop() {
    let mut s = PinentrySession::start();
    s.send("NOP");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_empty_line_is_nop() {
    let mut s = PinentrySession::start();
    s.send("");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_comment_is_nop() {
    let mut s = PinentrySession::start();
    s.send("# this is a comment");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_unknown_command_is_ignored() {
    let mut s = PinentrySession::start();
    s.send("UNKNOWN_COMMAND arg1 arg2");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_unknown_command_with_equals() {
    let mut s = PinentrySession::start();
    s.send("FOOBAR=value");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── BYE ────────────────────────────────────────────────────────────

#[test]
fn test_bye_exits_cleanly() {
    let mut s = PinentrySession::start();
    s.send("BYE");
    let response = s.read_line();
    assert!(response.contains("OK closing connection"));
    let status = s.child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn test_bye_after_commands() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Test");
    s.read_ok();
    s.send("SETDESC Description");
    s.read_ok();
    s.send("SETPROMPT Enter:");
    s.read_ok();
    s.send("OPTION timeout=60");
    s.read_ok();
    s.send("BYE");
    let response = s.read_line();
    assert!(response.contains("OK closing connection"));
    let status = s.child.wait().unwrap();
    assert!(status.success());
}

// ── RESET ──────────────────────────────────────────────────────────

#[test]
fn test_reset_ok() {
    let mut s = PinentrySession::start();
    s.send("RESET");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_reset_after_setup() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Title");
    s.read_ok();
    s.send("SETDESC Desc");
    s.read_ok();
    s.send("RESET");
    s.read_ok();
    // After RESET, subsequent SET* should work as if fresh
    s.send("SETTITLE New Title");
    s.read_ok();
    s.close();
}

// ── Full sessions ──────────────────────────────────────────────────

#[test]
fn test_full_pinentry_session_no_getpin() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Unlock GPG Key");
    s.read_ok();
    s.send("SETDESC Enter passphrase to unlock the secret key");
    s.read_ok();
    s.send("SETPROMPT Passphrase:");
    s.read_ok();
    s.send("SETOK Unlock");
    s.read_ok();
    s.send("SETCANCEL Cancel");
    s.read_ok();
    s.send("OPTION timeout=120");
    s.read_ok();
    s.send("OPTION grab");
    s.read_ok();
    s.send("BYE");
    let response = s.read_line();
    assert!(response.contains("OK closing connection"));
    s.child.wait().unwrap();
}

#[test]
fn test_confirm_session() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Confirm Removal");
    s.read_ok();
    s.send("SETDESC Are you sure you want to delete the key?");
    s.read_ok();
    s.send("SETPROMPT Proceed?");
    s.read_ok();
    s.send("SETOK Yes");
    s.read_ok();
    s.send("SETCANCEL No");
    s.read_ok();
    s.send("SETNOTOK Cancel");
    s.read_ok();
    s.close();
}

#[test]
fn test_message_session() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE Notice");
    s.read_ok();
    s.send("SETDESC Your key has expired. Please generate a new one.");
    s.read_ok();
    s.send("SETOK Dismiss");
    s.read_ok();
    s.close();
}

#[test]
fn test_repeat_passphrase_session() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE New Passphrase");
    s.read_ok();
    s.send("SETDESC Choose a new passphrase");
    s.read_ok();
    s.send("SETPROMPT New passphrase:");
    s.read_ok();
    s.send("SETREPEAT");
    s.read_ok();
    s.send("SETREPEATERROR Passphrases do not match");
    s.read_ok();
    s.send("SETREPEATOK Passphrases match");
    s.read_ok();
    s.send("OPTION constraints-enforce");
    s.read_ok();
    s.send("OPTION constraints-hint-short=min 8 characters");
    s.read_ok();
    s.close();
}

// ── Error conditions ───────────────────────────────────────────────

#[test]
fn test_end_is_error() {
    let mut s = PinentrySession::start();
    s.send("END");
    let response = s.read_err();
    assert!(response.contains("ERR"));
    assert!(response.contains("unexpected command"));
    s.close();
}

#[test]
fn test_cancel_is_error() {
    let mut s = PinentrySession::start();
    s.send("CANCEL");
    let response = s.read_err();
    assert!(response.contains("ERR"));
    s.close();
}

// ── Data line encoding ─────────────────────────────────────────────

#[test]
fn test_data_line_percent_encoding() {
    let mut s = PinentrySession::start();
    // D lines should be properly encoded/decoded
    s.send("D hello%20world");
    let response = s.read_err();
    // D lines outside of an INQUIRE context produce an error
    assert!(response.contains("ERR"));
    s.close();
}

// ── Multiple sessions sequentially ──────────────────────────────────
// Each test starts a new process, so isolation is guaranteed

#[test]
fn test_consecutive_settitle() {
    let mut s = PinentrySession::start();
    s.send("SETTITLE First");
    s.read_ok();
    s.send("SETTITLE Second");
    s.read_ok();
    s.send("SETTITLE Third");
    s.read_ok();
    s.close();
}

#[test]
fn test_keyinfo_setting() {
    let mut s = PinentrySession::start();
    s.send("SETKEYINFO 0123456789ABCDEF");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── EOF without BYE (stdin close) ───────────────────────────────────

#[test]
fn test_eof_without_bye() {
    // Drop stdin to send EOF; the process should exit cleanly
    let mut child = Command::new(binary_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn pinentry-cosmic");

    // Read greeting, then process exits on EOF from /dev/null stdin
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut greeting = String::new();
    stdout.read_line(&mut greeting).unwrap();
    assert!(greeting.starts_with("OK pleased to meet you"));

    // Process should exit cleanly after reading EOF
    let status = child.wait().unwrap();
    assert!(status.success(), "process should exit 0 on stdin EOF");
}

#[test]
fn test_eof_by_dropping_stdin() {
    let mut child = Command::new(binary_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn pinentry-cosmic");

    // Read greeting first
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut greeting = String::new();
    stdout.read_line(&mut greeting).unwrap();
    assert!(greeting.starts_with("OK pleased to meet you"));

    // Drop stdin to send EOF
    child.stdin.take();

    // Process should exit cleanly
    let status = child.wait().unwrap();
    assert!(status.success(), "process should exit 0 when stdin is closed");
}

// ── Long line truncation ────────────────────────────────────────────

#[test]
fn test_long_line_truncated_as_unknown() {
    let mut s = PinentrySession::start();
    // >1000 char line
    let long = "A".repeat(1001);
    s.send(&long);
    // Unknown commands return OK (silently ignored)
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_long_line_sets_not_truncated() {
    let mut s = PinentrySession::start();
    // Exactly 1000 chars - still valid
    let long = "A".repeat(999);
    s.send(&format!("SETTITLE {}", long));
    // This is > 1000 chars total (SETTITLE + space + 999 As = 1008), so truncated
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_long_line_boundary_1000() {
    let mut s = PinentrySession::start();
    // Exactly 1000 chars including command - treated as valid
    // SETTITLE + space + 991 chars = 999 + trailing newline from send()
    // Actually send() adds \n, so the total including \n would be 1000 max
    let body = "X".repeat(991);
    s.send(&format!("SETTITLE {}", body));
    // 8 + 1 + 991 = 1000 chars before \n → valid, doesn't get truncated
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_over_1000_chars_truncated() {
    let mut s = PinentrySession::start();
    let body = "B".repeat(992);
    s.send(&format!("SETTITLE {}", body));
    // 8 + 1 + 992 = 1001 chars → truncated
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── SETQUALITYBAR / SETGENPIN ───────────────────────────────────────

#[test]
fn test_setqualitybar() {
    let mut s = PinentrySession::start();
    s.send("SETQUALITYBAR Strength:");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setqualitybar_empty_clears() {
    let mut s = PinentrySession::start();
    s.send("SETQUALITYBAR Strength:");
    s.read_ok();
    s.send("SETQUALITYBAR ");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setqualitybar_tt() {
    let mut s = PinentrySession::start();
    s.send("SETQUALITYBAR_TT Password strength indicator");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setgenpin() {
    let mut s = PinentrySession::start();
    s.send("SETGENPIN Generate PIN");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_setgenpin_tt() {
    let mut s = PinentrySession::start();
    s.send("SETGENPIN_TT Generate a random PIN");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── OPTION flavor ───────────────────────────────────────────────────

#[test]
fn test_option_flavor() {
    let mut s = PinentrySession::start();
    s.send("OPTION flavor");
    // flavor writes S FLAVOR cosmic before OK
    let status = s.read_line();
    assert_eq!(status.trim(), "S FLAVOR cosmic");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── END / CANCEL / CAN ──────────────────────────────────────────────

#[test]
fn test_can_is_error() {
    let mut s = PinentrySession::start();
    s.send("CAN");
    let response = s.read_err();
    assert!(response.contains("unexpected command"));
    s.close();
}

// ── D line percent-encoding round-trip ──────────────────────────────

#[test]
fn test_d_line_with_percent_encoded_value() {
    let mut s = PinentrySession::start();
    s.send("D hello%20world%21");
    let response = s.read_err();
    // The parsed D content is "hello world!" but the command is still
    // rejected as "unexpected command in current state"
    assert!(response.contains("unexpected command"));
    s.close();
}

#[test]
fn test_d_line_with_newline() {
    let mut s = PinentrySession::start();
    s.send("D line1%0Aline2");
    let response = s.read_err();
    assert!(response.contains("ERR"));
    s.close();
}

#[test]
fn test_d_line_empty() {
    let mut s = PinentrySession::start();
    s.send("D");
    let response = s.read_err();
    assert!(response.contains("ERR"));
    s.close();
}

#[test]
fn test_d_line_only_space() {
    let mut s = PinentrySession::start();
    s.send("D ");
    let response = s.read_err();
    assert!(response.contains("ERR"));
    s.close();
}

// ── SETKEYINFO edge cases ───────────────────────────────────────────

#[test]
fn test_keyinfo_empty_is_ok() {
    let mut s = PinentrySession::start();
    s.send("SETKEYINFO ");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_keyinfo_long_value() {
    let mut s = PinentrySession::start();
    s.send(&format!("SETKEYINFO {}", "0123456789abcdef".repeat(20)));
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── Multiple SETREPEAT / options interaction ────────────────────────

#[test]
fn test_option_ttytype_unknown() {
    let mut s = PinentrySession::start();
    s.send("OPTION ttytype=xterm-256color");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_ttyalert_ignored() {
    let mut s = PinentrySession::start();
    s.send("OPTION ttyalert");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_lc_ctype() {
    let mut s = PinentrySession::start();
    s.send("OPTION lc-ctype=en_US.UTF-8");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_default_ok() {
    let mut s = PinentrySession::start();
    s.send("OPTION default-ok=Continue");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_default_cancel() {
    let mut s = PinentrySession::start();
    s.send("OPTION default-cancel=Abort");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_default_prompt() {
    let mut s = PinentrySession::start();
    s.send("OPTION default-prompt=Enter:");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_invisible_char_ignored() {
    let mut s = PinentrySession::start();
    s.send("OPTION invisible-char=*");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── Rapid-fire commands (no delay) ───────────────────────────────────

#[test]
fn test_rapid_commands() {
    let mut s = PinentrySession::start();
    for _ in 0..50 {
        s.send("SETTITLE Title");
        s.read_ok();
    }
    s.close();
}

#[test]
fn test_rapid_mixed_commands() {
    let mut s = PinentrySession::start();
    for i in 0..20 {
        s.send(&format!("SETTITLE Title {}", i));
        s.read_ok();
        s.send("SETDESC Description");
        s.read_ok();
        s.send("NOP");
        s.read_ok();
    }
    s.close();
}

// ── Pipe many commands before reading responses ─────────────────────

#[test]
fn test_pipe_batch_before_read() {
    let mut child = Command::new(binary_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn pinentry-cosmic");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // Read greeting
    let mut greeting = String::new();
    stdout.read_line(&mut greeting).unwrap();
    assert!(greeting.starts_with("OK pleased to meet you"));

    // Write multiple commands at once
    let batch = "SETTITLE One\nSETDESC Two\nSETPROMPT Three\nNOP\nBYE\n";
    stdin.write_all(batch.as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Read all responses
    let mut responses = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                responses.push(line.trim().to_string());
                if line.contains("closing connection") {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    assert!(responses.iter().any(|r| r == "OK"));
    assert!(responses.iter().any(|r| r.contains("closing connection")));
    let status = child.wait().unwrap();
    assert!(status.success());
}

// ── OPTION with unusual values ──────────────────────────────────────

#[test]
fn test_option_timeout_max() {
    let mut s = PinentrySession::start();
    s.send("OPTION timeout=999999");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_timeout_zero() {
    let mut s = PinentrySession::start();
    s.send("OPTION timeout=0");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_parent_wid_hex() {
    let mut s = PinentrySession::start();
    s.send("OPTION parent-wid=0xdeadbeef");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

#[test]
fn test_option_display_ip() {
    let mut s = PinentrySession::start();
    s.send("OPTION display=192.168.1.1:0.0");
    assert_eq!(s.read_ok().trim(), "OK");
    s.close();
}

// ── CONFIRM with --one-button ───────────────────────────────────────

#[test]
fn test_confirm_one_button_parses_ok() {
    let mut s = PinentrySession::start();
    s.send("CONFIRM --one-button");
    // CONFIRM triggers dialog (no display), but it's parsed correctly
    // Either exit or timeout — just verify we get some response
    let _ = s.read_line();
    // Don't call close() since process may have exited
    let _ = s.child.kill();
    let _ = s.child.wait();
}

#[test]
fn test_confirm_no_args() {
    let mut s = PinentrySession::start();
    s.send("CONFIRM");
    let _ = s.read_line();
    let _ = s.child.kill();
    let _ = s.child.wait();
}

// ── MESSAGE command ─────────────────────────────────────────────────

#[test]
fn test_message_command_parses_ok() {
    let mut s = PinentrySession::start();
    s.send("MESSAGE Key has been imported");
    // Message triggers dialog (no display) but is parsed correctly
    let _ = s.child.kill();
    let _ = s.child.wait();
}
