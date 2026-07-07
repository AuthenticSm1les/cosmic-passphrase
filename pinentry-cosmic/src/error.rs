//! GPG error codes for the pinentry-cosmic application.
//!
//! This module provides the standard GPG error codes required by the
//! Assuan pinentry protocol.
//!
//! # GPG Error Codes
//!
//! The error codes are defined by libgpg-error and are stable.
//! The codes actually used in this crate are:
//!
//! | Code | Name           | Description                        |
//! |------|----------------|------------------------------------|
//! | 99   | CANCELED       | User cancelled the operation       |
//! | 257  | ASS_GENERAL    | General Assuan protocol error      |
//! | 48   | NOT_CONFIRMED  | Confirmation was denied            |

/// GPG error codes used in the Assuan pinentry protocol.
///
/// These are stable constants from libgpg-error. They must match
/// exactly what gpg-agent expects.
pub mod gpg {
    /// User cancelled the operation.
    pub const CANCELED: u32 = 99;
    /// General Assuan protocol error code.
    pub const ASS_GENERAL: u32 = 257;
    /// Confirmation dialog was denied (notok button).
    pub const NOT_CONFIRMED: u32 = 48;
}
