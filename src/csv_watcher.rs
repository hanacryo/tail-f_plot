use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::groups::GroupConfig;
use crate::plot_state::PlotData;

pub struct CsvWatcher {
    path: PathBuf,
    x_col: usize,       // 0-based
    y_cols: Vec<usize>,  // 0-based
    data_start_row: usize, // 1-based
    reader: Option<BufReader<File>>,
    current_line: usize, // 1-based, next line to read
    incomplete_buf: String,
    _watcher: Option<RecommendedWatcher>,
    notify_rx: Option<mpsc::Receiver<notify::Result<Event>>>,
    delimiter: String,
    fixed_widths: Vec<usize>,
    string_quote: Option<char>,
    merge_delimiter: bool,
    x_proportion: f64,
    epoch_col: Option<usize>, // 0-based
    epoch_proportion: f64,
}

impl CsvWatcher {
    pub fn new(cfg: &GroupConfig) -> Self {
        Self {
            path: cfg.csv_path.clone(),
            x_col: cfg.x_col.saturating_sub(1),
            y_cols: cfg.y_cols.iter().map(|c| c.saturating_sub(1)).collect(),
            data_start_row: cfg.data_start_row,
            reader: None,
            current_line: 1,
            incomplete_buf: String::new(),
            _watcher: None,
            notify_rx: None,
            delimiter: cfg.delimiter.clone(),
            fixed_widths: cfg.fixed_width.clone(),
            string_quote: cfg.string_quote,
            merge_delimiter: cfg.merge_delimiter,
            x_proportion: cfg.x_proportion,
            epoch_col: cfg.epochtime_col.checked_sub(1),
            epoch_proportion: cfg.epoch_proportion(),
        }
    }

    pub fn start(&mut self) {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(tx);
        if let Ok(mut w) = watcher {
            let parent = self.path.parent().unwrap_or(Path::new("."));
            let _ = w.watch(parent, RecursiveMode::NonRecursive);
            self._watcher = Some(w);
        }
        self.notify_rx = Some(rx);
    }

    /// Poll for new data, pushing rows into `data`'s group `group_idx`.
    /// Returns true if any new rows were added.
    pub fn poll(&mut self, data: &mut PlotData, group_idx: usize) -> bool {
        // Drain notify events
        if let Some(rx) = &self.notify_rx {
            while rx.try_recv().is_ok() {}
        }

        // Try to open file if not yet open
        if self.reader.is_none() {
            match File::open(&self.path) {
                Ok(file) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[CsvWatcher] File opened: {:?}", self.path);
                    self.reader = Some(BufReader::new(file));
                    self.current_line = 1;
                    self.incomplete_buf.clear();
                }
                Err(_) => {
                    return false;
                }
            }
        }

        let mut added = false;

        // Collect rows to parse, then parse them after releasing the reader borrow.
        let mut rows_to_parse: Vec<String> = Vec::new();

        {
            let reader = self.reader.as_mut().unwrap();
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if !line.ends_with('\n') && !line.ends_with('\r') {
                            self.incomplete_buf.push_str(&line);
                            break;
                        }

                        let full_line = if self.incomplete_buf.is_empty() {
                            line
                        } else {
                            let mut combined = std::mem::take(&mut self.incomplete_buf);
                            combined.push_str(&line);
                            combined
                        };

                        if self.current_line < self.data_start_row {
                            self.current_line += 1;
                            continue;
                        }

                        let trimmed = full_line.trim().to_string();
                        if !trimmed.is_empty() {
                            rows_to_parse.push(trimmed);
                        }
                        self.current_line += 1;
                    }
                    Err(_) => break,
                }
            }
        }

        #[cfg(debug_assertions)]
        if !rows_to_parse.is_empty() {
            eprintln!("[CsvWatcher] {} new row(s) found", rows_to_parse.len());
        }
        for row in &rows_to_parse {
            if let Some((x, epoch, ys)) = self.parse_row(row) {
                data.push_row(group_idx, x, epoch, &ys);
                added = true;
            }
        }

        added
    }

    fn parse_row(&self, line: &str) -> Option<(f64, Option<f64>, Vec<Option<f64>>)> {
        let fields: Vec<&str> = if !self.fixed_widths.is_empty() {
            // Fixed-width mode
            let mut result = Vec::new();
            let mut pos = 0;
            let bytes = line.as_bytes();
            for &width in &self.fixed_widths {
                if pos >= bytes.len() {
                    result.push("");
                } else {
                    let end = (pos + width).min(bytes.len());
                    // Safe UTF-8 boundary handling
                    result.push(line.get(pos..end).unwrap_or(""));
                    pos = end;
                }
            }
            result
        } else {
            // Delimiter mode
            let raw: Vec<&str> = line.split(&*self.delimiter).collect();
            if self.merge_delimiter {
                raw.into_iter().filter(|s| !s.trim().is_empty()).collect()
            } else {
                raw
            }
        };

        let quote = self.string_quote;

        let x_str = fields.get(self.x_col)?.trim();
        let x_str = match quote {
            Some(q) => x_str.trim_matches(q),
            None => x_str,
        };
        let x: f64 = x_str.parse().ok()?;
        // "nan"/"inf" parse fine but cannot be plotted, and a non-finite x
        // would poison the sorted-x tracking and every range scan built on it.
        if !x.is_finite() {
            return None;
        }
        let x = x * self.x_proportion;

        // Epoch-time column: a row whose epoch cell is missing or invalid is
        // still plotted; the epoch anchor just waits for the first valid one.
        let epoch = self.epoch_col.and_then(|col| {
            let s = fields.get(col)?.trim();
            let s = match quote {
                Some(q) => s.trim_matches(q),
                None => s,
            };
            s.parse::<f64>().ok().map(|v| v * self.epoch_proportion)
        });

        let ys: Vec<Option<f64>> = self.y_cols
            .iter()
            .map(|&col| {
                fields
                    .get(col)
                    .and_then(|s| {
                        let trimmed = s.trim();
                        let trimmed = match quote {
                            Some(q) => trimmed.trim_matches(q),
                            None => trimmed,
                        };
                        if trimmed.is_empty() {
                            Some(None)
                        } else {
                            Some(trimmed.parse::<f64>().ok())
                        }
                    })
                    .unwrap_or(None)
            })
            .collect();

        Some((x, epoch, ys))
    }
}
