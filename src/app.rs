use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{PointerButton, Vec2b};
use egui_plot::{Line, MarkerShape, Plot, PlotPoint, PlotPoints, PlotResponse, Points};

use crate::csv_watcher::CsvWatcher;
use crate::custom_legend::LegendEntry;
use crate::groups::GroupConfig;
use crate::plot_state::{GroupView, PlotData, PlotMode};
use crate::toolbar::ToolbarState;
use crate::{Cli, resolve_colors};

/// Cycle marker shapes per series index
const MARKER_SHAPES: [MarkerShape; 5] = [
    MarkerShape::Circle,
    MarkerShape::Square,
    MarkerShape::Diamond,
    MarkerShape::Cross,
    MarkerShape::Plus,
];

/// id_salt of the linked-x-axis group shared by all stacked sub-plots
const X_LINK_GROUP: &str = "tailf_x_link";

/// Minimum y-axis width so stacked sub-plots align horizontally
const Y_AXIS_MIN_WIDTH: f32 = 52.0;

/// Auto-fit margin fraction (mimics egui_plot's native auto bounds feel)
const AUTO_MARGIN: f64 = 0.05;

/// `--oneshot`: capture anyway once this long has passed without the view
/// settling (e.g. a file that never appears, an offset that never resolves).
pub const ONESHOT_TIMEOUT: Duration = Duration::from_secs(15);
/// `--oneshot` frame pacing while waiting to settle. Must exceed egui's
/// predicted frame time (1/60 s): shorter delays are rounded down to an
/// immediate repaint, which the settle check reads as "still changing".
const ONESHOT_FRAME_DELAY: Duration = Duration::from_millis(50);
/// `--oneshot`: consecutive settled frames required before capturing.
const ONESHOT_STABLE_FRAMES: u32 = 2;
/// `--oneshot`: minimum frames before a capture (fonts, legend sizing pass).
const ONESHOT_MIN_FRAMES: u32 = 3;

/// State of a `--oneshot` run: wait for the view to settle, capture one
/// screenshot, write it as PNG, close.
struct OneShot {
    path: PathBuf,
    started: Instant,
    frames: u32,
    /// Consecutive frames that met every settle condition.
    stable: u32,
    /// Screenshot requested; data polling is frozen from here on so the
    /// captured frame shows the state that was judged settled.
    requested: bool,
    timed_out: bool,
    done: bool,
}

/// Write a screenshot as an opaque RGBA PNG.
fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for c in &image.pixels {
        rgba.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
    }
    image::save_buffer(path, &rgba, w as u32, h as u32, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())
}

/// Expand a min/max extent by a margin fraction (fixed margin when degenerate).
/// A reversed pair is normalized first — feeding `min > max` into
/// `PlotBounds` collapses the axis to a sliver around the midpoint.
fn with_margin((lo, hi): (f64, f64), frac: f64) -> (f64, f64) {
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let m = (hi - lo) * frac;
    let m = if m > 0.0 { m } else { 1.0 };
    (lo - m, hi + m)
}

/// Union of display-x extents over all renderable groups' visible slices.
/// Auto-X must fit ALL stacks' data: per-plot native auto would fit each
/// sub-plot to its own data only and fight the linked x axis.
fn global_x_extent(data: &PlotData, views: &[Option<GroupView>]) -> Option<(f64, f64)> {
    let mut ext: Option<(f64, f64)> = None;
    for (group, view) in data.groups.iter().zip(views) {
        let Some(v) = view else { continue };
        // Real min/max, not first/last row: an X column that is not a
        // timestamp (e.g. a temperature ramping up and down) is unsorted.
        let Some((lo, hi)) = group.x_extent(v.range) else { continue };
        let (lo, hi) = (lo + v.offset, hi + v.offset);
        ext = Some(match ext {
            None => (lo, hi),
            Some((a, b)) => (a.min(lo), b.max(hi)),
        });
    }
    ext
}

/// Y extent (in render space: log10 under log scale) of the visible series
/// belonging to `stack_id`, over their visible slices.
fn stack_y_extent(
    data: &PlotData,
    views: &[Option<GroupView>],
    series_stacks: &[usize],
    series_visible: &[bool],
    stack_id: usize,
    is_log_scale: bool,
) -> Option<(f64, f64)> {
    let mut ext: Option<(f64, f64)> = None;
    let mut flat_idx = 0usize;
    for (gi, group) in data.groups.iter().enumerate() {
        let view = views.get(gi).copied().flatten();
        for series in &group.series {
            let i = flat_idx;
            flat_idx += 1;
            if series_stacks.get(i).copied().unwrap_or(1) != stack_id {
                continue;
            }
            if !series_visible.get(i).copied().unwrap_or(true) {
                continue;
            }
            let Some(v) = view else { continue };
            let start = v.range.0.min(series.points.len());
            let end = v.range.1.min(series.points.len());
            for &(_, y_opt) in &series.points[start..end] {
                let Some(y) = y_opt else { continue };
                let y_render = if is_log_scale {
                    if y > 0.0 { y.log10() } else { continue; }
                } else {
                    y
                };
                ext = Some(match ext {
                    None => (y_render, y_render),
                    Some((a, b)) => (a.min(y_render), b.max(y_render)),
                });
            }
        }
    }
    ext
}

/// Deduplicate series names across all groups (flat order): the first
/// occurrence keeps its name, later ones get "(2)", "(3)", ... suffixes.
/// Covers inherited --y-names, explicit duplicates, and the same file
/// appearing in several groups.
fn dedupe_series_names(groups: &mut [Vec<String>]) {
    use std::collections::{HashMap, HashSet};
    let mut used: HashSet<String> = HashSet::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for names in groups.iter_mut() {
        for name in names.iter_mut() {
            let count = counts.entry(name.clone()).or_insert(0);
            *count += 1;
            let mut candidate = if *count == 1 {
                name.clone()
            } else {
                format!("{}({})", name, count)
            };
            // A suffixed name may itself collide with an explicit name
            // (e.g. "A", "A", "A(2)") — keep bumping until unique.
            while !used.insert(candidate.clone()) {
                *count += 1;
                candidate = format!("{}({})", name, count);
            }
            *name = candidate;
        }
    }
}

pub struct PlotApp {
    watchers: Vec<CsvWatcher>,
    data: PlotData,
    toolbar: ToolbarState,
    repaint_interval_ms: u64,
    x_axis_label: String,
    x_unit: String,
    line_width: f32,
    colors: Vec<[u8; 3]>,
    max_points: usize,
    max_x_range: f64,
    marker_radius: f32,
    /// First group's resolved proportion (used for tooltip time formatting)
    x_proportion: f64,
    /// Used stack indices, sorted ascending — one sub-plot per entry
    stacks: Vec<usize>,
    /// Y unit per entry of `stacks`
    stack_units: Vec<String>,
    /// Flat across all groups, in (group, series) order
    series_stacks: Vec<usize>,
    series_units: Vec<String>,
    series_visible: Vec<bool>,
    /// Shared y-axis width (min_width fed to every sub-plot). Ratchets up to
    /// the widest measured axis so all frames align pixel-exact; resets on Home.
    y_axis_width: f32,
    /// Per stack row: bounds to force on the next frame, per axis. Kept split
    /// because a gesture usually pins only one axis of one sub-plot — writing
    /// X on the others would fight the linked x axis.
    pending_x: Vec<Option<(f64, f64)>>,
    pending_y: Vec<Option<(f64, f64)>>,
    /// Per stack row: currently displayed bounds (x_min, x_max, y_min, y_max)
    current_bounds: Vec<Option<(f64, f64, f64, f64)>>,
    /// Physical pixel placement for SetWindowPos: [x, y, width, height]
    target_placement: Option<[i32; 4]>,
    /// This instance's own root window (HWND as isize)
    root_hwnd: Option<isize>,
    placement_applied: bool,
    oneshot: Option<OneShot>,
}

impl PlotApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cli: Cli,
        groups: Vec<GroupConfig>,
        target_placement: Option<[i32; 4]>,
    ) -> Self {
        crate::register_fonts(&cc.egui_ctx);

        let colors = resolve_colors(&cli.colors);
        let multi_file = groups.len() > 1;

        let mut group_names: Vec<Vec<String>> = groups
            .iter()
            .map(|cfg| {
                if cfg.y_names.is_empty() {
                    // Default names get a file-stem prefix in multi-file mode so
                    // identical column numbers from different files stay distinct.
                    let stem = cfg
                        .csv_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    cfg.y_cols
                        .iter()
                        .map(|c| {
                            if multi_file {
                                format!("{}:Col{}", stem, c)
                            } else {
                                format!("Col{}", c)
                            }
                        })
                        .collect()
                } else {
                    cfg.y_names.clone()
                }
            })
            .collect();
        dedupe_series_names(&mut group_names);

        // Flat per-series stack index ((group, series) order, 1-based)
        let mut series_stacks: Vec<usize> = Vec::new();
        for (cfg, names) in groups.iter().zip(&group_names) {
            for i in 0..names.len() {
                series_stacks.push(cfg.y_stacks.get(i).copied().unwrap_or(1).max(1));
            }
        }
        let mut stacks: Vec<usize> = series_stacks.clone();
        stacks.sort_unstable();
        stacks.dedup();
        if stacks.is_empty() {
            stacks.push(1);
        }
        let stack_unit = |s: usize| -> String {
            cli.y_units
                .get(s - 1)
                .cloned()
                .unwrap_or_else(|| cli.y_unit.clone())
        };
        let stack_units: Vec<String> = stacks.iter().map(|&s| stack_unit(s)).collect();
        let series_units: Vec<String> = series_stacks.iter().map(|&s| stack_unit(s)).collect();

        let mut data = PlotData::new(cli.absolute_x);
        let mut watchers = Vec::new();
        for (cfg, names) in groups.iter().zip(&group_names) {
            data.add_group(names, cfg.epochtime_col > 0);

            let mut watcher = CsvWatcher::new(cfg);
            watcher.start();
            watchers.push(watcher);
        }
        let series_total = data.series_count();

        let mut toolbar = ToolbarState::default();
        toolbar.set_log_scale(cli.log_y);

        let n_stacks = stacks.len();
        Self {
            watchers,
            data,
            toolbar,
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
            x_proportion: groups[0].x_proportion,
            stacks,
            stack_units,
            series_stacks,
            series_units,
            series_visible: vec![true; series_total],
            y_axis_width: Y_AXIS_MIN_WIDTH,
            pending_x: vec![None; n_stacks],
            pending_y: vec![None; n_stacks],
            current_bounds: vec![None; n_stacks],
            target_placement,
            root_hwnd: crate::native_hwnd(cc),
            placement_applied: false,
            oneshot: cli.oneshot.map(|path| OneShot {
                path,
                started: Instant::now(),
                frames: 0,
                stable: 0,
                requested: false,
                timed_out: false,
                done: false,
            }),
        }
    }
}

impl eframe::App for PlotApp {
    /// Convert scroll-wheel to zoom (inject Ctrl so egui treats scroll as zoom_delta)
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        use egui::Event;
        // One-shot: a window that opens under the cursor would otherwise
        // capture a hover crosshair/tooltip, making the result arbitrary.
        if self.oneshot.is_some() {
            raw_input.events.retain(|e| {
                !matches!(
                    e,
                    Event::PointerMoved(_)
                        | Event::MouseMoved(_)
                        | Event::PointerButton { .. }
                        | Event::PointerGone
                        | Event::MouseWheel { .. }
                        | Event::Touch { .. }
                        | Event::Zoom(_)
                        | Event::Rotate(_)
                )
            });
        }
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
            if let (Some(hwnd), Some([x, y, w, h])) = (self.root_hwnd, self.target_placement) {
                crate::apply_window_placement(hwnd, x, y, w, h);
            }
        }

        self.oneshot_receive(ui.ctx());

        // A requested one-shot capture freezes the data it was judged on.
        let frozen = self.oneshot.as_ref().is_some_and(|o| o.requested);
        if !frozen {
            for (i, watcher) in self.watchers.iter_mut().enumerate() {
                watcher.poll(&mut self.data, i);
            }
        }
        // One-shot paces its own frames (see oneshot_tick)
        if self.oneshot.is_none() {
            ui.ctx()
                .request_repaint_after(Duration::from_millis(self.repaint_interval_ms));
        }

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.toolbar.ui(ui);
        });

        if self.toolbar.show_about {
            crate::render_about_modal(ui.ctx(), &mut self.toolbar.show_about);
        }

        let mut all_resolved = false;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mode = self.toolbar.mode;
            let is_log_scale = self.toolbar.log_scale;
            let allow_drag = mode == PlotMode::None;
            let is_zoom_in = mode == PlotMode::ZoomIn;

            // Home: reset all sub-plots to full auto view
            let do_reset = self.toolbar.need_home;
            if do_reset {
                self.toolbar.auto_x = true;
                self.toolbar.auto_y = true;
                for p in self.pending_x.iter_mut() {
                    *p = None;
                }
                for p in self.pending_y.iter_mut() {
                    *p = None;
                }
                // Re-fit the shared axis width from scratch (ranges change on Home)
                self.y_axis_width = Y_AXIS_MIN_WIDTH;
                self.toolbar.need_home = false;
            }

            let views = self.data.views(self.max_points, self.max_x_range);
            all_resolved = views.iter().all(Option::is_some);
            let auto_x = self.toolbar.auto_x;
            let auto_y = self.toolbar.auto_y;

            // Manual auto-fit extents (see global_x_extent for why not native)
            let global_x = global_x_extent(&self.data, &views);
            let stack_y: Vec<Option<(f64, f64)>> = self
                .stacks
                .iter()
                .map(|&s| {
                    stack_y_extent(
                        &self.data,
                        &views,
                        &self.series_stacks,
                        &self.series_visible,
                        s,
                        is_log_scale,
                    )
                })
                .collect();

            let n = self.stacks.len();
            let spacing_y = ui.spacing().item_spacing.y;
            let plot_h =
                ((ui.available_height() - spacing_y * (n as f32 - 1.0)) / n as f32).max(50.0);

            // --- Render every stacked sub-plot, collecting responses ---
            let mut shows: Vec<PlotResponse<()>> = Vec::with_capacity(n);
            for (r, &stack_id) in self.stacks.iter().enumerate() {
                let is_bottom = r == n - 1;
                let unit = self.stack_units[r].clone();
                // A blank (not empty) label keeps the unit strip present on
                // every sub-plot so their axis offsets stay comparable.
                let axis_label = if n > 1 && unit.is_empty() {
                    " ".to_string()
                } else {
                    unit.clone()
                };

                let mut plot = Plot::new(("main_plot", stack_id))
                    .height(plot_h)
                    .y_axis_label(axis_label)
                    .y_axis_min_width(self.y_axis_width)
                    .link_axis(X_LINK_GROUP, Vec2b::new(true, false))
                    .show_axes(Vec2b::new(is_bottom, true))
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
                            // Dynamic decimals from the visible range's
                            // magnitude (tiny ranges like 1e-4 A need more
                            // than a fixed 4-decimal cap).
                            let precision = if range_size > 0.0 {
                                (2.0 - range_size.log10().floor()).clamp(0.0, 10.0) as usize
                            } else {
                                2
                            };
                            let s = format!("{:.*}", precision, mark.value);
                            if s.contains('.') {
                                s.trim_end_matches('0').trim_end_matches('.').to_string()
                            } else {
                                s
                            }
                        }
                    });

                if is_bottom {
                    plot = plot.x_axis_label(&self.x_axis_label);
                }
                if do_reset {
                    plot = plot.reset();
                }

                let pending_x = self.pending_x[r].take();
                let pending_y = self.pending_y[r].take();
                let y_fit = stack_y[r];

                let response = plot.show(ui, |plot_ui| {
                    // egui_plot's own auto-bounds are never used: auto-fit is
                    // done manually below. Its memory starts out (and returns
                    // after Home's reset) with native auto on, which would
                    // otherwise keep fitting per sub-plot whenever no bound
                    // is set below — e.g. Auto turned off before any data.
                    // With it off, an axis with nothing to fit keeps its last
                    // range.
                    plot_ui.set_auto_bounds(Vec2b::FALSE);

                    // Each axis is driven independently: an axis that is
                    // neither auto-fitted nor explicitly pinned this frame is
                    // left to the plot's own memory, so a gesture applied to
                    // it (zoom / pan / the linked x axis) survives.
                    //
                    // Manual auto-fit for X: it spans the union of all stacks'
                    // data (native per-plot auto would fit each sub-plot to
                    // its own data and fight the linked x axis).
                    if auto_x {
                        if let Some(e) = global_x {
                            let (x0, x1) = with_margin(e, AUTO_MARGIN);
                            plot_ui.set_plot_bounds_x(x0..=x1);
                        }
                    } else if let Some((x0, x1)) = pending_x {
                        plot_ui.set_plot_bounds_x(x0..=x1);
                    }
                    if auto_y {
                        if let Some(e) = y_fit {
                            let (y0, y1) = with_margin(e, AUTO_MARGIN);
                            plot_ui.set_plot_bounds_y(y0..=y1);
                        }
                    } else if let Some((y0, y1)) = pending_y {
                        plot_ui.set_plot_bounds_y(y0..=y1);
                    }

                    let mut flat_idx = 0usize;
                    for (gi, group) in self.data.groups.iter().enumerate() {
                        let view = views.get(gi).copied().flatten();
                        for series in &group.series {
                            let i = flat_idx;
                            flat_idx += 1;
                            if self.series_stacks.get(i).copied().unwrap_or(1) != stack_id {
                                continue;
                            }
                            if !self.series_visible.get(i).copied().unwrap_or(true) {
                                continue;
                            }
                            // Offset unresolved yet (e.g. waiting for the first
                            // file's origin or an epoch anchor): buffer silently.
                            let Some(view) = view else { continue };

                            let (start_idx, end_idx) = view.range;
                            let points_slice = if start_idx < series.points.len() {
                                &series.points[start_idx..end_idx.min(series.points.len())]
                            } else {
                                &[]
                            };

                            // Latest value for legend display name
                            let latest_value = points_slice.iter().rev().find_map(|(_, y)| *y);
                            let legend_name = match latest_value {
                                Some(v) => format!(
                                    "{} : {}({})",
                                    series.name,
                                    crate::tooltip::format_series_value(v, is_log_scale),
                                    unit
                                ),
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
                                        current_segment.push([x + view.offset, y_render]);
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
                    }
                });

                shows.push(response);
            }

            // Pixel-align sub-plot frames: the shared y-axis strip must match
            // the widest one (actual width = max(min_width, label width), so a
            // wider-labeled stack sticks out). Every sub-plot widget starts at
            // the same left x, so the frames' left edges must be identical —
            // feed the spread back into the shared min_width until they are.
            if n > 1 {
                let lefts: Vec<f32> = shows
                    .iter()
                    .map(|s| s.transform.frame().left())
                    .collect();
                let max_l = lefts.iter().copied().fold(f32::MIN, f32::max);
                let min_l = lefts.iter().copied().fold(f32::MAX, f32::min);
                if max_l - min_l > 0.5 && self.y_axis_width < 300.0 {
                    self.y_axis_width = (self.y_axis_width + (max_l - min_l)).min(300.0);
                    // Converge right away instead of at the poll interval
                    ui.ctx().request_repaint();
                }
            }

            // --- Interaction detection across sub-plots ---

            // Detect user scroll-zoom: raw_input_hook injects ctrl into MouseWheel
            // events, so the wheel delta is consumed as zoom. Check for the raw
            // MouseWheel event directly (egui 0.34 removed InputState::raw_scroll_delta).
            let wheel_scrolled = ui
                .ctx()
                .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));

            let mut any_hovered = false;
            // Sub-plot that received this frame's gesture (pan / wheel zoom /
            // box zoom / zoom-out click). Only its Y is allowed to move.
            let mut event_row: Option<usize> = None;
            let mut zoom_out_on: Option<usize> = None;
            for (r, show) in shows.iter().enumerate() {
                let pr = &show.response;
                any_hovered |= pr.hovered();
                // Match egui_plot's own zoom test (`contains_pointer`, not
                // `hovered`): over the legend overlay the plot still zooms but
                // is not "hovered", and missing that would let the next frame's
                // auto-fit undo the zoom.
                let touched = pr.dragged()
                    || pr.double_clicked()
                    || (wheel_scrolled && pr.contains_pointer())
                    || (is_zoom_in && pr.drag_stopped());
                if touched {
                    event_row.get_or_insert(r);
                }
                if mode == PlotMode::ZoomOut && pr.clicked() {
                    zoom_out_on = Some(r);
                    event_row = Some(r);
                }
            }

            // Save the bounds each sub-plot actually rendered with. This has to
            // run before the Y pin below, which re-asserts exactly these.
            for (r, show) in shows.iter().enumerate() {
                let bounds = show.transform.bounds();
                self.current_bounds[r] = Some((
                    bounds.min()[0],
                    bounds.max()[0],
                    bounds.min()[1],
                    bounds.max()[1],
                ));
            }

            // ZoomOut: click to zoom out 1.5x centered on the cursor. Computed
            // before the pin below so its X reaches every sub-plot too.
            let zoom_out_target: Option<(f64, f64, f64, f64)> = zoom_out_on.and_then(|r| {
                let hover_pos = shows[r].response.hover_pos()?;
                let plot_pos = shows[r].transform.value_from_position(hover_pos);
                let bounds = shows[r].transform.bounds();
                let half_width = (bounds.max()[0] - bounds.min()[0]) / 2.0 * 1.5;
                let half_height = (bounds.max()[1] - bounds.min()[1]) / 2.0 * 1.5;
                Some((
                    plot_pos.x - half_width,
                    plot_pos.x + half_width,
                    plot_pos.y - half_height,
                    plot_pos.y + half_height,
                ))
            });

            // A gesture takes the view off auto and is re-asserted on the next
            // frame, because neither egui_plot's memory nor the link group can
            // be trusted to carry it alone: the link group ends up holding the
            // X of whichever sub-plot was drawn last, which would undo the
            // gesture when the cursor sits on an earlier one.
            //
            // X: the gesture's X goes to every row (the axis is linked).
            // Y: only the row under the cursor moves — every other row
            //    re-asserts the Y it just rendered with.
            if let Some(evt) = event_row {
                self.toolbar.auto_x = false;
                self.toolbar.auto_y = false;
                let shared_x = zoom_out_target
                    .map(|t| (t.0, t.1))
                    .or_else(|| self.current_bounds[evt].map(|c| (c.0, c.1)));
                for r in 0..n {
                    if shared_x.is_some() {
                        self.pending_x[r] = shared_x;
                    }
                    let y = if r == evt {
                        zoom_out_target.map(|t| (t.2, t.3))
                    } else {
                        None
                    }
                    .or_else(|| self.current_bounds[r].map(|c| (c.2, c.3)));
                    if y.is_some() {
                        self.pending_y[r] = y;
                    }
                }
            }

            // Cursor icon per mode
            if any_hovered {
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

            // --- Custom tooltip + crosshair synchronized across sub-plots ---
            let hovered_row = shows.iter().enumerate().find_map(|(r, show)| {
                let pos = show.response.hover_pos()?;
                show.transform.frame().contains(pos).then_some((r, pos))
            });
            if let Some((r, hover_pos)) = hovered_row {
                let stack_ctx = crate::tooltip::StackContext {
                    series_stacks: &self.series_stacks,
                    series_units: &self.series_units,
                    hovered_stack: self.stacks[r],
                };
                let hover = crate::tooltip::find_hover(
                    hover_pos,
                    &shows[r].transform,
                    &self.data,
                    &views,
                    &stack_ctx,
                    &self.series_visible,
                    &self.colors,
                    is_log_scale,
                    400.0, // 20px squared
                );

                // Vertical line at the same data X in every sub-plot; the
                // horizontal line only in the hovered one.
                let anchor_x = shows[r].transform.value_from_position(hover.anchor).x;
                for (rr, show) in shows.iter().enumerate() {
                    let frame = *show.transform.frame();
                    let sx = if rr == r {
                        hover.anchor.x
                    } else {
                        show.transform
                            .position_from_point(&PlotPoint::new(anchor_x, 0.0))
                            .x
                    };
                    crate::tooltip::render_crosshair_v(ui, frame, sx);
                    if rr == r {
                        crate::tooltip::render_crosshair_h(ui, frame, hover.anchor.y);
                    }
                }

                // The tooltip may overflow the hovered sub-plot into its
                // neighbours — clip it to the whole stack, not to one frame.
                let stack_rect = shows
                    .iter()
                    .fold(egui::Rect::NOTHING, |acc, s| acc.union(*s.transform.frame()));
                crate::tooltip::render_tooltip(
                    ui,
                    *shows[r].transform.frame(),
                    stack_rect,
                    &hover,
                    &self.x_unit,
                    self.x_proportion,
                    is_log_scale,
                    &self.stack_units[r],
                );
            }

            // --- Custom legend (one per sub-plot, its own series only) ---
            for (r, &stack_id) in self.stacks.iter().enumerate() {
                let mut legend_entries: Vec<LegendEntry> = Vec::new();
                let mut flat_idx = 0usize;
                for (gi, group) in self.data.groups.iter().enumerate() {
                    let view = views.get(gi).copied().flatten();
                    for series in &group.series {
                        let i = flat_idx;
                        flat_idx += 1;
                        if self.series_stacks.get(i).copied().unwrap_or(1) != stack_id {
                            continue;
                        }
                        let latest = view.and_then(|v| {
                            let start = v.range.0.min(series.points.len());
                            let end = v.range.1.min(series.points.len());
                            series.points[start..end].iter().rev().find_map(|(_, y)| *y)
                        });
                        legend_entries.push(LegendEntry {
                            flat_idx: i,
                            name: series.name.clone(),
                            color: self.colors[i % self.colors.len()],
                            latest,
                        });
                    }
                }
                crate::custom_legend::render_legend(
                    ui.ctx(),
                    *shows[r].transform.frame(),
                    self.toolbar.legend_pos,
                    stack_id,
                    &legend_entries,
                    &mut self.series_visible,
                    &self.stack_units[r],
                    is_log_scale,
                );
            }
        });

        self.oneshot_tick(ui.ctx(), all_resolved);
    }
}

impl PlotApp {
    /// One-shot: decide whether the view has settled and request the capture.
    /// Settled = every file read through once, every group renderable, no
    /// image still loading, and no immediate repaint requested last pass
    /// (layout convergence: shared y-axis width, legend sizing pass) — held
    /// for `ONESHOT_STABLE_FRAMES` consecutive frames.
    fn oneshot_tick(&mut self, ctx: &egui::Context, all_resolved: bool) {
        let loaded = self.watchers.iter().all(CsvWatcher::initial_load_done);
        let Some(os) = self.oneshot.as_mut() else { return };
        if os.done {
            return;
        }
        if os.requested {
            // The capture arrives a few frames later; keep frames coming.
            ctx.request_repaint();
            return;
        }
        os.frames += 1;
        let settled = loaded
            && all_resolved
            && os.frames >= ONESHOT_MIN_FRAMES
            && !ctx.has_pending_images()
            && !ctx.requested_repaint_last_pass();
        os.stable = if settled { os.stable + 1 } else { 0 };
        let ready = os.stable >= ONESHOT_STABLE_FRAMES;
        if ready || os.started.elapsed() >= ONESHOT_TIMEOUT {
            os.timed_out = !ready;
            os.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(ONESHOT_FRAME_DELAY);
        }
    }

    /// One-shot: write a delivered screenshot, report it, close the app.
    fn oneshot_receive(&mut self, ctx: &egui::Context) {
        let Some(os) = self.oneshot.as_mut() else { return };
        if os.done || !os.requested {
            return;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else { return };
        os.done = true;
        let code = match save_png(&os.path, &image) {
            Ok(()) => {
                let shown = std::path::absolute(&os.path).unwrap_or_else(|_| os.path.clone());
                println!("SCREENSHOT: {}", shown.display());
                if os.timed_out {
                    crate::ONESHOT_EXIT_TIMEOUT
                } else {
                    0
                }
            }
            Err(e) => {
                println!("EXCEPTION: oneshot: cannot write {}: {}", os.path.display(), e);
                1
            }
        };
        crate::ONESHOT_EXIT.store(code, std::sync::atomic::Ordering::SeqCst);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::dedupe_series_names;

    fn to_groups(v: &[&[&str]]) -> Vec<Vec<String>> {
        v.iter()
            .map(|g| g.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn inherited_names_get_numbered_suffixes() {
        let mut g = to_groups(&[&["Temp", "Pres"], &["Temp", "Pres"], &["Temp"]]);
        dedupe_series_names(&mut g);
        assert_eq!(g[0], vec!["Temp", "Pres"]);
        assert_eq!(g[1], vec!["Temp(2)", "Pres(2)"]);
        assert_eq!(g[2], vec!["Temp(3)"]);
    }

    #[test]
    fn unique_names_stay_untouched() {
        let mut g = to_groups(&[&["A", "B"], &["C"]]);
        dedupe_series_names(&mut g);
        assert_eq!(g[0], vec!["A", "B"]);
        assert_eq!(g[1], vec!["C"]);
    }

    #[test]
    fn duplicates_within_one_group_are_suffixed_too() {
        let mut g = to_groups(&[&["A", "A", "A"]]);
        dedupe_series_names(&mut g);
        assert_eq!(g[0], vec!["A", "A(2)", "A(3)"]);
    }

    #[test]
    fn suffix_collision_with_explicit_name_keeps_bumping() {
        let mut g = to_groups(&[&["A", "A", "A(2)"]]);
        dedupe_series_names(&mut g);
        // Second "A" takes "A(2)" first; the literal "A(2)" then bumps.
        assert_eq!(g[0][0], "A");
        assert_eq!(g[0][1], "A(2)");
        assert_eq!(g[0][2], "A(2)(2)");
        // All unique regardless of spelling
        let set: std::collections::HashSet<_> = g[0].iter().collect();
        assert_eq!(set.len(), 3);
    }
}
