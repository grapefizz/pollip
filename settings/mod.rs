mod bepinex;
mod preferences;

pub use bepinex::{
    install_recommended, launch_silksong, recommended_status_for, reset_launch_script,
    BepinexStatus, InstallSummary,
};
pub use preferences::{load_preferences, mark_setup_complete, remember_install_folder};

use crate::detection::{self, DetectionError, SilksongInstall};
use crate::logging;
use crate::nexus::NexusSettings;
use crate::ui::section_heading;
use crate::ui::ToastQueue;
use bepinex::{
    build_kind_launch_hint, configure_injection, inspect_injection, open_launch_script,
    status_label, InjectionState, LaunchOptionsOutcome, LaunchScriptAction,
    RECOMMENDED_PACK_FULL_NAME, SMM_LAUNCH_SCRIPT,
};
use eframe::egui;

enum PendingConfirm {
    InstallBepinex { button_label: String },
    ResetLaunchScript,
}

pub struct SettingsPanel {
    install: Option<SilksongInstall>,
    bepinex_status: Option<BepinexStatus>,
    injection_state: Option<InjectionState>,
    injection_alert: Option<String>,
    status_message: Option<String>,
    bepinex_log: Option<String>,
    dry_run: bool,
    busy_message: Option<String>,
    pending_confirm: Option<PendingConfirm>,
    nexus: NexusSettings,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self {
            install: None,
            bepinex_status: None,
            injection_state: None,
            injection_alert: None,
            status_message: None,
            bepinex_log: None,
            dry_run: false,
            busy_message: None,
            pending_confirm: None,
            nexus: NexusSettings::default(),
        }
    }
}

impl SettingsPanel {
    pub fn install(&self) -> Option<&SilksongInstall> {
        self.install.as_ref()
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub fn busy_message(&self) -> Option<&str> {
        self.busy_message.as_deref()
    }

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    pub fn set_busy_message(&mut self, message: Option<String>) {
        self.busy_message = message;
    }

    pub fn accept_install_quiet(&mut self, install: SilksongInstall) {
        self.bepinex_status = Some(recommended_status_for(&install));
        self.injection_state = Some(inspect_injection(&install));
        self.injection_alert = injection_alert_from_state(self.injection_state.as_ref());
        self.install = Some(install);
        self.bepinex_log = None;
        self.status_message = None;
    }

    pub fn refresh_bepinex_status(&mut self) {
        let Some(install) = self.install.clone() else {
            return;
        };
        self.bepinex_status = Some(recommended_status_for(&install));
        self.injection_state = Some(inspect_injection(&install));
        self.injection_alert = injection_alert_from_state(self.injection_state.as_ref());
    }

    pub fn nexus(&self) -> &NexusSettings {
        &self.nexus
    }

    pub fn nexus_mut(&mut self) -> &mut NexusSettings {
        &mut self.nexus
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        ui.label(section_heading("settings"));
        ui.add_space(14.0);

        self.draw_install_section(ui, toasts);
        ui.add_space(20.0);
        self.draw_bepinex_section(ui, toasts);
        ui.add_space(20.0);
        self.nexus.draw(ui, toasts);
        ui.add_space(20.0);
        self.draw_diagnostics_section(ui, toasts);
        self.draw_pending_confirm(ui, toasts);
    }

    fn draw_install_section(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        ui.label("silksong install");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button("detect game").clicked() {
                self.run_automatic_detection(toasts);
            }
            if ui.button("choose folder").clicked() {
                self.pick_install_folder(toasts);
            }
        });

        ui.add_space(12.0);

        if let Some(message) = &self.status_message {
            ui.colored_label(ui.visuals().warn_fg_color, message);
            ui.add_space(8.0);
        }

        match &self.install {
            Some(install) => {
                egui::Grid::new("silksong_install_details")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("folder");
                        ui.monospace(install.install_folder.display().to_string());
                        ui.end_row();

                        ui.label("executable");
                        ui.monospace(install.executable.display().to_string());
                        ui.end_row();

                        ui.label("build");
                        ui.label(install.build_kind.to_string());
                        ui.end_row();

                        ui.label("proton prefix");
                        match &install.proton_prefix {
                            Some(prefix) => ui.monospace(prefix.display().to_string()),
                            None => ui.label("none"),
                        };
                        ui.end_row();
                    });
            }
            None => {
                ui.label("no install selected yet");
            }
        }
    }

    fn draw_bepinex_section(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        ui.label("bepinex");
        ui.add_space(6.0);

        ui.label(format!("recommended pack: {RECOMMENDED_PACK_FULL_NAME}"));
        ui.add_space(8.0);

        if let Some(alert) = &self.injection_alert {
            ui.colored_label(ui.visuals().error_fg_color, alert);
            ui.add_space(8.0);
        }

        let has_install = self.install.is_some();
        let status = self.bepinex_status.clone();
        let injection = self.injection_state.clone();

        if !has_install {
            ui.label("select a silksong install before setting up bepinex");
        } else if let Some(status) = status {
            ui.horizontal(|ui| {
                ui.label("pack status");
                ui.strong(status_label(&status));
            });

            match &status {
                BepinexStatus::NeedsUpdate {
                    pack_version,
                    bepinex_version,
                    recommended_pack_version,
                } => {
                    ui.label(format!(
                        "installed pack {}, bepinex {}, recommended pack {recommended_pack_version}",
                        pack_version.as_deref().unwrap_or("unknown"),
                        bepinex_version.as_deref().unwrap_or("unknown"),
                    ));
                }
                BepinexStatus::InstalledCurrent {
                    pack_version,
                    bepinex_version,
                } => {
                    ui.label(format!(
                        "pack {}, bepinex {}",
                        pack_version.as_deref().unwrap_or("unknown"),
                        bepinex_version.as_deref().unwrap_or("unknown"),
                    ));
                }
                BepinexStatus::NotInstalled => {}
            }

            if let Some(injection) = &injection {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("injection");
                    if injection.is_configured() {
                        ui.strong(injection.label());
                    } else {
                        ui.colored_label(ui.visuals().error_fg_color, injection.label());
                    }
                });

                match injection {
                    InjectionState::NotConfigured {
                        current_launch_options,
                        required_launch_options,
                        reason,
                        launch_script,
                    } => {
                        ui.colored_label(ui.visuals().error_fg_color, reason);
                        if let Some(current) = current_launch_options {
                            ui.label(format!("current steam launch options: {current}"));
                        }
                        if let Some(script) = launch_script {
                            ui.monospace(format!("editable launch script: {}", script.display()));
                        }
                        ui.label("steam launch options target:");
                        ui.horizontal(|ui| {
                            ui.monospace(required_launch_options);
                            if ui.button("copy").clicked() {
                                ui.ctx().copy_text(required_launch_options.clone());
                                toasts.info("copied launch options");
                            }
                        });
                    }
                    InjectionState::ConfiguredAwaitingLaunch {
                        launch_options,
                        launch_script,
                        steam_restart_recommended,
                    } => {
                        ui.label(format!("steam launch options: {launch_options}"));
                        ui.monospace(format!(
                            "editable launch script: {}",
                            launch_script.display()
                        ));
                        if *steam_restart_recommended {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                "fully restart steam once so launch options stick",
                            );
                        }
                        ui.label(
                            "launch silksong once from steam; bepinex should create LogOutput.log and plugins/",
                        );
                    }
                    InjectionState::Injected {
                        launch_options,
                        launch_script,
                        log_path,
                    } => {
                        ui.label(format!("steam launch options: {launch_options}"));
                        ui.monospace(format!(
                            "editable launch script: {}",
                            launch_script.display()
                        ));
                        ui.monospace(format!("runtime log: {}", log_path.display()));
                    }
                }
            }

            ui.add_space(6.0);
            ui.label(format!(
                "steam is pointed at editable {SMM_LAUNCH_SCRIPT} via: {}",
                self.install
                    .as_ref()
                    .map(build_kind_launch_hint)
                    .unwrap_or_default()
            ));

            ui.add_space(8.0);
            ui.checkbox(&mut self.dry_run, "dry run (preview only)");

            let button_label = match &status {
                BepinexStatus::NotInstalled => "install bepinex",
                BepinexStatus::NeedsUpdate { .. } => "update bepinex",
                BepinexStatus::InstalledCurrent { .. } => "reinstall bepinex",
            };
            let needs_confirm = !matches!(status, BepinexStatus::NotInstalled) && !self.dry_run;
            let mut run_install = false;
            let mut refresh_status = false;
            let mut configure = false;
            let mut open_script = false;
            let mut reset_script = false;
            ui.horizontal(|ui| {
                if ui.button(button_label).clicked() {
                    if needs_confirm {
                        self.pending_confirm = Some(PendingConfirm::InstallBepinex {
                            button_label: button_label.to_string(),
                        });
                    } else {
                        run_install = true;
                    }
                }
                if ui.button("configure injection").clicked() {
                    configure = true;
                }
                if ui.button("open launch script").clicked() {
                    open_script = true;
                }
                if ui.button("reset launch script").clicked() {
                    reset_script = true;
                }
                if ui.button("refresh status").clicked() {
                    refresh_status = true;
                }
            });
            if run_install {
                self.run_bepinex_install(toasts);
            }
            if configure {
                self.run_configure_injection(toasts);
            }
            if open_script {
                self.run_open_launch_script(toasts);
            }
            if reset_script {
                self.pending_confirm = Some(PendingConfirm::ResetLaunchScript);
            }
            if refresh_status {
                self.refresh_bepinex_status();
                toasts.info("bepinex status refreshed");
            }
        } else {
            ui.label("bepinex status has not been checked yet");
            if ui.button("refresh status").clicked() {
                self.refresh_bepinex_status();
                toasts.info("bepinex status refreshed");
            }
        }

        if let Some(busy) = &self.busy_message {
            ui.add_space(8.0);
            ui.label(busy);
        }

        if let Some(log) = &self.bepinex_log {
            ui.add_space(12.0);
            ui.label("install log");
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.monospace(log);
                });
        }
    }

    fn draw_diagnostics_section(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        ui.label("diagnostics");
        ui.add_space(6.0);
        if let Some(path) = logging::current_log_path() {
            ui.monospace(path.display().to_string());
        } else {
            ui.weak("no session log yet");
        }
        ui.add_space(6.0);
        if ui.button("open log file").clicked() {
            match logging::open_current_log() {
                Ok(path) => {
                    toasts.success(format!("opened {}", path.display()));
                    logging::info(format!("opened log at {}", path.display()));
                }
                Err(error) => {
                    toasts.error(error.to_string());
                    logging::error(error.to_string());
                }
            }
        }
    }

    fn draw_pending_confirm(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        match &self.pending_confirm {
            Some(PendingConfirm::InstallBepinex { button_label }) => {
                let button_label = button_label.clone();
                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "{button_label}? existing pack files are overwritten (a timestamped backup is kept under BepInEx/.manager_backup/)"
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(format!("confirm {button_label}")).clicked() {
                            self.pending_confirm = None;
                            self.run_bepinex_install(toasts);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_confirm = None;
                        }
                    });
                });
            }
            Some(PendingConfirm::ResetLaunchScript) => {
                ui.add_space(12.0);
                ui.group(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "reset the editable launch script to the default template? your custom edits will be lost",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("confirm reset").clicked() {
                            self.pending_confirm = None;
                            self.run_reset_launch_script(toasts);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_confirm = None;
                        }
                    });
                });
            }
            None => {}
        }
    }

    fn run_automatic_detection(&mut self, toasts: &mut ToastQueue) {
        match detection::detect() {
            Ok(install) => self.accept_install(install, toasts),
            Err(error) => {
                self.install = None;
                self.bepinex_status = None;
                self.injection_state = None;
                self.injection_alert = None;
                self.status_message = Some(error.to_string());
                logging::error(format!("detection failed: {error}"));
                toasts.error(error.to_string());
                self.pick_install_folder_after_failure(&error, toasts);
            }
        }
    }

    fn pick_install_folder_after_failure(
        &mut self,
        error: &DetectionError,
        toasts: &mut ToastQueue,
    ) {
        if !matches!(error, DetectionError::InstallNotFound { .. }) {
            return;
        }

        let Some(folder) = rfd::FileDialog::new()
            .set_title("select hollow knight silksong folder")
            .pick_folder()
        else {
            return;
        };

        match detection::inspect_install_folder(&folder) {
            Ok(install) => self.accept_install(install, toasts),
            Err(inspect_error) => {
                let message = format!(
                    "automatic detection failed, and the chosen folder is not a valid install:\n{inspect_error}"
                );
                self.status_message = Some(message.clone());
                logging::error(&message);
                toasts.error(inspect_error.to_string());
            }
        }
    }

    fn pick_install_folder(&mut self, toasts: &mut ToastQueue) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("select hollow knight silksong folder")
            .pick_folder()
        else {
            return;
        };

        match detection::inspect_install_folder(&folder) {
            Ok(install) => self.accept_install(install, toasts),
            Err(error) => {
                self.install = None;
                self.bepinex_status = None;
                self.injection_state = None;
                self.injection_alert = None;
                self.status_message = Some(error.to_string());
                logging::error(format!("folder inspect failed: {error}"));
                toasts.error(error.to_string());
            }
        }
    }

    fn accept_install(&mut self, install: SilksongInstall, toasts: &mut ToastQueue) {
        let folder = install.install_folder.clone();
        let mut message = format!(
            "found {} build at {}",
            install.build_kind,
            install.install_folder.display()
        );
        if install.build_kind == detection::BuildKind::Proton && install.proton_prefix.is_none() {
            message.push_str(
                "\nproton build detected, but no compatdata prefix was found for app 1030300 yet",
            );
        }
        self.status_message = Some(message.clone());
        self.accept_install_quiet(install);
        self.status_message = Some(message);
        if let Err(error) = remember_install_folder(&folder) {
            logging::error(format!("could not remember install folder: {error}"));
        }
        logging::info(format!("using install at {}", folder.display()));
        toasts.success(format!("using install at {}", folder.display()));
    }

    fn run_bepinex_install(&mut self, toasts: &mut ToastQueue) {
        let Some(install) = self.install.clone() else {
            return;
        };

        let dry_run = self.dry_run || dry_run_from_environment();
        self.busy_message = Some(if dry_run {
            "building dry-run plan…".to_string()
        } else {
            "downloading and installing bepinex…".to_string()
        });

        match install_recommended(&install, dry_run) {
            Ok(summary) => self.accept_install_summary(summary, toasts),
            Err(error) => {
                self.busy_message = None;
                self.bepinex_log = Some(error.to_string());
                logging::error(format!("bepinex install failed: {error}"));
                toasts.error(format!("bepinex install failed: {error}"));
            }
        }
    }

    fn run_configure_injection(&mut self, toasts: &mut ToastQueue) {
        let Some(install) = self.install.clone() else {
            return;
        };

        match configure_injection(&install) {
            Ok(outcome) => {
                self.busy_message = None;
                let description = describe_configure_outcome(&outcome);
                self.bepinex_log = Some(description.clone());
                self.refresh_bepinex_status();
                if let Some(alert) = outcome.alert_message() {
                    self.injection_alert = Some(alert);
                }
                logging::info(&description);
                toasts.success("injection configuration updated");
            }
            Err(error) => {
                self.busy_message = None;
                self.bepinex_log = Some(error.to_string());
                logging::error(format!("configure injection failed: {error}"));
                toasts.error(error.to_string());
            }
        }
    }

    fn run_open_launch_script(&mut self, toasts: &mut ToastQueue) {
        let Some(install) = self.install.clone() else {
            return;
        };
        match open_launch_script(&install) {
            Ok(()) => {
                let message = format!(
                    "opened editable launch script ({SMM_LAUNCH_SCRIPT}). steam uses: {}",
                    build_kind_launch_hint(&install)
                );
                self.bepinex_log = Some(message.clone());
                logging::info(&message);
                toasts.success("opened launch script");
            }
            Err(error) => {
                let message = format!("could not open launch script: {error}");
                self.bepinex_log = Some(message.clone());
                logging::error(&message);
                toasts.error(message);
            }
        }
    }

    fn run_reset_launch_script(&mut self, toasts: &mut ToastQueue) {
        let Some(install) = self.install.clone() else {
            return;
        };
        match reset_launch_script(&install) {
            Ok(message) => {
                self.bepinex_log = Some(message.clone());
                self.refresh_bepinex_status();
                logging::info(&message);
                toasts.success("launch script reset");
            }
            Err(error) => {
                self.bepinex_log = Some(error.to_string());
                logging::error(format!("reset launch script failed: {error}"));
                toasts.error(error.to_string());
            }
        }
    }

    fn accept_install_summary(&mut self, summary: InstallSummary, toasts: &mut ToastQueue) {
        self.busy_message = None;
        let description = summary.describe();
        self.bepinex_log = Some(description.clone());
        self.refresh_bepinex_status();
        if let Some(alert) = summary.launch.alert_message() {
            self.injection_alert = Some(alert);
        }
        logging::info(&description);
        if summary.dry_run {
            toasts.info("bepinex dry-run plan ready");
        } else {
            toasts.success("bepinex install finished");
        }
    }
}

fn injection_alert_from_state(state: Option<&InjectionState>) -> Option<String> {
    match state? {
        InjectionState::NotConfigured {
            required_launch_options,
            reason,
            ..
        } => Some(format!(
            "injection NOT configured — {reason}. click configure injection (or paste):\n{required_launch_options}"
        )),
        InjectionState::ConfiguredAwaitingLaunch {
            steam_restart_recommended: true,
            launch_options,
            ..
        } => Some(format!(
            "launch options written, but restart steam once so they stick:\n{launch_options}"
        )),
        _ => None,
    }
}

fn describe_configure_outcome(outcome: &LaunchOptionsOutcome) -> String {
    match outcome {
        LaunchOptionsOutcome::AlreadyConfigured {
            launch_options,
            script,
        } => format!(
            "injection already configured ({launch_options}); script {}",
            script_summary(script)
        ),
        LaunchOptionsOutcome::Updated {
            localconfig_path,
            launch_options,
            script,
            steam_restart_recommended,
        } => {
            let mut text = format!(
                "wrote steam launch options to {} ({launch_options}); script {}",
                localconfig_path.display(),
                script_summary(script)
            );
            if *steam_restart_recommended {
                text.push_str("\nfully restart steam once so the options stick");
            }
            text
        }
        LaunchOptionsOutcome::ManualPasteRequired {
            reason,
            launch_options,
            script,
        } => format!(
            "script {} ready, but steam launch options NOT set — {reason}. paste:\n{launch_options}",
            script_summary(script)
        ),
        LaunchOptionsOutcome::DryRun { launch_options } => {
            format!("would create {SMM_LAUNCH_SCRIPT} and set steam launch options:\n{launch_options}")
        }
    }
}

fn script_summary(script: &LaunchScriptAction) -> String {
    match script {
        LaunchScriptAction::Created { path } => format!("created {}", path.display()),
        LaunchScriptAction::AlreadyPresent { path } => {
            format!("kept editable {}", path.display())
        }
        LaunchScriptAction::Reset { path } => format!("reset {}", path.display()),
    }
}

fn dry_run_from_environment() -> bool {
    matches!(
        std::env::var("SILKSONG_BEPINEX_DRY_RUN").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}
