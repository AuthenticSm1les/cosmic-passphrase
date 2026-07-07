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

/// Shows the COSMIC passphrase/confirm/message dialog described by `config`
/// and blocks until the user dismisses it (or the configured timeout
/// elapses), returning the resulting [`DialogOutput`].
///
/// If the GUI fails to initialize (e.g. no Wayland/X11 display available),
/// this logs the error to stderr and returns [`DialogOutput::cancelled`]
/// rather than panicking, so a caller driven by a headless protocol (like
/// `pinentry-cosmic`'s Assuan loop) can still respond to its own client.
pub fn run_dialog(config: DialogConfig) -> DialogOutput {
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

    match cosmic::app::run::<CosmicDialog>(settings, flags) {
        Ok(()) => result
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .unwrap_or_else(DialogOutput::cancelled),
        Err(err) => {
            eprintln!("cosmic-passphrase: dialog failed: {}", err);
            DialogOutput::cancelled()
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

        content = content.push(
            widget::Space::new()
                .height(Length::Fixed(f32::from(spacing.space_m))),
        );

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

        content = content.push(button_row);

        widget::container(content)
            .width(Length::Fill)
            .height(Length::Shrink)
            .align_x(Alignment::Center)
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
