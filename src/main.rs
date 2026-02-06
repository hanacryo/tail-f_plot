// Hide CMD console window in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod csv_watcher;
mod plot_state;
mod toolbar;

use clap::Parser;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

/// Try to attach to parent console on Windows. Returns true (CLI) or false (double-click).
#[cfg(windows)]
fn try_attach_console() -> bool {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(not(windows))]
fn try_attach_console() -> bool { true }

/// Show a Windows MessageBox
#[cfg(windows)]
fn show_message_box(title: &str, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let title_wide = to_wide(title);
    let message_wide = to_wide(message);

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message_wide.as_ptr(),
            title_wide.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(not(windows))]
fn show_message_box(_title: &str, _message: &str) {}

/// Load icon data from embedded PNG
fn load_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../hana.png"))
        .expect("Failed to load icon")
}

#[derive(Parser, Debug)]
#[command(name = "tail-f_plot", version = env!("FULL_VERSION"), disable_help_flag = true)]
pub struct Cli {
    /// Print help
    #[arg(long, default_value_t = false)]
    help: bool,

    /// CSV file path
    csv_path: Option<PathBuf>,

    /// X-axis column (1-based)
    #[arg(long, default_value_t = 1)]
    x_col: usize,

    /// Y-axis columns (1-based, comma separated)
    #[arg(long, value_delimiter = ',')]
    y_cols: Vec<usize>,

    /// Y-axis legend names (comma separated)
    #[arg(long, value_delimiter = ',')]
    y_names: Vec<String>,

    /// Header row number (1-based)
    #[arg(long, default_value_t = 2)]
    header_row: usize,

    /// Data start row number (1-based)
    #[arg(long, default_value_t = 3)]
    data_start_row: usize,

    /// Y-axis unit (used in window title)
    #[arg(long, default_value = "")]
    y_unit: String,

    /// Start with log scale on Y-axis
    #[arg(long, default_value_t = false)]
    log_y: bool,

    /// Monitor index (0-based, optional)
    #[arg(long)]
    monitor: Option<u32>,

    /// Window bounds as work-area percentages: x1%,y1%,x2%,y2%
    #[arg(long, value_delimiter = ',')]
    bounds: Vec<f64>,

    /// Use absolute X values (skip origin subtraction)
    #[arg(long, default_value_t = false)]
    absolute_x: bool,

    // --- CSV parsing options ---

    /// Field delimiter: comma, tab, semicolon, space, vbar
    #[arg(long, default_value = "comma")]
    delimiter: String,

    /// String quote character: none, squote, dquote, backtick
    #[arg(long, default_value = "none")]
    string_quote: String,

    /// Merge consecutive delimiters into one
    #[arg(long, default_value_t = false)]
    merge_delimiter: bool,

    /// Fixed-width column widths (comma separated). Overrides delimiter when set.
    #[arg(long, value_delimiter = ',')]
    fixed_width: Vec<usize>,

    // --- Rendering options ---

    /// Repaint interval in milliseconds
    #[arg(long, default_value_t = 250)]
    repaint_interval_ms: u64,

    /// X-axis label
    #[arg(long, default_value = "Time (min)")]
    x_axis_label: String,

    /// X-value proportional constant (multiplied to raw X)
    #[arg(long, default_value_t = 0.0166666666666667)]
    x_proportion: f64,

    /// Line width
    #[arg(long, default_value_t = 1.5)]
    line_width: f32,

    /// Series colors as #RRGGBB (comma separated). Empty = default 12 colors.
    #[arg(long, value_delimiter = ',')]
    colors: Vec<String>,

    /// Maximum points per series to render
    #[arg(long, default_value_t = 5000)]
    max_points: usize,

    /// Maximum X range (after proportion applied)
    #[arg(long, default_value_t = 120.0)]
    max_x_range: f64,

    /// Marker radius
    #[arg(long, default_value_t = 4.0)]
    marker_radius: f32,
}

/// Resolve delimiter name to actual string
pub fn resolve_delimiter(name: &str) -> String {
    match name {
        "tab" => "\t".to_string(),
        "semicolon" => ";".to_string(),
        "space" => " ".to_string(),
        "vbar" => "|".to_string(),
        _ => ",".to_string(), // "comma" and default
    }
}

/// Resolve string-quote name to Option<char>
pub fn resolve_string_quote(name: &str) -> Option<char> {
    match name {
        "squote" => Some('\''),
        "dquote" => Some('"'),
        "backtick" => Some('`'),
        _ => None, // "none" and default
    }
}

fn help_text() -> String {
    format!(
        "HANA tail-f_plot v{ver}\n\n\
        Real-time CSV plotting tool.\n\n\
        Usage:\n\
        tail-f_plot.exe [OPTIONS] <CSV_PATH>\n\n\
        Arguments:\n\
        <CSV_PATH>  CSV file path\n\n\
        Options:\n\
        --x-col <N>              X-axis column (1-based) [default: 1]\n\
        --y-cols <N,N,...>       Y-axis columns (comma separated)\n\
        --y-names <A,B,...>      Y-axis legend names (comma separated)\n\
        --header-row <N>         Header row number [default: 2]\n\
        --data-start-row <N>     Data start row [default: 3]\n\
        --y-unit <UNIT>          Y-axis unit (shown in title)\n\
        --log-y                  Start with log scale on Y-axis\n\
        --monitor <N>            Monitor index (0-based)\n\
        --bounds <x1,y1,x2,y2>  Window bounds (work-area %)\n\
        --absolute-x             Use absolute X (no origin subtraction)\n\n\
        CSV Parsing:\n\
        --delimiter <NAME>       comma|tab|semicolon|space|vbar [default: comma]\n\
        --string-quote <NAME>    none|squote|dquote|backtick [default: none]\n\
        --merge-delimiter        Merge consecutive delimiters\n\
        --fixed-width <N,N,...>  Fixed-width columns (overrides delimiter)\n\n\
        Rendering:\n\
        --repaint-interval-ms <N>  Repaint interval [default: 250]\n\
        --x-axis-label <STR>       X-axis label [default: Time (min)]\n\
        --x-proportion <F>         X proportional constant [default: 1/60]\n\
        --line-width <F>           Line width [default: 1.5]\n\
        --colors <#RRGGBB,...>     Series colors (empty=default 12)\n\
        --max-points <N>           Max points per series [default: 5000]\n\
        --max-x-range <F>          Max X range [default: 120.0]\n\
        --marker-radius <F>        Marker radius [default: 4.0]\n\n\
        Example:\n\
        tail-f_plot.exe data.csv --y-cols 2,3 --y-names \"Temp,Pressure\"\n\
        tail-f_plot.exe data.tsv --delimiter tab --y-cols 2,3",
        ver = env!("FULL_VERSION")
    )
}

/// Default 12-color palette
pub const DEFAULT_COLORS: [[u8; 3]; 12] = [
    [31, 119, 180],   // blue
    [255, 127, 14],   // orange
    [44, 160, 44],    // green
    [214, 39, 40],    // red
    [148, 103, 189],  // purple
    [140, 86, 75],    // brown
    [227, 119, 194],  // pink
    [127, 127, 127],  // gray
    [188, 189, 34],   // olive
    [23, 190, 207],   // cyan
    [255, 187, 120],  // light orange
    [152, 223, 138],  // light green
];

/// Parse #RRGGBB strings to [u8;3]. Falls back to default 12 colors if empty.
pub fn resolve_colors(input: &[String]) -> Vec<[u8; 3]> {
    if input.is_empty() {
        return DEFAULT_COLORS.to_vec();
    }
    input
        .iter()
        .filter_map(|s| {
            let hex = s.trim_start_matches('#');
            if hex.len() != 6 { return None; }
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b])
        })
        .collect()
}

/// Get monitor work area (excluding taskbar) + DPI scale factor for given index (0-based).
/// Returns: (left, top, right, bottom, dpi, scale_factor) in physical pixels (virtual screen coords).
#[cfg(windows)]
fn get_monitor_work_area(index: u32) -> Option<(i32, i32, i32, i32, u32, f64)> {
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use std::mem;

    unsafe extern "system" fn cb(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let vec = &mut *(data as *mut Vec<HMONITOR>);
        vec.push(hmon);
        1 // TRUE
    }

    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            0 as HDC,
            std::ptr::null(),
            Some(cb),
            &mut monitors as *mut _ as LPARAM,
        );
    }

    let hmon = *monitors.get(index as usize)?;
    let mut info: MONITORINFO = unsafe { mem::zeroed() };
    info.cbSize = mem::size_of::<MONITORINFO>() as u32;

    if unsafe { GetMonitorInfoW(hmon, &mut info) } == 0 {
        return None;
    }

    // Query per-monitor DPI
    let mut dpi_x: u32 = 96;
    let mut dpi_y: u32 = 96;
    unsafe {
        GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let scale = dpi_x as f64 / 96.0;

    let r = info.rcWork;
    Some((r.left, r.top, r.right, r.bottom, dpi_x, scale))
}

fn main() -> eframe::Result {
    // Set Per-Monitor DPI Aware v2 (must be called first for accurate physical coords/DPI)
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
        // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
        SetProcessDpiAwarenessContext(-4isize as _);
    }

    // Parse CLI — all error/help paths show the same help_text()
    let cli = match Cli::try_parse() {
        Ok(cli) => {
            if cli.help || cli.csv_path.is_none() {
                let has_console = try_attach_console();
                if has_console {
                    eprintln!("{}", help_text());
                } else {
                    show_message_box("tail-f_plot Usage", &help_text());
                }
                std::process::exit(if cli.help { 0 } else { 1 });
            }
            cli
        }
        Err(e) => {
            // --version: clap 기본 동작 유지
            if e.kind() == clap::error::ErrorKind::DisplayVersion {
                let has_console = try_attach_console();
                if has_console {
                    eprintln!("{}", e);
                }
                std::process::exit(0);
            }
            let has_console = try_attach_console();
            if has_console {
                eprintln!("{}", help_text());
            } else {
                show_message_box("tail-f_plot Usage", &help_text());
            }
            std::process::exit(1);
        }
    };

    let csv_filename = cli
        .csv_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let names_str = cli.y_names.join(",");
    let title = format!(
        "{} - {} - HANA tail-f_plot - v{}",
        names_str, csv_filename, env!("FULL_VERSION")
    );

    // Compute physical pixel placement when --bounds is specified (for SetWindowPos)
    let physical_placement: Option<[i32; 4]> = if cli.bounds.len() == 4 {
        let mon_idx = cli.monitor.unwrap_or(0);
        #[cfg(windows)]
        {
            get_monitor_work_area(mon_idx).map(|(wl, wt, wr, wb, _dpi, _scale)| {
                let ww = (wr - wl) as f64;
                let wh = (wb - wt) as f64;
                let left = wl + (ww * cli.bounds[0] / 100.0) as i32;
                let top = wt + (wh * cli.bounds[1] / 100.0) as i32;
                let right = wl + (ww * cli.bounds[2] / 100.0) as i32;
                let bottom = wt + (wh * cli.bounds[3] / 100.0) as i32;
                [left, top, right - left, bottom - top]
            })
        }
        #[cfg(not(windows))]
        { let _ = mon_idx; None }
    } else {
        None
    };

    let viewport = egui::ViewportBuilder::default()
        .with_title(&title)
        .with_icon(Arc::new(load_icon()));

    const WIN_W: f32 = 1200.0;
    const WIN_H: f32 = 600.0;

    // Set default size for eframe. Actual position/size handled by SetWindowPos.
    let viewport = if let Some(_mon_idx) = cli.monitor {
        #[cfg(windows)]
        {
            if let Some((wl, wt, wr, wb, _dpi, scale)) = get_monitor_work_area(_mon_idx) {
                let phys_w = WIN_W as f64 * scale;
                let phys_h = WIN_H as f64 * scale;
                let cx = ((wl + wr) as f64 / 2.0 - phys_w / 2.0) / scale;
                let cy = ((wt + wb) as f64 / 2.0 - phys_h / 2.0) / scale;
                viewport
                    .with_position([cx as f32, cy as f32])
                    .with_inner_size([WIN_W, WIN_H])
            } else {
                viewport.with_inner_size([WIN_W, WIN_H])
            }
        }
        #[cfg(not(windows))]
        {
            viewport.with_inner_size([WIN_W, WIN_H])
        }
    } else {
        viewport.with_inner_size([WIN_W, WIN_H])
    };

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let title_for_app = title.clone();
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::PlotApp::new(cc, cli, title_for_app, physical_placement)))
        }),
    )
}

/// Place window using FindWindowW + SetWindowPos. Uses physical pixel coordinates directly.
#[cfg(windows)]
pub fn apply_window_placement(title: &str, x: i32, y: i32, w: i32, h: i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let title_wide: Vec<u16> = OsStr::new(title).encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
        if !hwnd.is_null() {
            // SWP_NOZORDER(0x0004): don't change z-order
            SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, 0x0004);
        }
    }
}

#[cfg(not(windows))]
pub fn apply_window_placement(_title: &str, _x: i32, _y: i32, _w: i32, _h: i32) {}
