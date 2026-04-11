# tail-f_plot

Real-time CSV/TSV plotting tool for Windows. Watches a data file and updates the plot as new rows are appended — like `tail -f` but with a graph.

Built with [egui](https://github.com/emilk/egui) and [egui_plot](https://github.com/emilk/egui_plot).

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- **Live tail**: Watches the file via OS notifications and redraws automatically.
- **Flexible CSV parsing**: Configurable delimiter (comma, tab, semicolon, space, pipe), quoted fields, merged delimiters, fixed-width columns.
- **Multiple Y series**: Plot any combination of columns on a single chart.
- **Interactive zoom/pan**: Scroll-wheel zoom, drag pan, box zoom-in, click zoom-out.
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
```

## Usage

```
tail-f_plot.exe <CSV_PATH> [OPTIONS]
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
| `PID: ` | Launched successfully. Second line is `WATCHING: <path>` (file exists) or `WAITING: <path>` (file not yet created). | — |
| `HANA ` | Help text displayed (no args, or `--help`). | 0 or 1 |
| `EXCEPTION: ` | Invalid arguments. Error detail follows. | 1 |

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
| `--y-names <A,B,...>` | | Legend names (comma-separated) |
| `--header-row <N>` | `2` | Header row number (1-based) |
| `--data-start-row <N>` | `3` | Data start row (1-based) |
| `--y-unit <UNIT>` | | Y-axis unit shown in title and legend |
| `--log-y` | `false` | Start with Y-axis log scale |
| `--monitor <N>` | | Target monitor index (0-based) |
| `--bounds <x1,y1,x2,y2>` | | Window bounds as work-area percentages |
| `--absolute-x` | `false` | Use raw X values (skip origin subtraction) |

### CSV Parsing Options

| Option | Default | Description |
|--------|---------|-------------|
| `--delimiter <NAME>` | `comma` | `comma`, `tab`, `semicolon`, `space`, `vbar` |
| `--string-quote <NAME>` | `none` | `none`, `squote`, `dquote`, `backtick` |
| `--merge-delimiter` | `false` | Treat consecutive delimiters as one |
| `--fixed-width <N,N,...>` | | Fixed-width column widths (overrides delimiter) |

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

> **`--max-points` / `--max-x-range`:** Both limit the visible window. The stricter of the two applies.

## Keyboard & Mouse

| Input | Action |
|-------|--------|
| Scroll wheel | Zoom in/out |
| Drag | Pan (in default mode) |
| Toolbar buttons | Home, Zoom In (box), Zoom Out (click), Auto X/Y, Log scale, Legend position |

## Data Format

The tool expects a text file with:
1. Optional header rows (configurable via `--header-row`, `--data-start-row`)
2. Numeric data in the specified columns
3. New rows appended over time (tail-f behavior)

Empty cells produce gaps in the line (no interpolation).

## License

[MIT](LICENSE)
