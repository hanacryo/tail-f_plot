#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotMode {
    None,
    ZoomIn,   // box zoom mode (drag to select area)
    ZoomOut,  // click-to-zoom-out at cursor position
}

/// Per-series data buffer.
pub struct SeriesData {
    pub name: String,
    /// (x, y_value) pairs. `x` is the raw column value with x-proportion
    /// applied but before any origin/epoch offset — offsets are applied at
    /// render time (see `PlotData::offset`). y_value is None when the cell
    /// was empty.
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

/// All series read from one watched file. Series within a group are
/// row-aligned (every `push_row` appends to all of them).
pub struct FileGroup {
    pub series: Vec<SeriesData>,
    /// Scaled x of the very first data row (origin for relative-x display).
    pub origin_x: Option<f64>,
    /// (scaled x, scaled epoch) of the first row whose epoch column parsed.
    pub epoch_anchor: Option<(f64, f64)>,
    /// Whether this group has an epoch-time column configured.
    epoch_expected: bool,
    /// True while every row's x is >= the previous row's. A time column stays
    /// sorted; an arbitrary X column (e.g. R plotted against a temperature
    /// that ramps up and down) does not. Binary search and the
    /// "first/last row = min/max x" shortcut are only valid while this holds.
    pub x_sorted: bool,
}

/// Per-group render info: the visible index range on the group's row-aligned
/// point buffers, plus the constant display offset added to stored x values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupView {
    pub range: (usize, usize),
    pub offset: f64,
}

pub struct PlotData {
    pub groups: Vec<FileGroup>,
    /// If true, use raw X values without origin subtraction
    absolute_x: bool,
}

impl PlotData {
    pub fn new(absolute_x: bool) -> Self {
        Self {
            groups: Vec::new(),
            absolute_x,
        }
    }

    pub fn add_group(&mut self, names: &[String], epoch_expected: bool) {
        self.groups.push(FileGroup {
            series: names.iter().map(|n| SeriesData::new(n.clone())).collect(),
            origin_x: None,
            epoch_anchor: None,
            epoch_expected,
            x_sorted: true,
        });
    }

    pub fn series_count(&self) -> usize {
        self.groups.iter().map(|g| g.series.len()).sum()
    }

    /// Forget everything read for a group (its file was replaced, truncated
    /// or rewritten): rows, origin, epoch anchor and sorted-x tracking.
    pub fn reset_group(&mut self, group: usize) {
        let g = &mut self.groups[group];
        for s in &mut g.series {
            s.points.clear();
        }
        g.origin_x = None;
        g.epoch_anchor = None;
        g.x_sorted = true;
    }

    /// Set a group's origin / epoch anchor found in rows that are not loaded
    /// (the head of a file read from its tail). Already-set values win, as in
    /// `push_row`, so call this before pushing the loaded rows.
    pub fn seed_group(&mut self, group: usize, origin_x: Option<f64>, epoch_anchor: Option<(f64, f64)>) {
        let g = &mut self.groups[group];
        if g.origin_x.is_none() {
            g.origin_x = origin_x;
        }
        if g.epoch_expected && g.epoch_anchor.is_none() {
            g.epoch_anchor = epoch_anchor;
        }
    }

    /// Whether a group still lacks its origin / epoch anchor.
    pub fn needs_seed(&self, group: usize) -> (bool, bool) {
        let g = &self.groups[group];
        (g.origin_x.is_none(), g.epoch_expected && g.epoch_anchor.is_none())
    }

    /// Drop a group's oldest rows so at most `keep` remain. Rows beyond the
    /// newest `max_points` are never rendered (`views` starts at least at
    /// `total - max_points`) and the origin / epoch anchor are stored apart,
    /// so this changes nothing on screen. Amortized: rows are only drained
    /// once the excess reaches `max(keep, 1024)`.
    pub fn evict_old_rows(&mut self, group: usize, keep: usize) {
        let g = &mut self.groups[group];
        let len = g.series.first().map_or(0, |s| s.points.len());
        if len <= keep.saturating_add(keep.max(1024)) {
            return;
        }
        let drop = len - keep;
        for s in &mut g.series {
            s.points.drain(..drop);
        }
    }

    /// Push one row into a group. `x` and `epoch` have their proportions
    /// already applied.
    pub fn push_row(&mut self, group: usize, x: f64, epoch: Option<f64>, y_values: &[Option<f64>]) {
        let g = &mut self.groups[group];
        if g.origin_x.is_none() {
            g.origin_x = Some(x);
        }
        // One out-of-order row is enough to disable the sorted-x shortcuts for
        // this group; the flag never goes back to true.
        if let Some(&(prev_x, _)) = g.series.first().and_then(|s| s.points.last()) {
            if x < prev_x {
                g.x_sorted = false;
            }
        }
        if g.epoch_expected && g.epoch_anchor.is_none() {
            if let Some(e) = epoch {
                g.epoch_anchor = Some((x, e));
            }
        }
        for (i, series) in g.series.iter_mut().enumerate() {
            let y = y_values.get(i).copied().flatten();
            series.points.push((x, y));
        }
    }

    /// Display offset for a group, or None while it cannot be resolved yet
    /// (data keeps accumulating; the group renders once resolvable).
    ///
    /// - `--absolute-x`: always 0 (epoch alignment unused).
    /// - Otherwise the first group's first row is the shared origin: every
    ///   group is shifted by `-origin_x(first group)`.
    /// - A later group with an epoch column configured — and the first group's
    ///   too — is instead shifted so equal epoch values land on equal display
    ///   x, aligning files whose recordings started at different times.
    pub fn offset(&self, group: usize) -> Option<f64> {
        if self.absolute_x {
            return Some(0.0);
        }
        let g0 = self.groups.first()?;
        let origin0 = g0.origin_x?;
        if group == 0 {
            return Some(-origin0);
        }
        let gk = self.groups.get(group)?;
        if gk.epoch_expected && g0.epoch_expected {
            // Wait for both anchors so the group doesn't jump from the
            // shared-origin position to the epoch-aligned one mid-flight.
            let (ax_k, ae_k) = gk.epoch_anchor?;
            let (ax_0, ae_0) = g0.epoch_anchor?;
            // A row of group k at wall time T must land where group 0 would
            // plot its own row at T:
            //   display = (x - ax_k) + (ae_k - ae_0) + (ax_0 - origin0)
            Some((ae_k - ae_0) - ax_k + ax_0 - origin0)
        } else {
            Some(-origin0)
        }
    }

    /// Compute per-group render views honoring max_points / max_x_range
    /// (the stricter of the two applies, per group). The x-range window is
    /// anchored at the latest display x across all resolvable groups.
    /// None = the group's offset is not resolvable yet.
    pub fn views(&self, max_points: usize, max_x_range: f64) -> Vec<Option<GroupView>> {
        let offsets: Vec<Option<f64>> = (0..self.groups.len()).map(|k| self.offset(k)).collect();

        let mut last_display: Option<f64> = None;
        for (g, off) in self.groups.iter().zip(&offsets) {
            if let (Some(off), Some(&(x, _))) =
                (off, g.series.first().and_then(|s| s.points.last()))
            {
                let d = x + off;
                last_display = Some(last_display.map_or(d, |v: f64| v.max(d)));
            }
        }

        self.groups
            .iter()
            .zip(&offsets)
            .map(|(g, off)| {
                let off = (*off)?;
                let total = g.series.first().map(|s| s.points.len()).unwrap_or(0);
                let last = match last_display {
                    Some(v) if total > 0 => v,
                    _ => return Some(GroupView { range: (0, 0), offset: off }),
                };

                // Time-based start index (threshold converted to stored x)
                let raw_threshold = (last - max_x_range) - off;
                let points = &g.series[0].points;
                let time_start = if g.x_sorted {
                    points.partition_point(|(x, _)| *x < raw_threshold)
                } else {
                    // Unsorted x: walk back from the newest row and stop at the
                    // first one outside the window. Keeps the rendered slice
                    // contiguous (no holes in the line) and reduces to the
                    // binary search above whenever x happens to be sorted.
                    let mut i = points.len();
                    while i > 0 && points[i - 1].0 >= raw_threshold {
                        i -= 1;
                    }
                    i
                };

                // Point-count-based start index
                let count_start = total.saturating_sub(max_points);

                // Use the larger (renders fewer points)
                Some(GroupView {
                    range: (time_start.max(count_start), total),
                    offset: off,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("s{}", i)).collect()
    }

    #[test]
    fn shared_origin_from_first_group() {
        let mut d = PlotData::new(false);
        d.add_group(&names(1), false);
        d.add_group(&names(1), false);

        // Unresolved until the first group has data
        assert_eq!(d.offset(0), None);
        assert_eq!(d.offset(1), None);

        d.push_row(1, 105.0, None, &[Some(2.0)]);
        assert_eq!(d.offset(1), None); // still waiting for group 0

        d.push_row(0, 100.0, None, &[Some(1.0)]);
        assert_eq!(d.offset(0), Some(-100.0));
        assert_eq!(d.offset(1), Some(-100.0)); // shared origin
    }

    #[test]
    fn absolute_x_needs_no_origin() {
        let mut d = PlotData::new(true);
        d.add_group(&names(1), true);
        d.add_group(&names(1), true);
        assert_eq!(d.offset(0), Some(0.0));
        assert_eq!(d.offset(1), Some(0.0));
        d.push_row(1, 50.0, Some(9999.0), &[Some(1.0)]);
        assert_eq!(d.offset(1), Some(0.0)); // epoch alignment unused
    }

    #[test]
    fn epoch_alignment_shifts_by_wall_clock_difference() {
        let mut d = PlotData::new(false);
        d.add_group(&names(1), true);
        d.add_group(&names(1), true);

        // Both files log elapsed x starting at 0; recordings 30 units apart.
        d.push_row(0, 0.0, Some(1000.0), &[Some(1.0)]);
        d.push_row(1, 0.0, Some(1030.0), &[Some(2.0)]);

        assert_eq!(d.offset(0), Some(0.0));
        assert_eq!(d.offset(1), Some(30.0));
    }

    #[test]
    fn epoch_alignment_waits_for_anchor() {
        let mut d = PlotData::new(false);
        d.add_group(&names(1), true);
        d.add_group(&names(1), true);

        d.push_row(0, 0.0, Some(1000.0), &[Some(1.0)]);
        // Group 1 rows whose epoch cell failed to parse: buffered, unresolved.
        d.push_row(1, 0.0, None, &[Some(2.0)]);
        assert_eq!(d.offset(1), None);

        // Anchor pairs the first valid epoch with that row's x.
        d.push_row(1, 5.0, Some(1035.0), &[Some(3.0)]);
        assert_eq!(d.offset(1), Some(30.0));
    }

    #[test]
    fn epoch_falls_back_to_shared_origin_without_first_group_column() {
        let mut d = PlotData::new(false);
        d.add_group(&names(1), false); // first file has no epoch column
        d.add_group(&names(1), true);

        d.push_row(0, 100.0, None, &[Some(1.0)]);
        d.push_row(1, 200.0, Some(1030.0), &[Some(2.0)]);
        assert_eq!(d.offset(1), Some(-100.0));
    }

    #[test]
    fn views_window_anchored_at_global_latest() {
        let mut d = PlotData::new(true);
        d.add_group(&names(1), false);
        d.add_group(&names(1), false);

        for x in 0..10 {
            d.push_row(0, x as f64, None, &[Some(1.0)]);
        }
        d.push_row(1, 100.0, None, &[Some(2.0)]);

        // Window = [100 - 5, 100]: group 0 entirely outside, group 1 inside.
        let views = d.views(1000, 5.0);
        assert_eq!(views[0].unwrap().range, (10, 10));
        assert_eq!(views[1].unwrap().range, (0, 1));
    }

    #[test]
    fn views_max_points_applies_per_group() {
        let mut d = PlotData::new(true);
        d.add_group(&names(1), false);
        for x in 0..10 {
            d.push_row(0, x as f64, None, &[Some(1.0)]);
        }
        let views = d.views(3, 1e9);
        assert_eq!(views[0].unwrap().range, (7, 10));
    }

    #[test]
    fn x_sorted_flag_tracks_out_of_order_rows() {
        let mut d = PlotData::new(true);
        d.add_group(&names(1), false);
        for x in [10.0, 20.0, 20.0, 30.0] {
            d.push_row(0, x, None, &[Some(1.0)]);
        }
        assert!(d.groups[0].x_sorted); // ties stay sorted
        d.push_row(0, 25.0, None, &[Some(1.0)]);
        assert!(!d.groups[0].x_sorted);
        d.push_row(0, 99.0, None, &[Some(1.0)]);
        assert!(!d.groups[0].x_sorted); // never goes back
    }

    #[test]
    fn views_window_on_unsorted_x_keeps_a_contiguous_tail() {
        let mut d = PlotData::new(true);
        d.add_group(&names(1), false);
        // Newest row is x = 40; rows 0..2 fall outside a window of 15.
        for x in [30.0, 50.0, 20.0, 40.0] {
            d.push_row(0, x, None, &[Some(1.0)]);
        }
        // Window = [40 - 15, ..] → walking back stops at x = 20 (row 2).
        assert_eq!(d.views(1000, 15.0)[0].unwrap().range, (3, 4));
        // A window wide enough to cover every row keeps them all.
        assert_eq!(d.views(1000, 1e9)[0].unwrap().range, (0, 4));
    }

    #[test]
    fn unresolved_group_has_no_view() {
        let mut d = PlotData::new(false);
        d.add_group(&names(1), false);
        d.add_group(&names(1), false);
        d.push_row(1, 5.0, None, &[Some(1.0)]);

        let views = d.views(1000, 1e9);
        assert!(views[0].is_none());
        assert!(views[1].is_none());
    }
}
