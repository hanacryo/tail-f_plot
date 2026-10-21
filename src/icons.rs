//! Toolbar icons drawn with the painter on a 24×24 grid, the same shapes as
//! the HANA framework's `assets/icons` SVGs. Drawn as vectors every frame, so
//! they stay sharp at any DPI and take the widget's text color, which follows
//! the theme and the hovered / selected state.

use eframe::egui;
use egui::epaint::CubicBezierShape;
use egui::widget_style::WidgetState;
use egui::{Color32, Painter, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// plot_fit_all.svg
    FitAll,
    /// zoom_in.svg
    ZoomIn,
    /// zoom_out.svg
    ZoomOut,
    /// plot_auto_x.svg
    AutoX,
    /// plot_auto_y.svg
    AutoY,
    /// graph_linear.svg (the Y axis is linear now)
    Linear,
    /// graph_log.svg (the Y axis is log now)
    Log,
}

/// Paint `icon` centered in `rect`, scaled from the 24×24 grid (stroke width
/// 2 units). egui strokes have no round caps, so a shaft that meets an
/// arrowhead runs to the arrow tip instead of stopping one unit short.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let k = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - Vec2::splat(12.0 * k);
    let at = |x: f32, y: f32| origin + Vec2::new(x, y) * k;
    let stroke = Stroke::new((2.0 * k).max(1.2), color);
    let path = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    };
    let curve = |pts: [(f32, f32); 4]| {
        let pts = pts.map(|(x, y)| at(x, y));
        painter.add(CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, stroke));
    };
    let axes = || {
        path(&[(4.0, 21.0), (4.0, 3.0)]);
        path(&[(2.0, 5.0), (4.0, 3.0), (6.0, 5.0)]);
        path(&[(3.0, 20.0), (21.0, 20.0)]);
        path(&[(19.0, 18.0), (21.0, 20.0), (19.0, 22.0)]);
    };
    match icon {
        Icon::FitAll => {
            // Outer corners, the two diagonals, inner corners
            path(&[(7.0, 2.0), (2.0, 2.0), (2.0, 7.0)]);
            path(&[(17.0, 2.0), (22.0, 2.0), (22.0, 7.0)]);
            path(&[(7.0, 22.0), (2.0, 22.0), (2.0, 17.0)]);
            path(&[(17.0, 22.0), (22.0, 22.0), (22.0, 17.0)]);
            path(&[(6.0, 6.0), (18.0, 18.0)]);
            path(&[(18.0, 6.0), (6.0, 18.0)]);
            path(&[(9.0, 6.0), (6.0, 6.0), (6.0, 9.0)]);
            path(&[(15.0, 6.0), (18.0, 6.0), (18.0, 9.0)]);
            path(&[(9.0, 18.0), (6.0, 18.0), (6.0, 15.0)]);
            path(&[(15.0, 18.0), (18.0, 18.0), (18.0, 15.0)]);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            painter.circle_stroke(at(10.0, 10.0), 6.0 * k, stroke);
            path(&[(7.0, 10.0), (13.0, 10.0)]);
            if icon == Icon::ZoomIn {
                path(&[(10.0, 7.0), (10.0, 13.0)]);
            }
            path(&[(14.5, 14.5), (21.0, 21.0)]);
        }
        Icon::AutoX => {
            path(&[(3.0, 4.0), (3.0, 20.0)]);
            path(&[(21.0, 4.0), (21.0, 20.0)]);
            path(&[(6.0, 12.0), (18.0, 12.0)]);
            path(&[(9.0, 9.0), (6.0, 12.0), (9.0, 15.0)]);
            path(&[(15.0, 9.0), (18.0, 12.0), (15.0, 15.0)]);
        }
        Icon::AutoY => {
            path(&[(4.0, 3.0), (20.0, 3.0)]);
            path(&[(4.0, 21.0), (20.0, 21.0)]);
            path(&[(12.0, 6.0), (12.0, 18.0)]);
            path(&[(9.0, 9.0), (12.0, 6.0), (15.0, 9.0)]);
            path(&[(9.0, 15.0), (12.0, 18.0), (15.0, 15.0)]);
        }
        Icon::Linear => {
            axes();
            path(&[(6.0, 19.0), (21.0, 4.0)]);
        }
        Icon::Log => {
            axes();
            curve([(4.5, 18.0), (6.0, 13.0), (11.0, 9.0), (21.0, 8.0)]);
            curve([(4.5, 6.0), (6.0, 11.0), (11.0, 15.0), (21.0, 16.0)]);
        }
    }
}

/// Height of every toolbar button; text buttons use it as their minimum
/// height so they line up with the icon buttons.
pub fn button_height(ui: &Ui) -> f32 {
    ui.spacing().interact_size.y + 4.0
}

/// Icon-only button that looks like `Button::selectable`: no frame until
/// hovered, filled with the selection color while `selected`. `tip` is the
/// hover text and the accessibility label.
pub fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, tip: &str) -> Response {
    let height = button_height(ui);
    // 3 points around the icon, plus 2 more left and right: slightly wide
    let icon_side = height - 6.0;
    let size = Vec2::new(height + 4.0, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, tip)
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.widget_state() != WidgetState::Inactive {
            ui.painter().rect(
                rect.expand(visuals.expansion),
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                StrokeKind::Inside,
            );
        }
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_side));
        paint_icon(ui.painter(), icon_rect, icon, visuals.text_color());
    }
    if response.hovered() {
        if let Some(cursor) = ui.visuals().interact_cursor {
            ui.ctx().set_cursor_icon(cursor);
        }
    }
    response.on_hover_text(tip)
}
