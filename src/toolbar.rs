use eframe::egui;
use egui_plot::Corner;

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
            if ui.button("Home").clicked() {
                self.need_home = true;
                self.mode = PlotMode::None;
            }

            ui.separator();

            // Zoom-In toggle
            let zoom_in_label = if self.mode == PlotMode::ZoomIn { "[ Zoom-In ]" } else { "Zoom-In" };
            if ui.selectable_label(self.mode == PlotMode::ZoomIn, zoom_in_label).clicked() {
                self.mode = if self.mode == PlotMode::ZoomIn {
                    PlotMode::None
                } else {
                    PlotMode::ZoomIn
                };
            }

            // Zoom-Out toggle (click-to-zoom-out mode)
            let zoom_out_label = if self.mode == PlotMode::ZoomOut { "[ Zoom-Out ]" } else { "Zoom-Out" };
            if ui.selectable_label(self.mode == PlotMode::ZoomOut, zoom_out_label).clicked() {
                self.mode = if self.mode == PlotMode::ZoomOut {
                    PlotMode::None
                } else {
                    PlotMode::ZoomOut
                };
            }

            ui.separator();

            ui.checkbox(&mut self.auto_x, "Auto-X");
            ui.checkbox(&mut self.auto_y, "Auto-Y");

            ui.separator();

            ui.checkbox(&mut self.log_scale, "Log Scale");

            ui.separator();

            // Legend position 4-way toggle (LB -> LT -> RT -> RB -> LB)
            let legend_label = match self.legend_pos {
                Corner::LeftBottom => "L\u{2199}",
                Corner::LeftTop => "L\u{2196}",
                Corner::RightTop => "R\u{2197}",
                Corner::RightBottom => "R\u{2198}",
            };
            if ui.button(legend_label).on_hover_text("Cycle legend position").clicked() {
                self.legend_pos = match self.legend_pos {
                    Corner::LeftBottom => Corner::LeftTop,
                    Corner::LeftTop => Corner::RightTop,
                    Corner::RightTop => Corner::RightBottom,
                    Corner::RightBottom => Corner::LeftBottom,
                };
            }

            // About button at right end
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").clicked() {
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
