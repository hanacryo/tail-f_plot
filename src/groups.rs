//! Multi-file argument groups.
//!
//! The command line is split on literal `--` tokens. The first segment is the
//! normal `Cli` (global window/render options + the first file's options).
//! Each later segment is a `GroupCli`: every field is optional, and omitted
//! fields inherit from the previous group's resolved `GroupConfig`.

use clap::Parser;
use std::path::PathBuf;

use crate::{resolve_delimiter, resolve_string_quote, Cli};

/// Split full argv on literal `--` tokens. The first segment keeps argv[0];
/// later segments are parsed with `no_binary_name`. Empty segments (e.g. a
/// trailing `--`) are kept here and skipped by the caller.
pub fn split_arg_groups(args: Vec<String>) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = vec![Vec::new()];
    for a in args {
        if a == "--" {
            groups.push(Vec::new());
        } else {
            groups.last_mut().unwrap().push(a);
        }
    }
    groups
}

/// Map `--x-time-scale` name to a proportion; unknown/absent falls back.
pub fn resolve_x_proportion(time_scale: Option<&str>, fallback: f64) -> f64 {
    match time_scale {
        Some("d") => 1.0 / 86400.0,
        Some("h") => 1.0 / 3600.0,
        Some("m") => 1.0 / 60.0,
        Some("s") => 1.0,
        _ => fallback,
    }
}

/// Per-group arguments after a `--` separator. `None` = inherit from the
/// previous group. Global options (window, colors, axes labels, ...) are not
/// accepted here — clap reports them as unexpected arguments.
#[derive(Parser, Debug, Default)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
pub struct GroupCli {
    /// CSV file path (omit to reuse the previous group's file)
    csv_path: Option<PathBuf>,

    /// X-axis column (1-based)
    #[arg(long)]
    x_col: Option<usize>,

    /// Y-axis columns (1-based, comma separated)
    #[arg(long, value_delimiter = ',')]
    y_cols: Option<Vec<usize>>,

    /// Y-axis legend names (comma separated)
    #[arg(long, value_delimiter = ',')]
    y_names: Option<Vec<String>>,

    /// Stack index per Y column (1-based, comma separated)
    #[arg(long, value_delimiter = ',')]
    y_stacks: Option<Vec<usize>>,

    /// Header row number (1-based)
    #[arg(long)]
    header_row: Option<usize>,

    /// Data start row number (1-based)
    #[arg(long)]
    data_start_row: Option<usize>,

    /// Field delimiter: comma, tab, semicolon, space, vbar
    #[arg(long)]
    delimiter: Option<String>,

    /// String quote character: none, squote, dquote, backtick
    #[arg(long)]
    string_quote: Option<String>,

    /// Merge consecutive delimiters (`--merge-delimiter=false` to reset)
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    merge_delimiter: Option<bool>,

    /// Fixed-width column widths (`--fixed-width 0` to reset to delimiter mode)
    #[arg(long, value_delimiter = ',')]
    fixed_width: Option<Vec<usize>>,

    /// X-axis time scale: d, h, m, s (overrides --x-proportion)
    #[arg(long)]
    x_time_scale: Option<String>,

    /// X-value proportional constant (multiplied to raw X)
    #[arg(long)]
    x_proportion: Option<f64>,

    /// Epoch-time column (1-based) for cross-file alignment (0 = disabled)
    #[arg(long)]
    epochtime_col: Option<usize>,

    /// Proportional constant for the epoch-time column
    #[arg(long)]
    epochtime_proportion: Option<f64>,
}

/// Fully resolved per-file settings (inheritance already applied).
#[derive(Debug, Clone)]
pub struct GroupConfig {
    pub csv_path: PathBuf,
    pub x_col: usize,
    pub y_cols: Vec<usize>,
    pub y_names: Vec<String>,
    /// Stack index per Y column (1-based; missing entries default to 1)
    pub y_stacks: Vec<usize>,
    pub header_row: usize,
    pub data_start_row: usize,
    /// Resolved delimiter string (e.g. "," or "\t")
    pub delimiter: String,
    pub string_quote: Option<char>,
    pub merge_delimiter: bool,
    pub fixed_width: Vec<usize>,
    /// Resolved proportion (time scale already applied)
    pub x_proportion: f64,
    /// 1-based epoch-time column, 0 = disabled
    pub epochtime_col: usize,
    /// None = follow this group's x_proportion
    pub epochtime_proportion: Option<f64>,
}

impl GroupConfig {
    /// First group: taken from the main `Cli` (defaults already applied).
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            csv_path: cli.csv_path.clone().expect("csv_path checked in main"),
            x_col: cli.x_col,
            y_cols: cli.y_cols.clone(),
            y_names: cli.y_names.clone(),
            y_stacks: cli.y_stacks.clone(),
            header_row: cli.header_row,
            data_start_row: cli.data_start_row,
            delimiter: resolve_delimiter(&cli.delimiter),
            string_quote: resolve_string_quote(&cli.string_quote),
            merge_delimiter: cli.merge_delimiter.unwrap_or(false),
            fixed_width: cli.fixed_width.clone(),
            x_proportion: resolve_x_proportion(cli.x_time_scale.as_deref(), cli.x_proportion),
            epochtime_col: cli.epochtime_col,
            epochtime_proportion: cli.epochtime_proportion,
        }
    }

    /// Next group: fields given in `g` override, everything else inherits.
    pub fn apply(&self, g: &GroupCli) -> Self {
        // Explicit x scaling in this group wins; otherwise inherit the already
        // resolved proportion (never re-resolve the previous group's args).
        let x_proportion = if g.x_time_scale.is_some() || g.x_proportion.is_some() {
            resolve_x_proportion(
                g.x_time_scale.as_deref(),
                g.x_proportion.unwrap_or(self.x_proportion),
            )
        } else {
            self.x_proportion
        };

        Self {
            csv_path: g.csv_path.clone().unwrap_or_else(|| self.csv_path.clone()),
            x_col: g.x_col.unwrap_or(self.x_col),
            y_cols: g.y_cols.clone().unwrap_or_else(|| self.y_cols.clone()),
            y_names: g.y_names.clone().unwrap_or_else(|| self.y_names.clone()),
            y_stacks: g.y_stacks.clone().unwrap_or_else(|| self.y_stacks.clone()),
            header_row: g.header_row.unwrap_or(self.header_row),
            data_start_row: g.data_start_row.unwrap_or(self.data_start_row),
            delimiter: g
                .delimiter
                .as_deref()
                .map(resolve_delimiter)
                .unwrap_or_else(|| self.delimiter.clone()),
            string_quote: g
                .string_quote
                .as_deref()
                .map(resolve_string_quote)
                .unwrap_or(self.string_quote),
            merge_delimiter: g.merge_delimiter.unwrap_or(self.merge_delimiter),
            fixed_width: match &g.fixed_width {
                Some(w) if w.as_slice() == [0] => Vec::new(),
                Some(w) => w.clone(),
                None => self.fixed_width.clone(),
            },
            x_proportion,
            epochtime_col: g.epochtime_col.unwrap_or(self.epochtime_col),
            epochtime_proportion: g.epochtime_proportion.or(self.epochtime_proportion),
        }
    }

    /// Effective proportion for the epoch-time column.
    pub fn epoch_proportion(&self) -> f64 {
        self.epochtime_proportion.unwrap_or(self.x_proportion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn base_config() -> GroupConfig {
        GroupConfig {
            csv_path: PathBuf::from("a.csv"),
            x_col: 1,
            y_cols: vec![2, 3],
            y_names: vec!["A".into(), "B".into()],
            y_stacks: Vec::new(),
            header_row: 2,
            data_start_row: 3,
            delimiter: ",".into(),
            string_quote: None,
            merge_delimiter: false,
            fixed_width: Vec::new(),
            x_proportion: 1.0 / 60.0,
            epochtime_col: 0,
            epochtime_proportion: None,
        }
    }

    #[test]
    fn split_on_double_dash() {
        let segs = split_arg_groups(strs(&["exe", "a.csv", "--y-cols", "2", "--", "b.csv", "--", "--x-col", "4"]));
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], strs(&["exe", "a.csv", "--y-cols", "2"]));
        assert_eq!(segs[1], strs(&["b.csv"]));
        assert_eq!(segs[2], strs(&["--x-col", "4"]));
    }

    #[test]
    fn split_without_separator_is_single_segment() {
        let segs = split_arg_groups(strs(&["exe", "a.csv"]));
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn trailing_separator_yields_empty_segment() {
        let segs = split_arg_groups(strs(&["exe", "a.csv", "--"]));
        assert_eq!(segs.len(), 2);
        assert!(segs[1].is_empty());
    }

    #[test]
    fn empty_group_inherits_everything() {
        let g = GroupCli::try_parse_from(Vec::<String>::new()).unwrap();
        let next = base_config().apply(&g);
        assert_eq!(next.csv_path, PathBuf::from("a.csv"));
        assert_eq!(next.y_cols, vec![2, 3]);
        assert_eq!(next.x_proportion, 1.0 / 60.0);
    }

    #[test]
    fn new_path_keeps_other_settings() {
        let g = GroupCli::try_parse_from(strs(&["b.csv"])).unwrap();
        let next = base_config().apply(&g);
        assert_eq!(next.csv_path, PathBuf::from("b.csv"));
        assert_eq!(next.y_cols, vec![2, 3]);
        assert_eq!(next.y_names, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn y_stacks_override_and_inherit() {
        let mut base = base_config();
        base.y_stacks = vec![1, 2];

        let g = GroupCli::try_parse_from(strs(&["b.csv"])).unwrap();
        let inherited = base.apply(&g);
        assert_eq!(inherited.y_stacks, vec![1, 2]);

        let g = GroupCli::try_parse_from(strs(&["--y-stacks", "3,3"])).unwrap();
        let overridden = inherited.apply(&g);
        assert_eq!(overridden.y_stacks, vec![3, 3]);
    }

    #[test]
    fn explicit_time_scale_overrides_inherited_proportion() {
        let g = GroupCli::try_parse_from(strs(&["b.csv", "--x-time-scale", "s"])).unwrap();
        let next = base_config().apply(&g);
        assert_eq!(next.x_proportion, 1.0);
    }

    #[test]
    fn explicit_proportion_overrides_inherited() {
        let g = GroupCli::try_parse_from(strs(&["--x-proportion", "0.5"])).unwrap();
        let next = base_config().apply(&g);
        assert_eq!(next.x_proportion, 0.5);
    }

    #[test]
    fn unknown_time_scale_falls_back_to_inherited() {
        let g = GroupCli::try_parse_from(strs(&["--x-time-scale", "weeks"])).unwrap();
        let next = base_config().apply(&g);
        assert_eq!(next.x_proportion, 1.0 / 60.0);
    }

    #[test]
    fn fixed_width_zero_resets_to_delimiter_mode() {
        let mut base = base_config();
        base.fixed_width = vec![8, 8, 8];
        let g = GroupCli::try_parse_from(strs(&["--fixed-width", "0"])).unwrap();
        let next = base.apply(&g);
        assert!(next.fixed_width.is_empty());
    }

    #[test]
    fn merge_delimiter_flag_and_reset() {
        let g = GroupCli::try_parse_from(strs(&["--merge-delimiter"])).unwrap();
        let on = base_config().apply(&g);
        assert!(on.merge_delimiter);

        let g = GroupCli::try_parse_from(strs(&["--merge-delimiter=false"])).unwrap();
        let off = on.apply(&g);
        assert!(!off.merge_delimiter);
    }

    #[test]
    fn epochtime_col_zero_disables_inherited() {
        let mut base = base_config();
        base.epochtime_col = 5;
        let g = GroupCli::try_parse_from(strs(&["--epochtime-col", "0"])).unwrap();
        let next = base.apply(&g);
        assert_eq!(next.epochtime_col, 0);
    }

    #[test]
    fn epoch_proportion_follows_x_proportion_unless_explicit() {
        let cfg = base_config();
        assert_eq!(cfg.epoch_proportion(), 1.0 / 60.0);

        let g = GroupCli::try_parse_from(strs(&["--epochtime-proportion", "1.0"])).unwrap();
        let next = cfg.apply(&g);
        assert_eq!(next.epoch_proportion(), 1.0);
    }

    #[test]
    fn global_option_in_group_segment_is_rejected() {
        assert!(GroupCli::try_parse_from(strs(&["b.csv", "--log-y"])).is_err());
        assert!(GroupCli::try_parse_from(strs(&["b.csv", "--max-points", "10"])).is_err());
    }
}
