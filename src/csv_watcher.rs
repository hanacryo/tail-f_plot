//! Tails one CSV/TSV file into a `PlotData` group.
//!
//! The file is not held open: every poll opens it by path, checks that it is
//! still the same file with the same content up to where reading stopped,
//! reads only what was appended, and closes it again. A held handle could
//! keep a writer from deleting and re-creating the file, and could never
//! notice the path being replaced.
//!
//! New data is always read from its end backwards, and only the newest
//! `max_points` parsable rows are kept — the most `views` ever renders —
//! so a huge file (or a huge burst appended while the window was minimized)
//! costs as much as the visible window, like `tail`. The skipped head is
//! only scanned forward for the first row's x (the shared display origin)
//! and, when files are aligned by epoch time, the first valid epoch.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;

use eframe::egui;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::groups::GroupConfig;
use crate::plot_state::PlotData;

/// Backward read chunk size.
const CHUNK: u64 = 64 * 1024;
/// Lines longer than this are skipped instead of buffered whole.
const LINE_CAP: usize = 1 << 20;
/// Bytes compared at the start of the file / right before the read position
/// to detect a file rewritten in place (same identity, size not shrunk).
const HEAD_FP: u64 = 256;
const TAIL_FP: u64 = 64;

/// One parsed data row: (scaled x, scaled epoch, y per configured column).
type Row = (f64, Option<f64>, Vec<Option<f64>>);

/// What the app needs from a file besides its newest rows.
#[derive(Debug, Clone, Copy)]
pub struct LoadPolicy {
    /// Parsable rows kept per file: `--max-points`, at least 1 (the newest
    /// row anchors the `--max-x-range` window of every file).
    pub keep_rows: usize,
    /// The first parsable row's x is the shared display origin (first file,
    /// relative x) and must be found even when the head is skipped.
    pub need_origin: bool,
    /// The first row with a valid epoch aligns files by wall-clock time.
    pub need_anchor: bool,
}

pub struct CsvWatcher {
    path: PathBuf,
    x_col: usize,          // 0-based
    y_cols: Vec<usize>,    // 0-based
    data_start_row: usize, // 1-based
    delimiter: String,
    fixed_widths: Vec<usize>,
    string_quote: Option<char>,
    merge_delimiter: bool,
    x_proportion: f64,
    epoch_col: Option<usize>, // 0-based
    epoch_proportion: f64,
    policy: LoadPolicy,
    /// Identity of the file the loaded rows came from.
    identity: Option<(u64, u64)>,
    /// Byte offset of the first data row, once the header is complete.
    data_start: Option<u64>,
    /// Byte offset right after the last complete line read.
    pos: u64,
    /// File bytes [0, min(HEAD_FP, pos)) and [pos - min(TAIL_FP, pos), pos)
    /// as last read.
    head_fp: Vec<u8>,
    tail_fp: Vec<u8>,
    /// The file has been opened and read through to its end at least once.
    loaded: bool,
    /// The last poll could not confirm that its rows and its fingerprints
    /// come from one version of the file: read it again from scratch.
    stale: bool,
    _watcher: Option<RecommendedWatcher>,
}

impl CsvWatcher {
    pub fn new(cfg: &GroupConfig, policy: LoadPolicy) -> Self {
        Self {
            // Absolute: the parent of a bare file name is "", which cannot
            // be watched.
            path: std::path::absolute(&cfg.csv_path).unwrap_or_else(|_| cfg.csv_path.clone()),
            x_col: cfg.x_col.saturating_sub(1),
            y_cols: cfg.y_cols.iter().map(|c| c.saturating_sub(1)).collect(),
            data_start_row: cfg.data_start_row,
            delimiter: cfg.delimiter.clone(),
            fixed_widths: cfg.fixed_width.clone(),
            string_quote: cfg.string_quote,
            merge_delimiter: cfg.merge_delimiter,
            x_proportion: cfg.x_proportion,
            epoch_col: cfg.epochtime_col.checked_sub(1),
            epoch_proportion: cfg.epoch_proportion(),
            policy,
            identity: None,
            data_start: None,
            pos: 0,
            head_fp: Vec::new(),
            tail_fp: Vec::new(),
            loaded: false,
            stale: false,
            _watcher: None,
        }
    }

    /// Watch the file's directory and repaint as soon as the file changes,
    /// instead of waiting for the next poll interval. Polling stays the
    /// source of truth (and the fallback when watching is impossible).
    pub fn start(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.path.parent() else { return };
        let Some(name) = self.path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
            return;
        };
        let ctx = ctx.clone();
        let handler = move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };
            // Other files in the same directory (e.g. other instances' data)
            // must not wake this window. Windows file names are
            // case-insensitive.
            let ours = event.paths.iter().any(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase() == name)
            });
            if ours {
                // Explicitly the main window: this runs on notify's thread.
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        };
        if let Ok(mut w) = notify::recommended_watcher(handler) {
            if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                self._watcher = Some(w);
            }
        }
    }

    /// True once the file has been opened and read to its end at least once
    /// (a trailing line without its newline is still pending).
    pub fn initial_load_done(&self) -> bool {
        self.loaded
    }

    /// Read whatever is new into `data`'s group `group_idx`. A file that was
    /// replaced, truncated or rewritten is re-read from scratch. A file that
    /// cannot be opened keeps the rows already shown (like `tail -F`).
    /// Returns true if any rows were added.
    pub fn poll(&mut self, data: &mut PlotData, group_idx: usize) -> bool {
        let Ok(mut file) = File::open(&self.path) else {
            return false;
        };
        match self.poll_file(&mut file, data, group_idx) {
            Ok(added) => added,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[CsvWatcher] read error on {:?}: {}", self.path, _e);
                false
            }
        }
        // `file` is closed here: no handle is held between polls.
    }

    fn poll_file(&mut self, file: &mut File, data: &mut PlotData, group_idx: usize) -> io::Result<bool> {
        let len = file.metadata()?.len();
        let identity = file_identity(file);

        // The tail fingerprint ends at `pos`; a state where it cannot is
        // inconsistent and read again.
        let tail_off = self.pos.checked_sub(self.tail_fp.len() as u64);
        let replaced = self.loaded
            && (self.stale
                || identity != self.identity
                || len < self.pos
                || tail_off.is_none()
                || read_at(file, 0, self.head_fp.len())? != self.head_fp
                || read_at(file, tail_off.unwrap_or(0), self.tail_fp.len())? != self.tail_fp);
        self.stale = false;
        if replaced {
            #[cfg(debug_assertions)]
            eprintln!("[CsvWatcher] file replaced or rewritten, reloading: {:?}", self.path);
            data.reset_group(group_idx);
            self.data_start = None;
            self.pos = 0;
            // Consistent with pos 0 even if this poll fails further on: the
            // next one then resumes the reload instead of comparing against
            // the old file's fingerprints.
            self.head_fp.clear();
            self.tail_fp.clear();
        }
        // Fingerprints just confirmed, to check after the load that the file
        // did not change under it
        let confirmed = (self.loaded && !replaced).then(|| {
            (tail_off.unwrap_or(0), self.head_fp.clone(), self.tail_fp.clone())
        });
        if !self.loaded || replaced {
            self.identity = identity;
            #[cfg(debug_assertions)]
            eprintln!("[CsvWatcher] File opened: {:?}", self.path);
        }

        if self.data_start.is_none() {
            self.data_start = self.find_data_start(file, len)?;
        }
        let added = match self.data_start {
            Some(start) => {
                // From here on the header is part of the fingerprinted
                // content, even before the first data line is complete.
                self.pos = self.pos.max(start);
                self.load_range(file, self.pos, len, data, group_idx)?
            }
            None => false, // header still being written
        };

        // The fingerprints are read after the rows. A writer that truncated
        // or rewrote the file meanwhile shows up as a short read, or as the
        // confirmed fingerprints no longer matching: then rows and
        // fingerprints may come from different versions, and the next poll
        // reads the file again instead of appending to mixed rows.
        let head_len = self.pos.min(HEAD_FP);
        let tail_len = self.pos.min(TAIL_FP);
        self.head_fp = read_at(file, 0, head_len as usize)?;
        self.tail_fp = read_at(file, self.pos - tail_len, tail_len as usize)?;
        let complete =
            self.head_fp.len() as u64 == head_len && self.tail_fp.len() as u64 == tail_len;
        let unchanged = match &confirmed {
            Some((off, head, tail)) => {
                read_at(file, 0, head.len())? == *head && read_at(file, *off, tail.len())? == *tail
            }
            None => true,
        };
        self.stale = !(complete && unchanged);
        self.loaded = true;
        Ok(added)
    }

    /// Byte offset of the first data row (line `data_start_row`), past a
    /// UTF-8 BOM when the data starts on the first line. None while the file
    /// has fewer complete lines than the header needs.
    fn find_data_start(&self, file: &mut File, len: u64) -> io::Result<Option<u64>> {
        let skip_lines = self.data_start_row.saturating_sub(1);
        file.seek(SeekFrom::Start(0))?;
        let mut reader = BufReader::new(Read::by_ref(file).take(len));
        let mut offset = 0u64;
        let mut buf = Vec::new();
        for _ in 0..skip_lines {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 || buf.last() != Some(&b'\n') {
                return Ok(None);
            }
            offset += n as u64;
        }
        drop(reader);
        if offset == 0 {
            const BOM: &[u8] = b"\xEF\xBB\xBF";
            let head = read_at(file, 0, BOM.len().min(len as usize))?;
            if head == BOM {
                offset = 3;
            } else if BOM.starts_with(&head) {
                // Empty, or a BOM still being written: deciding now would
                // keep a BOM written later in the first row's x cell.
                return Ok(None);
            }
        }
        Ok(Some(offset))
    }

    /// Load the complete lines in `[lo, len)`: the newest `keep_rows`
    /// parsable rows, plus origin / epoch anchor seeds from the lines before
    /// them when still missing. Advances `pos` past the last complete line.
    fn load_range(
        &mut self,
        file: &mut File,
        lo: u64,
        len: u64,
        data: &mut PlotData,
        group_idx: usize,
    ) -> io::Result<bool> {
        let Some(end) = last_line_end(file, lo, len, CHUNK)? else {
            return Ok(false); // no complete line yet
        };

        // Newest rows first, until enough are collected.
        let keep = self.policy.keep_rows.max(1);
        let mut rows: Vec<Row> = Vec::new();
        let mut tail_start = lo;
        scan_lines_back(file, lo, end, CHUNK, LINE_CAP, |line, start| {
            if let Some(row) = self.parse_line(line) {
                rows.push(row);
                if rows.len() >= keep {
                    tail_start = start;
                    return false;
                }
            }
            true
        })?;

        // Seeds from the skipped lines, only while still missing.
        let (no_origin, no_anchor) = data.needs_seed(group_idx);
        let want_origin = no_origin && self.policy.need_origin;
        let want_anchor = no_anchor && self.policy.need_anchor;
        if tail_start > lo && (want_origin || want_anchor) {
            let (origin, anchor) = self.scan_seeds(file, lo, tail_start, want_origin, want_anchor)?;
            data.seed_group(group_idx, origin, anchor);
        }

        #[cfg(debug_assertions)]
        if !rows.is_empty() {
            eprintln!("[CsvWatcher] {} new row(s) loaded", rows.len());
        }
        let added = !rows.is_empty();
        for (x, epoch, ys) in rows.into_iter().rev() {
            data.push_row(group_idx, x, epoch, &ys);
        }
        data.evict_old_rows(group_idx, keep);
        self.pos = end;
        Ok(added)
    }

    /// Forward scan of the complete lines in `[lo, hi)` for the first
    /// parsable row's x and the first row with a valid epoch.
    fn scan_seeds(
        &self,
        file: &mut File,
        lo: u64,
        hi: u64,
        mut want_origin: bool,
        mut want_anchor: bool,
    ) -> io::Result<(Option<f64>, Option<(f64, f64)>)> {
        let mut origin = None;
        let mut anchor = None;
        file.seek(SeekFrom::Start(lo))?;
        let mut reader = BufReader::new(Read::by_ref(file).take(hi - lo));
        let mut buf = Vec::new();
        while want_origin || want_anchor {
            buf.clear();
            if reader.read_until(b'\n', &mut buf)? == 0 {
                break;
            }
            let Some((x, epoch, _)) = self.parse_line(&buf) else { continue };
            if want_origin {
                origin = Some(x);
                want_origin = false;
            }
            if want_anchor {
                if let Some(e) = epoch {
                    anchor = Some((x, e));
                    want_anchor = false;
                }
            }
        }
        Ok((origin, anchor))
    }

    /// Parse one raw line (trailing newline included or not). None for a
    /// blank line or a row whose x is missing / not a finite number.
    /// Bytes that are not valid UTF-8 (e.g. a CP949 text column) are
    /// replaced, so the numeric columns of such a row still parse.
    fn parse_line(&self, line: &[u8]) -> Option<Row> {
        let line = trim_newline(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        self.with_cells(line, |cell| self.parse_cells(cell))
    }

    /// Split a line (newline removed) into cells and hand `f` a lookup of
    /// cell `col` (0-based: trimmed, quotes stripped; None past the last).
    /// Data rows and the header row are split by this one rule.
    fn with_cells<R>(
        &self,
        line: &[u8],
        f: impl for<'c> FnOnce(&'c dyn Fn(usize) -> Option<&'c str>) -> R,
    ) -> R {
        let text;
        let fields: Vec<Cow<'_, str>> = if !self.fixed_widths.is_empty() {
            // Fixed-width mode: columns are byte positions in the raw line,
            // so leading spaces (right-aligned columns) must stay.
            let mut result = Vec::with_capacity(self.fixed_widths.len());
            let mut pos: usize = 0;
            for &width in &self.fixed_widths {
                // Saturating: a huge width must end at the line, not wrap
                // around to before `pos`
                let end = pos.saturating_add(width).min(line.len());
                result.push(String::from_utf8_lossy(&line[pos..end]));
                pos = end;
            }
            result
        } else {
            // Delimiter mode
            text = String::from_utf8_lossy(line);
            let raw = text.trim().split(&*self.delimiter);
            if self.merge_delimiter {
                raw.filter(|s| !s.trim().is_empty()).map(Cow::Borrowed).collect()
            } else {
                raw.map(Cow::Borrowed).collect()
            }
        };

        let quote = self.string_quote;
        let cell = |col: usize| -> Option<&str> {
            let s = fields.get(col)?.trim();
            Some(match quote {
                Some(q) => s.trim_matches(q),
                None => s,
            })
        };
        f(&cell)
    }

    fn parse_cells<'c>(&self, cell: &'c dyn Fn(usize) -> Option<&'c str>) -> Option<Row> {
        let x: f64 = cell(self.x_col)?.parse().ok()?;
        // "nan"/"inf" parse fine but cannot be plotted, and a non-finite x
        // would poison the sorted-x tracking and every range scan built on it.
        if !x.is_finite() {
            return None;
        }
        let x = x * self.x_proportion;

        // Epoch-time column: a row whose epoch cell is missing or invalid is
        // still plotted; the epoch anchor just waits for the first valid one.
        let epoch = self
            .epoch_col
            .and_then(|col| cell(col)?.parse::<f64>().ok())
            .map(|v| v * self.epoch_proportion);

        let ys: Vec<Option<f64>> = self
            .y_cols
            .iter()
            .map(|&col| match cell(col) {
                Some(s) if !s.is_empty() => s.parse::<f64>().ok(),
                _ => None,
            })
            .collect();

        Some((x, epoch, ys))
    }
}

/// Cells of the header row (`--header-row`, 1-based; 0 = none) of a file,
/// split like its data rows. Reads lines up to the header row (at most
/// `LINE_CAP` bytes) and closes the file again; a file with fewer lines has
/// no header yet.
pub fn read_header(cfg: &GroupConfig) -> io::Result<Vec<String>> {
    if cfg.header_row == 0 {
        return Ok(Vec::new());
    }
    let file = File::open(&cfg.csv_path)?;
    let mut reader = BufReader::new(file.take(LINE_CAP as u64));
    let mut line = Vec::new();
    for _ in 0..cfg.header_row {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(Vec::new());
        }
    }
    let mut line = trim_newline(&line);
    if cfg.header_row == 1 {
        line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
    }
    let policy = LoadPolicy { keep_rows: 1, need_origin: false, need_anchor: false };
    let splitter = CsvWatcher::new(cfg, policy);
    Ok(splitter.with_cells(line, |cell| {
        (0..).map_while(|col| cell(col).map(String::from)).collect()
    }))
}

/// Strip one trailing "\n" or "\r\n".
fn trim_newline(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Read `n` bytes at `offset` (fewer only at end of file).
fn read_at(file: &mut File, offset: u64, n: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::with_capacity(n);
    Read::by_ref(file).take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Offset right after the last '\n' in `[lo, hi)`; None if there is none.
fn last_line_end(file: &mut File, lo: u64, hi: u64, chunk: u64) -> io::Result<Option<u64>> {
    let mut end = hi;
    while end > lo {
        let start = end.saturating_sub(chunk).max(lo);
        let buf = read_at(file, start, (end - start) as usize)?;
        if let Some(i) = buf.iter().rposition(|&b| b == b'\n') {
            return Ok(Some(start + i as u64 + 1));
        }
        end = start;
    }
    Ok(None)
}

/// Visit the complete lines of `[lo, hi)` newest first, as
/// `f(line_bytes, line_start_offset)` (the bytes include the newline).
/// `hi` must sit right after a '\n' or equal `lo`. `f` returns false to
/// stop. Reads `chunk_size` bytes at a time; a line longer than `line_cap`
/// is skipped without being held whole.
fn scan_lines_back(
    file: &mut File,
    lo: u64,
    hi: u64,
    chunk_size: u64,
    line_cap: usize,
    mut f: impl FnMut(&[u8], u64) -> bool,
) -> io::Result<()> {
    // Current line: ends (exclusive, after its '\n') at `line_end`; `tail`
    // holds its bytes already read, i.e. [pos, line_end).
    let mut line_end = hi;
    let mut tail: Vec<u8> = Vec::new();
    let mut too_long = false;
    let mut pos = hi;
    while pos > lo {
        let start = pos.saturating_sub(chunk_size).max(lo);
        let chunk = read_at(file, start, (pos - start) as usize)?;
        // chunk[..rel_end] is not consumed yet
        let mut rel_end = chunk.len();
        loop {
            // The current line's own terminator is not a boundary.
            let limit = if start + rel_end as u64 == line_end { rel_end - 1 } else { rel_end };
            match chunk[..limit].iter().rposition(|&b| b == b'\n') {
                Some(i) => {
                    let line_start = start + i as u64 + 1;
                    let line_len = rel_end - (i + 1) + tail.len();
                    if !too_long && line_len <= line_cap {
                        let keep_going = if tail.is_empty() {
                            f(&chunk[i + 1..rel_end], line_start)
                        } else {
                            let mut line = chunk[i + 1..rel_end].to_vec();
                            line.extend_from_slice(&tail);
                            f(&line, line_start)
                        };
                        if !keep_going {
                            return Ok(());
                        }
                    }
                    tail.clear();
                    too_long = false;
                    line_end = line_start;
                    rel_end = i + 1;
                }
                None => {
                    // The rest belongs to the current line, which starts in
                    // an earlier chunk (or at `lo`).
                    if !too_long {
                        let mut head = chunk[..rel_end].to_vec();
                        head.extend_from_slice(&tail);
                        tail = head;
                        if tail.len() > line_cap {
                            too_long = true;
                            tail = Vec::new();
                        }
                    }
                    break;
                }
            }
        }
        pos = start;
    }
    // The first line of the range starts at `lo`.
    if line_end > lo && !too_long && tail.len() <= line_cap {
        f(&tail, lo);
    }
    Ok(())
}

/// (volume serial, file index): changes when the path is re-created or
/// replaced by a rename. None if it cannot be queried (size and content
/// fingerprints still catch truncation and rewrites).
#[cfg(windows)]
fn file_identity(file: &File) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return None;
    }
    Some((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}

#[cfg(unix)]
fn file_identity(file: &File) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().ok()?;
    Some((m.dev(), m.ino()))
}

#[cfg(not(any(windows, unix)))]
fn file_identity(_file: &File) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A scratch file unique to one test.
    struct TempFile(PathBuf);
    impl TempFile {
        fn new(name: &str, content: &[u8]) -> Self {
            let dir = std::env::temp_dir().join(format!("tailf_test_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let t = TempFile(dir.join(name));
            t.write(content);
            t
        }
        fn write(&self, content: &[u8]) {
            std::fs::write(&self.0, content).unwrap();
        }
        fn append(&self, content: &[u8]) {
            let mut f = std::fs::OpenOptions::new().append(true).open(&self.0).unwrap();
            f.write_all(content).unwrap();
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn cfg(path: &std::path::Path) -> GroupConfig {
        GroupConfig {
            csv_path: path.to_path_buf(),
            x_col: 1,
            y_cols: vec![2],
            y_names: Vec::new(),
            y_stacks: Vec::new(),
            header_row: 1,
            data_start_row: 2,
            delimiter: ",".into(),
            string_quote: None,
            merge_delimiter: false,
            fixed_width: Vec::new(),
            x_proportion: 1.0,
            epochtime_col: 0,
            epochtime_proportion: None,
        }
    }

    #[test]
    fn header_is_split_like_data_rows() {
        let t = TempFile::new("header_split.csv", b"\xEF\xBB\xBFtime,\"temp K\", P \r\n1,2,3\r\n");
        let mut c = cfg(&t.0);
        c.string_quote = Some('"');
        assert_eq!(read_header(&c).unwrap(), ["time", "temp K", "P"]);

        // Header on row 2 (no BOM handling there), merged delimiters
        t.write(b"# comment\nA  B   C\n1 2 3\n");
        c.header_row = 2;
        c.delimiter = " ".into();
        c.merge_delimiter = true;
        assert_eq!(read_header(&c).unwrap(), ["A", "B", "C"]);

        // Fixed widths cut the raw line
        t.write(b"   x  y1  y2\n");
        c.header_row = 1;
        c.fixed_width = vec![4, 4, 4];
        assert_eq!(read_header(&c).unwrap(), ["x", "y1", "y2"]);

        // Fewer lines than the header row, or no header row at all
        c.header_row = 5;
        assert!(read_header(&c).unwrap().is_empty());
        c.header_row = 0;
        assert!(read_header(&c).unwrap().is_empty());

        // A missing file is an error
        c.header_row = 1;
        c.csv_path = t.0.with_file_name("missing.csv");
        assert!(read_header(&c).is_err());
    }

    #[test]
    fn a_bom_written_after_an_empty_first_poll_is_still_skipped() {
        let t = TempFile::new("bom_late.csv", b"");
        let mut c = cfg(&t.0);
        c.header_row = 0;
        c.data_start_row = 1;
        let (mut w, mut d) = setup(&c, 100);
        assert!(!w.poll(&mut d, 0));
        t.append(b"\xEF\xBB"); // BOM half written
        assert!(!w.poll(&mut d, 0));
        t.append(b"\xBF1.5,2\n2.5,3\n");
        assert!(w.poll(&mut d, 0));
        assert_eq!(xs(&d), [1.5, 2.5]);
    }

    /// A reload that failed half-way used to leave pos reset next to the old
    /// file's fingerprints: the next poll underflowed (panic in debug, stuck
    /// in release).
    #[test]
    fn inconsistent_fingerprints_reload_instead_of_underflowing() {
        let t = TempFile::new("fp_state.csv", b"x,y\n1,1\n2,2\n");
        let c = cfg(&t.0);
        let (mut w, mut d) = setup(&c, 100);
        assert!(w.poll(&mut d, 0));
        w.pos = 0;
        w.data_start = None;
        assert!(!w.tail_fp.is_empty());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), [1.0, 2.0]);
    }

    #[test]
    fn a_stale_poll_reads_the_file_again_without_duplicates() {
        let t = TempFile::new("stale.csv", b"x,y\n1,1\n2,2\n");
        let c = cfg(&t.0);
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        w.stale = true;
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), [1.0, 2.0]);
        assert!(!w.stale);
    }

    #[test]
    fn a_huge_fixed_width_ends_at_the_line() {
        let t = TempFile::new("fw_huge.csv", b"1 2\n");
        let mut c = cfg(&t.0);
        c.header_row = 0;
        c.data_start_row = 1;
        c.fixed_width = vec![1, usize::MAX];
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), [(1.0, Some(2.0))]);
    }

    fn setup(c: &GroupConfig, keep: usize) -> (CsvWatcher, PlotData) {
        let mut d = PlotData::new(false);
        d.add_group(&["y".into()], c.epochtime_col > 0);
        let policy = LoadPolicy { keep_rows: keep, need_origin: true, need_anchor: true };
        (CsvWatcher::new(c, policy), d)
    }

    /// (x, y) of every stored row of group 0.
    fn rows(d: &PlotData) -> Vec<(f64, Option<f64>)> {
        d.groups[0].series[0].points.clone()
    }

    fn xs(d: &PlotData) -> Vec<f64> {
        rows(d).iter().map(|r| r.0).collect()
    }

    /// "x,y" header + rows (i, 10·i).
    fn csv(range: std::ops::Range<i32>) -> String {
        let mut s = String::from("x,y\n");
        for i in range {
            s.push_str(&format!("{},{}\n", i, i * 10));
        }
        s
    }

    #[test]
    fn scan_back_matches_forward_split_for_any_chunk_size() {
        let content = "a\n\nbb,ccc\nd\r\n\u{d55c}\u{ae00},1\nlast\n";
        let t = TempFile::new("chunks.csv", content.as_bytes());
        let mut expected: Vec<(Vec<u8>, u64)> = Vec::new();
        let mut off = 0u64;
        for l in content.split_inclusive('\n') {
            expected.push((l.as_bytes().to_vec(), off));
            off += l.len() as u64;
        }
        expected.reverse();
        for chunk in [1u64, 2, 3, 5, 7, 13, 64 * 1024] {
            let mut f = File::open(&t.0).unwrap();
            let mut got = Vec::new();
            scan_lines_back(&mut f, 0, content.len() as u64, chunk, LINE_CAP, |l, s| {
                got.push((l.to_vec(), s));
                true
            })
            .unwrap();
            assert_eq!(got, expected, "chunk {chunk}");
        }
        // A sub-range starting mid-file
        let mut f = File::open(&t.0).unwrap();
        let mut got = Vec::new();
        scan_lines_back(&mut f, 3, 10, 2, LINE_CAP, |l, s| {
            got.push((l.to_vec(), s));
            true
        })
        .unwrap();
        assert_eq!(got, vec![(b"bb,ccc\n".to_vec(), 3)]);
    }

    #[test]
    fn scan_back_skips_over_long_lines() {
        let long = "x".repeat(50);
        let content = format!("a\n{long}\nb\n");
        let t = TempFile::new("long.csv", content.as_bytes());
        for chunk in [1u64, 4, 64] {
            let mut f = File::open(&t.0).unwrap();
            let mut got = Vec::new();
            scan_lines_back(&mut f, 0, content.len() as u64, chunk, 10, |l, _| {
                got.push(l.to_vec());
                true
            })
            .unwrap();
            assert_eq!(got, vec![b"b\n".to_vec(), b"a\n".to_vec()], "chunk {chunk}");
        }
        // An over-long first line is skipped too.
        let content = format!("{long}\nb\n");
        let t = TempFile::new("long2.csv", content.as_bytes());
        let mut f = File::open(&t.0).unwrap();
        let mut got = Vec::new();
        scan_lines_back(&mut f, 0, content.len() as u64, 4, 10, |l, _| {
            got.push(l.to_vec());
            true
        })
        .unwrap();
        assert_eq!(got, vec![b"b\n".to_vec()]);
    }

    #[test]
    fn last_line_end_finds_the_last_newline() {
        let t = TempFile::new("lle.csv", b"ab\ncd\nef");
        let mut f = File::open(&t.0).unwrap();
        for chunk in [1u64, 2, 64] {
            assert_eq!(last_line_end(&mut f, 0, 8, chunk).unwrap(), Some(6));
            assert_eq!(last_line_end(&mut f, 6, 8, chunk).unwrap(), None);
        }
    }

    #[test]
    fn tail_load_keeps_newest_rows_and_the_true_origin() {
        let t = TempFile::new("tail.csv", csv(0..100).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 5);
        assert!(!w.initial_load_done());
        assert!(w.poll(&mut d, 0));
        assert_eq!(
            rows(&d),
            (95..100).map(|i| (i as f64, Some(i as f64 * 10.0))).collect::<Vec<_>>()
        );
        // Origin = the file's first data row, not the first loaded one.
        assert_eq!(d.groups[0].origin_x, Some(0.0));
        assert!(w.initial_load_done());
    }

    #[test]
    fn appends_continue_exactly_including_split_lines() {
        let t = TempFile::new("append.csv", csv(0..3).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 100);
        w.poll(&mut d, 0);
        t.append(b"3,30\n4,4");
        w.poll(&mut d, 0); // "4,4" has no newline yet: not a row
        assert_eq!(xs(&d), vec![0.0, 1.0, 2.0, 3.0]);
        t.append(b"0\r");
        w.poll(&mut d, 0); // a bare CR does not complete a line either
        assert_eq!(xs(&d).len(), 4);
        t.append(b"\n");
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rows(&d)[4].1, Some(40.0));
        assert!(!w.poll(&mut d, 0)); // nothing new
    }

    #[test]
    fn a_big_append_loads_only_the_newest_rows() {
        let t = TempFile::new("burst.csv", csv(0..3).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 4);
        w.poll(&mut d, 0);
        let mut more = String::new();
        for i in 3..1000 {
            more.push_str(&format!("{},{}\n", i, i * 10));
        }
        t.append(more.as_bytes());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), vec![0.0, 1.0, 2.0, 996.0, 997.0, 998.0, 999.0]);
        // What views renders is unchanged by the skipped rows.
        let v = d.views(4, 1e18)[0].unwrap();
        assert_eq!(&xs(&d)[v.range.0..v.range.1], &[996.0, 997.0, 998.0, 999.0]);
        assert_eq!(d.groups[0].origin_x, Some(0.0));
    }

    #[test]
    fn truncation_reloads() {
        let t = TempFile::new("trunc.csv", csv(0..10).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 100);
        w.poll(&mut d, 0);
        t.write(csv(50..52).as_bytes());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), vec![50.0, 51.0]);
        assert_eq!(d.groups[0].origin_x, Some(50.0));
    }

    #[test]
    fn in_place_rewrite_of_equal_or_greater_length_reloads() {
        let t = TempFile::new("rewrite.csv", csv(10..20).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 100);
        w.poll(&mut d, 0);
        // Same length, different content.
        t.write(csv(30..40).as_bytes());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), (30..40).map(f64::from).collect::<Vec<_>>());
        // Longer, different content.
        t.write(csv(100..130).as_bytes());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), (100..130).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn delete_and_recreate_reloads_and_missing_file_keeps_rows() {
        let t = TempFile::new("recreate.csv", csv(0..5).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 100);
        w.poll(&mut d, 0);
        std::fs::remove_file(&t.0).unwrap();
        assert!(!w.poll(&mut d, 0));
        assert_eq!(xs(&d).len(), 5); // still shown while missing
        // Re-created with an identical prefix: only the identity tells.
        t.write(csv(0..8).as_bytes());
        w.poll(&mut d, 0);
        assert_eq!(xs(&d), (0..8).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn header_still_being_written_waits() {
        let t = TempFile::new("header.csv", b"# title\n");
        let mut c = cfg(&t.0);
        c.data_start_row = 3;
        let (mut w, mut d) = setup(&c, 100);
        assert!(!w.poll(&mut d, 0));
        assert!(w.initial_load_done());
        t.append(b"x,y\n1,10\n");
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.0, Some(10.0))]);
    }

    #[test]
    fn numeric_header_lines_are_never_data() {
        let t = TempFile::new("numhdr.csv", b"5,50\n6,60\n1,10\n");
        let mut c = cfg(&t.0);
        c.data_start_row = 3;
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.0, Some(10.0))]);
    }

    #[test]
    fn bom_on_the_first_data_line_is_skipped() {
        let t = TempFile::new("bom.csv", b"\xEF\xBB\xBF1.5,2\n2.5,3\n");
        let mut c = cfg(&t.0);
        c.data_start_row = 1;
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.5, Some(2.0)), (2.5, Some(3.0))]);
        assert_eq!(d.groups[0].origin_x, Some(1.5));
    }

    #[test]
    fn non_utf8_text_columns_do_not_drop_rows() {
        // CP949 bytes in a text column (and in the header).
        let t = TempFile::new("cp949.csv", b"x,\xc7\xd1,y\n1,\xb1\xdb,10\n2,\xc0\xda,20\n");
        let mut c = cfg(&t.0);
        c.y_cols = vec![3];
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.0, Some(10.0)), (2.0, Some(20.0))]);
    }

    #[test]
    fn fixed_width_keeps_leading_spaces() {
        // Right-aligned columns: trimming the line would shift every column.
        let t = TempFile::new("fixed.txt", b"hdr\n   1  10\n   2  20\n");
        let mut c = cfg(&t.0);
        c.fixed_width = vec![4, 4];
        let (mut w, mut d) = setup(&c, 100);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.0, Some(10.0)), (2.0, Some(20.0))]);
    }

    #[test]
    fn delimiter_mode_parsing_is_unchanged() {
        // Leading/trailing blanks around the line are trimmed, quotes and
        // merged delimiters as before; an empty cell is a gap.
        let t = TempFile::new("delim.csv", b"h\n  1 ;'2';; \n");
        let mut c = cfg(&t.0);
        c.delimiter = ";".into();
        c.string_quote = Some('\'');
        c.y_cols = vec![2, 3];
        let (mut w, mut d) = setup(&c, 100);
        d.groups[0].series.push(crate::plot_state::SeriesData::new("y2".into()));
        w.poll(&mut d, 0);
        assert_eq!(d.groups[0].series[0].points, vec![(1.0, Some(2.0))]);
        assert_eq!(d.groups[0].series[1].points, vec![(1.0, None)]);
    }

    #[test]
    fn epoch_anchor_comes_from_the_skipped_head() {
        // Epoch cells are empty in the first rows; the first valid one is
        // row x=2 — far before the loaded tail.
        let mut s = String::from("x,y,e\n0,0,\n1,0,\n2,0,1002\n");
        for i in 3..50 {
            s.push_str(&format!("{i},0,{}\n", 1000 + i));
        }
        let t = TempFile::new("epoch.csv", s.as_bytes());
        let mut c = cfg(&t.0);
        c.epochtime_col = 3;
        let (mut w, mut d) = setup(&c, 3);
        w.poll(&mut d, 0);
        assert_eq!(d.groups[0].epoch_anchor, Some((2.0, 1002.0)));
        assert_eq!(d.groups[0].origin_x, Some(0.0));
        assert_eq!(xs(&d), vec![47.0, 48.0, 49.0]);
    }

    #[test]
    fn unparsable_rows_do_not_count_toward_kept_rows() {
        let t = TempFile::new("junk.csv", b"x,y\n1,10\n2,20\nbad,row\n\n# note\n");
        let (mut w, mut d) = setup(&cfg(&t.0), 2);
        w.poll(&mut d, 0);
        assert_eq!(rows(&d), vec![(1.0, Some(10.0)), (2.0, Some(20.0))]);
    }

    #[test]
    fn eviction_bounds_memory_to_the_kept_rows() {
        let t = TempFile::new("evict.csv", csv(0..1).as_bytes());
        let (mut w, mut d) = setup(&cfg(&t.0), 10);
        w.poll(&mut d, 0);
        for i in 1..5000 {
            t.append(format!("{},{}\n", i, i).as_bytes());
            if i % 97 == 0 {
                w.poll(&mut d, 0);
            }
        }
        w.poll(&mut d, 0);
        let n = xs(&d).len();
        assert!(n <= 10 + 1024, "{n} rows kept");
        assert_eq!(xs(&d).last(), Some(&4999.0));
        // The views window over the kept rows is still the newest 10 rows.
        let v = d.views(10, 1e18)[0].unwrap();
        assert_eq!(v.range.1 - v.range.0, 10);
        assert_eq!(xs(&d)[v.range.0], 4990.0);
        assert_eq!(d.groups[0].origin_x, Some(0.0));
    }
}
