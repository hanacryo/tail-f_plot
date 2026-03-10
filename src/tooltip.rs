use eframe::egui;
use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Rect, StrokeKind, TextFormat, Ui};
use egui_plot::{PlotPoint, PlotTransform};

use crate::plot_state::SeriesData;

/// Result of hover detection: series name + plot point
pub struct HoverResult {
    pub name: String,
    pub value: PlotPoint,
}

/// Find the closest visible data point to the cursor position.
/// Returns None if no point is within `interact_radius_sq` pixels (squared).
pub fn find_closest_point(
    hover_pos: egui::Pos2,
    transform: &PlotTransform,
    series: &[SeriesData],
    series_visible: &[bool],
    render_range: Option<(usize, usize)>,
    is_log_scale: bool,
    interact_radius_sq: f32,
) -> Option<HoverResult> {
    let hover_value = transform.value_from_position(hover_pos);
    let mut best: Option<(String, PlotPoint, f32)> = None;

    for (i, s) in series.iter().enumerate() {
        if !series_visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        let (start_idx, end_idx) = render_range.unwrap_or((0, s.points.len()));
        let points_slice = if start_idx < s.points.len() {
            &s.points[start_idx..end_idx.min(s.points.len())]
        } else {
            &[]
        };

        // Binary search for closest X value, then check nearby points
        let idx = points_slice.partition_point(|(x, _)| *x < hover_value.x);
        let check_start = idx.saturating_sub(5);
        let check_end = (idx + 5).min(points_slice.len());

        for j in check_start..check_end {
            let (x, y_opt) = points_slice[j];
            if let Some(y) = y_opt {
                let y_render = if is_log_scale {
                    if y > 0.0 { y.log10() } else { continue; }
                } else {
                    y
                };
                let pt = PlotPoint::new(x, y_render);
                let screen_pos = transform.position_from_point(&pt);
                let dist_sq = hover_pos.distance_sq(screen_pos);

                if dist_sq <= interact_radius_sq {
                    if best.as_ref().map_or(true, |(_, _, d)| dist_sq < *d) {
                        best = Some((s.name.clone(), pt, dist_sq));
                    }
                }
            }
        }
    }

    best.map(|(name, value, _)| HoverResult { name, value })
}

/// Render custom tooltip with bold/normal mixed text at the given anchor position.
pub fn render_tooltip(
    ui: &Ui,
    frame_rect: Rect,
    anchor_pos: egui::Pos2,
    name: &str,
    value: &PlotPoint,
    x_axis_label: &str,
    x_proportion: f64,
    is_log_scale: bool,
    y_unit: &str,
) {
    if !frame_rect.contains(anchor_pos) {
        return;
    }

    let legend_name = if !name.is_empty() {
        name.to_string()
    } else {
        String::new()
    };

    let x_unit = extract_x_unit(x_axis_label);
    let x_parts = format_x_parts(value.x, x_proportion, x_unit);
    let y_parts = format_y_parts(value.y, is_log_scale, y_unit);

    let text_color = Color32::WHITE;
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

    if !legend_name.is_empty() {
        job.append(&legend_name, 0.0, bold_fmt.clone());
        job.append("\n", 0.0, TextFormat::default());
    }

    for (text, is_bold) in &x_parts {
        let fmt = if *is_bold { bold_fmt.clone() } else { normal_fmt.clone() };
        job.append(text, 0.0, fmt);
    }

    job.append("\n", 0.0, TextFormat::default());
    for (text, is_bold) in &y_parts {
        let fmt = if *is_bold { bold_fmt.clone() } else { normal_fmt.clone() };
        job.append(text, 0.0, fmt);
    }

    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let galley_size = galley.size();

    // Position: upper-right of anchor, with edge avoidance
    let mut pos = egui::pos2(anchor_pos.x + 8.0, anchor_pos.y - 8.0 - galley_size.y);

    if pos.x + galley_size.x + 4.0 > frame_rect.right() {
        pos.x = anchor_pos.x - 8.0 - galley_size.x;
    }
    if pos.y - 4.0 < frame_rect.top() {
        pos.y = anchor_pos.y + 8.0;
    }

    let tooltip_rect = Rect::from_min_size(pos, galley_size);
    let painter = ui.painter().with_clip_rect(frame_rect);
    let bg_rect = tooltip_rect.expand(4.0);
    painter.rect_filled(bg_rect, 2.0, Color32::from_black_alpha(180));
    painter.rect_stroke(
        bg_rect,
        2.0,
        egui::Stroke::new(1.0, Color32::WHITE),
        StrokeKind::Outside,
    );
    painter.galley(pos, galley, text_color);
}

/// Extract unit from axis label parentheses, e.g. "Time (min)" -> "min"
fn extract_x_unit(label: &str) -> &str {
    if let Some(start) = label.rfind('(') {
        if let Some(end) = label[start..].find(')') {
            return &label[start + 1..start + end];
        }
    }
    ""
}

/// Format X value as parts with bold flag: Vec<(text, is_bold)>
fn format_x_parts(x: f64, x_proportion: f64, unit: &str) -> Vec<(String, bool)> {
    let (sign, abs_x) = if x < 0.0 { ("-", -x) } else { ("", x) };

    let is_minute = matches!(unit, "min" | "minute" | "minutes");
    let is_hour = matches!(unit, "h" | "hour" | "hours");
    let is_second = matches!(unit, "s" | "sec" | "second" | "seconds");

    if (x_proportion - 1.0 / 60.0).abs() < 1e-6 && is_minute {
        let whole_min = abs_x.floor() as i64;
        let seconds = (abs_x - whole_min as f64) * 60.0;
        let sec_1dec = (seconds * 10.0).round() / 10.0;
        let sec_int = sec_1dec.floor() as i64;
        let frac = ((sec_1dec - sec_int as f64) * 10.0).round() as i64;
        let sec_str = if frac == 0 {
            format!(":{:02}", sec_int)
        } else {
            format!(":{:02}.{}", sec_int, frac)
        };
        vec![
            (format!("t = {}", sign), false),
            (format!("{}", whole_min), true),
            (sec_str, false),
        ]
    } else if (x_proportion - 1.0 / 3600.0).abs() < 1e-6 && is_hour {
        let whole_hours = abs_x.floor() as i64;
        let remaining = (abs_x - whole_hours as f64) * 60.0;
        let whole_min = remaining.floor() as i64;
        let seconds = (remaining - whole_min as f64) * 60.0;
        let sec_1dec = (seconds * 10.0).round() / 10.0;
        let sec_int = sec_1dec.floor() as i64;
        let frac = ((sec_1dec - sec_int as f64) * 10.0).round() as i64;
        let sec_str = if frac == 0 {
            format!(":{:02}", sec_int)
        } else {
            format!(":{:02}.{}", sec_int, frac)
        };
        vec![
            (format!("t = {}", sign), false),
            (format!("{}", whole_hours), true),
            (format!(":{:02}", whole_min), false),
            (sec_str, false),
        ]
    } else if is_second {
        let s = format!("{}{:.1}", sign, abs_x);
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        vec![
            ("t = ".to_string(), false),
            (trimmed.to_string(), true),
        ]
    } else if unit.is_empty() {
        let s = format!("{}{:.2}", sign, abs_x);
        vec![
            ("x = ".to_string(), false),
            (s, true),
        ]
    } else {
        let s = format!("{}{:.2}", sign, abs_x);
        vec![
            ("x = ".to_string(), false),
            (s, true),
            (format!(" ({})", unit), false),
        ]
    }
}

/// Format Y value as parts with bold flag: Vec<(text, is_bold)>
fn format_y_parts(y: f64, is_log_scale: bool, unit: &str) -> Vec<(String, bool)> {
    let formatted = if is_log_scale {
        let real_value = 10_f64.powf(y);
        let s = format!("{:.6e}", real_value);
        if let Some(e_pos) = s.find('e') {
            let (mantissa, exp) = s.split_at(e_pos);
            let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
            format!("{}{}", trimmed, exp)
        } else {
            s
        }
    } else {
        let abs_val = y.abs();
        let s = if abs_val >= 1000.0 {
            format!("{:.1}", y)
        } else if abs_val >= 100.0 {
            format!("{:.2}", y)
        } else if abs_val >= 10.0 {
            format!("{:.3}", y)
        } else if abs_val >= 1.0 {
            format!("{:.4}", y)
        } else {
            format!("{:.6}", y)
        };
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    vec![
        ("y = ".to_string(), false),
        (formatted, true),
        (format!(" ({})", unit), false),
    ]
}
