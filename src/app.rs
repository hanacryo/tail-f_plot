use eframe::egui;
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotBounds, PlotPoints, Points};

use crate::csv_watcher::CsvWatcher;
use crate::plot_state::{PlotData, PlotMode};
use crate::toolbar::ToolbarState;
use crate::{Cli, resolve_colors, resolve_delimiter, resolve_string_quote};

/// Cycle marker shapes per series index
const MARKER_SHAPES: [MarkerShape; 5] = [
    MarkerShape::Circle,
    MarkerShape::Square,
    MarkerShape::Diamond,
    MarkerShape::Cross,
    MarkerShape::Plus,
];

pub struct PlotApp {
    watcher: CsvWatcher,
    data: PlotData,
    toolbar: ToolbarState,
    y_unit: String,
    repaint_interval_ms: u64,
    x_axis_label: String,
    line_width: f32,
    colors: Vec<[u8; 3]>,
    max_points: usize,
    max_x_range: f64,
    marker_radius: f32,
    /// Pending bounds for zoom-out (x_min, x_max, y_min, y_max)
    pending_bounds: Option<(f64, f64, f64, f64)>,
    /// Currently displayed bounds (saved for drag restore)
    current_bounds: Option<(f64, f64, f64, f64)>,
    /// Physical pixel placement for SetWindowPos: [x, y, width, height]
    target_placement: Option<[i32; 4]>,
    window_title: String,
    placement_applied: bool,
}

impl PlotApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        cli: Cli,
        window_title: String,
        target_placement: Option<[i32; 4]>,
    ) -> Self {
        let names: Vec<String> = if cli.y_names.is_empty() {
            cli.y_cols.iter().map(|c| format!("Col{}", c)).collect()
        } else {
            cli.y_names.clone()
        };

        let delimiter = resolve_delimiter(&cli.delimiter);
        let string_quote = resolve_string_quote(&cli.string_quote);
        let colors = resolve_colors(&cli.colors);

        let mut watcher = CsvWatcher::new(
            cli.csv_path.unwrap(),
            cli.x_col,
            &cli.y_cols,
            cli.header_row,
            cli.data_start_row,
            delimiter,
            cli.fixed_width,
            string_quote,
            cli.merge_delimiter,
            cli.x_proportion,
        );
        watcher.start();

        let mut toolbar = ToolbarState::default();
        toolbar.set_log_scale(cli.log_y);

        Self {
            watcher,
            data: PlotData::new(&names, cli.absolute_x),
            toolbar,
            y_unit: cli.y_unit,
            repaint_interval_ms: cli.repaint_interval_ms,
            x_axis_label: cli.x_axis_label,
            line_width: cli.line_width,
            colors,
            max_points: cli.max_points,
            max_x_range: cli.max_x_range,
            marker_radius: cli.marker_radius,
            pending_bounds: None,
            current_bounds: None,
            target_placement,
            window_title,
            placement_applied: false,
        }
    }
}

impl eframe::App for PlotApp {
    /// Convert scroll-wheel to zoom (zoom works without Ctrl)
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        use egui::Event;
        for event in &mut raw_input.events {
            if let Event::MouseWheel { modifiers, .. } = event {
                // Add ctrl modifier to scroll events so egui treats them as zoom
                modifiers.ctrl = true;
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // First frame: use Win32 SetWindowPos for precise physical coordinate placement
        // (eframe's with_position has inaccurate logical coordinate conversion on multi-monitor)
        if !self.placement_applied {
            self.placement_applied = true;
            if let Some([x, y, w, h]) = self.target_placement {
                crate::apply_window_placement(&self.window_title, x, y, w, h);
            }
        }

        // Poll CSV for new data
        self.watcher.poll(&mut self.data);

        // Request repaint periodically for tail-f behavior
        ctx.request_repaint_after(std::time::Duration::from_millis(self.repaint_interval_ms));

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            self.toolbar.ui(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mode = self.toolbar.mode;

            // Capture current log scale state (used inside closure)
            let is_log_scale = self.toolbar.log_scale;

            // Drag panning only in default mode (disabled in ZoomIn/ZoomOut)
            let allow_drag = mode == PlotMode::None;

            // Auto-off on drag/scroll is handled after plot.show()
            // (doing it here causes race conditions)

            let legend_pos = self.toolbar.legend_pos;
            let mut plot = Plot::new("main_plot")
                .legend(Legend::default().position(legend_pos))
                .x_axis_label(&self.x_axis_label)
                .y_axis_label(&self.y_unit)
                .allow_drag(allow_drag)
                .allow_scroll(false)
                .allow_boxed_zoom(false) // box zoom disabled by default
                .allow_zoom(true) // scroll-to-zoom via raw_input_hook
                .y_axis_formatter(move |mark, range| {
                    let range_size = range.end() - range.start();
                    if is_log_scale {
                        // Log scale: range_size is order of magnitude difference
                        let real_value = 10_f64.powf(mark.value);
                        let precision = if range_size >= 2.0 { 0 }
                            else if range_size >= 1.0 { 1 }
                            else if range_size >= 0.3 { 2 }
                            else { 3 };
                        let s = match precision {
                            0 => format!("{:.0e}", real_value),
                            1 => format!("{:.1e}", real_value),
                            2 => format!("{:.2e}", real_value),
                            _ => format!("{:.3e}", real_value),
                        };
                        // Strip trailing zeros
                        if let Some(e_pos) = s.find('e') {
                            let (mantissa, exp) = s.split_at(e_pos);
                            let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
                            format!("{}{}", trimmed, exp)
                        } else { s }
                    } else {
                        // Linear scale: decimal places based on range size
                        let precision = if range_size >= 100.0 { 0 }
                            else if range_size >= 10.0 { 1 }
                            else if range_size >= 1.0 { 2 }
                            else if range_size >= 0.1 { 3 }
                            else { 4 };
                        let s = match precision {
                            0 => format!("{:.0}", mark.value),
                            1 => format!("{:.1}", mark.value),
                            2 => format!("{:.2}", mark.value),
                            3 => format!("{:.3}", mark.value),
                            _ => format!("{:.4}", mark.value),
                        };
                        // Strip trailing zeros only when decimal point exists (prevent 100 -> 1)
                        if s.contains('.') {
                            s.trim_end_matches('0').trim_end_matches('.').to_string()
                        } else {
                            s
                        }
                    }
                });

            // Box zoom only in ZoomIn mode with Primary button
            let is_zoom_in_mode = mode == PlotMode::ZoomIn;
            if is_zoom_in_mode {
                plot = plot
                    .allow_boxed_zoom(true)
                    .boxed_zoom_pointer_button(egui::PointerButton::Primary);
            }

            // Home: reset to full view + enable auto bounds
            if self.toolbar.need_home {
                self.toolbar.auto_x = true;
                self.toolbar.auto_y = true;
                self.toolbar.need_reset = true; // Home also triggers reset
                self.toolbar.need_home = false;
            }

            // Auto bounds handling
            let is_partial_auto = self.toolbar.auto_x != self.toolbar.auto_y;

            // Auto toggle needs reset (save bounds before reset, restore after)
            if self.toolbar.need_reset {
                let saved_bounds = self.current_bounds;
                plot = plot.reset();
                self.toolbar.need_reset = false;

                // Partial auto: restore bounds for the disabled axis
                if is_partial_auto {
                    if let (Some((cx_min, cx_max, cy_min, cy_max)), Some((dx_min, dx_max, dy_min, dy_max))) =
                        (saved_bounds, self.data.get_bounds())
                    {
                        let new_bounds = if self.toolbar.auto_x {
                            // X auto only: X from data, Y from previous
                            (dx_min, dx_max, cy_min, cy_max)
                        } else {
                            // Y auto only: Y from data, X from previous
                            (cx_min, cx_max, dy_min, dy_max)
                        };
                        self.pending_bounds = Some(new_bounds);
                    }
                }
            }

            // Partial auto: recompute bounds each frame (update auto axis on new data)
            if is_partial_auto && self.pending_bounds.is_none() {
                if let Some((dx_min, dx_max, dy_min, dy_max)) = self.data.get_bounds() {
                    let (cx_min, cx_max, cy_min, cy_max) =
                        self.current_bounds.unwrap_or((dx_min, dx_max, dy_min, dy_max));

                    let new_bounds = if self.toolbar.auto_x {
                        (dx_min, dx_max, cy_min, cy_max)
                    } else {
                        (cx_min, cx_max, dy_min, dy_max)
                    };
                    self.pending_bounds = Some(new_bounds);
                }
            }

            // Use explicit bounds if pending, otherwise use auto_bounds
            if self.pending_bounds.is_some() {
                plot = plot.auto_bounds(egui::Vec2b::FALSE);
            } else {
                plot = plot.auto_bounds(egui::Vec2b::new(
                    self.toolbar.auto_x,
                    self.toolbar.auto_y
                ));
            }

            // Compute render range
            let render_range = self.data.get_render_range(
                self.max_points,
                self.max_x_range,
            );

            // Capture y_unit for legend closure
            let y_unit = self.y_unit.clone();

            // Take pending_bounds for use inside closure
            let pending_bounds = self.pending_bounds.take();

            let response = plot.show(ui, |plot_ui| {
                // Apply pending_bounds via PlotUi::set_plot_bounds
                if let Some((x_min, x_max, y_min, y_max)) = pending_bounds {
                    plot_ui.set_plot_bounds(PlotBounds::from_min_max([x_min, y_min], [x_max, y_max]));
                }

                for (i, series) in self.data.series.iter().enumerate() {
                    // Apply render range
                    let (start_idx, end_idx) = render_range.unwrap_or((0, series.points.len()));
                    let points_slice = if start_idx < series.points.len() {
                        &series.points[start_idx..end_idx.min(series.points.len())]
                    } else {
                        &[]
                    };

                    // Get latest value in render range
                    let latest_value = points_slice.iter().rev()
                        .find_map(|(_, y)| *y);

                    // Legend: "name : value(unit)"
                    let legend_name = match latest_value {
                        Some(v) => {
                            let display_val = if is_log_scale {
                                // Strip trailing zeros: 6.000000e-01 -> "6e-01"
                                let s = format!("{:.6e}", v);
                                if let Some(e_pos) = s.find('e') {
                                    let (mantissa, exp) = s.split_at(e_pos);
                                    let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
                                    format!("{}{}", trimmed, exp)
                                } else {
                                    s
                                }
                            } else {
                                // Strip trailing zeros
                                let s = format!("{:.6}", v);
                                s.trim_end_matches('0').trim_end_matches('.').to_string()
                            };
                            format!("{} : {}({})", series.name, display_val, y_unit)
                        }
                        None => series.name.clone(),
                    };

                    // Build line segments, skipping None values (line breaks)
                    // Log scale: convert y to log10
                    let mut segments: Vec<Vec<[f64; 2]>> = Vec::new();
                    let mut current_segment: Vec<[f64; 2]> = Vec::new();

                    for &(x, y_opt) in points_slice {
                        match y_opt {
                            Some(y) => {
                                // Log scale: only show y > 0, apply log10
                                let y_render = if is_log_scale {
                                    if y > 0.0 { y.log10() } else { continue; }
                                } else {
                                    y
                                };
                                current_segment.push([x, y_render]);
                            }
                            None => {
                                if !current_segment.is_empty() {
                                    segments.push(std::mem::take(&mut current_segment));
                                }
                            }
                        }
                    }
                    if !current_segment.is_empty() {
                        segments.push(current_segment);
                    }

                    let color_idx = i % self.colors.len();
                    let [r, g, b] = self.colors[color_idx];
                    let line_color = egui::Color32::from_rgb(r, g, b);
                    // 80% opacity (204 = 255 * 0.8)
                    let marker_color = egui::Color32::from_rgba_unmultiplied(r, g, b, 204);
                    let marker_shape = MARKER_SHAPES[i % MARKER_SHAPES.len()];

                    // Draw each continuous segment
                    for (seg_idx, seg) in segments.iter().enumerate() {
                        // Line rendering
                        let line_points = PlotPoints::new(seg.clone());
                        let mut line = Line::new(line_points)
                            .color(line_color)
                            .width(self.line_width);
                        if seg_idx == 0 {
                            line = line.name(&legend_name);
                        }
                        plot_ui.line(line);

                        // Marker rendering
                        let marker_points = PlotPoints::new(seg.clone());
                        let points = Points::new(marker_points)
                            .color(marker_color)
                            .radius(self.marker_radius)
                            .shape(marker_shape);
                        plot_ui.points(points);
                    }
                }
            });

            // Save current bounds (for restore on drag)
            let bounds = response.transform.bounds();
            self.current_bounds = Some((
                bounds.min()[0], bounds.max()[0],
                bounds.min()[1], bounds.max()[1],
            ));

            let plot_response = &response.response;

            // Detect user interaction -> disable auto + set pending_bounds
            // Scroll-wheel zoom has ctrl modifier from raw_input_hook -> detect via raw_scroll_delta
            let scroll_delta = ctx.input(|i| i.raw_scroll_delta);
            let scrolled_on_plot = scroll_delta != egui::Vec2::ZERO && plot_response.hovered();
            let user_interacted = plot_response.dragged() || plot_response.double_clicked() || scrolled_on_plot;

            // Detect box-zoom completion in ZoomIn mode (via drag_released)
            let zoom_in_box_completed = is_zoom_in_mode && plot_response.drag_stopped();

            // ZoomOut mode: click to zoom out centered on cursor
            let zoom_out_clicked = mode == PlotMode::ZoomOut && plot_response.clicked();
            if zoom_out_clicked {
                if let Some(hover_pos) = plot_response.hover_pos() {
                    // Convert screen coords to plot coords
                    let plot_pos = response.transform.value_from_position(hover_pos);
                    let bounds = response.transform.bounds();
                    // Zoom out to 1.5x current view centered on click
                    let half_width = (bounds.max()[0] - bounds.min()[0]) / 2.0 * 1.5;
                    let half_height = (bounds.max()[1] - bounds.min()[1]) / 2.0 * 1.5;
                    self.pending_bounds = Some((
                        plot_pos.x - half_width,
                        plot_pos.x + half_width,
                        plot_pos.y - half_height,
                        plot_pos.y + half_height,
                    ));
                }
            }

            // Disable auto: single place to avoid race conditions
            if user_interacted || zoom_in_box_completed || zoom_out_clicked {
                if self.toolbar.auto_x || self.toolbar.auto_y {
                    self.toolbar.auto_x = false;
                    self.toolbar.auto_y = false;
                    // Save current bounds for next frame (prevent bounds jump)
                    // Skip if ZoomOut click already set pending_bounds
                    if self.pending_bounds.is_none() {
                        self.pending_bounds = self.current_bounds;
                    }
                }
            }

            // Set cursor icon based on mode
            if plot_response.hovered() {
                match mode {
                    PlotMode::ZoomIn => {
                        ctx.set_cursor_icon(egui::CursorIcon::ZoomIn);
                    }
                    PlotMode::ZoomOut => {
                        ctx.set_cursor_icon(egui::CursorIcon::ZoomOut);
                    }
                    PlotMode::None => {}
                }
            }
        });
    }
}
