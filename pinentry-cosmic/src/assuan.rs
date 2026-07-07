//! Assuan protocol implementation for the pinentry server.
//!
//! This module implements the server side of the [Assuan protocol],
//! a simple line-based IPC protocol used by GnuPG. The pinentry program
//! communicates with `gpg-agent` via stdin/stdout using this protocol.
//!
//! # Protocol Overview
//!
//! Commands are single lines of the form `COMMAND [parameters...]`.
//! Responses follow these formats:
//!
//! | Response | Format                       | Meaning                     |
//! |----------|------------------------------|-----------------------------|
//! | OK       | `OK [details]`              | Success                     |
//! | ERR      | `ERR <code> <description>`  | Error with GPG error code   |
//! | D        | `D <percent-encoded-data>`  | Data line (passphrase)      |
//! | S        | `S <keyword> <info>`        | Status information          |
//! | INQUIRE  | `INQUIRE <keyword> [params]`| Request data from client    |
//! | #        | `# <comment>`               | Debug comment (ignored)     |
//!
//! Lines are limited to 1000 characters per the Assuan specification.
//!
//! [Assuan protocol]: https://www.gnupg.org/documentation/manuals/assuan/

use std::io::{self, Write};

/// Maximum line length per the Assuan specification (section 3.1).
const MAX_LINE_LENGTH: usize = 1000;

/// Represents a parsed Assuan command received from the client (gpg-agent).
///
/// Each variant corresponds to a command that gpg-agent can send
/// to configure the pinentry dialog or trigger an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // -----------------------------------------------------------------
    // Session setup commands
    // -----------------------------------------------------------------

    /// `OPTION <name>[=<value>]` - Set a pinentry option.
    /// Used for display, ttyname, timeout, grab, parent-wid, etc.
    Option_(String, Option<String>),

    /// `SETTITLE <string>` - Set the window title.
    SetTitle(String),

    /// `SETDESC <desc>` - Set the description text shown in the dialog.
    SetDesc(String),

    /// `SETPROMPT <prompt>` - Set the prompt label near the input field.
    SetPrompt(String),

    /// `SETERROR <message>` - Set an error message from a previous failed attempt.
    SetError(String),

    /// `SETOK <label>` - Set the text for the OK/confirm button.
    SetOk(String),

    /// `SETCANCEL <label>` - Set the text for the Cancel button.
    SetCancel(String),

    /// `SETNOTOK <label>` - Set the text for the alternative button in confirm mode.
    SetNotOk(String),

    /// `SETREPEAT` - Enable repeat passphrase entry (confirmation field).
    SetRepeat,

    /// `SETREPEATERROR <msg>` - Error shown when repeat passphrases don't match.
    SetRepeatError(String),

    /// `SETREPEATOK <msg>` - Message shown when repeat passphrases match.
    SetRepeatOk(String),

    /// `SETQUALITYBAR <label>` - Enable quality bar with the given label.
    SetQualityBar(String),

    /// `SETQUALITYBAR_TT <tooltip>` - Set the quality bar tooltip text.
    SetQualityBarTt(String),

    /// `SETGENPIN <label>` - Enable PIN generation with the given label.
    SetGenPin(String),

    /// `SETGENPIN_TT <tooltip>` - Set the PIN generation tooltip text.
    SetGenPinTt(String),

    /// `SETKEYINFO <info>` - Set key information for password caching.
    SetKeyInfo(String),

    /// `MESSAGE <text>` - Display a simple informational message (one button).
    Message(String),

    // -----------------------------------------------------------------
    // Action commands
    // -----------------------------------------------------------------

    /// `GETPIN` - Show the passphrase entry dialog and return the passphrase.
    GetPin,

    /// `CONFIRM [--one-button]` - Show a confirmation dialog.
    /// With `--one-button`, only the OK button is shown.
    Confirm { one_button: bool },

    // -----------------------------------------------------------------
    // Session management
    // -----------------------------------------------------------------

    /// `RESET` - Reset all pinentry state to defaults.
    Reset,

    /// `BYE` - Close the connection and exit.
    Bye,

    /// `NOP` - No operation (do nothing, respond OK).
    Nop,

    // -----------------------------------------------------------------
    // Inquiry response commands (during dialog)
    // -----------------------------------------------------------------

    /// `END` - End of data transmission in response to an INQUIRE.
    End,

    /// `CAN` or `CANCEL` - Cancel the current inquiry operation.
    Cancel,

    /// `D <percent-encoded-data>` - Data line in response to an INQUIRE.
    Data(String),

    /// An unrecognized command. Pinentry must silently ignore unknown commands.
    Unknown(String),
}

/// Parses a single Assuan command line into a [`Command`].
///
/// # Arguments
///
/// * `line` - A single line from the client, including the trailing newline
///   which will be trimmed.
///
/// # Returns
///
/// The parsed command. Unknown commands return [`Command::Unknown`].
/// Empty lines and comments (`#`) return [`Command::Nop`].
///
/// # Examples
///
/// ```
/// # use pinentry_cosmic::assuan::{Command, parse_command};
/// // (doc tests reference the crate name; unit tests use `crate::` directly)
/// assert_eq!(parse_command("SETDESC Enter passphrase\n"), Command::SetDesc("Enter passphrase".into()));
/// assert_eq!(parse_command("GETPIN\n"), Command::GetPin);
/// assert_eq!(parse_command("BYE\n"), Command::Bye);
/// assert_eq!(parse_command("# comment\n"), Command::Nop);
/// ```
pub fn parse_command(line: &str) -> Command {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Command::Nop;
    }

    if line.len() > MAX_LINE_LENGTH {
        // Truncated line, treat as unknown to avoid processing truncated commands
        return Command::Unknown(line[..MAX_LINE_LENGTH].to_string());
    }

    let (cmd, args) = match line.find(' ') {
        Some(pos) => (&line[..pos], line[pos + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "OPTION" => parse_option(args),
        "SETTITLE" => Command::SetTitle(args.to_string()),
        "SETDESC" => Command::SetDesc(args.to_string()),
        "SETPROMPT" => Command::SetPrompt(args.to_string()),
        "SETERROR" => Command::SetError(args.to_string()),
        "SETOK" => Command::SetOk(args.to_string()),
        "SETCANCEL" => Command::SetCancel(args.to_string()),
        "SETNOTOK" => Command::SetNotOk(args.to_string()),
        "SETREPEAT" => Command::SetRepeat,
        "SETREPEATERROR" => Command::SetRepeatError(args.to_string()),
        "SETREPEATOK" => Command::SetRepeatOk(args.to_string()),
        "SETQUALITYBAR" => Command::SetQualityBar(args.to_string()),
        "SETQUALITYBAR_TT" => Command::SetQualityBarTt(args.to_string()),
        "SETGENPIN" => Command::SetGenPin(args.to_string()),
        "SETGENPIN_TT" => Command::SetGenPinTt(args.to_string()),
        "SETKEYINFO" => Command::SetKeyInfo(args.to_string()),
        "GETPIN" => Command::GetPin,
        "CONFIRM" => {
            let one_button = args.contains("--one-button");
            Command::Confirm { one_button }
        }
        "MESSAGE" => Command::Message(args.to_string()),
        "RESET" => Command::Reset,
        "BYE" => Command::Bye,
        "NOP" => Command::Nop,
        "END" => Command::End,
        "CAN" | "CANCEL" => Command::Cancel,
        // Data line: "D <percent-encoded-data>"
        cmd if cmd.len() == 1 && cmd.starts_with('D') => {
            let data = line.get(2..).unwrap_or("");
            Command::Data(percent_decode(data))
        }
        _ => Command::Unknown(cmd.to_string()),
    }
}

/// Parses an `OPTION name[=value]` command.
fn parse_option(args: &str) -> Command {
    if args.is_empty() {
        Command::Option_(String::new(), None)
    } else if let Some(eq_pos) = args.find('=') {
        let name = args[..eq_pos].trim().to_string();
        let value = if eq_pos + 1 < args.len() {
            args[eq_pos + 1..].trim().to_string()
        } else {
            String::new()
        };
        Command::Option_(name, if value.is_empty() { None } else { Some(value) })
    } else {
        Command::Option_(args.to_string(), None)
    }
}

/// Writes an `OK` response to the writer.
///
/// # Arguments
///
/// * `writer` - A writer, typically stdout.
/// * `details` - Optional details string. If empty, writes just "OK".
pub fn write_ok<W: Write>(writer: &mut W, details: &str) -> io::Result<()> {
    if details.is_empty() {
        writeln!(writer, "OK")
    } else {
        writeln!(writer, "OK {}", details)
    }
}

/// Writes an `ERR` response with a GPG error code.
///
/// # Arguments
///
/// * `writer` - A writer, typically stdout.
/// * `code` - The GPG error code (e.g., `gpg::CANCELED`).
/// * `description` - A human-readable error description.
pub fn write_err<W: Write>(writer: &mut W, code: u32, description: &str) -> io::Result<()> {
    writeln!(writer, "ERR {} {} <pinentry-cosmic>", code, description)
}

/// Writes a `D` (data) line with percent-encoded content.
///
/// This is used to return the passphrase after a `GETPIN` command.
/// Special characters (`%`, `\n`, `\r`, and non-ASCII) are percent-encoded.
///
/// # Arguments
///
/// * `writer` - A writer, typically stdout.
/// * `data` - The raw data to encode and send.
pub fn write_data<W: Write>(writer: &mut W, data: &str) -> io::Result<()> {
    let encoded = percent_encode(data);
    writeln!(writer, "D {}", encoded)
}

/// Writes a status (`S`) line.
///
/// Status lines provide informational output during processing.
/// This is not commonly used by pinentry but is part of the Assuan spec.
///
/// # Arguments
///
/// * `writer` - A writer, typically stdout.
/// * `keyword` - The status keyword.
/// * `info` - The status information string.
pub fn write_status<W: Write>(writer: &mut W, keyword: &str, info: &str) -> io::Result<()> {
    writeln!(writer, "S {} {}", keyword, info)
}

/// Percent-encodes a string for the `D` (data) line.
///
/// The following characters are always encoded:
/// - `%` → `%25`
/// - `\n` (LF) → `%0A`
/// - `\r` (CR) → `%0D`
/// - Other non-printable or non-ASCII bytes → `%XX` (uppercase hex)
///
/// This follows the Assuan specification for data encoding.
pub fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'%' => result.push_str("%25"),
            b'\n' => result.push_str("%0A"),
            b'\r' => result.push_str("%0D"),
            b if !b.is_ascii_graphic() && b != b' ' => {
                result.push_str(&format!("%{:02X}", byte));
            }
            _ => result.push(byte as char),
        }
    }
    result
}

/// Percent-decodes a string from a `D` (data) line.
///
/// Any `%XX` sequences are converted back to their byte values.
/// Invalid hex sequences are left as-is.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &bytes[i + 1..i + 3];
            if let Ok(hex_str) = std::str::from_utf8(hex)
                && let Ok(byte) = u8::from_str_radix(hex_str, 16)
            {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // parse_command tests
    // -----------------------------------------------------------------

    #[test]
    fn test_parse_empty_line() {
        assert_eq!(parse_command("\n"), Command::Nop);
        assert_eq!(parse_command(""), Command::Nop);
    }

    #[test]
    fn test_parse_comment() {
        assert_eq!(parse_command("# this is a comment\n"), Command::Nop);
    }

    #[test]
    fn test_parse_setdesc() {
        assert_eq!(
            parse_command("SETDESC Enter passphrase\n"),
            Command::SetDesc("Enter passphrase".into())
        );
    }

    #[test]
    fn test_parse_setdesc_empty() {
        // Empty args should still work (clears the description)
        assert_eq!(parse_command("SETDESC \n"), Command::SetDesc(String::new()));
    }

    #[test]
    fn test_parse_getpin() {
        assert_eq!(parse_command("GETPIN\n"), Command::GetPin);
    }

    #[test]
    fn test_parse_confirm() {
        assert_eq!(
            parse_command("CONFIRM\n"),
            Command::Confirm { one_button: false }
        );
    }

    #[test]
    fn test_parse_confirm_one_button() {
        assert_eq!(
            parse_command("CONFIRM --one-button\n"),
            Command::Confirm { one_button: true }
        );
    }

    #[test]
    fn test_parse_option_with_value() {
        assert_eq!(
            parse_command("OPTION ttyname=/dev/pts/0\n"),
            Command::Option_("ttyname".into(), Some("/dev/pts/0".into()))
        );
    }

    #[test]
    fn test_parse_option_without_value() {
        assert_eq!(
            parse_command("OPTION grab\n"),
            Command::Option_("grab".into(), None)
        );
    }

    #[test]
    fn test_parse_option_empty() {
        assert_eq!(
            parse_command("OPTION\n"),
            Command::Option_(String::new(), None)
        );
    }

    #[test]
    fn test_parse_data_line() {
        assert_eq!(
            parse_command("D hello%20world\n"),
            Command::Data("hello world".into())
        );
    }

    #[test]
    fn test_parse_setrepeat() {
        assert_eq!(parse_command("SETREPEAT\n"), Command::SetRepeat);
    }

    #[test]
    fn test_parse_setqualitybar() {
        assert_eq!(
            parse_command("SETQUALITYBAR Passphrase quality:\n"),
            Command::SetQualityBar("Passphrase quality:".into())
        );
    }

    #[test]
    fn test_parse_end() {
        assert_eq!(parse_command("END\n"), Command::End);
    }

    #[test]
    fn test_parse_cancel() {
        assert_eq!(parse_command("CAN\n"), Command::Cancel);
        assert_eq!(parse_command("CANCEL\n"), Command::Cancel);
    }

    #[test]
    fn test_parse_unknown() {
        assert_eq!(
            parse_command("FOOBAR baz\n"),
            Command::Unknown("FOOBAR".into())
        );
    }

    #[test]
    fn test_parse_long_line_truncation() {
        let long = "X".repeat(MAX_LINE_LENGTH + 10);
        let line = format!("{}\n", long);
        match parse_command(&line) {
            Command::Unknown(s) => assert!(s.len() <= MAX_LINE_LENGTH),
            other => panic!("Expected Unknown, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // percent_encode / percent_decode tests
    // -----------------------------------------------------------------

    #[test]
    fn test_percent_encode_decode_roundtrip() {
        let originals = vec![
            "hello world",
            "pass%phrase",
            "line1\nline2",
            "carriage\rreturn",
            "normal text with spaces",
            "café",
            "",
        ];

        for original in originals {
            let encoded = percent_encode(original);
            let decoded = percent_decode(&encoded);
            assert_eq!(
                decoded, original,
                "Roundtrip failed for: {:?}",
                original
            );
        }
    }

    #[test]
    fn test_percent_encode_special_chars() {
        assert_eq!(percent_encode("%"), "%25");
        assert_eq!(percent_encode("\n"), "%0A");
        assert_eq!(percent_encode("\r"), "%0D");
        assert_eq!(percent_encode("hello\nworld%"), "hello%0Aworld%25");
    }

    #[test]
    fn test_percent_decode_invalid_hex() {
        // Invalid hex sequences should be left as-is
        assert_eq!(percent_decode("%XX"), "%XX");
        assert_eq!(percent_decode("%G0"), "%G0");
    }

    #[test]
    fn test_percent_decode_partial_hex() {
        // Partial hex at end should be left as-is
        assert_eq!(percent_decode("hello%2"), "hello%2");
    }

    // -----------------------------------------------------------------
    // Response writer tests
    // -----------------------------------------------------------------

    #[test]
    fn test_write_ok() {
        let mut buf = Vec::new();
        write_ok(&mut buf, "").unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "OK\n");
    }

    #[test]
    fn test_write_ok_with_details() {
        let mut buf = Vec::new();
        write_ok(&mut buf, "closing connection").unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "OK closing connection\n"
        );
    }

    #[test]
    fn test_write_err() {
        let mut buf = Vec::new();
        write_err(&mut buf, 99, "canceled").unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("ERR 99 canceled"));
    }

    #[test]
    fn test_write_data() {
        let mut buf = Vec::new();
        write_data(&mut buf, "secret%pass").unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "D secret%25pass\n"
        );
    }

}
