#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotMode {
    None,
    ZoomIn,   // box zoom mode (drag to select area)
    ZoomOut,  // click-to-zoom-out at cursor position
}

/// Per-series data buffer.
pub struct SeriesData {
    pub name: String,
    /// (x_minutes, y_value) pairs. y_value is None when the cell was empty.
    pub points: Vec<(f64, Option<f64>)>,
}

impl SeriesData {
    pub fn new(name: String) -> Self {
        Self {
            name,
            points: Vec::new(),
        }
    }
}

pub struct PlotData {
    pub series: Vec<SeriesData>,
    /// The epoch value of the very first data row (used as X origin).
    pub epoch_origin: Option<f64>,
    /// If true, use raw X values without origin subtraction
    absolute_x: bool,
}

impl PlotData {
    pub fn new(names: &[String], absolute_x: bool) -> Self {
        Self {
            series: names.iter().map(|n| SeriesData::new(n.clone())).collect(),
            epoch_origin: None,
            absolute_x,
        }
    }

    /// Push one row of data. `x_value` has x_proportion already applied.
    /// If absolute_x is false, the first row's x is subtracted as origin.
    pub fn push_row(&mut self, x_value: f64, y_values: &[Option<f64>]) {
        let x = if self.absolute_x {
            x_value
        } else {
            if self.epoch_origin.is_none() {
                self.epoch_origin = Some(x_value);
            }
            x_value - self.epoch_origin.unwrap()
        };

        for (i, series) in self.series.iter_mut().enumerate() {
            let y = y_values.get(i).copied().flatten();
            series.points.push((x, y));
        }
    }

    /// Compute render range based on max points and max time.
    /// Returns None if no data.
    pub fn get_render_range(&self, max_points: usize, max_time_minutes: f64) -> Option<(usize, usize)> {
        if self.series.is_empty() {
            return None;
        }
        let total = self.series[0].points.len();
        if total == 0 {
            return None;
        }

        // Time-based start index
        let last_x = self.series[0].points.last().map(|(x, _)| *x).unwrap_or(0.0);
        let time_start_x = last_x - max_time_minutes;

        // Binary search for time-based start index
        let time_start_idx = self.series[0]
            .points
            .partition_point(|(x, _)| *x < time_start_x);

        // Point-count-based start index
        let points_start_idx = if total > max_points {
            total - max_points
        } else {
            0
        };

        // Use the larger (renders fewer points)
        let start_idx = time_start_idx.max(points_start_idx);
        Some((start_idx, total))
    }

    /// Compute X/Y bounds across all series. Only valid values included.
    /// Returns None if no data.
    pub fn get_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut x_min = f64::MAX;
        let mut x_max = f64::MIN;
        let mut y_min = f64::MAX;
        let mut y_max = f64::MIN;
        let mut has_data = false;

        for series in &self.series {
            for &(x, y_opt) in &series.points {
                if let Some(y) = y_opt {
                    has_data = true;
                    x_min = x_min.min(x);
                    x_max = x_max.max(x);
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }

        if has_data {
            // Add 5% margin
            let x_margin = (x_max - x_min).abs() * 0.05;
            let y_margin = (y_max - y_min).abs() * 0.05;
            // Use default margin if zero
            let x_margin = if x_margin > 0.0 { x_margin } else { 1.0 };
            let y_margin = if y_margin > 0.0 { y_margin } else { 1.0 };
            Some((x_min - x_margin, x_max + x_margin, y_min - y_margin, y_max + y_margin))
        } else {
            None
        }
    }
}
