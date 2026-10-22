# tail-f_plot

Real-time CSV/TSV plotting tool for Windows. Watches a data file and updates the plot as new rows are appended — like `tail -f` but with a graph.

Built with [egui](https://github.com/emilk/egui) and [egui_plot](https://github.com/emilk/egui_plot).

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- **Live tail**: Watches the file via OS notifications (polling as a fallback) and redraws automatically. Like `tail`, a file is read from its end: only the newest `--max-points` rows are read and kept, so a huge file opens instantly and memory stays bounded however long it runs. The file is not held open between reads, and a file that is replaced, truncated or rewritten is re-read from scratch.
- **Flexible CSV parsing**: Configurable delimiter (comma, tab, semicolon, space, pipe), quoted fields, merged delimiters, fixed-width columns.
- **Multiple Y series**: Plot any combination of columns on a single chart.
- **Multiple files**: Overlay several files on the same axes with `--` separators, including epoch-time alignment for recordings that started at different times.
- **Vertical stacks**: Split series into vertically stacked sub-plots (`--y-stacks`) with per-stack Y units, a linked X axis (pan/zoom in sync), and one hover driving the crosshair and tooltip across every sub-plot.
- **Interactive zoom/pan**: Scroll-wheel zoom, drag pan, box zoom-in, click zoom-out.
- **Auto-X / Auto-Y**: Each axis can follow the data on its own. With only one of them on, it fits just the lines drawn inside the other axis' current range — e.g. zoom Y to 3–5 K during a cool-down from 300 K, turn Auto-X on, and the time axis trims to where the trace is inside that band (still growing as new rows arrive) instead of spanning the empty hours above it. Hidden series (legend click) are ignored. When nothing is drawn in that range, the axis keeps its last range.
- **Log scale**: Toggle Y-axis log scale at runtime.
- **Multi-monitor DPI aware**: Per-monitor DPI v2 support with work-area-percentage window placement.
- **Zero config**: All settings via CLI arguments. No config files needed.

## Installation

### MSI Installer

Download the latest `.msi` from [Releases](../../releases). No admin rights needed.

- Installs to `%LOCALAPPDATA%\HANA\tail-f_plot\`
- Adds to user PATH automatically
- Start Menu shortcut in `HANA-Cryogenics`
- Uninstall via Windows Settings > Apps

### Portable

Download `tail-f_plot.exe` from [Releases](../../releases) and place it anywhere.

### From source

```
cargo build --release
```

## Quick Start

```
# Basic: plot column 2 over column 1
tail-f_plot.exe data.csv --y-cols 2

# Multiple series with names
tail-f_plot.exe data.csv --y-cols 2,3,4 --y-names "Voltage,Current,Power"

# TSV file
tail-f_plot.exe data.tsv --delimiter tab --y-cols 2,3

# Log scale, specific monitor, window placement
tail-f_plot.exe data.csv --y-cols 5 --log-y --monitor 1 --bounds 0,0,50,100

# Two files on the same axes (b.csv inherits a.csv's options)
tail-f_plot.exe a.csv --y-cols 2 -- b.csv

# Aligned by epoch time even though recordings started at different times
tail-f_plot.exe a.csv --y-cols 2 --epochtime-col 5 -- b.csv

# Three files as three stacked sub-plots (temp / current / voltage)
tail-f_plot.exe temp.csv --y-cols 5 --y-stacks 1 --y-units K,A,V ^
  -- current.csv --y-cols 5 --y-stacks 2 ^
  -- voltage.csv --y-cols 4 --y-stacks 3
```

## Usage

```
tail-f_plot.exe <CSV_PATH> [OPTIONS] [-- [CSV_PATH] [OPTIONS] ...]
```

### Launching

#### From the console

`tail-f_plot` is a GUI app that does **not** release the console prompt on its own. Use `start` to launch it in the background:

```cmd
rem cmd
start tail-f_plot.exe data.csv --y-cols 2,3
```

```powershell
# PowerShell 5.1
Start-Process .\tail-f_plot.exe -ArgumentList "data.csv","--y-cols","2,3"

# PowerShell 7
.\tail-f_plot.exe data.csv --y-cols 2,3 &
# [PS7] Job objects persist after the app exits. Use `Get-Job` / `Remove-Job` to clean up.
```

```bash
# Git Bash (stdout is printed to the terminal)
./tail-f_plot.exe data.csv --y-cols 2,3 &
```

> **Note (v0.2.x):** In v0.1.x the process detached automatically. From v0.2.x onward `start` (or equivalent) is required to get the prompt back immediately.

#### From another application

If your app spawns `tail-f_plot` as a child process:

- **Use a dedicated thread** (or async task) for the spawn call — the process stays alive as long as the GUI is open, so a synchronous wait will freeze your app.
- Alternatively, use `start` / `DETACHED_PROCESS` if you don't need stdout.

#### stdout status

On successful launch, two lines are printed to stdout and then **no further output is produced** (release builds only; debug builds may emit diagnostics to stderr). Parse the **first line prefix** to determine the outcome:

| First line prefix | Meaning | Exit code |
|-------------------|---------|-----------|
| `PID: ` | Launched successfully. Each following line is `WATCHING: <path>` (file exists) or `WAITING: <path>` (file not yet created) — one per watched file. | — |
| `HANA ` | Help text displayed (no args, or `--help`). | 0 or 1 |
| `EXCEPTION: ` | Invalid arguments. Error detail follows. | 1 |

With `--oneshot <PNG_PATH>` the window renders once, saves a screenshot and exits. After the usual lines it prints `SCREENSHOT: <absolute path>` (or `EXCEPTION: oneshot: ...`). Exit codes: `0` = captured once the view settled (every file read, layout stable); `3` = captured after a 15 s settle timeout (e.g. a file that never appeared); `1` = the PNG could not be written; `4` = no screenshot could be captured at all. The window opens without taking focus, ignores the mouse, and stops reading new rows once the capture is requested.

> `cmd start` and PowerShell `Start-Process` cannot capture stdout. Git Bash `&` is the only shell method that shows stdout while returning the prompt. To capture stdout programmatically, spawn the process directly with a pipe.

### Arguments

| Argument | Description |
|----------|-------------|
| `<CSV_PATH>` | Path to the CSV/TSV data file |

### General Options

| Option | Default | Description |
|--------|---------|-------------|
| `--x-col <N>` | `1` | X-axis column (1-based) |
| `--y-cols <N,N,...>` | | Y-axis columns (1-based, comma-separated) |
| `--y-names <A,B,...>` | | Legend names (comma-separated; `""` = default names) |
| `--y-stacks <N,N,...>` | all `1` | Stack index per Y column (1-based); see below |
| `--header-row <N>` | `2` | Header row number (1-based) |
| `--data-start-row <N>` | `3` | Data start row (1-based) |
| `--y-unit <UNIT>` | | Y-axis unit shown in title and legend |
| `--y-units <U,U,...>` | | Y-axis unit per stack index (overrides `--y-unit`) |
| `--log-y` | `false` | Start with Y-axis log scale |
| `--monitor <N>` | | Target monitor index (0-based) |
| `--bounds <x1,y1,x2,y2>` | | Window bounds as work-area percentages |
| `--absolute-x` | `false` | Use raw X values (skip origin subtraction) |

### CSV Parsing Options

| Option | Default | Description |
|--------|---------|-------------|
| `--delimiter <NAME>` | `comma` | `comma`, `tab`, `semicolon`, `space`, `vbar` |
| `--string-quote <NAME>` | `none` | `none`, `squote`, `dquote`, `backtick` |
| `--merge-delimiter` | `false` | Treat consecutive delimiters as one (`--merge-delimiter=false` to disable) |
| `--fixed-width <N,N,...>` | | Fixed-width column widths (overrides delimiter) |
| `--epochtime-col <N>` | `0` (off) | Epoch-time column (1-based) for multi-file alignment |
| `--epochtime-proportion <F>` | `--x-proportion` | Multiplier applied to the epoch-time column (`0` = follow `--x-proportion`) |

### Rendering Options

| Option | Default | Description |
|--------|---------|-------------|
| `--repaint-interval-ms <N>` | `250` | Poll/repaint interval in milliseconds |
| `--x-axis-label <STR>` | `Time` | X-axis label text |
| `--x-unit <UNIT>` | `min` | X-axis unit (shown in axis label and tooltip) |
| `--x-time-scale <NAME>` | | Time scale: `d`, `h`, `m`, `s` (overrides `--x-proportion`) |
| `--x-proportion <F>` | `0.01667` (1/60) | Multiplier applied to raw X values |
| `--line-width <F>` | `1.5` | Line stroke width |
| `--colors <#RRGGBB,...>` | 12 defaults | Series colors (empty = built-in palette) |
| `--max-points <N>` | `5000` | Max rendered points per series (see note below) |
| `--max-x-range <F>` | `120.0` | Max visible X range after proportion (see note below) |
| `--marker-radius <F>` | `4.0` | Data point marker radius |
| `--oneshot <PNG_PATH>` | | Render once, save a PNG screenshot, exit (see stdout status) |
| `--oneshot-settings <TAB>` | | With `--oneshot`: also open the settings window on `display`, `files`, `window` or `command-line` and save it as `<PNG stem>-settings.png` |

> **`--max-points` / `--max-x-range`:** Both limit the visible window. The stricter of the two applies (per file in multi-file mode). The X window is anchored at the newest row: `--max-x-range` keeps the rows back to the first one whose X leaves the window. This works for an X column that is not a timestamp too (e.g. `R` against a temperature that ramps up and down) — auto-fit then spans the column's true min/max, not the first and last row. Only the newest `--max-points` rows of each file are read and kept in memory at all.

## Multiple Files

`--` starts a new *file group* plotted on the same axes:

```
tail-f_plot.exe a.csv --y-cols 2,3 -- b.csv --y-cols 2 -- c.csv
```

**Inheritance** — options omitted in a group (even the file path) inherit from the previous group; options given apply to that group and are inherited onward. In the example above `c.csv` uses `--y-cols 2` from the `b.csv` group. A fully empty group (e.g. a trailing `--`) is ignored.

**Per-group options**: `<CSV_PATH>`, `--x-col`, `--y-cols`, `--y-names`, `--y-stacks`, `--header-row`, `--data-start-row`, `--delimiter`, `--string-quote`, `--merge-delimiter`, `--fixed-width`, `--x-time-scale`, `--x-proportion`, `--epochtime-col`, `--epochtime-proportion`. All other options are window-global and belong to the first group only — using one after `--` is an error.

**Resetting an inherited option**: `--epochtime-col 0` (disable), `--fixed-width 0` (back to delimiter mode), `--merge-delimiter=false`, `--y-names ""` (back to default names), `--epochtime-proportion 0` (back to following the group's `--x-proportion`).

**X units per file** — each group's `--x-time-scale` / `--x-proportion` scales that file's raw X into the shared display unit. E.g. first file logs seconds, second logs milliseconds:

```
tail-f_plot.exe a.csv --y-cols 2 -- b.csv --x-proportion 0.0000166666666667
```

**X offset** — without `--absolute-x`, every file shares the first file's X origin (the first file's first row is display x = 0). This aligns files whose X columns run on the same clock. With `--absolute-x`, all files plot raw scaled X values.

**Epoch-time alignment** — when X columns are *elapsed* times (each file starting at 0), shared origin cannot align them. If the files also log an epoch timestamp, give its column via `--epochtime-col`: each later file is then shifted by the wall-clock difference between its first epoch value and the first file's, so recordings that started at different times align exactly. Requires the epoch column on the first file too (inheritance makes this automatic); files without it fall back to the shared origin. The epoch column is scaled by `--epochtime-proportion` (default: the group's `--x-proportion`) — override it when the epoch unit differs from the X-column unit (e.g. epoch in ms, X in s). Ignored under `--absolute-x`.

## Vertical Stacks

`--y-stacks` assigns each Y column a 1-based stack index; series with the same index share one sub-plot, and different indices are laid out top-to-bottom in index order (unused indices are skipped). Works per file group, so multi-file and stacking combine freely:

```
tail-f_plot.exe temp.csv --y-cols 5 --y-names Temp --y-stacks 1 --y-units K,A,V ^
  -- current.csv --y-cols 5 --y-names Current --y-stacks 2 ^
  -- voltage.csv --y-cols 4 --y-names Voltage --y-stacks 3
```

- **Per-stack units**: `--y-units K,A,V` maps units positionally by stack *index* (stack 3 always takes the third entry). Missing entries fall back to `--y-unit`.
- **Linked X axis**: dragging or zooming the X range in any sub-plot moves all of them together. Y stays independent: a wheel zoom, box zoom, click zoom-out or pan only rescales the Y of the sub-plot under the cursor — every other one keeps the Y range it had. Auto-X fits the union of every stack's data.
- **One hover, every sub-plot**: the vertical crosshair tracks the cursor's time position across all sub-plots, and the tooltip lists every visible series (all stacks) at that time, each with its stack's unit. The tooltip box may overflow into the neighbouring sub-plots rather than being clipped at the stack boundary.
- **Layout**: sub-plots split the window height equally; the X axis (ticks + label) is drawn on the bottom sub-plot only. Each sub-plot has its own legend.

> **Notes**
> - Series colors and marker shapes continue across groups; default legend names get a file-stem prefix (`a:Col2`).
> - Until the first file produces its first row (and, for epoch alignment, both epoch anchors are known), later files buffer their data and appear once the offset is resolvable.
> - Duplicate series names (e.g. `--y-names` inherited into a later group) are disambiguated automatically: the second occurrence becomes `Name(2)`, the third `Name(3)`, and so on.
> - The same file may appear in several groups (e.g. plotting different columns of one file).

## Keyboard & Mouse

| Input | Action |
|-------|--------|
| Scroll wheel | Zoom in/out |
| Drag | Pan (in default mode) |
| Toolbar buttons | Home, Zoom In (box), Zoom Out (click), Auto X/Y, Log scale (the icon shows the current scale), Legend position. Hover a button for its tooltip |

## Data Format

The tool expects a text file with:
1. Optional header rows (configurable via `--header-row`, `--data-start-row`)
2. Numeric data in the specified columns
3. New rows appended over time (tail-f behavior)

Empty cells produce gaps in the line (no interpolation). Lines end with LF or CRLF; a last line without its newline waits until it is complete. A UTF-8 BOM is ignored, and text that is not UTF-8 (e.g. a CP949 comment column) does not affect the numeric columns. When the file disappears the plot keeps its rows until the file comes back.

## License

[MIT](LICENSE)
