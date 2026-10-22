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
    /// gear.svg
    Gear,
}

/// gear.svg's outline path (arcs and lines) flattened to a closed polyline,
/// arcs sampled every 15° or finer.
const GEAR_OUTLINE: [(f32, f32); 194] = [
    (19.400, 15.000), (19.285, 15.375), (19.263, 15.766), (19.332, 16.151),
    (19.491, 16.509), (19.730, 16.820), (19.790, 16.880), (20.070, 17.230),
    (20.264, 17.634), (20.364, 18.071), (20.364, 18.519), (20.264, 18.956),
    (20.070, 19.360), (19.790, 19.710), (19.440, 19.990), (19.036, 20.184),
    (18.599, 20.284), (18.151, 20.284), (17.714, 20.184), (17.310, 19.990),
    (16.960, 19.710), (16.900, 19.650), (16.589, 19.411), (16.231, 19.252),
    (15.846, 19.183), (15.455, 19.205), (15.080, 19.320), (14.748, 19.510),
    (14.469, 19.772), (14.258, 20.091), (14.126, 20.450), (14.080, 20.830),
    (14.080, 21.000), (14.012, 21.518), (13.812, 22.000), (13.494, 22.414),
    (13.080, 22.732), (12.598, 22.932), (12.080, 23.000), (11.562, 22.932),
    (11.080, 22.732), (10.666, 22.414), (10.348, 22.000), (10.148, 21.518),
    (10.080, 21.000), (10.080, 20.910), (10.024, 20.521), (9.877, 20.156), (9.649, 19.836),
    (9.350, 19.579), (9.000, 19.400), (8.625, 19.285), (8.234, 19.263), (7.849, 19.332),
    (7.491, 19.491), (7.180, 19.730), (7.120, 19.790), (6.706, 20.108), (6.223, 20.308),
    (5.705, 20.376), (5.187, 20.308), (4.704, 20.108), (4.290, 19.790), (3.972, 19.376),
    (3.772, 18.893), (3.704, 18.375), (3.772, 17.857), (3.972, 17.374), (4.290, 16.960),
    (4.350, 16.900), (4.606, 16.578), (4.774, 16.203), (4.844, 15.799), (4.812, 15.389),
    (4.680, 15.000), (4.490, 14.668), (4.228, 14.389), (3.909, 14.178), (3.550, 14.046),
    (3.170, 14.000), (3.000, 14.000), (2.482, 13.932), (2.000, 13.732), (1.586, 13.414),
    (1.268, 13.000), (1.068, 12.518), (1.000, 12.000), (1.068, 11.482), (1.268, 11.000),
    (1.586, 10.586), (2.000, 10.268), (2.482, 10.068), (3.000, 10.000), (3.090, 10.000),
    (3.470, 9.954), (3.829, 9.822), (4.148, 9.611), (4.410, 9.332), (4.600, 9.000),
    (4.715, 8.625), (4.737, 8.234), (4.668, 7.849), (4.509, 7.491), (4.270, 7.180),
    (4.210, 7.120), (3.892, 6.706), (3.692, 6.223), (3.624, 5.705), (3.692, 5.187),
    (3.892, 4.704), (4.210, 4.290), (4.624, 3.972), (5.107, 3.772), (5.625, 3.704),
    (6.143, 3.772), (6.626, 3.972), (7.040, 4.290), (7.100, 4.350), (7.422, 4.606),
    (7.797, 4.774), (8.201, 4.844), (8.611, 4.812), (9.000, 4.680), (9.332, 4.490),
    (9.611, 4.228), (9.822, 3.909), (9.954, 3.550), (10.000, 3.170), (10.000, 3.000),
    (10.068, 2.482), (10.268, 2.000), (10.586, 1.586), (11.000, 1.268), (11.482, 1.068),
    (12.000, 1.000), (12.518, 1.068), (13.000, 1.268), (13.414, 1.586), (13.732, 2.000),
    (13.932, 2.482), (14.000, 3.000), (14.000, 3.090), (14.046, 3.470), (14.178, 3.829),
    (14.389, 4.148), (14.668, 4.410), (15.000, 4.600), (15.375, 4.715), (15.766, 4.737),
    (16.151, 4.668), (16.509, 4.509), (16.820, 4.270), (16.880, 4.210), (17.294, 3.892),
    (17.777, 3.692), (18.295, 3.624), (18.813, 3.692), (19.296, 3.892), (19.710, 4.210),
    (20.028, 4.624), (20.228, 5.107), (20.296, 5.625), (20.228, 6.143), (20.028, 6.626),
    (19.710, 7.040), (19.650, 7.100), (19.411, 7.430), (19.260, 7.809), (19.207, 8.214),
    (19.254, 8.619), (19.400, 9.000), (19.590, 9.332), (19.852, 9.611), (20.171, 9.822),
    (20.530, 9.954), (20.910, 10.000), (21.000, 10.000), (21.518, 10.068), (22.000, 10.268),
    (22.414, 10.586), (22.732, 11.000), (22.932, 11.482), (23.000, 12.000),
    (22.932, 12.518), (22.732, 13.000), (22.414, 13.414), (22.000, 13.732),
    (21.518, 13.932), (21.000, 14.000), (20.910, 14.000), (20.530, 14.046),
    (20.171, 14.178), (19.852, 14.389), (19.590, 14.668),
];

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
        Icon::Gear => {
            painter.circle_stroke(at(12.0, 12.0), 3.0 * k, stroke);
            let outline = GEAR_OUTLINE.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(Shape::closed_line(outline, stroke));
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
