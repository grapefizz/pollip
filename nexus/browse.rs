use super::catalog::{cache_age_label, load_catalog, CatalogError, CatalogSnapshot};
use super::client::ValidateResult;
use super::domain::GAME_DOMAIN;
use super::icons::IconCache;
use super::install::{install_from_nxm, install_premium_mod};
use super::key::load_api_key;
use super::nxm::{parse_nxm_url, take_pending_nxm_urls};
use super::package::RemoteMod;
use crate::detection::SilksongInstall;
use crate::logging;
use crate::mods::InstalledMod;
use crate::ui::{
    draw_mod_icon, muted_text, page_list_height, shorten_line, soft_row, title_text, ui_text,
};
use crate::ui::ToastQueue;
use eframe::egui;
use std::collections::HashSet;
use std::process::Command;
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

pub struct NexusBrowse {
    mods: Vec<RemoteMod>,
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
    auto_started: bool,
    last_install_changed: bool,
    is_premium: Option<bool>,
    account_name: Option<String>,
    icons: IconCache,
}

impl Default for NexusBrowse {
    fn default() -> Self {
        Self {
            mods: Vec::new(),
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
            auto_started: false,
            last_install_changed: false,
            is_premium: None,
            account_name: None,
            icons: IconCache::default(),
        }
    }
}

impl NexusBrowse {
    pub fn take_install_changed(&mut self) -> bool {
        let changed = self.last_install_changed;
        self.last_install_changed = false;
        changed
    }

    pub fn set_account(&mut self, account: Option<&ValidateResult>) {
        match account {
            Some(info) => {
                self.is_premium = Some(info.is_premium);
                self.account_name = Some(info.username.clone());
            }
            None => {
                self.is_premium = None;
                self.account_name = None;
            }
        }
    }

    pub fn description_for(&self, entry_name: &str) -> Option<&str> {
        let mod_id = entry_name.strip_prefix("nexus-")?.parse::<u64>().ok()?;
        self.mods
            .iter()
            .find(|remote| remote.mod_id == mod_id)
            .map(|remote| remote.description.as_str())
            .filter(|text| !text.is_empty())
    }

    pub fn icon_url_for(&self, entry_name: &str) -> Option<&str> {
        let mod_id = entry_name.strip_prefix("nexus-")?.parse::<u64>().ok()?;
        self.mods
            .iter()
            .find(|remote| remote.mod_id == mod_id)
            .map(|remote| remote.picture_url.as_str())
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

    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        install: Option<&SilksongInstall>,
        toasts: &mut ToastQueue,
    ) {
        self.poll_background(ctx, toasts);
        self.poll_pending_nxm(ctx, install, toasts);
        self.icons.poll(ctx);
        if !self.auto_started && load_api_key().ok().flatten().is_some() {
            self.auto_started = true;
            self.start_catalog_fetch(ctx, false);
        }
    }

    pub fn request_catalog_refresh(&mut self, ctx: &egui::Context) {
        self.start_catalog_fetch(ctx, true);
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        install: &SilksongInstall,
        installed_mods: &[InstalledMod],
        toasts: &mut ToastQueue,
        account: Option<&ValidateResult>,
    ) {
        self.set_account(account);
        self.poll_background(ui.ctx(), toasts);
        self.poll_pending_nxm(ui.ctx(), Some(install), toasts);
        self.icons.poll(ui.ctx());

        if !self.auto_started && load_api_key().ok().flatten().is_some() {
            self.auto_started = true;
            self.start_catalog_fetch(ui.ctx(), false);
        }

        ui.label(muted_text(format!("nexus mods · {GAME_DOMAIN}")));
        ui.add_space(8.0);

        if load_api_key().ok().flatten().is_none() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "add your nexus api key in settings to browse and install",
            );
            return;
        }

        if let Some(name) = &self.account_name {
            let tier = if self.is_premium == Some(true) {
                "premium"
            } else {
                "free"
            };
            ui.label(muted_text(format!("signed in as {name} · {tier}")));
            ui.add_space(4.0);
        }

        if self.is_premium == Some(false) {
            ui.label(muted_text(
                "free accounts: use “mod manager download” on the nexus website (nxm://). premium accounts can install directly here.",
            ));
            ui.add_space(6.0);
        }

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
            ui.label(muted_text("loading nexus mods…"));
        } else if let Some(fetched_at) = self.fetched_at {
            let source = if self.from_cache { "cache" } else { "network" };
            ui.label(muted_text(format!(
                "{} shown · {} total · {source} · {}",
                self.filtered.len(),
                self.mods.len(),
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
            ui.colored_label(ui.visuals().warn_fg_color, "installing nexus mod…");
        }

        ui.add_space(8.0);
        self.rebuild_filter(false);

        if self.mods.is_empty() && !self.catalog_busy {
            ui.label(muted_text("no nexus mods loaded yet"));
            return;
        }

        if self.filtered.is_empty() {
            ui.label(muted_text("no mods match this search"));
            return;
        }

        let installed_ids: HashSet<u64> = installed_mods
            .iter()
            .filter_map(|entry| {
                entry
                    .entry_name
                    .strip_prefix("nexus-")
                    .and_then(|id| id.parse().ok())
            })
            .collect();

        let mut install_target: Option<RemoteMod> = None;
        let mut open_target: Option<String> = None;
        let row_count = self.filtered.len();
        let premium = self.is_premium == Some(true);

        egui::ScrollArea::vertical()
            .id_salt("nexus_browse_list")
            .max_height(page_list_height(ui))
            .auto_shrink([false, false])
            .show_rows(ui, BROWSE_ROW_HEIGHT, row_count, |ui, row_range| {
                for row in row_range {
                    let Some(&mod_index) = self.filtered.get(row) else {
                        continue;
                    };
                    let Some(remote) = self.mods.get(mod_index).cloned() else {
                        continue;
                    };
                    let already = installed_ids.contains(&remote.mod_id);
                    let icon = self.icons.texture_for(ui.ctx(), &remote.picture_url);

                    soft_row(ui, |ui| {
                        ui.set_min_height(BROWSE_ROW_HEIGHT - 10.0);
                        ui.horizontal(|ui| {
                            draw_mod_icon(ui, icon.as_ref(), 44.0);
                            ui.add_space(8.0);

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(title_text(&remote.name));
                                    ui.label(muted_text("nexus"));
                                });
                                ui.label(muted_text(format!(
                                    "by {} · v{} · {} endorsements",
                                    remote.author, remote.version, remote.endorsement_count
                                )));
                                if !remote.description.is_empty() {
                                    ui.label(muted_text(shorten_line(&remote.description, 90)));
                                }
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(ui_text("open")).clicked() {
                                    open_target = Some(remote.page_url());
                                }

                                if premium {
                                    let label = if already { "reinstall" } else { "install" };
                                    let enabled = !self.install_busy;
                                    if ui
                                        .add_enabled(enabled, egui::Button::new(ui_text(label)))
                                        .clicked()
                                    {
                                        install_target = Some(remote.clone());
                                    }
                                }

                                if already {
                                    ui.label(muted_text("installed"));
                                } else if !premium {
                                    ui.label(muted_text("use website download"));
                                }
                            });
                        });
                    });
                }
            });

        if let Some(url) = open_target {
            let _ = Command::new("xdg-open").arg(url).spawn();
        }

        if let Some(remote) = install_target {
            self.spawn_premium_install(ui.ctx(), install, remote);
        }
    }

    fn rebuild_filter(&mut self, force: bool) {
        if !force && self.filter_query == self.search_query {
            return;
        }
        self.filter_query = self.search_query.clone();
        self.filtered = self
            .mods
            .iter()
            .enumerate()
            .filter(|(_, remote)| remote.matches_search(&self.search_query))
            .map(|(index, _)| index)
            .collect();
    }

    fn invalidate_filter(&mut self) {
        self.filter_query.clear();
        self.filtered.clear();
        self.rebuild_filter(true);
    }

    fn spawn_premium_install(
        &mut self,
        ctx: &egui::Context,
        install: &SilksongInstall,
        remote: RemoteMod,
    ) {
        if self.install_busy {
            return;
        }
        let install_folder = install.install_folder.clone();
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        self.install_busy = true;
        self.error_message = None;
        self.status_message = Some(format!("installing {}…", remote.name));
        logging::info(format!("installing nexus mod {}", remote.mod_id));
        let ctx = ctx.clone();
        thread::spawn(move || {
            let message = match install_premium_mod(&install_folder, &remote) {
                Ok(_) => InstallMessage::Ready {
                    report_text: format!("installed {}", remote.name),
                },
                Err(error) => InstallMessage::Failed(error.to_string()),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    fn spawn_nxm_install(&mut self, ctx: &egui::Context, install: &SilksongInstall, url: String) {
        if self.install_busy {
            return;
        }
        let link = match parse_nxm_url(&url) {
            Ok(link) => link,
            Err(error) => {
                self.error_message = Some(error.to_string());
                toasts_error_log(&error.to_string());
                return;
            }
        };
        let install_folder = install.install_folder.clone();
        let (tx, rx) = mpsc::channel();
        self.install_rx = Some(rx);
        self.install_busy = true;
        self.error_message = None;
        self.status_message = Some(format!(
            "installing nexus mod {} (file {})…",
            link.mod_id, link.file_id
        ));
        logging::info(format!(
            "installing nexus mod {} file {} via nxm",
            link.mod_id, link.file_id
        ));
        let ctx = ctx.clone();
        thread::spawn(move || {
            let message = match install_from_nxm(&install_folder, &link) {
                Ok(_) => InstallMessage::Ready {
                    report_text: format!("installed nexus-{}", link.mod_id),
                },
                Err(error) => InstallMessage::Failed(error.to_string()),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    fn poll_pending_nxm(
        &mut self,
        ctx: &egui::Context,
        install: Option<&SilksongInstall>,
        toasts: &mut ToastQueue,
    ) {
        if self.install_busy {
            return;
        }
        let Some(install) = install else {
            return;
        };
        let Ok(mut urls) = take_pending_nxm_urls() else {
            return;
        };
        let Some(url) = urls.first().cloned() else {
            return;
        };
        urls.remove(0);
        for leftover in urls {
            let _ = super::nxm::enqueue_nxm_url(&leftover);
        }
        toasts.info("received nexus mod manager download");
        self.spawn_nxm_install(ctx, install, url);
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
            "refreshing nexus catalog…".to_string()
        } else {
            "loading nexus catalog…".to_string()
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
                    let count = snapshot.mods.len();
                    self.mods = snapshot.mods;
                    self.fetched_at = Some(snapshot.fetched_at);
                    self.from_cache = snapshot.from_cache;
                    self.catalog_busy = false;
                    self.catalog_rx = None;
                    self.invalidate_filter();
                    if let Some(warning) = snapshot.network_warning {
                        self.error_message = Some(warning.clone());
                        self.status_message = Some(format!("showing {count} cached mods"));
                        toasts.info(format!("nexus offline — showing {count} cached mods"));
                        logging::error(warning);
                    } else {
                        self.error_message = None;
                        self.status_message = Some(format!("loaded {count} nexus mods"));
                        toasts.info(format!("loaded {count} nexus mods"));
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
                    let message = "nexus catalog worker stopped unexpectedly".to_string();
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
                    toasts.error(format!("nexus install failed: {message}"));
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.install_busy = false;
                    self.install_rx = None;
                    let message = "nexus install worker stopped unexpectedly".to_string();
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

fn toasts_error_log(message: &str) {
    logging::error(message);
}
