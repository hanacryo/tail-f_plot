use eframe::egui;
use egui_plot::Corner;

use crate::icons::{button_height, icon_button, Icon};
use crate::plot_state::PlotMode;

pub struct ToolbarState {
    pub mode: PlotMode,
    pub auto_x: bool,           // auto bounds for X axis only
    pub auto_y: bool,           // auto bounds for Y axis only
    pub need_home: bool,
    pub log_scale: bool,        // manual log scale toggle
    pub legend_pos: Corner,     // legend position (4 corners)
    pub show_about: bool,       // About modal display flag
    prev_log_scale: bool,       // previous frame log scale state (for change detection)
}

impl Default for ToolbarState {
    fn default() -> Self {
        Self {
            mode: PlotMode::None,
            auto_x: true,
            auto_y: true,
            need_home: false,
            log_scale: false,
            legend_pos: Corner::LeftBottom, // default: bottom-left
            show_about: false,
            prev_log_scale: false,
        }
    }
}

impl ToolbarState {
    pub fn set_log_scale(&mut self, enabled: bool) {
        self.log_scale = enabled;
        self.prev_log_scale = enabled;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if icon_button(ui, Icon::FitAll, false, "Home: fit everything (Auto-X and Auto-Y on)")
                .clicked()
            {
                self.need_home = true;
                self.mode = PlotMode::None;
            }

            ui.separator();

            // Zoom-In toggle
            let zoom_in = self.mode == PlotMode::ZoomIn;
            if icon_button(ui, Icon::ZoomIn, zoom_in, "Zoom in: while on, drag a box to zoom into")
                .clicked()
            {
                self.mode = if zoom_in { PlotMode::None } else { PlotMode::ZoomIn };
            }

            // Zoom-Out toggle (click-to-zoom-out mode)
            let zoom_out = self.mode == PlotMode::ZoomOut;
            let tip = "Zoom out: while on, click to zoom out 1.5x around that point";
            if icon_button(ui, Icon::ZoomOut, zoom_out, tip).clicked() {
                self.mode = if zoom_out { PlotMode::None } else { PlotMode::ZoomOut };
            }

            ui.separator();

            let tip = "Auto-X: fit X to the data\n(with Auto-Y off, to what is drawn in the Y range)";
            if icon_button(ui, Icon::AutoX, self.auto_x, tip).clicked() {
                self.auto_x = !self.auto_x;
            }
            let tip = "Auto-Y: fit Y to the data\n(with Auto-X off, to what is drawn in the X range)";
            if icon_button(ui, Icon::AutoY, self.auto_y, tip).clicked() {
                self.auto_y = !self.auto_y;
            }

            ui.separator();

            // Shows the current Y scale; a click switches to the other one
            let (icon, tip) = if self.log_scale {
                (Icon::Log, "Y scale: log (click for linear)")
            } else {
                (Icon::Linear, "Y scale: linear (click for log)")
            };
            if icon_button(ui, icon, false, tip).clicked() {
                self.log_scale = !self.log_scale;
            }

            ui.separator();

            // Legend position 4-way toggle (LB -> LT -> RT -> RB -> LB)
            let legend_label = match self.legend_pos {
                Corner::LeftBottom => "L\u{2199}",
                Corner::LeftTop => "L\u{2196}",
                Corner::RightTop => "R\u{2197}",
                Corner::RightBottom => "R\u{2198}",
            };
            // Text buttons as tall as the icon buttons
            let min_size = egui::vec2(0.0, button_height(ui));
            if ui
                .add(egui::Button::new(legend_label).min_size(min_size))
                .on_hover_text("Cycle legend position")
                .clicked()
            {
                self.legend_pos = match self.legend_pos {
                    Corner::LeftBottom => Corner::LeftTop,
                    Corner::LeftTop => Corner::RightTop,
                    Corner::RightTop => Corner::RightBottom,
                    Corner::RightBottom => Corner::LeftBottom,
                };
            }

            // About button at right end
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("About").min_size(min_size)).clicked() {
                    self.show_about = true;
                }
            });
        });

        // Detect log scale change -> trigger Home
        if self.log_scale != self.prev_log_scale {
            self.need_home = true;
            self.prev_log_scale = self.log_scale;
        }
    }
}
