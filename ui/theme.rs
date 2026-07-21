use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, FontTweak, RichText, Sense,
    Shadow, Stroke, TextStyle, Theme, Visuals,
};
use eframe::epaint::text::VariationCoords;
use std::sync::Arc;

pub const WHITE: Color32 = Color32::from_rgb(245, 245, 245);
pub const TEXT: Color32 = Color32::from_rgb(210, 210, 210);
pub const TITLE: Color32 = Color32::from_rgb(190, 190, 190);
pub const MUTED: Color32 = Color32::from_rgb(140, 140, 140);
pub const BG: Color32 = Color32::from_rgb(18, 18, 18);
pub const RAISED: Color32 = Color32::from_rgb(28, 28, 28);
pub const LINE: Color32 = Color32::from_rgb(48, 48, 48);
pub const HOVER: Color32 = Color32::from_rgb(32, 32, 32);

pub fn display_family() -> FontFamily {
    FontFamily::Name(Arc::from("Display"))
}

pub fn ui_family() -> FontFamily {
    FontFamily::Name(Arc::from("Ui"))
}

pub fn brand_text(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(16.0, display_family()))
        .color(WHITE)
}

pub fn section_heading(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(14.0, display_family()))
        .color(WHITE)
}

pub fn ui_text(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(13.0, ui_family()))
        .color(TEXT)
}

pub fn title_text(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(14.0, ui_family()))
        .color(TITLE)
}

pub fn muted_text(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(12.0, ui_family()))
        .color(MUTED)
}

pub fn mono_text(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::new(12.0, FontFamily::Monospace))
        .color(MUTED)
}

pub fn page_list_height(ui: &egui::Ui) -> f32 {
    (ui.ctx().viewport_rect().height() - 220.0).clamp(240.0, 1400.0)
}

pub fn shorten_line(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let shortened: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

pub fn apply_dark_theme(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.warn_fg_color = WHITE;
    visuals.error_fg_color = WHITE;
    visuals.hyperlink_color = WHITE;
    visuals.window_fill = BG;
    visuals.panel_fill = BG;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = RAISED;
    visuals.code_bg_color = RAISED;
    visuals.widgets.noninteractive.bg_fill = BG;
    visuals.widgets.noninteractive.weak_bg_fill = RAISED;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.inactive.bg_fill = BG;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.weak_bg_fill = HOVER;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, WHITE);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, WHITE);
    visuals.widgets.active.bg_fill = HOVER;
    visuals.widgets.active.weak_bg_fill = HOVER;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, WHITE);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, WHITE);
    visuals.widgets.open.bg_fill = RAISED;
    visuals.widgets.open.weak_bg_fill = RAISED;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, LINE);
    visuals.selection.bg_fill = Color32::from_rgb(55, 55, 55);
    visuals.selection.stroke = Stroke::new(1.0, WHITE);
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.menu_corner_radius = 0.0.into();
    visuals.window_corner_radius = 0.0.into();
    visuals.widgets.noninteractive.corner_radius = 0.0.into();
    visuals.widgets.inactive.corner_radius = 0.0.into();
    visuals.widgets.hovered.corner_radius = 0.0.into();
    visuals.widgets.active.corner_radius = 0.0.into();
    visuals.widgets.open.corner_radius = 0.0.into();

    ctx.set_theme(Theme::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.window_margin = egui::Margin::same(0);
        style.visuals.window_shadow = Shadow::NONE;
        style.visuals.popup_shadow = Shadow::NONE;
        style.text_styles.insert(TextStyle::Small, FontId::new(11.0, ui_family()));
        style.text_styles.insert(TextStyle::Body, FontId::new(13.0, ui_family()));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, ui_family()));
        style.text_styles.insert(TextStyle::Heading, FontId::new(14.0, display_family()));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    });
}

pub fn soft_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        add_contents(ui);
    });
    ui.scope(|ui| {
        ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        ui.separator();
    });
}

pub fn draw_mod_icon(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>, size: f32) {
    let size = egui::vec2(size, size);
    if let Some(texture) = texture {
        ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(size)
                .corner_radius(0.0)
                .bg_fill(RAISED),
        );
        return;
    }

    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 0.0, RAISED);
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
}

pub fn nav_link(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let text = if selected {
        RichText::new(label)
            .font(FontId::new(13.0, ui_family()))
            .color(WHITE)
    } else {
        RichText::new(label)
            .font(FontId::new(13.0, ui_family()))
            .color(MUTED)
    };
    let response = ui.add(egui::Button::new(text).frame(false));
    if selected {
        let rect = response.rect;
        ui.painter().hline(
            rect.left()..=rect.right(),
            rect.bottom() + 1.0,
            Stroke::new(1.0, WHITE),
        );
    }
    response
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "SpaceGrotesk".to_owned(),
        Arc::new(
            FontData::from_static(include_bytes!("../media/fonts/SpaceGrotesk.ttf")).tweak(
                FontTweak {
                    coords: VariationCoords::new([(b"wght", 450.0)]),
                    ..Default::default()
                },
            ),
        ),
    );
    fonts.font_data.insert(
        "SpaceMono".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../media/fonts/SpaceMono-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "SpaceMonoBold".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../media/fonts/SpaceMono-Bold.ttf"
        ))),
    );

    fonts.families.insert(
        display_family(),
        vec!["SpaceMonoBold".to_owned(), "SpaceMono".to_owned()],
    );
    fonts.families.insert(
        ui_family(),
        vec!["SpaceGrotesk".to_owned(), "SpaceMono".to_owned()],
    );
    fonts.families.insert(
        FontFamily::Proportional,
        vec!["SpaceGrotesk".to_owned(), "SpaceMono".to_owned()],
    );
    fonts.families.insert(
        FontFamily::Monospace,
        vec!["SpaceMono".to_owned(), "SpaceGrotesk".to_owned()],
    );

    ctx.set_fonts(fonts);
}
