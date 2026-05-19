// Use console subsystem so that stdout/stderr works in all terminals
// (cmd, PowerShell, Git Bash / mintty).  For double-click launches a
// transient console window is hidden immediately — see `hide_own_console()`.
// #![windows_subsystem = "console"]  ← implicit default; kept as a comment for clarity.

mod app;
mod csv_watcher;
mod custom_legend;
mod plot_state;
mod toolbar;
mod tooltip;

use clap::Parser;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

/// If this process owns its console (double-click launch) AND stdout is not piped,
/// hide the console window immediately and return `true`.
/// When launched from a terminal or when stdout is redirected/piped by a parent
/// process, return `false` so output goes to stdout instead of a GUI window.
#[cfg(windows)]
fn hide_own_console() -> bool {
    use windows_sys::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE};
    unsafe {
        // If stdout is a pipe, a parent process is capturing our output — stay in CLI mode.
        let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if GetFileType(stdout_handle) == FILE_TYPE_PIPE {
            return false;
        }

        let mut pids = [0u32; 4];
        let count = GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32);
        if count <= 1 {
            // Only this process uses the console → we allocated it (double-click).
            let wnd = GetConsoleWindow();
            if !wnd.is_null() {
                ShowWindow(wnd, SW_HIDE);
            }
            return true;
        }
        false
    }
}

#[cfg(not(windows))]
fn hide_own_console() -> bool { false }

/// Register OS fonts as primary, with egui defaults as fallback.
pub fn register_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Load OS fonts
    let mut load = |key: &str, path: &str| -> bool {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(key.into(), egui::FontData::from_owned(data).into());
            true
        } else {
            false
        }
    };
    let has_segoe  = load("segoe_ui", "C:\\Windows\\Fonts\\segoeui.ttf");
    let has_bold   = load("segoe_bold", "C:\\Windows\\Fonts\\segoeuib.ttf");
    let has_mono   = load("consolas", "C:\\Windows\\Fonts\\consola.ttf");
    let has_symbol = load("segoe_symbol", "C:\\Windows\\Fonts\\seguisym.ttf");

    // Prepend OS font to Proportional (Segoe UI → Segoe UI Symbol → egui defaults)
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if has_symbol { family.insert(0, "segoe_symbol".into()); }
        if has_segoe  { family.insert(0, "segoe_ui".into()); }
    }

    // Prepend OS font to Monospace (Consolas → egui defaults as fallback)
    if has_mono {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.insert(0, "consolas".into());
        }
    }

    // Bold family: Segoe UI Bold + Proportional fallback
    let proportional_names = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let bold_family = fonts
        .families
        .entry(egui::FontFamily::Name("Bold".into()))
        .or_default();
    if has_bold {
        bold_family.push("segoe_bold".into());
    }
    for name in proportional_names {
        bold_family.push(name);
    }

    ctx.set_fonts(fonts);
}

/// Render About modal dialog (shared between PlotApp and HelpApp)
pub fn render_about_modal(ctx: &egui::Context, show: &mut bool) {
    use egui::text::LayoutJob;
    use egui::{FontFamily, FontId, TextFormat};

    let modal = egui::containers::Modal::new(egui::Id::new("about_modal")).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            let mut job = LayoutJob::default();
            job.append(
                "HANA tail-f_plot",
                0.0,
                TextFormat {
                    font_id: FontId::new(16.0, FontFamily::Name("Bold".into())),
                    ..Default::default()
                },
            );
            job.append(
                &format!(" v{}", env!("FULL_VERSION")),
                0.0,
                TextFormat {
                    font_id: FontId::new(14.0, FontFamily::Proportional),
                    ..Default::default()
                },
            );
            ui.label(job);

            ui.add_space(8.0);
            ui.label("Real-time CSV/TSV plotting tool (tail -f with a graph)");
            ui.add_space(4.0);
            ui.label("Copyright \u{24d2}2026 HANA");
            ui.add_space(4.0);
            ui.hyperlink("https://github.com/hanacryo/tail-f_plot");
            ui.add_space(12.0);

            let btn = egui::Button::new("OK").min_size(egui::vec2(80.0, 28.0));
            if ui.add(btn).clicked() {
                *show = false;
            }
        });
    });

    if modal.should_close() {
        *show = false;
    }
}

/// HelpApp: eframe app for displaying help text when launched without arguments (double-click)
struct HelpApp {
    help_text: String,
    show_about: bool,
}

impl eframe::App for HelpApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("help_buttons").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let btn_size = egui::vec2(80.0, 28.0);
                if ui.add(egui::Button::new("About").min_size(btn_size)).clicked() {
                    self.show_about = true;
                }
                if ui.add(egui::Button::new("OK").min_size(btn_size)).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                let mut text = self.help_text.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(30),
                );
            });
        });

        if self.show_about {
            render_about_modal(ui.ctx(), &mut self.show_about);
        }
    }
}

/// Show help text in an eframe window (used when double-clicking the exe)
fn show_help_window(help_text: String) {
    let viewport = egui::ViewportBuilder::default()
        .with_title("tail-f_plot Usage")
        .with_inner_size([700.0, 520.0])
        .with_icon(Arc::new(load_icon()));
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("tail-f_plot Usage", options, Box::new(move |cc| {
        register_fonts(&cc.egui_ctx);
        Ok(Box::new(HelpApp { help_text, show_about: false }))
    })).ok();
}

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
    #[arg(long, default_value = "Time")]
    x_axis_label: String,

    /// X-axis unit
    #[arg(long, default_value = "min")]
    x_unit: String,

    /// X-axis time scale: d, h, m, s (overrides --x-proportion)
    #[arg(long)]
    x_time_scale: Option<String>,

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

fn exe_name() -> String {
    std::env::current_exe()
        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
        .unwrap_or_else(|_| env!("CARGO_PKG_NAME").to_string())
}

fn help_text() -> String {
    let exe = exe_name();
    format!(
        "HANA tail-f_plot v{ver}\n\n\
        Real-time CSV plotting tool.\n\n\
        Usage:\n\
        {exe} <CSV_PATH> [OPTIONS]\n\n\
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
        --x-axis-label <STR>       X-axis label [default: Time]\n\
        --x-unit <UNIT>            X-axis unit [default: min]\n\
        --x-time-scale <NAME>      Time scale: d,h,m,s (overrides --x-proportion)\n\
        --x-proportion <F>         X proportional constant [default: 1/60]\n\
        --line-width <F>           Line width [default: 1.5]\n\
        --colors <#RRGGBB,...>     Series colors (empty=default 12)\n\
        --max-points <N>           Max points per series [default: 5000]\n\
        --max-x-range <F>          Max X range [default: 120.0]\n\
          (max-points / max-x-range: stricter of the two applies)\n\
        --marker-radius <F>        Marker radius [default: 4.0]\n\n\
        Example:\n\
        {exe} data.csv --y-cols 2,3 --y-names \"Temp,Pressure\"\n\
        {exe} data.tsv --delimiter tab --y-cols 2,3",
        ver = env!("FULL_VERSION"),
        exe = exe
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
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use std::mem;

    unsafe extern "system" fn cb(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> i32 {
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
    // If launched by double-click, hide the auto-allocated console window immediately
    // so it never visibly flashes.  CLI launches (cmd / PowerShell / Git Bash) are
    // unaffected — their console belongs to the shell.
    let owns_console = hide_own_console();

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
                let help = help_text();
                if owns_console {
                    show_help_window(help);
                } else {
                    println!("{}", help);
                }
                std::process::exit(if cli.help { 0 } else { 1 });
            }
            cli
        }
        Err(e) => {
            // --version: clap 기본 동작 유지
            if e.kind() == clap::error::ErrorKind::DisplayVersion {
                if !owns_console {
                    println!("{}", e);
                }
                std::process::exit(0);
            }
            let help = help_text();
            if owns_console {
                show_help_window(help);
            } else {
                let msg = e.to_string();
                let filtered: String = msg.lines()
                    .filter(|l| !l.starts_with("Usage:"))
                    .collect::<Vec<_>>()
                    .join("\n");
                println!("EXCEPTION: {}\n\nRun `{} --help` for usage.", filtered.trim(), exe_name());
            }
            std::process::exit(1);
        }
    };

    println!("PID: {}", std::process::id());
    let csv_path = cli.csv_path.as_ref().unwrap();
    if csv_path.exists() {
        println!("WATCHING: {}", csv_path.display());
    } else {
        println!("WAITING: {}", csv_path.display());
    }

    // Resolve x_time_scale → x_proportion
    let mut cli = cli;
    if let Some(ref scale) = cli.x_time_scale {
        cli.x_proportion = match scale.as_str() {
            "d" => 1.0 / 86400.0,
            "h" => 1.0 / 3600.0,
            "m" => 1.0 / 60.0,
            "s" => 1.0,
            _ => cli.x_proportion,
        };
    }

    let csv_filename = cli
        .csv_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let names_str = cli.y_names.join(",");
    let title = if names_str.is_empty() {
        format!("{} - HANA tail-f_plot - v{}", csv_filename, env!("FULL_VERSION"))
    } else {
        format!("{} - {} - HANA tail-f_plot - v{}", names_str, csv_filename, env!("FULL_VERSION"))
    };

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

    // Free the hidden console before launching the GUI (double-click case).
    // For CLI launches this is a no-op since we don't own the console.
    if owns_console {
        #[cfg(windows)]
        unsafe { windows_sys::Win32::System::Console::FreeConsole(); }
    }

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
