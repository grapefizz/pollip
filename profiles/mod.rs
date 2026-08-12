mod model;
mod store;
mod switch;

pub use model::ProfileSummary;
pub use store::{
    active_profile_name, create_from_current, delete_profile, duplicate_profile, export_profile,
    import_profile, list_profiles, profiles_directory,
};
pub use switch::{
    begin_switch, continue_switch, load_journal, preview_switch, rollback_switch, SwitchJournal,
    SwitchPhase,
};

use crate::detection::SilksongInstall;
use crate::logging;
use crate::thunderstore::{load_catalog, RemotePackage};
use crate::ui::{muted_text, page_list_height, section_heading, title_text};
use crate::ui::ToastQueue;
use eframe::egui;

enum PendingPrompt {
    Delete { name: String },
    Duplicate { source: String },
}

enum SwitchPrompt {
    Preview {
        target_name: String,
        lines: Vec<String>,
    },
    Interrupted {
        journal: SwitchJournal,
    },
}

pub struct ProfilesPanel {
    profiles: Vec<ProfileSummary>,
    active_name: Option<String>,
    new_profile_name: String,
    duplicate_name: String,
    status_message: Option<String>,
    error_message: Option<String>,
    pending_prompt: Option<PendingPrompt>,
    switch_prompt: Option<SwitchPrompt>,
    catalog: Vec<RemotePackage>,
    catalog_loaded: bool,
    scanned_once: bool,
    mods_on_disk_changed: bool,
}

impl Default for ProfilesPanel {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            active_name: None,
            new_profile_name: String::new(),
            duplicate_name: String::new(),
            status_message: None,
            error_message: None,
            pending_prompt: None,
            switch_prompt: None,
            catalog: Vec::new(),
            catalog_loaded: false,
            scanned_once: false,
            mods_on_disk_changed: false,
        }
    }
}

impl ProfilesPanel {
    pub fn take_mods_on_disk_changed(&mut self) -> bool {
        let changed = self.mods_on_disk_changed;
        self.mods_on_disk_changed = false;
        changed
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        install: Option<&SilksongInstall>,
        toasts: &mut ToastQueue,
    ) {
        ui.label(section_heading("profiles"));
        ui.add_space(14.0);

        if !self.scanned_once {
            self.scanned_once = true;
            self.reload_list();
            self.check_interrupted_switch();
        }

        if !self.catalog_loaded {
            self.catalog_loaded = true;
            self.refresh_catalog();
        }

        if let Some(message) = &self.error_message {
            ui.colored_label(ui.visuals().error_fg_color, message);
            ui.add_space(6.0);
        }
        if let Some(message) = &self.status_message {
            ui.colored_label(ui.visuals().warn_fg_color, message);
            ui.add_space(6.0);
        }

        if let Some(SwitchPrompt::Interrupted { journal }) = &self.switch_prompt {
            let target_name = journal.target_profile_name.clone();
            let next_step = journal.next_step_index;
            let total_steps = journal.steps.len();
            let mut finish = false;
            let mut rollback = false;
            ui.group(|ui| {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "interrupted switch to '{target_name}' — finish or roll back before continuing"
                    ),
                );
                ui.label(format!("progress: {next_step} / {total_steps} steps"));
                ui.horizontal(|ui| {
                    if ui.button("finish switch").clicked() {
                        finish = true;
                    }
                    if ui.button("roll back").clicked() {
                        rollback = true;
                    }
                });
            });
            if finish {
                self.finish_interrupted(toasts);
            }
            if rollback {
                self.rollback_interrupted(toasts);
            }
            ui.add_space(8.0);
        }

        let Some(install) = install else {
            ui.label("select a silksong install in settings before managing profiles");
            return;
        };

        if matches!(self.switch_prompt, Some(SwitchPrompt::Interrupted { .. })) {
            return;
        }

        ui.label(muted_text(format!(
            "stored under {}",
            profiles_directory()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "pollip data directory unavailable".into())
        )));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("name");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_profile_name)
                    .desired_width(220.0)
                    .hint_text("new profile"),
            );
            if ui.button("create from current").clicked() {
                self.create_profile(install, toasts);
            }
            if ui.button("import json").clicked() {
                self.import_json(toasts);
            }
            if ui.button("refresh").clicked() {
                self.reload_list();
                toasts.info(format!("{} profile(s)", self.profiles.len()));
            }
        });
        ui.add_space(12.0);

        if let Some(SwitchPrompt::Preview { target_name, lines }) = &self.switch_prompt {
            let target_name = target_name.clone();
            let lines = lines.clone();
            ui.group(|ui| {
                ui.label(title_text(format!("switch to '{target_name}'")));
                ui.add_space(4.0);
                if lines.is_empty() {
                    ui.label("no changes needed — already matches this profile");
                } else {
                    ui.label("these changes will be applied to game files:");
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for line in &lines {
                                ui.label(line);
                            }
                        });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("apply switch").clicked() {
                        self.apply_switch(install, &target_name, toasts);
                    }
                    if ui.button("cancel").clicked() {
                        self.switch_prompt = None;
                    }
                });
            });
            ui.add_space(12.0);
        }

        if self.profiles.is_empty() {
            ui.label("no saved profiles yet — create one from your current mods");
            return;
        }

        let mut switch_name: Option<String> = None;
        let mut delete_name: Option<String> = None;
        let mut duplicate_source: Option<String> = None;
        let mut export_name: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("profiles_list")
            .max_height(page_list_height(ui))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for summary in &self.profiles {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(title_text(&summary.name));
                                    if self.active_name.as_deref() == Some(summary.name.as_str()) {
                                        ui.label(muted_text("active"));
                                    }
                                });
                                ui.label(muted_text(format!(
                                    "{} mod(s) · {}",
                                    summary.mod_count,
                                    summary.path.display()
                                )));
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("delete").clicked() {
                                        delete_name = Some(summary.name.clone());
                                    }
                                    if ui.button("export").clicked() {
                                        export_name = Some(summary.name.clone());
                                    }
                                    if ui.button("duplicate").clicked() {
                                        duplicate_source = Some(summary.name.clone());
                                    }
                                    if ui.button("switch").clicked() {
                                        switch_name = Some(summary.name.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }
            });

        if let Some(name) = switch_name {
            self.preview_switch_to(install, &name, toasts);
        }
        if let Some(name) = export_name {
            self.export_json(&name, toasts);
        }
        if let Some(name) = delete_name {
            self.pending_prompt = Some(PendingPrompt::Delete { name });
            self.duplicate_name.clear();
        }
        if let Some(source) = duplicate_source {
            self.duplicate_name = format!("{source} copy");
            self.pending_prompt = Some(PendingPrompt::Duplicate { source });
        }

        self.draw_pending_prompt(ui, toasts);
    }

    fn draw_pending_prompt(&mut self, ui: &mut egui::Ui, toasts: &mut ToastQueue) {
        match &self.pending_prompt {
            Some(PendingPrompt::Delete { name }) => {
                let name = name.clone();
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("delete profile '{name}'? this cannot be undone"),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("confirm delete").clicked() {
                            self.confirm_delete(&name, toasts);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_prompt = None;
                        }
                    });
                });
            }
            Some(PendingPrompt::Duplicate { source }) => {
                let source = source.clone();
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.label(format!("duplicate '{source}' as"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.duplicate_name).desired_width(220.0),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("create copy").clicked() {
                            self.confirm_duplicate(&source, toasts);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_prompt = None;
                        }
                    });
                });
            }
            None => {}
        }
    }

    fn reload_list(&mut self) {
        match list_profiles() {
            Ok(profiles) => {
                self.profiles = profiles;
                self.active_name = active_profile_name().ok().flatten();
                self.error_message = None;
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
            }
        }
    }

    fn refresh_catalog(&mut self) {
        match load_catalog(false) {
            Ok(snapshot) => {
                self.catalog = snapshot.packages;
            }
            Err(_) => {
                self.catalog.clear();
            }
        }
    }

    fn check_interrupted_switch(&mut self) {
        match load_journal() {
            Ok(Some(journal)) if journal.phase == SwitchPhase::Interrupted => {
                self.switch_prompt = Some(SwitchPrompt::Interrupted { journal });
                self.status_message = Some(
                    "a profile switch was interrupted — choose finish or roll back".to_string(),
                );
            }
            Ok(_) => {}
            Err(error) => {
                self.error_message = Some(error.to_string());
            }
        }
    }

    fn create_profile(&mut self, install: &SilksongInstall, toasts: &mut ToastQueue) {
        let name = self.new_profile_name.clone();
        match create_from_current(&name, &install.install_folder, &self.catalog) {
            Ok(profile) => {
                let message = format!(
                    "saved profile '{}' with {} mod(s)",
                    profile.name,
                    profile.mods.len()
                );
                self.status_message = Some(message.clone());
                self.error_message = None;
                self.new_profile_name.clear();
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn preview_switch_to(
        &mut self,
        install: &SilksongInstall,
        name: &str,
        toasts: &mut ToastQueue,
    ) {
        match preview_switch(&install.install_folder, name, &self.catalog) {
            Ok((profile, _diff, lines)) => {
                self.switch_prompt = Some(SwitchPrompt::Preview {
                    target_name: profile.name,
                    lines,
                });
                self.error_message = None;
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn apply_switch(
        &mut self,
        install: &SilksongInstall,
        target_name: &str,
        toasts: &mut ToastQueue,
    ) {
        match begin_switch(&install.install_folder, target_name, &self.catalog) {
            Ok(mut journal) => match continue_switch(&mut journal) {
                Ok(()) => {
                    self.switch_prompt = None;
                    self.mods_on_disk_changed = true;
                    let message = format!("switched to profile '{target_name}'");
                    self.status_message = Some(message.clone());
                    self.error_message = None;
                    logging::info(&message);
                    toasts.success(message);
                    self.reload_list();
                }
                Err(error) => {
                    self.mods_on_disk_changed = true;
                    self.switch_prompt = Some(SwitchPrompt::Interrupted { journal });
                    let message = format!("switch failed mid-way ({error}) — finish or roll back");
                    self.error_message = Some(message.clone());
                    logging::error(&message);
                    toasts.error(message);
                }
            },
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn finish_interrupted(&mut self, toasts: &mut ToastQueue) {
        let Some(SwitchPrompt::Interrupted { mut journal }) = self.switch_prompt.take() else {
            return;
        };
        match continue_switch(&mut journal) {
            Ok(()) => {
                self.mods_on_disk_changed = true;
                let message = format!("finished switch to '{}'", journal.target_profile_name);
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.mods_on_disk_changed = true;
                self.switch_prompt = Some(SwitchPrompt::Interrupted { journal });
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn rollback_interrupted(&mut self, toasts: &mut ToastQueue) {
        let Some(SwitchPrompt::Interrupted { mut journal }) = self.switch_prompt.take() else {
            return;
        };
        match rollback_switch(&mut journal) {
            Ok(()) => {
                self.mods_on_disk_changed = true;
                let message = "rolled back to the pre-switch mod set".to_string();
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.mods_on_disk_changed = true;
                self.switch_prompt = Some(SwitchPrompt::Interrupted { journal });
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn confirm_delete(&mut self, name: &str, toasts: &mut ToastQueue) {
        match delete_profile(name) {
            Ok(()) => {
                self.pending_prompt = None;
                let message = format!("deleted profile '{name}'");
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn confirm_duplicate(&mut self, source: &str, toasts: &mut ToastQueue) {
        let new_name = self.duplicate_name.clone();
        match duplicate_profile(source, &new_name) {
            Ok(profile) => {
                self.pending_prompt = None;
                let message = format!("duplicated as '{}'", profile.name);
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn export_json(&mut self, name: &str, toasts: &mut ToastQueue) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("export profile json")
            .set_file_name(format!("{}.json", name.replace(' ', "_").to_lowercase()))
            .save_file()
        else {
            return;
        };
        match export_profile(name, &path) {
            Ok(()) => {
                let message = format!("exported to {}", path.display());
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }

    fn import_json(&mut self, toasts: &mut ToastQueue) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("import profile json")
            .add_filter("json", &["json"])
            .pick_file()
        else {
            return;
        };
        match import_profile(&path) {
            Ok(profile) => {
                let message = format!("imported profile '{}'", profile.name);
                self.status_message = Some(message.clone());
                self.error_message = None;
                logging::info(&message);
                toasts.success(message);
                self.reload_list();
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
                toasts.error(error.to_string());
            }
        }
    }
}
