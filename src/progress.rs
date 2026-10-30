//! Shows the progress of a scan on stderr, so a scan that downloads online-only files does not look frozen.

use crate::scan::ScanEvent;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The minimum time between two updates of the progress line for events. [`Progress::tick`] always redraws.
const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

/// The time between two calls of [`Progress::tick`] from the timer thread.
pub const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// A walk without events for this time waits for a folder listing from the file sync app. The line then says so.
const WAIT_HINT: Duration = Duration::from_secs(10);

pub const OFFLINE_TIP: &str =
    "Tip: Make the Fonts folder available offline in your file sync app. Then sorts does not download the files again.";

pub struct Progress<W> {
    /// In a terminal, one line is updated in place. Otherwise only the start and the end of the downloads are shown.
    terminal: bool,
    /// The width of the terminal in columns. The line is cut to fit, because a line that wraps cannot be updated in
    /// place. 0 means unknown (no cut).
    width: usize,
    start: Instant,
    inner: Mutex<Inner<W>>,
}

struct Inner<W> {
    out: W,
    found: usize,
    /// The folder that the walk lists now.
    folder: String,
    /// The time of the last event of the walk.
    last_walk: Option<Instant>,
    /// True after [`OFFLINE_TIP`] was shown, so it shows only once.
    tip_shown: bool,
    /// Set when the walk is complete: the number of font files to read.
    reading: Option<usize>,
    read: usize,
    files: usize,
    bytes: u64,
    done_files: usize,
    done_bytes: u64,
    last_draw: Option<Instant>,
}

impl<W: Write + Send> Progress<W> {
    pub fn new(terminal: bool, width: usize, out: W) -> Self {
        let inner = Inner {
            out,
            found: 0,
            folder: String::new(),
            last_walk: None,
            tip_shown: false,
            reading: None,
            read: 0,
            files: 0,
            bytes: 0,
            done_files: 0,
            done_bytes: 0,
            last_draw: None,
        };
        Self { terminal, width, start: Instant::now(), inner: Mutex::new(inner) }
    }

    /// The time the progress started. The line shows the time since then.
    pub fn start(&self) -> Instant {
        self.start
    }

    pub fn event(&self, event: ScanEvent) {
        self.event_at(event, Instant::now())
    }

    /// Like [`Progress::event`], at the time `now` (for tests).
    pub fn event_at(&self, event: ScanEvent, now: Instant) {
        let mut inner = self.inner.lock().unwrap();
        let force = match event {
            ScanEvent::Folder { id, .. } => {
                inner.folder = id;
                inner.last_walk = Some(now);
                false
            }
            ScanEvent::Found { count, .. } => {
                inner.found = count;
                inner.last_walk = Some(now);
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
            ScanEvent::Reading { files } => {
                inner.reading = Some(files);
                true
            }
            ScanEvent::ReadStart { .. } => false,
            ScanEvent::Read { downloaded, .. } => {
                inner.read += 1;
                if let Some(bytes) = downloaded {
                    inner.done_files += 1;
                    inner.done_bytes += bytes;
                }
                false
            }
        };
        if force || inner.last_draw.is_none_or(|t| now.saturating_duration_since(t) >= REDRAW_INTERVAL) {
            self.draw(&mut inner, now);
        }
    }

    /// Redraws the line, so the elapsed time moves and the line shows the latest counts also without new events.
    pub fn tick(&self) {
        self.tick_at(Instant::now())
    }

    /// Like [`Progress::tick`], at the time `now` (for tests).
    pub fn tick_at(&self, now: Instant) {
        let mut inner = self.inner.lock().unwrap();
        self.draw(&mut inner, now);
    }

    fn draw(&self, inner: &mut Inner<W>, now: Instant) {
        if !self.terminal {
            return;
        }
        let elapsed = now.saturating_duration_since(self.start);
        let line = if inner.files > 0 {
            download_line(inner.done_files, inner.files, inner.done_bytes, inner.bytes, elapsed)
        } else if let Some(files) = inner.reading {
            reading_line(inner.read, files, elapsed)
        } else if let waited = now.saturating_duration_since(inner.last_walk.unwrap_or(self.start))
            && waited >= WAIT_HINT
        {
            if !inner.tip_shown {
                let _ = writeln!(inner.out, "\r\x1b[2K{OFFLINE_TIP}");
                inner.tip_shown = true;
            }
            waiting_line(&inner.folder, waited)
        } else {
            scanning_line(inner.found, elapsed, &inner.folder)
        };
        let line = cut(&line, self.width);
        let _ = write!(inner.out, "\r\x1b[2K{line}");
        let _ = inner.out.flush();
        inner.last_draw = Some(now);
    }

    /// Clears the progress line. If files were downloaded, says so and shows [`OFFLINE_TIP`].
    pub fn finish(&self) {
        let mut inner = self.inner.lock().unwrap();
        if self.terminal && inner.last_draw.is_some() {
            let _ = write!(inner.out, "\r\x1b[2K");
        }
        if inner.files > 0 {
            let line = format!("Downloaded {} ({}).", files_text(inner.files), megabytes(inner.bytes));
            let _ = writeln!(inner.out, "{line}");
            if !inner.tip_shown {
                let _ = writeln!(inner.out, "{OFFLINE_TIP}");
            }
        }
        let _ = inner.out.flush();
    }

    pub fn into_inner(self) -> W {
        self.inner.into_inner().unwrap().out
    }
}

/// Cuts `line` to fewer than `width` characters, so the cursor stays on the same line. `width` 0 means no cut.
fn cut(line: &str, width: usize) -> &str {
    match line.char_indices().nth(width.saturating_sub(1)) {
        Some((index, _)) if width > 0 => &line[..index],
        _ => line,
    }
}

fn files_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "file" } else { "files" })
}

/// A duration for people to read: `m:ss`.
pub fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn scanning_line(found: usize, elapsed: Duration, folder: &str) -> String {
    let found = if found == 1 { "1 font file".into() } else { format!("{found} font files") };
    let line = format!("Scanning: {found} found, {}", elapsed_text(elapsed));
    if folder.is_empty() { line } else { format!("{line}, in {folder}") }
}

/// The line while the walk waits for the listing of `folder` (empty for the root) for the time `waited`.
pub fn waiting_line(folder: &str, waited: Duration) -> String {
    let folder = if folder.is_empty() { "the Fonts folder" } else { folder };
    format!("Waiting for {folder} to load from your file sync app, {}", elapsed_text(waited))
}

pub fn reading_line(read: usize, files: usize, elapsed: Duration) -> String {
    format!("Reading: {read} of {} font files, {}", files, elapsed_text(elapsed))
}

pub fn download_line(done_files: usize, files: usize, done_bytes: u64, bytes: u64, elapsed: Duration) -> String {
    format!(
        "Downloading: {done_files} of {} ({} of {}), {}",
        files_text(files),
        megabytes(done_bytes),
        megabytes(bytes),
        elapsed_text(elapsed)
    )
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

    fn found(count: usize) -> ScanEvent {
        ScanEvent::Found { count, id: format!("A/{count}.otf"), size: 1, flags: 0, online_only: false }
    }

    fn read(downloaded: Option<u64>) -> ScanEvent {
        ScanEvent::Read { id: "A/1.otf".into(), downloaded, error: None }
    }

    /// Sends all events at the start time, so only forced draws and the first draw happen.
    fn run(terminal: bool, events: &[ScanEvent]) -> String {
        let progress = Progress::new(terminal, 0, Vec::new());
        for event in events {
            progress.event_at(event.clone(), progress.start());
        }
        progress.finish();
        String::from_utf8(progress.into_inner()).unwrap()
    }

    fn downloads() -> Vec<ScanEvent> {
        vec![
            found(1),
            found(2),
            ScanEvent::Downloading { files: 2, bytes: 3_000_000 },
            ScanEvent::Reading { files: 2 },
            read(Some(1_000_000)),
            read(Some(2_000_000)),
        ]
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(1_540_000), "1.5 MB");
        assert_eq!(megabytes(210_400_000), "210 MB");
        assert_eq!(megabytes(2_345_000_000), "2.3 GB");
    }

    #[test]
    fn formats_the_lines() {
        let s = Duration::from_secs;
        assert_eq!(scanning_line(0, s(0), ""), "Scanning: 0 font files found, 0:00");
        assert_eq!(scanning_line(1, s(5), "A/"), "Scanning: 1 font file found, 0:05, in A/");
        assert_eq!(scanning_line(12, s(125), "A/B/"), "Scanning: 12 font files found, 2:05, in A/B/");
        assert_eq!(reading_line(3, 8, s(61)), "Reading: 3 of 8 font files, 1:01");
        assert_eq!(download_line(0, 1, 0, 500_000, s(0)), "Downloading: 0 of 1 file (0.0 MB of 0.5 MB), 0:00");
        assert_eq!(
            download_line(3, 8, 1_000_000, 4_000_000, s(42)),
            "Downloading: 3 of 8 files (1.0 MB of 4.0 MB), 0:42"
        );
    }

    #[test]
    fn cuts_the_line_to_the_terminal_width() {
        assert_eq!(cut("Scanning", 5), "Scan");
        assert_eq!(cut("Scanning", 9), "Scanning");
        assert_eq!(cut("Scanning", 8), "Scannin");
        assert_eq!(cut("Scanning", 0), "Scanning");
        assert_eq!(cut("Ærø Sans", 3), "Ær");
    }

    #[test]
    fn a_line_in_a_narrow_terminal_does_not_wrap() {
        let progress = Progress::new(true, 20, Vec::new());
        progress.tick_at(progress.start());
        assert_eq!(String::from_utf8(progress.into_inner()).unwrap(), "\r\x1b[2KScanning: 0 font fi");
    }

    #[test]
    fn without_a_terminal_shows_only_the_start_and_the_end_of_the_downloads() {
        assert_eq!(
            run(false, &downloads()),
            format!("Downloading 2 files (3.0 MB)...\nDownloaded 2 files (3.0 MB).\n{OFFLINE_TIP}\n")
        );
    }

    #[test]
    fn shows_nothing_without_a_terminal_and_without_downloads() {
        let progress = Progress::new(false, 0, Vec::new());
        progress.event(found(1));
        progress.tick();
        progress.finish();
        assert_eq!(progress.into_inner(), b"");
    }

    #[test]
    fn in_a_terminal_updates_one_line_and_clears_it_at_the_end() {
        let output = run(true, &downloads());
        assert!(output.starts_with("\r\x1b[2KScanning: 1 font file found, 0:00"), "{output:?}");
        // The start of the downloads is always drawn. Other updates wait for the redraw interval.
        assert!(output.contains("\r\x1b[2KDownloading: 0 of 2 files (0.0 MB of 3.0 MB), 0:00"), "{output:?}");
        assert!(output.ends_with(&format!("\r\x1b[2KDownloaded 2 files (3.0 MB).\n{OFFLINE_TIP}\n")), "{output:?}");
    }

    #[test]
    fn in_a_terminal_without_downloads_shows_the_reads() {
        let progress = Progress::new(true, 0, Vec::new());
        let start = progress.start();
        progress.event_at(found(1), start);
        progress.event_at(ScanEvent::Reading { files: 1 }, start);
        progress.event_at(read(None), start + Duration::from_secs(2));
        progress.finish();
        let output = String::from_utf8(progress.into_inner()).unwrap();
        assert_eq!(
            output,
            "\r\x1b[2KScanning: 1 font file found, 0:00\
             \r\x1b[2KReading: 0 of 1 font files, 0:00\
             \r\x1b[2KReading: 1 of 1 font files, 0:02\
             \r\x1b[2K"
        );
    }

    #[test]
    fn a_tick_shows_the_latest_count_and_folder_without_new_events() {
        let progress = Progress::new(true, 0, Vec::new());
        let start = progress.start();
        progress.event_at(ScanEvent::Folder { id: String::new(), flags: 0 }, start);
        progress.event_at(found(1), start);
        progress.event_at(ScanEvent::Folder { id: "Foundry/".into(), flags: 0 }, start);
        progress.event_at(found(2), start);
        progress.tick_at(start + Duration::from_secs(7));
        let output = String::from_utf8(progress.into_inner()).unwrap();
        assert_eq!(
            output,
            "\r\x1b[2KScanning: 0 font files found, 0:00\
             \r\x1b[2KScanning: 2 font files found, 0:07, in Foundry/"
        );
    }

    fn folder(id: &str) -> ScanEvent {
        ScanEvent::Folder { id: id.into(), flags: 0 }
    }

    #[test]
    fn formats_the_waiting_line() {
        let s = Duration::from_secs;
        assert_eq!(waiting_line("A/B/", s(40)), "Waiting for A/B/ to load from your file sync app, 0:40");
        assert_eq!(waiting_line("", s(10)), "Waiting for the Fonts folder to load from your file sync app, 0:10");
    }

    #[test]
    fn a_walk_without_events_shows_the_folder_that_it_waits_for_and_the_tip_once() {
        let progress = Progress::new(true, 0, Vec::new());
        let t = |s| progress.start() + Duration::from_secs(s);
        progress.event_at(folder("Foundry/"), t(0));
        progress.event_at(folder("Foundry/Slow/"), t(2));
        progress.tick_at(t(11));
        progress.tick_at(t(12));
        progress.tick_at(t(13));
        progress.event_at(found(1), t(14));
        progress.tick_at(t(15));
        progress.finish();
        let output = String::from_utf8(progress.into_inner()).unwrap();
        assert_eq!(
            output,
            format!(
                "\r\x1b[2KScanning: 0 font files found, 0:00, in Foundry/\
                 \r\x1b[2KScanning: 0 font files found, 0:02, in Foundry/Slow/\
                 \r\x1b[2KScanning: 0 font files found, 0:11, in Foundry/Slow/\
                 \r\x1b[2K{OFFLINE_TIP}\n\
                 \r\x1b[2KWaiting for Foundry/Slow/ to load from your file sync app, 0:10\
                 \r\x1b[2KWaiting for Foundry/Slow/ to load from your file sync app, 0:11\
                 \r\x1b[2KScanning: 1 font file found, 0:14, in Foundry/Slow/\
                 \r\x1b[2KScanning: 1 font file found, 0:15, in Foundry/Slow/\
                 \r\x1b[2K"
            )
        );
    }

    #[test]
    fn the_tip_shows_only_once_after_a_wait_and_downloads() {
        let progress = Progress::new(true, 0, Vec::new());
        let t = |s| progress.start() + Duration::from_secs(s);
        progress.event_at(folder(""), t(0));
        progress.tick_at(t(10));
        for event in downloads() {
            progress.event_at(event, t(11));
        }
        progress.finish();
        let output = String::from_utf8(progress.into_inner()).unwrap();
        assert!(output.contains("Waiting for the Fonts folder to load"), "{output:?}");
        assert_eq!(output.matches(OFFLINE_TIP).count(), 1, "{output:?}");
        assert!(output.ends_with("\r\x1b[2KDownloaded 2 files (3.0 MB).\n"), "{output:?}");
    }

    #[test]
    fn the_wait_has_no_effect_without_a_terminal() {
        let progress = Progress::new(false, 0, Vec::new());
        progress.event_at(folder("A/"), progress.start());
        progress.tick_at(progress.start() + Duration::from_secs(60));
        progress.finish();
        assert_eq!(progress.into_inner(), b"");
    }
}
