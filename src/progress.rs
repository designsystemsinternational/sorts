//! Shows the progress of a scan on stderr, so a scan that downloads online-only files does not look frozen.

use crate::scan::ScanEvent;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The minimum time between two updates of the progress line in a terminal.
const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

pub const OFFLINE_TIP: &str =
    "Tip: Make the Fonts folder available offline in your file sync app. Then sorts does not download the files again.";

pub struct Progress<W> {
    root: PathBuf,
    /// In a terminal, one line is updated in place. Otherwise only the start and the end of the downloads are shown.
    terminal: bool,
    inner: Mutex<Inner<W>>,
}

struct Inner<W> {
    out: W,
    found: usize,
    files: usize,
    bytes: u64,
    done_files: usize,
    done_bytes: u64,
    last_draw: Option<Instant>,
}

impl<W: Write + Send> Progress<W> {
    pub fn new(root: impl Into<PathBuf>, terminal: bool, out: W) -> Self {
        let inner = Inner { out, found: 0, files: 0, bytes: 0, done_files: 0, done_bytes: 0, last_draw: None };
        Self { root: root.into(), terminal, inner: Mutex::new(inner) }
    }

    pub fn event(&self, event: ScanEvent) {
        let mut inner = self.inner.lock().unwrap();
        let force = match event {
            ScanEvent::Found(count) => {
                inner.found = count;
                false
            }
            ScanEvent::Downloading { files, bytes } => {
                (inner.files, inner.bytes) = (files, bytes);
                if !self.terminal {
                    let line = format!("Downloading {} ({})...", files_text(files), megabytes(bytes));
                    let _ = writeln!(inner.out, "{line}");
                }
                true
            }
            ScanEvent::Downloaded { bytes } => {
                inner.done_files += 1;
                inner.done_bytes += bytes;
                false
            }
        };
        if self.terminal && (force || inner.last_draw.is_none_or(|t| t.elapsed() >= REDRAW_INTERVAL)) {
            let line = if inner.files == 0 {
                scanning_line(&self.root, inner.found)
            } else {
                download_line(inner.done_files, inner.files, inner.done_bytes, inner.bytes)
            };
            let _ = write!(inner.out, "\r\x1b[2K{line}");
            let _ = inner.out.flush();
            inner.last_draw = Some(Instant::now());
        }
    }

    /// Clears the progress line. If files were downloaded, says so and shows [`OFFLINE_TIP`].
    pub fn finish(&self) {
        let mut inner = self.inner.lock().unwrap();
        if self.terminal && inner.last_draw.is_some() {
            let _ = write!(inner.out, "\r\x1b[2K");
        }
        if inner.files > 0 {
            let line = format!("Downloaded {} ({}).", files_text(inner.files), megabytes(inner.bytes));
            let _ = writeln!(inner.out, "{line}\n{OFFLINE_TIP}");
        }
        let _ = inner.out.flush();
    }

    pub fn into_inner(self) -> W {
        self.inner.into_inner().unwrap().out
    }
}

fn files_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "file" } else { "files" })
}

pub fn scanning_line(root: &std::path::Path, found: usize) -> String {
    format!(
        "Scanning {}: {} found",
        root.display(),
        if found == 1 { "1 font file".into() } else { format!("{found} font files") }
    )
}

pub fn download_line(done_files: usize, files: usize, done_bytes: u64, bytes: u64) -> String {
    format!("Downloading: {done_files} of {} ({} of {})", files_text(files), megabytes(done_bytes), megabytes(bytes))
}

/// A size for people to read: one decimal below 10 MB, whole MB below 1000 MB, then GB.
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(terminal: bool, events: &[ScanEvent]) -> String {
        let progress = Progress::new("/fonts", terminal, Vec::new());
        for event in events {
            progress.event(event.clone());
        }
        progress.finish();
        String::from_utf8(progress.into_inner()).unwrap()
    }

    const DOWNLOADS: &[ScanEvent] = &[
        ScanEvent::Found(1),
        ScanEvent::Found(2),
        ScanEvent::Downloading { files: 2, bytes: 3_000_000 },
        ScanEvent::Downloaded { bytes: 1_000_000 },
        ScanEvent::Downloaded { bytes: 2_000_000 },
    ];

    #[test]
    fn formats_sizes() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(1_540_000), "1.5 MB");
        assert_eq!(megabytes(210_400_000), "210 MB");
        assert_eq!(megabytes(2_345_000_000), "2.3 GB");
    }

    #[test]
    fn formats_the_lines() {
        assert_eq!(scanning_line(Path::new("/fonts"), 1), "Scanning /fonts: 1 font file found");
        assert_eq!(scanning_line(Path::new("/fonts"), 12), "Scanning /fonts: 12 font files found");
        assert_eq!(download_line(0, 1, 0, 500_000), "Downloading: 0 of 1 file (0.0 MB of 0.5 MB)");
        assert_eq!(download_line(3, 8, 1_000_000, 4_000_000), "Downloading: 3 of 8 files (1.0 MB of 4.0 MB)");
    }

    #[test]
    fn without_a_terminal_shows_only_the_start_and_the_end_of_the_downloads() {
        assert_eq!(
            run(false, DOWNLOADS),
            format!("Downloading 2 files (3.0 MB)...\nDownloaded 2 files (3.0 MB).\n{OFFLINE_TIP}\n")
        );
    }

    #[test]
    fn shows_nothing_without_a_terminal_and_without_downloads() {
        assert_eq!(run(false, &[ScanEvent::Found(1)]), "");
    }

    #[test]
    fn in_a_terminal_updates_one_line_and_clears_it_at_the_end() {
        let output = run(true, DOWNLOADS);
        assert!(output.starts_with("\r\x1b[2KScanning /fonts: 1 font file found"), "{output:?}");
        // The start of the downloads is always drawn. Other updates wait for the redraw interval.
        assert!(output.contains("\r\x1b[2KDownloading: 0 of 2 files (0.0 MB of 3.0 MB)"), "{output:?}");
        assert!(output.ends_with(&format!("\r\x1b[2KDownloaded 2 files (3.0 MB).\n{OFFLINE_TIP}\n")), "{output:?}");
    }

    #[test]
    fn in_a_terminal_without_downloads_leaves_an_empty_line() {
        let output = run(true, &[ScanEvent::Found(1)]);
        assert_eq!(output, "\r\x1b[2KScanning /fonts: 1 font file found\r\x1b[2K");
    }
}
