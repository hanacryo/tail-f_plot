use eframe::egui;
use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Rect, StrokeKind, TextFormat, Ui};
use egui_plot::{PlotPoint, PlotTransform};

use crate::plot_state::SeriesData;

/// One series' value at the hovered row.
pub struct SeriesValue {
    pub name: String,
    pub color: [u8; 3],
    pub y: f64,
}

/// Result of hover detection. The tooltip layout is always the same:
/// row 1 = `x`, row 2 = the free cursor `y`, rows 3.. = each visible series'
/// value at the hovered row (in legend order).
pub struct HoverResult {
    /// X of the hovered data row (cursor X when there is no data).
    pub x: f64,
    /// Y shown on row 2: the snapped point's Y when snapped, otherwise the
    /// free cursor's Y. Real value even under a log scale.
    pub pointer_y: f64,
    /// Screen position to anchor the tooltip box / crosshair at: the snapped
    /// point when snapped, otherwise the cursor.
    pub anchor: egui::Pos2,
    /// Visible series values at the hovered row, in legend order.
    pub values: Vec<SeriesValue>,
}

/// Detect the hovered data row and collect every visible series' value on it.
///
/// All series are row-aligned (every `push_row` appends to all series), so a
/// single row index yields one shared X and one Y per series. The row is
/// chosen by point-snap when the cursor is within `interact_radius_sq` of a
/// plotted point, otherwise by nearest X to the cursor.
pub fn find_hover(
    hover_pos: egui::Pos2,
    transform: &PlotTransform,
    series: &[SeriesData],
    series_visible: &[bool],
    colors: &[[u8; 3]],
    render_range: Option<(usize, usize)>,
    is_log_scale: bool,
    interact_radius_sq: f32,
) -> HoverResult {
    let hover_value = transform.value_from_position(hover_pos);
    // The cursor's plot Y is in log10 space under a log scale; recover the
    // real value so the `y =` row reads in the same units as the data.
    let cursor_y = if is_log_scale {
        10_f64.powf(hover_value.y)
    } else {
        hover_value.y
    };

    let no_data = HoverResult {
        x: hover_value.x,
        pointer_y: cursor_y,
        anchor: hover_pos,
        values: Vec::new(),
    };

    if series.is_empty() {
        return no_data;
    }
    let total = series[0].points.len();
    let (start_idx, end_idx) = render_range.unwrap_or((0, total));
    let start_idx = start_idx.min(total);
    let end_idx = end_idx.min(total);
    if start_idx >= end_idx {
        return no_data;
    }

    // Binary search for the cursor's X within the visible range.
    let hv_idx = series[0].points[start_idx..end_idx]
        .partition_point(|(x, _)| *x < hover_value.x)
        + start_idx;

    // --- Snap detection: closest visible plotted point within radius ---
    let check_start = hv_idx.saturating_sub(5).max(start_idx);
    let check_end = (hv_idx + 5).min(end_idx);
    // (row_idx, raw_y, dist_sq, screen_pos)
    let mut snap: Option<(usize, f64, f32, egui::Pos2)> = None;

    for (i, s) in series.iter().enumerate() {
        if !series_visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        for j in check_start..check_end {
            let (x, y_opt) = s.points[j];
            if let Some(y) = y_opt {
                let y_render = if is_log_scale {
                    if y > 0.0 {
                        y.log10()
                    } else {
                        continue;
                    }
                } else {
                    y
                };
                let pt = PlotPoint::new(x, y_render);
                let screen_pos = transform.position_from_point(&pt);
                let dist_sq = hover_pos.distance_sq(screen_pos);
                if dist_sq <= interact_radius_sq
                    && snap.as_ref().map_or(true, |(_, _, d, _)| dist_sq < *d)
                {
                    snap = Some((j, y, dist_sq, screen_pos));
                }
            }
        }
    }

    // --- Pick the target row, anchor, and the row-2 pointer Y ---
    // When snapped, both the anchor (crosshair / tooltip) and the row-2 `y`
    // follow the snapped point rather than the free cursor.
    let (row_idx, anchor, pointer_y) = if let Some((idx, snap_y, _, pos)) = snap {
        (idx, pos, snap_y)
    } else {
        // Not snapped: nearest row by X to the cursor.
        let idx = if hv_idx >= end_idx {
            end_idx - 1
        } else if hv_idx <= start_idx {
            start_idx
        } else {
            let x_lo = series[0].points[hv_idx - 1].0;
            let x_hi = series[0].points[hv_idx].0;
            if (hover_value.x - x_lo).abs() <= (x_hi - hover_value.x).abs() {
                hv_idx - 1
            } else {
                hv_idx
            }
        };
        (idx, hover_pos, cursor_y)
    };

    // Collect every visible series' value at the row, in legend order.
    let x = series[0].points[row_idx].0;
    let mut values = Vec::new();
    for (i, s) in series.iter().enumerate() {
        if !series_visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some((_, Some(y))) = s.points.get(row_idx).copied() {
            values.push(SeriesValue {
                name: s.name.clone(),
                color: colors[i % colors.len()],
                y,
            });
        }
    }
    // Larger values first; `sort_by` is stable so ties keep legend order.
    values.sort_by(|a, b| {
        b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal)
    });

    HoverResult {
        x,
        pointer_y,
        anchor,
        values,
    }
}

/// Draw faint dashed crosshair lines tracking the cursor across the plot.
pub fn render_crosshair(ui: &Ui, frame_rect: Rect, cursor: egui::Pos2) {
    if !frame_rect.contains(cursor) {
        return;
    }
    // Faint, theme-aware: tint of the current text color so it shows on both
    // light and dark backgrounds.
    let base = ui.visuals().text_color();
    let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 70);
    let painter = ui.painter().with_clip_rect(frame_rect);
    let stroke = egui::Stroke::new(1.0, color);
    // Vertical line.
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(cursor.x, frame_rect.top()),
            egui::pos2(cursor.x, frame_rect.bottom()),
        ],
        stroke,
        4.0,
        4.0,
    ));
    // Horizontal line.
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(frame_rect.left(), cursor.y),
            egui::pos2(frame_rect.right(), cursor.y),
        ],
        stroke,
        4.0,
        4.0,
    ));
}

/// Render the tooltip: row 1 is the X (`t =`/`x =`), row 2 is the free cursor
/// `y =`, and rows 3.. are each visible series (`swatch + name = value`) at
/// the hovered row, in legend order.
pub fn render_tooltip(
    ui: &Ui,
    frame_rect: Rect,
    hover: &HoverResult,
    x_unit: &str,
    x_proportion: f64,
    is_log_scale: bool,
    y_unit: &str,
) {
    if !frame_rect.contains(hover.anchor) {
        return;
    }

    // Theme-aware colors: text follows the current visuals, the translucent
    // background is dark-on-dark / light-on-light.
    let dark_mode = ui.visuals().dark_mode;
    let text_color = ui.visuals().text_color();
    let bg_color = if dark_mode {
        Color32::from_black_alpha(200)
    } else {
        Color32::from_white_alpha(220)
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

    // Build rows as (galley, optional swatch color).
    let mut rows: Vec<(_, Option<[u8; 3]>)> = Vec::new();

    // Row 1: X.
    let mut x_job = LayoutJob::default();
    for (text, is_bold) in format_x_parts(hover.x, x_proportion, x_unit) {
        let fmt = if is_bold { bold_fmt.clone() } else { normal_fmt.clone() };
        x_job.append(&text, 0.0, fmt);
    }
    rows.push((ui.fonts_mut(|f| f.layout_job(x_job)), None));

    // Helper to build a `label = value (unit)` row galley.
    let value_row = |label: &str, y: f64| {
        let mut job = LayoutJob::default();
        job.append(label, 0.0, normal_fmt.clone());
        job.append(" = ", 0.0, normal_fmt.clone());
        job.append(&format_y_value(y, is_log_scale), 0.0, bold_fmt.clone());
        if !y_unit.is_empty() {
            job.append(&format!(" ({})", y_unit), 0.0, normal_fmt.clone());
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };

    // Row 2: pointer Y (snapped point's Y when snapped, else free cursor Y).
    rows.push((value_row("y", hover.pointer_y), None));

    // Rows 3..: visible series at the hovered row.
    for v in &hover.values {
        rows.push((value_row(&v.name, v.y), Some(v.color)));
    }

    // Geometry. Swatched rows reserve a column on the left; the X / Y rows are
    // indented to keep all text left-aligned with the swatched rows.
    let swatch_size = 10.0;
    let gap = 4.0;
    let has_swatch = rows.iter().any(|(_, c)| c.is_some());
    let text_x = if has_swatch { swatch_size + gap } else { 0.0 };
    let row_h = rows[0].0.size().y;
    let content_w = text_x
        + rows
            .iter()
            .map(|(g, _)| g.size().x)
            .fold(0.0_f32, f32::max);
    let content_h = row_h * rows.len() as f32;

    // Position: upper-right of anchor, with edge avoidance.
    let mut pos = egui::pos2(hover.anchor.x + 8.0, hover.anchor.y - 8.0 - content_h);
    if pos.x + content_w + 4.0 > frame_rect.right() {
        pos.x = hover.anchor.x - 8.0 - content_w;
    }
    if pos.y - 4.0 < frame_rect.top() {
        pos.y = hover.anchor.y + 8.0;
    }

    let content_rect = Rect::from_min_size(pos, egui::vec2(content_w, content_h));
    let painter = ui.painter().with_clip_rect(frame_rect);
    let bg_rect = content_rect.expand(4.0);
    painter.rect_filled(bg_rect, 2.0, bg_color);
    painter.rect_stroke(
        bg_rect,
        2.0,
        egui::Stroke::new(1.0, text_color),
        StrokeKind::Outside,
    );

    for (i, (galley, swatch)) in rows.into_iter().enumerate() {
        let row_y = pos.y + row_h * i as f32;
        if let Some([r, g, b]) = swatch {
            let swatch_rect = Rect::from_min_size(
                egui::pos2(pos.x, row_y + (row_h - swatch_size) / 2.0),
                egui::vec2(swatch_size, swatch_size),
            );
            painter.rect_filled(swatch_rect, 2.0, Color32::from_rgb(r, g, b));
        }
        painter.galley(egui::pos2(pos.x + text_x, row_y), galley, text_color);
    }
}

/// Format X value as parts with bold flag: Vec<(text, is_bold)>
fn format_x_parts(x: f64, x_proportion: f64, unit: &str) -> Vec<(String, bool)> {
    let (sign, abs_x) = if x < 0.0 { ("-", -x) } else { ("", x) };

    let is_day = matches!(unit, "d" | "day" | "days");
    let is_minute = matches!(unit, "min" | "minute" | "minutes");
    let is_hour = matches!(unit, "h" | "hour" | "hours");
    let is_second = matches!(unit, "s" | "sec" | "second" | "seconds");

    if (x_proportion - 1.0 / 86400.0).abs() < 1e-6 && is_day {
        let whole_days = abs_x.floor() as i64;
        let remaining_h = (abs_x - whole_days as f64) * 24.0;
        let whole_hours = remaining_h.floor() as i64;
        let remaining_m = (remaining_h - whole_hours as f64) * 60.0;
        let whole_min = remaining_m.floor() as i64;
        let seconds = (remaining_m - whole_min as f64) * 60.0;
        let sec_int = seconds.round() as i64;
        vec![
            (format!("t = {}", sign), false),
            (format!("{}", whole_days), true),
            (format!("d {:02}:{:02}:{:02}", whole_hours, whole_min, sec_int), false),
        ]
    } else if (x_proportion - 1.0 / 60.0).abs() < 1e-6 && is_minute {
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

/// Format a raw Y value for display (scientific in log scale, fixed otherwise).
fn format_y_value(y: f64, is_log_scale: bool) -> String {
    if is_log_scale {
        let s = format!("{:.6e}", y);
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
    }
}
