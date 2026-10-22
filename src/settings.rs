//! Settings window: a separate native window (egui deferred viewport) for
//! changing the command-line options at runtime. While it is open the main
//! window keeps rendering but takes no input: on Windows the settings window
//! is owned by the main window, centered on its monitor's work area, and the
//! main window is disabled, like a modal dialog.
//!
//! The window edits a `Draft` (every field as typed) and applies it through
//! the command line: the draft is written as an argv and parsed like the real
//! one, so what is applied is exactly what the copied command line starts.
//!
//! Showing it and the modal bookkeeping run in `App::logic`: eframe skips
//! `ui()` while the main window is minimized, and a pass that does not show
//! the viewport closes it, which would leave the main window disabled.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui;
use egui::{Align, Layout, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId};

use crate::cmdline::{self, ParsedArgs};
use crate::groups::{resolve_x_proportion, GroupConfig};
use crate::{resolve_delimiter, resolve_string_quote, Cli};

/// Fixed size of the settings window (points); long lists scroll inside it
const WINDOW_SIZE: [f32; 2] = [720.0, 520.0];
/// How long "Copied to clipboard" stays
const TOAST_SECONDS: f64 = 2.0;
/// `--oneshot-settings`: frames the window renders before it is captured
const ONESHOT_FRAMES: u32 = 4;
/// Field widths: text, number, small number
const WIDE: f32 = 280.0;
const NARROW: f32 = 90.0;
const NUM: f32 = 50.0;
/// Files tab: the file group list, the path field, the Y column sheet
const LIST_WIDTH: f32 = 160.0;
const LIST_HEIGHT: f32 = 300.0;
const PATH_WIDTH: f32 = 380.0;
const SHEET_HEIGHT: f32 = 130.0;
/// A header read that has not answered after this long (e.g. a network path
/// that hangs) no longer holds back the next one
const HEADER_READ_PATIENCE: f64 = 2.0;
/// Seconds after which a header row shown is read again
const HEADER_REFRESH: f64 = 2.0;

/// `--delimiter` / `--string-quote` / `--x-time-scale` names and labels
const DELIMITERS: [(&str, &str); 5] = [
    ("comma", "comma ( , )"),
    ("tab", "tab"),
    ("semicolon", "semicolon ( ; )"),
    ("space", "space"),
    ("vbar", "vertical bar ( | )"),
];
const QUOTES: [(&str, &str); 4] = [
    ("none", "none"),
    ("squote", "single ( ' )"),
    ("dquote", "double ( \" )"),
    ("backtick", "backtick ( ` )"),
];
const X_SCALES: [(&str, &str); 4] = [
    ("d", "days (\u{f7} 86400)"),
    ("h", "hours (\u{f7} 3600)"),
    ("m", "minutes (\u{f7} 60)"),
    ("s", "seconds (\u{d7} 1)"),
];

/// Hover help of every field and its label, with the command-line option.
mod help {
    pub const X_AXIS_LABEL: &str =
        "Text under the X axis; the X unit follows in parentheses.\n(--x-axis-label)";
    pub const X_UNIT: &str =
        "Unit shown after the X axis label and in the hover readout, e.g. min or s.\n(--x-unit)";
    pub const Y_UNIT: &str = "Y axis label of every sub-plot, also shown after the values in \
        the legend. \"Y unit per stack\" overrides it per sub-plot.\n(--y-unit)";
    pub const Y_UNITS: &str = "Units of sub-plot 1, 2, 3, ... in order, comma separated \
        (e.g. K,mbar). A sub-plot without an entry uses Y unit.\n(--y-units)";
    pub const COLORS: &str = "Line colors in series order as #RRGGBB, comma separated; they \
        repeat when there are more series. Empty = the default 12 colors.\n(--colors)";
    pub const LINE_WIDTH: &str = "Line thickness, in points.\n(--line-width)";
    pub const MARKER_RADIUS: &str = "Radius of the point markers, in points.\n(--marker-radius)";
    pub const MAX_POINTS: &str = "How many of the newest rows each file keeps and draws; older \
        rows are dropped. With Max X range, the stricter of the two applies. Changing it reads \
        the files again.\n(--max-points)";
    pub const MAX_X_RANGE: &str = "Width of the X window that ends at the newest row, in \
        displayed X units (after the X scale). With Max points, the stricter of the two \
        applies.\n(--max-x-range)";
    pub const REPAINT: &str = "How often the plot redraws, in milliseconds. A change of a \
        file also redraws at once.\n(--repaint-interval-ms)";
    pub const LOG_Y: &str =
        "Logarithmic Y axis, the same as the toolbar's Y scale button.\n(--log-y)";
    pub const ABSOLUTE_X: &str = "Plot X values as they are. Off: X counts from the first row \
        of the first file, which is 0. Changing it reads the files again.\n(--absolute-x)";
    pub const MONITOR: &str = "Monitor the bounds are measured on (numbered as Windows lists \
        them). Without bounds, choosing one centers the window on it. Default: monitor 0 for \
        bounds.\n(--monitor)";
    pub const BOUNDS: &str = "Window position and size as percentages of the monitor's work \
        area (the screen without the taskbar): left, top, right, bottom. All four empty: the \
        window stays as it is.\n(--bounds)";

    pub const FILE_LIST: &str = "Files in command-line order. The first file sets the shared \
        X origin (0) and the epoch the others align to.";
    pub const PATH: &str = "The CSV file to plot. It need not exist yet: the plot waits for \
        it and follows it as it grows.\n(the file path argument)";
    pub const BROWSE: &str = "Choose the file";
    pub const ADD_FILE: &str = "Add a copy of the selected file's settings below it";
    pub const REMOVE_FILE: &str = "Remove the selected file";
    pub const UP: &str = "Move the selected file up (the first file sets the X origin)";
    pub const DOWN: &str = "Move the selected file down (the first file sets the X origin)";
    pub const HEADER_ROW: &str = "Line number (1 = first line) with the column names, listed \
        in the column pickers. 0 = no header line.\n(--header-row)";
    pub const DATA_START_ROW: &str =
        "Line number (1 = first line) of the first data row.\n(--data-start-row)";
    pub const X_COL: &str = "Column with the X values (1 = first column). The list on the \
        right picks one by its header name.\n(--x-col)";
    pub const X_SCALE: &str = "Raw X values in seconds shown as days, hours, minutes or \
        seconds; or multiplied by a custom factor.\n(--x-time-scale, --x-proportion)";
    pub const X_FACTOR: &str = "Raw X values are multiplied by this factor.\n(--x-proportion)";
    pub const DELIMITER: &str = "Character between columns.\n(--delimiter)";
    pub const QUOTE: &str = "Quote character removed around cell values.\n(--string-quote)";
    pub const FIXED_WIDTH: &str = "Column widths in characters, comma separated, for files \
        laid out in fixed columns. Empty = split on the delimiter.\n(--fixed-width)";
    pub const MERGE: &str = "Several delimiters in a row count as one (e.g. columns padded \
        with spaces).\n(--merge-delimiter)";
    pub const EPOCH_COL: &str = "Column with an absolute time (epoch), to line up files that \
        started recording at different times. 0 = off.\n(--epochtime-col)";
    pub const EPOCH_FACTOR: &str = "Multiplier that turns epoch values into displayed X \
        units. Empty = the same factor as X.\n(--epochtime-proportion)";
    pub const Y_COL: &str = "Column with this series' Y values (1 = first column). The list \
        on the right picks one by its header name.\n(--y-cols)";
    pub const NAME: &str = "Name in the legend. Name every column or none: none gives the \
        default names (ColN, or file:ColN with several files).\n(--y-names)";
    pub const STACK: &str = "Sub-plot this series is drawn in: 1 = the top one, 2 = the next \
        one below, and so on. Series with the same number share a sub-plot.\n(--y-stacks)";
    pub const ADD_COLUMN: &str = "Add a Y column (the next column number, same stack)";
    pub const REMOVE_COLUMN: &str = "Remove this Y column";

    pub const COPY: &str = "Copy the command line to the clipboard, for the cmd prompt";
    pub const COPY_BATCH: &str =
        "Copy the command line with every % doubled, for a .cmd (batch) file";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Display,
    Files,
    Window,
    CommandLine,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Display, Tab::Files, Tab::Window, Tab::CommandLine];

    fn label(self) -> &'static str {
        match self {
            Tab::Display => "Display",
            Tab::Files => "Files",
            Tab::Window => "Window",
            Tab::CommandLine => "Command line",
        }
    }

    /// The tab showing field `key` of a problem ("" = no field)
    fn of_field(key: &str) -> Option<Tab> {
        if key.is_empty() {
            None
        } else if key.starts_with("bounds") {
            Some(Tab::Window)
        } else if key.starts_with("file") {
            Some(Tab::Files)
        } else {
            Some(Tab::Display)
        }
    }

    /// `--oneshot-settings` value (clap accepts only these names)
    fn from_name(name: &str) -> Tab {
        match name {
            "files" => Tab::Files,
            "window" => Tab::Window,
            "command-line" => Tab::CommandLine,
            _ => Tab::Display,
        }
    }
}

/// A draft problem: the field it belongs to ("" = none) and the message.
/// File group fields are keyed `file<index>:<field>`.
type Problem = (String, String);

const BOUND_KEYS: [&str; 4] = ["bounds_left", "bounds_top", "bounds_right", "bounds_bottom"];
const BOUND_NAMES: [&str; 4] = ["left", "top", "right", "bottom"];

/// What the settings window edits, every text as typed: a half-typed
/// number is neither lost nor rewritten, and an unchanged field gives back
/// exactly the value it was made from.
#[derive(Clone, PartialEq)]
struct Draft {
    x_axis_label: String,
    x_unit: String,
    y_unit: String,
    y_units: String,
    /// The list `y_units` was made from: `--y-units=` (one empty unit) joins
    /// to "" like no units at all, but gives stack 1 no unit instead of
    /// Y unit. Kept while the field still reads as it was made.
    y_units_given: Vec<String>,
    line_width: String,
    marker_radius: String,
    colors: String,
    max_points: String,
    max_x_range: String,
    repaint_interval_ms: String,
    log_y: bool,
    absolute_x: bool,
    monitor: Option<u32>,
    bounds: [String; 4],
    groups: Vec<GroupDraft>,
}

/// One file group of a draft (the Files tab).
#[derive(Clone, PartialEq)]
struct GroupDraft {
    path: String,
    x_col: String,
    header_row: String,
    data_start_row: String,
    /// `--delimiter` / `--string-quote` names
    delimiter: &'static str,
    quote: &'static str,
    merge_delimiter: bool,
    fixed_width: String,
    /// `--x-time-scale` name; None = the custom factor in `x_proportion`
    x_scale: Option<&'static str>,
    x_proportion: String,
    epochtime_col: String,
    /// Empty = follow the X factor
    epochtime_proportion: String,
    columns: Vec<ColumnDraft>,
}

/// One Y column: number, legend name, stack.
#[derive(Clone, Default, PartialEq)]
struct ColumnDraft {
    col: String,
    name: String,
    stack: String,
}

/// A draft that passed the command-line round trip.
struct Built {
    cli: Cli,
    groups: Vec<GroupConfig>,
}

impl Draft {
    fn new(cli: &Cli, groups: &[GroupConfig]) -> Self {
        let mut bounds: [String; 4] = Default::default();
        for (field, value) in bounds.iter_mut().zip(&cli.bounds) {
            *field = value.to_string();
        }
        Self {
            x_axis_label: cli.x_axis_label.clone(),
            x_unit: cli.x_unit.clone(),
            y_unit: cli.y_unit.clone(),
            y_units: cli.y_units.join(","),
            y_units_given: cli.y_units.clone(),
            line_width: cli.line_width.to_string(),
            marker_radius: cli.marker_radius.to_string(),
            colors: cli.colors.join(","),
            max_points: cli.max_points.to_string(),
            max_x_range: cli.max_x_range.to_string(),
            repaint_interval_ms: cli.repaint_interval_ms.to_string(),
            log_y: cli.log_y,
            absolute_x: cli.absolute_x,
            monitor: cli.monitor,
            bounds,
            groups: groups.iter().map(GroupDraft::new).collect(),
        }
    }

    /// The settings this draft describes: fields checked one by one, then
    /// written as an argv and parsed like the real command line. `base`
    /// supplies what the window does not edit.
    fn build(&self, base: &Cli) -> Result<Built, Vec<Problem>> {
        let mut problems = Vec::new();
        let p = &mut problems;
        let mut cli = base.clone();
        cli.x_axis_label = self.x_axis_label.clone();
        cli.x_unit = self.x_unit.clone();
        cli.y_unit = self.y_unit.clone();
        cli.y_units = if self.y_units == self.y_units_given.join(",") {
            self.y_units_given.clone()
        } else {
            list(&self.y_units)
        };
        let width = number(p, "line_width", &self.line_width, "Line width", |v: f32| {
            v.is_finite() && v > 0.0
        });
        let radius = number(p, "marker_radius", &self.marker_radius, "Marker radius", |v: f32| {
            v.is_finite() && v >= 0.0
        });
        let points = number(p, "max_points", &self.max_points, "Max points", |v: usize| v >= 1);
        let range = number(p, "max_x_range", &self.max_x_range, "Max X range", |v: f64| {
            v.is_finite() && v > 0.0
        });
        let repaint = number(p, "repaint", &self.repaint_interval_ms, "Repaint interval", |v: u64| {
            v >= 1
        });
        cli.line_width = width.unwrap_or(cli.line_width);
        cli.marker_radius = radius.unwrap_or(cli.marker_radius);
        cli.max_points = points.unwrap_or(cli.max_points);
        cli.max_x_range = range.unwrap_or(cli.max_x_range);
        cli.repaint_interval_ms = repaint.unwrap_or(cli.repaint_interval_ms);
        cli.colors = list(&self.colors);
        if let Some(bad) = cli.colors.iter().find(|c| crate::parse_color(c).is_none()) {
            p.push(("colors".into(), format!("Colors: \"{bad}\" is not #RRGGBB")));
        }
        cli.log_y = self.log_y;
        cli.absolute_x = self.absolute_x;
        cli.monitor = self.monitor;
        cli.bounds = self.build_bounds(p);
        let groups: Vec<GroupConfig> =
            self.groups.iter().enumerate().map(|(i, g)| g.build(i, p)).collect();
        if !problems.is_empty() {
            return Err(problems);
        }

        let argv = cmdline::to_argv(&cli, &groups).map_err(|e| vec![(String::new(), e)])?;
        let full = std::iter::once("tail-f_plot".to_string()).chain(argv.iter().cloned());
        match cmdline::parse_args(full.collect()) {
            Ok(ParsedArgs::Run { cli, groups }) => Ok(Built { cli, groups }),
            Ok(ParsedArgs::Help { .. }) => Err(vec![(String::new(), "No file to plot".into())]),
            Err(e) => Err(vec![(String::new(), clap_message(&e))]),
        }
    }

    /// All four empty = no bounds; otherwise four percentages with left <
    /// right and top < bottom.
    fn build_bounds(&self, p: &mut Vec<Problem>) -> Vec<f64> {
        if self.bounds.iter().all(|b| b.trim().is_empty()) {
            return Vec::new();
        }
        let mut values = [0.0; 4];
        let mut valid = true;
        for i in 0..4 {
            let name = format!("Bounds {}", BOUND_NAMES[i]);
            match number(p, BOUND_KEYS[i], &self.bounds[i], &name, |v: f64| {
                (0.0..=100.0).contains(&v)
            }) {
                Some(v) => values[i] = v,
                None => valid = false,
            }
        }
        if valid && values[0] >= values[2] {
            p.push(("bounds_right".into(), "Bounds: right must be greater than left".into()));
        }
        if valid && values[1] >= values[3] {
            p.push(("bounds_bottom".into(), "Bounds: bottom must be greater than top".into()));
        }
        values.to_vec()
    }
}

impl GroupDraft {
    fn new(g: &GroupConfig) -> Self {
        let delimiter = DELIMITERS
            .iter()
            .map(|&(name, _)| name)
            .find(|&name| resolve_delimiter(name) == g.delimiter)
            .unwrap_or("comma");
        let quote = QUOTES
            .iter()
            .map(|&(name, _)| name)
            .find(|&name| resolve_string_quote(name) == g.string_quote)
            .unwrap_or("none");
        let x_scale = X_SCALES
            .iter()
            .map(|&(name, _)| name)
            .find(|&name| resolve_x_proportion(Some(name), f64::NAN) == g.x_proportion);
        let columns = g
            .y_cols
            .iter()
            .enumerate()
            .map(|(i, col)| ColumnDraft {
                col: col.to_string(),
                name: g.y_names.get(i).cloned().unwrap_or_default(),
                // As drawn: a missing entry or 0 is stack 1
                stack: g.y_stacks.get(i).copied().unwrap_or(1).max(1).to_string(),
            })
            .collect();
        Self {
            path: g.csv_path.display().to_string(),
            x_col: g.x_col.to_string(),
            header_row: g.header_row.to_string(),
            data_start_row: g.data_start_row.to_string(),
            delimiter,
            quote,
            merge_delimiter: g.merge_delimiter,
            fixed_width: g.fixed_width.iter().map(ToString::to_string).collect::<Vec<_>>().join(","),
            x_scale,
            x_proportion: g.x_proportion.to_string(),
            epochtime_col: g.epochtime_col.to_string(),
            epochtime_proportion: g.epochtime_proportion.map(|p| p.to_string()).unwrap_or_default(),
            columns,
        }
    }

    /// The group's settings; problems are keyed `file<gi>:<field>`.
    fn build(&self, gi: usize, p: &mut Vec<Problem>) -> GroupConfig {
        let key = |field: &str| format!("file{gi}:{field}");
        let n = gi + 1;
        let path = self.path.trim();
        if path.is_empty() {
            p.push((key("path"), format!("File {n}: no file path")));
        }
        let what = |name: &str| format!("File {n}: {name}");
        let x_col = number(p, key("x_col"), &self.x_col, &what("X column"), |v: usize| v >= 1);
        let header_row = number(p, key("header_row"), &self.header_row, &what("Header row"), |_: usize| {
            true
        });
        let data_start_row =
            number(p, key("data_start_row"), &self.data_start_row, &what("Data start row"), |v: usize| {
                v >= 1
            });
        let fixed_width = number_list(p, key("fixed_width"), &self.fixed_width, &what("Fixed widths"));
        let x_proportion = match self.x_scale {
            Some(name) => Some(resolve_x_proportion(Some(name), f64::NAN)),
            None => number(p, key("x_proportion"), &self.x_proportion, &what("X factor"), |v: f64| {
                v.is_finite() && v != 0.0
            }),
        };
        let epochtime_col = number(p, key("epochtime_col"), &self.epochtime_col, &what("Epoch column"), |_: usize| {
            true
        });
        let epochtime_proportion = if self.epochtime_proportion.trim().is_empty() {
            None
        } else {
            let text = &self.epochtime_proportion;
            number(p, key("epochtime_proportion"), text, &what("Epoch factor"), |v: f64| {
                v.is_finite() && v != 0.0
            })
        };

        if self.columns.is_empty() {
            p.push((key("columns"), format!("File {n}: add at least one Y column")));
        }
        let mut y_cols = Vec::new();
        let mut y_stacks = Vec::new();
        for (i, c) in self.columns.iter().enumerate() {
            let col = number(p, key(&format!("col{i}")), &c.col, &what("Y column"), |v: usize| v >= 1);
            y_cols.push(col.unwrap_or(1));
            let stack = if c.stack.trim().is_empty() {
                Some(1)
            } else {
                number(p, key(&format!("stack{i}")), &c.stack, &what("Stack"), |v: usize| v >= 1)
            };
            y_stacks.push(stack.unwrap_or(1));
        }
        // Legend names: every column or none (none = default names)
        let named = self.columns.iter().filter(|c| !c.name.trim().is_empty()).count();
        let y_names = if named == 0 {
            Vec::new()
        } else {
            for (i, c) in self.columns.iter().enumerate() {
                if c.name.trim().is_empty() {
                    let msg = format!("File {n}: name every Y column, or none for default names");
                    p.push((key(&format!("name{i}")), msg));
                } else if c.name.contains(',') {
                    let msg = format!("File {n}: legend name \"{}\" cannot contain a comma", c.name);
                    p.push((key(&format!("name{i}")), msg));
                }
            }
            self.columns.iter().map(|c| c.name.clone()).collect()
        };

        GroupConfig {
            csv_path: PathBuf::from(path),
            x_col: x_col.unwrap_or(1),
            y_cols,
            y_names,
            y_stacks,
            header_row: header_row.unwrap_or(0),
            data_start_row: data_start_row.unwrap_or(1),
            delimiter: resolve_delimiter(self.delimiter),
            string_quote: resolve_string_quote(self.quote),
            merge_delimiter: self.merge_delimiter,
            fixed_width: fixed_width.unwrap_or_default(),
            x_proportion: x_proportion.unwrap_or(1.0),
            epochtime_col: epochtime_col.unwrap_or(0),
            epochtime_proportion,
        }
    }

    /// What reading the header row needs, if those fields are valid.
    fn header_request(&self) -> Option<(HeaderKey, GroupConfig)> {
        let path = self.path.trim();
        let header_row: usize = self.header_row.trim().parse().ok()?;
        if path.is_empty() || header_row == 0 {
            return None;
        }
        let mut ignored = Vec::new();
        let fixed_width = number_list(&mut ignored, String::new(), &self.fixed_width, "")?;
        let key = HeaderKey {
            path: PathBuf::from(path),
            header_row,
            delimiter: self.delimiter,
            quote: self.quote,
            merge_delimiter: self.merge_delimiter,
            fixed_width: fixed_width.clone(),
        };
        let mut cfg = self.build(0, &mut ignored);
        cfg.csv_path = key.path.clone();
        cfg.header_row = header_row;
        cfg.fixed_width = fixed_width;
        Some((key, cfg))
    }
}

/// A comma-separated field as a list ("" = empty list).
fn list(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').map(String::from).collect()
    }
}

/// Parse a number field; a value that does not parse or fails `valid` is a
/// problem of field `key`.
fn number<T: std::str::FromStr + Copy>(
    problems: &mut Vec<Problem>,
    key: impl Into<String>,
    text: &str,
    name: &str,
    valid: impl Fn(T) -> bool,
) -> Option<T> {
    match text.trim().parse::<T>() {
        Ok(v) if valid(v) => Some(v),
        Ok(_) => {
            problems.push((key.into(), format!("{name}: {} is out of range", text.trim())));
            None
        }
        Err(_) => {
            problems.push((key.into(), format!("{name}: \"{}\" is not a number", text.trim())));
            None
        }
    }
}

/// Comma-separated widths, each at least 1 ("" = none).
fn number_list(
    problems: &mut Vec<Problem>,
    key: impl Into<String>,
    text: &str,
    name: &str,
) -> Option<Vec<usize>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let values: Option<Vec<usize>> = text
        .split(',')
        .map(|s| s.trim().parse::<usize>().ok().filter(|&v| v >= 1))
        .collect();
    if values.is_none() {
        problems.push((key.into(), format!("{name}: whole numbers of 1 or more, comma separated")));
    }
    values
}

/// The first line of a clap error, without its "error: " prefix.
fn clap_message(e: &clap::Error) -> String {
    let text = e.to_string();
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.trim_start_matches("error: ").to_string()
}

fn problem<'a>(problems: &'a [Problem], key: &str) -> Option<&'a str> {
    problems.iter().find(|(k, _)| k == key).map(|(_, m)| m.as_str())
}

/// The command line that starts these settings: the running executable,
/// file paths made absolute so it works from any directory.
fn command_line_for(cli: &Cli, groups: &[GroupConfig]) -> Result<String, String> {
    let groups: Vec<GroupConfig> = groups
        .iter()
        .map(|g| {
            let mut g = g.clone();
            if let Ok(path) = std::path::absolute(&g.csv_path) {
                g.csv_path = path;
            }
            g
        })
        .collect();
    let argv = cmdline::to_argv(cli, &groups)?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "tail-f_plot.exe".into());
    Ok(cmdline::command_line(&exe, &argv))
}

/// What a header row read depends on.
#[derive(Clone, PartialEq, Eq, Hash)]
struct HeaderKey {
    path: PathBuf,
    header_row: usize,
    delimiter: &'static str,
    quote: &'static str,
    merge_delimiter: bool,
    fixed_width: Vec<usize>,
}

/// The header row of one `HeaderKey`: the last result, refreshed every
/// `HEADER_REFRESH` seconds (the file may appear, get its header, or be
/// replaced by one with other columns while the window is open).
#[derive(Default)]
struct Header {
    /// None until the first read finished
    result: Option<Result<Vec<String>, String>>,
    /// When `result` arrived
    read_at: f64,
    /// The read in flight; `result` stays shown meanwhile
    reading: Option<mpsc::Receiver<Result<Vec<String>, String>>>,
}

/// Files tab state: selection, header rows read so far, an open file dialog.
#[derive(Default)]
struct Files {
    selected: usize,
    headers: HashMap<HeaderKey, Header>,
    /// When the header read in flight started (one read at a time)
    reading_since: Option<f64>,
    /// File dialog for group `.0`
    browse: Option<(usize, mpsc::Receiver<Option<String>>)>,
}

impl Files {
    /// Collect finished header reads and a finished file dialog.
    fn poll(&mut self, groups: &mut [GroupDraft], now: f64) {
        for header in self.headers.values_mut() {
            let Some(rx) = &header.reading else { continue };
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => Err("could not be read".into()),
            };
            header.result = Some(result);
            header.read_at = now;
            header.reading = None;
        }
        if !self.reading() {
            self.reading_since = None;
        }
        if let Some((gi, rx)) = &self.browse {
            match rx.try_recv() {
                Ok(picked) => {
                    if let (Some(path), Some(g)) = (picked, groups.get_mut(*gi)) {
                        g.path = path;
                    }
                    self.browse = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.browse = None,
            }
        }
    }

    fn reading(&self) -> bool {
        self.headers.values().any(|h| h.reading.is_some())
    }

    /// The header row for `key`, read in the background when not read yet
    /// or older than `HEADER_REFRESH` (one read at a time, unless the one in
    /// flight hangs).
    fn header(&mut self, key: HeaderKey, cfg: GroupConfig, ctx: &egui::Context, now: f64) -> &Header {
        let may_start = self.reading_since.is_none_or(|t| now - t > HEADER_READ_PATIENCE);
        let header = self.headers.entry(key).or_default();
        let due = header.result.is_none() || now - header.read_at >= HEADER_REFRESH;
        if header.reading.is_none() && due && may_start {
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("header-read".into()).spawn(move || {
                let names = crate::csv_watcher::read_header(&cfg).map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => "file not found".to_string(),
                    _ => e.to_string(),
                });
                let _ = tx.send(names);
                ctx.request_repaint_of(viewport_id());
            });
            if spawned.is_ok() {
                header.reading = Some(rx);
                self.reading_since = Some(now);
            }
        }
        header
    }
}

/// State shared with the viewport's UI callback, which runs whenever the
/// settings window repaints.
struct Shared {
    tab: Tab,
    draft: Draft,
    files: Files,
    /// What a draft starts from for the fields the window does not edit
    base: Cli,
    /// The draft as opened or last applied. Changed = differs from it: the
    /// settings in effect may be ones the window shows tidied up (e.g. more
    /// inherited names than columns), which must not count as a change.
    applied: Draft,
    /// Settings to apply, taken by the main window
    apply: Option<(Cli, Vec<GroupConfig>)>,
    /// OK, Cancel, Esc, the close button or Alt+F4
    close: bool,
    /// `--monitor` choices
    monitors: Vec<String>,
    /// The settings window (owner of the file dialog), once identified
    hwnd: Option<isize>,
    toast_until: Option<f64>,
    /// `--oneshot-settings`: count frames and hand over captures
    oneshot: bool,
    frames: u32,
    /// Screenshots delivered to this window's input. eframe hands every
    /// pending capture to whichever window runs next, so these can be the
    /// main window's too: sorted out by viewport id.
    captures: Vec<(ViewportId, Arc<egui::ColorImage>)>,
}

impl Shared {
    fn new(cli: Cli, groups: Vec<GroupConfig>, tab: Tab, oneshot: bool) -> Self {
        Self {
            tab,
            draft: Draft::new(&cli, &groups),
            applied: Draft::new(&cli, &groups),
            files: Files::default(),
            base: cli,
            apply: None,
            close: false,
            monitors: crate::monitor_labels(),
            hwnd: None,
            toast_until: None,
            oneshot,
            frames: 0,
            captures: Vec::new(),
        }
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A panic inside the UI callback must not make every later pass panic
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// Screenshots among a window's input events, with the window they show.
pub fn screenshot_events(events: &[egui::Event]) -> Vec<(ViewportId, Arc<egui::ColorImage>)> {
    events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Screenshot { viewport_id, image, .. } => {
                Some((*viewport_id, Arc::clone(image)))
            }
            _ => None,
        })
        .collect()
}

pub struct SettingsWindow {
    icon: Arc<egui::IconData>,
    open: Option<Open>,
}

struct Open {
    shared: Arc<Mutex<Shared>>,
    /// Main-window passes since the window was requested
    passes: u32,
    /// `--oneshot-settings`: shown at once without focus, no modality
    oneshot: bool,
    #[cfg(windows)]
    modal: win32::Modal,
}

impl SettingsWindow {
    pub fn new(icon: Arc<egui::IconData>) -> Self {
        Self { icon, open: None }
    }

    /// Open the window on `cli` / `groups`, the settings in effect (no-op
    /// while it is open). `root_hwnd` is the main window; its thread's
    /// windows are listed now, before the settings window exists, so the
    /// new one can be told apart. `oneshot_tab` = `--oneshot-settings`.
    pub fn open(
        &mut self,
        ctx: &egui::Context,
        root_hwnd: Option<isize>,
        cli: Cli,
        groups: Vec<GroupConfig>,
        oneshot_tab: Option<&str>,
    ) {
        if self.open.is_some() {
            return;
        }
        #[cfg(not(windows))]
        let _ = root_hwnd;
        let tab = oneshot_tab.map(Tab::from_name).unwrap_or_default();
        let oneshot = oneshot_tab.is_some();
        self.open = Some(Open {
            shared: Arc::new(Mutex::new(Shared::new(cli, groups, tab, oneshot))),
            passes: 0,
            oneshot,
            #[cfg(windows)]
            modal: win32::Modal::new(root_hwnd),
        });
        ctx.request_repaint();
    }

    /// Show the window and keep the modal state; call from `App::logic`
    /// every pass. Returns settings the user applied (Apply or OK).
    pub fn logic(&mut self, ctx: &egui::Context) -> Option<(Cli, Vec<GroupConfig>)> {
        let open = self.open.as_mut()?;
        let (apply, close) = {
            let mut s = lock(&open.shared);
            (s.apply.take(), s.close)
        };
        if close {
            // Enable the main window before the settings window goes away,
            // or Windows activates another application's window instead.
            #[cfg(windows)]
            open.modal.release();
            self.open = None;
            // Not showing the viewport in this pass closes it
            return apply;
        }
        #[cfg(windows)]
        if !open.oneshot {
            open.modal.step(ctx, viewport_id(), open.passes);
            if let Some(hwnd) = open.modal.window() {
                lock(&open.shared).hwnd = Some(hwnd);
            }
        }
        open.passes = open.passes.saturating_add(1);

        // Same builder every pass: a changed one recreates the window
        let builder = ViewportBuilder::default()
            .with_title("Settings - tail-f_plot")
            .with_icon(Arc::clone(&self.icon))
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            // Windows: hidden until owned and centered (see win32::Modal);
            // a one-shot capture shows it at once, without taking focus
            .with_visible(open.oneshot || !cfg!(windows))
            .with_active(!open.oneshot);
        let shared = Arc::clone(&open.shared);
        ctx.show_viewport_deferred(viewport_id(), builder, move |ui, class| {
            settings_ui(ui, class, &shared);
        });
        apply
    }

    /// `--oneshot-settings`: the window has rendered enough to be captured,
    /// and no header row is still being read.
    pub fn ready_for_screenshot(&self) -> bool {
        self.open.as_ref().is_some_and(|o| {
            let s = lock(&o.shared);
            s.frames >= ONESHOT_FRAMES && !s.files.reading()
        })
    }

    /// Capture the window at its next paint, which is requested too (an idle
    /// deferred window does not paint when the main window does).
    pub fn request_screenshot(&self, ctx: &egui::Context) {
        if self.open.is_some() {
            let cmd = ViewportCommand::Screenshot(egui::UserData::default());
            ctx.send_viewport_cmd_to(viewport_id(), cmd);
            ctx.request_repaint_of(viewport_id());
        }
    }

    /// Screenshots this window received (of any window, see `Shared`).
    pub fn take_captures(&self) -> Vec<(ViewportId, Arc<egui::ColorImage>)> {
        self.open
            .as_ref()
            .map(|o| std::mem::take(&mut lock(&o.shared).captures))
            .unwrap_or_default()
    }
}

fn settings_ui(ui: &mut egui::Ui, _class: ViewportClass, shared: &Mutex<Shared>) {
    let mut guard = lock(shared);
    let s = &mut *guard;
    let now = ui.input(|i| i.time);
    // Esc closes the window like Cancel, except when it closes an open
    // combo popup (egui closes the popup on the same, unconsumed key)
    let popup_open = egui::Popup::is_any_open(ui.ctx());
    let mut close = ui.input(|i| {
        i.viewport().close_requested() || (i.key_pressed(egui::Key::Escape) && !popup_open)
    });

    s.frames = s.frames.saturating_add(1);
    if s.oneshot {
        let captures = ui.input(|i| screenshot_events(&i.raw.events));
        if !captures.is_empty() {
            s.captures.extend(captures);
            ui.ctx().request_repaint_of(ViewportId::ROOT);
        }
        if s.frames < ONESHOT_FRAMES {
            ui.ctx().request_repaint();
        }
    }
    s.files.poll(&mut s.draft.groups, now);

    let built = s.draft.build(&s.base);
    let changed = s.draft != s.applied;
    let toast = s.toast_until.is_some_and(|t| now < t);
    if let Some(t) = s.toast_until.filter(|_| toast) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(t - now));
    }

    // Panels are not resizable: a resizable panel starts from last frame's
    // size, so content that fills it would grow it on every repaint.
    let mut apply = false;
    let mut ok = false;
    egui::Panel::bottom("settings_buttons").resizable(false).show_inside(ui, |ui| {
        ui.add_space(4.0);
        // One row: a bare right-to-left layout would take the whole panel
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right to left: reads OK, Cancel, Apply
                apply = ui.add_enabled(built.is_ok() && changed, egui::Button::new("Apply")).clicked();
                close |= ui.button("Cancel").clicked();
                ok = ui.add_enabled(built.is_ok(), egui::Button::new("OK")).clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    status_line(ui, &built, changed, toast);
                });
            });
        });
        ui.add_space(4.0);
    });
    let no_problems = Vec::new();
    let problems = built.as_ref().err().unwrap_or(&no_problems);
    egui::Panel::top("settings_tabs").resizable(false).show_inside(ui, |ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                // A tab with a problem in it shows in red
                let mut label = egui::RichText::new(tab.label());
                if problems.iter().any(|(key, _)| Tab::of_field(key) == Some(tab)) {
                    label = label.color(ui.visuals().error_fg_color);
                }
                ui.selectable_value(&mut s.tab, tab, label);
            }
        });
        ui.add_space(2.0);
    });
    egui::CentralPanel::default().show_inside(ui, |ui| {
        ui.add_space(6.0);
        match s.tab {
            Tab::Display => display_tab(ui, &mut s.draft, problems),
            Tab::Files => files_tab(ui, &mut s.draft.groups, &mut s.files, problems, s.hwnd, now),
            Tab::Window => window_tab(ui, &mut s.draft, problems, &s.monitors),
            Tab::CommandLine => {
                if command_line_tab(ui, &built) {
                    s.toast_until = Some(now + TOAST_SECONDS);
                }
            }
        }
    });
    if s.files.reading() || s.files.browse.is_some() {
        // Finished reads and dialogs repaint this window; this is a fallback
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }

    // Apply / OK act on the draft as it is now: the fields above may have
    // changed it in this same pass (input that queued up while the UI thread
    // was busy), after `built` was made.
    let mut close_ok = false;
    if apply || ok {
        if let Ok(b) = s.draft.build(&s.base) {
            if s.draft != s.applied {
                s.apply = Some((b.cli, b.groups));
                s.applied = s.draft.clone();
                ui.ctx().request_repaint_of(ViewportId::ROOT);
            }
            close_ok = ok;
        }
    }
    if (close_ok || close) && !s.close {
        s.close = true;
        ui.ctx().request_repaint_of(ViewportId::ROOT);
    }
}

/// Left of the buttons: the first problem, the copy toast, or a note that
/// changes are not applied yet.
fn status_line(ui: &mut egui::Ui, built: &Result<Built, Vec<Problem>>, changed: bool, toast: bool) {
    match built {
        Err(problems) => {
            let mut text = problems[0].1.clone();
            if problems.len() > 1 {
                text.push_str(&format!(" (+{} more)", problems.len() - 1));
            }
            let color = ui.visuals().error_fg_color;
            ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
        }
        Ok(_) if toast => {
            ui.label(egui::RichText::new("Copied to clipboard").color(ui.visuals().warn_fg_color));
        }
        Ok(_) if changed => {
            ui.weak("Changes not applied yet");
        }
        Ok(_) => {}
    }
}

/// A text field with `help` as hover text, tinted red with its problem
/// shown first when it has one. Sized explicitly: in a grid cell a text
/// field only grows up to the column's width of the previous frame, so a
/// desired width never takes.
fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    problem: Option<&str>,
    hint: &str,
    help: &str,
) {
    let mut edit = egui::TextEdit::singleline(text).hint_text(hint);
    if problem.is_some() {
        edit = edit.background_color(ui.visuals().error_fg_color.gamma_multiply(0.25));
    }
    let response = ui.add_sized([width, ui.spacing().interact_size.y], edit);
    match problem {
        Some(problem) => response.on_hover_text(format!("{problem}\n\n{help}")),
        None => response.on_hover_text(help),
    };
}

/// A field label with the field's help as hover text.
fn label(ui: &mut egui::Ui, text: &str, help: &str) {
    ui.label(text).on_hover_text(help);
}

/// One grid row: label, field, hint.
#[allow(clippy::too_many_arguments)]
fn field_row(
    ui: &mut egui::Ui,
    text_label: &str,
    text: &mut String,
    width: f32,
    key: &str,
    hint: &str,
    help: &str,
    problems: &[Problem],
) {
    label(ui, text_label, help);
    text_field(ui, text, width, problem(problems, key), "", help);
    ui.weak(hint);
    ui.end_row();
}

fn display_tab(ui: &mut egui::Ui, d: &mut Draft, problems: &[Problem]) {
    egui::Grid::new("display_grid")
        .num_columns(3)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            let (h, p) = (help::X_AXIS_LABEL, problems);
            field_row(ui, "X axis label", &mut d.x_axis_label, WIDE, "", "", h, p);
            let hint = "shown after the label";
            field_row(ui, "X unit", &mut d.x_unit, WIDE, "", hint, help::X_UNIT, p);
            field_row(ui, "Y unit", &mut d.y_unit, WIDE, "", "Y axis label", help::Y_UNIT, p);
            let hint = "comma separated, overrides Y unit";
            field_row(ui, "Y unit per stack", &mut d.y_units, WIDE, "", hint, help::Y_UNITS, p);
            let hint = "#RRGGBB, comma separated; empty = default";
            field_row(ui, "Series colors", &mut d.colors, WIDE, "colors", hint, help::COLORS, p);
            let h = help::LINE_WIDTH;
            field_row(ui, "Line width", &mut d.line_width, NARROW, "line_width", "", h, p);
            let h = help::MARKER_RADIUS;
            field_row(ui, "Marker radius", &mut d.marker_radius, NARROW, "marker_radius", "", h, p);
            let (hint, h) = ("newest rows read and kept per file", help::MAX_POINTS);
            field_row(ui, "Max points", &mut d.max_points, NARROW, "max_points", hint, h, p);
            let (hint, h) = ("in displayed X units", help::MAX_X_RANGE);
            field_row(ui, "Max X range", &mut d.max_x_range, NARROW, "max_x_range", hint, h, p);
            let (text, h) = (&mut d.repaint_interval_ms, help::REPAINT);
            field_row(ui, "Repaint interval", text, NARROW, "repaint", "milliseconds", h, p);
        });
    ui.add_space(10.0);
    ui.checkbox(&mut d.log_y, "Log scale on Y").on_hover_text(help::LOG_Y);
    ui.checkbox(&mut d.absolute_x, "Absolute X (no origin subtraction)")
        .on_hover_text(help::ABSOLUTE_X);
}

/// Name of a file group in the list: its file name, else the path as typed.
fn short_name(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return "(no file)".into();
    }
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn files_tab(
    ui: &mut egui::Ui,
    groups: &mut Vec<GroupDraft>,
    files: &mut Files,
    problems: &[Problem],
    hwnd: Option<isize>,
    now: f64,
) {
    files.selected = files.selected.min(groups.len().saturating_sub(1));
    let error = ui.visuals().error_fg_color;
    ui.horizontal_top(|ui| {
        // Left: the file groups, in command-line order
        ui.vertical(|ui| {
            ui.set_width(LIST_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt("file_list")
                .max_height(LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, g) in groups.iter().enumerate() {
                        let mut label = egui::RichText::new(format!("{}  {}", i + 1, short_name(&g.path)));
                        let prefix = format!("file{i}:");
                        if problems.iter().any(|(key, _)| key.starts_with(&prefix)) {
                            label = label.color(error);
                        }
                        let item = ui.selectable_label(files.selected == i, label);
                        if item.on_hover_text(help::FILE_LIST).clicked() {
                            files.selected = i;
                        }
                    }
                });
            ui.add_space(4.0);
            let sel = files.selected;
            ui.horizontal(|ui| {
                if ui.button("Add").on_hover_text(help::ADD_FILE).clicked() {
                    let copy = groups[sel].clone();
                    groups.insert(sel + 1, copy);
                    files.selected = sel + 1;
                }
                let remove = ui.add_enabled(groups.len() > 1, egui::Button::new("Remove"));
                if remove.on_hover_text(help::REMOVE_FILE).clicked() {
                    groups.remove(sel);
                    files.selected = sel.min(groups.len() - 1);
                }
            });
            ui.horizontal(|ui| {
                let up = ui.add_enabled(sel > 0, egui::Button::new("Up"));
                if up.on_hover_text(help::UP).clicked() {
                    groups.swap(sel, sel - 1);
                    files.selected = sel - 1;
                }
                let down = ui.add_enabled(sel + 1 < groups.len(), egui::Button::new("Down"));
                if down.on_hover_text(help::DOWN).clicked() {
                    groups.swap(sel, sel + 1);
                    files.selected = sel + 1;
                }
            });
            ui.add_space(4.0);
            ui.weak("The first file sets the shared X origin and epoch.");
        });
        ui.separator();
        // Right: the selected file group
        ui.vertical(|ui| {
            let gi = files.selected;
            group_form(ui, gi, &mut groups[gi], files, problems, hwnd, now);
        });
    });
}

fn group_form(
    ui: &mut egui::Ui,
    gi: usize,
    g: &mut GroupDraft,
    files: &mut Files,
    problems: &[Problem],
    hwnd: Option<isize>,
    now: f64,
) {
    let key = |field: &str| format!("file{gi}:{field}");
    let p = |field: &str| problem(problems, &key(field));
    #[cfg(not(windows))]
    let _ = hwnd;

    // The header row's names, for the column pickers. Read again every
    // HEADER_REFRESH seconds while shown, also without input.
    let (names, header_note) = match g.header_request() {
        None => (Vec::new(), "No header row".to_string()),
        Some((hkey, cfg)) => {
            let refresh = std::time::Duration::from_secs_f64(HEADER_REFRESH);
            ui.ctx().request_repaint_after(refresh);
            match &files.header(hkey, cfg, ui.ctx(), now).result {
                None => (Vec::new(), "Reading the header row\u{2026}".to_string()),
                Some(Ok(names)) if names.is_empty() => {
                    (Vec::new(), "The header row is empty or not written yet".to_string())
                }
                Some(Ok(names)) => {
                    let note = format!("Header row: {} columns", names.len());
                    (names.clone(), note)
                }
                Some(Err(e)) => (Vec::new(), format!("Header row: {e}")),
            }
        }
    };

    ui.horizontal(|ui| {
        label(ui, "File", help::PATH);
        text_field(ui, &mut g.path, PATH_WIDTH, p("path"), "path of the CSV file", help::PATH);
        #[cfg(windows)]
        {
            let browse = ui.add_enabled(files.browse.is_none(), egui::Button::new("Browse\u{2026}"));
            if browse.on_hover_text(help::BROWSE).clicked() {
                let initial = std::path::absolute(g.path.trim())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                files.browse = Some((gi, win32::pick_file(hwnd, &initial, ui.ctx().clone())));
            }
        }
    });
    ui.weak(header_note).on_hover_text(help::HEADER_ROW);
    ui.add_space(6.0);

    egui::Grid::new(("file_grid", gi))
        .num_columns(4)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let h = help::HEADER_ROW;
            label(ui, "Header row", h);
            text_field(ui, &mut g.header_row, NUM, p("header_row"), "0 = none", h);
            let h = help::DATA_START_ROW;
            label(ui, "Data start row", h);
            text_field(ui, &mut g.data_start_row, NUM, p("data_start_row"), "", h);
            ui.end_row();

            label(ui, "X column", help::X_COL);
            ui.horizontal(|ui| {
                column_field(ui, ("x_col", gi), &mut g.x_col, &names, p("x_col"), help::X_COL);
            });
            label(ui, "X scale", help::X_SCALE);
            ui.horizontal(|ui| {
                let selected = match g.x_scale {
                    Some(name) => X_SCALES.iter().find(|s| s.0 == name).map_or(name, |s| s.1),
                    None => "custom factor",
                };
                egui::ComboBox::from_id_salt(("x_scale", gi))
                    .width(110.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (name, label) in X_SCALES {
                            ui.selectable_value(&mut g.x_scale, Some(name), label);
                        }
                        ui.selectable_value(&mut g.x_scale, None, "custom factor");
                    })
                    .response
                    .on_hover_text(help::X_SCALE);
                if g.x_scale.is_none() {
                    let h = help::X_FACTOR;
                    text_field(ui, &mut g.x_proportion, 70.0, p("x_proportion"), "factor", h);
                }
            });
            ui.end_row();

            label(ui, "Delimiter", help::DELIMITER);
            choice(ui, ("delimiter", gi), &mut g.delimiter, &DELIMITERS, help::DELIMITER);
            label(ui, "Quote", help::QUOTE);
            choice(ui, ("quote", gi), &mut g.quote, &QUOTES, help::QUOTE);
            ui.end_row();

            let h = help::FIXED_WIDTH;
            label(ui, "Fixed widths", h);
            text_field(ui, &mut g.fixed_width, 110.0, p("fixed_width"), "e.g. 8,12,12", h);
            ui.label("");
            ui.checkbox(&mut g.merge_delimiter, "Merge delimiters").on_hover_text(help::MERGE);
            ui.end_row();

            let h = help::EPOCH_COL;
            label(ui, "Epoch column", h);
            text_field(ui, &mut g.epochtime_col, NUM, p("epochtime_col"), "0 = off", h);
            let h = help::EPOCH_FACTOR;
            label(ui, "Epoch factor", h);
            let text = &mut g.epochtime_proportion;
            text_field(ui, text, 110.0, p("epochtime_proportion"), "same as X", h);
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.strong("Y columns");
        if let Some(msg) = p("columns") {
            ui.colored_label(ui.visuals().error_fg_color, msg);
        }
    });
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt(("columns", gi))
        .max_height(SHEET_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("columns_grid", gi))
                .num_columns(4)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.weak("Column").on_hover_text(help::Y_COL);
                    ui.weak("Legend name").on_hover_text(help::NAME);
                    ui.weak("Stack").on_hover_text(help::STACK);
                    ui.end_row();
                    for (i, c) in g.columns.iter_mut().enumerate() {
                        let col_problem = problem(problems, &key(&format!("col{i}")));
                        ui.horizontal(|ui| {
                            let id = ("col", gi, i);
                            column_field(ui, id, &mut c.col, &names, col_problem, help::Y_COL);
                        });
                        let name_problem = problem(problems, &key(&format!("name{i}")));
                        text_field(ui, &mut c.name, 140.0, name_problem, "default", help::NAME);
                        let stack_problem = problem(problems, &key(&format!("stack{i}")));
                        text_field(ui, &mut c.stack, 36.0, stack_problem, "1", help::STACK);
                        if ui.button("Remove").on_hover_text(help::REMOVE_COLUMN).clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(i) = remove {
        g.columns.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("Add column").on_hover_text(help::ADD_COLUMN).clicked() {
            // Next column after the last one, same stack
            let next = g.columns.last().map_or_else(
                || ColumnDraft { col: "2".into(), name: String::new(), stack: "1".into() },
                |last| ColumnDraft {
                    col: last
                        .col
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|c| c.checked_add(1))
                        .map_or(String::new(), |c| c.to_string()),
                    name: String::new(),
                    stack: last.stack.clone(),
                },
            );
            g.columns.push(next);
        }
        ui.weak("Legend names: all or none (none = default names)");
    });
}

/// A 1-based column number with a picker of the header row's names.
fn column_field(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    text: &mut String,
    names: &[String],
    problem: Option<&str>,
    help: &str,
) {
    text_field(ui, text, 40.0, problem, "", help);
    let current = text.trim().parse::<usize>().ok().and_then(|c| c.checked_sub(1));
    let shown = current.and_then(|i| names.get(i)).map_or("", String::as_str);
    ui.add_enabled_ui(!names.is_empty(), |ui| {
        egui::ComboBox::from_id_salt(id)
            .width(110.0)
            .truncate()
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    let picked = ui.selectable_label(current == Some(i), format!("{}: {}", i + 1, name));
                    if picked.clicked() {
                        *text = (i + 1).to_string();
                    }
                }
            })
            .response
            .on_hover_text("Pick the column by its name in the header row")
            .on_disabled_hover_text("No header row names to pick from");
    });
}

/// A combo box over `(name, label)` choices.
fn choice(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    value: &mut &'static str,
    choices: &[(&'static str, &'static str)],
    help: &str,
) {
    let shown = choices.iter().find(|c| c.0 == *value).map_or(*value, |c| c.1);
    egui::ComboBox::from_id_salt(id)
        .width(110.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for &(name, label) in choices {
                ui.selectable_value(value, name, label);
            }
        })
        .response
        .on_hover_text(help);
}

fn window_tab(ui: &mut egui::Ui, d: &mut Draft, problems: &[Problem], monitors: &[String]) {
    egui::Grid::new("window_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            label(ui, "Monitor", help::MONITOR);
            let selected = match d.monitor {
                None => "Default".to_string(),
                Some(i) => monitors
                    .get(i as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("{i} (not connected)")),
            };
            egui::ComboBox::from_id_salt("monitor")
                .width(WIDE)
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.monitor, None, "Default");
                    for (i, label) in monitors.iter().enumerate() {
                        ui.selectable_value(&mut d.monitor, Some(i as u32), label);
                    }
                })
                .response
                .on_hover_text(help::MONITOR);
            ui.end_row();

            label(ui, "Bounds (%)", help::BOUNDS);
            ui.horizontal(|ui| {
                for i in 0..4 {
                    label(ui, BOUND_NAMES[i], help::BOUNDS);
                    let p = problem(problems, BOUND_KEYS[i]);
                    text_field(ui, &mut d.bounds[i], 48.0, p, "", help::BOUNDS);
                }
            });
            ui.end_row();
        });
    ui.add_space(10.0);
    ui.weak(
        "Bounds are percentages of the monitor's work area. Leave all four empty \
         to keep the window's size; changing only the monitor centers the window on it.",
    );
}

/// The command line of the draft and a Copy button. Returns true when copied.
fn command_line_tab(ui: &mut egui::Ui, built: &Result<Built, Vec<Problem>>) -> bool {
    let line = match built {
        Ok(b) => command_line_for(&b.cli, &b.groups),
        Err(_) => Err("Fix the settings marked in red first.".into()),
    };
    let line = match line {
        Ok(line) => line,
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return false;
        }
    };
    ui.label("Starts tail-f_plot with these settings (file paths made absolute):");
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut line.as_str())
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
    });
    ui.add_space(6.0);
    let mut copied = false;
    ui.horizontal(|ui| {
        if ui.button("Copy").on_hover_text(help::COPY).clicked() {
            ui.ctx().copy_text(line.clone());
            copied = true;
        }
        // A .cmd file expands % even inside quotes; the cmd prompt keeps %%
        // as is, so one line cannot serve both
        if line.contains('%') {
            let button = ui.button("Copy for .cmd file").on_hover_text(help::COPY_BATCH);
            if button.clicked() {
                ui.ctx().copy_text(cmdline::for_batch_file(&line));
                copied = true;
            }
        }
    });
    ui.weak("Copy: for the cmd prompt (in PowerShell, put & in front).");
    if line.contains('%') {
        ui.weak("This line has %: in a .cmd file each % must be doubled (Copy for .cmd file).");
    }
    copied
}

#[cfg(windows)]
mod win32 {
    use std::sync::mpsc;

    use eframe::egui::{self, ViewportCommand, ViewportId};
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, FNERR_INVALIDFILENAME, OFN_EXPLORER,
        OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
        IsWindow, IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW, GWLP_HWNDPARENT,
    };

    /// Main-window passes to wait for the settings window before showing it
    /// without modality
    const GIVE_UP_PASSES: u32 = 60;

    enum State {
        /// Waiting for the viewport's window to be created
        Finding,
        /// Owned, centered and asked to show; the main window is disabled
        /// once it is visible
        Showing { child: isize },
        /// Main window disabled until `release`
        Modal { child: isize },
        /// No modality: not identified, gone, or released
        Plain,
    }

    pub struct Modal {
        root: Option<isize>,
        /// Top-level windows of the UI thread before the settings window
        before: Vec<isize>,
        state: State,
    }

    impl Modal {
        pub fn new(root: Option<isize>) -> Self {
            let before = root.map(thread_windows).unwrap_or_default();
            Self { root, before, state: State::Finding }
        }

        /// The settings window, once identified.
        pub fn window(&self) -> Option<isize> {
            match self.state {
                State::Showing { child } | State::Modal { child } => Some(child),
                _ => None,
            }
        }

        /// One main-window pass. The viewport's window is created at the end
        /// of the first pass that shows it (`passes == 0`).
        pub fn step(&mut self, ctx: &egui::Context, id: ViewportId, passes: u32) {
            let Some(root) = self.root else {
                // Main window unknown: just show it once it exists
                if matches!(self.state, State::Finding) && passes > 0 {
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
                    self.state = State::Plain;
                }
                return;
            };
            match self.state {
                State::Finding => {
                    if passes > 0 {
                        if let Some(child) = new_window(root, &self.before) {
                            own_and_center(child, root);
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                            self.state = State::Showing { child };
                        } else if passes >= GIVE_UP_PASSES {
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
                            self.state = State::Plain;
                        }
                    }
                    ctx.request_repaint();
                }
                State::Showing { child } => {
                    // Disable the main window only once the settings window
                    // shows: disabling the active window first would hand
                    // activation to another application.
                    if unsafe { IsWindowVisible(child as HWND) } != 0 {
                        unsafe { EnableWindow(root as HWND, 0) };
                        self.state = State::Modal { child };
                    } else if unsafe { IsWindow(child as HWND) } == 0
                        || passes >= 2 * GIVE_UP_PASSES
                    {
                        self.state = State::Plain;
                    } else {
                        ctx.request_repaint();
                    }
                }
                State::Modal { child } => {
                    // Never leave the main window disabled without its dialog
                    if unsafe { IsWindow(child as HWND) } == 0 {
                        self.release();
                    }
                }
                State::Plain => {}
            }
        }

        /// Enable the main window again; call before the settings window
        /// closes.
        pub fn release(&mut self) {
            if let (State::Modal { child }, Some(root)) = (&self.state, self.root) {
                unsafe {
                    EnableWindow(root as HWND, 1);
                    // While the dialog has the foreground, hand it to the main
                    // window: left to Windows, the next window in z-order gets
                    // it, which can be another application's window.
                    if GetForegroundWindow() == *child as HWND {
                        SetForegroundWindow(root as HWND);
                    }
                }
            }
            self.state = State::Plain;
        }
    }

    impl Drop for Modal {
        fn drop(&mut self) {
            self.release();
        }
    }

    /// Top-level windows (shown or not) of the main window's thread
    fn thread_windows(root: isize) -> Vec<isize> {
        unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> i32 {
            let list = &mut *(data as *mut Vec<isize>);
            list.push(hwnd as isize);
            1 // continue
        }
        let mut list: Vec<isize> = Vec::new();
        unsafe {
            let thread = GetWindowThreadProcessId(root as HWND, std::ptr::null_mut());
            EnumThreadWindows(thread, Some(collect), &mut list as *mut _ as LPARAM);
        }
        list
    }

    /// The settings window: the one winit window of the main window's thread
    /// that was not there before. Never guessed: with none or several new
    /// ones, nothing is touched.
    fn new_window(root: isize, before: &[isize]) -> Option<isize> {
        let found: Vec<isize> = thread_windows(root)
            .into_iter()
            .filter(|&h| h != root && !before.contains(&h) && is_winit_window(h))
            .collect();
        match found[..] {
            [one] => Some(one),
            _ => None,
        }
    }

    fn is_winit_window(hwnd: isize) -> bool {
        let mut name = [0u16; 32];
        let n = unsafe { GetClassNameW(hwnd as HWND, name.as_mut_ptr(), name.len() as i32) };
        n > 0 && String::from_utf16_lossy(&name[..n as usize]) == "Window Class"
    }

    /// Make the main window the owner (the settings window stays above it
    /// and hides with it) and center the settings window on the work area of
    /// the main window's monitor.
    fn own_and_center(child: isize, root: isize) {
        unsafe { SetWindowLongPtrW(child as HWND, GWLP_HWNDPARENT, root) };
        if let Some(work) = crate::work_area_of(root) {
            crate::center_window(child, work);
        }
    }

    /// The Open dialog for a data file, run on its own thread (it blocks
    /// until closed) and owned by the settings window, which Windows disables
    /// meanwhile. The file need not exist yet: tail-f_plot waits for it. The
    /// working directory is left alone (relative paths of other files).
    /// Receives Some(path), or None when cancelled.
    pub fn pick_file(owner: Option<isize>, initial: &str, ctx: egui::Context) -> mpsc::Receiver<Option<String>> {
        let (tx, rx) = mpsc::channel();
        let initial = initial.to_string();
        let _ = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
            let mut buf = vec![0u16; 4096];
            let init: Vec<u16> = initial.encode_utf16().take(buf.len() - 1).collect();
            buf[..init.len()].copy_from_slice(&init);
            let filter: Vec<u16> = "Data files (*.csv;*.tsv;*.txt;*.dat;*.log)\0\
                                    *.csv;*.tsv;*.txt;*.dat;*.log\0All files (*.*)\0*.*\0\0"
                .encode_utf16()
                .collect();
            let title: Vec<u16> = "Data file\0".encode_utf16().collect();
            let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
            ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
            ofn.hwndOwner = owner.unwrap_or(0) as HWND;
            ofn.lpstrFilter = filter.as_ptr();
            ofn.nFilterIndex = 1;
            ofn.lpstrFile = buf.as_mut_ptr();
            ofn.nMaxFile = buf.len() as u32;
            ofn.lpstrTitle = title.as_ptr();
            ofn.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY;
            let mut picked = unsafe { GetOpenFileNameW(&mut ofn) } != 0;
            // An initial name the dialog rejects (a pasted "quoted" path,
            // < > |) makes it fail without opening: open it empty instead
            if !picked && unsafe { CommDlgExtendedError() } == FNERR_INVALIDFILENAME {
                buf[0] = 0;
                picked = unsafe { GetOpenFileNameW(&mut ofn) } != 0;
            }
            let path = picked.then(|| {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf16_lossy(&buf[..n])
            });
            let _ = tx.send(path);
            ctx.request_repaint_of(super::viewport_id());
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::containers::panel::PanelState;

    fn parse(line: &str) -> (Cli, Vec<GroupConfig>) {
        let args = std::iter::once("tail-f_plot").chain(line.split_whitespace());
        match cmdline::parse_args(args.map(String::from).collect()) {
            Ok(ParsedArgs::Run { cli, groups }) => (cli, groups),
            _ => panic!("{line}"),
        }
    }

    /// The command line a built draft applies
    fn argv(built: &Built) -> Vec<String> {
        cmdline::to_argv(&built.cli, &built.groups).unwrap()
    }

    fn keys(result: Result<Built, Vec<Problem>>) -> Vec<String> {
        match result {
            Ok(_) => Vec::new(),
            Err(problems) => problems.into_iter().map(|(k, _)| k).collect(),
        }
    }

    /// Where a text is drawn (its galley's top-left), searching nested shapes.
    fn text_pos(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Pos2> {
        fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        shapes.iter().find_map(|c| find(&c.shape, text))
    }

    /// Esc while a combo list is open closes the list only; the next Esc
    /// closes the window, so two presses close both and no more are needed.
    /// Drives the real window UI with clicks and keys.
    #[test]
    fn esc_closes_an_open_list_first_then_the_window() {
        let (cli, groups) = parse("a.csv --y-cols=2");
        let shared = Mutex::new(Shared::new(cli, groups, Tab::Files, false));
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW_SIZE.into());
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.5;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| settings_ui(ui, ViewportClass::Deferred, &shared))
        };
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let click = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        let out = frame(Vec::new());
        let combo = text_pos(&out.shapes, "comma ( , )").expect("delimiter combo") + egui::vec2(5.0, 5.0);
        frame(vec![egui::Event::PointerMoved(combo)]);
        frame(vec![click(combo, true)]);
        frame(vec![click(combo, false)]);
        let out = frame(Vec::new());
        assert!(text_pos(&out.shapes, "tab").is_some(), "the list is open");

        // Press and release in one frame (a busy UI thread batches them)
        let mut presses = 0;
        while !lock(&shared).close && presses < 5 {
            presses += 1;
            frame(vec![key(true), key(false)]);
            frame(Vec::new());
        }
        assert_eq!(presses, 2, "one Esc for the list, one for the window");
    }

    /// Repaints (e.g. on every mouse move) must not change the layout: a
    /// panel sized from its previous frame grew whenever its content took
    /// the whole panel, squeezing the central area to nothing.
    #[test]
    fn panels_keep_their_size_across_repaints() {
        for tab in Tab::ALL {
            let (cli, groups) = parse("a.csv --y-cols=2 -- b.csv --y-cols=3,4");
            let shared = Mutex::new(Shared::new(cli, groups, tab, false));
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW_SIZE.into());
            let mut sizes = Vec::new();
            for i in 0..20 {
                let mut input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                input.events.push(egui::Event::PointerMoved(egui::pos2(100.0 + i as f32, 200.0)));
                let _ = ctx.run_ui(input, |ui| settings_ui(ui, ViewportClass::Deferred, &shared));
                let height = |id: &str| PanelState::load(&ctx, egui::Id::new(id)).unwrap().rect.height();
                sizes.push((height("settings_tabs"), height("settings_buttons")));
            }
            // The first frames may still settle (sizing pass)
            assert!(sizes[3..].windows(2).all(|w| w[0] == w[1]), "{tab:?} {sizes:?}");
            // Tabs and buttons stay one row each; the content keeps the rest
            let (tabs, buttons) = *sizes.last().unwrap();
            assert!(screen.height() - tabs - buttons > 400.0, "{tab:?} {sizes:?}");
        }
    }

    #[test]
    fn untouched_draft_writes_the_applied_command_line() {
        // (Without --y-cols the window asks for a Y column: a plot of nothing)
        for line in [
            "a.csv --y-cols=2",
            "a.csv --y-cols=2,3 --y-names=A,B -- b.csv --y-cols=4,5",
            "a.csv --y-cols=2 --log-y --absolute-x --monitor=1 --bounds=0,0,50,50.5 --colors=#ff0000,00ff00 \
             --y-unit=K --y-units=K,mbar --x-axis-label=T --x-unit= --line-width=2.25 \
             --marker-radius=0 --max-points=10 --max-x-range=0.016666666666666666 \
             --repaint-interval-ms=100",
            "a.csv --x-col=3 --y-cols=4,5,6 --y-stacks=1,2 --header-row=0 --data-start-row=7 \
             --delimiter=tab --string-quote=dquote --merge-delimiter --x-time-scale=h \
             --epochtime-col=9 --epochtime-proportion=0.001 \
             -- b.csv --fixed-width=4,8,8 --x-proportion=0.5 --y-names=P,Q,R --epochtime-proportion=0 \
             -- b.csv --y-cols=2 --y-names= --delimiter=vbar --string-quote=backtick",
        ] {
            let (cli, groups) = parse(line);
            let built = Draft::new(&cli, &groups).build(&cli).unwrap_or_else(|p| panic!("{p:?}"));
            assert_eq!(argv(&built), cmdline::to_argv(&cli, &groups).unwrap(), "{line}");
        }
    }

    /// Inherited names outnumbering a group's columns make extra, empty
    /// series on the command line; the window shows one row per column and
    /// applies that, but only together with a change of the user's.
    #[test]
    fn more_names_than_columns_shows_one_row_per_column() {
        let (cli, groups) = parse("a.csv --y-cols=2,3 --y-names=A,B -- b.csv --y-cols=4");
        let d = Draft::new(&cli, &groups);
        assert_eq!(d.groups[1].columns.len(), 1);
        let built = d.build(&cli).unwrap();
        assert_eq!(built.groups[1].y_names, ["A"]);
        // Opened as is: nothing to apply
        let shared = Shared::new(cli, groups, Tab::Files, false);
        assert!(shared.draft == shared.applied);
    }

    /// `--y-units=` is one empty unit (stack 1 without a unit), not no units
    #[test]
    fn a_single_empty_y_unit_survives_the_draft() {
        let (cli, groups) = parse("a.csv --y-cols=2 --y-unit=K --y-units=");
        assert_eq!(cli.y_units, [""]);
        let mut d = Draft::new(&cli, &groups);
        d.line_width = "2".into();
        assert_eq!(d.build(&cli).unwrap().cli.y_units, [""]);
        // Typed over, the field is read as typed
        d.y_units = "mbar".into();
        assert_eq!(d.build(&cli).unwrap().cli.y_units, ["mbar"]);
    }

    #[test]
    fn bad_fields_are_reported_on_their_own_field() {
        let (cli, groups) = parse("a.csv --y-cols=2");
        let mut d = Draft::new(&cli, &groups);
        d.line_width = "0".into();
        d.marker_radius = "-1".into();
        d.max_points = "many".into();
        d.max_x_range = "inf".into();
        d.repaint_interval_ms = "0".into();
        d.colors = "#ff0000,red".into();
        assert_eq!(
            keys(d.build(&cli)),
            ["line_width", "marker_radius", "max_points", "max_x_range", "repaint", "colors"]
        );
    }

    #[test]
    fn bounds_are_all_or_nothing_and_ordered() {
        let (cli, groups) = parse("a.csv --y-cols=2");
        let mut d = Draft::new(&cli, &groups);
        assert!(d.build(&cli).unwrap().cli.bounds.is_empty());

        d.bounds = ["10".into(), "".into(), "90".into(), "120".into()];
        assert_eq!(keys(d.build(&cli)), ["bounds_top", "bounds_bottom"]);

        d.bounds = ["60".into(), "50".into(), "40".into(), "50".into()];
        assert_eq!(keys(d.build(&cli)), ["bounds_right", "bounds_bottom"]);

        d.bounds = ["0".into(), "0".into(), "50".into(), "100".into()];
        assert_eq!(d.build(&cli).unwrap().cli.bounds, [0.0, 0.0, 50.0, 100.0]);
    }

    #[test]
    fn a_changed_field_changes_the_command_line() {
        let (cli, groups) = parse("a.csv --y-cols=2");
        let mut d = Draft::new(&cli, &groups);
        d.max_points = "100".into();
        d.log_y = true;
        let built = d.build(&cli).unwrap();
        assert_eq!(argv(&built), ["a.csv", "--y-cols=2", "--log-y", "--max-points=100"]);
        assert!(built.cli.log_y);
        assert_eq!(built.cli.max_points, 100);
    }

    #[test]
    fn file_group_edits_reach_the_command_line() {
        let (cli, groups) = parse("a.csv --y-cols=2");
        let mut d = Draft::new(&cli, &groups);
        let mut b = d.groups[0].clone();
        b.path = "b.csv".into();
        b.columns = vec![
            ColumnDraft { col: "3".into(), name: "T".into(), stack: "".into() },
            ColumnDraft { col: "4".into(), name: "P".into(), stack: "2".into() },
        ];
        b.x_scale = None;
        b.x_proportion = "0.5".into();
        d.groups.push(b);
        let built = d.build(&cli).unwrap();
        assert_eq!(
            argv(&built),
            ["a.csv", "--y-cols=2", "--", "b.csv", "--y-cols=3,4", "--y-names=T,P", "--y-stacks=1,2", "--x-proportion=0.5"]
        );
        assert_eq!(built.groups.len(), 2);
        assert_eq!(built.groups[1].y_names, ["T", "P"]);
    }

    #[test]
    fn file_group_problems_point_at_their_field() {
        let (cli, groups) = parse("a.csv --y-cols=2,3 -- b.csv");
        let mut d = Draft::new(&cli, &groups);
        let g = &mut d.groups[1];
        g.path = " ".into();
        g.x_col = "0".into();
        g.data_start_row = "".into();
        g.fixed_width = "4,0".into();
        g.x_scale = None;
        g.x_proportion = "0".into();
        g.epochtime_proportion = "nan".into();
        g.columns[0].name = "A".into(); // the other column has no name
        g.columns[1].stack = "x".into();
        assert_eq!(
            keys(d.build(&cli)),
            [
                "file1:path",
                "file1:x_col",
                "file1:data_start_row",
                "file1:fixed_width",
                "file1:x_proportion",
                "file1:epochtime_proportion",
                "file1:stack1",
                "file1:name1",
            ]
        );

        d.groups[1].columns.clear();
        assert!(keys(d.build(&cli)).contains(&"file1:columns".to_string()));
    }
}
