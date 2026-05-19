use eframe::egui;
use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, TextFormat};
use egui_plot::Corner;

use crate::plot_state::SeriesData;

/// Render custom legend overlay with bold value formatting and click-to-toggle.
pub fn render_legend(
    ctx: &egui::Context,
    frame_rect: egui::Rect,
    corner: Corner,
    series: &[SeriesData],
    series_visible: &mut Vec<bool>,
    colors: &[[u8; 3]],
    render_range: Option<(usize, usize)>,
    y_unit: &str,
    is_log_scale: bool,
) {
    // Ensure series_visible has entries for all series
    while series_visible.len() < series.len() {
        series_visible.push(true);
    }

    // Collect legend entry data
    let entries: Vec<(String, [u8; 3], bool, Option<f64>)> = series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let visible = series_visible.get(i).copied().unwrap_or(true);
            let color = colors[i % colors.len()];
            let (si, ei) = render_range.unwrap_or((0, s.points.len()));
            let slice = if si < s.points.len() {
                &s.points[si..ei.min(s.points.len())]
            } else {
                &[]
            };
            let latest = slice.iter().rev().find_map(|(_, y)| *y);
            (s.name.clone(), color, visible, latest)
        })
        .collect();

    // Theme-aware colors: text follows the current visuals, the translucent
    // background is dark-on-dark / light-on-light.
    let base_text = ctx.global_style().visuals.text_color();
    let legend_fill = if ctx.global_style().visuals.dark_mode {
        Color32::from_black_alpha(180)
    } else {
        Color32::from_white_alpha(200)
    };

    let (legend_anchor, legend_pivot) = match corner {
        Corner::LeftTop => (
            frame_rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
        ),
        Corner::RightTop => (
            frame_rect.right_top() + egui::vec2(-8.0, 8.0),
            egui::Align2::RIGHT_TOP,
        ),
        Corner::LeftBottom => (
            frame_rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
        ),
        Corner::RightBottom => (
            frame_rect.right_bottom() + egui::vec2(-8.0, -8.0),
            egui::Align2::RIGHT_BOTTOM,
        ),
    };

    egui::Area::new(egui::Id::new("plot_legend"))
        .fixed_pos(legend_anchor)
        .pivot(legend_pivot)
        .order(egui::Order::Foreground)
        .movable(false)
        .interactable(true)
        .show(ctx, |legend_ui| {
            egui::Frame::NONE
                .fill(legend_fill)
                .stroke(egui::Stroke::new(1.0, base_text))
                .corner_radius(2.0)
                .inner_margin(6.0)
                .show(legend_ui, |legend_ui| {
                    for (i, (name, color_rgb, visible, latest)) in entries.iter().enumerate() {
                        let [r, g, b] = *color_rgb;
                        let series_color = if *visible {
                            Color32::from_rgb(r, g, b)
                        } else {
                            Color32::from_rgba_unmultiplied(r, g, b, 80)
                        };
                        let text_color = if *visible {
                            base_text
                        } else {
                            Color32::from_rgba_unmultiplied(
                                base_text.r(),
                                base_text.g(),
                                base_text.b(),
                                80,
                            )
                        };

                        let normal_fmt = TextFormat {
                            font_id: FontId::new(13.0, FontFamily::Proportional),
                            color: text_color,
                            ..Default::default()
                        };
                        let bold_fmt = TextFormat {
                            font_id: FontId::new(13.0, FontFamily::Name("Bold".into())),
                            color: text_color,
                            ..Default::default()
                        };

                        let mut job = LayoutJob::default();
                        job.append(name, 0.0, normal_fmt.clone());
                        if let Some(v) = latest {
                            let display_val = if is_log_scale {
                                let s = format!("{:.6e}", v);
                                if let Some(e_pos) = s.find('e') {
                                    let (m, exp) = s.split_at(e_pos);
                                    let t = m.trim_end_matches('0').trim_end_matches('.');
                                    format!("{}{}", t, exp)
                                } else {
                                    s
                                }
                            } else {
                                let s = format!("{:.6}", v);
                                s.trim_end_matches('0').trim_end_matches('.').to_string()
                            };
                            job.append(" : ", 0.0, normal_fmt.clone());
                            job.append(&display_val, 0.0, bold_fmt);
                            job.append(&format!("({})", y_unit), 0.0, normal_fmt);
                        }

                        legend_ui.horizontal(|ui| {
                            let (marker_rect, _) = ui.allocate_exact_size(
                                egui::vec2(10.0, 10.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(marker_rect, 2.0, series_color);
                            let resp = ui.add(
                                egui::Label::new(job)
                                    .selectable(false)
                                    .sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                if let Some(v) = series_visible.get_mut(i) {
                                    *v = !*v;
                                }
                            }
                        });
                    }
                });
        });
}
