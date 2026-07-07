//! COSMIC GUI dialog for `cosmic-passphrase`.
//!
//! This crate is intentionally the *only* place in the workspace that
//! depends on `libcosmic` (and therefore on `iced`/`wgpu`/`winit`/X11 and
//! Wayland client libraries). Everything it needs to know about what to
//! show and what to hand back is expressed purely in terms of the headless
//! types from [`cosmic_passphrase_core`] (`DialogConfig` in, `DialogOutput`
//! out) — this crate has no knowledge of the Assuan protocol, D-Bus, or the
//! Secret Service, and `cosmic-passphrase-core` has no knowledge of GUI
//! toolkits.
//!
//! Keeping this split means anything that only needs the cache/config/output
//! logic (unit tests, a future headless tool, CI checks) never has to
//! compile the GUI stack at all.
//!
//! The single entry point is [`run_dialog`], which blocks the calling
//! thread until the user closes the dialog (or it times out) and returns
//! the resulting [`DialogOutput`].
//!
//! ## Why dialogs past the first one run in a child process
//!
//! `winit` (which `cosmic::app::run` is built on) hard-codes a process-wide
//! "one event loop per process, ever" rule — confirmed by reading its
//! Wayland backend source (a `static EVENT_LOOP_CREATED: AtomicBool`, no
//! opt-out) and by reproducing it live: a *second* `cosmic::app::run` call
//! in the same process reliably panics with `RecreationAttempt`, even with
//! a real display attached, not just headlessly.
//!
//! That matters because a single `pinentry-cosmic` process legitimately
//! shows more than one dialog over its lifetime — gpg-agent keeps one
//! pinentry process alive across a whole retry sequence (wrong passphrase
//! -> `SETERROR` -> `GETPIN` again, in the *same* process). Before this was
//! understood, that second dialog attempt would panic; [`run_dialog`] only
//! ever creates a real event loop for the *first* dialog a process shows.
//! Every dialog after that is delegated to a **freshly spawned child
//! process** (re-running the same binary with an internal marker env var —
//! see [`maybe_run_as_dialog_child`]), which gets its own untouched
//! one-event-loop budget. The `DialogConfig` is sent to the child and the
//! `DialogOutput` read back over the child's stdin/stdout pipes — not argv
//! or environment variables, since the *output* can contain a passphrase,
//! and both of those are visible to other processes on the system in ways
//! a private pipe isn't.
//!
//! This is also what makes it safe for a caller to show a separate
//! Allow/Deny [`DialogMode::Confirm`] dialog first when a cached passphrase
//! exists, then fall through to a normal [`DialogMode::Passphrase`] dialog
//! if the user picks Deny — two `run_dialog()` calls in the same process,
//! same as the retry sequence above. A crashed/failed child is retried once
//! (see `run_dialog_in_child`) before falling back to
//! [`DialogOutput::cancelled`], so a rare failure here is never mistaken for
//! a confirm and can never fabricate or leak a passphrase.

use std::io::Write as _;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cosmic::app::{Action as AppAction, Settings};
use cosmic::iced::{self, Alignment, Length, Limits, Size, Subscription};
use cosmic::widget;
use cosmic::{Application, ApplicationExt, Element, Task};
use zeroize::Zeroizing;

use cosmic_passphrase_core::config::{DialogConfig, DialogMode, ExtraContent};
use cosmic_passphrase_core::output::DialogOutput;

const APP_ID: &str = "com.system76.CosmicPassphrase";

/// Env var that marks a process as a re-exec'd dialog-only child (see
/// [`maybe_run_as_dialog_child`]) rather than a normal invocation of the
/// binary. Its value is unused; only presence matters.
const DIALOG_CHILD_MARKER: &str = "COSMIC_PASSPHRASE_DIALOG_CHILD";

/// Shows the COSMIC passphrase/confirm/message dialog described by `config`
/// and blocks until the user dismisses it (or the configured timeout
/// elapses), returning the resulting [`DialogOutput`].
///
/// If the GUI fails to initialize (e.g. no Wayland/X11 display available),
/// this logs the error to stderr and returns [`DialogOutput::cancelled`]
/// rather than panicking, so a caller driven by a headless protocol (like
/// `pinentry-cosmic`'s Assuan loop) can still respond to its own client.
///
/// Safe to call more than once in the same process — see the module docs
/// for why every call after the first transparently runs in a child
/// process instead of in-process.
pub fn run_dialog(config: DialogConfig) -> DialogOutput {
    static EVENT_LOOP_BUDGET_USED: AtomicBool = AtomicBool::new(false);
    if EVENT_LOOP_BUDGET_USED.swap(true, Ordering::SeqCst) {
        return run_dialog_in_child(&config);
    }
    run_dialog_in_process(config)
}

/// Must be called at the very start of `main()` in any binary that calls
/// [`run_dialog`] (before anything else — argument parsing, stdin reading,
/// etc.). If this process was spawned by [`run_dialog_in_child`] to show a
/// dialog on another process's behalf, this reads the `DialogConfig` from
/// stdin, shows it for real (this is a fresh process, so it has its own
/// untouched one-event-loop budget), writes the resulting `DialogOutput` to
/// stdout, and exits — this call never returns in that case. Otherwise
/// (the normal case) it returns immediately and the caller's real `main()`
/// proceeds as usual.
pub fn maybe_run_as_dialog_child() {
    if std::env::var_os(DIALOG_CHILD_MARKER).is_none() {
        return;
    }

    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        std::process::exit(1);
    }
    let Ok(wire_config) = serde_json::from_str::<WireDialogConfig>(&input) else {
        std::process::exit(1);
    };

    let output = run_dialog_in_process(wire_config.into());

    let wire_output = WireDialogOutput::from(&output);
    let Ok(json) = serde_json::to_string(&wire_output) else {
        std::process::exit(1);
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(json.as_bytes());
    let _ = stdout.flush();
    std::process::exit(0);
}

/// Spawns a fresh child process (re-running the current binary with
/// [`DIALOG_CHILD_MARKER`] set) to show `config`, and returns the
/// `DialogOutput` it reports back. See the module docs for why.
///
/// A crash or spawn failure is retried once before falling back to
/// [`DialogOutput::cancelled`] — observed live: showing several dialogs in
/// rapid succession this way occasionally hits what looks like a
/// Wayland-compositor-side race on reconnect (a `Bad file descriptor` I/O
/// error immediately followed by a child `SIGSEGV`), not reproduced across
/// a dozen further attempts, so a single retry is enough to ride it out
/// without weakening the fail-safe default for a genuine second failure.
/// This never masks the user actually choosing Cancel/closing the window —
/// that's a clean (`status.success()`) exit with `cancelled: true` in the
/// deserialized output, which is returned as-is on the first attempt.
fn run_dialog_in_child(config: &DialogConfig) -> DialogOutput {
    match try_run_dialog_in_child_once(config) {
        Ok(output) => output,
        Err(reason) => {
            eprintln!("cosmic-passphrase: {reason}; retrying once");
            match try_run_dialog_in_child_once(config) {
                Ok(output) => output,
                Err(reason) => {
                    eprintln!("cosmic-passphrase: retry also failed ({reason}); giving up");
                    DialogOutput::cancelled()
                }
            }
        }
    }
}

/// One attempt at [`run_dialog_in_child`]. `Err` carries a human-readable
/// reason and is only returned for failures that a retry might plausibly
/// route around (spawn failure, wait failure, non-success exit/crash) —
/// not for deterministic failures like a serialization bug, which a retry
/// cannot fix.
fn try_run_dialog_in_child_once(config: &DialogConfig) -> Result<DialogOutput, String> {
    let wire_config = WireDialogConfig::from(config);
    let Ok(json) = serde_json::to_string(&wire_config) else {
        eprintln!("cosmic-passphrase: failed to serialize dialog config for child process");
        return Ok(DialogOutput::cancelled());
    };

    let Ok(current_exe) = std::env::current_exe() else {
        eprintln!("cosmic-passphrase: could not determine current executable path");
        return Ok(DialogOutput::cancelled());
    };

    let child = std::process::Command::new(current_exe)
        .env(DIALOG_CHILD_MARKER, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to spawn dialog child process: {e}")),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(json.as_bytes());
        // Dropping `stdin` here closes it, signaling EOF to the child.
    }

    let Ok(output) = child.wait_with_output() else {
        return Err("failed to wait for dialog child process".to_string());
    };

    if !output.status.success() {
        return Err(format!(
            "dialog child process exited with {}",
            output.status
        ));
    }

    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return Ok(DialogOutput::cancelled());
    };
    let Ok(wire_output) = serde_json::from_str::<WireDialogOutput>(&stdout) else {
        return Ok(DialogOutput::cancelled());
    };
    Ok(wire_output.into())
}

/// The actual, in-process `cosmic::app::run` call. Only ever safe to call
/// once per process — callers must go through [`run_dialog`], which
/// enforces that.
fn run_dialog_in_process(config: DialogConfig) -> DialogOutput {
    let result: Arc<Mutex<Option<DialogOutput>>> = Arc::new(Mutex::new(None));
    let flags = DialogInit {
        config,
        result: result.clone(),
    };

    let (min_w, min_h, max_w, max_h) = (360.0, 200.0, 560.0, 600.0);
    let settings = Settings::default()
        .resizable(None)
        .size(Size::new(440.0, 280.0))
        .size_limits(
            Limits::NONE
                .min_width(min_w)
                .min_height(min_h)
                .max_width(max_w)
                .max_height(max_h),
        );

    // cosmic::app::run (via winit) doesn't always fail gracefully with an
    // Err when it can't initialize a window — confirmed empirically, it can
    // panic outright (e.g. no display available). Catch that so one failed
    // dialog attempt can't take the whole caller down with it —
    // pinentry-cosmic's Assuan loop, mid-session, needs to be able to
    // respond with an error instead of crashing entirely.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cosmic::app::run::<CosmicDialog>(settings, flags)
    }));

    match outcome {
        Ok(Ok(())) => result
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .unwrap_or_else(DialogOutput::cancelled),
        Ok(Err(err)) => {
            eprintln!("cosmic-passphrase: dialog failed: {}", err);
            DialogOutput::cancelled()
        }
        Err(_) => {
            eprintln!(
                "cosmic-passphrase: dialog panicked during initialization; treating as cancelled"
            );
            DialogOutput::cancelled()
        }
    }
}

// ── Wire format for the parent<->child dialog IPC ──────────────────────
//
// Deliberately separate from `cosmic_passphrase_core::config`/`output`'s
// types rather than adding serde derives to them directly: this
// serialization is purely an implementation detail of how this crate
// works around winit's one-event-loop-per-process limit, not something
// `cosmic-passphrase-core` (or anything consuming its public API) needs to
// know about.

#[derive(serde::Serialize, serde::Deserialize)]
struct WireDialogConfig {
    title: String,
    description: Option<String>,
    error: Option<String>,
    prompt: String,
    ok_label: String,
    cancel_label: String,
    notok_label: Option<String>,
    mode: WireDialogMode,
    extra: WireExtraContent,
    timeout_secs: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum WireDialogMode {
    Passphrase,
    Confirm,
    Message,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum WireExtraContent {
    None,
    Repeat,
    Remember,
}

impl From<&DialogConfig> for WireDialogConfig {
    fn from(c: &DialogConfig) -> Self {
        Self {
            title: c.title.clone(),
            description: c.description.clone(),
            error: c.error.clone(),
            prompt: c.prompt.clone(),
            ok_label: c.ok_label.clone(),
            cancel_label: c.cancel_label.clone(),
            notok_label: c.notok_label.clone(),
            mode: match c.mode {
                DialogMode::Passphrase => WireDialogMode::Passphrase,
                DialogMode::Confirm => WireDialogMode::Confirm,
                DialogMode::Message => WireDialogMode::Message,
            },
            extra: match c.extra {
                ExtraContent::None => WireExtraContent::None,
                ExtraContent::Repeat => WireExtraContent::Repeat,
                ExtraContent::Remember => WireExtraContent::Remember,
            },
            timeout_secs: c.timeout.map(|d| d.as_secs()),
        }
    }
}

impl From<WireDialogConfig> for DialogConfig {
    fn from(w: WireDialogConfig) -> Self {
        Self {
            title: w.title,
            description: w.description,
            error: w.error,
            prompt: w.prompt,
            ok_label: w.ok_label,
            cancel_label: w.cancel_label,
            notok_label: w.notok_label,
            mode: match w.mode {
                WireDialogMode::Passphrase => DialogMode::Passphrase,
                WireDialogMode::Confirm => DialogMode::Confirm,
                WireDialogMode::Message => DialogMode::Message,
            },
            extra: match w.extra {
                WireExtraContent::None => ExtraContent::None,
                WireExtraContent::Repeat => ExtraContent::Repeat,
                WireExtraContent::Remember => ExtraContent::Remember,
            },
            timeout: w.timeout_secs.map(std::time::Duration::from_secs),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WireDialogOutput {
    passphrase: Option<String>,
    confirmed: bool,
    cancelled: bool,
    remember: bool,
}

impl From<&DialogOutput> for WireDialogOutput {
    fn from(o: &DialogOutput) -> Self {
        Self {
            passphrase: o.passphrase.as_ref().map(|p| p.as_str().to_string()),
            confirmed: o.confirmed,
            cancelled: o.cancelled,
            remember: o.remember,
        }
    }
}

impl From<WireDialogOutput> for DialogOutput {
    fn from(w: WireDialogOutput) -> Self {
        Self {
            passphrase: w.passphrase.map(Zeroizing::new),
            confirmed: w.confirmed,
            cancelled: w.cancelled,
            remember: w.remember,
        }
    }
}

#[derive(Debug, Clone)]
struct DialogInit {
    config: DialogConfig,
    result: Arc<Mutex<Option<DialogOutput>>>,
}

struct CosmicDialog {
    core: cosmic::Core,
    config: DialogConfig,
    passphrase: Zeroizing<String>,
    passphrase_visible: bool,
    repeat_passphrase: Zeroizing<String>,
    repeat_visible: bool,
    remember: bool,
    repeat_error: Option<String>,
    result: Arc<Mutex<Option<DialogOutput>>>,
    timeout_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
enum Message {
    PassphraseChanged(String),
    RepeatPassphraseChanged(String),
    ToggleVisibility,
    ToggleRemember(bool),
    OkPressed,
    CancelPressed,
    NotOkPressed,
    CloseRequested(iced::window::Id),
    Tick,
}

impl Application for CosmicDialog {
    type Executor = cosmic::executor::Default;
    type Flags = DialogInit;
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut dialog = CosmicDialog {
            core,
            config: flags.config,
            passphrase: Zeroizing::new(String::new()),
            passphrase_visible: false,
            repeat_passphrase: Zeroizing::new(String::new()),
            repeat_visible: false,
            remember: false,
            repeat_error: None,
            result: flags.result,
            timeout_deadline: None,
        };

        dialog.timeout_deadline = dialog.config.timeout.map(|d| Instant::now() + d);

        let title = dialog.config.title.clone();
        let task = if let Some(id) = dialog.core.main_window_id() {
            dialog.set_window_title(title, id)
        } else {
            Task::none()
        };

        (dialog, task)
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let spacing = cosmic::theme::spacing();
        let theme = cosmic::theme::active();
        let destructive_color: iced::Color = theme.cosmic().destructive.base.into();

        let mut content = widget::Column::new()
            .spacing(f32::from(spacing.space_s))
            .padding(f32::from(spacing.space_m))
            .align_x(Alignment::Center);

        if let Some(ref desc) = self.config.description {
            content = content.push(
                widget::text::body(desc.as_str()).width(Length::Fill),
            );
        }

        if let Some(ref error) = self.config.error {
            content = content.push(
                widget::text::body(error.as_str())
                    .class(cosmic::theme::Text::Color(destructive_color))
                    .width(Length::Fill),
            );
            content = content.push(
                widget::Space::new()
                    .height(Length::Fixed(f32::from(spacing.space_xxs))),
            );
        }

        match self.config.mode {
            DialogMode::Confirm => {
                content = content.push(
                    widget::text::body(self.config.prompt.as_str())
                        .width(Length::Fill),
                );
            }
            DialogMode::Message => {}
            DialogMode::Passphrase => {
                content = content.push(
                    widget::text::body(self.config.prompt.as_str())
                        .width(Length::Fill),
                );

                let passphrase_input = widget::secure_input(
                    "",
                    self.passphrase.as_str(),
                    Some(Message::ToggleVisibility),
                    !self.passphrase_visible,
                )
                .on_input(Message::PassphraseChanged)
                .on_submit(|_| Message::OkPressed)
                .width(Length::Fixed(360.0));

                content = content.push(passphrase_input);

                if matches!(self.config.extra, ExtraContent::Repeat) {
                    content = content.push(
                        widget::Space::new()
                            .height(Length::Fixed(f32::from(spacing.space_xxs))),
                    );

                    content = content.push(
                        widget::text::body("Confirm passphrase:")
                            .width(Length::Fill),
                    );

                    let repeat_input = widget::secure_input(
                        "",
                        self.repeat_passphrase.as_str(),
                        Some(Message::ToggleVisibility),
                        !self.repeat_visible,
                    )
                    .on_input(Message::RepeatPassphraseChanged)
                    .on_submit(|_| Message::OkPressed)
                    .width(Length::Fixed(360.0));

                    content = content.push(repeat_input);

                    if let Some(ref err) = self.repeat_error {
                        content = content.push(
                            widget::text::body(err.as_str())
                                .class(cosmic::theme::Text::Color(
                                    destructive_color,
                                ))
                                .width(Length::Fill),
                        );
                    }
                }

                if matches!(self.config.extra, ExtraContent::Remember) {
                    content = content.push(
                        widget::Space::new()
                            .height(Length::Fixed(f32::from(spacing.space_xxs))),
                    );

                    content = content.push(
                        widget::checkbox(self.remember)
                            .label("Remember passphrase")
                            .on_toggle(Message::ToggleRemember),
                    );
                }
            }
        }

        let mut button_row = widget::Row::new()
            .spacing(f32::from(spacing.space_xs))
            .align_y(Alignment::Center);

        if !matches!(self.config.mode, DialogMode::Message) {
            button_row = button_row.push(
                widget::button::standard(&self.config.cancel_label)
                    .class(cosmic::theme::Button::Destructive)
                    .on_press(Message::CancelPressed),
            );
        }

        if let Some(ref notok) = self.config.notok_label {
            button_row = button_row.push(
                widget::button::standard(notok).on_press(Message::NotOkPressed),
            );
        }

        button_row = button_row.push(
            widget::button::suggested(&self.config.ok_label)
                .on_press(Message::OkPressed),
        );

        // The button row is deliberately kept *outside* the scrollable area
        // below, rather than as the last item in `content`: gpg-agent can
        // send an arbitrarily long, multi-line SETDESC (a real one seen in
        // testing was 4 lines), and this dialog's window is fixed-size and
        // non-resizable (see `run_dialog`'s `Settings`). Without this split,
        // a long enough description silently pushed the OK/Cancel buttons
        // below the visible window with no way to reach them — confirmed by
        // actually rendering the dialog, not just by reading the layout
        // code. Keeping buttons pinned outside the scroll area guarantees
        // they're always visible and clickable regardless of content length.
        let button_bar = widget::container(button_row)
            .width(Length::Fill)
            .padding(f32::from(spacing.space_m))
            .align_x(Alignment::Center);

        let layout = widget::Column::new()
            .push(widget::scrollable(content).height(Length::Fill))
            .push(button_bar);

        widget::container(layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        use iced::event;
        use iced::keyboard;
        use iced::keyboard::key::Named;

        let mut subs = Vec::new();

        subs.push(event::listen_with(|event, _status, _window| {
            if let event::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
                if key == keyboard::Key::Named(Named::Escape) {
                    return Some(Message::CancelPressed);
                }
            }
            None
        }));

        if self.timeout_deadline.is_some() {
            subs.push(iced::time::every(std::time::Duration::from_millis(500)).map(
                |_| Message::Tick,
            ));
        }

        Subscription::batch(subs)
    }

    fn update(
        &mut self,
        message: Self::Message,
    ) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::PassphraseChanged(text) => {
                self.passphrase = Zeroizing::new(text);
                self.repeat_error = None;
                Task::none()
            }
            Message::RepeatPassphraseChanged(text) => {
                self.repeat_passphrase = Zeroizing::new(text);
                self.repeat_error = None;
                Task::none()
            }
            Message::ToggleVisibility => {
                self.passphrase_visible = !self.passphrase_visible;
                self.repeat_visible = !self.repeat_visible;
                Task::none()
            }
            Message::ToggleRemember(checked) => {
                self.remember = checked;
                Task::none()
            }
            Message::OkPressed => {
                match self.config.mode {
                    DialogMode::Confirm | DialogMode::Message => {
                        self.set_result(DialogOutput::confirmed());
                    }
                    DialogMode::Passphrase => {
                        if matches!(self.config.extra, ExtraContent::Repeat)
                            && self.passphrase.as_str()
                                != self.repeat_passphrase.as_str()
                        {
                            self.repeat_error = Some(String::from(
                                "Passphrases do not match.",
                            ));
                            return Task::none();
                        }
                        let passphrase = Zeroizing::new(
                            self.passphrase.as_str().to_string(),
                        );
                        self.set_result(DialogOutput::ok_remember(
                            passphrase,
                            self.remember,
                        ));
                    }
                }
                Task::done(cosmic::Action::Cosmic(AppAction::Close))
            }
            Message::CancelPressed => {
                self.set_result(DialogOutput::cancelled());
                Task::done(cosmic::Action::Cosmic(AppAction::Close))
            }
            Message::NotOkPressed => {
                self.set_result(DialogOutput::not_confirmed());
                Task::done(cosmic::Action::Cosmic(AppAction::Close))
            }
            Message::CloseRequested(_id) => {
                let already_set = self
                    .result
                    .lock()
                    .map(|g| g.is_some())
                    .unwrap_or(false);
                if !already_set {
                    self.set_result(DialogOutput::cancelled());
                }
                Task::none()
            }
            Message::Tick => {
                if let Some(deadline) = self.timeout_deadline {
                    if Instant::now() >= deadline {
                        self.set_result(DialogOutput::cancelled());
                        return Task::done(cosmic::Action::Cosmic(AppAction::Close));
                    }
                }
                Task::none()
            }
        }
    }

    fn on_close_requested(
        &self,
        _id: iced::window::Id,
    ) -> Option<Message> {
        Some(Message::CloseRequested(_id))
    }
}

impl CosmicDialog {
    fn set_result(&mut self, result: DialogOutput) {
        if let Ok(mut guard) = self.result.lock() {
            *guard = Some(result);
        }
    }
}
