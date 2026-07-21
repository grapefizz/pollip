use super::catalog::{cache_age_label, load_catalog, CatalogError, CatalogSnapshot};
use super::community::COMMUNITY_SLUG;
use super::icons::IconCache;
use super::install::{
    collect_missing_dependencies, find_update_version, install_package_tree, InstallRequest,
};
use super::package::RemotePackage;
use crate::detection::SilksongInstall;
use crate::logging;
use crate::mods::InstalledMod;
use crate::ui::{draw_mod_icon, muted_text, page_list_height, shorten_line, soft_row, title_text, ui_text};
use crate::ui::ToastQueue;
use eframe::egui;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

const BROWSE_ROW_HEIGHT: f32 = 72.0;

enum CatalogMessage {
    Ready(CatalogSnapshot),
    Failed(String),
}

enum InstallMessage {
    Ready { report_text: String },
    Failed(String),
}

#[derive(Debug, Clone)]
enum PendingInstall {
    Dependencies {
        package: RemotePackage,
        missing_names: Vec<String>,
    },
    Reinstall {
        package: RemotePackage,
    },
}

pub struct ThunderstoreBrowse {
    packages: Vec<RemotePackage>,
    filtered: Vec<usize>,
    filter_query: String,
    fetched_at: Option<std::time::SystemTime>,
    from_cache: bool,
    search_query: String,
    status_message: Option<String>,
    error_message: Option<String>,
    catalog_busy: bool,
    install_busy: bool,
    catalog_rx: Option<Receiver<CatalogMessage>>,
    install_rx: Option<Receiver<InstallMessage>>,
    pending_install: Option<PendingInstall>,
    auto_started: bool,
    last_install_changed: bool,
    icons: IconCache,
}

impl Default for ThunderstoreBrowse {
    fn default() -> Self {
        Self {
            packages: Vec::new(),
            filtered: Vec::new(),
            filter_query: String::new(),
            fetched_at: None,
            from_cache: false,
            search_query: String::new(),
            status_message: None,
            error_message: None,
            catalog_busy: false,
            install_busy: false,
            catalog_rx: None,
            install_rx: None,
            pending_install: None,
            auto_started: false,
            last_install_changed: false,
            icons: IconCache::default(),
        }
    }
}

impl ThunderstoreBrowse {
    pub fn take_install_changed(&mut self) -> bool {
        let changed = self.last_install_changed;
        self.last_install_changed = false;
        changed
    }

    pub fn tick(&mut self, ctx: &egui::Context, toasts: &mut ToastQueue) {
        self.poll_background(ctx, toasts);
        self.icons.poll(ctx);
        if !self.auto_started {
            self.auto_started = true;
            self.start_catalog_fetch(ctx, false);
        }
    }

    pub fn update_version_for(&self, installed: &InstalledMod) -> Option<String> {
        find_update_version(installed, &self.packages)
    }

    pub fn icon_url_for(&self, entry_name: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|package| package.full_name == entry_name)
            .map(|package| package.icon_url.as_str())
            .filter(|url| !url.is_empty())
    }

    pub fn icon_texture_for(
        &mut self,
        ctx: &egui::Context,
        entry_name: &str,
    ) -> Option<egui::TextureHandle> {
        let url = self.icon_url_for(entry_name)?.to_owned();
        self.icons.texture_for(ctx, &url)
    }

    pub fn description_for(&self, entry_name: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|package| package.full_name == entry_name)
            .map(|package| package.description.as_str())
            .filter(|text| !text.is_empty())
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        install: &SilksongInstall,
        installed_mods: &[InstalledMod],
        toasts: &mut ToastQueue,
    ) {
        self.poll_background(ui.ctx(), toasts);
        self.icons.poll(ui.ctx());

        if !self.auto_started {
            self.auto_started = true;
            self.start_catalog_fetch(ui.ctx(), false);
        }

        ui.label(muted_text(format!("community · {COMMUNITY_SLUG}")));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(ui_text("search"));
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .desired_width(320.0)
                    .hint_text("name, author, description"),
            );
            if response.changed() {
                self.rebuild_filter(true);
            }
            let refresh_enabled = !self.catalog_busy;
            if ui
                .add_enabled(refresh_enabled, egui::Button::new(ui_text("refresh")))
                .clicked()
            {
                self.start_catalog_fetch(ui.ctx(), true);
            }
        });

        ui.add_space(6.0);

        if self.catalog_busy {
            ui.label(muted_text("loading thunderstore packages…"));
        } else if let Some(fetched_at) = self.fetched_at {
            let source = if self.from_cache { "cache" } else { "network" };
            ui.label(muted_text(format!(
                "{} shown · {} total · {source} · {}",
                self.filtered.len(),
                self.packages.len(),
                cache_age_label(fetched_at)
            )));
        }

        if let Some(error) = &self.error_message {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(status) = &self.status_message {
            ui.label(muted_text(status.clone()));
        }

        if self.install_busy {
            ui.colored_label(ui.visuals().warn_fg_color, "installing package…");
        }

        if let Some(pending) = self.pending_install.clone() {
            self.draw_pending_prompt(ui, install, installed_mods, &pending);
        }

        ui.add_space(8.0);
        self.rebuild_filter(false);

        if self.packages.is_empty() && !self.catalog_busy {
            ui.label(muted_text("no packages loaded yet"));
            return;
        }

        if self.filtered.is_empty() {
            ui.label(muted_text("no packages match this search"));
            return;
        }

        let installed_names: HashSet<&str> = installed_mods
            .iter()
            .map(|entry| entry.entry_name.as_str())
            .collect();
        let mut install_target: Option<RemotePackage> = None;
        let row_count = self.filtered.len();

        egui::ScrollArea::vertical()
            .id_salt("thunderstore_browse_list")
            .max_height(page_list_height(ui))
            .auto_shrink([false, false])
            .show_rows(ui, BROWSE_ROW_HEIGHT, row_count, |ui, row_range| {
                for row in row_range {
                    let Some(&package_index) = self.filtered.get(row) else {
                        continue;
                    };
                    let Some(package) = self.packages.get(package_index).cloned() else {
                        continue;
                    };

                    let already = installed_names.contains(package.full_name.as_str());
                    let update = installed_mods.iter().find_map(|entry| {
                        if entry.entry_name == package.full_name {
                            find_update_version(entry, std::slice::from_ref(&package))
                        } else {
                            None
                        }
                    });
                    let icon = self.icons.texture_for(ui.ctx(), &package.icon_url);

                    soft_row(ui, |ui| {
                        ui.set_min_height(BROWSE_ROW_HEIGHT - 10.0);
                        ui.horizontal(|ui| {
                            draw_mod_icon(ui, icon.as_ref(), 44.0);
                            ui.add_space(8.0);

                            ui.vertical(|ui| {
                                ui.label(title_text(&package.name));
                                ui.label(muted_text(format!(
                                    "by {} · v{} · {} downloads",
                                    package.owner, package.version, package.downloads
                                )));
                                if !package.description.is_empty() {
                                    ui.label(muted_text(shorten_line(&package.description, 90)));
                                }
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let label = if update.is_some() {
                                    "update"
                                } else if already {
                                    "reinstall"
                                } else {
                                    "install"
                                };

                                let enabled = !self.install_busy && self.pending_install.is_none();
                                if ui
                                    .add_enabled(enabled, egui::Button::new(ui_text(label)))
                                    .clicked()
                                {
                                    install_target = Some(package.clone());
                                }

                                if let Some(version) = &update {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        format!("update {version}"),
                                    );
                                } else if already {
                                    ui.label(muted_text("installed"));
                                }
                            });
                        });
                    });
                }
            });

        if let Some(package) = install_target {
            self.begin_install_flow(ui.ctx(), install, installed_mods, package, false);
        }
    }

    fn rebuild_filter(&mut self, force: bool) {
        if !force && self.filter_query == self.search_query {
            return;
        }
        self.filter_query = self.search_query.clone();
        self.filtered = self
            .packages
            .iter()
            .enumerate()
            .filter(|(_, package)| package.matches_search(&self.search_query))
            .map(|(index, _)| index)
            .collect();
    }

    fn invalidate_filter(&mut self) {
        self.filter_query.clear();
        self.filtered.clear();
        self.rebuild_filter(true);
    }

    fn draw_pending_prompt(
        &mut self,
        ui: &mut egui::Ui,
        install: &SilksongInstall,
        installed_mods: &[InstalledMod],
        pending: &PendingInstall,
    ) {
        match pending {
            PendingInstall::Dependencies {
                package,
                missing_names,
            } => {
                ui.add_space(6.0);
                ui.group(|ui| {
                    ui.label(title_text(format!("install {}?", package.full_name)));
                    ui.label(muted_text(format!(
                        "missing dependencies:\n{}",
                        missing_names.join("\n")
                    )));
                    ui.horizontal(|ui| {
                        if ui.button("install with dependencies").clicked() {
                            let package = package.clone();
                            self.pending_install = None;
                            self.spawn_install(ui.ctx(), install, installed_mods, package, true);
                        }
                        if ui.button("install without dependencies").clicked() {
                            let package = package.clone();
                            self.pending_install = None;
                            self.spawn_install(ui.ctx(), install, installed_mods, package, false);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_install = None;
                            self.status_message = Some("install cancelled".to_string());
                        }
                    });
                });
            }
            PendingInstall::Reinstall { package } => {
                ui.add_space(6.0);
                ui.group(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "replace installed files for {}? existing plugin files for this package will be overwritten",
                            package.full_name
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("confirm replace").clicked() {
                            let package = package.clone();
                            self.pending_install = None;
                            self.begin_install_flow(ui.ctx(), install, installed_mods, package, true);
                        }
                        if ui.button("cancel").clicked() {
                            self.pending_install = None;
                        }
                    });
                });
            }
        }
    }

    fn begin_install_flow(
        &mut self,
        ctx: &egui::Context,
        install: &SilksongInstall,
        installed_mods: &[InstalledMod],
        package: RemotePackage,
        skip_reinstall_prompt: bool,
    ) {
        let already = installed_mods
            .iter()
            .any(|entry| entry.entry_name == package.full_name);
        if already && !skip_reinstall_prompt {
            self.pending_install = Some(PendingInstall::Reinstall { package });
            return;
        }

        match collect_missing_dependencies(&package, installed_mods, &self.packages) {
            Ok((missing, _skipped)) if missing.is_empty() => {
                self.spawn_install(ctx, install, installed_mods, package, true);
            }
            Ok((missing, _)) => {
                self.pending_install = Some(PendingInstall::Dependencies {
                    package,
                    missing_names: missing
                        .into_iter()
                        .map(|entry| format!("{} (>= {})", entry.full_name, entry.required_version))
                        .collect(),
                });
            }
            Err(error) => {
                self.error_message = Some(error.to_string());
                logging::error(error.to_string());
            }
        }
    }

    fn spawn_install(
        &mut self,
        ctx: &egui::Context,
        install: &SilksongInstall,
        installed_mods: &[InstalledMod],
        package: RemotePackage,
        include_dependencies: bool,
    ) {
        if self.install_busy {
            return;
        }

        let install_folder = install.install_folder.clone();
        let catalog = self.packages.clone();
        let installed = installed_mods.to_vec();
        let request = InstallRequest {
            package: package.clone(),
            include_dependencies,
        };
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        self.install_busy = true;
        self.error_message = None;
        self.status_message = Some(format!("installing {}…", package.full_name));
        logging::info(format!("installing {}", package.full_name));
        let ctx = ctx.clone();

        thread::spawn(move || {
            let message = match install_package_tree(
                &install_folder,
                &request,
                &catalog,
                &installed,
            ) {
                Ok(report) => {
                    let mut text = format!("installed {}", report.installed.join(", "));
                    if !report.skipped_modloader_deps.is_empty() {
                        text.push_str(&format!(
                            "\nmodloader dependency handled separately: {}",
                            report.skipped_modloader_deps.join(", ")
                        ));
                    }
                    InstallMessage::Ready { report_text: text }
                }
                Err(error) => InstallMessage::Failed(error.to_string()),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    fn start_catalog_fetch(&mut self, ctx: &egui::Context, force_refresh: bool) {
        if self.catalog_busy {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.catalog_rx = Some(rx);
        self.catalog_busy = true;
        self.error_message = None;
        self.status_message = Some(if force_refresh {
            "refreshing thunderstore catalog…".to_string()
        } else {
            "loading thunderstore catalog…".to_string()
        });
        let ctx = ctx.clone();
        thread::spawn(move || {
            let message = match load_catalog(force_refresh) {
                Ok(snapshot) => CatalogMessage::Ready(snapshot),
                Err(error) => CatalogMessage::Failed(format_catalog_error(error)),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    fn poll_background(&mut self, ctx: &egui::Context, toasts: &mut ToastQueue) {
        if let Some(rx) = &self.catalog_rx {
            match rx.try_recv() {
                Ok(CatalogMessage::Ready(snapshot)) => {
                    let count = snapshot.packages.len();
                    self.packages = snapshot.packages;
                    self.fetched_at = Some(snapshot.fetched_at);
                    self.from_cache = snapshot.from_cache;
                    self.catalog_busy = false;
                    self.catalog_rx = None;
                    self.invalidate_filter();
                    if let Some(warning) = snapshot.network_warning {
                        self.error_message = Some(warning.clone());
                        self.status_message =
                            Some(format!("showing {count} cached packages"));
                        toasts.info(format!("catalog offline — showing {count} cached packages"));
                        logging::error(warning);
                    } else {
                        self.error_message = None;
                        self.status_message = Some(format!("loaded {count} packages"));
                        toasts.info(format!("loaded {count} thunderstore packages"));
                    }
                    ctx.request_repaint();
                }
                Ok(CatalogMessage::Failed(message)) => {
                    self.catalog_busy = false;
                    self.catalog_rx = None;
                    self.error_message = Some(message.clone());
                    self.status_message = None;
                    logging::error(&message);
                    toasts.error(message);
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.catalog_busy = false;
                    self.catalog_rx = None;
                    let message = "catalog worker stopped unexpectedly".to_string();
                    self.error_message = Some(message.clone());
                    toasts.error(message);
                }
            }
        }

        if let Some(rx) = &self.install_rx {
            match rx.try_recv() {
                Ok(InstallMessage::Ready { report_text }) => {
                    self.install_busy = false;
                    self.install_rx = None;
                    self.status_message = Some(report_text.clone());
                    self.error_message = None;
                    self.last_install_changed = true;
                    logging::info(&report_text);
                    toasts.success(report_text);
                    ctx.request_repaint();
                }
                Ok(InstallMessage::Failed(message)) => {
                    self.install_busy = false;
                    self.install_rx = None;
                    self.error_message = Some(message.clone());
                    self.status_message = None;
                    logging::error(&message);
                    toasts.error(format!("install failed: {message}"));
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.install_busy = false;
                    self.install_rx = None;
                    let message = "install worker stopped unexpectedly".to_string();
                    self.error_message = Some(message.clone());
                    toasts.error(message);
                }
            }
        }
    }
}

fn format_catalog_error(error: CatalogError) -> String {
    error.to_string()
}
