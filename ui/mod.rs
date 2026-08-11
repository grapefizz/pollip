mod theme;
mod toast;

pub use theme::{
    apply_dark_theme, brand_text, draw_mod_icon, mono_text, muted_text, nav_link, page_list_height,
    section_heading, shorten_line, soft_row, title_text, ui_text,
};
pub use toast::ToastQueue;

use crate::detection;
use crate::logging;
use crate::mods::ModsPanel;
use crate::profiles::ProfilesPanel;
use crate::settings::{
    install_recommended, launch_silksong, load_preferences, mark_setup_complete,
    remember_install_folder, recommended_status_for, BepinexStatus, SettingsPanel,
};
use crate::ui::theme::{BG, LINE, MUTED, WHITE};
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivePanel {
    #[default]
    Mods,
    Profiles,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstLaunchStep {
    DetectGame,
    InstallBepinex,
}

pub struct Shell {
    active_panel: ActivePanel,
    settings_panel: SettingsPanel,
    mods_panel: ModsPanel,
    profiles_panel: ProfilesPanel,
    toasts: ToastQueue,
    first_launch: Option<FirstLaunchStep>,
}

impl Default for Shell {
    fn default() -> Self {
        let mut shell = Self {
            active_panel: ActivePanel::Mods,
            settings_panel: SettingsPanel::default(),
            mods_panel: ModsPanel::default(),
            profiles_panel: ProfilesPanel::default(),
            toasts: ToastQueue::default(),
            first_launch: None,
        };
        shell.bootstrap_from_preferences();
        shell
    }
}

impl Shell {
    fn bootstrap_from_preferences(&mut self) {
        let preferences = load_preferences().unwrap_or_default();
        if let Some(folder) = &preferences.install_folder {
            match detection::inspect_install_folder(folder) {
                Ok(install) => {
                    self.settings_panel.accept_install_quiet(install);
                    if preferences.setup_complete {
                        self.first_launch = None;
                        self.active_panel = ActivePanel::Mods;
                    } else {
                        self.advance_first_launch_after_detect();
                    }
                }
                Err(error) => {
                    logging::error(format!("saved install path is invalid: {error}"));
                    self.first_launch = Some(FirstLaunchStep::DetectGame);
                    self.active_panel = ActivePanel::Settings;
                }
            }
        } else if preferences.setup_complete {
            self.first_launch = None;
            self.active_panel = ActivePanel::Mods;
        } else {
            self.first_launch = Some(FirstLaunchStep::DetectGame);
            self.active_panel = ActivePanel::Settings;
        }
    }

    fn advance_first_launch_after_detect(&mut self) {
        let Some(install) = self.settings_panel.install() else {
            self.first_launch = Some(FirstLaunchStep::DetectGame);
            return;
        };
        match recommended_status_for(install) {
            BepinexStatus::NotInstalled => {
                self.first_launch = Some(FirstLaunchStep::InstallBepinex);
            }
            _ => {
                self.finish_first_launch();
            }
        }
    }

    fn finish_first_launch(&mut self) {
        if let Err(error) = mark_setup_complete() {
            logging::error(format!("could not save setup_complete: {error}"));
        }
        self.first_launch = None;
        self.active_panel = ActivePanel::Mods;
        self.toasts.success("setup complete — browse or manage mods");
        logging::info("first-launch setup complete");
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        if self.first_launch.is_some() {
            self.draw_first_launch(ui);
            self.toasts.draw(ui.ctx());
            return;
        }

        self.settings_panel
            .nexus_mut()
            .tick(ui.ctx(), &mut self.toasts);

        if self.settings_panel.nexus_mut().take_catalog_refresh() {
            self.mods_panel
                .request_nexus_catalog_refresh(ui.ctx());
        }

        let account = self.settings_panel.nexus().account().cloned();
        self.mods_panel.tick_background(
            ui.ctx(),
            self.settings_panel.install(),
            &mut self.toasts,
            account.as_ref(),
        );

        egui::Panel::top("top_bar")
            .exact_size(44.0)
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(brand_text("pollip"));
                    ui.add_space(28.0);

                    if nav_link(ui, self.active_panel == ActivePanel::Mods, "mods").clicked() {
                        self.active_panel = ActivePanel::Mods;
                    }
                    ui.add_space(16.0);
                    if nav_link(ui, self.active_panel == ActivePanel::Profiles, "profiles")
                        .clicked()
                    {
                        self.active_panel = ActivePanel::Profiles;
                    }
                    ui.add_space(16.0);
                    if nav_link(ui, self.active_panel == ActivePanel::Settings, "settings")
                        .clicked()
                    {
                        self.active_panel = ActivePanel::Settings;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let play_enabled = self.settings_panel.install().is_some();
                        let play = egui::Button::new(
                            egui::RichText::new("play")
                                .color(if play_enabled { BG } else { MUTED })
                                .font(egui::FontId::new(13.0, theme::ui_family())),
                        )
                        .fill(if play_enabled { WHITE } else { LINE })
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(0.0)
                        .min_size(egui::vec2(72.0, 28.0));

                        if ui
                            .add_enabled(play_enabled, play)
                            .on_hover_text("launch silksong through steam with bepinex")
                            .clicked()
                        {
                            self.run_play();
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ui, |ui| {
                let scroll_id = match self.active_panel {
                    ActivePanel::Mods => "page_scroll_mods",
                    ActivePanel::Profiles => "page_scroll_profiles",
                    ActivePanel::Settings => "page_scroll_settings",
                };
                egui::ScrollArea::vertical()
                    .id_salt(scroll_id)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        match self.active_panel {
                            ActivePanel::Mods => {
                                let account = self.settings_panel.nexus().account().cloned();
                                self.mods_panel.draw(
                                    ui,
                                    self.settings_panel.install(),
                                    &mut self.toasts,
                                    account.as_ref(),
                                );
                            }
                            ActivePanel::Profiles => {
                                self.profiles_panel.draw(
                                    ui,
                                    self.settings_panel.install(),
                                    &mut self.toasts,
                                );
                                if self.profiles_panel.take_mods_on_disk_changed() {
                                    self.mods_panel.invalidate_scan();
                                }
                            }
                            ActivePanel::Settings => {
                                self.settings_panel.draw(ui, &mut self.toasts);
                            }
                        }
                    });
            });

        self.toasts.draw(ui.ctx());
    }

    fn draw_first_launch(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(24)))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(48.0);
                    ui.label(brand_text("pollip"));
                    ui.add_space(8.0);
                    let rect = ui.max_rect();
                    ui.painter().hline(
                        rect.left()..=(rect.left() + 120.0),
                        ui.cursor().top(),
                        egui::Stroke::new(1.0, LINE),
                    );
                    ui.add_space(24.0);

                    match self.first_launch {
                        Some(FirstLaunchStep::DetectGame) => {
                            ui.label(ui_text(
                                "find your hollow knight: silksong install to get started",
                            ));
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                if ui.button(ui_text("detect game")).clicked() {
                                    self.run_first_launch_detect();
                                }
                                if ui.button(ui_text("choose folder")).clicked() {
                                    self.run_first_launch_choose_folder();
                                }
                            });
                            if let Some(message) = self.settings_panel.status_message() {
                                ui.add_space(12.0);
                                ui.label(muted_text(message));
                            }
                        }
                        Some(FirstLaunchStep::InstallBepinex) => {
                            ui.label(ui_text(
                                "install the recommended bepinex pack so mods can load",
                            ));
                            ui.add_space(8.0);
                            if let Some(install) = self.settings_panel.install() {
                                ui.label(mono_text(
                                    install.install_folder.display().to_string(),
                                ));
                            }
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                if ui.button(ui_text("install bepinex")).clicked() {
                                    self.run_first_launch_bepinex();
                                }
                                if ui.button(ui_text("skip for now")).clicked() {
                                    self.finish_first_launch();
                                }
                            });
                            if let Some(busy) = self.settings_panel.busy_message() {
                                ui.add_space(12.0);
                                ui.label(muted_text(busy));
                            }
                        }
                        None => {}
                    }
                });
            });
    }

    fn run_first_launch_detect(&mut self) {
        match detection::detect() {
            Ok(install) => {
                let folder = install.install_folder.clone();
                self.settings_panel.accept_install_quiet(install);
                if let Err(error) = remember_install_folder(&folder) {
                    logging::error(format!("could not remember install folder: {error}"));
                }
                self.toasts.success(format!("found silksong at {}", folder.display()));
                logging::info(format!("detected install at {}", folder.display()));
                self.advance_first_launch_after_detect();
            }
            Err(error) => {
                logging::error(format!("detection failed: {error}"));
                self.toasts.error(error.to_string());
                self.settings_panel.set_status_message(error.to_string());
            }
        }
    }

    fn run_first_launch_choose_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("select hollow knight silksong folder")
            .pick_folder()
        else {
            return;
        };
        match detection::inspect_install_folder(&folder) {
            Ok(install) => {
                let folder = install.install_folder.clone();
                self.settings_panel.accept_install_quiet(install);
                if let Err(error) = remember_install_folder(&folder) {
                    logging::error(format!("could not remember install folder: {error}"));
                }
                self.toasts.success(format!("using install at {}", folder.display()));
                logging::info(format!("chose install at {}", folder.display()));
                self.advance_first_launch_after_detect();
            }
            Err(error) => {
                logging::error(format!("folder inspect failed: {error}"));
                self.toasts.error(error.to_string());
                self.settings_panel.set_status_message(error.to_string());
            }
        }
    }

    fn run_play(&mut self) {
        let Some(install) = self.settings_panel.install().cloned() else {
            self.toasts.error("select a silksong install before playing");
            return;
        };

        match launch_silksong(&install) {
            Ok(report) => {
                logging::info(format!("launching silksong via {}", report.method));
                self.toasts.success("starting silksong through steam");
                if let Some(warning) = report.injection_warning {
                    logging::error(&warning);
                    self.toasts.info(warning);
                }
            }
            Err(error) => {
                logging::error(error.to_string());
                self.toasts.error(error.to_string());
            }
        }
    }

    fn run_first_launch_bepinex(&mut self) {
        let Some(install) = self.settings_panel.install().cloned() else {
            return;
        };
        self.settings_panel
            .set_busy_message(Some("downloading and installing bepinex…".into()));
        match install_recommended(&install, false) {
            Ok(summary) => {
                self.settings_panel.set_busy_message(None);
                self.settings_panel.refresh_bepinex_status();
                let message = summary.describe();
                logging::info(&message);
                self.toasts.success("bepinex installed");
                self.finish_first_launch();
            }
            Err(error) => {
                self.settings_panel.set_busy_message(None);
                logging::error(format!("bepinex install failed: {error}"));
                self.toasts.error(error.to_string());
            }
        }
    }
}
