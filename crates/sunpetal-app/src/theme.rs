//! Нейтральная тема меню под игру (K-021, `docs/RECON-MENU.md`, «Правило лицензии»).
//! Только числа палитры (тёмно-синий фон, белые плитки, оранжевый курсор, прямые углы), ничего из ресурсов игры:
//! рамки и значки из ROM пользователя — отдельный слой поверх, когда они найдены.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Тёмно-синий фон окна и панелей.
pub const BACK: Color32 = Color32::from_rgb(0x10, 0x20, 0x40);
/// Плитка кнопки и поля в покое.
pub const TILE: Color32 = Color32::from_rgb(0x20, 0x30, 0x50);
/// Плитка под курсором мыши.
pub const TILE_HOT: Color32 = Color32::from_rgb(0x30, 0x48, 0x70);
/// Самое тёмное: фон полей ввода и полос.
/// Выбранная плитка: светлее `TILE_HOT`, белый текст читается (контраст ≥ 4,5).
pub const SELECTED: Color32 = Color32::from_rgb(0x28, 0x50, 0xa0);
pub const DEEP: Color32 = Color32::from_rgb(0x08, 0x12, 0x28);
/// Белый текст и белая плитка выбранного.
pub const WHITE: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
/// Оранжевый курсор.
pub const ORANGE: Color32 = Color32::from_rgb(0xf0, 0x90, 0x20);
/// Бирюзовый второй акцент (ссылки, подсказки).
pub const TEAL: Color32 = Color32::from_rgb(0x60, 0xc0, 0xd0);

/// Цвета и формы виджетов `egui` по палитре игры.
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let square = CornerRadius::ZERO;
    v.window_fill = BACK;
    v.panel_fill = BACK;
    v.extreme_bg_color = DEEP;
    v.faint_bg_color = Color32::from_rgb(0x18, 0x28, 0x48);
    v.window_corner_radius = square;
    v.menu_corner_radius = square;
    v.window_stroke = Stroke::new(2.0, WHITE);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.hyperlink_color = TEAL;
    v.selection.bg_fill = ORANGE;
    v.selection.stroke = Stroke::new(1.0, Color32::BLACK);
    let w = &mut v.widgets;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = square;
        s.expansion = 0.0;
    }
    w.noninteractive.bg_fill = BACK;
    w.noninteractive.weak_bg_fill = BACK;
    w.noninteractive.bg_stroke = Stroke::new(1.0, TILE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, WHITE);
    // в покое: тёмная плитка с чёрной окантовкой, белый текст
    w.inactive.bg_fill = TILE;
    w.inactive.weak_bg_fill = TILE;
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::BLACK);
    w.inactive.fg_stroke = Stroke::new(1.0, WHITE);
    // под мышью: светлее, оранжевая окантовка
    w.hovered.bg_fill = TILE_HOT;
    w.hovered.weak_bg_fill = TILE_HOT;
    w.hovered.bg_stroke = Stroke::new(1.5, ORANGE);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    // выбранное (фокус), нажатое, открытое: яркая синяя плитка с оранжевой окантовкой и белым текстом
    // (светлая заливка с чёрным текстом не годилась: egui красит текст кнопки не цветом этого состояния, B-32)
    for s in [&mut w.active, &mut w.open] {
        s.bg_fill = SELECTED;
        s.weak_bg_fill = SELECTED;
        s.bg_stroke = Stroke::new(2.0, ORANGE);
        s.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    }
    v
}

/// The menu's sizes times `k`: the text, the gaps, the buttons, the slider and the check mark.
pub fn zoom_style(st: &mut egui::Style, k: f32) {
    for f in st.text_styles.values_mut() {
        f.size *= k;
    }
    let sp = &mut st.spacing;
    sp.item_spacing *= k;
    sp.button_padding *= k;
    sp.interact_size *= k;
    sp.slider_width *= k;
    sp.icon_width *= k;
    sp.icon_width_inner *= k;
    sp.icon_spacing *= k;
    sp.combo_width *= k;
    sp.text_edit_width *= k;
    sp.indent *= k;
    sp.scroll.bar_width *= k;
}

/// Тема в силе для всех стилей (светлого и тёмного) контекста.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|st| st.visuals = visuals());
}

/// The corners of the cursor round `r`: four orange brackets, each `arm` long (the game's menu cursor, drawn from numbers).
pub fn cursor_corners(r: egui::Rect, arm: f32) -> [[egui::Pos2; 3]; 4] {
    let (a, b) = (r.left_top(), r.right_bottom());
    let (w, h) = (arm.min(r.width() / 2.0), arm.min(r.height() / 2.0));
    let p = egui::pos2;
    [[p(a.x, a.y + h), a, p(a.x + w, a.y)], [p(b.x - w, a.y), p(b.x, a.y), p(b.x, a.y + h)], [p(a.x, b.y - h), p(a.x, b.y), p(a.x + w, b.y)], [p(b.x - w, b.y), b, p(b.x, b.y - h)]]
}

/// Paints the cursor round the widget the pad or the keys are on.
pub fn paint_cursor(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    let Some(resp) = ctx.read_response(id) else { return };
    // a text field keeps its own caret and frame
    if ctx.text_edit_focused() {
        return;
    }
    // the window's own frame takes a focus stop too: no cursor round a whole window
    if resp.rect.height() > 120.0 {
        return;
    }
    let rect = resp.rect.expand(3.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sunpetal-cursor")));
    let clip = ctx.content_rect();
    for c in cursor_corners(rect.intersect(clip), 7.0) {
        painter.add(egui::Shape::line(c.to_vec(), Stroke::new(3.0, ORANGE)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Относительная яркость по WCAG.
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_corner_is_square_and_the_window_is_the_games_blue() {
        let v = visuals();
        assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
        for s in [&v.widgets.noninteractive, &v.widgets.inactive, &v.widgets.hovered, &v.widgets.active, &v.widgets.open] {
            assert_eq!(s.corner_radius, CornerRadius::ZERO);
        }
        assert_eq!(v.window_fill, BACK);
    }

    #[test]
    fn the_cursor_has_four_corners_that_fit_a_small_widget() {
        let r = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 4.0));
        let c = cursor_corners(r, 7.0);
        assert_eq!(c[0][1], r.left_top());
        assert_eq!(c[3][1], r.right_bottom());
        // an arm never longer than half the side
        assert!(c.iter().flatten().all(|p| r.expand(0.01).contains(*p)));
    }

    #[test]
    fn the_text_reads_on_every_tile() {
        let v = visuals();
        let w = &v.widgets;
        for (fill, text) in [
            (w.noninteractive.bg_fill, w.noninteractive.fg_stroke.color),
            (w.inactive.weak_bg_fill, w.inactive.fg_stroke.color),
            (w.hovered.weak_bg_fill, w.hovered.fg_stroke.color),
            (w.active.weak_bg_fill, w.active.fg_stroke.color),
            (w.open.weak_bg_fill, w.open.fg_stroke.color),
            (v.extreme_bg_color, WHITE),
            (w.active.weak_bg_fill, WHITE),
        ] {
            assert!(contrast(fill, text) >= 4.5, "{:?} on {:?}: {:.1}", text, fill, contrast(fill, text));
        }
    }
}
