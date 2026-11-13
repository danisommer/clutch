//! Visual identity: colors, typography and hand-drawn widgets in a Swiss-style layout.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId,
    Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Theme, ThemePreference, Ui, Vec2,
    WidgetInfo, WidgetType,
};

use crate::game_mode::{Item, Status};
use crate::storage::ThemeChoice;

/// Colors of one theme. `accent` fills buttons and marks; `accent_text` is the accent used for text and lines.
pub struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub rule: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
}

pub static LIGHT: Palette = Palette {
    background: Color32::from_rgb(244, 243, 239),
    surface: Color32::from_rgb(234, 232, 226),
    text: Color32::from_rgb(20, 20, 20),
    text_muted: Color32::from_rgb(112, 110, 104),
    text_faint: Color32::from_rgb(168, 165, 158),
    rule: Color32::from_rgb(220, 217, 209),
    accent: Color32::from_rgb(227, 0, 27),
    accent_hover: Color32::from_rgb(186, 0, 22),
    accent_text: Color32::from_rgb(227, 0, 27),
    on_accent: Color32::WHITE,
};

pub static DARK: Palette = Palette {
    background: Color32::from_rgb(22, 22, 21),
    surface: Color32::from_rgb(38, 38, 36),
    text: Color32::from_rgb(236, 234, 228),
    text_muted: Color32::from_rgb(160, 157, 150),
    text_faint: Color32::from_rgb(98, 96, 92),
    rule: Color32::from_rgb(52, 51, 48),
    accent: Color32::from_rgb(227, 0, 27),
    accent_hover: Color32::from_rgb(186, 0, 22),
    accent_text: Color32::from_rgb(255, 69, 80),
    on_accent: Color32::WHITE,
};

const SEMIBOLD: &str = "semibold";
const LIGHT_WEIGHT: &str = "light";

/// Palette of the theme being shown, which follows Windows when the choice is "System".
pub fn palette(ctx: &egui::Context) -> &'static Palette {
    match ctx.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

fn load_font(file: &str) -> Option<Arc<FontData>> {
    std::fs::read(format!(r"C:\Windows\Fonts\{file}")).ok().map(|b| Arc::new(FontData::from_owned(b)))
}

fn style_for(style: &mut egui::Style, theme: Theme, p: &Palette) {
    style.visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    style.visuals.panel_fill = p.background;
    style.visuals.window_fill = p.background;
    style.visuals.extreme_bg_color = p.background;
    style.visuals.override_text_color = Some(p.text);
    style.visuals.selection.bg_fill = p.accent;
    style.visuals.selection.stroke = stroke(1.0, p.on_accent);
    for w in [&mut style.visuals.widgets.noninteractive, &mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::NONE;
    }
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.scroll = egui::style::ScrollStyle { bar_width: 4.0, ..egui::style::ScrollStyle::floating() };
    style.visuals.widgets.inactive.bg_fill = p.rule;
    style.visuals.widgets.hovered.bg_fill = p.text_faint;
    style.visuals.widgets.active.bg_fill = p.text_muted;
}

/// Loads Segoe UI from Windows, applies the flat, square-cornered style to both themes and selects one.
pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    let mut fonts = FontDefinitions::empty();
    for (name, file) in [("regular", "segoeui.ttf"), (SEMIBOLD, "seguisb.ttf"), (LIGHT_WEIGHT, "segoeuil.ttf"), ("mono", "consola.ttf"), ("fallback", "arial.ttf")] {
        if let Some(data) = load_font(file) {
            fonts.font_data.insert(name.into(), data);
        }
    }
    let family = |main: &str| vec![main.to_string(), "regular".into(), "fallback".into()];
    fonts.families.insert(FontFamily::Proportional, family("regular"));
    fonts.families.insert(FontFamily::Monospace, family("mono"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family(SEMIBOLD));
    fonts.families.insert(FontFamily::Name(LIGHT_WEIGHT.into()), family(LIGHT_WEIGHT));
    ctx.set_fonts(fonts);

    ctx.style_mut_of(Theme::Light, |style| style_for(style, Theme::Light, &LIGHT));
    ctx.style_mut_of(Theme::Dark, |style| style_for(style, Theme::Dark, &DARK));
    set_theme(ctx, choice);
}

pub fn set_theme(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(match choice {
        ThemeChoice::System => ThemePreference::System,
        ThemeChoice::Light => ThemePreference::Light,
        ThemeChoice::Dark => ThemePreference::Dark,
    });
}

pub fn stroke(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

/// Click sense for enabled widgets, which also lets them take keyboard focus.
fn click_sense(enabled: bool) -> Sense {
    match enabled {
        true => Sense::click(),
        false => Sense::hover(),
    }
}

/// Outline around the widget that has keyboard focus.
pub fn focus_ring(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let p = palette(ui.ctx());
        ui.painter().rect_stroke(response.rect.expand(3.0), CornerRadius::ZERO, stroke(2.0, p.accent_text), StrokeKind::Outside);
    }
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn light(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(LIGHT_WEIGHT.into()))
}

pub fn text(content: impl Into<String>, font: FontId, color: Color32) -> RichText {
    RichText::new(content).font(font).color(color)
}

pub fn heading(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = palette(ui.ctx());
    ui.label(text(title, semibold(40.0), p.text));
    ui.add_space(2.0);
    ui.label(text(subtitle, regular(15.0), p.text_muted));
    ui.add_space(28.0);
}

pub fn section_label(ui: &mut Ui, label: &str) {
    let p = palette(ui.ctx());
    ui.label(text(label.to_uppercase(), semibold(11.0), p.text_muted).extra_letter_spacing(1.4));
    ui.add_space(6.0);
}

/// Full-width 1 px horizontal rule.
pub fn rule(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.rule);
}

fn button(ui: &mut Ui, label: &str, width: f32, enabled: bool, primary: bool) -> Response {
    let p = palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 46.0), click_sense(enabled));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    focus_ring(ui, &response);
    let hovered = enabled && response.hovered();
    let (fill, outline, text_color) = match (primary, enabled, hovered) {
        (_, false, _) => (p.surface, Stroke::NONE, p.text_faint),
        (true, true, false) => (p.accent, Stroke::NONE, p.on_accent),
        (true, true, true) => (p.accent_hover, Stroke::NONE, p.on_accent),
        (false, true, false) => (Color32::TRANSPARENT, stroke(1.5, p.text), p.text),
        (false, true, true) => (p.surface, stroke(1.5, p.text), p.text),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    if outline != Stroke::NONE {
        painter.rect_stroke(rect, CornerRadius::ZERO, outline, StrokeKind::Inside);
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, label, semibold(15.0), text_color);
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

pub fn primary_button(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> Response {
    button(ui, label, width, enabled, true)
}

pub fn secondary_button(ui: &mut Ui, label: &str, width: f32) -> Response {
    button(ui, label, width, true, false)
}

pub fn link(ui: &mut Ui, label: &str, color: Color32) -> Response {
    let response = ui.add(egui::Label::new(text(label, semibold(13.0), color)).sense(Sense::click()));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, label));
    focus_ring(ui, &response);
    if response.hovered() {
        let r = response.rect;
        ui.painter().line_segment([Pos2::new(r.left(), r.bottom()), Pos2::new(r.right(), r.bottom())], stroke(1.0, color));
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn draw_box(ui: &Ui, rect: Rect, checked: bool, enabled: bool, radio: bool) {
    let p = palette(ui.ctx());
    let square = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    let painter = ui.painter();
    let border = if enabled { p.text } else { p.text_faint };
    let fill = if enabled { p.accent } else { p.text_faint };
    match (checked, radio) {
        (false, _) => {
            painter.rect_stroke(square, CornerRadius::ZERO, stroke(1.5, border), StrokeKind::Inside);
        }
        (true, false) => {
            painter.rect_filled(square, CornerRadius::ZERO, fill);
            let c = square.center();
            painter.add(Shape::line(
                vec![Pos2::new(c.x - 4.5, c.y + 0.5), Pos2::new(c.x - 1.2, c.y + 3.8), Pos2::new(c.x + 4.8, c.y - 3.6)],
                stroke(2.0, p.on_accent),
            ));
        }
        (true, true) => {
            painter.rect_stroke(square, CornerRadius::ZERO, stroke(1.5, border), StrokeKind::Inside);
            painter.rect_filled(square.shrink(5.0), CornerRadius::ZERO, fill);
        }
    }
}

fn choice(ui: &mut Ui, checked: bool, label: &str, enabled: bool, radio: bool) -> Response {
    let p = palette(ui.ctx());
    let font = regular(14.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text);
    let width = 18.0 + 12.0 + galley.size().x;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), click_sense(enabled));
    let kind = if radio { WidgetType::RadioButton } else { WidgetType::Checkbox };
    response.widget_info(|| WidgetInfo::selected(kind, enabled, checked, label));
    focus_ring(ui, &response);
    draw_box(ui, Rect::from_min_size(rect.min, Vec2::new(18.0, rect.height())), checked, enabled, radio);
    let color = if enabled { p.text } else { p.text_faint };
    ui.painter().text(Pos2::new(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, label, font, color);
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// Square checkbox; the returned response is already marked as changed when clicked.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str, enabled: bool) -> Response {
    let mut response = choice(ui, *checked, label, enabled, false);
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response
}

pub fn radio(ui: &mut Ui, selected: bool, label: &str) -> Response {
    choice(ui, selected, label, true, true)
}

/// Checkbox square drawn without its label, for rows with their own layout; `label` is what screen readers say.
pub fn bare_checkbox(ui: &mut Ui, checked: &mut bool, label: &str, enabled: bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(18.0, 22.0), click_sense(enabled));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *checked, label));
    focus_ring(ui, &response);
    draw_box(ui, rect, *checked, enabled, false);
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// Small outlined uppercase tag, such as "ADMIN".
pub fn tag(ui: &mut Ui, label: &str, color: Color32) {
    let font = semibold(9.5);
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font.clone(), color);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 10.0, 17.0), Sense::hover());
    ui.painter().rect_stroke(rect, CornerRadius::ZERO, stroke(1.0, color), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label.to_uppercase(), font, color);
}

pub fn status_color(p: &Palette, status: Status) -> Color32 {
    match status {
        Status::Ok => p.text,
        Status::Warning => p.accent_text,
        Status::Neutral => p.text_muted,
    }
}

/// Left-aligned text in a fixed-width column.
pub fn column(ui: &mut Ui, width: f32, content: RichText) {
    ui.allocate_ui_with_layout(Vec2::new(width, 20.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.set_min_width(width);
        ui.add(egui::Label::new(content).truncate());
    });
}

/// "Label — value" row, with the hint right below the value when there is one.
pub fn item_row(ui: &mut Ui, item: &Item, label_width: f32) {
    let p = palette(ui.ctx());
    ui.add_space(9.0);
    ui.horizontal_top(|ui| {
        column(ui, label_width, text(&item.label, regular(14.0), p.text_muted));
        ui.vertical(|ui| {
            ui.add(egui::Label::new(text(&item.value, semibold(14.0), status_color(p, item.status))).wrap());
            if let Some(hint) = &item.hint {
                ui.add(egui::Label::new(text(format!("→ {hint}"), regular(12.5), p.text_muted)).wrap());
            }
        });
    });
    ui.add_space(9.0);
    rule(ui);
}

pub fn loading(ui: &mut Ui, message: &str) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(16.0).color(p.text));
        ui.label(text(message, regular(14.0), p.text_muted));
    });
}

/// The brand mark on a 32-unit grid: a full bar over a half bar, each as `[x, y, width, height]`.
const MARK_GRID: u32 = 32;
const MARK_BARS: [[u32; 4]; 2] = [[8, 10, 16, 4], [8, 18, 8, 4]];

/// The brand mark, `size` points wide: the two white bars on the accent square.
pub fn mark(ui: &mut Ui, size: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let unit = size / MARK_GRID as f32;
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, p.accent);
    for [x, y, width, height] in MARK_BARS {
        let bar = Rect::from_min_size(rect.min + Vec2::new(x as f32, y as f32) * unit, Vec2::new(width as f32, height as f32) * unit);
        painter.rect_filled(bar, CornerRadius::ZERO, p.on_accent);
    }
}

/// 32×32 window icon: the brand mark.
pub fn icon() -> egui::IconData {
    let mut rgba = Vec::with_capacity((MARK_GRID * MARK_GRID * 4) as usize);
    for y in 0..MARK_GRID {
        for x in 0..MARK_GRID {
            let bar = MARK_BARS.iter().any(|&[left, top, width, height]| (left..left + width).contains(&x) && (top..top + height).contains(&y));
            let (r, g, b) = if bar { (255, 255, 255) } else { (227, 0, 27) };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    egui::IconData { rgba, width: MARK_GRID, height: MARK_GRID }
}
