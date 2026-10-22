//! Command line <-> settings. `parse_args` turns a full argv into the global
//! options (`Cli`) plus one resolved `GroupConfig` per file; `to_argv` writes
//! settings back as the shortest argv that parses to the same settings. The
//! settings window applies through this round trip, so the command line it
//! copies is exactly what it applied.

use std::path::Path;

use clap::Parser;

use crate::groups::{
    self, epoch_proportion_arg, fixed_widths, names_or_default, resolve_x_proportion, GroupCli,
    GroupConfig,
};
use crate::{resolve_delimiter, resolve_string_quote, Cli};

/// Result of `parse_args`.
pub enum ParsedArgs {
    /// `--help`, or no CSV path (`requested` = `--help` was given)
    Help { requested: bool },
    Run { cli: Cli, groups: Vec<GroupConfig> },
}

/// Parse a full argv (program name first). The segment before the first
/// `--` is the `Cli` (global options + first file); each later segment is
/// one more file group whose omitted options inherit from the group before
/// it. An empty segment (e.g. a trailing `--`) is skipped.
pub fn parse_args(args: Vec<String>) -> Result<ParsedArgs, clap::Error> {
    let segments = groups::split_arg_groups(args);
    let cli = Cli::try_parse_from(&segments[0])?;
    if cli.help || cli.csv_path.is_none() {
        return Ok(ParsedArgs::Help { requested: cli.help });
    }
    let mut cfgs = vec![GroupConfig::from_cli(&cli)];
    for seg in &segments[1..] {
        // A fully empty group would only duplicate the previous one
        if seg.is_empty() {
            continue;
        }
        let g = GroupCli::try_parse_from(seg)?;
        let next = cfgs.last().unwrap().apply(&g);
        cfgs.push(next);
    }
    Ok(ParsedArgs::Run { cli, groups: cfgs })
}

/// The shortest argv (without the program name) that `parse_args` reads
/// back as `cli`'s global options and `groups`: the first segment writes
/// what differs from the defaults, each later group what differs from the
/// group before it. Valued options are written as `--name=value`, so a
/// value starting with `-` stays a value. `--help` and `--oneshot` are never
/// written. Err = a setting the command line cannot express.
#[allow(dead_code)] // TODO: drop once the settings window calls it
pub fn to_argv(cli: &Cli, groups: &[GroupConfig]) -> Result<Vec<String>, String> {
    let defaults = Cli::try_parse_from(["tail-f_plot", "x"]).expect("defaults parse");
    let Some((first, rest)) = groups.split_first() else {
        return Err("No file to plot".into());
    };
    let mut args = vec![path_arg(&first.csv_path, 1)?];
    group_diff(&GroupConfig::from_cli(&defaults), first, 1, &mut args)?;
    global_diff(&defaults, cli, &mut args)?;
    let mut prev = first;
    for (i, cur) in rest.iter().enumerate() {
        let n = i + 2;
        let mut options = Vec::new();
        group_diff(prev, cur, n, &mut options)?;
        args.push("--".into());
        // The parser skips an empty segment: name the file even when it is
        // the same one, so an identical group is not dropped.
        if cur.csv_path != prev.csv_path || options.is_empty() {
            args.push(path_arg(&cur.csv_path, n)?);
        }
        args.extend(options);
        prev = cur;
    }
    Ok(args)
}

/// Append the per-file options of `cur` (file number `n`, for messages)
/// that differ from `prev`: what an omitted option would inherit.
fn group_diff(
    prev: &GroupConfig,
    cur: &GroupConfig,
    n: usize,
    args: &mut Vec<String>,
) -> Result<(), String> {
    if cur.x_col != prev.x_col {
        args.push(format!("--x-col={}", cur.x_col));
    }
    if cur.y_cols != prev.y_cols {
        if cur.y_cols.is_empty() {
            return Err(format!("File {n}: no Y column"));
        }
        args.push(format!("--y-cols={}", join(&cur.y_cols)));
    }
    let names = names_or_default(&cur.y_names);
    if names != names_or_default(&prev.y_names) {
        // An empty list is written as `--y-names=`: back to default names
        let names = list(&names, || format!("File {n}: legend name"))?;
        args.push(format!("--y-names={names}"));
    }
    // Stacks by effect: one per series, a missing entry or 0 reads as 1
    let series = if names_or_default(&cur.y_names).is_empty() {
        cur.y_cols.len()
    } else {
        cur.y_names.len()
    };
    let stacks = |g: &GroupConfig| -> Vec<usize> {
        (0..series)
            .map(|i| g.y_stacks.get(i).copied().unwrap_or(1).max(1))
            .collect()
    };
    if stacks(cur) != stacks(prev) {
        args.push(format!("--y-stacks={}", join(&stacks(cur))));
    }
    if cur.header_row != prev.header_row {
        args.push(format!("--header-row={}", cur.header_row));
    }
    if cur.data_start_row != prev.data_start_row {
        args.push(format!("--data-start-row={}", cur.data_start_row));
    }
    if cur.delimiter != prev.delimiter {
        let name = ["comma", "tab", "semicolon", "space", "vbar"]
            .into_iter()
            .find(|&name| resolve_delimiter(name) == cur.delimiter)
            .ok_or_else(|| format!("File {n}: unsupported delimiter {:?}", cur.delimiter))?;
        args.push(format!("--delimiter={name}"));
    }
    if cur.string_quote != prev.string_quote {
        let name = ["none", "squote", "dquote", "backtick"]
            .into_iter()
            .find(|&name| resolve_string_quote(name) == cur.string_quote)
            .ok_or_else(|| format!("File {n}: unsupported quote {:?}", cur.string_quote))?;
        args.push(format!("--string-quote={name}"));
    }
    if cur.merge_delimiter != prev.merge_delimiter {
        args.push(format!("--merge-delimiter={}", cur.merge_delimiter));
    }
    let widths = fixed_widths(&cur.fixed_width);
    if widths != fixed_widths(&prev.fixed_width) {
        // `--fixed-width=0` = back to delimiter mode
        let value = if widths.is_empty() { "0".into() } else { join(&widths) };
        args.push(format!("--fixed-width={value}"));
    }
    if cur.x_proportion != prev.x_proportion {
        let p = finite(cur.x_proportion, || format!("File {n}: X proportion"))?;
        // A time-scale name when it is exactly one
        let scale = ["d", "h", "m", "s"]
            .into_iter()
            .find(|&name| resolve_x_proportion(Some(name), f64::NAN) == p);
        args.push(match scale {
            Some(name) => format!("--x-time-scale={name}"),
            None => format!("--x-proportion={p}"),
        });
    }
    if cur.epochtime_col != prev.epochtime_col {
        args.push(format!("--epochtime-col={}", cur.epochtime_col));
    }
    let epoch = cur.epochtime_proportion.and_then(epoch_proportion_arg);
    if epoch != prev.epochtime_proportion.and_then(epoch_proportion_arg) {
        // `--epochtime-proportion=0` = back to following the X proportion
        let p = match epoch {
            Some(p) => finite(p, || format!("File {n}: epoch-time proportion"))?,
            None => 0.0,
        };
        args.push(format!("--epochtime-proportion={p}"));
    }
    Ok(())
}

/// Append the global options of `cli` that differ from `defaults`.
fn global_diff(defaults: &Cli, cli: &Cli, args: &mut Vec<String>) -> Result<(), String> {
    let d = defaults;
    if cli.y_unit != d.y_unit {
        args.push(format!("--y-unit={}", cli.y_unit));
    }
    if cli.y_units != d.y_units {
        let units = list(&cli.y_units, || "Y unit".into())?;
        args.push(format!("--y-units={units}"));
    }
    if cli.log_y {
        args.push("--log-y".into());
    }
    if let Some(m) = cli.monitor {
        args.push(format!("--monitor={m}"));
    }
    if cli.bounds != d.bounds {
        for &b in &cli.bounds {
            finite(b, || "Window bounds".into())?;
        }
        args.push(format!("--bounds={}", join(&cli.bounds)));
    }
    if cli.absolute_x {
        args.push("--absolute-x".into());
    }
    if cli.repaint_interval_ms != d.repaint_interval_ms {
        args.push(format!("--repaint-interval-ms={}", cli.repaint_interval_ms));
    }
    if cli.x_axis_label != d.x_axis_label {
        args.push(format!("--x-axis-label={}", cli.x_axis_label));
    }
    if cli.x_unit != d.x_unit {
        args.push(format!("--x-unit={}", cli.x_unit));
    }
    if cli.line_width != d.line_width {
        finite(cli.line_width.into(), || "Line width".into())?;
        args.push(format!("--line-width={}", cli.line_width));
    }
    if cli.colors != d.colors {
        let colors = list(&cli.colors, || "Color".into())?;
        args.push(format!("--colors={colors}"));
    }
    if cli.max_points != d.max_points {
        args.push(format!("--max-points={}", cli.max_points));
    }
    if cli.max_x_range != d.max_x_range {
        let r = finite(cli.max_x_range, || "Max X range".into())?;
        args.push(format!("--max-x-range={r}"));
    }
    if cli.marker_radius != d.marker_radius {
        finite(cli.marker_radius.into(), || "Marker radius".into())?;
        args.push(format!("--marker-radius={}", cli.marker_radius));
    }
    Ok(())
}

/// A file path as a positional argument; a relative path starting with `-`
/// gets a `.\` prefix so it does not read as an option (or as `--`).
fn path_arg(path: &Path, n: usize) -> Result<String, String> {
    let s = path
        .to_str()
        .ok_or_else(|| format!("File {n}: the path is not valid Unicode"))?;
    if s.is_empty() {
        return Err(format!("File {n}: no file path"));
    }
    Ok(if s.starts_with('-') {
        format!(".\\{s}")
    } else {
        s.to_string()
    })
}

/// Comma-joined values of an option with `value_delimiter = ','`. An item
/// containing a comma would split in two, so it is an error.
fn list(items: &[String], what: impl Fn() -> String) -> Result<String, String> {
    match items.iter().find(|s| s.contains(',')) {
        Some(bad) => Err(format!("{}: \"{bad}\" cannot contain a comma", what())),
        None => Ok(items.join(",")),
    }
}

fn join<T: ToString>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn finite(v: f64, what: impl Fn() -> String) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{} must be a finite number", what()))
    }
}

/// Characters that make an argument need quotes: whitespace and quotes for
/// the argv rules, the rest so cmd.exe does not act on them.
const QUOTE_TRIGGERS: &[char] = &[' ', '\t', '"', '&', '|', '<', '>', '^', '(', ')'];

/// One argument quoted for a Windows command line so the MSVC /
/// `CommandLineToArgvW` rules read it back unchanged: inside quotes a `"` is
/// written `\"`, and backslashes are doubled only right before a quote.
#[allow(dead_code)] // TODO: drop once the settings window calls it
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(QUOTE_TRIGGERS) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(2 * backslashes + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(2 * backslashes));
    out.push('"');
    out
}

/// `program` and `args` as one line for cmd.exe or a `.cmd` file. The
/// program is only wrapped in quotes: Windows reads the first argument up to
/// the closing quote, without backslash escapes.
#[allow(dead_code)] // TODO: drop once the settings window calls it
pub fn command_line(program: &str, args: &[String]) -> String {
    let mut line = if program.contains(QUOTE_TRIGGERS) {
        format!("\"{program}\"")
    } else {
        program.to_string()
    };
    for arg in args {
        line.push(' ');
        line.push_str(&quote_arg(arg));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn with_program(args: impl IntoIterator<Item = String>) -> Vec<String> {
        std::iter::once("tail-f_plot".to_string()).chain(args).collect()
    }

    /// argv from a line split on whitespace (empty values as `--name=`)
    fn argv(line: &str) -> Vec<String> {
        with_program(line.split_whitespace().map(String::from))
    }

    fn parse(args: Vec<String>) -> (Cli, Vec<GroupConfig>) {
        match parse_args(args) {
            Ok(ParsedArgs::Run { cli, groups }) => (cli, groups),
            Ok(ParsedArgs::Help { .. }) => panic!("parsed as a help request"),
            Err(e) => panic!("{e}"),
        }
    }

    /// Global options as text (Debug of a float is exact)
    fn globals(c: &Cli) -> String {
        format!(
            "{:?}",
            (
                (&c.y_unit, &c.y_units, c.log_y, c.monitor, &c.bounds, c.absolute_x),
                (c.repaint_interval_ms, &c.x_axis_label, &c.x_unit, c.line_width),
                (&c.colors, c.max_points, c.max_x_range, c.marker_radius),
            )
        )
    }

    /// Groups compared by effect: one stack per series, missing = 1
    fn effective(groups: &[GroupConfig]) -> Vec<GroupConfig> {
        groups
            .iter()
            .map(|g| {
                let mut g = g.clone();
                let series = if g.y_names.is_empty() { g.y_cols.len() } else { g.y_names.len() };
                g.y_stacks = (0..series)
                    .map(|i| g.y_stacks.get(i).copied().unwrap_or(1).max(1))
                    .collect();
                g
            })
            .collect()
    }

    /// parse -> to_argv -> parse gives the same settings, and writing those
    /// again gives the same argv. Returns the written argv.
    fn round_trip(line: &str) -> Vec<String> {
        let (cli, groups) = parse(argv(line));
        let written = to_argv(&cli, &groups).unwrap();
        let (cli2, groups2) = parse(with_program(written.clone()));
        assert_eq!(globals(&cli2), globals(&cli), "{line}");
        assert_eq!(effective(&groups2), effective(&groups), "{line}\n{written:?}");
        assert_eq!(to_argv(&cli2, &groups2).unwrap(), written, "{line}");
        written
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_and_missing_path_are_help_requests() {
        assert!(matches!(parse_args(argv("")), Ok(ParsedArgs::Help { requested: false })));
        assert!(matches!(parse_args(argv("--help")), Ok(ParsedArgs::Help { requested: true })));
        assert!(parse_args(argv("a.csv -- --bogus")).is_err());
        assert!(parse_args(argv("a.csv -- b.csv --log-y")).is_err());
    }

    #[test]
    fn defaults_write_only_the_path() {
        assert_eq!(round_trip("a.csv"), strs(&["a.csv"]));
    }

    #[test]
    fn every_first_segment_option_round_trips() {
        let line = "a.csv --x-col=2 --y-cols=3,4 --y-names=T,P --y-stacks=1,2 --header-row=1 \
            --data-start-row=4 --delimiter=tab --string-quote=dquote --merge-delimiter \
            --fixed-width=5,6 --x-time-scale=s --epochtime-col=7 --epochtime-proportion=0.001 \
            --y-unit=K --y-units=K,mbar --log-y --monitor=1 --bounds=0,0,50,50.5 --absolute-x \
            --repaint-interval-ms=100 --x-axis-label=T --x-unit=s --line-width=2 \
            --colors=#ff0000,#00ff00 --max-points=10 --max-x-range=5.5 --marker-radius=3";
        let written = round_trip(line);
        // Every option differs from its default, so every one is written
        assert_eq!(written.len(), line.split_whitespace().count(), "{written:?}");
    }

    #[test]
    fn later_groups_write_only_differences() {
        let written = round_trip("a.csv --y-cols=2,3 --y-names=A,B -- b.csv -- b.csv --y-cols=4 --y-names=C");
        assert_eq!(
            written,
            strs(&["a.csv", "--y-cols=2,3", "--y-names=A,B", "--", "b.csv", "--", "--y-cols=4", "--y-names=C"])
        );
    }

    #[test]
    fn cleared_options_are_written_as_resets() {
        let written = round_trip(
            "a.csv --y-cols=2 --y-names=A --fixed-width=3,4 --merge-delimiter --epochtime-col=5 \
             --epochtime-proportion=2 -- b.csv --y-names= --fixed-width=0 --merge-delimiter=false \
             --epochtime-col=0 --epochtime-proportion=0",
        );
        let second = written.iter().position(|a| a == "--").unwrap();
        assert_eq!(
            written[second..],
            strs(&[
                "--",
                "b.csv",
                "--y-names=",
                "--merge-delimiter=false",
                "--fixed-width=0",
                "--epochtime-col=0",
                "--epochtime-proportion=0",
            ])[..]
        );
    }

    #[test]
    fn x_scale_is_written_by_name_when_exact() {
        let written = round_trip(
            "a.csv --x-time-scale=s -- b.csv --x-proportion=0.5 -- c.csv --x-time-scale=h \
             -- d.csv --x-proportion=0.016666666666666666",
        );
        for want in ["--x-time-scale=s", "--x-proportion=0.5", "--x-time-scale=h", "--x-time-scale=m"] {
            assert!(written.iter().any(|a| a == want), "{want} in {written:?}");
        }
        // The CLI default (0.0166666666666667) is not exactly 1/60
        assert_eq!(round_trip("a.csv --x-time-scale=m"), strs(&["a.csv", "--x-time-scale=m"]));
    }

    #[test]
    fn identical_group_keeps_its_segment() {
        let written = round_trip("a.csv --y-cols=2 -- a.csv");
        assert_eq!(written, strs(&["a.csv", "--y-cols=2", "--", "a.csv"]));
        assert_eq!(parse(with_program(written)).1.len(), 2);
    }

    #[test]
    fn stacks_are_compared_by_effect() {
        assert_eq!(round_trip("a.csv --y-cols=2,3 --y-stacks=1,1"), strs(&["a.csv", "--y-cols=2,3"]));
        assert_eq!(
            round_trip("a.csv --y-cols=2,3 --y-stacks=0,2"),
            strs(&["a.csv", "--y-cols=2,3", "--y-stacks=1,2"])
        );
        let written = round_trip("a.csv --y-cols=2,3 --y-stacks=1,2 -- b.csv --y-stacks=1");
        assert_eq!(written.last().unwrap(), "--y-stacks=1,1");
    }

    #[test]
    fn values_starting_with_a_hyphen_stay_values() {
        let written = round_trip("a.csv --y-cols=2 --y-names=-5V --x-proportion=-1 --x-unit=-");
        assert!(written.contains(&"--y-names=-5V".to_string()), "{written:?}");
    }

    #[test]
    fn path_starting_with_a_hyphen_gets_a_dot_prefix() {
        let (cli, mut groups) = parse(argv("x.csv"));
        groups[0].csv_path = PathBuf::from("-a.csv");
        let written = to_argv(&cli, &groups).unwrap();
        assert_eq!(written, strs(&[".\\-a.csv"]));
        assert!(parse_args(with_program(written)).is_ok());
    }

    #[test]
    fn draft_spellings_of_resets_mean_the_reset() {
        let (cli, mut groups) = parse(argv("a.csv --y-cols=2"));
        groups[0].y_names = strs(&[""]);
        groups[0].fixed_width = vec![0];
        groups[0].epochtime_proportion = Some(0.0);
        assert_eq!(to_argv(&cli, &groups).unwrap(), strs(&["a.csv", "--y-cols=2"]));
    }

    #[test]
    fn inexpressible_settings_are_errors() {
        let (cli, groups) = parse(argv("a.csv --y-cols=2 -- b.csv"));

        let mut g = groups.clone();
        g[1].y_names = strs(&["a,b"]);
        assert!(to_argv(&cli, &g).unwrap_err().contains("File 2"));

        let mut g = groups.clone();
        g[1].y_cols.clear();
        assert!(to_argv(&cli, &g).unwrap_err().contains("File 2"));

        let mut g = groups.clone();
        g[0].csv_path = PathBuf::new();
        assert!(to_argv(&cli, &g).unwrap_err().contains("File 1"));

        let mut c = cli.clone();
        c.max_x_range = f64::NAN;
        assert!(to_argv(&c, &groups).is_err());

        assert!(to_argv(&cli, &[]).is_err());
    }

    #[test]
    fn quoting_follows_the_windows_argv_rules() {
        assert_eq!(quote_arg("a.csv"), "a.csv");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg(r"C:\x y\"), r#""C:\x y\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote_arg(r"a\b c"), r#""a\b c""#);
        assert_eq!(quote_arg("--y-names=Heater(%)"), "\"--y-names=Heater(%)\"");
        assert_eq!(quote_arg("x&y"), "\"x&y\"");
    }

    #[test]
    fn command_line_quotes_the_program_without_escapes() {
        let args = strs(&["a b.csv", "--y-cols=2"]);
        assert_eq!(
            command_line(r"C:\Program Files\tail-f_plot.exe", &args),
            r#""C:\Program Files\tail-f_plot.exe" "a b.csv" --y-cols=2"#
        );
        assert_eq!(command_line(r"C:\t\tail-f_plot.exe", &[]), r"C:\t\tail-f_plot.exe");
    }
}
