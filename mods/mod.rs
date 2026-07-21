mod inventory;
mod manage;

pub use inventory::{
    disabled_folder, plugins_folder, scan_installed_mods, InstalledMod, ModKind, ModSource,
};
pub use manage::{
    describe_mod_locations, disable_mod, enable_mod, ensure_mod_folders, open_plugins_folder,
    remove_mod, ModError,
};

use crate::detection::SilksongInstall;
use crate::logging;
use crate::nexus::{NexusBrowse, ValidateResult};
use crate::thunderstore::ThunderstoreBrowse;
use crate::ui::{
    draw_mod_icon, muted_text, nav_link, page_list_height, section_heading, shorten_line, soft_row,
    title_text, ui_text,
};
use crate::ui::ToastQueue;
use eframe::egui;
use std::path::PathBuf;

const INSTALLED_ROW_HEIGHT: f32 = 68.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ModsTab {
    #[default]
    Installed,
    Thunderstore,
    Nexus,
}

pub struct ModsPanel {
    mods: Vec<InstalledMod>,
    status_message: Option<String>,
    tracked_install_folder: Option<PathBuf>,
    scanned_once: bool,
    thunderstore: ThunderstoreBrowse,
    nexus: NexusBrowse,
    pending_remove: Option<String>,
    tab: ModsTab,
}

impl Default for ModsPanel {
    fn default() -> Self {
        Self {
            mods: Vec::new(),
            status_message: None,
            tracked_install_folder: None,
            scanned_once: false,
            thunderstore: ThunderstoreBrowse::default(),
            nexus: NexusBrowse::default(),
            pending_remove: None,
            tab: ModsTab::Installed,
        }
    }
}

impl ModsPanel {
    pub fn invalidate_scan(&mut self) {
        self.scanned_once = false;
    }

    pub fn tick_background(
        &mut self,
        ctx: &egui::Context,
        install: Option<&SilksongInstall>,
        toasts: &mut ToastQueue,
        nexus_account: Option<&ValidateResult>,
    ) {
        self.thunderstore.tick(ctx, toasts);
        self.nexus.set_account(nexus_account);
        self.nexus.tick(ctx, install, toasts);
        if self.thunderstore.take_install_changed() || self.nexus.take_install_changed() {
            if let Some(install) = install {
                self.refresh(install);
            } else {
                self.invalidate_scan();
            }
        }
    }

    pub fn request_nexus_catalog_refresh(&mut self, ctx: &egui::Context) {
        self.nexus.request_catalog_refresh(ctx);
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        install: Option<&SilksongInstall>,
        toasts: &mut ToastQueue,
        nexus_account: Option<&ValidateResult>,
    ) {
        ui.horizontal(|ui| {
            ui.label(section_heading("mods"));
            ui.add_space(20.0);
            if nav_link(ui, self.tab == ModsTab::Installed, "installed").clicked() {
                self.tab = ModsTab::Installed;
            }
            ui.add_space(14.0);
            if nav_link(ui, self.tab == ModsTab::Thunderstore, "thunderstore").clicked() {
                self.tab = ModsTab::Thunderstore;
            }
            ui.add_space(14.0);
            if nav_link(ui, self.tab == ModsTab::Nexus, "nexus").clicked() {
                self.tab = ModsTab::Nexus;
            }
        });
        ui.add_space(12.0);

        let Some(install) = install else {
            ui.label(muted_text("select a silksong install in settings first"));
            return;
        };

        self.resync_if_install_changed(install);

        match self.tab {
            ModsTab::Installed => self.draw_installed(ui, install, toasts),
            ModsTab::Thunderstore => {
                self.thunderstore.draw(ui, install, &self.mods, toasts);
            }
            ModsTab::Nexus => {
                self.nexus
                    .draw(ui, install, &self.mods, toasts, nexus_account);
            }
        }

        if self.thunderstore.take_install_changed() || self.nexus.take_install_changed() {
            self.refresh(install);
        }
    }

    fn draw_installed(
        &mut self,
        ui: &mut egui::Ui,
        install: &SilksongInstall,
        toasts: &mut ToastQueue,
    ) {
        ui.horizontal(|ui| {
            if ui.button(ui_text("refresh")).clicked() {
                self.status_message = None;
                self.refresh(install);
                toasts.info(format!("found {} mod(s)", self.mods.len()));
            }
            if ui.button(ui_text("open mods folder")).clicked() {
                self.open_folder(install, toasts);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(muted_text(format!("{} installed", self.mods.len())));
            });
        });

        ui.add_space(6.0);
        ui.label(muted_text(describe_mod_locations(&install.install_folder)));
        ui.add_space(8.0);

        if let Some(message) = &self.status_message {
            ui.colored_label(ui.visuals().warn_fg_color, message);
            ui.add_space(6.0);
        }

        self.draw_remove_confirm(ui, install, toasts);

        if self.mods.is_empty() {
            ui.label(muted_text(
                "no mods found in bepinex/plugins or bepinex/plugins_disabled",
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(ui_text("browse thunderstore")).clicked() {
                    self.tab = ModsTab::Thunderstore;
                }
                if ui.button(ui_text("browse nexus")).clicked() {
                    self.tab = ModsTab::Nexus;
                }
            });
            return;
        }

        let mut enable_entry: Option<String> = None;
        let mut disable_entry: Option<String> = None;
        let mut remove_entry: Option<String> = None;
        let update_targets: Vec<(String, String)> = self
            .mods
            .iter()
            .filter_map(|installed| {
                if installed.source != ModSource::Thunderstore
                    && installed.source != ModSource::Unknown
                {
                    return None;
                }
                self.thunderstore
                    .update_version_for(installed)
                    .map(|newer| (installed.entry_name.clone(), newer))
            })
            .collect();

        let row_count = self.mods.len();
        let list_height = page_list_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("installed_mods_list")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show_rows(ui, INSTALLED_ROW_HEIGHT, row_count, |ui, row_range| {
                for row in row_range {
                    let Some(installed) = self.mods.get(row) else {
                        continue;
                    };
                    let icon = match installed.source {
                        ModSource::Nexus => self
                            .nexus
                            .icon_texture_for(ui.ctx(), &installed.entry_name),
                        _ => self
                            .thunderstore
                            .icon_texture_for(ui.ctx(), &installed.entry_name),
                    };
                    let description = match installed.source {
                        ModSource::Nexus => self
                            .nexus
                            .description_for(&installed.entry_name)
                            .map(str::to_owned),
                        _ => self
                            .thunderstore
                            .description_for(&installed.entry_name)
                            .map(str::to_owned),
                    };
                    let update_version = update_targets
                        .iter()
                        .find(|(entry, _)| entry == &installed.entry_name)
                        .map(|(_, version)| version.clone());

                    soft_row(ui, |ui| {
                        ui.set_min_height(INSTALLED_ROW_HEIGHT - 8.0);
                        ui.horizontal(|ui| {
                            let mut enabled = installed.enabled;
                            if ui
                                .checkbox(&mut enabled, "")
                                .on_hover_text(if installed.enabled {
                                    "disable mod"
                                } else {
                                    "enable mod"
                                })
                                .changed()
                            {
                                if enabled {
                                    enable_entry = Some(installed.entry_name.clone());
                                } else {
                                    disable_entry = Some(installed.entry_name.clone());
                                }
                            }

                            draw_mod_icon(ui, icon.as_ref(), 36.0);
                            ui.add_space(6.0);

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(title_text(&installed.display_name));
                                    ui.label(muted_text(installed.source.label()));
                                    if let Some(version) = &update_version {
                                        ui.label(muted_text(format!("update {version}")));
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label(muted_text(&installed.entry_name));
                                    if let Some(version) = &installed.version {
                                        ui.label(muted_text(format!("v{version}")));
                                    }
                                    if !installed.enabled {
                                        ui.label(muted_text("disabled"));
                                    }
                                });
                                if let Some(description) = &description {
                                    ui.label(muted_text(shorten_line(description, 90)));
                                }
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button(ui_text("remove")).clicked() {
                                        remove_entry = Some(installed.entry_name.clone());
                                    }
                                },
                            );
                        });
                    });
                }
            });

        if let Some(entry_name) = enable_entry {
            self.toggle_enabled(install, &entry_name, true, toasts);
        }
        if let Some(entry_name) = disable_entry {
            self.toggle_enabled(install, &entry_name, false, toasts);
        }
        if let Some(entry_name) = remove_entry {
            self.pending_remove = Some(entry_name);
        }
    }

    fn draw_remove_confirm(
        &mut self,
        ui: &mut egui::Ui,
        install: &SilksongInstall,
        toasts: &mut ToastQueue,
    ) {
        let Some(entry_name) = self.pending_remove.clone() else {
            return;
        };
        let display = self
            .mods
            .iter()
            .find(|entry| entry.entry_name == entry_name)
            .map(|entry| entry.display_name.clone())
            .unwrap_or_else(|| entry_name.clone());

        ui.group(|ui| {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "remove '{display}'? it will be moved to BepInEx/plugins_removed (not permanently deleted)"
                ),
            );
            ui.horizontal(|ui| {
                if ui.button("confirm remove").clicked() {
                    self.pending_remove = None;
                    self.remove_entry(install, &entry_name, toasts);
                }
                if ui.button("cancel").clicked() {
                    self.pending_remove = None;
                }
            });
        });
        ui.add_space(8.0);
    }

    fn resync_if_install_changed(&mut self, install: &SilksongInstall) {
        let folder = install.install_folder.clone();
        let changed = self.tracked_install_folder.as_ref() != Some(&folder);
        if changed || !self.scanned_once {
            self.tracked_install_folder = Some(folder);
            self.status_message = None;
            self.refresh(install);
        }
    }

    fn refresh(&mut self, install: &SilksongInstall) {
        self.scanned_once = true;
        if let Err(error) = ensure_mod_folders(&install.install_folder) {
            self.status_message = Some(error.to_string());
            self.mods.clear();
            return;
        }

        match scan_installed_mods(&install.install_folder) {
            Ok(mods) => {
                let count = mods.len();
                self.mods = mods;
                if self.status_message.is_none() {
                    self.status_message = Some(format!("found {count} mod(s)"));
                }
            }
            Err(error) => {
                self.mods.clear();
                self.status_message = Some(error.to_string());
            }
        }
    }

    fn refresh_keeping_message(&mut self, install: &SilksongInstall, message: String) {
        self.status_message = Some(message);
        self.refresh(install);
    }

    fn open_folder(&mut self, install: &SilksongInstall, toasts: &mut ToastQueue) {
        match open_plugins_folder(&install.install_folder) {
            Ok(path) => {
                let message = format!("opened {}", path.display());
                self.status_message = Some(message.clone());
                logging::info(&message);
                toasts.success(message);
            }
            Err(error) => {
                self.status_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn toggle_enabled(
        &mut self,
        install: &SilksongInstall,
        entry_name: &str,
        enable: bool,
        toasts: &mut ToastQueue,
    ) {
        let Some(installed) = self
            .mods
            .iter()
            .find(|entry| entry.entry_name == entry_name)
            .cloned()
        else {
            self.refresh_keeping_message(
                install,
                format!("mod '{entry_name}' is no longer listed"),
            );
            return;
        };

        let outcome = if enable {
            enable_mod(&install.install_folder, &installed)
        } else {
            disable_mod(&install.install_folder, &installed)
        };

        match outcome {
            Ok(path) => {
                let action = if enable { "enabled" } else { "disabled" };
                let message = format!(
                    "{action} {} → {}",
                    installed.display_name,
                    path.display()
                );
                logging::info(&message);
                toasts.success(format!("{action} {}", installed.display_name));
                self.refresh_keeping_message(install, message);
            }
            Err(error) => {
                logging::error(error.to_string());
                toasts.error(error.to_string());
                self.refresh_keeping_message(install, error.to_string());
            }
        }
    }

    fn remove_entry(
        &mut self,
        install: &SilksongInstall,
        entry_name: &str,
        toasts: &mut ToastQueue,
    ) {
        let Some(installed) = self
            .mods
            .iter()
            .find(|entry| entry.entry_name == entry_name)
            .cloned()
        else {
            self.refresh_keeping_message(
                install,
                format!("mod '{entry_name}' is no longer listed"),
            );
            return;
        };

        match remove_mod(&install.install_folder, &installed) {
            Ok(path) => {
                let message = format!(
                    "moved {} to backup at {}",
                    installed.display_name,
                    path.display()
                );
                logging::info(&message);
                toasts.success(format!("removed {}", installed.display_name));
                self.refresh_keeping_message(install, message);
            }
            Err(error) => {
                logging::error(error.to_string());
                toasts.error(error.to_string());
                self.refresh_keeping_message(install, error.to_string());
            }
        }
    }
}
