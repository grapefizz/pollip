use super::client::{validate_api_key, ClientError, ValidateResult};
use super::key::{clear_api_key, load_api_key, save_api_key};
use super::nxm::{handler_is_registered, register_nxm_handler};
use crate::logging;
use crate::ui::{muted_text, title_text, ui_text};
use crate::ui::ToastQueue;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

enum ValidateMessage {
    Ready(ValidateResult),
    Failed(String),
}

pub struct NexusSettings {
    key_input: String,
    show_key: bool,
    account: Option<ValidateResult>,
    error_message: Option<String>,
    status_message: Option<String>,
    validating: bool,
    validate_from_user: bool,
    validate_rx: Option<Receiver<ValidateMessage>>,
    loaded: bool,
    handler_registered: bool,
    catalog_refresh_pending: bool,
}

impl Default for NexusSettings {
    fn default() -> Self {
        Self {
            key_input: String::new(),
            show_key: false,
            account: None,
            error_message: None,
            status_message: None,
            validating: false,
            validate_from_user: false,
            validate_rx: None,
            loaded: false,
            handler_registered: false,
            catalog_refresh_pending: false,
        }
    }
}

impl NexusSettings {
    pub fn account(&self) -> Option<&ValidateResult> {
        self.account.as_ref()
    }

    pub fn take_catalog_refresh(&mut self) -> bool {
        let pending = self.catalog_refresh_pending;
        self.catalog_refresh_pending = false;
        pending
    }

    pub fn tick(&mut self, ctx: &egui::Context, toasts: &mut ToastQueue) {
        if !self.loaded {
            self.loaded = true;
            self.handler_registered = handler_is_registered();
            match load_api_key() {
                Ok(Some(key)) => {
                    self.key_input = key;
                    self.start_validate(ctx, false);
                }
                Ok(None) => {}
                Err(error) => {
                    self.error_message = Some(error.to_string());
                    logging::error(error.to_string());
                }
            }
        }
        self.poll_validate(ctx, toasts);
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        self.tick(ui.ctx(), toasts);

        ui.label(title_text("nexus mods"));
        ui.add_space(6.0);
        ui.label(muted_text(
            "paste your personal api key from nexusmods.com/users/myaccount?tab=api. it is stored only on this machine and sent only to the nexus api.",
        ));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(ui_text("api key"));
            let edit = egui::TextEdit::singleline(&mut self.key_input)
                .password(!self.show_key)
                .desired_width(360.0)
                .hint_text("paste api key");
            ui.add(edit);
            let reveal_label = if self.show_key { "hide" } else { "reveal" };
            if ui.button(ui_text(reveal_label)).clicked() {
                self.show_key = !self.show_key;
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_save = !self.key_input.trim().is_empty() && !self.validating;
            if ui
                .add_enabled(can_save, egui::Button::new(ui_text("save & validate")))
                .clicked()
            {
                self.save_and_validate(ui.ctx(), toasts);
            }
            if ui
                .add_enabled(!self.validating, egui::Button::new(ui_text("clear key")))
                .clicked()
            {
                self.clear_key(toasts);
            }
        });

        ui.add_space(8.0);

        if self.validating {
            ui.label(muted_text("validating api key with nexus…"));
        }

        if let Some(error) = &self.error_message {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }

        if let Some(account) = &self.account {
            let tier = if account.is_premium {
                "premium"
            } else {
                "free (non-premium)"
            };
            ui.colored_label(
                ui.visuals().strong_text_color(),
                format!("signed in as {} · {tier}", account.username),
            );
            if account.is_supporter && !account.is_premium {
                ui.label(muted_text("supporter account"));
            }
        } else if let Some(status) = &self.status_message {
            ui.label(muted_text(status.clone()));
        }

        ui.add_space(12.0);
        ui.label(ui_text("mod manager downloads (nxm://)"));
        ui.add_space(4.0);
        ui.label(muted_text(
            "required for free accounts: register this app so “mod manager download” on the nexus website opens here.",
        ));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if self.handler_registered {
                ui.label(muted_text("nxm handler registered"));
            } else {
                ui.colored_label(ui.visuals().warn_fg_color, "nxm handler not registered");
            }
            if ui.button(ui_text("register nxm handler")).clicked() {
                match register_nxm_handler() {
                    Ok(path) => {
                        self.handler_registered = true;
                        let message = format!("registered nxm handler at {}", path.display());
                        self.status_message = Some(message.clone());
                        logging::info(&message);
                        toasts.success("registered as nxm:// handler");
                    }
                    Err(error) => {
                        self.error_message = Some(error.to_string());
                        logging::error(error.to_string());
                        toasts.error(error.to_string());
                    }
                }
            }
        });
    }

    fn save_and_validate(&mut self, ctx: &egui::Context, toasts: &mut ToastQueue) {
        let key = self.key_input.trim().to_string();
        if key.is_empty() {
            self.error_message = Some("api key cannot be empty".to_string());
            return;
        }
        match save_api_key(&key) {
            Ok(()) => {
                logging::info("saved nexus api key locally");
                toasts.info("api key saved locally");
                self.start_validate(ctx, true);
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn clear_key(&mut self, toasts: &mut ToastQueue) {
        match clear_api_key() {
            Ok(()) => {
                self.key_input.clear();
                self.account = None;
                self.error_message = None;
                self.status_message = Some("nexus api key cleared".to_string());
                logging::info("cleared nexus api key");
                toasts.info("nexus api key cleared");
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn start_validate(&mut self, ctx: &egui::Context, from_user: bool) {
        if self.validating {
            return;
        }
        let key = self.key_input.trim().to_string();
        if key.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.validate_rx = Some(rx);
        self.validating = true;
        self.validate_from_user = from_user;
        self.error_message = None;
        if from_user {
            self.status_message = Some("validating…".to_string());
        }
        let ctx = ctx.clone();
        thread::spawn(move || {
            let message = match validate_api_key(&key) {
                Ok(result) => ValidateMessage::Ready(result),
                Err(error) => ValidateMessage::Failed(format_validate_error(error)),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    fn poll_validate(&mut self, ctx: &egui::Context, toasts: &mut ToastQueue) {
        let Some(rx) = &self.validate_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(ValidateMessage::Ready(result)) => {
                self.validating = false;
                self.validate_rx = None;
                self.error_message = None;
                let tier = if result.is_premium {
                    "premium"
                } else {
                    "free"
                };
                self.status_message = Some(format!("validated · {} · {tier}", result.username));
                logging::info(format!(
                    "nexus api key validated for user {} ({tier})",
                    result.username
                ));
                toasts.success(format!("nexus: signed in as {}", result.username));
                self.account = Some(result);
                if self.validate_from_user {
                    self.catalog_refresh_pending = true;
                }
                self.validate_from_user = false;
                if self.account.as_ref().is_some_and(|a| !a.is_premium)
                    && !self.handler_registered
                {
                    if let Ok(path) = register_nxm_handler() {
                        self.handler_registered = true;
                        logging::info(format!(
                            "auto-registered nxm handler at {}",
                            path.display()
                        ));
                        toasts.info("registered nxm:// handler for free-account downloads");
                    }
                }
                ctx.request_repaint();
            }
            Ok(ValidateMessage::Failed(message)) => {
                self.validating = false;
                self.validate_rx = None;
                self.validate_from_user = false;
                self.account = None;
                self.error_message = Some(message.clone());
                self.status_message = None;
                logging::error(&message);
                toasts.error(message);
                ctx.request_repaint();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.validating = false;
                self.validate_rx = None;
                self.validate_from_user = false;
                let message = "nexus validation worker stopped unexpectedly".to_string();
                self.error_message = Some(message.clone());
                toasts.error(message);
            }
        }
    }
}

fn format_validate_error(error: ClientError) -> String {
    match error {
        ClientError::Unauthorized => {
            "invalid nexus api key — check the key and try again".to_string()
        }
        ClientError::RateLimited => {
            "rate limited by nexus mods while validating — try again later".to_string()
        }
        other => other.to_string(),
    }
}
