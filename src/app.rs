use eframe::egui;
use egui::{PointerButton, Vec2b};
use egui_plot::{Line, MarkerShape, Plot, PlotBounds, PlotPoints, Points};

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
    x_unit: String,
    line_width: f32,
    colors: Vec<[u8; 3]>,
    max_points: usize,
    max_x_range: f64,
    marker_radius: f32,
    x_proportion: f64,
    series_visible: Vec<bool>,
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
        cc: &eframe::CreationContext<'_>,
        cli: Cli,
        window_title: String,
        target_placement: Option<[i32; 4]>,
    ) -> Self {
        crate::register_fonts(&cc.egui_ctx);

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
            x_axis_label: if cli.x_unit.is_empty() {
                cli.x_axis_label.clone()
            } else {
                format!("{} ({})", cli.x_axis_label, cli.x_unit)
            },
            x_unit: cli.x_unit,
            line_width: cli.line_width,
            colors,
            max_points: cli.max_points,
            max_x_range: cli.max_x_range,
            marker_radius: cli.marker_radius,
            x_proportion: cli.x_proportion,
            series_visible: vec![true; names.len()],
            pending_bounds: None,
            current_bounds: None,
            target_placement,
            window_title,
            placement_applied: false,
        }
    }
}

impl eframe::App for PlotApp {
    /// Convert scroll-wheel to zoom (inject Ctrl so egui treats scroll as zoom_delta)
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        use egui::Event;
        for event in &mut raw_input.events {
            if let Event::MouseWheel { modifiers, .. } = event {
                modifiers.ctrl = true;
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // First frame: Win32 SetWindowPos for precise physical coordinate placement
        if !self.placement_applied {
            self.placement_applied = true;
            if let Some([x, y, w, h]) = self.target_placement {
                crate::apply_window_placement(&self.window_title, x, y, w, h);
            }
        }

        self.watcher.poll(&mut self.data);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(self.repaint_interval_ms));

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.toolbar.ui(ui);
        });

        if self.toolbar.show_about {
            crate::render_about_modal(ui.ctx(), &mut self.toolbar.show_about);
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mode = self.toolbar.mode;
            let is_log_scale = self.toolbar.log_scale;
            let allow_drag = mode == PlotMode::None;
            let is_zoom_in = mode == PlotMode::ZoomIn;

            // --- Build Plot ---
            let mut plot = Plot::new("main_plot")
                .x_axis_label(&self.x_axis_label)
                .y_axis_label(&self.y_unit)
                .allow_drag(allow_drag)
                .allow_scroll(false)
                .allow_boxed_zoom(is_zoom_in)
                .boxed_zoom_pointer_button(PointerButton::Primary)
                .allow_zoom(true)
                .allow_axis_zoom_drag(false)
                .allow_double_click_reset(false)
                .show_x(false) // suppress built-in tooltip (renders empty roundbox in 0.34)
                .show_y(false)
                .y_axis_formatter(move |mark, range| {
                    let range_size = range.end() - range.start();
                    if is_log_scale {
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
                        if let Some(e_pos) = s.find('e') {
                            let (mantissa, exp) = s.split_at(e_pos);
                            let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
                            format!("{}{}", trimmed, exp)
                        } else { s }
                    } else {
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
                        if s.contains('.') {
                            s.trim_end_matches('0').trim_end_matches('.').to_string()
                        } else {
                            s
                        }
                    }
                });

            // --- Bounds management ---

            // Home: reset to full auto view
            if self.toolbar.need_home {
                self.toolbar.auto_x = true;
                self.toolbar.auto_y = true;
                self.pending_bounds = None;
                plot = plot.reset();
                self.toolbar.need_home = false;
            }

            // Auto toggle: just clear the flag.
            // set_auto_bounds() handles per-axis auto natively in 0.34.
            // No plot.reset() needed — reset destroys both axes and causes view jump.
            if self.toolbar.need_reset {
                self.toolbar.need_reset = false;
            }

            let render_range = self.data.get_render_range(self.max_points, self.max_x_range);
            let y_unit = self.y_unit.clone();
            let pending_bounds = self.pending_bounds.take();
            let auto_x = self.toolbar.auto_x;
            let auto_y = self.toolbar.auto_y;

            // --- Plot show ---
            let response = plot.show(ui, |plot_ui| {
                // Per-axis auto bounds (0.34 native support)
                plot_ui.set_auto_bounds(Vec2b::new(auto_x, auto_y));

                // Apply programmatic bounds (zoom-out, partial auto restore)
                if let Some((x_min, x_max, y_min, y_max)) = pending_bounds {
                    plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                        [x_min, y_min],
                        [x_max, y_max],
                    ));
                }

                for (i, series) in self.data.series.iter().enumerate() {
                    if !self.series_visible.get(i).copied().unwrap_or(true) {
                        continue;
                    }

                    let (start_idx, end_idx) =
                        render_range.unwrap_or((0, series.points.len()));
                    let points_slice = if start_idx < series.points.len() {
                        &series.points[start_idx..end_idx.min(series.points.len())]
                    } else {
                        &[]
                    };

                    // Latest value for legend display name
                    let latest_value = points_slice.iter().rev().find_map(|(_, y)| *y);
                    let legend_name = match latest_value {
                        Some(v) => {
                            let display_val = if is_log_scale {
                                let s = format!("{:.6e}", v);
                                if let Some(e_pos) = s.find('e') {
                                    let (mantissa, exp) = s.split_at(e_pos);
                                    let trimmed = mantissa
                                        .trim_end_matches('0')
                                        .trim_end_matches('.');
                                    format!("{}{}", trimmed, exp)
                                } else {
                                    s
                                }
                            } else {
                                let s = format!("{:.6}", v);
                                s.trim_end_matches('0').trim_end_matches('.').to_string()
                            };
                            format!("{} : {}({})", series.name, display_val, y_unit)
                        }
                        None => series.name.clone(),
                    };

                    // Build line segments, skipping None values (line breaks)
                    let mut segments: Vec<Vec<[f64; 2]>> = Vec::new();
                    let mut current_segment: Vec<[f64; 2]> = Vec::new();

                    for &(x, y_opt) in points_slice {
                        match y_opt {
                            Some(y) => {
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
                    let marker_color =
                        egui::Color32::from_rgba_unmultiplied(r, g, b, 204);
                    let marker_shape = MARKER_SHAPES[i % MARKER_SHAPES.len()];

                    for (seg_idx, seg) in segments.iter().enumerate() {
                        let line_name = if seg_idx == 0 {
                            legend_name.clone()
                        } else {
                            String::new()
                        };
                        let line = Line::new(line_name, PlotPoints::new(seg.clone()))
                            .color(line_color)
                            .width(self.line_width);
                        plot_ui.line(line);

                        let points =
                            Points::new(String::new(), PlotPoints::new(seg.clone()))
                                .color(marker_color)
                                .radius(self.marker_radius)
                                .shape(marker_shape);
                        plot_ui.points(points);
                    }
                }
            });

            // --- Interaction detection ---
            let plot_response = &response.response;
            let plot_hovered = plot_response.hovered();

            // Detect user scroll-zoom: raw_input_hook injects ctrl into MouseWheel
            // events, so the wheel delta is consumed as zoom. Check for the raw
            // MouseWheel event directly (egui 0.34 removed InputState::raw_scroll_delta).
            let wheel_scrolled = ui
                .ctx()
                .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
            let scrolled_on_plot = wheel_scrolled && plot_hovered;
            let user_interacted =
                plot_response.dragged() || plot_response.double_clicked() || scrolled_on_plot;

            // Detect box-zoom completion in ZoomIn mode
            let zoom_in_box_completed = is_zoom_in && plot_response.drag_stopped();

            // ZoomOut: click to zoom out 1.5x centered on cursor
            let zoom_out_clicked = mode == PlotMode::ZoomOut && plot_response.clicked();
            if zoom_out_clicked {
                if let Some(hover_pos) = plot_response.hover_pos() {
                    let plot_pos = response.transform.value_from_position(hover_pos);
                    let bounds = response.transform.bounds();
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

            // Disable auto on any user interaction (single place to avoid race conditions)
            if user_interacted || zoom_in_box_completed || zoom_out_clicked {
                if self.toolbar.auto_x || self.toolbar.auto_y {
                    self.toolbar.auto_x = false;
                    self.toolbar.auto_y = false;
                    // Save current bounds to prevent jump (unless ZoomOut already set pending)
                    if self.pending_bounds.is_none() {
                        self.pending_bounds = self.current_bounds;
                    }
                }
            }

            // Save current bounds each frame
            let bounds = response.transform.bounds();
            self.current_bounds = Some((
                bounds.min()[0],
                bounds.max()[0],
                bounds.min()[1],
                bounds.max()[1],
            ));

            // Cursor icon per mode
            if plot_hovered {
                match mode {
                    PlotMode::ZoomIn => {
                        ui.ctx()
                            .output_mut(|o| o.cursor_icon = egui::CursorIcon::ZoomIn);
                    }
                    PlotMode::ZoomOut => {
                        ui.ctx()
                            .output_mut(|o| o.cursor_icon = egui::CursorIcon::ZoomOut);
                    }
                    PlotMode::None => {}
                }
            }

            let frame_rect = *response.transform.frame();

            // --- Custom tooltip (manual hover detection, no built-in tooltip) ---
            if let Some(hover_pos) = plot_response.hover_pos() {
                if frame_rect.contains(hover_pos) {
                    let hover = crate::tooltip::find_hover(
                        hover_pos,
                        &response.transform,
                        &self.data.series,
                        &self.series_visible,
                        &self.colors,
                        render_range,
                        is_log_scale,
                        400.0, // 20px squared
                    );
                    crate::tooltip::render_crosshair(ui, frame_rect, hover.anchor);
                    crate::tooltip::render_tooltip(
                        ui,
                        frame_rect,
                        &hover,
                        &self.x_unit,
                        self.x_proportion,
                        is_log_scale,
                        &self.y_unit,
                    );
                }
            }

            // --- Custom legend ---
            crate::custom_legend::render_legend(
                ui.ctx(),
                frame_rect,
                self.toolbar.legend_pos,
                &self.data.series,
                &mut self.series_visible,
                &self.colors,
                render_range,
                &self.y_unit,
                is_log_scale,
            );
        });
    }
}
