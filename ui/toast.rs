use eframe::egui::{self, Align2, Frame, Order, Sense};
use std::time::{Duration, Instant};

use super::theme::{BG, MUTED, WHITE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

struct ToastEntry {
    kind: ToastKind,
    message: String,
    shown_at: Instant,
}

pub struct ToastQueue {
    entries: Vec<ToastEntry>,
}

impl Default for ToastQueue {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl ToastQueue {
    pub fn success(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Success, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Error, message);
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.push(ToastKind::Info, message);
    }

    fn push(&mut self, kind: ToastKind, message: impl Into<String>) {
        self.entries.push(ToastEntry {
            kind,
            message: message.into(),
            shown_at: Instant::now(),
        });
        if self.entries.len() > 5 {
            self.entries.remove(0);
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context) {
        let lifetime = Duration::from_secs(5);
        self.entries
            .retain(|entry| entry.shown_at.elapsed() < lifetime);

        if self.entries.is_empty() {
            return;
        }

        let mut dismiss_index: Option<usize> = None;

        egui::Area::new(egui::Id::new("toast_stack"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for (index, entry) in self.entries.iter().enumerate().rev() {
                    let stroke = match entry.kind {
                        ToastKind::Error => WHITE,
                        ToastKind::Success | ToastKind::Info => MUTED,
                    };

                    let response = Frame::NONE
                        .fill(BG)
                        .stroke(egui::Stroke::new(1.0, stroke))
                        .corner_radius(0.0)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_min_width(260.0);
                            let prefix = match entry.kind {
                                ToastKind::Success => "ok",
                                ToastKind::Error => "err",
                                ToastKind::Info => "info",
                            };
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(prefix)
                                        .color(if entry.kind == ToastKind::Error {
                                            WHITE
                                        } else {
                                            MUTED
                                        })
                                        .monospace(),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(&entry.message).color(WHITE),
                                );
                            });
                        })
                        .response
                        .interact(Sense::click());

                    if response.clicked() {
                        dismiss_index = Some(index);
                    }
                    ui.add_space(6.0);
                }
            });

        if let Some(index) = dismiss_index {
            self.entries.remove(index);
        }
    }
}
