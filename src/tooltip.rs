use eframe::egui;
use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Rect, StrokeKind, TextFormat, Ui};
use egui_plot::{PlotPoint, PlotTransform};

use crate::plot_state::{GroupView, PlotData};

/// One series' value at the hovered row.
pub struct SeriesValue {
    pub name: String,
    pub color: [u8; 3],
    pub y: f64,
    /// Y unit of the series' stack (shown after the value).
    pub unit: String,
    /// Stack index the series belongs to (tooltip rows group by stack).
    pub stack: usize,
}

/// Flat per-series stack metadata for `find_hover`.
pub struct StackContext<'a> {
    /// Stack index per flat series ((group, series) order, 1-based)
    pub series_stacks: &'a [usize],
    /// Y unit per flat series
    pub series_units: &'a [String],
    /// Stack index of the sub-plot under the cursor (snap targets only it)
    pub hovered_stack: usize,
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

/// Nearest row (by stored x) to `raw_target` within `range`, on a group's
/// row-aligned point buffer. None when the range holds no rows.
/// `sorted` mirrors `FileGroup::x_sorted`: false forces a linear scan, since
/// binary search would land on an arbitrary row for an unsorted X column.
fn nearest_row(
    points: &[(f64, Option<f64>)],
    range: (usize, usize),
    raw_target: f64,
    sorted: bool,
) -> Option<usize> {
    let start = range.0.min(points.len());
    let end = range.1.min(points.len());
    if start >= end {
        return None;
    }
    if !sorted {
        let mut best: Option<(usize, f64)> = None;
        for i in start..end {
            let d = (points[i].0 - raw_target).abs();
            if d.is_nan() {
                continue;
            }
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        // All-NaN range: fall back to the first row rather than reporting none.
        return Some(best.map_or(start, |(i, _)| i));
    }
    let idx = points[start..end].partition_point(|(x, _)| *x < raw_target) + start;
    Some(if idx >= end {
        end - 1
    } else if idx <= start {
        start
    } else {
        let x_lo = points[idx - 1].0;
        let x_hi = points[idx].0;
        if (raw_target - x_lo).abs() <= (x_hi - raw_target).abs() {
            idx - 1
        } else {
            idx
        }
    })
}

/// Whether `raw_target` lies within the plotted x extent of a group's visible
/// range. Outside it no line exists (the file's recording ended earlier, or
/// starts later), so no value should be reported there.
fn extent_contains(
    points: &[(f64, Option<f64>)],
    range: (usize, usize),
    raw_target: f64,
    sorted: bool,
) -> bool {
    let start = range.0.min(points.len());
    let end = range.1.min(points.len());
    if start >= end {
        return false;
    }
    if !sorted {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &(x, _) in &points[start..end] {
            if x < lo {
                lo = x;
            }
            if x > hi {
                hi = x;
            }
        }
        return lo <= hi && raw_target >= lo && raw_target <= hi;
    }
    raw_target >= points[start].0 && raw_target <= points[end - 1].0
}

/// Detect the hovered data row and collect every visible series' value on it.
///
/// Series within a file group are row-aligned, so one row index per group
/// yields one X and one Y per series of that group. The anchor row is chosen
/// by point-snap when the cursor is within `interact_radius_sq` of a plotted
/// point of the hovered stack's sub-plot, otherwise by nearest display X to
/// the cursor across all groups; every other group then contributes its own
/// row nearest to that X. Values come from every stack (the X axis is shared
/// across sub-plots), each labeled with its stack's unit.
/// `views` carries each group's visible index range and display offset
/// (None = the group is not renderable yet).
pub fn find_hover(
    hover_pos: egui::Pos2,
    transform: &PlotTransform,
    data: &PlotData,
    views: &[Option<GroupView>],
    stack: &StackContext<'_>,
    series_visible: &[bool],
    colors: &[[u8; 3]],
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

    // --- Snap detection: closest visible plotted point within radius ---
    // (group_idx, row_idx, raw_y, dist_sq, screen_pos)
    let mut snap: Option<(usize, usize, f64, f32, egui::Pos2)> = None;
    let mut flat_base = 0usize;

    for (gi, group) in data.groups.iter().enumerate() {
        let base = flat_base;
        flat_base += group.series.len();

        let Some(view) = views.get(gi).copied().flatten() else { continue };
        let Some(first) = group.series.first() else { continue };
        let start_idx = view.range.0.min(first.points.len());
        let end_idx = view.range.1.min(first.points.len());
        if start_idx >= end_idx {
            continue;
        }

        // Narrow the snap search to the cursor's neighbourhood by binary
        // search. An unsorted X column has no such neighbourhood — any row
        // may land near the cursor — so it scans the visible range instead.
        let (check_start, check_end) = if group.x_sorted {
            let raw_cursor = hover_value.x - view.offset;
            let hv_idx = first.points[start_idx..end_idx]
                .partition_point(|(x, _)| *x < raw_cursor)
                + start_idx;
            (
                hv_idx.saturating_sub(5).max(start_idx),
                (hv_idx + 5).min(end_idx),
            )
        } else {
            (start_idx, end_idx)
        };

        for (si, s) in group.series.iter().enumerate() {
            // Snap only to points drawn in the hovered sub-plot — other
            // stacks' screen coordinates belong to different transforms.
            if stack.series_stacks.get(base + si).copied().unwrap_or(1) != stack.hovered_stack {
                continue;
            }
            if !series_visible.get(base + si).copied().unwrap_or(true) {
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
                    let pt = PlotPoint::new(x + view.offset, y_render);
                    let screen_pos = transform.position_from_point(&pt);
                    let dist_sq = hover_pos.distance_sq(screen_pos);
                    if dist_sq <= interact_radius_sq
                        && snap.as_ref().map_or(true, |(_, _, _, d, _)| dist_sq < *d)
                    {
                        snap = Some((gi, j, y, dist_sq, screen_pos));
                    }
                }
            }
        }
    }

    // --- Pick the target display X, anchor, and the row-2 pointer Y ---
    // When snapped, both the anchor (crosshair / tooltip) and the row-2 `y`
    // follow the snapped point rather than the free cursor.
    let (target_x, anchor, pointer_y) = if let Some((gi, j, snap_y, _, pos)) = snap {
        let view = views[gi].expect("snapped group has a view");
        (data.groups[gi].series[0].points[j].0 + view.offset, pos, snap_y)
    } else {
        // Not snapped: nearest row by display X to the cursor, across groups.
        let mut best_x: Option<f64> = None;
        for (gi, group) in data.groups.iter().enumerate() {
            let Some(view) = views.get(gi).copied().flatten() else { continue };
            let Some(first) = group.series.first() else { continue };
            let Some(row) = nearest_row(
                &first.points,
                view.range,
                hover_value.x - view.offset,
                group.x_sorted,
            ) else {
                continue;
            };
            let dx = first.points[row].0 + view.offset;
            if best_x.map_or(true, |b| (dx - hover_value.x).abs() < (b - hover_value.x).abs()) {
                best_x = Some(dx);
            }
        }
        match best_x {
            Some(x) => (x, hover_pos, cursor_y),
            None => return no_data,
        }
    };

    // Collect every visible series' value at each group's row nearest to the
    // target X, in legend order.
    let mut values = Vec::new();
    let mut flat_base = 0usize;
    for (gi, group) in data.groups.iter().enumerate() {
        let base = flat_base;
        flat_base += group.series.len();

        let Some(view) = views.get(gi).copied().flatten() else { continue };
        let Some(first) = group.series.first() else { continue };
        let raw_target = target_x - view.offset;
        let Some(row) = nearest_row(&first.points, view.range, raw_target, group.x_sorted) else {
            continue;
        };
        // Showing a nearest-row value where the group draws no line would be
        // misleading — skip the group entirely.
        if !extent_contains(&first.points, view.range, raw_target, group.x_sorted) {
            continue;
        }
        for (si, s) in group.series.iter().enumerate() {
            if !series_visible.get(base + si).copied().unwrap_or(true) {
                continue;
            }
            if let Some((_, Some(y))) = s.points.get(row).copied() {
                values.push(SeriesValue {
                    name: s.name.clone(),
                    color: colors[(base + si) % colors.len()],
                    y,
                    unit: stack.series_units.get(base + si).cloned().unwrap_or_default(),
                    stack: stack.series_stacks.get(base + si).copied().unwrap_or(1),
                });
            }
        }
    }
    // Group rows by stack (sub-plot order), larger values first within one;
    // `sort_by` is stable so ties keep legend order.
    values.sort_by(|a, b| {
        a.stack
            .cmp(&b.stack)
            .then(b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal))
    });

    HoverResult {
        x: target_x,
        pointer_y,
        anchor,
        values,
    }
}

/// Faint, theme-aware crosshair stroke: tint of the current text color so it
/// shows on both light and dark backgrounds.
fn crosshair_stroke(ui: &Ui) -> egui::Stroke {
    let base = ui.visuals().text_color();
    let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 70);
    egui::Stroke::new(1.0, color)
}

/// Dashed vertical crosshair line at screen `x` (drawn in every sub-plot to
/// mark the same data X across the stack).
pub fn render_crosshair_v(ui: &Ui, frame_rect: Rect, x: f32) {
    if x < frame_rect.left() || x > frame_rect.right() {
        return;
    }
    let painter = ui.painter().with_clip_rect(frame_rect);
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(x, frame_rect.top()),
            egui::pos2(x, frame_rect.bottom()),
        ],
        crosshair_stroke(ui),
        4.0,
        4.0,
    ));
}

/// Dashed horizontal crosshair line at screen `y` (hovered sub-plot only).
pub fn render_crosshair_h(ui: &Ui, frame_rect: Rect, y: f32) {
    if y < frame_rect.top() || y > frame_rect.bottom() {
        return;
    }
    let painter = ui.painter().with_clip_rect(frame_rect);
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(frame_rect.left(), y),
            egui::pos2(frame_rect.right(), y),
        ],
        crosshair_stroke(ui),
        4.0,
        4.0,
    ));
}

/// Render the tooltip: row 1 is the X (`t =`/`x =`), row 2 is the free cursor
/// `y =` (in the hovered stack's unit), and rows 3.. are each visible series
/// (`swatch + name = value (unit)`) at the hovered row, grouped by stack.
///
/// `frame_rect` is the hovered sub-plot (it anchors the tooltip); `area_rect`
/// is the region the box may occupy — the union of every sub-plot's frame, so
/// a tooltip near a stack boundary spills over its neighbour instead of being
/// clipped away.
pub fn render_tooltip(
    ui: &Ui,
    frame_rect: Rect,
    area_rect: Rect,
    hover: &HoverResult,
    x_unit: &str,
    x_proportion: f64,
    is_log_scale: bool,
    pointer_unit: &str,
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
    let value_row = |label: &str, y: f64, unit: &str| {
        let mut job = LayoutJob::default();
        job.append(label, 0.0, normal_fmt.clone());
        job.append(" = ", 0.0, normal_fmt.clone());
        job.append(&format_y_value(y, is_log_scale), 0.0, bold_fmt.clone());
        if !unit.is_empty() {
            job.append(&format!(" ({})", unit), 0.0, normal_fmt.clone());
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };

    // Row 2: pointer Y (snapped point's Y when snapped, else free cursor Y).
    rows.push((value_row("y", hover.pointer_y, pointer_unit), None));

    // Rows 3..: visible series at the hovered row.
    for v in &hover.values {
        rows.push((value_row(&v.name, v.y, &v.unit), Some(v.color)));
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

    // Position: upper-right of anchor, with edge avoidance against the whole
    // stack area (flip first, then clamp so the box never leaves it).
    let mut pos = egui::pos2(hover.anchor.x + 8.0, hover.anchor.y - 8.0 - content_h);
    if pos.x + content_w + 4.0 > area_rect.right() {
        pos.x = hover.anchor.x - 8.0 - content_w;
    }
    if pos.y - 4.0 < area_rect.top() {
        pos.y = hover.anchor.y + 8.0;
    }
    pos.x = pos.x.clamp(
        area_rect.left() + 4.0,
        (area_rect.right() - content_w - 4.0).max(area_rect.left() + 4.0),
    );
    pos.y = pos.y.clamp(
        area_rect.top() + 4.0,
        (area_rect.bottom() - content_h - 4.0).max(area_rect.top() + 4.0),
    );

    let content_rect = Rect::from_min_size(pos, egui::vec2(content_w, content_h));
    let painter = ui.painter().with_clip_rect(area_rect);
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

#[cfg(test)]
mod tests {
    use super::{extent_contains, nearest_row};

    fn pts(xs: &[f64]) -> Vec<(f64, Option<f64>)> {
        xs.iter().map(|&x| (x, Some(1.0))).collect()
    }

    #[test]
    fn extent_excludes_before_start_and_after_end() {
        let p = pts(&[10.0, 20.0, 30.0]);
        assert!(!extent_contains(&p, (0, 3), 9.9, true));   // before the first row
        assert!(extent_contains(&p, (0, 3), 10.0, true));   // boundary inclusive
        assert!(extent_contains(&p, (0, 3), 25.0, true));
        assert!(extent_contains(&p, (0, 3), 30.0, true));   // boundary inclusive
        assert!(!extent_contains(&p, (0, 3), 30.1, true));  // after the last row
    }

    #[test]
    fn extent_of_unsorted_x_uses_real_min_max() {
        // Rises then falls: first/last row are neither the min nor the max.
        let p = pts(&[30.0, 50.0, 20.0, 40.0]);
        assert!(extent_contains(&p, (0, 4), 20.0, false));
        assert!(extent_contains(&p, (0, 4), 50.0, false));
        assert!(!extent_contains(&p, (0, 4), 19.9, false));
        assert!(!extent_contains(&p, (0, 4), 50.1, false));
        // The sorted shortcut would have reported the reversed [30, 40] window
        assert!(!extent_contains(&p, (0, 4), 20.0, true));
    }

    #[test]
    fn small_values_keep_significant_digits() {
        use super::{format_series_value, format_y_value};
        // 1.00214e-4 must not collapse to "0.0001"
        assert_eq!(format_y_value(0.000100214, false), "0.00010021");
        assert_eq!(format_series_value(0.000100214, false), "0.00010021");
        // Ordinary magnitudes keep the existing formatting
        assert_eq!(format_series_value(297.6315, false), "297.6315");
        assert_eq!(format_y_value(298.29, false), "298.29");
        assert_eq!(format_series_value(0.5878, false), "0.5878");
        assert_eq!(format_series_value(0.0, false), "0");
        // Negative small values keep sign and digits
        assert_eq!(format_series_value(-0.000100214, false), "-0.00010021");
    }

    #[test]
    fn extent_follows_visible_range_not_full_data() {
        let p = pts(&[0.0, 10.0, 20.0, 30.0]);
        // Only rows [1..3) visible: extent is [10, 20]
        assert!(!extent_contains(&p, (1, 3), 5.0, true));
        assert!(extent_contains(&p, (1, 3), 15.0, true));
        assert!(!extent_contains(&p, (1, 3), 25.0, true));
        // Empty range → nothing plotted
        assert!(!extent_contains(&p, (2, 2), 20.0, true));
    }

    #[test]
    fn nearest_row_picks_closer_neighbor() {
        let p = pts(&[0.0, 10.0, 20.0]);
        assert_eq!(nearest_row(&p, (0, 3), 4.0, true), Some(0));
        assert_eq!(nearest_row(&p, (0, 3), 6.0, true), Some(1));
        assert_eq!(nearest_row(&p, (0, 3), 100.0, true), Some(2));
        assert_eq!(nearest_row(&p, (0, 3), -100.0, true), Some(0));
    }

    #[test]
    fn nearest_row_respects_range() {
        let p = pts(&[0.0, 10.0, 20.0, 30.0]);
        assert_eq!(nearest_row(&p, (2, 4), 0.0, true), Some(2));
        assert_eq!(nearest_row(&p, (2, 2), 0.0, true), None);
        assert_eq!(nearest_row(&p, (5, 9), 0.0, true), None); // clamped past end
    }

    #[test]
    fn nearest_row_scans_when_x_is_unsorted() {
        // Binary search would stop at the first ordering break; the scan finds
        // the genuinely closest row wherever it sits.
        let p = pts(&[30.0, 50.0, 20.0, 40.0]);
        assert_eq!(nearest_row(&p, (0, 4), 21.0, false), Some(2));
        assert_eq!(nearest_row(&p, (0, 4), 49.0, false), Some(1));
        assert_eq!(nearest_row(&p, (0, 4), 1000.0, false), Some(1)); // max x
        assert_eq!(nearest_row(&p, (0, 4), -1000.0, false), Some(2)); // min x
        // Range still bounds the scan
        assert_eq!(nearest_row(&p, (0, 2), 21.0, false), Some(0));
        assert_eq!(nearest_row(&p, (2, 2), 0.0, false), None);
    }
}

/// Decimal places for a small magnitude (< 0.001): enough significant digits
/// that values like 1.00214e-4 don't collapse to "0.0001".
fn small_value_decimals(abs: f64) -> usize {
    ((4.0 - abs.log10().floor()) as usize).min(10)
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
        } else if abs_val > 0.0 && abs_val < 0.001 {
            format!("{:.*}", small_value_decimals(abs_val), y)
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

/// Format a series' latest value for the legend: up to 6 decimals (trimmed),
/// scientific under log scale, extra decimals for tiny magnitudes.
pub fn format_series_value(v: f64, is_log_scale: bool) -> String {
    if is_log_scale {
        let s = format!("{:.6e}", v);
        if let Some(e_pos) = s.find('e') {
            let (mantissa, exp) = s.split_at(e_pos);
            let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
            format!("{}{}", trimmed, exp)
        } else {
            s
        }
    } else {
        let abs = v.abs();
        let s = if abs > 0.0 && abs < 0.001 {
            format!("{:.*}", small_value_decimals(abs), v)
        } else {
            format!("{:.6}", v)
        };
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    }
}
