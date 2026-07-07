//! Headless core for `cosmic-passphrase`: passphrase caching and the plain
//! data types used to describe and receive the result of a dialog.
//!
//! This crate deliberately has **no GUI dependency**. The dialog itself
//! (which does depend on `libcosmic`) lives in the separate
//! `cosmic-passphrase-dialog` crate and only talks to this one through the
//! [`config::DialogConfig`] / [`output::DialogOutput`] types. Keeping the
//! split lets anything that only needs caching or configuration logic
//! (unit tests, a future headless tool) build in seconds instead of
//! pulling in the full GUI toolchain (`iced`/`wgpu`/`winit`/X11/Wayland).
//!
//! - [`cache`] — the [`cache::CacheBackend`] trait and its implementations
//!   ([`cache::DbusBackend`] backed by the D-Bus Secret Service / oo7,
//!   [`cache::NullBackend`] for tests).
//! - [`config`] — [`config::DialogConfig`], the input to a dialog.
//! - [`output`] — [`output::DialogOutput`], the result of a dialog.

pub mod config;
pub mod output;
pub mod cache;
